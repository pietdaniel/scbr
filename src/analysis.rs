use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Instant;

use blake3::Hasher;
use rayon::prelude::*;
use regex::Regex;
use tree_sitter::{Node, Parser, Point};

use crate::language::Language;
use crate::model::{
    AnalysisReport, AnalysisSettings, CloneGroup, CloneInstance, FileMetrics, Finding,
    FindingCategory, FunctionMetric, LanguageSummary, METRIC_VERSION, Severity, SkippedFile, Span,
    Summary,
};

#[derive(Debug)]
struct FileAnalysis {
    metrics: FileMetrics,
    functions: Vec<FunctionMetric>,
    clone_candidates: Vec<CloneCandidate>,
    findings: Vec<Finding>,
    sloc_lines: BTreeSet<u32>,
    rule_lines: BTreeSet<u32>,
    rule_physical_lines: BTreeSet<u32>,
}

#[derive(Debug, Clone)]
struct CloneCandidate {
    path: PathBuf,
    language: Language,
    span: Span,
    sloc_lines: Vec<u32>,
    fingerprint: String,
    preview: String,
}

#[derive(Debug)]
struct LineFacts {
    loc: usize,
    sloc_lines: BTreeSet<u32>,
    sloc_prefix: Vec<u32>,
    blank_lines: usize,
    comment_lines: BTreeSet<u32>,
    comment_only_lines: BTreeSet<u32>,
    code_lines: Vec<String>,
}

impl LineFacts {
    fn count_sloc(&self, span: Span) -> u32 {
        if self.loc == 0 {
            return 0;
        }
        let start = span.start_line.max(1).min(self.loc as u32) as usize;
        let end = span.end_line.max(span.start_line).min(self.loc as u32) as usize;
        self.sloc_prefix[end].saturating_sub(self.sloc_prefix[start - 1])
    }

    fn sloc_in_span(&self, span: Span) -> Vec<u32> {
        self.sloc_lines
            .range(span.start_line..=span.end_line)
            .copied()
            .collect()
    }
}

#[derive(Debug)]
struct RuleSpec {
    id: &'static str,
    message: &'static str,
    languages: &'static [Language],
    regex: Regex,
}

impl RuleSpec {
    fn applies_to(&self, language: Language) -> bool {
        self.languages.is_empty() || self.languages.contains(&language)
    }
}

static LINE_RULES: LazyLock<Vec<RuleSpec>> = LazyLock::new(|| {
    vec![
        RuleSpec {
            id: "redundant-boolean-comparison",
            message: "comparison with a boolean literal can usually be simplified",
            languages: &[],
            regex: Regex::new(r"(?:==|!=)\s*(?:true|false|True|False)\b").unwrap(),
        },
        RuleSpec {
            id: "python-range-len",
            message: "index iteration with range(len(...)) is usually clearer with direct iteration or enumerate",
            languages: &[Language::Python],
            regex: Regex::new(r"\brange\s*\(\s*len\s*\(").unwrap(),
        },
        RuleSpec {
            id: "python-redundant-get-default",
            message: "dict.get(..., None) repeats the default value",
            languages: &[Language::Python],
            regex: Regex::new(r"\.get\s*\([^,\n]+,\s*None\s*\)").unwrap(),
        },
        RuleSpec {
            id: "python-explicit-length-check",
            message: "Python collections can normally be tested directly for emptiness",
            languages: &[Language::Python],
            regex: Regex::new(r"\b(?:if|while)\s+len\s*\([^)]*\)\s*(?:>|!=)\s*0\s*:").unwrap(),
        },
        RuleSpec {
            id: "python-list-before-iteration",
            message: "materializing list(...) only to iterate is often unnecessary",
            languages: &[Language::Python],
            regex: Regex::new(r"\bfor\b[^\n]*\bin\s+list\s*\(").unwrap(),
        },
        RuleSpec {
            id: "rust-length-check",
            message: "use is_empty() for a collection emptiness check",
            languages: &[Language::Rust],
            regex: Regex::new(r"\.len\s*\(\s*\)\s*(?:>|!=)\s*0").unwrap(),
        },
        RuleSpec {
            id: "jvm-length-check",
            message: "use isEmpty()/isNotEmpty() for a collection emptiness check",
            languages: &[Language::Java, Language::Kotlin],
            regex: Regex::new(r"\.(?:size|length)\s*(?:\(\s*\))?\s*(?:>|!=)\s*0").unwrap(),
        },
        RuleSpec {
            id: "javascript-empty-array-constructor",
            message: "an empty array literal is shorter and clearer than new Array()",
            languages: &[Language::JavaScript, Language::TypeScript],
            regex: Regex::new(r"\bnew\s+Array\s*\(\s*\)").unwrap(),
        },
        RuleSpec {
            id: "redundant-boolean-ternary",
            message: "a true/false conditional expression can usually be the condition itself",
            languages: &[
                Language::JavaScript,
                Language::TypeScript,
                Language::Java,
                Language::Kotlin,
            ],
            regex: Regex::new(r"\?\s*(?:true|false)\s*:\s*(?:false|true)\b").unwrap(),
        },
    ]
});

static DEBT_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:TODO|FIXME|HACK|XXX)\b").unwrap());
static COMMENTED_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)^\s*(?:(?:if|for|while|return|let|const|var|def|fn|fun|class|public|private)\b|[A-Za-z_][A-Za-z0-9_]*\s*=)",
    )
    .unwrap()
});

/// Analyze a deterministic list of supported source files.
///
/// Individual read or parser setup failures are returned in `skipped_files` so
/// one damaged file cannot silently erase an otherwise useful repository report.
pub fn analyze_files(files: &[PathBuf], settings: &AnalysisSettings) -> AnalysisReport {
    let started = Instant::now();
    let mut outcomes: Vec<Result<FileAnalysis, SkippedFile>> = files
        .par_iter()
        .map(|path| analyze_file(path, settings))
        .collect();

    outcomes.sort_by(|left, right| outcome_path(left).cmp(outcome_path(right)));

    let mut analyses = Vec::new();
    let mut skipped_files = Vec::new();
    for outcome in outcomes {
        match outcome {
            Ok(analysis) => analyses.push(analysis),
            Err(skipped) => skipped_files.push(skipped),
        }
    }

    build_report(
        analyses,
        skipped_files,
        settings.clone(),
        started.elapsed().as_millis() as u64,
    )
}

fn outcome_path(outcome: &Result<FileAnalysis, SkippedFile>) -> &Path {
    match outcome {
        Ok(analysis) => &analysis.metrics.path,
        Err(skipped) => &skipped.path,
    }
}

fn analyze_file(path: &Path, settings: &AnalysisSettings) -> Result<FileAnalysis, SkippedFile> {
    let language = Language::from_path(path).ok_or_else(|| SkippedFile {
        path: path.to_path_buf(),
        reason: "unsupported file extension".into(),
    })?;
    let source = fs::read_to_string(path).map_err(|error| SkippedFile {
        path: path.to_path_buf(),
        reason: format!("could not read UTF-8 source: {error}"),
    })?;

    let mut parser = Parser::new();
    parser
        .set_language(&language.tree_sitter_language(path))
        .map_err(|error| SkippedFile {
            path: path.to_path_buf(),
            reason: format!("could not load {language} grammar: {error}"),
        })?;
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| SkippedFile {
            path: path.to_path_buf(),
            reason: "Tree-sitter parser returned no syntax tree".into(),
        })?;
    let root = tree.root_node();

    let tree_scan = scan_tree(root, language, &source);
    let line_facts = build_line_facts(
        &source,
        &tree_scan.comment_ranges,
        &tree_scan.docstring_ranges,
        &tree_scan.rule_string_ranges,
    );

    let mut findings = tree_scan.comment_findings(path);
    if tree_scan.parse_errors > 0 {
        findings.push(Finding {
            rule_id: "parse-error".into(),
            category: FindingCategory::Parse,
            severity: Severity::Error,
            path: path.to_path_buf(),
            span: Span::new(1, line_facts.loc.max(1)),
            message: format!(
                "Tree-sitter recovered from {} syntax error or missing node(s); metrics may be partial",
                tree_scan.parse_errors
            ),
            counts_toward_verbosity: false,
        });
    }

    findings.extend(line_rule_findings(path, language, &line_facts.code_lines));

    let mut functions = Vec::new();
    for function_node in tree_scan.function_nodes {
        let metric = function_metric(path, language, function_node, &source, &line_facts);
        findings.extend(function_findings(function_node, &metric, &source, settings));
        functions.push(metric);
    }
    functions.sort_by_key(|function| (function.span.start_line, function.span.end_line));

    let clone_candidates = clone_candidates(path, language, root, &source, &line_facts, settings);

    if line_facts.loc >= 20
        && !line_facts.sloc_lines.is_empty()
        && ratio(line_facts.comment_lines.len() as u64, line_facts.loc as u64) > 0.5
    {
        findings.push(Finding {
            rule_id: "high-comment-density".into(),
            category: FindingCategory::Comment,
            severity: Severity::Info,
            path: path.to_path_buf(),
            span: Span::new(1, line_facts.loc),
            message: "more than half of the physical lines contain comments; review stale or commented-out code".into(),
            counts_toward_verbosity: false,
        });
    }

    findings.sort_by(|left, right| {
        (
            left.span.start_line,
            left.span.end_line,
            left.rule_id.as_str(),
        )
            .cmp(&(
                right.span.start_line,
                right.span.end_line,
                right.rule_id.as_str(),
            ))
    });
    let rule_lines = finding_sloc_lines(&findings, &line_facts.sloc_lines);
    let rule_physical_lines = finding_physical_lines(&findings, line_facts.loc);
    let debt_markers = findings
        .iter()
        .filter(|finding| finding.rule_id == "debt-marker")
        .count();

    Ok(FileAnalysis {
        metrics: FileMetrics {
            path: path.to_path_buf(),
            language,
            loc: line_facts.loc as u64,
            sloc: line_facts.sloc_lines.len() as u64,
            blank_lines: line_facts.blank_lines as u64,
            comment_lines: line_facts.comment_lines.len() as u64,
            comment_only_lines: line_facts.comment_only_lines.len() as u64,
            syntax_nodes: tree_scan.syntax_nodes as u64,
            parse_errors: tree_scan.parse_errors as u64,
            functions: functions.len() as u64,
            duplicated_sloc_lines: 0,
            excess_duplicate_sloc_lines: 0,
            rule_flagged_sloc_lines: rule_lines.len() as u64,
            verbosity_flagged_sloc_lines: rule_lines.len() as u64,
            debt_markers: debt_markers as u64,
        },
        functions,
        clone_candidates,
        findings,
        sloc_lines: line_facts.sloc_lines,
        rule_lines,
        rule_physical_lines,
    })
}

#[derive(Debug)]
struct TreeScan<'tree> {
    syntax_nodes: usize,
    parse_errors: usize,
    function_nodes: Vec<Node<'tree>>,
    comment_ranges: Vec<(usize, usize, Point, Point, String)>,
    docstring_ranges: Vec<(usize, usize)>,
    rule_string_ranges: Vec<(usize, usize)>,
}

impl TreeScan<'_> {
    fn comment_findings(&self, path: &Path) -> Vec<Finding> {
        let mut findings = Vec::new();
        for (_, _, start, end, text) in &self.comment_ranges {
            let span = point_span(*start, *end);
            for marker in DEBT_MARKER.find_iter(text) {
                findings.push(Finding {
                    rule_id: "debt-marker".into(),
                    category: FindingCategory::Comment,
                    severity: Severity::Info,
                    path: path.to_path_buf(),
                    span,
                    message: format!("{} marker in comment", marker.as_str().to_ascii_uppercase()),
                    counts_toward_verbosity: false,
                });
            }

            for (line_offset, line) in text.lines().enumerate() {
                let content = strip_comment_prefix(line);
                if content.len() >= 5 && COMMENTED_CODE.is_match(content) {
                    let line_number = (start.row + line_offset + 1).min(end.row + 1);
                    findings.push(Finding {
                        rule_id: "commented-out-code".into(),
                        category: FindingCategory::Comment,
                        severity: Severity::Info,
                        path: path.to_path_buf(),
                        span: Span::new(line_number, line_number),
                        message: "comment resembles disabled source code".into(),
                        counts_toward_verbosity: false,
                    });
                }
            }
        }
        findings
    }
}

fn scan_tree<'tree>(root: Node<'tree>, language: Language, source: &str) -> TreeScan<'tree> {
    let mut scan = TreeScan {
        syntax_nodes: 0,
        parse_errors: 0,
        function_nodes: Vec::new(),
        comment_ranges: Vec::new(),
        docstring_ranges: Vec::new(),
        rule_string_ranges: Vec::new(),
    };
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        scan.syntax_nodes += 1;
        if node.is_error() || node.is_missing() {
            scan.parse_errors += 1;
        }
        if node.is_named() && language.is_function(node.kind()) {
            scan.function_nodes.push(node);
        }
        if language.is_comment(node.kind()) {
            scan.comment_ranges.push((
                node.start_byte(),
                node.end_byte(),
                node.start_position(),
                node.end_position(),
                node_text(node, source).to_owned(),
            ));
        } else if language == Language::Python && is_plain_string_statement(node, source) {
            scan.docstring_ranges
                .push((node.start_byte(), node.end_byte()));
        }
        if is_string_content(node.kind())
            || (is_string_container(node.kind()) && !contains_interpolation(node))
        {
            scan.rule_string_ranges
                .push((node.start_byte(), node.end_byte()));
        }

        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    scan
}

fn build_line_facts(
    source: &str,
    comment_ranges: &[(usize, usize, Point, Point, String)],
    docstring_ranges: &[(usize, usize)],
    rule_string_ranges: &[(usize, usize)],
) -> LineFacts {
    let raw_lines: Vec<&str> = source.lines().collect();
    let loc = raw_lines.len();
    let mut cleaned = source.as_bytes().to_vec();
    let mut comment_lines = BTreeSet::new();

    for (start_byte, end_byte, start, end, _) in comment_ranges {
        mask_range(&mut cleaned, *start_byte, *end_byte);
        if loc == 0 {
            continue;
        }
        let end_row = effective_end_row(*start, *end).min(loc.saturating_sub(1));
        for row in start.row.min(end_row)..=end_row {
            comment_lines.insert((row + 1) as u32);
        }
    }
    for (start, end) in docstring_ranges {
        mask_range(&mut cleaned, *start, *end);
    }

    let mut rule_cleaned = cleaned.clone();
    for (start, end) in rule_string_ranges {
        mask_range(&mut rule_cleaned, *start, *end);
    }

    let cleaned_source = String::from_utf8(cleaned).expect("comment masking preserves UTF-8");
    let rule_source = String::from_utf8(rule_cleaned).expect("string masking preserves UTF-8");
    let cleaned_lines: Vec<&str> = cleaned_source.lines().collect();
    let rule_lines: Vec<String> = rule_source.lines().map(str::to_owned).collect();
    let mut sloc_lines = BTreeSet::new();
    let mut blank_lines = 0;
    let mut comment_only_lines = BTreeSet::new();
    let mut sloc_prefix = vec![0_u32; loc + 1];

    for (index, raw) in raw_lines.iter().enumerate() {
        let line_number = (index + 1) as u32;
        let cleaned_line = cleaned_lines.get(index).copied().unwrap_or_default().trim();
        if raw.trim().is_empty() {
            blank_lines += 1;
        }
        if comment_lines.contains(&line_number) && cleaned_line.is_empty() {
            comment_only_lines.insert(line_number);
        }
        if !cleaned_line.is_empty() && !is_punctuation_only(cleaned_line) {
            sloc_lines.insert(line_number);
        }
        sloc_prefix[index + 1] = sloc_prefix[index] + u32::from(sloc_lines.contains(&line_number));
    }

    LineFacts {
        loc,
        sloc_lines,
        sloc_prefix,
        blank_lines,
        comment_lines,
        comment_only_lines,
        code_lines: rule_lines,
    }
}

fn is_string_container(kind: &str) -> bool {
    matches!(
        kind,
        "string"
            | "string_literal"
            | "template_string"
            | "interpreted_string_literal"
            | "raw_string_literal"
            | "line_string_literal"
            | "multi_line_string_literal"
            | "concatenated_string"
    ) || kind.contains("string_literal")
}

fn is_string_content(kind: &str) -> bool {
    matches!(
        kind,
        "string_content" | "string_fragment" | "multiline_string_fragment"
    )
}

fn contains_interpolation(node: Node<'_>) -> bool {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if matches!(current.kind(), "interpolation" | "template_substitution") {
            return true;
        }
        let mut cursor = current.walk();
        stack.extend(current.named_children(&mut cursor));
    }
    false
}

fn mask_range(bytes: &mut [u8], start: usize, end: usize) {
    for byte in bytes.iter_mut().take(end).skip(start) {
        if *byte != b'\n' && *byte != b'\r' {
            *byte = b' ';
        }
    }
}

fn effective_end_row(start: Point, end: Point) -> usize {
    if end.row > start.row && end.column == 0 {
        end.row - 1
    } else {
        end.row
    }
}

fn is_punctuation_only(line: &str) -> bool {
    let compact: String = line
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if matches!(compact.as_str(), "()" | "();" | "[]" | "[];" | "{}" | "{};") {
        return false;
    }
    compact
        .chars()
        .all(|character| character.is_whitespace() || "{}[]();,:".contains(character))
}

fn is_plain_string_statement(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "expression_statement" || node.named_child_count() != 1 {
        return false;
    }
    let Some(expression) = node.named_child(0) else {
        return false;
    };
    if !matches!(expression.kind(), "string" | "concatenated_string") {
        return false;
    }
    if contains_interpolation(expression) {
        return false;
    }
    let mut stack = vec![expression];
    while let Some(current) = stack.pop() {
        if current.kind() == "string" {
            let text = node_text(current, source).trim_start();
            let prefix = text
                .chars()
                .take_while(|character| character.is_ascii_alphabetic());
            if prefix
                .map(|character| character.to_ascii_lowercase())
                .any(|character| matches!(character, 'b' | 'f'))
            {
                return false;
            }
        }
        let mut cursor = current.walk();
        stack.extend(current.named_children(&mut cursor));
    }
    true
}

fn line_rule_findings(path: &Path, language: Language, code_lines: &[String]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (index, line) in code_lines.iter().enumerate() {
        for rule in LINE_RULES.iter().filter(|rule| rule.applies_to(language)) {
            if rule.regex.is_match(line) {
                findings.push(Finding {
                    rule_id: rule.id.into(),
                    category: FindingCategory::Verbosity,
                    severity: Severity::Warning,
                    path: path.to_path_buf(),
                    span: Span::new(index + 1, index + 1),
                    message: rule.message.into(),
                    counts_toward_verbosity: true,
                });
            }
        }
    }
    findings
}

fn function_metric(
    path: &Path,
    language: Language,
    node: Node<'_>,
    source: &str,
    lines: &LineFacts,
) -> FunctionMetric {
    let span = function_span(node, language);
    let sloc = exclusive_function_sloc(node, span, language, lines);
    let (cyclomatic_complexity, cognitive_complexity, max_nesting) =
        function_complexity(node, language, source);
    FunctionMetric {
        path: path.to_path_buf(),
        language,
        name: function_name(node, source),
        span,
        sloc,
        parameters: parameter_count(node, language),
        cyclomatic_complexity,
        cognitive_complexity,
        max_nesting,
        mass: cyclomatic_complexity as f64 * (sloc as f64).sqrt(),
        cognitive_mass: cognitive_complexity as f64 * (sloc as f64).sqrt(),
    }
}

fn function_span(node: Node<'_>, language: Language) -> Span {
    node_span(function_span_node(node, language))
}

fn function_span_node<'tree>(node: Node<'tree>, language: Language) -> Node<'tree> {
    if language == Language::Python {
        node.parent()
            .filter(|parent| parent.kind() == "decorated_definition")
            .unwrap_or(node)
    } else {
        node
    }
}

fn function_name(node: Node<'_>, source: &str) -> String {
    for field in ["name", "declarator"] {
        if let Some(name) = node.child_by_field_name(field) {
            if name.kind().contains("identifier") {
                return node_text(name, source).to_owned();
            }
            if let Some(identifier) = first_identifier(name) {
                return node_text(identifier, source).to_owned();
            }
        }
    }
    if matches!(node.kind(), "getter" | "setter")
        && let Some(property) = node
            .parent()
            .filter(|parent| parent.kind() == "property_declaration")
            .and_then(first_identifier)
    {
        let accessor = if node.kind() == "getter" {
            "get"
        } else {
            "set"
        };
        return format!("{}.{}", node_text(property, source), accessor);
    }
    if let Some(parent) = node.parent()
        && matches!(
            parent.kind(),
            "variable_declarator"
                | "assignment_expression"
                | "assignment"
                | "assignment_statement"
                | "let_declaration"
                | "const_declaration"
                | "property_declaration"
        )
    {
        for field in ["name", "left", "pattern"] {
            if let Some(binding) = parent.child_by_field_name(field)
                && let Some(identifier) = first_identifier(binding)
            {
                return node_text(identifier, source).to_owned();
            }
        }
    }
    format!("<anonymous@{}>", node.start_position().row + 1)
}

fn exclusive_function_sloc(
    node: Node<'_>,
    span: Span,
    language: Language,
    lines: &LineFacts,
) -> u32 {
    let mut nested_nodes = Vec::new();
    let mut cursor = node.walk();
    let mut stack: Vec<_> = node.named_children(&mut cursor).collect();
    while let Some(current) = stack.pop() {
        if language.is_function(current.kind()) {
            nested_nodes.push(function_span_node(current, language));
            continue;
        }
        let mut cursor = current.walk();
        stack.extend(current.named_children(&mut cursor));
    }
    let sloc = lines
        .sloc_in_span(span)
        .into_iter()
        .filter(|line| {
            !nested_nodes.iter().any(|nested| {
                let nested = node_span(*nested);
                nested.start_line <= *line && *line <= nested.end_line
            })
        })
        .count() as u32;
    if sloc == 0 && lines.count_sloc(span) > 0 {
        // Multiple functions can occupy one physical line. Their ownership
        // cannot be partitioned, but each real function needs nonzero mass.
        1
    } else {
        sloc
    }
}

fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind().contains("identifier") {
            return Some(current);
        }
        let mut cursor = current.walk();
        let mut children: Vec<_> = current.named_children(&mut cursor).collect();
        children.reverse();
        stack.extend(children);
    }
    None
}

fn parameter_count(node: Node<'_>, language: Language) -> u32 {
    if language == Language::Kotlin && node.kind() == "setter" {
        return 1;
    }
    let parameter_owner =
        if language == Language::Java && node.kind() == "compact_constructor_declaration" {
            node.parent()
                .and_then(|parent| parent.parent())
                .filter(|ancestor| ancestor.kind() == "record_declaration")
                .unwrap_or(node)
        } else {
            node
        };
    let parameters = parameter_owner
        .child_by_field_name("parameters")
        .or_else(|| parameter_owner.child_by_field_name("parameter"))
        .or_else(|| {
            let mut cursor = parameter_owner.walk();
            parameter_owner.named_children(&mut cursor).find(|child| {
                matches!(
                    child.kind(),
                    "parameters"
                        | "formal_parameters"
                        | "inferred_parameters"
                        | "parameter_list"
                        | "closure_parameters"
                        | "function_value_parameters"
                        | "lambda_parameters"
                )
            })
        });
    let Some(parameters) = parameters else {
        return 0;
    };
    if !matches!(
        parameters.kind(),
        "parameters"
            | "formal_parameters"
            | "inferred_parameters"
            | "parameter_list"
            | "closure_parameters"
            | "function_value_parameters"
            | "lambda_parameters"
    ) {
        // Unparenthesized single-parameter lambdas and arrow functions expose
        // the parameter itself through the field.
        return 1;
    }

    let mut cursor = parameters.walk();
    let children: Vec<_> = parameters
        .named_children(&mut cursor)
        .filter(|child| !language.is_comment(child.kind()))
        .collect();
    match language {
        Language::Python => children
            .iter()
            .filter(|child| !matches!(child.kind(), "positional_separator" | "keyword_separator"))
            .count() as u32,
        Language::JavaScript | Language::TypeScript => children.len() as u32,
        Language::Go => children
            .iter()
            .map(|child| {
                let mut cursor = child.walk();
                let names = child
                    .named_children(&mut cursor)
                    .filter(|candidate| candidate.kind() == "identifier")
                    .count();
                names.max(1) as u32
            })
            .sum(),
        Language::Rust if parameters.kind() == "closure_parameters" => children.len() as u32,
        Language::Rust => children
            .iter()
            .filter(|child| {
                matches!(
                    child.kind(),
                    "parameter" | "self_parameter" | "variadic_parameter"
                )
            })
            .count() as u32,
        Language::Java if parameters.kind() == "inferred_parameters" => children.len() as u32,
        Language::Java => children
            .iter()
            .filter(|child| {
                matches!(
                    child.kind(),
                    "formal_parameter" | "receiver_parameter" | "spread_parameter"
                )
            })
            .count() as u32,
        Language::Kotlin => children
            .iter()
            .filter(|child| {
                matches!(
                    child.kind(),
                    "parameter" | "variable_declaration" | "multi_variable_declaration"
                )
            })
            .count() as u32,
    }
}

fn function_complexity(node: Node<'_>, language: Language, source: &str) -> (u32, u32, u32) {
    let mut cyclomatic = 1_u32;
    let mut cognitive = 0_u32;
    let mut max_nesting = 0_u32;
    let mut stack = Vec::new();
    let mut root_cursor = node.walk();
    stack.extend(
        node.named_children(&mut root_cursor)
            .map(|child| (child, 0_u32)),
    );

    while let Some((current, nesting)) = stack.pop() {
        if language.is_function(current.kind()) {
            continue;
        }

        let logical_operators = logical_operator_count(current, language, source);
        if logical_operators > 0 {
            cyclomatic += logical_operators;
            cognitive += logical_operators;
        }

        let is_decision = is_decision_node(current, language, source);
        if is_decision {
            cyclomatic += 1;
            cognitive += 1 + nesting;
        } else if language.is_jump(current.kind()) {
            cognitive += 1;
        }

        let child_nesting = if is_decision && language.is_nesting_decision(current.kind()) {
            nesting + 1
        } else {
            nesting
        };
        max_nesting = max_nesting.max(child_nesting);
        let mut cursor = current.walk();
        stack.extend(
            current
                .named_children(&mut cursor)
                .map(|child| (child, child_nesting)),
        );
    }
    (cyclomatic, cognitive, max_nesting)
}

fn is_decision_node(node: Node<'_>, language: Language, source: &str) -> bool {
    if !language.is_decision(node.kind()) {
        return false;
    }

    // Fallback arms add no independent path. Several grammars give the
    // unconditional arm the same node kind as conditional arms.
    let text = node_text(node, source).trim_start();
    match node.kind() {
        "switch_label" => !text.starts_with("default"),
        "when_entry" => !text.starts_with("else"),
        "match_arm" => {
            let pattern = node
                .child_by_field_name("pattern")
                .map(|pattern| node_text(pattern, source).trim())
                .unwrap_or_default();
            let guarded = text
                .split_once("=>")
                .is_some_and(|(head, _)| head.contains(" if "));
            !is_bare_rust_binding(pattern) || guarded
        }
        "case_clause" => {
            let mut cursor = node.walk();
            let pattern = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "case_pattern")
                .map(|pattern| node_text(pattern, source).trim())
                .unwrap_or_default();
            !is_bare_identifier(pattern)
        }
        _ => true,
    }
}

fn is_bare_rust_binding(pattern: &str) -> bool {
    let pattern = pattern.strip_prefix("r#").unwrap_or(pattern);
    is_bare_identifier(pattern)
        && pattern
            .chars()
            .next()
            .is_some_and(|first| first == '_' || first.is_lowercase())
}

fn is_bare_identifier(pattern: &str) -> bool {
    let mut characters = pattern.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

fn logical_operator_count(node: Node<'_>, language: Language, source: &str) -> u32 {
    if !language.is_boolean_expression(node.kind()) {
        return 0;
    }
    if let Some(operator) = node.child_by_field_name("operator") {
        return u32::from(matches!(
            node_text(operator, source),
            "&&" | "||" | "and" | "or"
        ));
    }
    let mut cursor = node.walk();
    let count = node
        .children(&mut cursor)
        .filter(|child| matches!(node_text(*child, source), "&&" | "||" | "and" | "or"))
        .count() as u32;
    if count == 0 && language == Language::Python {
        1
    } else {
        count
    }
}

fn function_findings(
    node: Node<'_>,
    metric: &FunctionMetric,
    source: &str,
    settings: &AnalysisSettings,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    if metric.cyclomatic_complexity > settings.complexity_threshold {
        findings.push(function_finding(
            metric,
            "high-cyclomatic-complexity",
            if metric.cyclomatic_complexity > settings.extreme_complexity_threshold {
                Severity::Error
            } else {
                Severity::Warning
            },
            format!(
                "{} has cyclomatic complexity {} (threshold > {})",
                metric.name, metric.cyclomatic_complexity, settings.complexity_threshold
            ),
        ));
    }
    if metric.cognitive_complexity > settings.cognitive_threshold {
        findings.push(function_finding(
            metric,
            "high-cognitive-complexity",
            Severity::Warning,
            format!(
                "{} has cognitive complexity {} (threshold > {})",
                metric.name, metric.cognitive_complexity, settings.cognitive_threshold
            ),
        ));
    }
    if metric.sloc > settings.long_function_sloc {
        findings.push(function_finding(
            metric,
            "long-function",
            Severity::Warning,
            format!(
                "{} spans {} source lines (threshold > {})",
                metric.name, metric.sloc, settings.long_function_sloc
            ),
        ));
    }
    if metric.parameters > settings.max_parameters {
        findings.push(function_finding(
            metric,
            "large-parameter-list",
            Severity::Warning,
            format!(
                "{} has {} parameters (threshold > {})",
                metric.name, metric.parameters, settings.max_parameters
            ),
        ));
    }
    if metric.max_nesting > settings.max_nesting {
        findings.push(function_finding(
            metric,
            "deep-nesting",
            Severity::Warning,
            format!(
                "{} reaches nesting depth {} (threshold > {})",
                metric.name, metric.max_nesting, settings.max_nesting
            ),
        ));
    }
    if !metric.name.starts_with("<anonymous@") && is_trivial_wrapper(node, metric.language, source)
    {
        findings.push(Finding {
            rule_id: "trivial-wrapper".into(),
            category: FindingCategory::Verbosity,
            severity: Severity::Warning,
            path: metric.path.clone(),
            span: metric.span,
            message: format!("{} only forwards to another callable", metric.name),
            counts_toward_verbosity: true,
        });
    }
    findings
}

fn function_finding(
    metric: &FunctionMetric,
    rule_id: &str,
    severity: Severity,
    message: String,
) -> Finding {
    Finding {
        rule_id: rule_id.into(),
        category: if rule_id.contains("nesting") || rule_id.contains("parameter") {
            FindingCategory::Structure
        } else {
            FindingCategory::Complexity
        },
        severity,
        path: metric.path.clone(),
        span: metric.span,
        message,
        counts_toward_verbosity: false,
    }
}

fn is_trivial_wrapper(node: Node<'_>, language: Language, source: &str) -> bool {
    if matches!(
        node.kind(),
        "constructor_declaration" | "compact_constructor_declaration" | "secondary_constructor"
    ) {
        return false;
    }
    let Some(body) = function_body(node) else {
        return false;
    };
    if is_direct_call(body, language) {
        return !contains_decision(body, language);
    }
    let statements = if is_statement_container(body.kind()) {
        executable_children(body, language, source)
    } else {
        vec![body]
    };
    if statements.len() != 1 {
        return false;
    }
    let statement = statements[0];
    is_direct_call(statement, language) && !contains_decision(statement, language)
}

fn is_direct_call(node: Node<'_>, language: Language) -> bool {
    if matches!(
        node.kind(),
        "call"
            | "call_expression"
            | "method_invocation"
            | "object_creation_expression"
            | "delegation_call"
    ) {
        return true;
    }
    if !matches!(
        node.kind(),
        "expression_statement"
            | "return_statement"
            | "return_expression"
            | "parenthesized_expression"
            | "await"
            | "await_expression"
    ) {
        return false;
    }
    let mut cursor = node.walk();
    let mut children = node
        .named_children(&mut cursor)
        .filter(|child| !language.is_comment(child.kind()));
    let Some(child) = children.next() else {
        return false;
    };
    children.next().is_none() && is_direct_call(child, language)
}

fn contains_decision(node: Node<'_>, language: Language) -> bool {
    let root_id = node.id();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.id() != root_id && language.is_function(current.kind()) {
            continue;
        }
        if language.is_decision(current.kind()) {
            return true;
        }
        let mut cursor = current.walk();
        stack.extend(current.named_children(&mut cursor));
    }
    false
}

fn clone_candidates(
    path: &Path,
    language: Language,
    root: Node<'_>,
    source: &str,
    lines: &LineFacts,
    settings: &AnalysisSettings,
) -> Vec<CloneCandidate> {
    let mut hashes: HashMap<usize, blake3::Hash> = HashMap::new();
    let mut candidates = Vec::new();
    let mut stack = vec![(root, false)];

    while let Some((node, visited)) = stack.pop() {
        if !visited {
            stack.push((node, true));
            let mut cursor = node.walk();
            stack.extend(node.children(&mut cursor).map(|child| (child, false)));
            continue;
        }

        let hash = normalized_node_hash(node, language, source, &hashes);
        hashes.insert(node.id(), hash);

        if !node.is_named()
            || !language.is_clone_candidate(node.kind())
            || is_type_checking_block(node, source)
        {
            continue;
        }
        let span = node_span(node);
        let sloc = lines.count_sloc(span);
        if sloc < settings.min_clone_lines
            || max_body_statement_count(node, language, source) < settings.min_clone_statements
        {
            continue;
        }
        let sloc_lines = lines.sloc_in_span(span);
        let fingerprint = clone_fingerprint(hash, node, language, source);
        candidates.push(CloneCandidate {
            path: path.to_path_buf(),
            language,
            span,
            sloc_lines,
            fingerprint,
            preview: source_preview(source, span),
        });
    }

    candidates.sort_by_key(|candidate| (candidate.span.start_line, candidate.span.end_line));
    candidates
}

fn clone_fingerprint(
    structural_hash: blake3::Hash,
    root: Node<'_>,
    language: Language,
    source: &str,
) -> String {
    let mut identifiers: HashMap<&str, u32> = HashMap::new();
    let mut identity_hasher = Hasher::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if language.is_comment(node.kind())
            || (language == Language::Python && is_plain_string_statement(node, source))
        {
            continue;
        }
        if language.is_identifier(node.kind()) {
            let text = node_text(node, source);
            let next_id = identifiers.len() as u32 + 1;
            let id = *identifiers.entry(text).or_insert(next_id);
            identity_hasher.update(&id.to_le_bytes());
            continue;
        }
        let mut cursor = node.walk();
        let mut children: Vec<_> = node.children(&mut cursor).collect();
        children.reverse();
        stack.extend(children);
    }

    let mut combined = Hasher::new();
    combined.update(language.display_name().as_bytes());
    combined.update(structural_hash.as_bytes());
    combined.update(identity_hasher.finalize().as_bytes());
    combined.finalize().to_hex()[..16].to_owned()
}

fn normalized_node_hash(
    node: Node<'_>,
    language: Language,
    source: &str,
    hashes: &HashMap<usize, blake3::Hash>,
) -> blake3::Hash {
    let mut hasher = Hasher::new();
    if language.is_comment(node.kind()) {
        hasher.update(b"$COMMENT");
    } else if language.is_identifier(node.kind()) {
        hasher.update(b"$IDENT");
    } else if is_string_content(node.kind()) {
        hasher.update(b"$STRING");
    } else if language.is_literal(node.kind()) && !contains_interpolation(node) {
        hasher.update(literal_bucket(node.kind()).as_bytes());
    } else {
        hasher.update(node.kind().as_bytes());
        hasher.update(&[0]);
        let mut cursor = node.walk();
        let mut child_count = 0_u32;
        for child in node.children(&mut cursor) {
            if language.is_comment(child.kind())
                || (language == Language::Python && is_plain_string_statement(child, source))
            {
                continue;
            }
            if let Some(child_hash) = hashes.get(&child.id()) {
                hasher.update(child_hash.as_bytes());
                child_count += 1;
            }
        }
        if child_count == 0 {
            hasher.update(node_text(node, source).trim().as_bytes());
        }
    }
    hasher.finalize()
}

fn literal_bucket(kind: &str) -> &'static str {
    if kind.contains("string") || kind.contains("char") {
        "$STRING"
    } else if matches!(kind, "true" | "false") || kind.contains("boolean") {
        "$BOOL"
    } else if matches!(kind, "none" | "null" | "nil" | "undefined")
        || kind.contains("null")
        || kind.contains("none")
        || kind.contains("nil")
        || kind.contains("undefined")
    {
        "$NULL"
    } else {
        "$NUMBER"
    }
}

fn max_body_statement_count(node: Node<'_>, language: Language, source: &str) -> u32 {
    let mut containers = Vec::new();
    for field in ["body", "consequence", "alternative"] {
        if let Some(container) = node.child_by_field_name(field) {
            containers.push(container);
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "block" | "statement_block" | "function_body" | "suite" | "control_structure_body"
        ) && !containers
            .iter()
            .any(|container| container.id() == child.id())
        {
            containers.push(child);
        }
    }
    containers
        .into_iter()
        .map(|container| executable_child_count(container, language, source))
        .max()
        .unwrap_or(0)
}

fn executable_child_count(container: Node<'_>, language: Language, source: &str) -> u32 {
    if language.is_comment(container.kind())
        || (language == Language::Python && is_plain_string_statement(container, source))
    {
        0
    } else if is_statement_container(container.kind()) {
        executable_children(container, language, source).len() as u32
    } else {
        1
    }
}

fn executable_children<'tree>(
    container: Node<'tree>,
    language: Language,
    source: &str,
) -> Vec<Node<'tree>> {
    let mut cursor = container.walk();
    let children: Vec<_> = container
        .named_children(&mut cursor)
        .filter(|child| !language.is_comment(child.kind()))
        .filter(|child| {
            !(language == Language::Python && is_plain_string_statement(*child, source))
        })
        .collect();
    if children.len() == 1 && is_statement_container(children[0].kind()) {
        executable_children(children[0], language, source)
    } else {
        children
    }
}

fn is_statement_container(kind: &str) -> bool {
    matches!(
        kind,
        "block"
            | "statement_block"
            | "function_body"
            | "suite"
            | "control_structure_body"
            | "statement_list"
            | "statements"
            | "declarations"
    )
}

fn function_body(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("body").or_else(|| {
        let mut cursor = node.walk();
        node.named_children(&mut cursor).find(|child| {
            matches!(
                child.kind(),
                "block" | "statement_block" | "function_body" | "suite" | "control_structure_body"
            )
        })
    })
}

fn is_type_checking_block(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "if_statement" {
        return false;
    }
    node.child_by_field_name("condition")
        .map(|condition| {
            matches!(
                node_text(condition, source).trim(),
                "TYPE_CHECKING" | "typing.TYPE_CHECKING"
            )
        })
        .unwrap_or(false)
}

fn source_preview(source: &str, span: Span) -> String {
    source
        .lines()
        .skip(span.start_line.saturating_sub(1) as usize)
        .take(3)
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(120)
        .collect()
}

fn finding_sloc_lines(findings: &[Finding], sloc: &BTreeSet<u32>) -> BTreeSet<u32> {
    let mut lines = BTreeSet::new();
    for finding in findings
        .iter()
        .filter(|finding| finding.counts_toward_verbosity)
    {
        lines.extend(
            sloc.range(finding.span.start_line..=finding.span.end_line)
                .copied(),
        );
    }
    lines
}

fn finding_physical_lines(findings: &[Finding], loc: usize) -> BTreeSet<u32> {
    let mut lines = BTreeSet::new();
    for finding in findings
        .iter()
        .filter(|finding| finding.counts_toward_verbosity)
    {
        let end = finding.span.end_line.min(loc as u32);
        if finding.span.start_line <= end {
            lines.extend(finding.span.start_line..=end);
        }
    }
    lines
}

fn build_report(
    mut analyses: Vec<FileAnalysis>,
    mut skipped_files: Vec<SkippedFile>,
    settings: AnalysisSettings,
    elapsed_ms: u64,
) -> AnalysisReport {
    analyses.sort_by(|left, right| left.metrics.path.cmp(&right.metrics.path));
    skipped_files.sort_by(|left, right| left.path.cmp(&right.path));

    let clone_groups = build_clone_groups(&analyses);
    let mut duplicated_by_file: BTreeMap<PathBuf, BTreeSet<u32>> = BTreeMap::new();
    let mut excess_duplicated_by_file: BTreeMap<PathBuf, BTreeSet<u32>> = BTreeMap::new();
    let mut duplicate_physical_by_file: BTreeMap<PathBuf, BTreeSet<u32>> = BTreeMap::new();
    for group in &clone_groups {
        for (instance_index, instance) in group.instances.iter().enumerate() {
            let lines = analyses
                .iter()
                .find(|analysis| analysis.metrics.path == instance.path)
                .map(|analysis| &analysis.sloc_lines);
            if let Some(sloc) = lines {
                let duplicated_lines = sloc
                    .range(instance.span.start_line..=instance.span.end_line)
                    .copied();
                duplicated_by_file
                    .entry(instance.path.clone())
                    .or_default()
                    .extend(duplicated_lines.clone());
                if instance_index > 0 {
                    excess_duplicated_by_file
                        .entry(instance.path.clone())
                        .or_default()
                        .extend(duplicated_lines);
                }
            }
            duplicate_physical_by_file
                .entry(instance.path.clone())
                .or_default()
                .extend(instance.span.start_line..=instance.span.end_line);
        }
    }

    let mut summary = Summary {
        files_failed: skipped_files.len() as u64,
        analysis_time_ms: elapsed_ms,
        ..Summary::default()
    };
    let mut languages: BTreeMap<Language, LanguageSummary> = BTreeMap::new();
    let mut files = Vec::new();
    let mut functions = Vec::new();
    let mut findings = Vec::new();

    for mut analysis in analyses {
        let duplicate_lines = duplicated_by_file
            .remove(&analysis.metrics.path)
            .unwrap_or_default();
        let duplicate_physical = duplicate_physical_by_file
            .remove(&analysis.metrics.path)
            .unwrap_or_default();
        let excess_duplicate_lines = excess_duplicated_by_file
            .remove(&analysis.metrics.path)
            .unwrap_or_default();
        let verbosity_lines: BTreeSet<u32> = analysis
            .rule_lines
            .union(&duplicate_lines)
            .copied()
            .collect();
        let blog_lines: BTreeSet<u32> = analysis
            .rule_physical_lines
            .union(&duplicate_physical)
            .copied()
            .filter(|line| *line <= analysis.metrics.loc as u32)
            .collect();

        analysis.metrics.duplicated_sloc_lines = duplicate_lines.len() as u64;
        analysis.metrics.excess_duplicate_sloc_lines = excess_duplicate_lines.len() as u64;
        analysis.metrics.verbosity_flagged_sloc_lines = verbosity_lines.len() as u64;

        summary.files_scanned += 1;
        summary.loc += analysis.metrics.loc;
        summary.sloc += analysis.metrics.sloc;
        summary.blank_lines += analysis.metrics.blank_lines;
        summary.comment_lines += analysis.metrics.comment_lines;
        summary.comment_only_lines += analysis.metrics.comment_only_lines;
        summary.syntax_nodes += analysis.metrics.syntax_nodes;
        summary.parse_errors += analysis.metrics.parse_errors;
        summary.total_functions += analysis.metrics.functions;
        summary.cloned_sloc_lines += duplicate_lines.len() as u64;
        summary.excess_duplicate_sloc_lines += excess_duplicate_lines.len() as u64;
        summary.rule_flagged_sloc_lines += analysis.rule_lines.len() as u64;
        summary.verbosity_flagged_sloc_lines += verbosity_lines.len() as u64;
        summary.blog_verbosity_flagged_lines += blog_lines.len() as u64;
        summary.debt_markers += analysis.metrics.debt_markers;

        let language_summary = languages
            .entry(analysis.metrics.language)
            .or_insert_with(|| LanguageSummary {
                rule_coverage: "extended_local_rules",
                rules_available: LINE_RULES
                    .iter()
                    .filter(|rule| rule.applies_to(analysis.metrics.language))
                    .count() as u64
                    + 1,
                ..LanguageSummary::default()
            });
        language_summary.files += 1;
        language_summary.loc += analysis.metrics.loc;
        language_summary.sloc += analysis.metrics.sloc;
        language_summary.comment_lines += analysis.metrics.comment_lines;
        language_summary.functions += analysis.metrics.functions;
        language_summary.syntax_nodes += analysis.metrics.syntax_nodes;
        language_summary.parse_errors += analysis.metrics.parse_errors;
        language_summary.duplicate_sloc_lines += duplicate_lines.len() as u64;
        language_summary.excess_duplicate_sloc_lines += excess_duplicate_lines.len() as u64;
        language_summary.rule_flagged_sloc_lines += analysis.rule_lines.len() as u64;
        language_summary.verbosity_flagged_sloc_lines += verbosity_lines.len() as u64;
        language_summary.rule_findings += analysis
            .findings
            .iter()
            .filter(|finding| finding.counts_toward_verbosity)
            .count() as u64;

        functions.append(&mut analysis.functions);
        findings.append(&mut analysis.findings);
        files.push(analysis.metrics);
    }

    functions.sort_by(|left, right| {
        (&left.path, left.span.start_line, left.name.as_str()).cmp(&(
            &right.path,
            right.span.start_line,
            right.name.as_str(),
        ))
    });
    findings.sort_by(|left, right| {
        (&left.path, left.span.start_line, left.rule_id.as_str()).cmp(&(
            &right.path,
            right.span.start_line,
            right.rule_id.as_str(),
        ))
    });

    populate_complexity_summary(&mut summary, &functions, &settings);
    summary.clone_groups = clone_groups.len() as u64;
    summary.clone_instances = clone_groups
        .iter()
        .map(|group| group.instances.len() as u64)
        .sum();
    summary.rule_findings = findings
        .iter()
        .filter(|finding| finding.counts_toward_verbosity)
        .count() as u64;
    summary.comment_density = ratio(summary.comment_lines, summary.loc);
    summary.clone_ratio = ratio(summary.cloned_sloc_lines, summary.sloc);
    summary.excess_clone_ratio = ratio(summary.excess_duplicate_sloc_lines, summary.sloc);
    summary.rule_flagged_ratio = ratio(summary.rule_flagged_sloc_lines, summary.sloc);
    summary.verbosity = ratio(summary.verbosity_flagged_sloc_lines, summary.sloc);
    summary.blog_verbosity = ratio(summary.blog_verbosity_flagged_lines, summary.loc);

    AnalysisReport {
        metric_version: METRIC_VERSION,
        settings,
        summary,
        languages,
        files,
        functions,
        clone_groups,
        findings,
        skipped_files,
        delta: None,
    }
}

fn build_clone_groups(analyses: &[FileAnalysis]) -> Vec<CloneGroup> {
    let mut grouped: BTreeMap<(Language, String), Vec<&CloneCandidate>> = BTreeMap::new();
    for candidate in analyses
        .iter()
        .flat_map(|analysis| analysis.clone_candidates.iter())
    {
        grouped
            .entry((candidate.language, candidate.fingerprint.clone()))
            .or_default()
            .push(candidate);
    }

    let mut groups = Vec::new();
    for ((language, fingerprint), mut candidates) in grouped {
        candidates.sort_by_key(|candidate| {
            (
                candidate.path.clone(),
                candidate.span.start_line,
                candidate.span.end_line,
            )
        });
        candidates.dedup_by_key(|candidate| {
            (
                candidate.path.clone(),
                candidate.span.start_line,
                candidate.span.end_line,
            )
        });
        if candidates.len() < 2 {
            continue;
        }
        let instances: Vec<_> = candidates
            .into_iter()
            .map(|candidate| CloneInstance {
                path: candidate.path.clone(),
                span: candidate.span,
                sloc: candidate.sloc_lines.len() as u32,
                preview: candidate.preview.clone(),
            })
            .collect();
        let total_sloc = instances.iter().map(|instance| instance.sloc).sum();
        groups.push(CloneGroup {
            fingerprint,
            language,
            instances,
            total_sloc,
        });
    }
    groups.sort_by(|left, right| {
        right
            .total_sloc
            .cmp(&left.total_sloc)
            .then_with(|| left.language.cmp(&right.language))
            .then_with(|| left.fingerprint.cmp(&right.fingerprint))
    });
    groups
}

fn populate_complexity_summary(
    summary: &mut Summary,
    functions: &[FunctionMetric],
    settings: &AnalysisSettings,
) {
    if functions.is_empty() {
        return;
    }
    let cc: Vec<f64> = functions
        .iter()
        .map(|function| function.cyclomatic_complexity as f64)
        .collect();
    let cc_sum: f64 = cc.iter().sum();
    summary.cc_max = functions
        .iter()
        .map(|function| function.cyclomatic_complexity)
        .max()
        .unwrap_or(0);
    summary.cc_mean = cc_sum / functions.len() as f64;
    summary.cc_stddev = (cc
        .iter()
        .map(|value| (value - summary.cc_mean).powi(2))
        .sum::<f64>()
        / functions.len() as f64)
        .sqrt();
    summary.cc_high_count = functions
        .iter()
        .filter(|function| function.cyclomatic_complexity > settings.complexity_threshold)
        .count() as u64;
    summary.cc_extreme_count = functions
        .iter()
        .filter(|function| function.cyclomatic_complexity > settings.extreme_complexity_threshold)
        .count() as u64;
    summary.cc_concentration = gini(&cc);
    summary.total_mass = functions.iter().map(|function| function.mass).sum();
    summary.high_cc_mass = functions
        .iter()
        .filter(|function| function.cyclomatic_complexity > settings.complexity_threshold)
        .map(|function| function.mass)
        .sum();
    summary.erosion = float_ratio(summary.high_cc_mass, summary.total_mass);
    summary.cognitive_max = functions
        .iter()
        .map(|function| function.cognitive_complexity)
        .max()
        .unwrap_or(0);
    summary.total_cognitive_mass = functions
        .iter()
        .map(|function| function.cognitive_mass)
        .sum();
    summary.high_cognitive_mass = functions
        .iter()
        .filter(|function| function.cognitive_complexity > settings.cognitive_threshold)
        .map(|function| function.cognitive_mass)
        .sum();
    summary.cognitive_erosion =
        float_ratio(summary.high_cognitive_mass, summary.total_cognitive_mass);
}

fn gini(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let sum: f64 = sorted.iter().sum();
    if sum == 0.0 {
        return 0.0;
    }
    let weighted: f64 = sorted
        .iter()
        .enumerate()
        .map(|(index, value)| (index + 1) as f64 * value)
        .sum();
    (2.0 * weighted) / (sorted.len() as f64 * sum)
        - (sorted.len() as f64 + 1.0) / sorted.len() as f64
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn float_ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

fn node_span(node: Node<'_>) -> Span {
    point_span(node.start_position(), node.end_position())
}

fn point_span(start: Point, end: Point) -> Span {
    let start_line = start.row + 1;
    let end_row = effective_end_row(start, end);
    Span::new(start_line, end_row + 1)
}

fn node_text<'source>(node: Node<'_>, source: &'source str) -> &'source str {
    source.get(node.byte_range()).unwrap_or_default()
}

fn strip_comment_prefix(line: &str) -> &str {
    line.trim_start()
        .trim_start_matches("//")
        .trim_start_matches('#')
        .trim_start_matches("/*")
        .trim_start_matches('*')
        .trim_start()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{analyze_files, gini};
    use crate::model::AnalysisSettings;

    #[test]
    fn empty_gini_is_zero() {
        assert_eq!(gini(&[]), 0.0);
        assert_eq!(gini(&[1.0, 1.0, 1.0]), 0.0);
    }

    #[test]
    fn analyzes_python_lines_complexity_rules_and_comments() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("sample.py");
        fs::write(
            &path,
            "# TODO: simplify\n\ndef choose(items):\n    if len(items) > 0:\n        for i in range(len(items)):\n            if items[i]:\n                return True\n    return False\n",
        )
        .unwrap();

        let report = analyze_files(&[path], &AnalysisSettings::default());
        assert_eq!(report.summary.files_scanned, 1);
        assert_eq!(report.summary.loc, 8);
        assert_eq!(report.summary.comment_lines, 1);
        assert_eq!(report.summary.debt_markers, 1);
        assert_eq!(report.functions.len(), 1);
        assert!(report.functions[0].cyclomatic_complexity >= 4);
        assert!(report.summary.rule_flagged_sloc_lines >= 2);
    }

    #[test]
    fn finds_identifier_normalized_clones_across_files() {
        let directory = tempdir().unwrap();
        let first = directory.path().join("first.rs");
        let second = directory.path().join("second.rs");
        fs::write(
            &first,
            "fn first(x: i32) -> i32 {\n let y = x + 1;\n let z = y * 2;\n z\n}\n",
        )
        .unwrap();
        fs::write(
            &second,
            "fn second(a: i32) -> i32 {\n let b = a + 9;\n let c = b * 7;\n c\n}\n",
        )
        .unwrap();

        let report = analyze_files(&[first, second], &AnalysisSettings::default());
        assert!(!report.clone_groups.is_empty());
        assert!(report.summary.cloned_sloc_lines > 0);
    }
}

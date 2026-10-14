use std::fs;
use std::path::{Path, PathBuf};

use scbr::analysis::analyze_files;
use scbr::model::{AnalysisReport, AnalysisSettings, Span};
use tempfile::TempDir;

fn write_fixture(root: &Path, name: &str, source: &str) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, source).expect("write analyzer fixture");
    path
}

fn analyze_fixture(name: &str, source: &str) -> AnalysisReport {
    let directory = TempDir::new().expect("create analyzer fixture directory");
    let path = write_fixture(directory.path(), name, source);
    analyze_files(&[path], &AnalysisSettings::default())
}

fn has_clone_with_spans(report: &AnalysisReport, expected: &[Span]) -> bool {
    report.clone_groups.iter().any(|group| {
        expected.iter().all(|span| {
            group
                .instances
                .iter()
                .any(|instance| instance.span == *span)
        })
    })
}

#[test]
fn empty_and_crlf_sources_have_exact_line_counts() {
    let directory = TempDir::new().expect("create fixture directory");
    let empty = write_fixture(directory.path(), "empty.rs", "");
    let crlf = write_fixture(
        directory.path(),
        "crlf.rs",
        "// heading\r\n\r\nfn value() -> i32 {\r\n    1\r\n}\r\n",
    );

    let report = analyze_files(&[empty, crlf], &AnalysisSettings::default());
    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );

    let empty = report
        .files
        .iter()
        .find(|file| file.path.ends_with("empty.rs"))
        .expect("empty file metrics");
    assert_eq!(empty.loc, 0);
    assert_eq!(empty.sloc, 0);
    assert_eq!(empty.blank_lines, 0);
    assert_eq!(empty.comment_lines, 0);
    assert_eq!(empty.comment_only_lines, 0);

    let crlf = report
        .files
        .iter()
        .find(|file| file.path.ends_with("crlf.rs"))
        .expect("CRLF file metrics");
    assert_eq!(crlf.loc, 5, "CRLF must not create phantom physical lines");
    assert_eq!(crlf.sloc, 2, "delimiter-only lines are not SLOC");
    assert_eq!(crlf.blank_lines, 1);
    assert_eq!(crlf.comment_lines, 1);
    assert_eq!(crlf.comment_only_lines, 1);
}

#[test]
fn punctuation_expressions_are_sloc_even_when_they_resemble_delimiters() {
    let report = analyze_fixture(
        "unit.rs",
        r#"fn explicit_unit() -> () {
    ()
}
"#,
    );

    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    assert_eq!(report.files[0].loc, 3);
    assert_eq!(
        report.files[0].sloc, 2,
        "the unit expression has semantics and is not a delimiter line"
    );
    assert_eq!(report.functions[0].sloc, 2);
}

#[test]
fn multiline_comments_distinguish_inline_code_from_comment_only_lines() {
    let report = analyze_fixture(
        "comments.js",
        concat!(
            "const first = 1; /* opening\r\n",
            " * body\r\n",
            " */ const second = 2;\r\n",
            "// standalone\r\n",
        ),
    );

    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    let file = &report.files[0];
    assert_eq!(file.loc, 4);
    assert_eq!(
        file.sloc, 2,
        "inline code on comment boundary lines is SLOC"
    );
    assert_eq!(file.blank_lines, 0);
    assert_eq!(file.comment_lines, 4);
    assert_eq!(
        file.comment_only_lines, 2,
        "only the block-comment body and standalone comment lack code"
    );
}

#[test]
fn verbosity_rules_ignore_examples_inside_string_literals() {
    let directory = TempDir::new().expect("create fixture directory");
    let javascript = write_fixture(
        directory.path(),
        "strings.js",
        "const explanation = \"Prefer active over ready == true\";\n",
    );
    let python = write_fixture(
        directory.path(),
        "strings.py",
        "example = 'for i in range(len(items)):'\n",
    );

    let report = analyze_files(&[javascript, python], &AnalysisSettings::default());
    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    assert_eq!(
        report.summary.rule_findings, 0,
        "text examples are data, not verbose syntax: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.rule_flagged_sloc_lines, 0);
}

#[test]
fn python_byte_and_formatted_string_statements_remain_sloc() {
    let report = analyze_fixture(
        "string_prefixes.py",
        "def render(value):\n    rb\"payload\"\n    rf\"{value}\"\n    return value\n",
    );

    assert_eq!(report.summary.sloc, 4);
    assert_eq!(report.functions[0].sloc, 4);
}

#[test]
fn javascript_shebang_is_metadata_instead_of_source_code() {
    let report = analyze_fixture(
        "script.js",
        "#!/usr/bin/env node\nfunction main() {\n  return 0;\n}\n",
    );

    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    let file = &report.files[0];
    assert_eq!(file.loc, 4);
    assert_eq!(file.sloc, 2, "the interpreter directive is not source code");
    assert_eq!(file.comment_lines, 1);
    assert_eq!(file.comment_only_lines, 1);
}

#[test]
fn operator_changes_do_not_form_normalized_clone_groups() {
    let report = analyze_fixture(
        "operators.rs",
        r#"fn add_path(value: i32) -> i32 {
    if value > 0 {
        let changed = value + 1;
        return changed + 2;
    }
    value + 3
}

fn subtract_path(input: i32) -> i32 {
    if input > 0 {
        let changed = input - 1;
        return changed - 2;
    }
    input - 3
}
"#,
    );

    let spans: Vec<_> = report
        .functions
        .iter()
        .map(|function| function.span)
        .collect();
    assert_eq!(spans.len(), 2, "{:#?}", report.functions);
    assert!(
        !has_clone_with_spans(&report, &spans),
        "operators are semantic structure and must survive normalization: {:#?}",
        report.clone_groups
    );
}

#[test]
fn null_and_numeric_literals_use_distinct_clone_buckets() {
    let report = analyze_fixture(
        "literal_buckets.java",
        r#"class LiteralBuckets {
    Object absent() {
        Object value = null;
        consume(value);
        return value;
    }

    Object zero() {
        Object value = 0;
        consume(value);
        return value;
    }
}
"#,
    );
    let spans: Vec<_> = report
        .functions
        .iter()
        .map(|function| function.span)
        .collect();
    assert_eq!(spans.len(), 2);
    assert!(!has_clone_with_spans(&report, &spans));
}

#[test]
fn operators_inside_template_interpolations_remain_clone_significant() {
    let report = analyze_fixture(
        "templates.js",
        r#"function renderAdd(value) {
    const text = `result ${value + 1}`;
    return text;
}

function renderSubtract(input) {
    const output = `result ${input - 1}`;
    return output;
}
"#,
    );

    let spans: Vec<_> = report
        .functions
        .iter()
        .map(|function| function.span)
        .collect();
    assert_eq!(spans.len(), 2, "{:#?}", report.functions);
    assert!(
        !has_clone_with_spans(&report, &spans),
        "normalization must retain operators inside interpolated literals: {:#?}",
        report.clone_groups
    );
}

#[test]
fn operators_inside_python_f_strings_remain_clone_significant() {
    let report = analyze_fixture(
        "formatted.py",
        r#"def render_add(value):
    text = f"result {value + 1}"
    return text

def render_subtract(input):
    output = f"result {input - 1}"
    return output
"#,
    );

    let spans: Vec<_> = report
        .functions
        .iter()
        .map(|function| function.span)
        .collect();
    assert_eq!(spans.len(), 2, "{:#?}", report.functions);
    assert!(
        !has_clone_with_spans(&report, &spans),
        "normalization must retain operators inside formatted strings: {:#?}",
        report.clone_groups
    );
}

#[test]
fn operators_inside_kotlin_string_templates_remain_clone_significant() {
    let report = analyze_fixture(
        "templates.kt",
        r#"fun renderAdd(value: Int): String {
    val text = "result ${value + 1}"
    return text
}

fun renderSubtract(input: Int): String {
    val output = "result ${input - 1}"
    return output
}
"#,
    );

    let spans: Vec<_> = report
        .functions
        .iter()
        .map(|function| function.span)
        .collect();
    assert_eq!(spans.len(), 2, "{:#?}", report.functions);
    assert!(
        !has_clone_with_spans(&report, &spans),
        "normalization must retain operators inside Kotlin string templates: {:#?}",
        report.clone_groups
    );
}

#[test]
fn different_identifier_relationships_are_not_alpha_equivalent_clones() {
    let report = analyze_fixture(
        "bindings.rs",
        r#"fn preserve_first(first: i32, second: i32) -> i32 {
    let merged = first + second;
    emit(merged, first);
    finish(merged)
}

fn preserve_second(left: i32, right: i32) -> i32 {
    let result = left + right;
    emit(result, right);
    finish(result)
}
"#,
    );

    let spans: Vec<_> = report
        .functions
        .iter()
        .map(|function| function.span)
        .collect();
    assert_eq!(spans.len(), 2, "{:#?}", report.functions);
    assert!(
        !has_clone_with_spans(&report, &spans),
        "changing which binding is used changes behavior: {:#?}",
        report.clone_groups
    );
}

#[test]
fn overlapping_clone_and_rule_spans_are_unioned_once() {
    let report = analyze_fixture(
        "overlap.py",
        r#"def first(items):
    if len(items) > 0:
        value = items.get("key", None)
        return value
    return None

def second(things):
    if len(things) > 0:
        result = things.get("other", None)
        return result
    return None
"#,
    );

    assert_eq!(report.summary.sloc, 10);
    assert_eq!(report.summary.rule_flagged_sloc_lines, 4);
    assert_eq!(report.summary.cloned_sloc_lines, 10);
    assert_eq!(
        report.summary.excess_duplicate_sloc_lines, 5,
        "one canonical copy of each clone group is retained"
    );
    assert!((report.summary.excess_clone_ratio - 0.5).abs() < f64::EPSILON);
    assert_eq!(
        report.summary.verbosity_flagged_sloc_lines, 10,
        "rule lines already covered by clones must not be added twice"
    );
    assert_eq!(report.summary.blog_verbosity_flagged_lines, 10);
    assert!((report.summary.verbosity - 1.0).abs() < f64::EPSILON);
    assert_eq!(report.files[0].duplicated_sloc_lines, 10);
    assert_eq!(report.files[0].excess_duplicate_sloc_lines, 5);
    assert_eq!(report.files[0].verbosity_flagged_sloc_lines, 10);
}

#[test]
fn excess_duplicate_accounting_retains_the_first_sorted_instance() {
    let directory = TempDir::new().expect("create fixture directory");
    let first = write_fixture(
        directory.path(),
        "a.py",
        r#"def total(values):
    result = 0
    for value in values:
        result += value
    return result
"#,
    );
    let second = write_fixture(
        directory.path(),
        "b.py",
        r#"def sum_items(items):
    answer = 0
    for item in items:
        answer += item
    return answer
"#,
    );

    let report = analyze_files(&[second, first], &AnalysisSettings::default());
    assert_eq!(report.summary.cloned_sloc_lines, 10);
    assert_eq!(report.summary.excess_duplicate_sloc_lines, 5);

    let canonical = report
        .files
        .iter()
        .find(|file| file.path.ends_with("a.py"))
        .expect("canonical file metrics");
    let repeated = report
        .files
        .iter()
        .find(|file| file.path.ends_with("b.py"))
        .expect("repeated file metrics");
    assert_eq!(canonical.duplicated_sloc_lines, 5);
    assert_eq!(canonical.excess_duplicate_sloc_lines, 0);
    assert_eq!(repeated.duplicated_sloc_lines, 5);
    assert_eq!(repeated.excess_duplicate_sloc_lines, 5);
}

fn python_function(name: &str, branches: usize) -> String {
    let mut source = format!("def {name}(value):\n");
    for branch in 0..branches {
        source.push_str(&format!("    if value == {branch}:\n        value += 1\n"));
    }
    source.push_str("    return value\n");
    source
}

#[test]
fn erosion_uses_a_strictly_greater_than_ten_threshold_and_mass_formula() {
    let source = format!(
        "{}\n{}",
        python_function("at_threshold", 9),
        python_function("above_threshold", 10)
    );
    let report = analyze_fixture("erosion.py", &source);

    let at_threshold = report
        .functions
        .iter()
        .find(|function| function.name == "at_threshold")
        .expect("function at threshold");
    let above_threshold = report
        .functions
        .iter()
        .find(|function| function.name == "above_threshold")
        .expect("function above threshold");
    assert_eq!(at_threshold.cyclomatic_complexity, 10);
    assert_eq!(above_threshold.cyclomatic_complexity, 11);
    assert_eq!(report.summary.cc_high_count, 1, "CC == 10 is not high");

    let expected_high_mass = 11.0 * (above_threshold.sloc as f64).sqrt();
    let expected_total_mass = 10.0 * (at_threshold.sloc as f64).sqrt() + expected_high_mass;
    assert!((report.summary.high_cc_mass - expected_high_mass).abs() < 1e-10);
    assert!((report.summary.total_mass - expected_total_mass).abs() < 1e-10);
    assert!((report.summary.erosion - expected_high_mass / expected_total_mass).abs() < 1e-12);
}

#[test]
fn nested_function_decisions_do_not_inflate_the_outer_function() {
    let report = analyze_fixture(
        "nested.py",
        r#"def outer(value):
    def inner(item):
        if item > 0:
            if item > 1:
                return item
        return 0
    if value:
        return inner(value)
    return 0
"#,
    );

    assert_eq!(report.functions.len(), 2, "{:#?}", report.functions);
    let outer = report
        .functions
        .iter()
        .find(|function| function.name == "outer")
        .expect("outer function");
    let inner = report
        .functions
        .iter()
        .find(|function| function.name == "inner")
        .expect("inner function");
    assert_eq!(outer.cyclomatic_complexity, 2);
    assert_eq!(inner.cyclomatic_complexity, 3);
    assert_eq!(
        outer.sloc, 4,
        "nested function lines belong to the inner metric"
    );
    assert_eq!(inner.sloc, 5);
    assert_eq!(
        outer.sloc as u64 + inner.sloc as u64,
        report.summary.sloc,
        "nested function SLOC must not be charged to both masses"
    );
}

#[test]
fn assigned_python_lambda_has_its_own_name_complexity_and_sloc() {
    let report = analyze_fixture(
        "lambda.py",
        r#"def outer():
    chooser = lambda value: value if value > 0 else 0
    return chooser
"#,
    );

    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    assert_eq!(report.summary.sloc, 3);
    assert_eq!(report.functions.len(), 2, "{:#?}", report.functions);
    let outer = report
        .functions
        .iter()
        .find(|function| function.name == "outer")
        .expect("outer function");
    let chooser = report
        .functions
        .iter()
        .find(|function| function.name == "chooser")
        .expect("assigned lambda metric");
    assert_eq!(outer.cyclomatic_complexity, 1);
    assert_eq!(chooser.cyclomatic_complexity, 2);
    assert_eq!(outer.sloc, 2);
    assert_eq!(chooser.sloc, 1);
    assert_eq!(outer.sloc as u64 + chooser.sloc as u64, report.summary.sloc);
}

#[test]
fn counts_top_level_parameters_across_all_supported_grammars() {
    let directory = TempDir::new().expect("create fixture directory");
    let fixtures = [
        write_fixture(
            directory.path(),
            "parameters.py",
            r#"def python_params(positional, /, normal=1, *items, keyword, **options):
    return positional
"#,
        ),
        write_fixture(
            directory.path(),
            "parameters.js",
            r#"function javascriptParams({ first, second }, [third], regular = 1, ...rest) {
    return regular;
}
"#,
        ),
        write_fixture(
            directory.path(),
            "parameters.ts",
            r#"type Pair = { first: number; second: number };
function typescriptParams({ first, second }: Pair, regular: number = 1, ...rest: string[]): number {
    return regular;
}
"#,
        ),
        write_fixture(
            directory.path(),
            "parameters.go",
            r#"package fixture

func goParams(first, second int, values ...string) int {
    return first + second + len(values)
}
"#,
        ),
        write_fixture(
            directory.path(),
            "parameters.rs",
            r#"struct Counter;
impl Counter {
    fn rust_params(&self, first: i32, mut second: i32, (left, right): (i32, i32)) -> i32 {
        second += left + right;
        first + second
    }
}
"#,
        ),
        write_fixture(
            directory.path(),
            "Parameters.java",
            r#"class Parameters {
    static int javaParams(int first, @Deprecated String second, int... rest) {
        return first + second.length() + rest.length;
    }
}
"#,
        ),
        write_fixture(
            directory.path(),
            "parameters.kt",
            r#"fun kotlinParams(first: Int, second: String = "x", vararg rest: Long): Int {
    return first + second.length + rest.size
}
"#,
        ),
    ];

    let report = analyze_files(&fixtures, &AnalysisSettings::default());
    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    assert_eq!(report.summary.parse_errors, 0, "{:#?}", report.findings);
    let expected = [
        ("python_params", 5),
        ("javascriptParams", 4),
        ("typescriptParams", 3),
        ("goParams", 3),
        ("rust_params", 4),
        ("javaParams", 3),
        ("kotlinParams", 3),
    ];
    for (name, count) in expected {
        let function = report
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| panic!("missing {name}: {:#?}", report.functions));
        assert_eq!(
            function.parameters, count,
            "wrong top-level parameter count for {name}: {function:#?}"
        );
    }
}

#[test]
fn inferred_java_lambda_and_untyped_rust_closure_parameters_are_counted() {
    let directory = TempDir::new().expect("create fixture directory");
    let java = write_fixture(
        directory.path(),
        "Lambdas.java",
        r#"class Lambdas {
    java.util.function.BiFunction<Integer, Integer, Integer> combiner = (x, y) -> x + y;
}
"#,
    );
    let rust = write_fixture(
        directory.path(),
        "closures.rs",
        "fn build() {\n    let combine = |x, mut y, (a, b)| x + y + a + b;\n}\n",
    );
    let report = analyze_files(&[java, rust], &AnalysisSettings::default());
    assert_eq!(report.summary.parse_errors, 0);
    let combiner = report
        .functions
        .iter()
        .find(|function| function.name == "combiner")
        .expect("Java inferred lambda");
    let combine = report
        .functions
        .iter()
        .find(|function| function.name == "combine")
        .expect("Rust untyped closure");
    assert_eq!(combiner.parameters, 2);
    assert_eq!(combine.parameters, 3);
}

#[test]
fn python_function_sloc_includes_decorators() {
    let report = analyze_fixture(
        "decorated.py",
        r#"@registered("worker")
def run():
    return 1
"#,
    );

    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    assert_eq!(report.summary.sloc, 3);
    assert_eq!(report.functions.len(), 1, "{:#?}", report.functions);
    assert_eq!(
        report.functions[0].sloc, 3,
        "decorators execute during definition and belong to the function declaration"
    );
    assert_eq!(report.functions[0].span.start_line, 1);
}

#[test]
fn nested_decorated_function_sloc_is_not_charged_to_the_outer_function() {
    let report = analyze_fixture(
        "nested_decorated.py",
        r#"def outer():
    @registered
    def inner():
        return 1
    return inner()
"#,
    );

    assert_eq!(report.summary.sloc, 5);
    assert_eq!(report.functions.len(), 2, "{:#?}", report.functions);
    let outer = report
        .functions
        .iter()
        .find(|function| function.name == "outer")
        .expect("outer function");
    let inner = report
        .functions
        .iter()
        .find(|function| function.name == "inner")
        .expect("inner function");
    assert_eq!(outer.sloc, 2);
    assert_eq!(inner.sloc, 3);
    assert_eq!(outer.sloc as u64 + inner.sloc as u64, report.summary.sloc);
}

#[test]
fn calls_inside_a_returned_closure_do_not_make_the_outer_function_a_wrapper() {
    let report = analyze_fixture(
        "factory.js",
        r#"function makeWorker() {
    return () => performWork();
}

"#,
    );

    assert_eq!(report.functions.len(), 2, "{:#?}", report.functions);
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.rule_id != "trivial-wrapper"),
        "the outer function constructs a callback rather than forwarding a call: {:#?}",
        report.findings
    );
}

#[test]
fn transformed_call_results_are_not_trivial_wrappers() {
    let report = analyze_fixture(
        "transform.js",
        "function normalize(input) {\n  return parseInt(input, 10) * 2;\n}\n",
    );

    assert!(report.findings.iter().all(|finding| {
        !(finding.rule_id == "trivial-wrapper" && finding.span.start_line == 1)
    }));
    assert_eq!(report.summary.rule_flagged_sloc_lines, 0);
}

#[test]
fn property_access_after_a_kotlin_call_is_not_a_trivial_wrapper() {
    let report = analyze_fixture(
        "transform.kt",
        "fun normalize(input: String): Int {\n    return parse(input).value\n}\n",
    );
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.rule_id != "trivial-wrapper")
    );
}

#[test]
fn direct_call_results_are_trivial_wrappers() {
    let report = analyze_fixture(
        "forward.js",
        "function forward(input) {\n  return parseInt(input, 10);\n}\n",
    );
    assert!(report.findings.iter().any(|finding| {
        finding.rule_id == "trivial-wrapper" && finding.counts_toward_verbosity
    }));
}

#[test]
fn expression_bodied_functions_count_as_one_clone_statement() {
    let directory = TempDir::new().expect("create fixture directory");
    let path = write_fixture(
        directory.path(),
        "arrows.js",
        "const first = value => normalize(value);\nconst second = input => normalize(input);\n",
    );
    let settings = AnalysisSettings {
        min_clone_lines: 1,
        min_clone_statements: 1,
        ..AnalysisSettings::default()
    };
    let report = analyze_files(&[path], &settings);
    assert_eq!(report.functions.len(), 2);
    assert!(report.clone_groups.iter().any(|group| {
        group.instances.len() == 2 && group.instances.iter().all(|instance| instance.sloc == 1)
    }));
}

#[test]
fn same_line_nested_functions_do_not_give_the_outer_function_zero_mass() {
    let report = analyze_fixture(
        "same_line.js",
        "function makeWorker() { return () => performWork(); }\n",
    );

    assert_eq!(report.summary.sloc, 1);
    assert_eq!(report.functions.len(), 2, "{:#?}", report.functions);
    let outer = report
        .functions
        .iter()
        .find(|function| function.name == "makeWorker")
        .expect("outer function");
    assert_eq!(
        outer.sloc, 1,
        "a real function must retain source mass when its nested closure shares the line"
    );
    assert!(outer.mass > 0.0);
}

#[test]
fn rust_if_let_is_one_control_flow_decision() {
    let report = analyze_fixture(
        "if_let.rs",
        r#"fn unwrap_or_zero(value: Option<i32>) -> i32 {
    if let Some(number) = value {
        return number;
    }
    0
}
"#,
    );

    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    assert_eq!(report.functions.len(), 1, "{:#?}", report.functions);
    assert_eq!(
        report.functions[0].cyclomatic_complexity, 2,
        "an if-let has one alternative path; its let-condition is part of the if"
    );
}

#[test]
fn every_python_comprehension_generator_adds_a_path() {
    let report = analyze_fixture(
        "comprehension.py",
        r#"def pairs(left, right):
    return [(x, y) for x in left for y in right if x != y]
"#,
    );

    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    assert_eq!(report.functions.len(), 1, "{:#?}", report.functions);
    assert_eq!(
        report.functions[0].cyclomatic_complexity, 4,
        "baseline + two generators + one filter"
    );
}

#[test]
fn fallback_switch_match_and_when_arms_do_not_add_cyclomatic_complexity() {
    let directory = TempDir::new().expect("create fixture directory");
    let java = write_fixture(
        directory.path(),
        "SwitchFixture.java",
        r#"class SwitchFixture {
    static int classify(int value) {
        switch (value) {
            case 1:
                return 1;
            default:
                return 0;
        }
    }
}
"#,
    );
    let kotlin = write_fixture(
        directory.path(),
        "when.kt",
        r#"fun classify(value: Int): Int {
    return when (value) {
        1 -> 1
        else -> 0
    }
}
"#,
    );
    let rust = write_fixture(
        directory.path(),
        "match.rs",
        r#"fn classify(value: i32) -> i32 {
    match value {
        1 => 1,
        _ => 0,
    }
}
"#,
    );
    let python = write_fixture(
        directory.path(),
        "match.py",
        r#"def classify(value):
    match value:
        case 1:
            return 1
        case _:
            return 0
"#,
    );

    let report = analyze_files(&[java, kotlin, rust, python], &AnalysisSettings::default());
    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    assert_eq!(report.functions.len(), 4, "{:#?}", report.functions);
    for function in &report.functions {
        assert_eq!(
            function.cyclomatic_complexity, 2,
            "the explicit case adds a path, while default/else is the fallback: {function:#?}"
        );
    }
}

#[test]
fn irrefutable_capture_patterns_are_fallback_match_arms() {
    let directory = TempDir::new().expect("create fixture directory");
    let rust = write_fixture(
        directory.path(),
        "capture.rs",
        r#"fn classify(value: i32) -> i32 {
    match value {
        1 => 1,
        other => other,
    }
}
"#,
    );
    let python = write_fixture(
        directory.path(),
        "capture.py",
        r#"def classify(value):
    match value:
        case 1:
            return 1
        case other:
            return other
"#,
    );

    let report = analyze_files(&[rust, python], &AnalysisSettings::default());
    assert!(
        report.skipped_files.is_empty(),
        "{:#?}",
        report.skipped_files
    );
    assert_eq!(report.functions.len(), 2, "{:#?}", report.functions);
    for function in &report.functions {
        assert_eq!(
            function.cyclomatic_complexity, 2,
            "a bare capture is an unconditional fallback arm: {function:#?}"
        );
    }
}

#[test]
fn recoverable_parse_errors_are_reported_without_dropping_the_file() {
    let report = analyze_fixture("broken.py", "def broken(:\n    if:\n        return 1\n");

    assert_eq!(report.summary.files_scanned, 1);
    assert_eq!(report.summary.files_failed, 0);
    assert!(report.summary.parse_errors > 0, "{report:#?}");
    assert!(report.skipped_files.is_empty());
    let parse_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.rule_id == "parse-error")
        .collect();
    assert_eq!(parse_findings.len(), 1, "{:#?}", report.findings);
    assert!(!parse_findings[0].counts_toward_verbosity);
}

#[test]
fn kotlin_accessors_and_java_compact_constructors_are_functions() {
    let directory = TempDir::new().expect("create fixture directory");
    let kotlin = write_fixture(
        directory.path(),
        "Accessors.kt",
        r#"class Accessors(private val flag: Boolean) {
    var value: Int = 0
        get() {
            if (flag) return field
            return 0
        }
        set(next) {
            if (next > 0) field = next
        }
}
"#,
    );
    let java = write_fixture(
        directory.path(),
        "Record.java",
        r#"record Record(int value, String label) {
    Record {
        if (value < 0) throw new IllegalArgumentException();
    }
}
"#,
    );

    let report = analyze_files(&[kotlin, java], &AnalysisSettings::default());
    assert_eq!(report.summary.parse_errors, 0, "{:#?}", report.findings);
    let getter = report
        .functions
        .iter()
        .find(|function| function.name == "value.get")
        .expect("Kotlin getter metric");
    let setter = report
        .functions
        .iter()
        .find(|function| function.name == "value.set")
        .expect("Kotlin setter metric");
    let constructor = report
        .functions
        .iter()
        .find(|function| function.name == "Record")
        .expect("Java compact constructor metric");
    assert_eq!((getter.parameters, setter.parameters), (0, 1));
    assert_eq!(constructor.parameters, 2);
    assert_eq!(getter.cyclomatic_complexity, 2);
    assert_eq!(setter.cyclomatic_complexity, 2);
    assert_eq!(constructor.cyclomatic_complexity, 2);
}

#[test]
fn unreadable_utf8_is_skipped_without_losing_valid_files() {
    let directory = TempDir::new().expect("create fixture directory");
    let invalid = directory.path().join("invalid.rs");
    fs::write(&invalid, [0xff, 0xfe]).expect("write invalid UTF-8 fixture");
    let valid = write_fixture(directory.path(), "valid.rs", "fn valid() -> i32 { 1 }\n");

    let report = analyze_files(&[invalid, valid], &AnalysisSettings::default());
    assert_eq!(report.summary.files_scanned, 1);
    assert_eq!(report.summary.files_failed, 1);
    assert_eq!(report.skipped_files.len(), 1);
    assert!(
        report.skipped_files[0]
            .reason
            .contains("could not read UTF-8 source")
    );
    assert_eq!(report.functions.len(), 1);
    assert_eq!(report.functions[0].name, "valid");
}

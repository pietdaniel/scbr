use std::fs;

use scbr::analysis::analyze_files;
use scbr::language::Language;
use scbr::model::{AnalysisReport, AnalysisSettings};
use tempfile::TempDir;

fn dump_tree(extension: &str, source: &str) -> String {
    let mut parser = tree_sitter::Parser::new();
    let language = Language::from_path(std::path::Path::new(&format!("fixture.{extension}")))
        .expect("fixture language");
    parser
        .set_language(
            &language.tree_sitter_language(std::path::Path::new(&format!("fixture.{extension}"))),
        )
        .expect("fixture grammar");
    parser
        .parse(source, None)
        .expect("fixture tree")
        .root_node()
        .to_sexp()
}

fn analyze_fixture(extension: &str, source: &str) -> AnalysisReport {
    let temp = TempDir::new().expect("create language fixture directory");
    let path = temp.path().join(format!("fixture.{extension}"));
    fs::write(&path, source).expect("write language fixture");

    analyze_files(&[path], &AnalysisSettings::default())
}

fn assert_language_fixture(
    extension: &str,
    expected_language: Language,
    source: &str,
    expected_complexity: u32,
) {
    let report = analyze_fixture(extension, source);
    let context = format!("{expected_language} .{extension} fixture");

    assert!(
        report.skipped_files.is_empty(),
        "{context}: {:#?}",
        report.skipped_files
    );
    assert_eq!(report.summary.files_scanned, 1, "{context}");
    assert_eq!(report.summary.files_failed, 0, "{context}");
    assert_eq!(report.files.len(), 1, "{context}");

    let file = &report.files[0];
    assert_eq!(file.language, expected_language, "{context}");
    assert_eq!(file.parse_errors, 0, "{context}");
    assert_eq!(report.summary.parse_errors, 0, "{context}");
    assert_eq!(file.loc, source.lines().count() as u64, "{context}");
    assert!(
        file.sloc >= 8,
        "{context}: unexpectedly low SLOC: {}",
        file.sloc
    );
    assert!(
        file.sloc < file.loc,
        "{context}: comments and blank lines should not be SLOC"
    );
    assert!(
        file.comment_lines >= 2,
        "{context}: comments were not recognized"
    );
    assert!(
        file.comment_only_lines >= 2,
        "{context}: standalone comments were counted as code"
    );
    assert_eq!(file.functions, 2, "{context}");
    assert_eq!(report.functions.len(), 2, "{context}");

    for function in &report.functions {
        assert_eq!(
            function.language, expected_language,
            "{context}: {function:#?}"
        );
        assert_eq!(
            function.cyclomatic_complexity,
            expected_complexity,
            "{context}: complexity should be baseline + if + boolean operator for {}; tree={}",
            function.name,
            dump_tree(extension, source)
        );
        assert!(function.sloc >= 4, "{context}: {function:#?}");
    }

    let language_summary = report
        .languages
        .get(&expected_language)
        .unwrap_or_else(|| panic!("{context}: missing language summary"));
    assert_eq!(language_summary.files, 1, "{context}");
    assert_eq!(language_summary.functions, 2, "{context}");
    assert_eq!(language_summary.parse_errors, 0, "{context}");

    // The two functions deliberately use different names, parameter names, and
    // numeric literals. Matching their full spans verifies AST clone
    // normalization rather than a coincidental match between their `if` blocks.
    let function_starts: Vec<_> = report
        .functions
        .iter()
        .map(|function| function.span.start_line)
        .collect();
    let normalized_function_clone = report.clone_groups.iter().find(|group| {
        group.language == expected_language
            && group.instances.len() == 2
            && group.instances.iter().all(|instance| {
                function_starts.contains(&instance.span.start_line)
                    && report.functions.iter().any(|function| {
                        function.span == instance.span && function.path == instance.path
                    })
            })
    });
    assert!(
        normalized_function_clone.is_some(),
        "{context}: renamed functions were not grouped as normalized clones; groups={:#?}; tree={}",
        report.clone_groups,
        dump_tree(extension, source)
    );
    assert!(report.summary.cloned_sloc_lines > 0, "{context}");
    assert!(report.summary.clone_ratio > 0.0, "{context}");
}

#[test]
fn analyzes_typescript() {
    assert_language_fixture(
        "ts",
        Language::TypeScript,
        r#"// café: first implementation
export function classifyValue(value: number): number {
  if (value > 10 && value < 100) {
    return value + 1;
  }
  return value - 1;
}

// 漢字: renamed implementation
export function bucketScore(score: number): number {
  if (score > 20 && score < 200) {
    return score + 2;
  }
  return score - 2;
}
"#,
        3,
    );
}

#[test]
fn analyzes_tsx_with_jsx_syntax() {
    assert_language_fixture(
        "tsx",
        Language::TypeScript,
        r#"// café: component one
export function StatusBadge({ value }: { value: number }) {
  if (value > 10 && value < 100) {
    return <span data-kind="result">{value + 1}</span>;
  }
  return <span>{value - 1}</span>;
}

// 漢字: component two
export function ScoreBadge({ score }: { score: number }) {
  if (score > 20 && score < 200) {
    return <span data-kind="result">{score + 2}</span>;
  }
  return <span>{score - 2}</span>;
}
"#,
        3,
    );
}

#[test]
fn uppercase_tsx_extension_selects_the_tsx_grammar() {
    let report = analyze_fixture(
        "TSX",
        "export const Widget = () => <section><span>ok</span></section>;\n",
    );
    assert!(report.skipped_files.is_empty());
    assert_eq!(report.files[0].language, Language::TypeScript);
    assert_eq!(report.summary.parse_errors, 0);
}

#[test]
fn analyzes_javascript() {
    assert_language_fixture(
        "js",
        Language::JavaScript,
        r#"// café: first implementation
function classifyValue(value) {
  if (value > 10 && value < 100) {
    return value + 1;
  }
  return value - 1;
}

// 漢字: renamed implementation
function bucketScore(score) {
  if (score > 20 && score < 200) {
    return score + 2;
  }
  return score - 2;
}
"#,
        3,
    );
}

#[test]
fn analyzes_python_with_crlf_and_unicode_comments() {
    let source = concat!(
        "# café: first implementation\r\n",
        "def classify_value(value):\r\n",
        "    if value > 10 and value < 100:\r\n",
        "        return value + 1\r\n",
        "    return value - 1\r\n",
        "\r\n",
        "# 漢字: renamed implementation\r\n",
        "def bucket_score(score):\r\n",
        "    if score > 20 and score < 200:\r\n",
        "        return score + 2\r\n",
        "    return score - 2\r\n",
    );
    assert_language_fixture("py", Language::Python, source, 3);
}

#[test]
fn analyzes_go() {
    assert_language_fixture(
        "go",
        Language::Go,
        r#"package fixture

// café: first implementation
func classifyValue(value int) int {
    if value > 10 && value < 100 {
        return value + 1
    }
    return value - 1
}

// 漢字: renamed implementation
func bucketScore(score int) int {
    if score > 20 && score < 200 {
        return score + 2
    }
    return score - 2
}
"#,
        3,
    );
}

#[test]
fn analyzes_rust() {
    assert_language_fixture(
        "rs",
        Language::Rust,
        r#"// café: first implementation
pub fn classify_value(value: i32) -> i32 {
    if value > 10 && value < 100 {
        return value + 1;
    }
    value - 1
}

// 漢字: renamed implementation
pub fn bucket_score(score: i32) -> i32 {
    if score > 20 && score < 200 {
        return score + 2;
    }
    score - 2
}
"#,
        3,
    );
}

#[test]
fn analyzes_java() {
    assert_language_fixture(
        "java",
        Language::Java,
        r#"class Fixture {
    // café: first implementation
    static int classifyValue(int value) {
        if (value > 10 && value < 100) {
            return value + 1;
        }
        return value - 1;
    }

    // 漢字: renamed implementation
    static int bucketScore(int score) {
        if (score > 20 && score < 200) {
            return score + 2;
        }
        return score - 2;
    }
}
"#,
        3,
    );
}

#[test]
fn analyzes_kotlin() {
    assert_language_fixture(
        "kt",
        Language::Kotlin,
        r#"// café: first implementation
fun classifyValue(value: Int): Int {
    if (value > 10 && value < 100) {
        return value + 1
    }
    return value - 1
}

// 漢字: renamed implementation
fun bucketScore(score: Int): Int {
    if (score > 20 && score < 200) {
        return score + 2
    }
    return score - 2
}
"#,
        3,
    );
}

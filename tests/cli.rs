use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn scbr() -> Command {
    Command::cargo_bin("scbr").expect("the scbr binary should be built for integration tests")
}

fn write_source(root: &Path, relative: &str, source: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, source).unwrap();
    path
}

fn successful_json(mut command: Command) -> Value {
    let output = command.output().expect("scbr should start");
    assert!(
        output.status.success(),
        "command failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout should be JSON: {error}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn scan_json(subcommand: &str, paths: &[&Path]) -> Value {
    let mut command = scbr();
    command.args([subcommand, "--no-config", "--format", "json"]);
    command.args(paths);
    successful_json(command)
}

#[test]
fn top_level_and_every_subcommand_have_useful_help() {
    let top = scbr()
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let top = String::from_utf8(top).unwrap();
    assert!(top.contains("Usage: scbr <COMMAND>"));
    for subcommand in [
        "check",
        "loc",
        "complexity",
        "duplicates",
        "verbosity",
        "comments",
        "languages",
        "rules",
    ] {
        assert!(
            top.contains(subcommand),
            "top-level help should list {subcommand}"
        );

        let help = scbr()
            .args([subcommand, "--help"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let help = String::from_utf8(help).unwrap();
        assert!(help.contains("Usage:"), "{subcommand} help needs usage");
        if matches!(
            subcommand,
            "check" | "loc" | "complexity" | "duplicates" | "verbosity" | "comments"
        ) {
            assert!(help.contains("<PATH>..."));
            assert!(help.contains("--format <FORMAT>"));
            assert!(help.contains("--language <LANGUAGE>"));
            assert!(help.contains("--baseline <PATH>"));
        }
    }

    scbr()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("scbr {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn all_report_subcommands_and_the_analyze_alias_run() {
    let temp = TempDir::new().unwrap();
    let source = write_source(
        temp.path(),
        "sample.rs",
        "fn answer(value: i32) -> i32 {\n    if value > 0 { value } else { 0 }\n}\n",
    );

    for (subcommand, heading) in [
        ("check", "scbr report"),
        ("analyze", "scbr report"),
        ("loc", "Line counts"),
        ("complexity", "Complexity"),
        ("duplicates", "Duplicates"),
        ("verbosity", "Verbosity"),
        ("comments", "Comments"),
    ] {
        scbr()
            .args([subcommand, "--no-config"])
            .arg(&source)
            .assert()
            .success()
            .stdout(predicates::str::contains(heading));
    }

    scbr()
        .arg("rules")
        .assert()
        .success()
        .stdout(predicates::str::contains("verbosity ="))
        .stdout(predicates::str::contains("erosion ="))
        .stdout(predicates::str::contains("high-cyclomatic-complexity"));
}

#[test]
fn languages_command_and_analysis_cover_all_seven_languages() {
    let listed = scbr()
        .arg("languages")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let listed = String::from_utf8(listed).unwrap();
    for expected in [
        "TypeScript  .ts .tsx .mts .cts",
        "JavaScript  .js .jsx .mjs .cjs",
        "Python      .py .pyw .pyi",
        "Go          .go",
        "Rust        .rs",
        "Java        .java",
        "Kotlin      .kt .kts",
    ] {
        assert!(
            listed.contains(expected),
            "missing language row {expected:?}"
        );
    }

    let temp = TempDir::new().unwrap();
    write_source(
        temp.path(),
        "sample.ts",
        "function tsValue(x: number): number { return x + 1; }\n",
    );
    write_source(
        temp.path(),
        "sample.js",
        "function jsValue(x) { return x + 1; }\n",
    );
    write_source(
        temp.path(),
        "sample.py",
        "def py_value(x):\n    return x + 1\n",
    );
    write_source(
        temp.path(),
        "sample.go",
        "package sample\nfunc goValue(x int) int { return x + 1 }\n",
    );
    write_source(
        temp.path(),
        "sample.rs",
        "fn rust_value(x: i32) -> i32 { x + 1 }\n",
    );
    write_source(
        temp.path(),
        "Sample.java",
        "class Sample { int value(int x) { return x + 1; } }\n",
    );
    write_source(
        temp.path(),
        "sample.kt",
        "fun value(x: Int): Int { return x + 1 }\n",
    );

    let report = scan_json("check", &[temp.path()]);
    assert_eq!(report["summary"]["files_scanned"], 7);
    let languages = report["languages"].as_object().unwrap();
    assert_eq!(languages.len(), 7);
    for language in [
        "typescript",
        "javascript",
        "python",
        "go",
        "rust",
        "java",
        "kotlin",
    ] {
        assert_eq!(languages[language]["files"], 1, "missing {language}");
    }
}

#[test]
fn accepts_a_file_a_directory_and_deduplicates_overlapping_roots() {
    let temp = TempDir::new().unwrap();
    let first = write_source(temp.path(), "first.rs", "fn first() -> i32 { 1 }\n");
    let second = write_source(
        temp.path(),
        "nested/second.py",
        "def second():\n    return 2\n",
    );

    let file_report = scan_json("loc", &[&first]);
    assert_eq!(file_report["summary"]["files_scanned"], 1);
    let canonical_first = first.canonicalize().unwrap();
    assert_eq!(
        file_report["files"][0]["path"].as_str(),
        canonical_first.to_str()
    );

    let directory_report = scan_json("loc", &[temp.path()]);
    assert_eq!(directory_report["summary"]["files_scanned"], 2);

    let nested = temp.path().join("nested");
    let overlapping_report = scan_json("loc", &[temp.path(), &nested, &first, &second]);
    assert_eq!(overlapping_report["summary"]["files_scanned"], 2);
    assert_eq!(overlapping_report["files"].as_array().unwrap().len(), 2);
}

#[test]
fn compact_and_pretty_json_are_both_valid_complete_reports() {
    let temp = TempDir::new().unwrap();
    let source = write_source(
        temp.path(),
        "sample.py",
        "# TODO: simplify\ndef answer(flag):\n    return flag == True\n",
    );

    let mut compact = scbr();
    compact
        .args(["check", "--no-config", "--format", "json"])
        .arg(&source);
    let compact_output = compact.output().unwrap();
    assert!(compact_output.status.success());
    let compact_text = String::from_utf8(compact_output.stdout).unwrap();
    assert!(!compact_text.trim_end().contains('\n'));
    let compact_json: Value = serde_json::from_str(&compact_text).unwrap();

    let mut pretty = scbr();
    pretty
        .args(["check", "--no-config", "--format", "json", "--pretty"])
        .arg(&source);
    let pretty_output = pretty.output().unwrap();
    assert!(pretty_output.status.success());
    let pretty_text = String::from_utf8(pretty_output.stdout).unwrap();
    assert!(pretty_text.contains("\n  \"metric_version\""));
    let pretty_json: Value = serde_json::from_str(&pretty_text).unwrap();

    assert_eq!(compact_json["metric_version"], "scbr-v1");
    assert_eq!(compact_json["summary"]["files_scanned"], 1);
    let mut compact_summary = compact_json["summary"].clone();
    let mut pretty_summary = pretty_json["summary"].clone();
    compact_summary
        .as_object_mut()
        .unwrap()
        .remove("analysis_time_ms");
    pretty_summary
        .as_object_mut()
        .unwrap()
        .remove("analysis_time_ms");
    assert_eq!(pretty_summary, compact_summary);
    assert_eq!(pretty_json["findings"], compact_json["findings"]);
}

#[test]
fn language_filter_accepts_natural_names_aliases_and_comma_separated_values() {
    let temp = TempDir::new().unwrap();
    write_source(temp.path(), "one.rs", "fn one() {}\n");
    write_source(temp.path(), "two.py", "def two():\n    pass\n");
    write_source(temp.path(), "three.go", "package sample\nfunc three() {}\n");
    write_source(
        temp.path(),
        "four.ts",
        "function four(): number { return 4; }\n",
    );
    write_source(temp.path(), "five.js", "function five() { return 5; }\n");

    let mut rust_only = scbr();
    rust_only
        .args([
            "check",
            "--no-config",
            "--format",
            "json",
            "--language",
            "rust",
        ])
        .arg(temp.path());
    let rust_only = successful_json(rust_only);
    assert_eq!(rust_only["summary"]["files_scanned"], 1);
    assert_eq!(rust_only["languages"].as_object().unwrap().len(), 1);
    assert_eq!(rust_only["languages"]["rust"]["files"], 1);

    let mut python_and_go = scbr();
    python_and_go
        .args([
            "check",
            "--no-config",
            "--format",
            "json",
            "--language",
            "python,go",
        ])
        .arg(temp.path());
    let python_and_go = successful_json(python_and_go);
    assert_eq!(python_and_go["summary"]["files_scanned"], 2);
    assert!(python_and_go["languages"].get("python").is_some());
    assert!(python_and_go["languages"].get("go").is_some());
    assert!(python_and_go["languages"].get("rust").is_none());

    for language_names in ["typescript,javascript", "type-script,java-script", "ts,js"] {
        let mut web_languages = scbr();
        web_languages
            .args([
                "check",
                "--no-config",
                "--format",
                "json",
                "--language",
                language_names,
            ])
            .arg(temp.path());
        let web_languages = successful_json(web_languages);
        assert_eq!(web_languages["summary"]["files_scanned"], 2);
        assert_eq!(web_languages["languages"].as_object().unwrap().len(), 2);
        assert!(web_languages["languages"].get("typescript").is_some());
        assert!(web_languages["languages"].get("javascript").is_some());
    }

    let mut golang_alias = scbr();
    golang_alias
        .args([
            "check",
            "--no-config",
            "--format",
            "json",
            "--language",
            "golang",
        ])
        .arg(temp.path());
    let golang_alias = successful_json(golang_alias);
    assert_eq!(golang_alias["summary"]["files_scanned"], 1);
    assert_eq!(golang_alias["languages"]["go"]["files"], 1);
}

#[test]
fn directory_walk_respects_gitignore_and_include_ignored() {
    let temp = TempDir::new().unwrap();
    write_source(temp.path(), "visible.rs", "fn visible() {}\n");
    write_source(temp.path(), "generated.rs", "fn generated() {}\n");
    fs::write(temp.path().join(".gitignore"), "generated.rs\n").unwrap();

    let ordinary = scan_json("loc", &[temp.path()]);
    assert_eq!(ordinary["summary"]["files_scanned"], 1);
    assert!(
        ordinary["files"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("visible.rs")
    );

    let mut included = scbr();
    included
        .args([
            "loc",
            "--no-config",
            "--format",
            "json",
            "--include-ignored",
        ])
        .arg(temp.path());
    let included = successful_json(included);
    assert_eq!(included["summary"]["files_scanned"], 2);
}

#[test]
fn baseline_reports_metric_deltas() {
    let temp = TempDir::new().unwrap();
    let baseline = temp.path().join("baseline");
    let current = temp.path().join("current");
    write_source(&baseline, "sample.rs", "fn value() -> i32 {\n    1\n}\n");
    write_source(
        &current,
        "sample.rs",
        "fn value() -> i32 {\n    let one = 1;\n    let two = 2;\n    one + two\n}\n",
    );

    let mut command = scbr();
    command
        .args(["check", "--no-config", "--format", "json", "--baseline"])
        .arg(&baseline)
        .arg(&current);
    let report = successful_json(command);
    let delta = report["delta"].as_object().expect("baseline adds a delta");
    let baseline_loc = delta["baseline_loc"].as_i64().unwrap();
    let current_loc = delta["current_loc"].as_i64().unwrap();
    assert!(current_loc > baseline_loc);
    assert_eq!(
        delta["loc_delta"].as_i64().unwrap(),
        current_loc - baseline_loc
    );
    assert!(delta["loc_delta_pct"].as_f64().unwrap() > 0.0);
    let baseline_sloc = delta["baseline_sloc"].as_i64().unwrap();
    let current_sloc = delta["current_sloc"].as_i64().unwrap();
    assert!(current_sloc > baseline_sloc);
    assert_eq!(
        delta["sloc_delta"].as_i64().unwrap(),
        current_sloc - baseline_sloc
    );
    assert!(delta["sloc_delta_pct"].as_f64().unwrap() > 0.0);
    assert!(delta["verbosity_delta"].is_number());
    assert!(delta["erosion_delta"].is_number());
}

#[test]
fn discovers_scbr_config_formats() {
    for (name, contents) in [
        ("scbr.toml", "[thresholds]\ncomplexity = 7\n"),
        (".scbr.toml", "[thresholds]\ncomplexity = 7\n"),
        (
            "pyproject.toml",
            "[tool.scbr]\n[tool.scbr.thresholds]\ncomplexity = 7\n",
        ),
    ] {
        let temp = TempDir::new().unwrap();
        write_source(temp.path(), name, contents);
        let source = write_source(temp.path(), "src/sample.rs", "fn sample() {}\n");
        let mut command = scbr();
        command.args(["check", "--format", "json"]).arg(source);
        let report = successful_json(command);
        assert_eq!(report["settings"]["complexity_threshold"], 7, "{name}");
    }
}

#[test]
fn explicit_config_applies_thresholds_languages_and_excludes_with_cli_overrides() {
    let temp = TempDir::new().unwrap();
    let config = write_source(
        temp.path(),
        "settings.toml",
        r#"exclude = ["ignored.py"]
languages = ["python"]

[thresholds]
complexity = 2
cognitive-complexity = 3
min-clone-lines = 4
min-clone-statements = 2
long-function-sloc = 2
max-parameters = 1
max-nesting = 1
"#,
    );
    write_source(
        temp.path(),
        "kept.py",
        "def kept(first, second):\n    if first:\n        return second\n    return first\n",
    );
    write_source(temp.path(), "ignored.py", "def ignored():\n    pass\n");
    write_source(
        temp.path(),
        "also-ignored.py",
        "def ignored_too():\n    pass\n",
    );
    write_source(temp.path(), "filtered.rs", "fn filtered() {}\n");

    let mut command = scbr();
    command
        .args(["check", "--format", "json", "--config"])
        .arg(&config)
        .args([
            "--exclude",
            "also-ignored.py",
            "--complexity-threshold",
            "4",
            "--extreme-complexity-threshold",
            "40",
        ])
        .arg(temp.path());
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("using configuration"));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(report["summary"]["files_scanned"], 1);
    assert!(
        report["files"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("kept.py")
    );
    assert_eq!(report["settings"]["complexity_threshold"], 4);
    assert_eq!(report["settings"]["extreme_complexity_threshold"], 40);
    assert_eq!(report["settings"]["cognitive_threshold"], 3);
    assert_eq!(report["settings"]["min_clone_lines"], 4);
    assert_eq!(report["settings"]["min_clone_statements"], 2);
    assert_eq!(report["settings"]["long_function_sloc"], 2);
    assert_eq!(report["settings"]["max_parameters"], 1);
    assert_eq!(report["settings"]["max_nesting"], 1);
}

#[test]
fn fail_on_findings_returns_one_after_printing_the_report() {
    let temp = TempDir::new().unwrap();
    let source = write_source(
        temp.path(),
        "finding.rs",
        "fn has_items(items: &[i32]) -> bool {\n    items.len() > 0\n}\n",
    );

    let mut ordinary = scbr();
    ordinary
        .args(["check", "--no-config", "--format", "json"])
        .arg(&source)
        .assert()
        .success();

    let mut failing = scbr();
    let output = failing
        .args([
            "check",
            "--no-config",
            "--format",
            "json",
            "--fail-on-findings",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| {
                finding["rule_id"] == "rust-length-check"
                    && finding["counts_toward_verbosity"] == true
            })
    );
}

#[test]
fn strict_returns_two_for_parse_errors_after_printing_the_report() {
    let temp = TempDir::new().unwrap();
    let source = write_source(
        temp.path(),
        "broken.py",
        "def broken(:\n    if True\n        return 1\n",
    );

    let mut command = scbr();
    let output = command
        .args(["check", "--no-config", "--format", "json", "--strict"])
        .arg(&source)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report["summary"]["parse_errors"].as_u64().unwrap() > 0);
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| {
                finding["rule_id"] == "parse-error" && finding["severity"] == "error"
            })
    );
}

#[test]
fn strict_returns_two_for_baseline_parse_errors() {
    let temp = TempDir::new().unwrap();
    let current = write_source(temp.path(), "current.rs", "fn current() -> i32 { 1 }\n");
    let baseline = write_source(
        temp.path(),
        "baseline/broken.py",
        "def broken(:\n    if True\n        return 1\n",
    );

    let mut ordinary = scbr();
    ordinary
        .args(["check", "--no-config", "--format", "json", "--baseline"])
        .arg(&baseline)
        .arg(&current)
        .assert()
        .success();

    let mut strict = scbr();
    let output = strict
        .args([
            "check",
            "--no-config",
            "--format",
            "json",
            "--strict",
            "--baseline",
        ])
        .arg(&baseline)
        .arg(&current)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["parse_errors"], 0);
    assert_eq!(report["delta"]["baseline_files_scanned"], 1);
    assert!(report["delta"]["baseline_parse_errors"].as_u64().unwrap() > 0);
}

#[test]
fn all_oversized_files_still_produce_a_report_with_skips() {
    let temp = TempDir::new().unwrap();
    write_source(temp.path(), "large.rs", "fn larger_than_one_byte() {}\n");

    let mut command = scbr();
    command.args([
        "check",
        "--no-config",
        "--format",
        "json",
        "--max-file-bytes",
        "1",
    ]);
    command.arg(temp.path());
    let report = successful_json(command);
    assert_eq!(report["summary"]["files_scanned"], 0);
    assert_eq!(report["summary"]["files_failed"], 1);
    assert_eq!(report["skipped_files"].as_array().unwrap().len(), 1);
}

#[test]
fn invalid_inputs_and_empty_selections_return_actionable_errors() {
    let temp = TempDir::new().unwrap();
    let missing = temp.path().join("missing.rs");
    scbr()
        .args(["check", "--no-config"])
        .arg(&missing)
        .assert()
        .code(2)
        .stderr(predicates::str::contains("input does not exist"))
        .stderr(predicates::str::contains("missing.rs"));

    write_source(temp.path(), "README.md", "not source code\n");
    scbr()
        .args(["check", "--no-config"])
        .arg(temp.path())
        .assert()
        .code(2)
        .stderr(predicates::str::contains(
            "no supported source files were found",
        ));

    let source = write_source(temp.path(), "sample.rs", "fn sample() {}\n");
    scbr()
        .args(["check", "--no-config", "--exclude", "[unclosed"])
        .arg(&source)
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid exclude glob"));

    scbr()
        .args(["check", "--no-config", "--threads", "0"])
        .arg(&source)
        .assert()
        .code(2)
        .stderr(predicates::str::contains("value must be greater than zero"));

    scbr()
        .args(["check", "--no-config", "--pretty"])
        .arg(&source)
        .assert()
        .code(2)
        .stderr(predicates::str::contains(
            "required arguments were not provided",
        ))
        .stderr(predicates::str::contains("--format <FORMAT>"));

    scbr()
        .args(["check", "--no-config", "--format", "human", "--pretty"])
        .arg(&source)
        .assert()
        .code(2)
        .stderr(predicates::str::contains("--pretty requires --format json"));
}

#[test]
fn source_directories_named_target_build_and_dist_are_scanned_by_default() {
    let temp = TempDir::new().unwrap();
    write_source(temp.path(), "target/model.rs", "fn model() {}\n");
    write_source(
        temp.path(),
        "build/task.go",
        "package build\nfunc Task() {}\n",
    );
    write_source(
        temp.path(),
        "dist/module.js",
        "export function module() {}\n",
    );

    let mut command = scbr();
    command.args(["loc", "--no-config", "--format", "json"]);
    command.arg(temp.path());
    let report = successful_json(command);
    assert_eq!(report["summary"]["files_scanned"], 3);
}

#[cfg(unix)]
#[test]
fn json_reports_non_utf8_source_paths_without_losing_the_run() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let temp = TempDir::new().unwrap();
    let path = temp
        .path()
        .join(OsString::from_vec(b"invalid-\xff.rs".to_vec()));
    fs::write(&path, "fn valid_source() {}\n").unwrap();

    let mut command = scbr();
    command.args(["check", "--no-config", "--format", "json"]);
    command.arg(temp.path());
    let report = successful_json(command);
    assert_eq!(report["summary"]["files_scanned"], 1);
    assert!(report["files"][0]["path"].as_str().is_some());
}

#[test]
fn skipped_path_diagnostics_strip_terminal_control_characters() {
    let temp = TempDir::new().unwrap();
    write_source(temp.path(), "terminal-\u{1b}[31m.rs", "fn oversized() {}\n");
    let mut command = scbr();
    command.args([
        "check",
        "--no-config",
        "--format",
        "json",
        "--max-file-bytes",
        "1",
    ]);
    command.arg(temp.path());
    let output = command.output().unwrap();
    assert!(output.status.success());
    assert!(!output.stderr.contains(&0x1b));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["files_failed"], 1);
}

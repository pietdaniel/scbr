use std::collections::BTreeMap;
use std::path::PathBuf;

use scbr::language::Language;
use scbr::model::{
    AnalysisReport, AnalysisSettings, CloneGroup, CloneInstance, DeltaReport, FileMetrics, Finding,
    FindingCategory, FunctionMetric, LanguageSummary, METRIC_VERSION, Severity, SkippedFile, Span,
    Summary,
};
use scbr::output::{ReportView, render_human, render_json};

fn empty_report() -> AnalysisReport {
    AnalysisReport {
        metric_version: METRIC_VERSION,
        settings: AnalysisSettings::default(),
        summary: Summary::default(),
        languages: BTreeMap::new(),
        files: Vec::new(),
        functions: Vec::new(),
        clone_groups: Vec::new(),
        findings: Vec::new(),
        skipped_files: Vec::new(),
        delta: None,
    }
}

fn sample_report() -> AnalysisReport {
    let mut languages = BTreeMap::new();
    languages.insert(
        Language::Rust,
        LanguageSummary {
            files: 2,
            loc: 200,
            sloc: 150,
            comment_lines: 25,
            functions: 2,
            syntax_nodes: 500,
            duplicate_sloc_lines: 20,
            rule_flagged_sloc_lines: 10,
            verbosity_flagged_sloc_lines: 28,
            rules_available: 2,
            rule_findings: 2,
            rule_coverage: "extended_local_rules",
            ..LanguageSummary::default()
        },
    );

    let easy = FunctionMetric {
        path: PathBuf::from("z.rs"),
        language: Language::Rust,
        name: "easy".into(),
        span: Span::new(10, 14),
        sloc: 5,
        parameters: 1,
        cyclomatic_complexity: 2,
        cognitive_complexity: 1,
        max_nesting: 1,
        mass: 4.47,
        cognitive_mass: 2.24,
    };
    let hard = FunctionMetric {
        path: PathBuf::from("a.rs"),
        language: Language::Rust,
        name: "hard\nfunction".into(),
        span: Span::new(20, 59),
        sloc: 40,
        parameters: 7,
        cyclomatic_complexity: 18,
        cognitive_complexity: 23,
        max_nesting: 6,
        mass: 113.84,
        cognitive_mass: 145.46,
    };

    let small_clone = CloneGroup {
        fingerprint: "bbbbbbbbbbbbbbbb".into(),
        language: Language::Rust,
        instances: vec![
            CloneInstance {
                path: PathBuf::from("z.rs"),
                span: Span::new(70, 74),
                sloc: 5,
                preview: "if z {\nwork();\n}".into(),
            },
            CloneInstance {
                path: PathBuf::from("a.rs"),
                span: Span::new(80, 84),
                sloc: 5,
                preview: "if a { work(); }".into(),
            },
        ],
        total_sloc: 10,
    };
    let large_clone = CloneGroup {
        fingerprint: "aaaaaaaaaaaaaaaa".into(),
        language: Language::Rust,
        instances: vec![
            CloneInstance {
                path: PathBuf::from("z.rs"),
                span: Span::new(100, 109),
                sloc: 10,
                preview: "for item in things { use_item(item); }".into(),
            },
            CloneInstance {
                path: PathBuf::from("a.rs"),
                span: Span::new(2, 11),
                sloc: 10,
                preview: "for value in values { use_item(value); }".into(),
            },
        ],
        total_sloc: 20,
    };

    AnalysisReport {
        metric_version: METRIC_VERSION,
        settings: AnalysisSettings::default(),
        summary: Summary {
            files_scanned: 2,
            files_failed: 1,
            loc: 200,
            sloc: 150,
            blank_lines: 25,
            comment_lines: 25,
            comment_only_lines: 20,
            comment_density: 0.125,
            syntax_nodes: 500,
            parse_errors: 1,
            total_functions: 2,
            cc_max: 18,
            cc_mean: 10.0,
            cc_stddev: 8.0,
            cc_high_count: 1,
            cc_extreme_count: 0,
            cc_concentration: 0.4,
            total_mass: 118.31,
            high_cc_mass: 113.84,
            erosion: 0.9622,
            cognitive_max: 23,
            total_cognitive_mass: 147.70,
            high_cognitive_mass: 145.46,
            cognitive_erosion: 0.9848,
            clone_groups: 2,
            clone_instances: 4,
            cloned_sloc_lines: 20,
            clone_ratio: 20.0 / 150.0,
            excess_duplicate_sloc_lines: 10,
            excess_clone_ratio: 10.0 / 150.0,
            rule_findings: 2,
            rule_flagged_sloc_lines: 10,
            rule_flagged_ratio: 10.0 / 150.0,
            verbosity_flagged_sloc_lines: 28,
            verbosity: 28.0 / 150.0,
            blog_verbosity_flagged_lines: 32,
            blog_verbosity: 0.16,
            debt_markers: 3,
            analysis_time_ms: 12,
        },
        languages,
        files: vec![
            FileMetrics {
                path: PathBuf::from("z.rs"),
                language: Language::Rust,
                loc: 120,
                sloc: 90,
                blank_lines: 15,
                comment_lines: 15,
                comment_only_lines: 12,
                syntax_nodes: 300,
                parse_errors: 1,
                functions: 1,
                duplicated_sloc_lines: 10,
                excess_duplicate_sloc_lines: 0,
                rule_flagged_sloc_lines: 2,
                verbosity_flagged_sloc_lines: 12,
                debt_markers: 1,
            },
            FileMetrics {
                path: PathBuf::from("a.rs"),
                language: Language::Rust,
                loc: 80,
                sloc: 60,
                blank_lines: 10,
                comment_lines: 10,
                comment_only_lines: 8,
                syntax_nodes: 200,
                parse_errors: 0,
                functions: 1,
                duplicated_sloc_lines: 10,
                excess_duplicate_sloc_lines: 10,
                rule_flagged_sloc_lines: 8,
                verbosity_flagged_sloc_lines: 16,
                debt_markers: 2,
            },
        ],
        functions: vec![easy, hard],
        clone_groups: vec![small_clone, large_clone],
        findings: vec![
            Finding {
                rule_id: "commented-code".into(),
                category: FindingCategory::Comment,
                severity: Severity::Info,
                path: PathBuf::from("z.rs"),
                span: Span::new(7, 7),
                message: "comment appears to contain\nsource code".into(),
                counts_toward_verbosity: false,
            },
            Finding {
                rule_id: "parse-error".into(),
                category: FindingCategory::Parse,
                severity: Severity::Error,
                path: PathBuf::from("z.rs"),
                span: Span::new(1, 120),
                message: "partial parse".into(),
                counts_toward_verbosity: false,
            },
            Finding {
                rule_id: "long-function".into(),
                category: FindingCategory::Verbosity,
                severity: Severity::Warning,
                path: PathBuf::from("a.rs"),
                span: Span::new(20, 59),
                message: "function has many lines".into(),
                counts_toward_verbosity: true,
            },
        ],
        skipped_files: vec![SkippedFile {
            path: PathBuf::from("bad.kt"),
            reason: "could not read UTF-8 source".into(),
        }],
        delta: Some(DeltaReport {
            baseline_files_scanned: 2,
            baseline_files_failed: 0,
            baseline_parse_errors: 0,
            baseline_loc: 190,
            current_loc: 200,
            loc_delta: 10,
            loc_delta_pct: Some(10.0 / 190.0 * 100.0),
            baseline_sloc: 140,
            current_sloc: 150,
            sloc_delta: 10,
            sloc_delta_pct: Some(10.0 / 140.0 * 100.0),
            verbosity_delta: 0.02,
            erosion_delta: -0.015,
        }),
    }
}

#[test]
fn every_view_handles_an_empty_report_without_terminal_codes() {
    let report = empty_report();
    let views = [
        ReportView::Check,
        ReportView::Full,
        ReportView::Loc,
        ReportView::Complexity,
        ReportView::Duplicates,
        ReportView::Verbosity,
        ReportView::Comments,
    ];

    for view in views {
        let rendered = render_human(&report, view);
        assert!(!rendered.is_empty(), "{view:?} should have a heading");
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains("NaN"));
        assert!(!rendered.contains("inf"));
    }
}

#[test]
fn check_view_has_headline_scores_counts_and_ranked_findings() {
    let report = sample_report();
    let rendered = render_human(&report, ReportView::Check);

    assert!(rendered.contains("Verbosity: 18.67% (28 of 150 SLOC flagged)"));
    assert!(rendered.contains("Blog verbosity: 16.00%"));
    assert!(rendered.contains("Erosion: 96.22%"));
    assert!(rendered.contains(
        "Baseline delta: LOC +10 (+5.26%), SLOC +10 (+7.14%), verbosity +2.00 pp, erosion -1.50 pp"
    ));
    assert!(rendered.contains("Files: 2 scanned, 1 skipped | LOC: 200 | SLOC: 150"));
    assert!(rendered.contains("Top complex functions"));
    assert!(rendered.contains("Top duplicate groups"));
    assert!(rendered.contains("Top findings"));
    assert!(
        rendered.find("[error] parse-error").unwrap()
            < rendered.find("[warning] long-function").unwrap()
    );
    assert_eq!(rendered, render_human(&report, ReportView::Full));
}

#[test]
fn specialized_views_include_requested_detail_in_stable_order() {
    let report = sample_report();

    let loc = render_human(&report, ReportView::Loc);
    assert!(loc.contains(
        "Baseline: 190 LOC / 140 SLOC | current: 200 LOC / 150 SLOC | deltas: LOC +10 (+5.26%), SLOC +10 (+7.14%)"
    ));
    assert!(loc.find("a.rs [Rust]").unwrap() < loc.find("z.rs [Rust]").unwrap());

    let complexity = render_human(&report, ReportView::Complexity);
    assert!(complexity.contains("CC mean: 10.00"));
    assert!(complexity.contains("Baseline erosion delta: -1.50 pp"));
    assert!(complexity.find("hard function").unwrap() < complexity.find("easy").unwrap());

    let duplicates = render_human(&report, ReportView::Duplicates);
    assert!(duplicates.contains("Groups: 2 | instances: 4"));
    assert!(
        duplicates.find("fingerprint aaaaaaaaaaaa").unwrap()
            < duplicates.find("fingerprint bbbbbbbbbbbb").unwrap()
    );
    assert!(duplicates.find("a.rs:2-11").unwrap() < duplicates.find("z.rs:100-109").unwrap());

    let verbosity = render_human(&report, ReportView::Verbosity);
    assert!(verbosity.contains("SLOC score: 18.67%"));
    assert!(verbosity.contains("Baseline verbosity delta: +2.00 pp"));
    assert!(verbosity.contains("long-function"));
    assert!(!verbosity.contains("parse-error"));
    assert!(!verbosity.contains("commented-code"));

    let comments = render_human(&report, ReportView::Comments);
    assert!(comments.contains("Comment lines: 25 of 200 LOC (12.50%)"));
    assert!(comments.contains("commented-code"));
    assert!(comments.contains("comment appears to contain source code"));
    assert!(!comments.contains("long-function"));
}

#[test]
fn json_can_be_compact_or_pretty_and_round_trips() {
    let report = sample_report();
    let compact = render_json(&report, false).unwrap();
    let pretty = render_json(&report, true).unwrap();

    assert!(!compact.contains('\n'));
    assert!(pretty.contains('\n'));
    assert!(!compact.contains('\u{1b}'));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&compact).unwrap(),
        serde_json::from_str::<serde_json::Value>(&pretty).unwrap()
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&compact).unwrap()["metric_version"],
        METRIC_VERSION
    );
}

#[test]
fn human_output_does_not_pass_through_control_characters() {
    let mut report = sample_report();
    report.findings[0].message = "\u{1b}[31mred\u{7}\nmessage".into();
    report.findings[0].path = PathBuf::from("odd\u{1b}[2J.rs");

    let rendered = render_human(&report, ReportView::Comments);

    assert!(!rendered.contains('\u{1b}'));
    assert!(!rendered.contains('\u{7}'));
    assert!(!rendered.contains("\nmessage"));
}

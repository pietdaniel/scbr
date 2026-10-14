//! Deterministic, terminal-neutral report rendering.

use std::fmt::Write as _;
use std::path::Path;

use crate::model::{
    AnalysisReport, CloneGroup, FileMetrics, Finding, FindingCategory, FunctionMetric, Severity,
};

/// Selects the detail included in a human-readable report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportView {
    /// The complete `check` report.
    Check,
    /// Alias for [`ReportView::Check`] for callers that describe it as a full report.
    Full,
    /// Physical and source line counts.
    Loc,
    /// Function complexity and erosion.
    Complexity,
    /// Normalized syntax clone groups.
    Duplicates,
    /// Rule and clone based verbosity.
    Verbosity,
    /// Comment density, debt markers, and comment findings.
    Comments,
}

/// Render a report as deterministic plain text with no terminal control codes.
pub fn render_human(report: &AnalysisReport, view: ReportView) -> String {
    let mut output = String::new();
    match view {
        ReportView::Check | ReportView::Full => render_check(report, &mut output),
        ReportView::Loc => render_loc(report, &mut output),
        ReportView::Complexity => render_complexity(report, &mut output, None),
        ReportView::Duplicates => render_duplicates(report, &mut output, None),
        ReportView::Verbosity => render_verbosity(report, &mut output),
        ReportView::Comments => render_comments(report, &mut output),
    }
    output
}

/// Serialize the complete report as either pretty or compact JSON.
pub fn render_json(report: &AnalysisReport, pretty: bool) -> Result<String, serde_json::Error> {
    if pretty {
        serde_json::to_string_pretty(report)
    } else {
        serde_json::to_string(report)
    }
}

fn render_check(report: &AnalysisReport, output: &mut String) {
    let summary = &report.summary;
    line(
        output,
        format_args!("scbr report ({})", report.metric_version),
    );
    line(
        output,
        format_args!(
            "Verbosity: {} ({} of {} SLOC flagged)",
            percent(summary.verbosity),
            summary.verbosity_flagged_sloc_lines,
            summary.sloc
        ),
    );
    line(
        output,
        format_args!(
            "Blog verbosity: {} ({} of {} physical lines flagged)",
            percent(summary.blog_verbosity),
            summary.blog_verbosity_flagged_lines,
            summary.loc
        ),
    );
    line(
        output,
        format_args!(
            "Erosion: {} | cognitive erosion: {}",
            percent(summary.erosion),
            percent(summary.cognitive_erosion)
        ),
    );
    line(
        output,
        format_args!(
            "Files: {} scanned, {} skipped | LOC: {} | SLOC: {} | functions: {}",
            summary.files_scanned,
            report.skipped_files.len(),
            summary.loc,
            summary.sloc,
            summary.total_functions
        ),
    );
    line(
        output,
        format_args!(
            "Complexity: mean {:.2}, max {}, {} high, {} extreme",
            finite(summary.cc_mean),
            summary.cc_max,
            summary.cc_high_count,
            summary.cc_extreme_count
        ),
    );
    line(
        output,
        format_args!(
            "Duplicates: {} groups, {} instances, {} participating SLOC ({}) | {} excess SLOC ({})",
            summary.clone_groups,
            summary.clone_instances,
            summary.cloned_sloc_lines,
            percent(summary.clone_ratio),
            summary.excess_duplicate_sloc_lines,
            percent(summary.excess_clone_ratio)
        ),
    );
    line(
        output,
        format_args!(
            "Comments: {} lines ({}) | debt markers: {} | parse errors: {}",
            summary.comment_lines,
            percent(summary.comment_density),
            summary.debt_markers,
            summary.parse_errors
        ),
    );
    render_delta(report, output, DeltaFields::All);

    blank(output);
    render_complexity(report, output, Some(5));
    blank(output);
    render_duplicates(report, output, Some(3));
    blank(output);
    line(output, format_args!("Top findings"));
    let findings = sorted_findings(&report.findings, false);
    if findings.is_empty() {
        line(output, format_args!("  None"));
    } else {
        for finding in findings.into_iter().take(8) {
            render_finding(output, finding);
        }
    }

    if !report.skipped_files.is_empty() {
        blank(output);
        line(output, format_args!("Skipped files"));
        let mut skipped: Vec<_> = report.skipped_files.iter().collect();
        skipped.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.reason.cmp(&right.reason))
        });
        for file in skipped {
            line(
                output,
                format_args!(
                    "  {}: {}",
                    display_path(&file.path),
                    one_line(&file.reason, 160)
                ),
            );
        }
    }
}

fn render_loc(report: &AnalysisReport, output: &mut String) {
    let summary = &report.summary;
    line(output, format_args!("Line counts"));
    line(
        output,
        format_args!(
            "Files: {} scanned, {} skipped | LOC: {} | SLOC: {}",
            summary.files_scanned,
            report.skipped_files.len(),
            summary.loc,
            summary.sloc
        ),
    );
    line(
        output,
        format_args!(
            "Blank: {} | comment: {} | comment-only: {}",
            summary.blank_lines, summary.comment_lines, summary.comment_only_lines
        ),
    );
    render_delta(report, output, DeltaFields::Sloc);

    blank(output);
    line(output, format_args!("By language"));
    if report.languages.is_empty() {
        line(output, format_args!("  None"));
    } else {
        for (language, metrics) in &report.languages {
            line(
                output,
                format_args!(
                    "  {}: {} files, {} LOC, {} SLOC, {} comment lines",
                    language, metrics.files, metrics.loc, metrics.sloc, metrics.comment_lines
                ),
            );
        }
    }

    blank(output);
    line(output, format_args!("Files"));
    let files = sorted_files(&report.files);
    if files.is_empty() {
        line(output, format_args!("  None"));
    } else {
        for file in files {
            line(
                output,
                format_args!(
                    "  {} [{}]: {} LOC, {} SLOC, {} blank, {} comment",
                    display_path(&file.path),
                    file.language,
                    file.loc,
                    file.sloc,
                    file.blank_lines,
                    file.comment_lines
                ),
            );
        }
    }
}

fn render_complexity(report: &AnalysisReport, output: &mut String, limit: Option<usize>) {
    let summary = &report.summary;
    let heading = if limit.is_some() {
        "Top complex functions"
    } else {
        "Complexity"
    };
    line(output, format_args!("{heading}"));
    if limit.is_none() {
        line(
            output,
            format_args!(
                "Functions: {} | CC mean: {:.2} | max: {} | high: {} | extreme: {}",
                summary.total_functions,
                finite(summary.cc_mean),
                summary.cc_max,
                summary.cc_high_count,
                summary.cc_extreme_count
            ),
        );
        line(
            output,
            format_args!(
                "Erosion: {} | cognitive erosion: {} | CC concentration: {}",
                percent(summary.erosion),
                percent(summary.cognitive_erosion),
                percent(summary.cc_concentration)
            ),
        );
        render_delta(report, output, DeltaFields::Erosion);
    }

    let functions = sorted_functions(&report.functions);
    let count = limit.unwrap_or(usize::MAX);
    if functions.is_empty() {
        line(output, format_args!("  None"));
    } else {
        for function in functions.into_iter().take(count) {
            line(
                output,
                format_args!(
                    "  {}:{} {} [{}] - CC {}, cognitive {}, SLOC {}, nesting {}, parameters {}",
                    display_path(&function.path),
                    function.span.start_line,
                    one_line(&function.name, 80),
                    function.language,
                    function.cyclomatic_complexity,
                    function.cognitive_complexity,
                    function.sloc,
                    function.max_nesting,
                    function.parameters
                ),
            );
        }
    }
}

fn render_duplicates(report: &AnalysisReport, output: &mut String, limit: Option<usize>) {
    let summary = &report.summary;
    let heading = if limit.is_some() {
        "Top duplicate groups"
    } else {
        "Duplicates"
    };
    line(output, format_args!("{heading}"));
    if limit.is_none() {
        line(
            output,
            format_args!(
                "Groups: {} | instances: {} | participating SLOC: {} ({}) | excess SLOC: {} ({})",
                summary.clone_groups,
                summary.clone_instances,
                summary.cloned_sloc_lines,
                percent(summary.clone_ratio),
                summary.excess_duplicate_sloc_lines,
                percent(summary.excess_clone_ratio)
            ),
        );
    }

    let groups = sorted_clone_groups(&report.clone_groups);
    let count = limit.unwrap_or(usize::MAX);
    if groups.is_empty() {
        line(output, format_args!("  None"));
        return;
    }

    for (index, group) in groups.into_iter().take(count).enumerate() {
        line(
            output,
            format_args!(
                "  {}. {} - {} total SLOC, {} instances, fingerprint {}",
                index + 1,
                group.language,
                group.total_sloc,
                group.instances.len(),
                short_fingerprint(&group.fingerprint)
            ),
        );
        let mut instances: Vec<_> = group.instances.iter().collect();
        instances.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.span.start_line.cmp(&right.span.start_line))
                .then_with(|| left.span.end_line.cmp(&right.span.end_line))
                .then_with(|| left.preview.cmp(&right.preview))
        });
        for instance in instances {
            line(
                output,
                format_args!(
                    "     {}:{}-{} ({} SLOC) {}",
                    display_path(&instance.path),
                    instance.span.start_line,
                    instance.span.end_line,
                    instance.sloc,
                    one_line(&instance.preview, 100)
                ),
            );
        }
    }
}

fn render_verbosity(report: &AnalysisReport, output: &mut String) {
    let summary = &report.summary;
    line(output, format_args!("Verbosity"));
    line(
        output,
        format_args!(
            "SLOC score: {} ({} of {} SLOC flagged)",
            percent(summary.verbosity),
            summary.verbosity_flagged_sloc_lines,
            summary.sloc
        ),
    );
    line(
        output,
        format_args!(
            "Blog LOC score: {} ({} of {} physical lines flagged)",
            percent(summary.blog_verbosity),
            summary.blog_verbosity_flagged_lines,
            summary.loc
        ),
    );
    line(
        output,
        format_args!(
            "Rule lines: {} ({}) | duplicate lines: {} ({})",
            summary.rule_flagged_sloc_lines,
            percent(summary.rule_flagged_ratio),
            summary.cloned_sloc_lines,
            percent(summary.clone_ratio)
        ),
    );
    render_delta(report, output, DeltaFields::Verbosity);

    blank(output);
    line(output, format_args!("By file"));
    let mut files = sorted_files(&report.files);
    files.sort_by(|left, right| {
        right
            .verbosity_flagged_sloc_lines
            .cmp(&left.verbosity_flagged_sloc_lines)
            .then_with(|| left.path.cmp(&right.path))
    });
    if files.is_empty() {
        line(output, format_args!("  None"));
    } else {
        for file in files {
            line(
                output,
                format_args!(
                    "  {}: {} of {} SLOC flagged ({}) - {} rule, {} duplicate",
                    display_path(&file.path),
                    file.verbosity_flagged_sloc_lines,
                    file.sloc,
                    percent(ratio(file.verbosity_flagged_sloc_lines, file.sloc)),
                    file.rule_flagged_sloc_lines,
                    file.duplicated_sloc_lines
                ),
            );
        }
    }

    blank(output);
    line(output, format_args!("Verbose-code findings"));
    let findings = sorted_findings(&report.findings, true);
    if findings.is_empty() {
        line(output, format_args!("  None"));
    } else {
        for finding in findings {
            render_finding(output, finding);
        }
    }
}

fn render_comments(report: &AnalysisReport, output: &mut String) {
    let summary = &report.summary;
    line(output, format_args!("Comments"));
    line(
        output,
        format_args!(
            "Comment lines: {} of {} LOC ({}) | comment-only: {} | debt markers: {}",
            summary.comment_lines,
            summary.loc,
            percent(summary.comment_density),
            summary.comment_only_lines,
            summary.debt_markers
        ),
    );

    blank(output);
    line(output, format_args!("By file"));
    let files = sorted_files(&report.files);
    if files.is_empty() {
        line(output, format_args!("  None"));
    } else {
        for file in files {
            line(
                output,
                format_args!(
                    "  {}: {} of {} lines comment ({}) - {} comment-only, {} debt markers",
                    display_path(&file.path),
                    file.comment_lines,
                    file.loc,
                    percent(ratio(file.comment_lines, file.loc)),
                    file.comment_only_lines,
                    file.debt_markers
                ),
            );
        }
    }

    blank(output);
    line(output, format_args!("Comment findings"));
    let mut findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.category == FindingCategory::Comment)
        .collect();
    sort_findings(&mut findings);
    if findings.is_empty() {
        line(output, format_args!("  None"));
    } else {
        for finding in findings {
            render_finding(output, finding);
        }
    }
}

fn render_finding(output: &mut String, finding: &Finding) {
    line(
        output,
        format_args!(
            "  {}:{}-{} [{}] {}: {}",
            display_path(&finding.path),
            finding.span.start_line,
            finding.span.end_line,
            severity_name(finding.severity),
            one_line(&finding.rule_id, 80),
            one_line(&finding.message, 160)
        ),
    );
}

fn sorted_files(files: &[FileMetrics]) -> Vec<&FileMetrics> {
    let mut sorted: Vec<_> = files.iter().collect();
    sorted.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.language.cmp(&right.language))
    });
    sorted
}

fn sorted_functions(functions: &[FunctionMetric]) -> Vec<&FunctionMetric> {
    let mut sorted: Vec<_> = functions.iter().collect();
    sorted.sort_by(|left, right| {
        right
            .cyclomatic_complexity
            .cmp(&left.cyclomatic_complexity)
            .then_with(|| right.cognitive_complexity.cmp(&left.cognitive_complexity))
            .then_with(|| right.sloc.cmp(&left.sloc))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.span.start_line.cmp(&right.span.start_line))
            .then_with(|| left.name.cmp(&right.name))
    });
    sorted
}

fn sorted_clone_groups(groups: &[CloneGroup]) -> Vec<&CloneGroup> {
    let mut sorted: Vec<_> = groups.iter().collect();
    sorted.sort_by(|left, right| {
        right
            .total_sloc
            .cmp(&left.total_sloc)
            .then_with(|| right.instances.len().cmp(&left.instances.len()))
            .then_with(|| left.language.cmp(&right.language))
            .then_with(|| left.fingerprint.cmp(&right.fingerprint))
    });
    sorted
}

fn sorted_findings(findings: &[Finding], verbosity_only: bool) -> Vec<&Finding> {
    let mut sorted: Vec<_> = findings
        .iter()
        .filter(|finding| !verbosity_only || finding.counts_toward_verbosity)
        .collect();
    sort_findings(&mut sorted);
    sorted
}

fn sort_findings(findings: &mut Vec<&Finding>) {
    findings.sort_by(|left, right| {
        severity_rank(right.severity)
            .cmp(&severity_rank(left.severity))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.span.start_line.cmp(&right.span.start_line))
            .then_with(|| left.span.end_line.cmp(&right.span.end_line))
            .then_with(|| left.rule_id.cmp(&right.rule_id))
            .then_with(|| left.message.cmp(&right.message))
    });
}

fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Info => 0,
        Severity::Warning => 1,
        Severity::Error => 2,
    }
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

#[derive(Clone, Copy)]
enum DeltaFields {
    All,
    Sloc,
    Verbosity,
    Erosion,
}

fn render_delta(report: &AnalysisReport, output: &mut String, fields: DeltaFields) {
    let Some(delta) = &report.delta else {
        return;
    };

    match fields {
        DeltaFields::All => line(
            output,
            format_args!(
                "Baseline delta: LOC {} ({}), SLOC {} ({}), verbosity {}, erosion {}",
                signed_integer(delta.loc_delta),
                optional_signed_percent(delta.loc_delta_pct),
                signed_integer(delta.sloc_delta),
                optional_signed_percent(delta.sloc_delta_pct),
                signed_percentage_points(delta.verbosity_delta),
                signed_percentage_points(delta.erosion_delta)
            ),
        ),
        DeltaFields::Sloc => line(
            output,
            format_args!(
                "Baseline: {} LOC / {} SLOC | current: {} LOC / {} SLOC | deltas: LOC {} ({}), SLOC {} ({})",
                delta.baseline_loc,
                delta.baseline_sloc,
                delta.current_loc,
                delta.current_sloc,
                signed_integer(delta.loc_delta),
                optional_signed_percent(delta.loc_delta_pct),
                signed_integer(delta.sloc_delta),
                optional_signed_percent(delta.sloc_delta_pct)
            ),
        ),
        DeltaFields::Verbosity => line(
            output,
            format_args!(
                "Baseline verbosity delta: {}",
                signed_percentage_points(delta.verbosity_delta)
            ),
        ),
        DeltaFields::Erosion => line(
            output,
            format_args!(
                "Baseline erosion delta: {}",
                signed_percentage_points(delta.erosion_delta)
            ),
        ),
    }
    if delta.baseline_files_failed > 0 || delta.baseline_parse_errors > 0 {
        line(
            output,
            format_args!(
                "Baseline coverage: {} scanned, {} skipped, {} parse errors",
                delta.baseline_files_scanned,
                delta.baseline_files_failed,
                delta.baseline_parse_errors
            ),
        );
    }
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn percent(value: f64) -> String {
    if value.is_finite() {
        format!("{:.2}%", value * 100.0)
    } else {
        "n/a".to_owned()
    }
}

fn signed_integer(value: i64) -> String {
    format!("{value:+}")
}

fn optional_signed_percent(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map_or_else(|| "n/a".to_owned(), |value| format!("{value:+.2}%"))
}

fn signed_percentage_points(value: f64) -> String {
    if value.is_finite() {
        format!("{:+.2} pp", value * 100.0)
    } else {
        "n/a".to_owned()
    }
}

fn finite(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

fn short_fingerprint(fingerprint: &str) -> String {
    let cleaned = single_line(fingerprint);
    let shortened: String = cleaned.chars().take(12).collect();
    if shortened.is_empty() {
        "-".to_owned()
    } else {
        shortened
    }
}

fn one_line(value: &str, max_chars: usize) -> String {
    let normalized = single_line(value);
    let mut chars = normalized.chars();
    let shortened: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{shortened}...")
    } else {
        shortened
    }
}

fn single_line(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Render an operating-system path without terminal control characters.
pub fn display_path(path: &Path) -> String {
    single_line(path.to_string_lossy().as_ref())
}

/// Render diagnostic text on one terminal-safe line.
pub fn safe_text(value: &str) -> String {
    single_line(value)
}

fn line(output: &mut String, arguments: std::fmt::Arguments<'_>) {
    let _ = writeln!(output, "{arguments}");
}

fn blank(output: &mut String) {
    output.push('\n');
}

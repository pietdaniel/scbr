use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use rayon::ThreadPoolBuilder;

use crate::analysis::analyze_files;
use crate::config::{DEFAULT_EXCLUDES, load_config};
use crate::discovery::{DiscoveryOptions, discover_sources};
use crate::language::Language;
use crate::model::{AnalysisReport, AnalysisSettings, DeltaReport, SkippedFile};
use crate::output::{ReportView, display_path, render_human, render_json, safe_text};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Fast, local code sloppiness metrics for seven languages",
    long_about = "Analyze files, repositories, or multiple directories for LOC, SLOC, complexity, duplication, verbosity, erosion, and comment debt. Source stays on this machine."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the complete analysis.
    #[command(visible_alias = "analyze")]
    Check(ScanArgs),
    /// Report physical lines, source lines, blanks, and comments.
    Loc(ScanArgs),
    /// Report per-function complexity, mass, and erosion.
    Complexity(ScanArgs),
    /// Report normalized syntax clone groups and duplicated lines.
    Duplicates(ScanArgs),
    /// Report local style rules, clone coverage, and verbosity.
    Verbosity(ScanArgs),
    /// Report comment density, debt markers, and disabled-code hints.
    Comments(ScanArgs),
    /// List supported languages and extensions.
    Languages,
    /// List built-in finding classes and metric semantics.
    Rules,
}

#[derive(Debug, Clone, Args)]
pub struct ScanArgs {
    /// One or more source files or directories. Overlapping inputs are deduplicated.
    #[arg(required = true, value_name = "PATH", num_args = 1..)]
    paths: Vec<PathBuf>,

    /// Output encoding.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,

    /// Pretty-print JSON output.
    #[arg(long, requires = "format")]
    pretty: bool,

    /// Analyze only these languages (repeat or use comma-separated values).
    #[arg(long, value_enum, value_delimiter = ',', action = clap::ArgAction::Append)]
    language: Vec<Language>,

    /// Exclude a path glob. May be repeated.
    #[arg(long, value_name = "GLOB", action = clap::ArgAction::Append)]
    exclude: Vec<String>,

    /// Do not add common dependency and cache directory exclusions.
    #[arg(long)]
    no_default_excludes: bool,

    /// Include files excluded by Git and .ignore rules.
    #[arg(long, visible_alias = "include-all")]
    include_ignored: bool,

    /// Include hidden files and directories.
    #[arg(long)]
    hidden: bool,

    /// Explicit TOML or pyproject configuration.
    #[arg(long, value_name = "FILE", conflicts_with = "no_config")]
    config: Option<PathBuf>,

    /// Disable automatic configuration discovery.
    #[arg(long)]
    no_config: bool,

    /// Number of analysis worker threads.
    #[arg(long, value_name = "N", value_parser = parse_positive_usize)]
    threads: Option<usize>,

    /// Skip files larger than this many bytes and report them explicitly.
    #[arg(long, value_name = "BYTES")]
    max_file_bytes: Option<u64>,

    /// Additional baseline path used to calculate LOC and score deltas. Repeat for multiple roots.
    #[arg(long, value_name = "PATH", action = clap::ArgAction::Append)]
    baseline: Vec<PathBuf>,

    /// Cyclomatic complexity cutoff; erosion uses functions strictly above it.
    #[arg(long, value_name = "N")]
    complexity_threshold: Option<u32>,

    /// Extreme cyclomatic-complexity cutoff; must be at least the normal cutoff.
    #[arg(long, value_name = "N")]
    extreme_complexity_threshold: Option<u32>,

    /// Cognitive complexity cutoff.
    #[arg(long, value_name = "N")]
    cognitive_threshold: Option<u32>,

    /// Minimum SLOC span admitted as a clone candidate.
    #[arg(long, value_name = "N", value_parser = parse_positive_u32)]
    min_clone_lines: Option<u32>,

    /// Minimum immediate executable statements in a clone candidate body.
    #[arg(long, value_name = "N", value_parser = parse_positive_u32)]
    min_clone_statements: Option<u32>,

    /// Long-function warning threshold in SLOC.
    #[arg(long, value_name = "N")]
    long_function_sloc: Option<u32>,

    /// Large-parameter-list warning threshold.
    #[arg(long, value_name = "N")]
    max_parameters: Option<u32>,

    /// Deep-nesting warning threshold.
    #[arg(long, value_name = "N")]
    max_nesting: Option<u32>,

    /// Exit 1 when any slop or high-complexity finding exists.
    #[arg(long)]
    fail_on_findings: bool,

    /// Exit 2 when any file is skipped or has parser errors.
    #[arg(long)]
    strict: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    match execute(cli) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            let message = safe_text(&format!("{error:#}"));
            let _ = writeln!(io::stderr(), "error: {message}");
            ExitCode::from(2)
        }
    }
}

fn execute(cli: Cli) -> Result<u8> {
    match cli.command {
        Command::Check(args) => run_scan(args, ReportView::Full),
        Command::Loc(args) => run_scan(args, ReportView::Loc),
        Command::Complexity(args) => run_scan(args, ReportView::Complexity),
        Command::Duplicates(args) => run_scan(args, ReportView::Duplicates),
        Command::Verbosity(args) => run_scan(args, ReportView::Verbosity),
        Command::Comments(args) => run_scan(args, ReportView::Comments),
        Command::Languages => {
            print_languages();
            Ok(0)
        }
        Command::Rules => {
            print_rules();
            Ok(0)
        }
    }
}

fn run_scan(args: ScanArgs, view: ReportView) -> Result<u8> {
    if args.pretty && matches!(args.format, OutputFormat::Human) {
        bail!("--pretty requires --format json");
    }
    let (config, config_path) = if args.no_config {
        (crate::config::Config::default(), None)
    } else {
        load_config(
            args.config.as_deref(),
            args.paths.first().map(PathBuf::as_path),
        )?
    };

    let mut excludes: Vec<String> = if args.no_default_excludes {
        Vec::new()
    } else {
        DEFAULT_EXCLUDES
            .iter()
            .map(|value| (*value).into())
            .collect()
    };
    excludes.extend(config.exclude);
    excludes.extend(args.exclude.clone());
    let discovery_options = DiscoveryOptions {
        include_hidden: args.hidden || config.include_hidden,
        include_ignored: args.include_ignored || config.include_ignored,
        exclude: excludes,
    };

    let requested_languages = if args.language.is_empty() {
        config.languages
    } else {
        args.language.clone()
    };
    let (files, mut preliminary_skips) = discover_and_filter(
        &args.paths,
        &discovery_options,
        &requested_languages,
        args.max_file_bytes,
    )?;
    if files.is_empty() && preliminary_skips.is_empty() {
        bail!("no supported source files were found in the requested paths");
    }

    let mut settings = AnalysisSettings::default();
    config.thresholds.apply_to(&mut settings);
    apply_cli_thresholds(&args, &mut settings);
    validate_settings(&settings)?;

    let mut report = analyze_with_threads(&files, &settings, args.threads)?;
    append_skipped(&mut report, &mut preliminary_skips);

    let mut baseline_failed = false;
    if !args.baseline.is_empty() {
        let (baseline_files, mut baseline_skips) = discover_and_filter(
            &args.baseline,
            &discovery_options,
            &requested_languages,
            args.max_file_bytes,
        )?;
        if baseline_files.is_empty() && baseline_skips.is_empty() {
            bail!("no supported source files were found in baseline paths");
        }
        let mut baseline = analyze_with_threads(&baseline_files, &settings, args.threads)?;
        append_skipped(&mut baseline, &mut baseline_skips);
        baseline_failed = baseline.summary.files_failed > 0 || baseline.summary.parse_errors > 0;
        report.delta = Some(delta_report(&report, &baseline));
        for skipped in baseline.skipped_files {
            eprintln!(
                "baseline skipped {}: {}",
                display_path(&skipped.path),
                safe_text(&skipped.reason)
            );
        }
    }

    if let Some(path) = config_path {
        eprintln!("using configuration {}", display_path(&path));
    }
    for skipped in &report.skipped_files {
        eprintln!(
            "skipped {}: {}",
            display_path(&skipped.path),
            safe_text(&skipped.reason)
        );
    }

    match args.format {
        OutputFormat::Human => print!("{}", render_human(&report, view)),
        OutputFormat::Json => println!("{}", render_json(&report, args.pretty)?),
    }

    if args.strict
        && (report.summary.files_failed > 0 || report.summary.parse_errors > 0 || baseline_failed)
    {
        return Ok(2);
    }
    if args.fail_on_findings && has_findings(&report) {
        return Ok(1);
    }
    Ok(0)
}

fn discover_and_filter(
    inputs: &[PathBuf],
    options: &DiscoveryOptions,
    languages: &[Language],
    max_file_bytes: Option<u64>,
) -> Result<(Vec<PathBuf>, Vec<SkippedFile>)> {
    let discovered = discover_sources(inputs, options)?;
    let mut skipped: Vec<SkippedFile> = discovered
        .walk_errors
        .into_iter()
        .map(|issue| SkippedFile {
            path: issue
                .path
                .unwrap_or_else(|| PathBuf::from("<directory walk>")),
            reason: issue.message,
        })
        .collect();
    let mut files = Vec::new();
    for file in discovered.files {
        if !languages.is_empty() && !languages.contains(&file.language) {
            continue;
        }
        if let Some(limit) = max_file_bytes {
            match std::fs::metadata(&file.path) {
                Ok(metadata) if metadata.len() > limit => {
                    skipped.push(SkippedFile {
                        path: file.path,
                        reason: format!(
                            "file is {} bytes, above --max-file-bytes {limit}",
                            metadata.len()
                        ),
                    });
                    continue;
                }
                Err(error) => {
                    skipped.push(SkippedFile {
                        path: file.path,
                        reason: format!("could not inspect file size: {error}"),
                    });
                    continue;
                }
                _ => {}
            }
        }
        files.push(file.path);
    }
    Ok((files, skipped))
}

fn analyze_with_threads(
    files: &[PathBuf],
    settings: &AnalysisSettings,
    threads: Option<usize>,
) -> Result<AnalysisReport> {
    if let Some(threads) = threads {
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .context("could not construct analysis thread pool")?;
        Ok(pool.install(|| analyze_files(files, settings)))
    } else {
        Ok(analyze_files(files, settings))
    }
}

fn append_skipped(report: &mut AnalysisReport, skipped: &mut Vec<SkippedFile>) {
    report.summary.files_failed += skipped.len() as u64;
    report.skipped_files.append(skipped);
    report
        .skipped_files
        .sort_by(|left, right| left.path.cmp(&right.path));
}

fn delta_report(current: &AnalysisReport, baseline: &AnalysisReport) -> DeltaReport {
    let loc_delta = current.summary.loc as i128 - baseline.summary.loc as i128;
    let sloc_delta = current.summary.sloc as i128 - baseline.summary.sloc as i128;
    DeltaReport {
        baseline_files_scanned: baseline.summary.files_scanned,
        baseline_files_failed: baseline.summary.files_failed,
        baseline_parse_errors: baseline.summary.parse_errors,
        baseline_loc: baseline.summary.loc,
        current_loc: current.summary.loc,
        loc_delta: loc_delta.clamp(i64::MIN as i128, i64::MAX as i128) as i64,
        loc_delta_pct: if baseline.summary.loc == 0 {
            None
        } else {
            Some(loc_delta as f64 * 100.0 / baseline.summary.loc as f64)
        },
        baseline_sloc: baseline.summary.sloc,
        current_sloc: current.summary.sloc,
        sloc_delta: sloc_delta.clamp(i64::MIN as i128, i64::MAX as i128) as i64,
        sloc_delta_pct: if baseline.summary.sloc == 0 {
            None
        } else {
            Some(sloc_delta as f64 * 100.0 / baseline.summary.sloc as f64)
        },
        verbosity_delta: current.summary.verbosity - baseline.summary.verbosity,
        erosion_delta: current.summary.erosion - baseline.summary.erosion,
    }
}

fn apply_cli_thresholds(args: &ScanArgs, settings: &mut AnalysisSettings) {
    if let Some(value) = args.complexity_threshold {
        settings.complexity_threshold = value;
    }
    if let Some(value) = args.extreme_complexity_threshold {
        settings.extreme_complexity_threshold = value;
    }
    if let Some(value) = args.cognitive_threshold {
        settings.cognitive_threshold = value;
    }
    if let Some(value) = args.min_clone_lines {
        settings.min_clone_lines = value;
    }
    if let Some(value) = args.min_clone_statements {
        settings.min_clone_statements = value;
    }
    if let Some(value) = args.long_function_sloc {
        settings.long_function_sloc = value;
    }
    if let Some(value) = args.max_parameters {
        settings.max_parameters = value;
    }
    if let Some(value) = args.max_nesting {
        settings.max_nesting = value;
    }
}

fn validate_settings(settings: &AnalysisSettings) -> Result<()> {
    if settings.extreme_complexity_threshold < settings.complexity_threshold {
        bail!(
            "extreme complexity threshold {} is below complexity threshold {}",
            settings.extreme_complexity_threshold,
            settings.complexity_threshold
        );
    }
    if settings.min_clone_lines == 0 || settings.min_clone_statements == 0 {
        bail!("clone thresholds must be greater than zero");
    }
    Ok(())
}

fn has_findings(report: &AnalysisReport) -> bool {
    !report.clone_groups.is_empty()
        || !report.findings.is_empty()
        || report.summary.cc_high_count > 0
        || report.summary.cognitive_erosion > 0.0
}

fn print_languages() {
    println!("TypeScript  .ts .tsx .mts .cts");
    println!("JavaScript  .js .jsx .mjs .cjs");
    println!("Python      .py .pyw .pyi");
    println!("Go          .go");
    println!("Rust        .rs");
    println!("Java        .java");
    println!("Kotlin      .kt .kts");
}

fn print_rules() {
    println!("Composite metrics:");
    println!("  verbosity = union(rule SLOC, clone SLOC) / SLOC");
    println!("  erosion = mass(CC > threshold) / total mass, mass = CC * sqrt(function SLOC)");
    println!("\nBuilt-in verbosity rules:");
    println!("  trivial-wrapper, redundant-boolean-comparison, redundant-boolean-ternary");
    println!("  python-range-len, python-redundant-get-default, python-explicit-length-check");
    println!("  python-list-before-iteration, rust-length-check, jvm-length-check");
    println!("  javascript-empty-array-constructor");
    println!("\nAdditional diagnostics:");
    println!("  high-cyclomatic-complexity, high-cognitive-complexity, long-function");
    println!("  deep-nesting, large-parameter-list, debt-marker, commented-out-code");
}

fn parse_positive_usize(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|error| format!("invalid positive integer: {error}"))?;
    if parsed == 0 {
        Err("value must be greater than zero".into())
    } else {
        Ok(parsed)
    }
}

fn parse_positive_u32(value: &str) -> Result<u32, String> {
    let parsed = value
        .parse::<u32>()
        .map_err(|error| format!("invalid positive integer: {error}"))?;
    if parsed == 0 {
        Err("value must be greater than zero".into())
    } else {
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::delta_report;
    use crate::analysis::analyze_files;
    use crate::model::AnalysisSettings;

    #[test]
    fn zero_baseline_delta_has_no_percentage() {
        let settings = AnalysisSettings::default();
        let baseline = analyze_files(&[], &settings);
        let current = analyze_files(&[], &settings);
        assert_eq!(delta_report(&current, &baseline).sloc_delta_pct, None);
    }
}

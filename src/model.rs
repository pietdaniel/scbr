use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};

use crate::language::Language;

pub const METRIC_VERSION: &str = "scbr-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    Verbosity,
    Complexity,
    Structure,
    Comment,
    Parse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub start_line: u32,
    pub end_line: u32,
}

impl Span {
    pub fn new(start_line: usize, end_line: usize) -> Self {
        Self {
            start_line: start_line as u32,
            end_line: end_line as u32,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub category: FindingCategory,
    pub severity: Severity,
    #[serde(serialize_with = "serialize_path")]
    pub path: PathBuf,
    pub span: Span,
    pub message: String,
    pub counts_toward_verbosity: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionMetric {
    #[serde(serialize_with = "serialize_path")]
    pub path: PathBuf,
    pub language: Language,
    pub name: String,
    pub span: Span,
    pub sloc: u32,
    pub parameters: u32,
    pub cyclomatic_complexity: u32,
    pub cognitive_complexity: u32,
    pub max_nesting: u32,
    pub mass: f64,
    pub cognitive_mass: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloneInstance {
    #[serde(serialize_with = "serialize_path")]
    pub path: PathBuf,
    pub span: Span,
    pub sloc: u32,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloneGroup {
    pub fingerprint: String,
    pub language: Language,
    pub instances: Vec<CloneInstance>,
    pub total_sloc: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileMetrics {
    #[serde(serialize_with = "serialize_path")]
    pub path: PathBuf,
    pub language: Language,
    pub loc: u64,
    pub sloc: u64,
    pub blank_lines: u64,
    pub comment_lines: u64,
    pub comment_only_lines: u64,
    pub syntax_nodes: u64,
    pub parse_errors: u64,
    pub functions: u64,
    pub duplicated_sloc_lines: u64,
    /// Duplicate SLOC after retaining one canonical instance per clone group.
    pub excess_duplicate_sloc_lines: u64,
    pub rule_flagged_sloc_lines: u64,
    pub verbosity_flagged_sloc_lines: u64,
    pub debt_markers: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LanguageSummary {
    pub files: u64,
    pub loc: u64,
    pub sloc: u64,
    pub comment_lines: u64,
    pub functions: u64,
    pub syntax_nodes: u64,
    pub parse_errors: u64,
    pub duplicate_sloc_lines: u64,
    pub excess_duplicate_sloc_lines: u64,
    pub rule_flagged_sloc_lines: u64,
    pub verbosity_flagged_sloc_lines: u64,
    pub rules_available: u64,
    pub rule_findings: u64,
    pub rule_coverage: &'static str,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub files_scanned: u64,
    pub files_failed: u64,
    pub loc: u64,
    pub sloc: u64,
    pub blank_lines: u64,
    pub comment_lines: u64,
    pub comment_only_lines: u64,
    pub comment_density: f64,
    pub syntax_nodes: u64,
    pub parse_errors: u64,
    pub total_functions: u64,
    pub cc_max: u32,
    pub cc_mean: f64,
    pub cc_stddev: f64,
    pub cc_high_count: u64,
    pub cc_extreme_count: u64,
    pub cc_concentration: f64,
    pub total_mass: f64,
    pub high_cc_mass: f64,
    pub erosion: f64,
    pub cognitive_max: u32,
    pub total_cognitive_mass: f64,
    pub high_cognitive_mass: f64,
    pub cognitive_erosion: f64,
    pub clone_groups: u64,
    pub clone_instances: u64,
    pub cloned_sloc_lines: u64,
    pub clone_ratio: f64,
    pub excess_duplicate_sloc_lines: u64,
    pub excess_clone_ratio: f64,
    pub rule_findings: u64,
    pub rule_flagged_sloc_lines: u64,
    pub rule_flagged_ratio: f64,
    pub verbosity_flagged_sloc_lines: u64,
    pub verbosity: f64,
    pub blog_verbosity_flagged_lines: u64,
    pub blog_verbosity: f64,
    pub debt_markers: u64,
    pub analysis_time_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnalysisSettings {
    pub complexity_threshold: u32,
    pub extreme_complexity_threshold: u32,
    pub cognitive_threshold: u32,
    pub min_clone_lines: u32,
    pub min_clone_statements: u32,
    pub long_function_sloc: u32,
    pub max_parameters: u32,
    pub max_nesting: u32,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            complexity_threshold: 10,
            extreme_complexity_threshold: 30,
            cognitive_threshold: 10,
            min_clone_lines: 3,
            min_clone_statements: 2,
            long_function_sloc: 50,
            max_parameters: 5,
            max_nesting: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedFile {
    #[serde(serialize_with = "serialize_path")]
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeltaReport {
    pub baseline_files_scanned: u64,
    pub baseline_files_failed: u64,
    pub baseline_parse_errors: u64,
    pub baseline_loc: u64,
    pub current_loc: u64,
    pub loc_delta: i64,
    pub loc_delta_pct: Option<f64>,
    pub baseline_sloc: u64,
    pub current_sloc: u64,
    pub sloc_delta: i64,
    pub sloc_delta_pct: Option<f64>,
    pub verbosity_delta: f64,
    pub erosion_delta: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisReport {
    pub metric_version: &'static str,
    pub settings: AnalysisSettings,
    pub summary: Summary,
    pub languages: BTreeMap<Language, LanguageSummary>,
    pub files: Vec<FileMetrics>,
    pub functions: Vec<FunctionMetric>,
    pub clone_groups: Vec<CloneGroup>,
    pub findings: Vec<Finding>,
    pub skipped_files: Vec<SkippedFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<DeltaReport>,
}

fn serialize_path<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&path.to_string_lossy())
}

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::language::Language;
use crate::model::AnalysisSettings;

pub const DEFAULT_EXCLUDES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".venv",
    "venv",
    "__pycache__",
    "node_modules",
    ".gradle",
    ".idea",
];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    pub exclude: Vec<String>,
    pub include_hidden: bool,
    pub include_ignored: bool,
    pub languages: Vec<Language>,
    pub thresholds: ThresholdConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ThresholdConfig {
    pub complexity: Option<u32>,
    pub extreme_complexity: Option<u32>,
    pub cognitive_complexity: Option<u32>,
    pub min_clone_lines: Option<u32>,
    pub min_clone_statements: Option<u32>,
    pub long_function_sloc: Option<u32>,
    pub max_parameters: Option<u32>,
    pub max_nesting: Option<u32>,
}

impl ThresholdConfig {
    pub fn apply_to(&self, settings: &mut AnalysisSettings) {
        if let Some(value) = self.complexity {
            settings.complexity_threshold = value;
        }
        if let Some(value) = self.extreme_complexity {
            settings.extreme_complexity_threshold = value;
        }
        if let Some(value) = self.cognitive_complexity {
            settings.cognitive_threshold = value;
        }
        if let Some(value) = self.min_clone_lines {
            settings.min_clone_lines = value;
        }
        if let Some(value) = self.min_clone_statements {
            settings.min_clone_statements = value;
        }
        if let Some(value) = self.long_function_sloc {
            settings.long_function_sloc = value;
        }
        if let Some(value) = self.max_parameters {
            settings.max_parameters = value;
        }
        if let Some(value) = self.max_nesting {
            settings.max_nesting = value;
        }
    }
}

pub fn load_config(
    explicit: Option<&Path>,
    first_input: Option<&Path>,
) -> Result<(Config, Option<PathBuf>)> {
    let path = if let Some(path) = explicit {
        if !path.is_file() {
            bail!("configuration file does not exist: {}", path.display());
        }
        Some(path.to_path_buf())
    } else {
        discover_config(first_input)
    };

    let Some(path) = path else {
        return Ok((Config::default(), None));
    };
    let source = fs::read_to_string(&path)
        .with_context(|| format!("could not read configuration {}", path.display()))?;
    let config = if path.file_name().and_then(|name| name.to_str()) == Some("pyproject.toml") {
        parse_pyproject(&source, &path)?
    } else {
        toml::from_str(&source)
            .with_context(|| format!("invalid configuration {}", path.display()))?
    };
    Ok((config, Some(path)))
}

fn parse_pyproject(source: &str, path: &Path) -> Result<Config> {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct PyProject {
        tool: Tool,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Tool {
        scbr: Option<Config>,
    }

    let project: PyProject = toml::from_str(source)
        .with_context(|| format!("invalid pyproject configuration {}", path.display()))?;
    Ok(project.tool.scbr.unwrap_or_default())
}

fn discover_config(first_input: Option<&Path>) -> Option<PathBuf> {
    let start = first_input
        .and_then(|path| {
            if path.is_dir() {
                Some(path)
            } else {
                path.parent()
            }
        })
        .and_then(|path| path.canonicalize().ok())
        .or_else(|| std::env::current_dir().ok())?;

    for directory in start.ancestors() {
        for name in ["scbr.toml", ".scbr.toml", "pyproject.toml"] {
            let candidate = directory.join(name);
            if candidate.is_file()
                && (name != "pyproject.toml" || pyproject_has_scbr_section(&candidate))
            {
                return Some(candidate);
            }
        }
        if directory.join(".git").exists() {
            break;
        }
    }
    None
}

fn pyproject_has_scbr_section(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|source| source.contains("[tool.scbr]"))
}

#[cfg(test)]
mod tests {
    use super::{Config, ThresholdConfig};
    use crate::model::AnalysisSettings;

    #[test]
    fn parses_and_applies_thresholds() {
        let config: Config = toml::from_str(
            r#"
exclude = ["generated"]
include-hidden = true
languages = ["rust", "python"]

[thresholds]
complexity = 7
min-clone-lines = 5
"#,
        )
        .unwrap();
        assert_eq!(config.exclude, ["generated"]);
        assert!(config.include_hidden);
        let mut settings = AnalysisSettings::default();
        config.thresholds.apply_to(&mut settings);
        assert_eq!(settings.complexity_threshold, 7);
        assert_eq!(settings.min_clone_lines, 5);
    }

    #[test]
    fn absent_thresholds_preserve_defaults() {
        let mut settings = AnalysisSettings::default();
        ThresholdConfig::default().apply_to(&mut settings);
        assert_eq!(settings, AnalysisSettings::default());
    }
}

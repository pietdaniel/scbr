//! Deterministic source-file discovery.
//!
//! Explicit file inputs are always considered, including hidden and ignored
//! files. Hidden and ignore filtering applies while walking directory inputs.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use thiserror::Error;

use crate::language::Language;

/// Options controlling how source paths are discovered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryOptions {
    /// Traverse hidden files and directories. Explicit file inputs are not
    /// affected by this option.
    pub include_hidden: bool,
    /// Traverse paths matched by `.gitignore`, `.git/info/exclude`, global Git
    /// excludes, and `.ignore` files.
    pub include_ignored: bool,
    /// Glob patterns to exclude from the result. Patterns are matched against
    /// the full path, the path relative to each directory input, and the base
    /// name. A pattern matching a directory prunes that directory.
    pub exclude: Vec<String>,
}

/// A supported source file found during discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    /// Canonical, absolute path. Canonical paths make overlapping inputs and
    /// symlink aliases deduplicate reliably.
    pub path: PathBuf,
    pub language: Language,
}

/// A traversal problem that did not prevent discovery from continuing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkIssue {
    pub path: Option<PathBuf>,
    pub message: String,
}

/// Files and nonfatal traversal issues produced by discovery.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryResult {
    /// Canonical paths, sorted lexicographically and deduplicated.
    pub files: Vec<DiscoveredFile>,
    /// Errors from individual entries encountered while walking directories.
    /// Discovery continues after these errors.
    pub walk_errors: Vec<WalkIssue>,
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("no input paths were provided")]
    NoInputs,

    #[error("input does not exist: {path}", path = .path.display())]
    InputNotFound { path: PathBuf },

    #[error("cannot inspect input {path}: {source}", path = .path.display())]
    CannotInspectInput {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("input is neither a regular file nor a directory: {path}", path = .path.display())]
    UnsupportedInputType { path: PathBuf },

    #[error(
        "unsupported source file {path}; supported extensions are: {extensions}",
        path = .path.display(),
        extensions = supported_extensions().join(", ")
    )]
    UnsupportedFile { path: PathBuf },

    #[error("invalid exclude glob {pattern:?}: {source}")]
    InvalidExcludeGlob {
        pattern: String,
        #[source]
        source: globset::Error,
    },
}

/// Extensions recognized by discovery, without leading dots.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "py", "pyw", "pyi", "go", "rs", "java",
    "kt", "kts",
];

pub fn supported_extensions() -> &'static [&'static str] {
    SUPPORTED_EXTENSIONS
}

/// Discover supported source files below one or more paths.
///
/// Missing paths and explicitly supplied unsupported files are fatal input
/// errors. Problems with individual entries below directory inputs are
/// returned in [`DiscoveryResult::walk_errors`] so callers can report partial
/// results accurately.
pub fn discover_sources(
    inputs: &[PathBuf],
    options: &DiscoveryOptions,
) -> Result<DiscoveryResult, DiscoveryError> {
    if inputs.is_empty() {
        return Err(DiscoveryError::NoInputs);
    }

    let excludes = Arc::new(build_excludes(&options.exclude)?);
    let mut files = BTreeMap::<PathBuf, DiscoveredFile>::new();
    let mut walk_errors = Vec::new();

    for input in inputs {
        let metadata = match fs::metadata(input) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(DiscoveryError::InputNotFound {
                    path: input.clone(),
                });
            }
            Err(source) => {
                return Err(DiscoveryError::CannotInspectInput {
                    path: input.clone(),
                    source,
                });
            }
        };

        if metadata.is_file() {
            add_explicit_file(input, &excludes, &mut files)?;
        } else if metadata.is_dir() {
            walk_directory(
                input,
                options,
                Arc::clone(&excludes),
                &mut files,
                &mut walk_errors,
            )?;
        } else {
            return Err(DiscoveryError::UnsupportedInputType {
                path: input.clone(),
            });
        }
    }

    Ok(DiscoveryResult {
        files: files.into_values().collect(),
        walk_errors,
    })
}

fn add_explicit_file(
    path: &Path,
    excludes: &GlobSet,
    files: &mut BTreeMap<PathBuf, DiscoveredFile>,
) -> Result<(), DiscoveryError> {
    let Some(language) = Language::from_path(path) else {
        return Err(DiscoveryError::UnsupportedFile {
            path: path.to_path_buf(),
        });
    };

    if is_excluded(
        excludes,
        path,
        path.parent().unwrap_or_else(|| Path::new("")),
    ) {
        return Ok(());
    }

    let canonical =
        fs::canonicalize(path).map_err(|source| DiscoveryError::CannotInspectInput {
            path: path.to_path_buf(),
            source,
        })?;
    files.entry(canonical.clone()).or_insert(DiscoveredFile {
        path: canonical,
        language,
    });
    Ok(())
}

fn walk_directory(
    input: &Path,
    options: &DiscoveryOptions,
    excludes: Arc<GlobSet>,
    files: &mut BTreeMap<PathBuf, DiscoveredFile>,
    walk_errors: &mut Vec<WalkIssue>,
) -> Result<(), DiscoveryError> {
    let root = fs::canonicalize(input).map_err(|source| DiscoveryError::CannotInspectInput {
        path: input.to_path_buf(),
        source,
    })?;

    let filter_root = root.clone();
    let filter_excludes = Arc::clone(&excludes);
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(!options.include_hidden)
        .ignore(!options.include_ignored)
        .git_ignore(!options.include_ignored)
        .git_global(!options.include_ignored)
        .git_exclude(!options.include_ignored)
        .parents(!options.include_ignored)
        // Treat a standalone directory containing `.gitignore` like a project
        // root, even when it has no `.git` directory (common in archives and
        // temporary checkouts).
        .require_git(false)
        .follow_links(false)
        .filter_entry(move |entry| {
            entry.depth() == 0 || !is_excluded(&filter_excludes, entry.path(), &filter_root)
        });

    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                walk_errors.push(WalkIssue {
                    path: walk_error_path(&error),
                    message: error.to_string(),
                });
                continue;
            }
        };

        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }

        let path = entry.path();
        let Some(language) = Language::from_path(path) else {
            continue;
        };
        // `filter_entry` handles this too. Retain the check so correctness does
        // not depend on whether a future walker applies filters to files.
        if is_excluded(&excludes, path, &root) {
            continue;
        }

        match fs::canonicalize(path) {
            Ok(canonical) => {
                files.entry(canonical.clone()).or_insert(DiscoveredFile {
                    path: canonical,
                    language,
                });
            }
            Err(error) => walk_errors.push(WalkIssue {
                path: Some(path.to_path_buf()),
                message: error.to_string(),
            }),
        }
    }

    Ok(())
}

fn build_excludes(patterns: &[String]) -> Result<GlobSet, DiscoveryError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .backslash_escape(true)
            .build()
            .map_err(|source| DiscoveryError::InvalidExcludeGlob {
                pattern: pattern.clone(),
                source,
            })?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|source| DiscoveryError::InvalidExcludeGlob {
            pattern: patterns.join(", "),
            source,
        })
}

fn is_excluded(excludes: &GlobSet, path: &Path, root: &Path) -> bool {
    excludes.is_match(path)
        || path
            .strip_prefix(root)
            .is_ok_and(|relative| excludes.is_match(relative))
        || path
            .file_name()
            .is_some_and(|name| excludes.is_match(Path::new(name)))
}

fn walk_error_path(error: &ignore::Error) -> Option<PathBuf> {
    match error {
        ignore::Error::Partial(errors) => errors.iter().find_map(walk_error_path),
        ignore::Error::WithLineNumber { err, .. } | ignore::Error::WithDepth { err, .. } => {
            walk_error_path(err)
        }
        ignore::Error::WithPath { path, .. } => Some(path.clone()),
        ignore::Error::Loop { child, .. } => Some(child.clone()),
        ignore::Error::Io(_)
        | ignore::Error::Glob { .. }
        | ignore::Error::UnrecognizedFileType(_)
        | ignore::Error::InvalidDefinition => None,
    }
}

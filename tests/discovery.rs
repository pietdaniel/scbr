use std::fs;
use std::path::{Path, PathBuf};

use scbr::discovery::{DiscoveryError, DiscoveryOptions, SUPPORTED_EXTENSIONS, discover_sources};
use scbr::language::Language;
use tempfile::TempDir;

fn write_file(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, "// source\n").unwrap();
    path
}

fn discovered_paths(root: &Path, options: DiscoveryOptions) -> Vec<PathBuf> {
    discover_sources(&[root.to_path_buf()], &options)
        .unwrap()
        .files
        .into_iter()
        .map(|file| file.path)
        .collect()
}

#[test]
fn discovers_every_supported_extension() {
    let temp = TempDir::new().unwrap();
    for extension in SUPPORTED_EXTENSIONS {
        write_file(temp.path(), &format!("sample.{extension}"));
    }
    write_file(temp.path(), "README.md");
    write_file(temp.path(), "image.png");

    let result =
        discover_sources(&[temp.path().to_path_buf()], &DiscoveryOptions::default()).unwrap();

    assert_eq!(result.files.len(), SUPPORTED_EXTENSIONS.len());
    assert!(result.walk_errors.is_empty());
    assert_eq!(
        result
            .files
            .iter()
            .find(|file| file.path.ends_with("sample.tsx"))
            .unwrap()
            .language,
        Language::TypeScript
    );
    assert_eq!(
        result
            .files
            .iter()
            .find(|file| file.path.ends_with("sample.kts"))
            .unwrap()
            .language,
        Language::Kotlin
    );
}

#[test]
fn paths_are_sorted_and_overlapping_inputs_are_deduplicated() {
    let temp = TempDir::new().unwrap();
    let zed = write_file(temp.path(), "z.rs");
    let alpha = write_file(temp.path(), "nested/a.py");

    let result = discover_sources(
        &[temp.path().to_path_buf(), zed.clone(), alpha.clone()],
        &DiscoveryOptions::default(),
    )
    .unwrap();

    let expected = vec![
        fs::canonicalize(alpha).unwrap(),
        fs::canonicalize(zed).unwrap(),
    ];
    assert_eq!(
        result
            .files
            .into_iter()
            .map(|file| file.path)
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn respects_nested_ignore_files_and_hidden_paths_by_default() {
    let temp = TempDir::new().unwrap();
    write_file(temp.path(), "visible.go");
    write_file(temp.path(), "generated.js");
    write_file(temp.path(), "nested/ignored.rs");
    write_file(temp.path(), ".hidden.py");
    write_file(temp.path(), ".secret/secret.kt");
    fs::write(temp.path().join(".gitignore"), "generated.js\n").unwrap();
    fs::write(temp.path().join("nested/.ignore"), "ignored.rs\n").unwrap();

    let paths = discovered_paths(temp.path(), DiscoveryOptions::default());

    assert_eq!(
        paths,
        vec![fs::canonicalize(temp.path().join("visible.go")).unwrap()]
    );
}

#[test]
fn can_include_ignored_and_hidden_paths() {
    let temp = TempDir::new().unwrap();
    let visible = write_file(temp.path(), "visible.go");
    let ignored = write_file(temp.path(), "generated.js");
    let nested_ignored = write_file(temp.path(), "nested/ignored.rs");
    let hidden = write_file(temp.path(), ".hidden.py");
    let hidden_nested = write_file(temp.path(), ".secret/secret.kt");
    fs::write(temp.path().join(".gitignore"), "generated.js\n").unwrap();
    fs::write(temp.path().join("nested/.ignore"), "ignored.rs\n").unwrap();

    let paths = discovered_paths(
        temp.path(),
        DiscoveryOptions {
            include_hidden: true,
            include_ignored: true,
            exclude: Vec::new(),
        },
    );
    let mut expected = vec![visible, ignored, nested_ignored, hidden, hidden_nested]
        .into_iter()
        .map(fs::canonicalize)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    expected.sort();

    assert_eq!(paths, expected);
}

#[test]
fn exclude_globs_match_directories_relative_paths_and_basenames() {
    let temp = TempDir::new().unwrap();
    let keep = write_file(temp.path(), "src/keep.rs");
    write_file(temp.path(), "src/api.generated.ts");
    write_file(temp.path(), "vendor/dependency.go");
    write_file(temp.path(), "nested/test_fixture.py");

    let paths = discovered_paths(
        temp.path(),
        DiscoveryOptions {
            exclude: vec![
                "vendor".into(),
                "*.generated.ts".into(),
                "nested/test_fixture.py".into(),
            ],
            ..DiscoveryOptions::default()
        },
    );

    assert_eq!(paths, vec![fs::canonicalize(keep).unwrap()]);
}

#[test]
fn explicit_hidden_or_ignored_file_is_discovered() {
    let temp = TempDir::new().unwrap();
    let hidden = write_file(temp.path(), ".hidden.py");
    fs::write(temp.path().join(".gitignore"), ".hidden.py\n").unwrap();

    let result =
        discover_sources(std::slice::from_ref(&hidden), &DiscoveryOptions::default()).unwrap();

    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].path, fs::canonicalize(hidden).unwrap());
}

#[test]
fn reports_missing_and_unsupported_explicit_inputs() {
    let temp = TempDir::new().unwrap();
    let missing = temp.path().join("missing.rs");
    let unsupported = write_file(temp.path(), "README.md");

    let missing_error =
        discover_sources(std::slice::from_ref(&missing), &DiscoveryOptions::default())
            .expect_err("missing input should fail");
    assert!(matches!(
        missing_error,
        DiscoveryError::InputNotFound { ref path } if path == &missing
    ));
    assert!(missing_error.to_string().contains("does not exist"));

    let unsupported_error = discover_sources(
        std::slice::from_ref(&unsupported),
        &DiscoveryOptions::default(),
    )
    .expect_err("unsupported explicit file should fail");
    assert!(matches!(
        unsupported_error,
        DiscoveryError::UnsupportedFile { ref path } if path == &unsupported
    ));
    assert!(
        unsupported_error
            .to_string()
            .contains("supported extensions")
    );
}

#[test]
fn reports_empty_inputs_and_invalid_exclude_globs() {
    assert!(matches!(
        discover_sources(&[], &DiscoveryOptions::default()).unwrap_err(),
        DiscoveryError::NoInputs
    ));

    let temp = TempDir::new().unwrap();
    let error = discover_sources(
        &[temp.path().to_path_buf()],
        &DiscoveryOptions {
            exclude: vec!["[unclosed".into()],
            ..DiscoveryOptions::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error, DiscoveryError::InvalidExcludeGlob { .. }));
}

#[test]
fn directory_without_supported_files_is_a_successful_empty_result() {
    let temp = TempDir::new().unwrap();
    write_file(temp.path(), "README.md");

    let result =
        discover_sources(&[temp.path().to_path_buf()], &DiscoveryOptions::default()).unwrap();

    assert!(result.files.is_empty());
    assert!(result.walk_errors.is_empty());
}

#[test]
fn malformed_nested_ignore_file_does_not_lose_sibling_files() {
    let temp = TempDir::new().unwrap();
    let source = write_file(temp.path(), "visible.rs");
    write_file(temp.path(), "nested/may_be_skipped.rs");
    fs::write(temp.path().join("nested/.ignore"), "dangling\\\n").unwrap();

    let result =
        discover_sources(&[temp.path().to_path_buf()], &DiscoveryOptions::default()).unwrap();

    assert!(
        result
            .files
            .iter()
            .any(|file| file.path == fs::canonicalize(&source).unwrap())
    );
    // `ignore` versions differ on whether this malformed pattern is reported
    // or treated literally. Either behavior must preserve unaffected siblings.
}

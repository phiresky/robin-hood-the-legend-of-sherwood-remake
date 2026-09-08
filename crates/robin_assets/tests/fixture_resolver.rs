#[allow(dead_code)]
mod support;

use std::ffi::OsStr;
use std::path::Path;

use support::{FixtureKind, resolve_data_path_from};

#[test]
fn resolver_requires_an_explicit_data_directory() {
    let error = resolve_data_path_from(
        None,
        Path::new("Data/Levels/Dem_Lei_MP.scb"),
        FixtureKind::File,
    )
    .unwrap_err();

    assert!(error.contains("ROBINHOOD_DATA_DIR is not set"));
    assert!(error.contains("--ignored"));
}

#[test]
fn resolver_finds_a_required_fixture_below_the_data_root() {
    let root = tempfile::tempdir().unwrap();
    let levels = root.path().join("Data/Levels");
    std::fs::create_dir_all(&levels).unwrap();
    let script = levels.join("Test.scb");
    std::fs::write(&script, b"fixture").unwrap();

    let resolved = resolve_data_path_from(
        Some(root.path().as_os_str()),
        Path::new("Data/Levels/Test.scb"),
        FixtureKind::File,
    )
    .unwrap();

    assert_eq!(resolved, script.canonicalize().unwrap());
}

#[test]
fn resolver_finds_a_required_fixture_directory() {
    let root = tempfile::tempdir().unwrap();
    let levels = root.path().join("Data/Levels");
    std::fs::create_dir_all(&levels).unwrap();

    let resolved = resolve_data_path_from(
        Some(root.path().as_os_str()),
        Path::new("Data/Levels"),
        FixtureKind::Directory,
    )
    .unwrap();

    assert_eq!(resolved, levels.canonicalize().unwrap());
}

#[test]
fn resolver_rejects_a_missing_required_fixture() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("Data")).unwrap();

    let error = resolve_data_path_from(
        Some(root.path().as_os_str()),
        Path::new("Data/Levels/Missing.scb"),
        FixtureKind::File,
    )
    .unwrap_err();

    assert!(error.contains("required original-data file Data/Levels/Missing.scb"));
    assert!(error.contains("cannot be resolved"));
}

#[test]
fn resolver_rejects_an_absolute_fixture_path() {
    let error = resolve_data_path_from(
        Some(OsStr::new("unused")),
        Path::new("/Data/Levels/Test.scb"),
        FixtureKind::File,
    )
    .unwrap_err();

    assert!(error.contains("must be relative to ROBINHOOD_DATA_DIR"));
}

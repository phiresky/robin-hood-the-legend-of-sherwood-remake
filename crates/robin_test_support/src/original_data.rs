use std::ffi::OsStr;
use std::path::{Path, PathBuf};

const DATA_DIR_ENV: &str = "ROBINHOOD_DATA_DIR";

#[derive(Clone, Copy, Debug)]
pub enum FixtureKind {
    File,
    Directory,
}

impl FixtureKind {
    fn description(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
        }
    }

    fn matches(self, path: &Path) -> bool {
        match self {
            Self::File => path.is_file(),
            Self::Directory => path.is_dir(),
        }
    }
}

pub fn data_file(relative_path: impl AsRef<Path>) -> PathBuf {
    require_data_path(relative_path.as_ref(), FixtureKind::File)
}

pub fn data_directory(relative_path: impl AsRef<Path>) -> PathBuf {
    require_data_path(relative_path.as_ref(), FixtureKind::Directory)
}

fn require_data_path(relative_path: &Path, kind: FixtureKind) -> PathBuf {
    resolve_data_path_from(
        std::env::var_os(DATA_DIR_ENV).as_deref(),
        relative_path,
        kind,
    )
    .unwrap_or_else(|error| panic!("{error}"))
}

pub fn resolve_data_path_from(
    data_dir: Option<&OsStr>,
    relative_path: &Path,
    kind: FixtureKind,
) -> Result<PathBuf, String> {
    if relative_path.is_absolute() {
        return Err(format!(
            "test fixture path must be relative to {DATA_DIR_ENV}, got {}",
            relative_path.display()
        ));
    }

    let data_dir = data_dir.ok_or_else(|| {
        format!(
            "{DATA_DIR_ENV} is not set; this ignored test requires original game data. \
             Set it to an extracted game-data root containing Data/ and rerun the test \
             with --ignored (see README.md, Testing with original game data)."
        )
    })?;
    let data_dir = PathBuf::from(data_dir);
    let root = data_dir.canonicalize().map_err(|error| {
        format!(
            "{DATA_DIR_ENV}={} cannot be resolved: {error}",
            data_dir.display()
        )
    })?;
    if !root.is_dir() {
        return Err(format!(
            "{DATA_DIR_ENV}={} is not a directory",
            root.display()
        ));
    }

    let path = resolve_distribution_casing(&root, relative_path).map_err(|error| {
        format!(
            "required original-data {} {} (from {DATA_DIR_ENV}={}) cannot be resolved: {error}",
            kind.description(),
            relative_path.display(),
            root.display()
        )
    })?;
    let resolved = path.canonicalize().map_err(|error| {
        format!(
            "required original-data {} {} (from {DATA_DIR_ENV}={}) cannot be resolved: {error}",
            kind.description(),
            relative_path.display(),
            root.display()
        )
    })?;
    if !kind.matches(&resolved) {
        return Err(format!(
            "required original-data {} {} resolved to {}, but it is not a {}",
            kind.description(),
            relative_path.display(),
            resolved.display(),
            kind.description()
        ));
    }

    Ok(resolved)
}

// Original distributions use Data/, DATA/, and locale-specific data/. Prefer
// exact names; only a unique ASCII case-insensitive sibling may substitute.
fn resolve_distribution_casing(root: &Path, relative: &Path) -> std::io::Result<PathBuf> {
    let mut resolved = root.to_path_buf();
    for component in relative.components() {
        let exact = resolved.join(component.as_os_str());
        match std::fs::symlink_metadata(&exact) {
            Ok(_) => {
                resolved = exact;
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut matched = None;
        for entry in std::fs::read_dir(&resolved)? {
            let entry = entry?;
            let name = entry.file_name();
            if name
                .to_str()
                .zip(component.as_os_str().to_str())
                .is_some_and(|(actual, requested)| actual.eq_ignore_ascii_case(requested))
            {
                if matched.is_some() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("ambiguous original-data path {}", exact.display()),
                    ));
                }
                matched = Some(entry.path());
            }
        }
        resolved = matched.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("missing original-data path {}", exact.display()),
            )
        })?;
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_configuration_is_required() {
        let error = resolve_data_path_from(None, Path::new("Data"), FixtureKind::Directory)
            .expect_err("missing explicit root must fail");
        assert!(error.contains("ROBINHOOD_DATA_DIR is not set"));
    }

    #[test]
    fn absolute_fixture_paths_are_rejected() {
        let error = resolve_data_path_from(
            Some(OsStr::new(env!("CARGO_MANIFEST_DIR"))),
            Path::new(env!("CARGO_MANIFEST_DIR")),
            FixtureKind::Directory,
        )
        .expect_err("fixture paths are root-relative");
        assert!(error.contains("must be relative"));
    }

    #[test]
    fn fixture_paths_accept_original_distribution_casing() {
        let root = Some(OsStr::new(env!("CARGO_MANIFEST_DIR")));
        let exact =
            resolve_data_path_from(root, Path::new("src/lib.rs"), FixtureKind::File).unwrap();
        assert_eq!(
            resolve_data_path_from(root, Path::new("SRC/Lib.RS"), FixtureKind::File).unwrap(),
            exact
        );
        assert!(
            resolve_data_path_from(root, Path::new("SRC"), FixtureKind::Directory)
                .unwrap()
                .is_dir()
        );
        assert!(
            resolve_data_path_from(root, Path::new("SRC/MISSING.RS"), FixtureKind::File)
                .unwrap_err()
                .contains("required original-data file")
        );
        assert!(
            resolve_data_path_from(root, Path::new("SRC"), FixtureKind::File)
                .unwrap_err()
                .contains("not a file")
        );
    }

    #[test]
    fn resolver_checks_the_expected_kind_and_missing_files() {
        let root = Some(OsStr::new(env!("CARGO_MANIFEST_DIR")));
        assert!(
            resolve_data_path_from(root, Path::new("src"), FixtureKind::Directory)
                .unwrap()
                .is_dir()
        );
        assert!(
            resolve_data_path_from(root, Path::new("src/lib.rs"), FixtureKind::File)
                .unwrap()
                .is_file()
        );
        assert!(
            resolve_data_path_from(root, Path::new("src"), FixtureKind::File)
                .unwrap_err()
                .contains("not a file")
        );
        assert!(
            resolve_data_path_from(root, Path::new("src/lib.rs"), FixtureKind::Directory)
                .unwrap_err()
                .contains("not a directory")
        );
        assert!(
            resolve_data_path_from(
                root,
                Path::new("missing-required-fixture"),
                FixtureKind::File
            )
            .unwrap_err()
            .contains("cannot be resolved")
        );
    }
}

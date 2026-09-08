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

    let path = root.join(relative_path);
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

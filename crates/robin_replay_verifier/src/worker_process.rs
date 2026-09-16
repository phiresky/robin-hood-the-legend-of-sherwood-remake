//! One-job process boundary shared by the verifier binary and subprocess tests.
//!
//! These checks supplement the worker's bwrap namespaces, read-only bind
//! mounts, prlimit limits and wall timeout. An allocator abort during hostile
//! bitcode decoding cannot be caught inside this process.

use nix::sys::resource::{Resource, getrlimit, rlim_t, setrlimit};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Seek as _, Write as _};
use std::path::{Component, Path, PathBuf};

/// `--job FILE --replay FILE --content-root DIR --result FILE`
/// with optional `--checkpoints FILE` for a pre-created sidecar output.
#[derive(clap::Parser, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[command(name = "robin-replay-verifier", disable_help_flag = true)]
#[serde(deny_unknown_fields)]
pub struct WorkerPaths {
    #[arg(long)]
    pub job: PathBuf,
    #[arg(long)]
    pub replay: PathBuf,
    #[arg(long)]
    pub content_root: PathBuf,
    #[arg(long)]
    pub result: PathBuf,
    #[arg(long)]
    pub checkpoints: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerPathError {
    #[error("unknown verifier argument `{0}`")]
    UnknownFlag(String),
    #[error(transparent)]
    Arguments(#[from] clap::Error),
    #[error("verifier path for `{flag}` is not a normalized absolute path: `{path}`")]
    UnsafePath { flag: &'static str, path: PathBuf },
    #[error("verifier path for `{flag}` is not a regular non-symlink file: `{path}`")]
    NotAFile { flag: &'static str, path: PathBuf },
    #[error("verifier path for `{flag}` is not a directory: `{path}`")]
    NotADirectory { flag: &'static str, path: PathBuf },
    #[error("verifier paths `{first}` and `{second}` alias the same target")]
    AliasedPaths {
        first: &'static str,
        second: &'static str,
    },
    #[error("checking verifier path `{path}` failed: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl WorkerPaths {
    /// Parse the required flag/value pairs and optional checkpoint output. The iterator does not include
    /// argv[0]. No positional arguments or `--flag=value` spellings are
    /// accepted, keeping the worker invocation unambiguous.
    pub fn parse_flags(args: impl IntoIterator<Item = OsString>) -> Result<Self, WorkerPathError> {
        let args = args.into_iter().collect::<Vec<_>>();
        for flag in args.iter().step_by(2) {
            if flag.as_encoded_bytes().contains(&b'=') {
                return Err(WorkerPathError::UnknownFlag(
                    flag.to_string_lossy().into_owned(),
                ));
            }
        }
        let paths = <Self as clap::Parser>::try_parse_from(
            std::iter::once(OsString::from("robin-replay-verifier")).chain(args),
        )?;
        paths.validate()?;
        Ok(paths)
    }

    pub fn validate(&self) -> Result<(), WorkerPathError> {
        let mut files = vec![
            ("--job", &self.job),
            ("--replay", &self.replay),
            ("--result", &self.result),
        ];
        if let Some(path) = &self.checkpoints {
            files.push(("--checkpoints", path));
        }
        for &(flag, path) in &files {
            validate_normalized_absolute(flag, path)?;
            validate_existing_components(flag, path)?;
            let metadata =
                std::fs::symlink_metadata(path).map_err(|source| WorkerPathError::Io {
                    path: path.clone(),
                    source,
                })?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(WorkerPathError::NotAFile {
                    flag,
                    path: path.clone(),
                });
            }
        }
        validate_normalized_absolute("--content-root", &self.content_root)?;
        validate_existing_components("--content-root", &self.content_root)?;
        if !self.content_root.is_dir() {
            return Err(WorkerPathError::NotADirectory {
                flag: "--content-root",
                path: self.content_root.clone(),
            });
        }

        let mut lexical = BTreeMap::<&Path, &'static str>::new();
        for &(flag, path) in &files {
            if let Some(first) = lexical.insert(path.as_path(), flag) {
                return Err(WorkerPathError::AliasedPaths {
                    first,
                    second: flag,
                });
            }
        }
        reject_inode_aliases(&files)
    }
}

fn validate_normalized_absolute(flag: &'static str, path: &Path) -> Result<(), WorkerPathError> {
    let canonical_lexical = path
        .components()
        .fold(PathBuf::new(), |mut rebuilt, component| {
            rebuilt.push(component.as_os_str());
            rebuilt
        });
    let safe = path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        && canonical_lexical.as_os_str() == path.as_os_str();
    if !safe || path.file_name().is_none() {
        return Err(WorkerPathError::UnsafePath {
            flag,
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn validate_existing_components(flag: &'static str, path: &Path) -> Result<(), WorkerPathError> {
    let mut current = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir => continue,
            Component::Normal(component) => current.push(component),
            _ => {
                return Err(WorkerPathError::UnsafePath {
                    flag,
                    path: path.to_path_buf(),
                });
            }
        }
        let metadata =
            std::fs::symlink_metadata(&current).map_err(|source| WorkerPathError::Io {
                path: current.clone(),
                source,
            })?;
        if metadata.file_type().is_symlink() {
            return Err(WorkerPathError::UnsafePath {
                flag,
                path: path.to_path_buf(),
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn reject_inode_aliases(paths: &[(&'static str, &PathBuf)]) -> Result<(), WorkerPathError> {
    use std::os::unix::fs::MetadataExt as _;

    let mut identities = BTreeMap::<(u64, u64), &'static str>::new();
    for &(flag, path) in paths {
        let metadata = std::fs::metadata(path).map_err(|source| WorkerPathError::Io {
            path: path.clone(),
            source,
        })?;
        if let Some(first) = identities.insert((metadata.dev(), metadata.ino()), flag) {
            return Err(WorkerPathError::AliasedPaths {
                first,
                second: flag,
            });
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn reject_inode_aliases(_paths: &[(&'static str, &PathBuf)]) -> Result<(), WorkerPathError> {
    Ok(())
}

/// Hard compiled ceilings installed before reading any input. The sandbox's
/// prlimit values may only lower these.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapResourceLimits {
    pub address_space_bytes: u64,
    pub cpu_seconds: u64,
    pub output_file_bytes: u64,
    pub open_files: u64,
}

impl Default for BootstrapResourceLimits {
    fn default() -> Self {
        Self {
            address_space_bytes: 4 * 1024 * 1024 * 1024,
            cpu_seconds: 10 * 60,
            output_file_bytes: 1024 * 1024 * 1024,
            open_files: 64,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResourceLimitError {
    #[error("bootstrap resource limit `{field}` must be nonzero")]
    Zero { field: &'static str },
    #[error("installing bootstrap resource limit `{field}` failed: {source}")]
    Set {
        field: &'static str,
        #[source]
        source: nix::errno::Errno,
    },
}

/// Install process-local hard limits. This must be the first fallible worker
/// action.
pub fn apply_bootstrap_resource_limits(
    limits: BootstrapResourceLimits,
) -> Result<(), ResourceLimitError> {
    for (field, resource, requested) in [
        (
            "address_space_bytes",
            Resource::RLIMIT_AS,
            limits.address_space_bytes,
        ),
        ("cpu_seconds", Resource::RLIMIT_CPU, limits.cpu_seconds),
        (
            "output_file_bytes",
            Resource::RLIMIT_FSIZE,
            limits.output_file_bytes,
        ),
        ("open_files", Resource::RLIMIT_NOFILE, limits.open_files),
        ("core_bytes", Resource::RLIMIT_CORE, 0),
    ] {
        if requested == 0 && field != "core_bytes" {
            return Err(ResourceLimitError::Zero { field });
        }
        lower_resource_limit(field, resource, requested)?;
    }
    Ok(())
}

fn lower_resource_limit(
    field: &'static str,
    resource: Resource,
    requested: u64,
) -> Result<(), ResourceLimitError> {
    let (_, current_hard) =
        getrlimit(resource).map_err(|source| ResourceLimitError::Set { field, source })?;
    let requested: rlim_t = requested;
    let new_limit = requested.min(current_hard);
    setrlimit(resource, new_limit, new_limit)
        .map_err(|source| ResourceLimitError::Set { field, source })
}

#[derive(Debug, thiserror::Error)]
pub enum AtomicOutputError {
    #[error("output has {observed} bytes, limit {limit}")]
    Limit { observed: usize, limit: usize },
    #[error("worker output target is not an existing regular file: `{0}`")]
    InvalidTarget(PathBuf),
    #[error("worker output I/O failed for `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Replace the contents of the pre-created bounded result file.
pub fn write_truncated_output(
    target: &Path,
    bytes: &[u8],
    max_bytes: usize,
) -> Result<(), AtomicOutputError> {
    if bytes.len() > max_bytes {
        return Err(AtomicOutputError::Limit {
            observed: bytes.len(),
            limit: max_bytes,
        });
    }
    let metadata = std::fs::symlink_metadata(target).map_err(|source| AtomicOutputError::Io {
        path: target.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AtomicOutputError::InvalidTarget(target.to_path_buf()));
    }
    let io_error = |source| AtomicOutputError::Io {
        path: target.to_path_buf(),
        source,
    };
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .open(target)
        .map_err(io_error)?;
    output.set_len(0).map_err(io_error)?;
    output.seek(std::io::SeekFrom::Start(0)).map_err(io_error)?;
    output.write_all(bytes).map_err(io_error)?;
    output.flush().map_err(io_error)?;
    Ok(())
}

/// Clear the pre-created output before processing untrusted input.
pub fn truncate_output(target: &Path) -> Result<(), AtomicOutputError> {
    write_truncated_output(target, &[], 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, WorkerPaths) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("job.json"), b"job").unwrap();
        std::fs::write(directory.path().join("replay.rhrec"), b"replay").unwrap();
        std::fs::write(directory.path().join("result.json"), b"stale result").unwrap();
        std::fs::create_dir(directory.path().join("content")).unwrap();
        let paths = WorkerPaths {
            checkpoints: None,
            job: directory.path().join("job.json"),
            replay: directory.path().join("replay.rhrec"),
            content_root: directory.path().join("content"),
            result: directory.path().join("result.json"),
        };
        (directory, paths)
    }

    fn args(paths: &WorkerPaths) -> Vec<OsString> {
        [
            ("--job", &paths.job),
            ("--replay", &paths.replay),
            ("--content-root", &paths.content_root),
            ("--result", &paths.result),
        ]
        .into_iter()
        .flat_map(|(flag, path)| [OsString::from(flag), path.as_os_str().to_owned()])
        .collect()
    }

    #[test]
    fn frozen_four_flag_cli_rejects_unknown_equals_and_aliases() {
        let (_directory, paths) = fixture();
        assert_eq!(WorkerPaths::parse_flags(args(&paths)).unwrap(), paths);

        assert!(matches!(
            WorkerPaths::parse_flags([OsString::from("--unknown"), OsString::from("/tmp/x")]),
            Err(WorkerPathError::Arguments(_))
        ));
        assert!(matches!(
            WorkerPaths::parse_flags([OsString::from("--job=/tmp/x")]),
            Err(WorkerPathError::UnknownFlag(_))
        ));

        let mut aliased = paths.clone();
        aliased.result = aliased.replay.clone();
        assert!(matches!(
            aliased.validate(),
            Err(WorkerPathError::AliasedPaths { .. })
        ));

        let mut file_root = paths;
        file_root.content_root = file_root.job.clone();
        assert!(matches!(
            file_root.validate(),
            Err(WorkerPathError::NotADirectory { .. })
        ));
    }

    #[test]
    fn worker_paths_require_exact_lexical_normalization() {
        assert!(validate_normalized_absolute("--job", Path::new("/run/robin/job")).is_ok());
        for hostile in [
            "/run//robin/job",
            "/run/./robin/job",
            "/run/robin/../job",
            "//run/robin/job",
            "/run/robin/job/",
        ] {
            assert!(matches!(
                validate_normalized_absolute("--job", Path::new(hostile)),
                Err(WorkerPathError::UnsafePath { .. })
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_input_and_output_are_rejected_as_aliases() {
        let (_directory, paths) = fixture();
        std::fs::remove_file(&paths.result).unwrap();
        std::fs::hard_link(&paths.replay, &paths.result).unwrap();
        assert!(matches!(
            paths.validate(),
            Err(WorkerPathError::AliasedPaths { .. })
        ));
    }

    #[test]
    fn precreated_output_is_bounded_and_replaced() {
        let (_directory, paths) = fixture();
        write_truncated_output(&paths.result, b"result", 6).unwrap();
        assert_eq!(std::fs::read(&paths.result).unwrap(), b"result");
        assert!(matches!(
            write_truncated_output(&paths.result, b"too large", 3),
            Err(AtomicOutputError::Limit { .. })
        ));
        truncate_output(&paths.result).unwrap();
        assert!(std::fs::read(&paths.result).unwrap().is_empty());
    }
}

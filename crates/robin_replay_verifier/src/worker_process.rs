//! One-job process boundary shared by the verifier binary and subprocess
//! adversarial tests.
//!
//! These checks supplement, but do not replace, the deployment cgroup,
//! read-only bind mounts, private network namespace, seccomp profile, and
//! supervisor wall timeout. In particular, an allocator abort during hostile
//! bitcode decoding cannot be caught inside this process.
//!
//! The supervisor must expose these already-opened artifacts through an
//! immutable private bind/memfd namespace for the lifetime of the child.
//! Path and inode admission rejects aliases at startup; it cannot make a
//! shared, mutable host directory immune to post-validation replacement.

use nix::sys::resource::{Resource, getrlimit, rlim_t, setrlimit};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Seek as _, Write as _};
use std::path::{Component, Path, PathBuf};

const REQUIRED_PATH_FLAGS: [&str; 6] = [
    "--request",
    "--replay",
    "--config",
    "--starting-campaign",
    "--final-campaign",
    "--result",
];

/// Frozen six-path invocation. Canonical replay bytes remain outside the
/// signed request document; the worker re-hashes the one artifact and verifies
/// those exact bytes without manufacturing a second replay.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerPaths {
    pub request: PathBuf,
    pub replay: PathBuf,
    pub config: PathBuf,
    pub starting_campaign: PathBuf,
    pub final_campaign: PathBuf,
    pub result: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerPathError {
    #[error("unknown verifier argument `{0}`")]
    UnknownFlag(String),
    #[error("verifier argument `{0}` occurs more than once")]
    DuplicateFlag(String),
    #[error("verifier argument `{0}` has no path value")]
    MissingValue(String),
    #[error("required verifier argument `{0}` is absent")]
    MissingFlag(&'static str),
    #[error("verifier path for `{flag}` is not a normalized absolute path: `{path}`")]
    UnsafePath { flag: &'static str, path: PathBuf },
    #[error("verifier input for `{flag}` is not a regular non-symlink file: `{path}`")]
    InvalidInput { flag: &'static str, path: PathBuf },
    #[error("verifier output for `{flag}` is not a regular non-symlink file: `{path}`")]
    InvalidOutput { flag: &'static str, path: PathBuf },
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
    /// Parse exactly the frozen six flag/value pairs. The iterator does not
    /// include argv[0]. No positional arguments or `--flag=value` spellings
    /// are accepted, keeping backend invocation and audit logs unambiguous.
    pub fn parse_flags(args: impl IntoIterator<Item = OsString>) -> Result<Self, WorkerPathError> {
        let mut values = BTreeMap::<String, PathBuf>::new();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let flag = flag
                .into_string()
                .map_err(|flag| WorkerPathError::UnknownFlag(flag.to_string_lossy().into()))?;
            if !REQUIRED_PATH_FLAGS.contains(&flag.as_str()) {
                return Err(WorkerPathError::UnknownFlag(flag));
            }
            let value = args
                .next()
                .ok_or_else(|| WorkerPathError::MissingValue(flag.clone()))?;
            if values.insert(flag.clone(), PathBuf::from(value)).is_some() {
                return Err(WorkerPathError::DuplicateFlag(flag));
            }
        }

        let mut take = |flag: &'static str| {
            values
                .remove(flag)
                .ok_or(WorkerPathError::MissingFlag(flag))
        };
        let paths = Self {
            request: take("--request")?,
            replay: take("--replay")?,
            config: take("--config")?,
            starting_campaign: take("--starting-campaign")?,
            final_campaign: take("--final-campaign")?,
            result: take("--result")?,
        };
        paths.validate()?;
        Ok(paths)
    }

    pub fn validate(&self) -> Result<(), WorkerPathError> {
        let inputs = [
            ("--request", &self.request),
            ("--replay", &self.replay),
            ("--config", &self.config),
            ("--starting-campaign", &self.starting_campaign),
        ];
        let outputs = [
            ("--final-campaign", &self.final_campaign),
            ("--result", &self.result),
        ];

        for &(flag, path) in &inputs {
            validate_normalized_absolute(flag, path)?;
            validate_existing_components(flag, path)?;
            let metadata =
                std::fs::symlink_metadata(path).map_err(|source| WorkerPathError::Io {
                    path: path.clone(),
                    source,
                })?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(WorkerPathError::InvalidInput {
                    flag,
                    path: path.clone(),
                });
            }
        }
        for &(flag, path) in &outputs {
            validate_normalized_absolute(flag, path)?;
            validate_existing_components(flag, path)?;
            let metadata =
                std::fs::symlink_metadata(path).map_err(|source| WorkerPathError::Io {
                    path: path.clone(),
                    source,
                })?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(WorkerPathError::InvalidOutput {
                    flag,
                    path: path.clone(),
                });
            }
        }

        let all = inputs.into_iter().chain(outputs).collect::<Vec<_>>();
        let mut lexical = BTreeMap::<&Path, &'static str>::new();
        for &(flag, path) in &all {
            if let Some(first) = lexical.insert(path.as_path(), flag) {
                return Err(WorkerPathError::AliasedPaths {
                    first,
                    second: flag,
                });
            }
        }
        reject_inode_aliases(&all)?;
        Ok(())
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

/// Hard compiled ceilings installed before reading request, config, replay,
/// or campaign bytes. Per-job signed/configured limits may only lower these.
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
/// action. A cgroup remains mandatory for RSS and wall-time enforcement.
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
    #[error("atomic output has {observed} bytes, limit {limit}")]
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

/// Replace the contents of one pre-created bounded output memfd.
///
/// The complete document is serialized and capped by the caller before this
/// function is invoked. No path is created or renamed: the supervisor exposes
/// distinct anonymous memfds at the six-path child interface.
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
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .open(target)
        .map_err(|source| AtomicOutputError::Io {
            path: target.to_path_buf(),
            source,
        })?;
    output.set_len(0).map_err(|source| AtomicOutputError::Io {
        path: target.to_path_buf(),
        source,
    })?;
    output
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|source| AtomicOutputError::Io {
            path: target.to_path_buf(),
            source,
        })?;
    output
        .write_all(bytes)
        .map_err(|source| AtomicOutputError::Io {
            path: target.to_path_buf(),
            source,
        })?;
    output.flush().map_err(|source| AtomicOutputError::Io {
        path: target.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Defensively clear a pre-created output before processing untrusted input.
pub fn truncate_output(target: &Path) -> Result<(), AtomicOutputError> {
    write_truncated_output(target, &[], 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, WorkerPaths) {
        let directory = tempfile::tempdir().unwrap();
        for name in ["request.json", "replay.rhrec", "config.json", "start.bin"] {
            std::fs::write(directory.path().join(name), name).unwrap();
        }
        let paths = WorkerPaths {
            request: directory.path().join("request.json"),
            replay: directory.path().join("replay.rhrec"),
            config: directory.path().join("config.json"),
            starting_campaign: directory.path().join("start.bin"),
            final_campaign: directory.path().join("final.bin"),
            result: directory.path().join("result.json"),
        };
        std::fs::write(&paths.final_campaign, b"stale campaign").unwrap();
        std::fs::write(&paths.result, b"stale result").unwrap();
        (directory, paths)
    }

    #[test]
    fn frozen_six_path_cli_rejects_unknown_missing_duplicate_and_aliases() {
        let (_directory, paths) = fixture();
        let args = [
            ("--request", &paths.request),
            ("--replay", &paths.replay),
            ("--config", &paths.config),
            ("--starting-campaign", &paths.starting_campaign),
            ("--final-campaign", &paths.final_campaign),
            ("--result", &paths.result),
        ]
        .into_iter()
        .flat_map(|(flag, path)| [OsString::from(flag), path.as_os_str().to_owned()]);
        assert_eq!(WorkerPaths::parse_flags(args).unwrap(), paths);

        assert!(matches!(
            WorkerPaths::parse_flags([OsString::from("--unknown"), OsString::from("/tmp/x")]),
            Err(WorkerPathError::UnknownFlag(_))
        ));

        let mut aliased = paths;
        aliased.config = aliased.replay.clone();
        assert!(matches!(
            aliased.validate(),
            Err(WorkerPathError::AliasedPaths { .. })
        ));
    }

    #[test]
    fn worker_paths_require_exact_lexical_normalization() {
        assert!(validate_normalized_absolute("--request", Path::new("/run/robin/request")).is_ok());
        for hostile in [
            "/run//robin/request",
            "/run/./robin/request",
            "/run/robin/../request",
            "//run/robin/request",
            "/run/robin/request/",
        ] {
            assert!(matches!(
                validate_normalized_absolute("--request", Path::new(hostile)),
                Err(WorkerPathError::UnsafePath { .. })
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_inputs_are_rejected_as_aliases() {
        let (_directory, paths) = fixture();
        std::fs::remove_file(&paths.config).unwrap();
        std::fs::hard_link(&paths.replay, &paths.config).unwrap();
        assert!(matches!(
            paths.validate(),
            Err(WorkerPathError::AliasedPaths { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_input_and_output_are_rejected_as_aliases() {
        let (_directory, paths) = fixture();
        std::fs::remove_file(&paths.final_campaign).unwrap();
        std::fs::hard_link(&paths.replay, &paths.final_campaign).unwrap();
        assert!(matches!(
            paths.validate(),
            Err(WorkerPathError::AliasedPaths { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_outputs_are_rejected_as_aliases() {
        let (_directory, paths) = fixture();
        std::fs::remove_file(&paths.result).unwrap();
        std::fs::hard_link(&paths.final_campaign, &paths.result).unwrap();
        assert!(matches!(
            paths.validate(),
            Err(WorkerPathError::AliasedPaths { .. })
        ));
    }

    #[test]
    fn precreated_memfd_outputs_are_bounded_and_replaced() {
        let (_directory, paths) = fixture();
        write_truncated_output(&paths.result, b"result", 6).unwrap();
        assert_eq!(std::fs::read(&paths.result).unwrap(), b"result");
        assert!(matches!(
            write_truncated_output(&paths.final_campaign, b"too large", 3),
            Err(AtomicOutputError::Limit { .. })
        ));
        assert_eq!(
            std::fs::read(&paths.final_campaign).unwrap(),
            b"stale campaign"
        );
        truncate_output(&paths.final_campaign).unwrap();
        assert!(std::fs::read(&paths.final_campaign).unwrap().is_empty());
    }
}

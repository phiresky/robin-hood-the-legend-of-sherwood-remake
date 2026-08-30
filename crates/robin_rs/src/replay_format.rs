//! Game-facing replay loading facade.
//!
//! The canonical codec and server/verifier admission contract live in
//! [`robin_replay_format`]. This module adds two application-only concerns:
//!
//! - local developer JSONL/path loading, visibly separate from production;
//! - isolated public compact validation before the game process/wasm instance
//!   decodes the exact bytes a second time.

pub use robin_replay_format::*;

/// Exact source commit used by multiplayer artifact selection.
pub const ENGINE_SOURCE_COMMIT: &str = env!("ROBIN_GIT_COMMIT");

#[derive(Debug, thiserror::Error)]
pub enum ReplayLoadError {
    #[error(transparent)]
    Compact(#[from] robin_replay_format::FormatError),
    #[error("local replay I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(not(target_arch = "wasm32"))]
    #[error("local JSONL replay decode failed: {0}")]
    LocalJsonl(String),
    #[cfg(not(target_arch = "wasm32"))]
    #[error("isolated replay admission rejected the artifact: {0}")]
    AdmissionRejected(String),
    #[cfg(not(target_arch = "wasm32"))]
    #[error("replay admission exhausted its {stage} resource limit: {detail}")]
    ResourceLimit { stage: &'static str, detail: String },
    #[cfg(not(target_arch = "wasm32"))]
    #[error("this platform cannot securely contain replay admission: {0}")]
    ContainmentUnavailable(String),
    #[cfg(not(target_arch = "wasm32"))]
    #[error("isolated replay admission protocol failed: {0}")]
    WorkerProtocol(String),
    #[cfg(target_arch = "wasm32")]
    #[error("browser compact replay was not validated by the isolated Web Worker")]
    BrowserWorkerValidationRequired,
    #[cfg(target_arch = "wasm32")]
    #[error("browser replay loading accepts only an inline compact replay")]
    BrowserCompactOnly,
}

/// Load user-supplied compact bytes only after an isolated validator has
/// accepted the exact digest. Native uses a short-lived child process with
/// OS memory/CPU limits. Browser builds require the shell's dedicated Worker
/// to install a one-shot digest proof before this call.
pub fn decode_compact_for_public_playback(
    text: &str,
) -> Result<(String, robin_engine::replay::ReplayData), ReplayLoadError> {
    #[cfg(not(target_arch = "wasm32"))]
    validate_in_native_child(text)?;
    #[cfg(target_arch = "wasm32")]
    consume_browser_worker_proof(text)?;

    // The isolated worker validated and canonically re-encoded these exact
    // immutable bytes. Repeating typed decode here is safe: collection/string
    // sizes and total work were already proven under external containment.
    robin_replay_format::decode_compact_for_admission(text).map_err(Into::into)
}

/// Explicitly local CLI/developer loader. Production network/server code must
/// call the canonical crate's bounded admission API and has no JSONL branch.
pub fn load_replay_spec(spec: &str) -> Result<robin_engine::replay::ReplayData, ReplayLoadError> {
    if spec.starts_with(COMPACT_PREFIX) {
        return decode_compact_for_public_playback(spec).map(|(_, replay)| replay);
    }

    #[cfg(target_arch = "wasm32")]
    return Err(ReplayLoadError::BrowserCompactOnly);

    #[cfg(not(target_arch = "wasm32"))]
    {
        // JSONL is a visibly named, trusted local developer lane. It is never
        // auto-detected from bytes and is unavailable to network admission.
        if spec.ends_with(".rhrec.jsonl") {
            let replay = robin_engine::replay::ReplayData::from_file(spec)
                .map_err(ReplayLoadError::LocalJsonl)?;
            robin_replay_format::validate_replay_data(&replay)?;
            return Ok(replay);
        }

        // Every other path is a production compact artifact. Bound acquisition
        // before allocating a String so hostile local/URL-derived paths cannot
        // bypass the codec's transport preflight with `read_to_string`.
        use std::io::Read as _;
        let limit = DEFAULT_REPLAY_ADMISSION_LIMITS.max_input_bytes;
        let file = std::fs::File::open(spec)?;
        if file.metadata()?.len() > u64::try_from(limit).unwrap_or(u64::MAX) {
            return Err(robin_replay_format::FormatError::LimitExceeded {
                kind: ReplayLimitKind::CompactInputBytes,
                observed: limit.saturating_add(1),
                limit,
            }
            .into());
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(file.metadata()?.len())
                .unwrap_or(limit)
                .min(limit),
        );
        file.take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(robin_replay_format::FormatError::LimitExceeded {
                kind: ReplayLimitKind::CompactInputBytes,
                observed: bytes.len(),
                limit,
            }
            .into());
        }
        let contents = std::str::from_utf8(&bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("compact replay is not UTF-8: {error}"),
            )
        })?;
        decode_compact_for_public_playback(contents).map(|(_, replay)| replay)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum AdmissionWorkerReply {
    Accepted { sha256: String },
    Rejected { error: String },
}

#[cfg(not(target_arch = "wasm32"))]
const ADMISSION_WORKER_ARG: &str = "--internal-replay-admission-worker";
#[cfg(not(target_arch = "wasm32"))]
const ADMISSION_WORKER_WALL_TIME: std::time::Duration = std::time::Duration::from_secs(15);
#[cfg(not(target_arch = "wasm32"))]
const ADMISSION_WORKER_REPLY_LIMIT: usize = 16 * 1024;

/// Hidden native child entry point. The game binary dispatches here before
/// tracing, asset loading, clap, windowing, or networking.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_native_admission_worker() -> i32 {
    use sha2::Digest as _;
    use std::io::Read as _;

    let limit = DEFAULT_REPLAY_ADMISSION_LIMITS.max_input_bytes;
    let mut bytes = Vec::new();
    let read_result = std::io::stdin()
        .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes);
    let reply = match read_result {
        Err(error) => AdmissionWorkerReply::Rejected {
            error: format!("read compact replay: {error}"),
        },
        Ok(_) if bytes.len() > limit => AdmissionWorkerReply::Rejected {
            error: format!(
                "compact replay {:?} observed {}, limit is {}",
                ReplayLimitKind::CompactInputBytes,
                bytes.len(),
                limit
            ),
        },
        Ok(_) => match std::str::from_utf8(&bytes) {
            Err(error) => AdmissionWorkerReply::Rejected {
                error: format!("compact replay is not UTF-8: {error}"),
            },
            Ok(text) => match robin_replay_format::decode_compact_for_admission(text) {
                Ok(_) => AdmissionWorkerReply::Accepted {
                    sha256: format!("{:x}", sha2::Sha256::digest(&bytes)),
                },
                Err(error) => AdmissionWorkerReply::Rejected {
                    error: error.to_string(),
                },
            },
        },
    };
    match serde_json::to_writer(std::io::stdout().lock(), &reply) {
        Ok(()) => 0,
        Err(_) => 2,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_in_native_child(text: &str) -> Result<(), ReplayLoadError> {
    use sha2::Digest as _;
    use std::io::{Read as _, Write as _};
    use std::process::{Command, Stdio};

    preflight_compact_transport(text, &DEFAULT_REPLAY_ADMISSION_LIMITS)?;
    let executable = std::env::current_exe().map_err(|error| {
        ReplayLoadError::WorkerProtocol(format!("resolve current executable: {error}"))
    })?;
    let mut command = Command::new(executable);
    command
        .arg(ADMISSION_WORKER_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_native_worker_limits(&mut command)?;
    let mut child = command.spawn().map_err(|error| {
        ReplayLoadError::WorkerProtocol(format!("spawn admission worker: {error}"))
    })?;
    child
        .stdin
        .take()
        .ok_or_else(|| ReplayLoadError::WorkerProtocol("worker stdin is unavailable".into()))?
        .write_all(text.as_bytes())
        .map_err(|error| ReplayLoadError::WorkerProtocol(format!("write worker input: {error}")))?;

    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait().map_err(|error| {
            ReplayLoadError::WorkerProtocol(format!("wait for admission worker: {error}"))
        })? {
            Some(status) => break status,
            None if started.elapsed() >= ADMISSION_WORKER_WALL_TIME => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ReplayLoadError::ResourceLimit {
                    stage: "wall-time",
                    detail: format!(
                        "limit of {} seconds exceeded",
                        ADMISSION_WORKER_WALL_TIME.as_secs()
                    ),
                });
            }
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    };
    let mut output = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| ReplayLoadError::WorkerProtocol("worker stdout is unavailable".into()))?
        .take((ADMISSION_WORKER_REPLY_LIMIT + 1) as u64)
        .read_to_end(&mut output)
        .map_err(|error| ReplayLoadError::WorkerProtocol(format!("read worker reply: {error}")))?;
    if output.len() > ADMISSION_WORKER_REPLY_LIMIT {
        return Err(ReplayLoadError::ResourceLimit {
            stage: "worker-output",
            detail: format!(
                "observed at least {} bytes, limit is {}",
                output.len(),
                ADMISSION_WORKER_REPLY_LIMIT
            ),
        });
    }
    if !status.success() {
        return Err(ReplayLoadError::ResourceLimit {
            stage: "worker-process",
            detail: format!("worker exited abnormally with {status}"),
        });
    }
    let reply: AdmissionWorkerReply = serde_json::from_slice(&output).map_err(|error| {
        ReplayLoadError::WorkerProtocol(format!("decode worker reply: {error}"))
    })?;
    match reply {
        AdmissionWorkerReply::Rejected { error } => Err(ReplayLoadError::AdmissionRejected(error)),
        AdmissionWorkerReply::Accepted { sha256 } => {
            let actual = format!("{:x}", sha2::Sha256::digest(text.as_bytes()));
            if sha256 != actual {
                return Err(ReplayLoadError::WorkerProtocol(
                    "worker accepted a different replay digest".into(),
                ));
            }
            Ok(())
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
fn configure_native_worker_limits(
    command: &mut std::process::Command,
) -> Result<(), ReplayLoadError> {
    use std::os::unix::process::CommandExt as _;

    // The worker has no game assets/window/network stack. The 384 MiB ceiling
    // leaves >4x the bounded binary-buffer overlap while containing bitcode's
    // pre-validation collection allocation multiplier. Browser admission uses
    // the identical 384 MiB linear-memory maximum.
    const ADDRESS_SPACE_BYTES: libc::rlim_t = 384 * 1024 * 1024;
    unsafe {
        command.pre_exec(|| {
            set_limit(libc::RLIMIT_AS, ADDRESS_SPACE_BYTES)?;
            set_limit(libc::RLIMIT_CPU, 10)?;
            set_limit(libc::RLIMIT_FSIZE, 1024 * 1024)?;
            set_limit(libc::RLIMIT_NOFILE, 64)?;
            Ok(())
        });
    }
    Ok(())
}

#[cfg(all(not(target_arch = "wasm32"), unix))]
fn set_limit(resource: libc::__rlimit_resource_t, value: libc::rlim_t) -> std::io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    if unsafe { libc::setrlimit(resource, &limit) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(unix)))]
fn configure_native_worker_limits(
    _command: &mut std::process::Command,
) -> Result<(), ReplayLoadError> {
    // A subprocess without an address-space/job memory ceiling can still OOM
    // the machine. Fail closed until a platform-specific hard limit (Windows
    // Job Object, sandbox profile, etc.) is installed before `spawn`.
    Err(ReplayLoadError::ContainmentUnavailable(
        "native replay admission currently requires Unix setrlimit containment".into(),
    ))
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static BROWSER_WORKER_PROOF: std::cell::RefCell<Option<[u8; 32]>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Install a one-shot digest after the shell's isolated wasm Worker accepted
/// the exact canonical compact replay. This performs transport preflight only;
/// it never zstd/bitcode-decodes in the main wasm instance.
#[cfg(target_arch = "wasm32")]
pub fn mark_browser_worker_validated(text: &str) -> Result<(), ReplayLoadError> {
    use sha2::Digest as _;

    preflight_compact_transport(text, &DEFAULT_REPLAY_ADMISSION_LIMITS)?;
    let digest: [u8; 32] = sha2::Sha256::digest(text.as_bytes()).into();
    BROWSER_WORKER_PROOF.with(|proof| *proof.borrow_mut() = Some(digest));
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn consume_browser_worker_proof(text: &str) -> Result<(), ReplayLoadError> {
    use sha2::Digest as _;

    let digest: [u8; 32] = sha2::Sha256::digest(text.as_bytes()).into();
    let accepted = BROWSER_WORKER_PROOF.with(|proof| proof.borrow_mut().take()) == Some(digest);
    if accepted {
        Ok(())
    } else {
        Err(ReplayLoadError::BrowserWorkerValidationRequired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_worker_argument_is_not_a_user_cli_format() {
        assert!(ADMISSION_WORKER_ARG.starts_with("--internal-"));
    }
}

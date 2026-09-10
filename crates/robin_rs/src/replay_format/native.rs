//! Native replay admission process protocol and operating-system containment.

use super::{
    LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS, ReplayLimitKind, ReplayLoadError,
    decode_compact_for_local_playback, preflight_compact_transport,
};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum AdmissionWorkerReply {
    Accepted { sha256: String },
    Rejected { error: String },
}

const ADMISSION_WORKER_ARG: &str = "--internal-replay-admission-worker";
const ADMISSION_WORKER_WALL_TIME: std::time::Duration = std::time::Duration::from_secs(15);
const ADMISSION_WORKER_REPLY_LIMIT: usize = 16 * 1024;
const ADMISSION_WORKER_ERROR_LIMIT: usize = 8 * 1024;

fn bounded_worker_error(error: impl std::fmt::Display) -> String {
    let mut message = error.to_string();
    if message.len() <= ADMISSION_WORKER_ERROR_LIMIT {
        return message;
    }
    let mut end = ADMISSION_WORKER_ERROR_LIMIT;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message.push_str(" [truncated]");
    message
}

/// Hidden native child entry point. The game binary dispatches here before
/// tracing, asset loading, clap, windowing, or networking.
pub fn run_native_admission_worker() -> i32 {
    use sha2::Digest as _;
    use std::io::Read as _;

    let limit = LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS.max_input_bytes;
    let mut bytes = Vec::new();
    // Apply the limits again inside the worker. The normal parent installs
    // them in `pre_exec`, before any child code can run; this second gate is
    // defence in depth and makes a directly invoked hidden worker fail closed
    // instead of becoming an unconstrained decoder.
    let containment = configure_current_native_worker_limits();
    let read_result = containment.and_then(|()| {
        std::io::stdin()
            .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
            .read_to_end(&mut bytes)
    });
    let reply = match read_result {
        Err(error) => AdmissionWorkerReply::Rejected {
            error: bounded_worker_error(format_args!(
                "establish containment and read compact replay: {error}"
            )),
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
                error: bounded_worker_error(format_args!("compact replay is not UTF-8: {error}")),
            },
            Ok(text) => match decode_compact_for_local_playback(text) {
                Ok(_) => AdmissionWorkerReply::Accepted {
                    sha256: hex::encode(sha2::Sha256::digest(&bytes)),
                },
                Err(error) => AdmissionWorkerReply::Rejected {
                    error: bounded_worker_error(error),
                },
            },
        },
    };
    match serde_json::to_writer(std::io::stdout().lock(), &reply) {
        Ok(()) => 0,
        Err(_) => 2,
    }
}

pub(super) fn validate_in_native_child(text: &str) -> Result<(), ReplayLoadError> {
    use sha2::Digest as _;
    use std::io::{Read as _, Write as _};
    use std::process::{Command, Stdio};

    preflight_compact_transport(text, &LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS)?;
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
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| ReplayLoadError::WorkerProtocol("worker stdin is unavailable".into()))?
        .write_all(text.as_bytes());
    if let Err(error) = write_result {
        // A worker killed by its memory/CPU limit commonly closes stdin while
        // the parent is still writing. Reap it here rather than leaving a
        // zombie and report the failure as containment, not ordinary I/O.
        let _ = child.kill();
        let status = child.wait().ok();
        return Err(ReplayLoadError::ResourceLimit {
            stage: "worker-input",
            detail: match status {
                Some(status) => format!("worker closed input ({error}); exit status {status}"),
                None => format!("worker closed input ({error})"),
            },
        });
    }

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
            let actual = hex::encode(sha2::Sha256::digest(text.as_bytes()));
            if sha256 != actual {
                return Err(ReplayLoadError::WorkerProtocol(
                    "worker accepted a different replay digest".into(),
                ));
            }
            Ok(())
        }
    }
}

#[cfg(unix)]
fn configure_native_worker_limits(
    command: &mut std::process::Command,
) -> Result<(), ReplayLoadError> {
    use std::os::unix::process::CommandExt as _;

    unsafe {
        command.pre_exec(configure_current_native_worker_limits);
    }
    Ok(())
}

#[cfg(unix)]
fn configure_current_native_worker_limits() -> std::io::Result<()> {
    // `libc::setrlimit` deliberately uses the platform ABI's resource type:
    // glibc exposes `__rlimit_resource_t`, while musl and most other Unix
    // targets expose `c_int`. Keep each constant's native inferred type at
    // the call site instead of naming a libc-internal, target-specific alias.
    macro_rules! set_limit {
        ($resource:expr, $value:expr) => {{
            let value: libc::rlim_t = $value;
            let limit = libc::rlimit {
                rlim_cur: value,
                rlim_max: value,
            };
            if unsafe { libc::setrlimit($resource, &limit) } == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        }};
    }

    // The worker has no game assets/window/network stack. The 384 MiB ceiling
    // leaves >4x the bounded binary-buffer overlap while containing bitcode's
    // pre-validation collection allocation multiplier. Browser admission uses
    // the identical 384 MiB linear-memory maximum.
    // TODO: decouple admission from the full game executable: video-enabled
    // debug builds can exhaust this ceiling on code/shared-library mappings
    // alone. Preserve decode containment while isolating that dependency footprint.
    const ADDRESS_SPACE_BYTES: libc::rlim_t = 384 * 1024 * 1024;
    set_limit!(libc::RLIMIT_AS, ADDRESS_SPACE_BYTES)?;
    set_limit!(libc::RLIMIT_CPU, 10)?;
    set_limit!(libc::RLIMIT_FSIZE, 1024 * 1024)?;
    set_limit!(libc::RLIMIT_NOFILE, 64)
}

#[cfg(not(unix))]
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

#[cfg(not(unix))]
fn configure_current_native_worker_limits() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "this platform has no hard replay admission containment",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_worker_argument_is_not_a_user_cli_format() {
        assert!(ADMISSION_WORKER_ARG.starts_with("--internal-"));
    }

    #[test]
    fn worker_errors_are_bounded_on_utf8_boundaries() {
        let error = "é".repeat(ADMISSION_WORKER_ERROR_LIMIT);
        let bounded = bounded_worker_error(error);
        assert!(bounded.is_char_boundary(bounded.len()));
        assert!(bounded.len() <= ADMISSION_WORKER_ERROR_LIMIT + " [truncated]".len());
        assert!(bounded.ends_with(" [truncated]"));
    }
}

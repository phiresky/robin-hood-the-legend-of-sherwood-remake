//! Native replay admission process protocol and operating-system containment.

use super::{
    LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS, ReplayLimitKind, ReplayLoadError,
    decode_compact_for_local_playback, preflight_compact_transport,
};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum AdmissionWorkerReply {
    Accepted { sha256: String },
    Rejected { error: String },
}

const ADMISSION_WORKER_ARG: &str = "--internal-replay-admission-worker";
const ADMISSION_WORKER_WALL_TIME: std::time::Duration = std::time::Duration::from_secs(15);
const ADMISSION_WORKER_REPLY_LIMIT: usize = 16 * 1024;
// A control byte can expand to six JSON bytes (\u00xx). Reserve space for
// the reply envelope and truncation suffix before budgeting diagnostic text.
const ADMISSION_WORKER_ERROR_LIMIT: usize = (ADMISSION_WORKER_REPLY_LIMIT - 128) / 6;

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
    let child = command.spawn().map_err(|error| {
        ReplayLoadError::WorkerProtocol(format!("spawn admission worker: {error}"))
    })?;
    let output = exchange_with_worker(child, text, ADMISSION_WORKER_WALL_TIME)?;
    verify_worker_reply(&output, text)
}

fn verify_worker_reply(output: &[u8], text: &str) -> Result<(), ReplayLoadError> {
    use sha2::Digest as _;

    let reply: AdmissionWorkerReply = serde_json::from_slice(output).map_err(|error| {
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

fn exchange_with_worker(
    mut child: std::process::Child,
    text: &str,
    wall_time: std::time::Duration,
) -> Result<Vec<u8>, ReplayLoadError> {
    use std::io::{Read as _, Write as _};

    let mut stdin = child
        .stdin
        .take()
        .expect("admission worker stdin was piped");
    let stdout = child
        .stdout
        .take()
        .expect("admission worker stdout was piped");
    std::thread::scope(|scope| {
        let started = std::time::Instant::now();
        // Both pipes progress independently of the deadline monitor. Scoped
        // threads borrow the input without cloning a potentially large replay.
        let reader = match std::thread::Builder::new()
            .name("replay-admission-reply".into())
            .spawn_scoped(scope, move || {
                let mut output = Vec::new();
                stdout
                    .take((ADMISSION_WORKER_REPLY_LIMIT + 1) as u64)
                    .read_to_end(&mut output)
                    .map(|_| output)
            }) {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ReplayLoadError::WorkerProtocol(format!(
                    "spawn worker reply reader: {error}"
                )));
            }
        };
        let writer = match std::thread::Builder::new()
            .name("replay-admission-input".into())
            .spawn_scoped(scope, move || stdin.write_all(text.as_bytes()))
        {
            Ok(writer) => writer,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(ReplayLoadError::WorkerProtocol(format!(
                    "spawn worker input writer: {error}"
                )));
            }
        };
        let completion = (|| {
            loop {
                match child.try_wait().map_err(|error| {
                    ReplayLoadError::WorkerProtocol(format!("wait for admission worker: {error}"))
                })? {
                    Some(status) => return Ok(status),
                    None if started.elapsed() >= wall_time => {
                        return Err(ReplayLoadError::ResourceLimit {
                            stage: "wall-time",
                            detail: format!("limit of {} seconds exceeded", wall_time.as_secs()),
                        });
                    }
                    None => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            }
        })();
        // Reap before joining: killing the worker releases either blocked pipe.
        if completion.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let input = writer.join();
        let output = reader.join();
        let status = completion?;
        input
            .map_err(|_| ReplayLoadError::WorkerProtocol("worker input writer panicked".into()))?
            .map_err(|error| ReplayLoadError::ResourceLimit {
                stage: "worker-input",
                detail: format!("worker closed input ({error}); exit status {status}"),
            })?;
        let output = output
            .map_err(|_| ReplayLoadError::WorkerProtocol("worker reply reader panicked".into()))?
            .map_err(|error| {
                ReplayLoadError::WorkerProtocol(format!("read worker reply: {error}"))
            })?;
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
        Ok(output)
    })
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
    fn worker_acceptance_is_bound_to_the_exact_input_bytes() {
        use sha2::Digest as _;
        let text = "exact replay bytes 🦊";
        let reply = serde_json::to_vec(&AdmissionWorkerReply::Accepted {
            sha256: hex::encode(sha2::Sha256::digest(text.as_bytes())),
        })
        .unwrap();
        verify_worker_reply(&reply, text).unwrap();
        for changed in ["", "exact replay bytes", "exact replay bytes 🦊\n"] {
            assert!(matches!(
                verify_worker_reply(&reply, changed),
                Err(ReplayLoadError::WorkerProtocol(_))
            ));
        }
    }

    #[test]
    fn worker_rejection_retains_its_diagnostic() {
        let message = "invalid replay\n\"details\": é";
        let reply = serde_json::to_vec(&AdmissionWorkerReply::Rejected {
            error: message.into(),
        })
        .unwrap();
        assert!(
            matches!(verify_worker_reply(&reply, "input"), Err(ReplayLoadError::AdmissionRejected(error)) if error == message)
        );
    }

    #[test]
    fn malformed_and_ambiguous_worker_replies_fail_closed() {
        for reply in [
            "",
            "null",
            "{}",
            "[]",
            "not JSON",
            r#"{"status":"unknown"}"#,
            r#"{"status":"accepted"}"#,
            r#"{"status":"accepted","sha256":false}"#,
            r#"{"status":"accepted","sha256":"bad"}"#,
            r#"{"status":"rejected","error":"bad","sha256":"also accepted"}"#,
            r#"{"status":"rejected","error":"bad","extra":true}"#,
            r#"{"status":"rejected","error":"one","error":"two"}"#,
            r#"{"status":"rejected","error":"bad"} trailing"#,
        ] {
            assert!(
                matches!(
                    verify_worker_reply(reply.as_bytes(), "input"),
                    Err(ReplayLoadError::WorkerProtocol(_))
                ),
                "{reply}"
            );
        }
        use sha2::Digest as _;
        let mut reply = serde_json::json!({
            "status": "accepted",
            "sha256": hex::encode(sha2::Sha256::digest(b"input")),
            "error": "also rejected",
        });
        assert!(matches!(
            verify_worker_reply(&serde_json::to_vec(&reply).unwrap(), "input"),
            Err(ReplayLoadError::WorkerProtocol(_))
        ));
        reply.as_object_mut().unwrap().remove("error");
        verify_worker_reply(&serde_json::to_vec(&reply).unwrap(), "input").unwrap();
    }

    #[cfg(unix)]
    fn shell_worker(script: &str) -> std::process::Child {
        use std::process::{Command, Stdio};
        Command::new("sh")
            .args(["-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn test worker")
    }

    #[cfg(unix)]
    #[test]
    fn worker_reply_is_drained_before_waiting_for_exit() {
        let output = exchange_with_worker(
            shell_worker("printf '%12288s' ''"),
            "",
            std::time::Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(output, vec![b' '; 12288]);
    }

    #[cfg(unix)]
    #[test]
    fn oversized_worker_reply_is_bounded_without_a_pipe_deadlock() {
        let error = exchange_with_worker(
            shell_worker("printf '%32768s' ''"),
            "",
            std::time::Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ReplayLoadError::ResourceLimit {
                stage: "worker-output",
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn worker_timeout_reaps_process_and_joins_reply_reader() {
        let error = exchange_with_worker(
            shell_worker("while :; do :; done"),
            "",
            std::time::Duration::from_millis(50),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ReplayLoadError::ResourceLimit {
                stage: "wall-time",
                ..
            }
        ));
        let error = exchange_with_worker(
            shell_worker("exit 7"),
            "",
            std::time::Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ReplayLoadError::ResourceLimit {
                stage: "worker-process",
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn worker_deadline_includes_a_blocked_input_write() {
        let input = "x".repeat(2 * 1024 * 1024);
        let started = std::time::Instant::now();
        let error = exchange_with_worker(
            shell_worker("while :; do :; done"),
            &input,
            std::time::Duration::from_millis(50),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ReplayLoadError::ResourceLimit {
                stage: "wall-time",
                ..
            }
        ));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn worker_pipes_progress_when_output_precedes_input_consumption() {
        let input = "x".repeat(2 * 1024 * 1024);
        let output = exchange_with_worker(
            shell_worker("printf '%12288s' ''; cat >/dev/null; printf done"),
            &input,
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let mut expected = vec![b' '; 12288];
        expected.extend_from_slice(b"done");
        assert_eq!(output, expected);
    }

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

    #[test]
    fn escaped_worker_diagnostics_fit_the_reply_budget() {
        for character in (0u8..=127).map(char::from).chain(['é', '🦊']) {
            let error = character.to_string().repeat(ADMISSION_WORKER_REPLY_LIMIT);
            let reply = AdmissionWorkerReply::Rejected {
                error: bounded_worker_error(error),
            };
            let bytes = serde_json::to_vec(&reply).unwrap();
            assert!(bytes.len() <= ADMISSION_WORKER_REPLY_LIMIT, "{character:?}");
            assert!(matches!(
                serde_json::from_slice::<AdmissionWorkerReply>(&bytes).unwrap(),
                AdmissionWorkerReply::Rejected { .. }
            ));
        }
    }
}

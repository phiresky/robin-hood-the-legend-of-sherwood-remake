//! Native replay admission process protocol and operating-system containment.

use crate::{LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS, ReplayLimitKind, preflight_compact_transport};

#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    #[error(transparent)]
    Compact(#[from] crate::FormatError),
    #[error("isolated replay admission rejected the artifact: {0}")]
    AdmissionRejected(String),
    #[error("replay admission exhausted its {stage} resource limit: {detail}")]
    ResourceLimit { stage: &'static str, detail: String },
    #[error("this platform cannot securely contain replay admission: {0}")]
    ContainmentUnavailable(String),
    #[error("isolated replay admission protocol failed: {0}")]
    WorkerProtocol(String),
}

fn decode_compact_for_local_playback(text: &str) -> Result<(), crate::FormatError> {
    let (_, data) = crate::decode_compact_for_local_playback(text)?;
    if let Some(package) = &data.header().spellforge_package {
        robin_spellforge::validate_package(package).map_err(|error| {
            crate::FormatError::InvalidLayout(format!("invalid replay Spellforge package: {error}"))
        })?;
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum AdmissionWorkerReply {
    Accepted {
        sha256: String,
        protocol: u32,
        engine: String,
        spellforge_abi: String,
        decoder: String,
    },
    Rejected {
        error: String,
    },
}

#[cfg(windows)]
pub const HELPER_NAME: &str = "robin-replay-admission.exe";
#[cfg(not(windows))]
pub const HELPER_NAME: &str = "robin-replay-admission";
const ADMISSION_PROTOCOL: u32 = 1;
const ADMISSION_WORKER_WALL_TIME: std::time::Duration = std::time::Duration::from_secs(15);
const ADMISSION_WORKER_REPLY_LIMIT: usize = 16 * 1024;
// A control byte can expand to six JSON bytes (\u00xx). Reserve space for
// the reply envelope and truncation suffix before budgeting diagnostic text.
const ADMISSION_WORKER_ERROR_LIMIT: usize = (ADMISSION_WORKER_REPLY_LIMIT - 128) / 6;

fn decoder_identity() -> String {
    use sha2::Digest as _;
    let mut digest = sha2::Sha256::new();
    digest.update(include_bytes!("native_admission.rs"));
    digest.update(include_bytes!("lib.rs"));
    digest.update(env!("ROBIN_CARGO_LOCK_SHA256"));
    hex::encode(digest.finalize())
}

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

/// Entry point of the dedicated native helper. No client/window/audio modules
/// are linked into this binary; directly invoking it still establishes limits.
pub fn run_native_admission_worker() -> i32 {
    use sha2::Digest as _;
    use std::io::Read as _;

    let limit = LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS.max_input_bytes;
    let mut bytes = Vec::new();
    // Apply the limits again inside the worker. The normal parent installs
    // them in `pre_exec`, before any child code can run; this second gate is
    // defence in depth and makes a directly invoked helper fail closed
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
                    protocol: ADMISSION_PROTOCOL,
                    engine: crate::ENGINE_VERSION_HASH.into(),
                    spellforge_abi: robin_spellforge::spellforge_vm_abi().into(),
                    decoder: decoder_identity(),
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

pub fn validate_in_native_child(text: &str) -> Result<(), AdmissionError> {
    let executable = std::env::current_exe().map_err(|error| {
        AdmissionError::WorkerProtocol(format!("resolve current executable: {error}"))
    })?;
    validate_next_to(text, &executable)
}

/// Validate through the matching helper beside an explicitly owned launcher.
/// This also supports frozen executable snapshots; all containment and reply
/// checks are identical to the ordinary current-executable path.
pub fn validate_next_to(text: &str, executable: &std::path::Path) -> Result<(), AdmissionError> {
    use std::process::{Command, Stdio};
    preflight_compact_transport(text, &LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS)?;
    let helper = helper_next_to(executable)?;
    let mut command = Command::new(helper);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_native_worker_limits(&mut command)?;
    let child = command.spawn().map_err(|error| {
        AdmissionError::WorkerProtocol(format!("spawn admission worker: {error}"))
    })?;
    let output = exchange_with_worker(child, text, ADMISSION_WORKER_WALL_TIME)?;
    verify_worker_reply(&output, text)
}

/// Installed tools and the game share a sibling helper. Cargo test/example
/// binaries resolve through their known artifact layout to that profile. Never search
/// PATH or the working directory, and never fall back to the game executable.
pub fn helper_next_to(executable: &std::path::Path) -> Result<std::path::PathBuf, AdmissionError> {
    if !executable.is_absolute() {
        return Err(AdmissionError::WorkerProtocol(
            "native helper discovery requires an absolute executable path".into(),
        ));
    }
    let directory = executable.parent().ok_or_else(|| {
        AdmissionError::WorkerProtocol("executable has no parent directory".into())
    })?;
    let sibling = directory.join(HELPER_NAME);
    if sibling.is_file() {
        return Ok(sibling);
    }
    if matches!(
        directory.file_name().and_then(|name| name.to_str()),
        Some("deps" | "examples")
    ) {
        if let Some(profile) = directory.parent() {
            let development = profile.join(HELPER_NAME);
            if development.is_file() {
                return Ok(development);
            }
        }
    }
    // Cargo's artifact-cache layout executes tests from
    // PROFILE/build/PACKAGE/FINGERPRINT/out instead of PROFILE/deps.
    // Recognize only that complete layout, not an arbitrary ancestor search.
    if directory.file_name().is_some_and(|name| name == "out")
        && directory
            .parent()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.len() == 16 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        && directory
            .ancestors()
            .nth(3)
            .and_then(|path| path.file_name())
            .is_some_and(|name| name == "build")
        && let Some(profile) = directory.ancestors().nth(4)
    {
        let development = profile.join(HELPER_NAME);
        if development.is_file() {
            return Ok(development);
        }
    }
    Err(AdmissionError::WorkerProtocol(format!(
        "missing native replay admission helper beside {}; install the matching package or build it with cargo build -p robin_replay_format --features native-admission --bin robin-replay-admission",
        executable.display()
    )))
}

fn verify_worker_reply(output: &[u8], text: &str) -> Result<(), AdmissionError> {
    use sha2::Digest as _;

    let reply: AdmissionWorkerReply = serde_json::from_slice(output)
        .map_err(|error| AdmissionError::WorkerProtocol(format!("decode worker reply: {error}")))?;
    match reply {
        AdmissionWorkerReply::Rejected { error } => Err(AdmissionError::AdmissionRejected(error)),
        AdmissionWorkerReply::Accepted {
            sha256,
            protocol,
            engine,
            spellforge_abi,
            decoder,
        } => {
            if protocol != ADMISSION_PROTOCOL
                || engine != crate::ENGINE_VERSION_HASH
                || spellforge_abi != robin_spellforge::spellforge_vm_abi()
                || decoder != decoder_identity()
            {
                return Err(AdmissionError::WorkerProtocol(
                    "incompatible replay admission helper identity; rebuild or install the matching helper".into(),
                ));
            }
            let actual = hex::encode(sha2::Sha256::digest(text.as_bytes()));
            if sha256 != actual {
                return Err(AdmissionError::WorkerProtocol(
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
) -> Result<Vec<u8>, AdmissionError> {
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
                return Err(AdmissionError::WorkerProtocol(format!(
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
                return Err(AdmissionError::WorkerProtocol(format!(
                    "spawn worker input writer: {error}"
                )));
            }
        };
        let completion = (|| {
            loop {
                match child.try_wait().map_err(|error| {
                    AdmissionError::WorkerProtocol(format!("wait for admission worker: {error}"))
                })? {
                    Some(status) => return Ok(status),
                    None if started.elapsed() >= wall_time => {
                        return Err(AdmissionError::ResourceLimit {
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
            .map_err(|_| AdmissionError::WorkerProtocol("worker input writer panicked".into()))?
            .map_err(|error| AdmissionError::ResourceLimit {
                stage: "worker-input",
                detail: format!("worker closed input ({error}); exit status {status}"),
            })?;
        let output = output
            .map_err(|_| AdmissionError::WorkerProtocol("worker reply reader panicked".into()))?
            .map_err(|error| {
                AdmissionError::WorkerProtocol(format!("read worker reply: {error}"))
            })?;
        if output.len() > ADMISSION_WORKER_REPLY_LIMIT {
            return Err(AdmissionError::ResourceLimit {
                stage: "worker-output",
                detail: format!(
                    "observed at least {} bytes, limit is {}",
                    output.len(),
                    ADMISSION_WORKER_REPLY_LIMIT
                ),
            });
        }
        if !status.success() {
            return Err(AdmissionError::ResourceLimit {
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
) -> Result<(), AdmissionError> {
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
    const ADDRESS_SPACE_BYTES: libc::rlim_t = 384 * 1024 * 1024;
    set_limit!(libc::RLIMIT_AS, ADDRESS_SPACE_BYTES)?;
    set_limit!(libc::RLIMIT_CPU, 10)?;
    set_limit!(libc::RLIMIT_FSIZE, 1024 * 1024)?;
    set_limit!(libc::RLIMIT_NOFILE, 64)
}

#[cfg(not(unix))]
fn configure_native_worker_limits(
    _command: &mut std::process::Command,
) -> Result<(), AdmissionError> {
    // A subprocess without an address-space/job memory ceiling can still OOM
    // the machine. Fail closed until a platform-specific hard limit (Windows
    // Job Object, sandbox profile, etc.) is installed before `spawn`.
    Err(AdmissionError::ContainmentUnavailable(
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
            protocol: ADMISSION_PROTOCOL,
            engine: crate::ENGINE_VERSION_HASH.into(),
            spellforge_abi: robin_spellforge::spellforge_vm_abi().into(),
            decoder: decoder_identity(),
        })
        .unwrap();
        verify_worker_reply(&reply, text).unwrap();
        for changed in ["", "exact replay bytes", "exact replay bytes 🦊\n"] {
            assert!(matches!(
                verify_worker_reply(&reply, changed),
                Err(AdmissionError::WorkerProtocol(_))
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
            matches!(verify_worker_reply(&reply, "input"), Err(AdmissionError::AdmissionRejected(error)) if error == message)
        );
    }

    #[test]
    fn matching_input_cannot_authorize_a_different_helper_build_or_protocol() {
        use sha2::Digest as _;
        let valid = serde_json::json!({
            "status": "accepted", "sha256": hex::encode(sha2::Sha256::digest(b"input")),
            "protocol": ADMISSION_PROTOCOL, "engine": crate::ENGINE_VERSION_HASH,
            "spellforge_abi": robin_spellforge::spellforge_vm_abi(),
            "decoder": decoder_identity(),
        });
        for (field, replacement) in [
            ("protocol", serde_json::json!(999)),
            ("engine", serde_json::json!("wrong-engine")),
            ("decoder", serde_json::json!("wrong-decoder")),
            ("spellforge_abi", serde_json::json!("wrong-vm")),
        ] {
            let mut wrong = valid.clone();
            wrong[field] = replacement;
            assert!(matches!(
                verify_worker_reply(&serde_json::to_vec(&wrong).unwrap(), "input"),
                Err(AdmissionError::WorkerProtocol(_))
            ));
        }
    }

    #[test]
    fn helper_discovery_requires_a_matching_install_or_cargo_profile_location() {
        assert!(helper_next_to(std::path::Path::new("robin")).is_err());
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        assert!(helper_next_to(&root.join("robin")).is_err());
        std::fs::write(root.join(HELPER_NAME), b"discovery fixture").unwrap();
        for launcher in [
            "robin",
            "renamed-client",
            "deps/test-binary",
            "examples/render-map",
            "build/robin_rs/0123456789abcdef/out/robin-test",
        ] {
            assert_eq!(
                helper_next_to(&root.join(launcher)).unwrap(),
                root.join(HELPER_NAME)
            );
        }
        assert!(helper_next_to(&root.join("unrelated/robin")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn closed_input_pipe_fails_without_releasing_admission() {
        let error = exchange_with_worker(
            shell_worker("exit 0"),
            &"x".repeat(1024 * 1024),
            std::time::Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AdmissionError::ResourceLimit {
                stage: "worker-input",
                ..
            }
        ));
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
                    Err(AdmissionError::WorkerProtocol(_))
                ),
                "{reply}"
            );
        }
        use sha2::Digest as _;
        let mut reply = serde_json::json!({
            "status": "accepted",
            "sha256": hex::encode(sha2::Sha256::digest(b"input")),
            "error": "also rejected",
            "protocol": ADMISSION_PROTOCOL,
            "engine": crate::ENGINE_VERSION_HASH,
            "spellforge_abi": robin_spellforge::spellforge_vm_abi(),
            "decoder": decoder_identity(),
        });
        assert!(matches!(
            verify_worker_reply(&serde_json::to_vec(&reply).unwrap(), "input"),
            Err(AdmissionError::WorkerProtocol(_))
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
            AdmissionError::ResourceLimit {
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
            AdmissionError::ResourceLimit {
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
            AdmissionError::ResourceLimit {
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
            AdmissionError::ResourceLimit {
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

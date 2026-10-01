//! Native uploader which outlives the game through an inherited liveness pipe.
use anyhow::Result;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const REPORTER_MODE: &str = "ROBIN_INTERNAL_CRASH_REPORTER";

/// Spawn the current executable in upload-only mode. Keep the returned child
/// (and its stdin writer) alive until shutdown; dropping it closes the pipe.
pub fn start() -> Result<Child> {
    let mut command = Command::new(std::env::current_exe()?);
    command.env(REPORTER_MODE, "1");
    spawn(command)
}

fn spawn(mut command: Command) -> Result<Child> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Survive a terminal signal delivered to the game's process group.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    Ok(command.spawn()?)
}

/// Called before any normal entry-point work; the helper never opens a game
/// window, touches player profiles, runs the updater, or spawns another helper.
pub fn run_if_requested() -> bool {
    if std::env::var_os(REPORTER_MODE).as_deref() != Some(std::ffi::OsStr::new("1")) {
        return false;
    }
    let mut cursor = None;
    monitor(
        std::io::stdin().lock(),
        || crate::bug_report::upload_pending_with_cursor(&mut cursor),
        || {},
    );
    true
}

pub(crate) fn note(message: &str) {
    // Keep a small local record even when no console or tracing subscriber exists.
    let result = (|| -> Result<()> {
        let directory = crate::bug_report::directory()?;
        std::fs::create_dir_all(&directory)?;
        let path = directory.join("uploader.log");
        let truncate = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 64 * 1024);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(!truncate)
            .truncate(truncate)
            .open(path)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        writeln!(
            file,
            "{now}: {}",
            crate::bug_report::bound_text(message.into(), 2048)
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        tracing::warn!("Cannot write uploader status: {error:#}");
    }
}

fn monitor(
    mut parent: impl Read,
    mut upload: impl FnMut() -> Result<crate::bug_report::UploadBatch>,
    ready: impl FnOnce(),
) {
    // Submit old reports promptly, but always perform a fresh pass after EOF:
    // a crash report may have been persisted while the first pass was running.
    match upload() {
        Ok(batch) => note(&format!(
            "Startup upload: {} submitted, {} remaining, busy={}",
            batch.submitted, batch.remaining, batch.busy
        )),
        Err(error) => note(&format!("Startup upload failed: {error:#}")),
    }
    ready();
    if let Err(error) = std::io::copy(&mut parent, &mut std::io::sink()) {
        note(&format!("Parent pipe closed with an error: {error}"));
    }
    let mut retries = [1, 5, 20].into_iter();
    loop {
        match upload() {
            Ok(batch) => {
                note(&format!(
                    "Shutdown upload: {} submitted, {} remaining, busy={}",
                    batch.submitted, batch.remaining, batch.busy
                ));
                if !batch.busy && batch.remaining == 0 {
                    break;
                }
                if batch.more || batch.submitted > 0 {
                    continue;
                }
            }
            Err(error) => note(&format!("Shutdown upload failed: {error:#}")),
        }
        let Some(seconds) = retries.next() else {
            note("Upload retries exhausted; pending reports retained for the next launch");
            break;
        };
        std::thread::sleep(Duration::from_secs(seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    fn wait_for(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn reporter_child() {
        let Some(root) = std::env::var_os("ROBIN_REPORTER_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let endpoint = std::env::var("ROBIN_REPORTER_TEST_ENDPOINT").unwrap();
        let mut cursor = None;
        monitor(
            std::io::stdin().lock(),
            || crate::bug_report::upload_pending_batch_at(&root, &endpoint, &mut cursor),
            || {
                std::fs::write(root.join("ready"), b"ready").unwrap();
            },
        );
        std::fs::write(root.join("finished"), b"finished").unwrap();
    }

    #[test]
    fn simulated_game_child() {
        let Some(root) = std::env::var_os("ROBIN_REPORTER_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "crash_reporter::tests::reporter_child",
            "--nocapture",
        ]);
        let _reporter = spawn(command).unwrap();
        wait_for(&root.join("ready"));
        let report = robin_run_protocol::diagnostics::DiagnosticReportV1 {
            schema_version: 1,
            kind: robin_run_protocol::diagnostics::DiagnosticKindV1::Panic,
            description: "game crashed before it could upload".into(),
            engine_commit: "test".into(),
            platform: "test".into(),
            occurred_at_unix_ms: 1,
            backtrace: None,
            recent_log: String::new(),
            attachments: Vec::new(),
            warnings: Vec::new(),
        };
        std::fs::write(
            root.join("crash.pending.json"),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
        // No Rust cleanup: the OS must close the liveness writer for us.
        std::process::exit(23);
    }

    #[test]
    fn reporter_survives_game_exit_and_retries_until_matching_receipt() {
        use sha2::{Digest, Sha256};
        let directory = tempfile::tempdir().unwrap();
        let reports = directory.path().join("robin_hood/reports");
        std::fs::create_dir_all(&reports).unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crash_reporter::tests::simulated_game_child",
                "--nocapture",
            ])
            .env("ROBIN_REPORTER_TEST_ROOT", &reports)
            .env(
                "ROBIN_REPORTER_TEST_ENDPOINT",
                format!("http://{}/diagnostics", server.server_addr()),
            )
            .env("XDG_DATA_HOME", directory.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(23));
        for status in [503, 202] {
            let mut request = server
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .expect("reporter upload after game exit");
            let mut bytes = Vec::new();
            request.as_reader().read_to_end(&mut bytes).unwrap();
            let report: robin_run_protocol::diagnostics::DiagnosticReportV1 =
                serde_json::from_slice(&zstd::stream::decode_all(bytes.as_slice()).unwrap())
                    .unwrap();
            assert_eq!(report.description, "game crashed before it could upload");
            assert!(reports.join("crash.pending.json").exists());
            let receipt = robin_run_protocol::diagnostics::DiagnosticReceiptV1 {
                schema_version: 1,
                report_id: hex::encode(Sha256::digest(&bytes)),
            };
            request
                .respond(
                    tiny_http::Response::from_string(serde_json::to_string(&receipt).unwrap())
                        .with_status_code(status),
                )
                .unwrap();
        }
        wait_for(&reports.join("finished"));
        assert!(!reports.join("crash.pending.json").exists());
        assert!(reports.join("crash.pending.submitted").exists());
    }

    #[test]
    fn damaged_reports_do_not_hide_later_batches() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..21 {
            std::fs::write(
                directory.path().join(format!("{index:02}.pending.json")),
                b"invalid JSON",
            )
            .unwrap();
        }
        let mut cursor = None;
        let first = crate::bug_report::upload_pending_batch_at(
            directory.path(),
            "http://127.0.0.1:1/diagnostics",
            &mut cursor,
        )
        .unwrap();
        assert!(first.more);
        assert_eq!(first.remaining, 21);
        assert_eq!(
            cursor.as_deref(),
            Some(std::ffi::OsStr::new("19.pending.json"))
        );
        let second = crate::bug_report::upload_pending_batch_at(
            directory.path(),
            "http://127.0.0.1:1/diagnostics",
            &mut cursor,
        )
        .unwrap();
        assert!(!second.more);
        assert_eq!(second.remaining, 21);
        assert!(cursor.is_none());
    }

    #[test]
    fn another_uploader_cannot_claim_a_locked_queue() {
        let directory = tempfile::tempdir().unwrap();
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(directory.path().join("upload.lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        let batch = crate::bug_report::upload_pending_at(
            directory.path(),
            "http://127.0.0.1:1/diagnostics",
        )
        .unwrap();
        assert!(batch.busy);
        assert_eq!(batch.submitted, 0);
        drop(lock);
        let batch = crate::bug_report::upload_pending_at(
            directory.path(),
            "http://127.0.0.1:1/diagnostics",
        )
        .unwrap();
        assert!(!batch.busy);
    }
}

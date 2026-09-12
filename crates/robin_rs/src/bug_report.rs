//! Durable native crash and bug report queue for the leaderboard VPS.
//! TODO: native report form, queue retention settings and minidump recovery.
use anyhow::{Context, Result};
use robin_run_protocol::diagnostics::{
    DiagnosticAttachmentV1, DiagnosticKindV1, DiagnosticReceiptV1, DiagnosticReportV1,
    MAX_DIAGNOSTIC_ATTACHMENT_BYTES, MAX_DIAGNOSTIC_BODY_BYTES,
};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const LOG_LIMIT: usize = 256 * 1024;
static LOG: Mutex<VecDeque<u8>> = Mutex::new(VecDeque::new());
static DROPPED_LOG_BYTES: AtomicUsize = AtomicUsize::new(0);
static REPLAY: Mutex<Option<PathBuf>> = Mutex::new(None);
static UPLOADING: AtomicBool = AtomicBool::new(false);
static MESSAGES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

pub(crate) fn record_log(bytes: &[u8]) {
    if let Ok(mut log) = LOG.try_lock() {
        let bytes = &bytes[bytes.len().saturating_sub(LOG_LIMIT)..];
        let remove = (log.len() + bytes.len()).saturating_sub(LOG_LIMIT);
        log.drain(..remove);
        log.extend(bytes);
    } else {
        DROPPED_LOG_BYTES.fetch_add(bytes.len(), Ordering::Relaxed);
    }
}
pub(crate) fn set_replay(path: &Path) {
    match REPLAY.lock() {
        Ok(mut replay) => *replay = Some(path.to_owned()),
        Err(error) => tracing::warn!("Cannot track diagnostic replay: {error}"),
    }
}
pub fn directory() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .context("OS data directory is unavailable")?
        .join("robin_hood/reports"))
}
fn bound_text(mut text: String, limit: usize) -> String {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}
pub fn capture(
    kind: DiagnosticKindV1,
    description: &str,
    backtrace: Option<String>,
) -> Result<PathBuf> {
    let mut report = DiagnosticReportV1 {
        schema_version: 1,
        kind,
        description: bound_text(description.into(), 16384),
        engine_commit: crate::replay_format::ENGINE_SOURCE_COMMIT.into(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        occurred_at_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
            .try_into()?,
        backtrace: backtrace.map(|s| bound_text(s, 128 * 1024)),
        recent_log: String::new(),
        attachments: vec![],
        warnings: vec![],
    };
    // A panic may occur while either lock is held by this very thread.
    match LOG.try_lock() {
        Ok(log) => {
            report.recent_log = bound_text(
                String::from_utf8_lossy(&log.iter().copied().collect::<Vec<_>>()).into_owned(),
                LOG_LIMIT,
            )
        }
        Err(e) => report.warnings.push(format!("Recent log unavailable: {e}")),
    }
    let replay = match REPLAY.try_lock() {
        Ok(replay) => replay.clone(),
        Err(e) => {
            report
                .warnings
                .push(format!("Replay tracking unavailable: {e}"));
            None
        }
    };
    let dropped = DROPPED_LOG_BYTES.load(Ordering::Relaxed);
    if dropped > 0 {
        report.warnings.push(format!(
            "{dropped} log bytes omitted due to concurrent logging."
        ));
    }
    collect_replay(&mut report, replay.as_deref());
    persist(&directory()?, &report)
}
fn collect_replay(report: &mut DiagnosticReportV1, replay: Option<&Path>) {
    let Some(replay) = replay else {
        report
            .warnings
            .push("No active replay is registered.".into());
        return;
    };
    let result = (|| -> Result<()> {
        let mut paths = if replay.is_dir() {
            let mut paths = Vec::new();
            for entry in std::fs::read_dir(replay)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name == "mission.json" || name.ends_with(".rhrec.jsonl") {
                    paths.push(entry.path());
                }
                anyhow::ensure!(paths.len() <= 60, "too many replay chunks");
            }
            paths
        } else {
            vec![replay.to_owned()]
        };
        paths.sort();
        let mut remaining = MAX_DIAGNOSTIC_ATTACHMENT_BYTES;
        for path in paths {
            let attachment = (|| -> Result<DiagnosticAttachmentV1> {
                anyhow::ensure!(
                    std::fs::symlink_metadata(&path)?.file_type().is_file(),
                    "replay is not a regular file"
                );
                let file = std::fs::File::open(&path)?;
                anyhow::ensure!(
                    file.metadata()?.len() <= remaining as u64,
                    "replay exceeds attachment budget"
                );
                let mut content = String::new();
                file.take(remaining as u64 + 1)
                    .read_to_string(&mut content)?;
                anyhow::ensure!(
                    content.len() <= remaining,
                    "replay exceeds attachment budget"
                );
                Ok(DiagnosticAttachmentV1 {
                    filename: path
                        .file_name()
                        .context("missing filename")?
                        .to_string_lossy()
                        .into_owned(),
                    content,
                })
            })();
            match attachment {
                Ok(attachment) => {
                    remaining -= attachment.content.len();
                    report.attachments.push(attachment);
                }
                Err(error) => report.warnings.push(bound_text(
                    format!("Replay {}: {error:#}", path.display()),
                    1024,
                )),
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        report
            .warnings
            .push(bound_text(format!("Replay unavailable: {error:#}"), 1024));
    }
    // TODO: freeze the recorder for a coherent manual snapshot. Crash captures
    // may have an incomplete last JSONL record; never claim full replay fidelity.
    report
        .warnings
        .push("Replay snapshot may end during a write.".into());
}
fn persist(directory: &Path, report: &DiagnosticReportV1) -> Result<PathBuf> {
    report.validate().map_err(anyhow::Error::msg)?;
    let bytes = serde_json::to_vec(report)?;
    anyhow::ensure!(
        bytes.len() <= MAX_DIAGNOSTIC_BODY_BYTES,
        "encoded report exceeds upload limit"
    );
    std::fs::create_dir_all(directory)?;
    let mut temporary = tempfile::Builder::new()
        .prefix("report-")
        .suffix(".partial")
        .tempfile_in(directory)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    let path = temporary.path().with_extension("pending.json");
    temporary.persist_noclobber(&path)?;
    Ok(path)
}
/// Panic capture is disk-only. Upload on the next launch, outside the panic hook.
#[allow(clippy::print_stderr)]
pub fn capture_failure(kind: DiagnosticKindV1, description: &str, backtrace: Option<String>) {
    match capture(kind, description, backtrace) {
        Ok(path) => eprintln!("Diagnostic report queued: {}", path.display()),
        Err(error) => eprintln!("Failed to queue diagnostic report: {error:#}"),
    }
}
pub fn take_messages() -> Vec<String> {
    match MESSAGES.lock() {
        Ok(mut messages) => messages.drain(..).collect(),
        Err(error) => {
            tracing::warn!("Diagnostic status unavailable: {error}");
            Vec::new()
        }
    }
}
fn message(text: String) {
    tracing::info!("{text}");
    if let Ok(mut messages) = MESSAGES.lock() {
        if messages.len() >= 20 {
            messages.pop_front();
        }
        messages.push_back(text);
    }
}
/// Process a bounded batch on a worker, keeping failures for a later launch.
/// No network activity occurs from a panic hook.
pub fn submit_pending() {
    if UPLOADING.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Err(error) = std::thread::Builder::new()
        .name("bug-reports".into())
        .spawn(|| {
            if let Err(error) = upload_pending() {
                message(format!(
                    "Report submission failed; queued reports retained: {error:#}"
                ));
            }
            UPLOADING.store(false, Ordering::Release);
        })
    {
        UPLOADING.store(false, Ordering::Release);
        tracing::warn!("Cannot start diagnostic uploader: {error}");
    }
}
fn upload_pending() -> Result<()> {
    let directory = directory()?;
    if !directory.exists() {
        return Ok(());
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let endpoint = format!(
        "{}/diagnostics",
        crate::leaderboard_preferences::NATIVE_PRODUCTION_API_BASE_URL
    );
    let mut paths = std::fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
    paths.sort_by_key(|entry| entry.file_name());
    for entry in paths
        .into_iter()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".pending.json"))
        .take(20)
    {
        let path = entry.path();
        let result = upload_one(&client, &endpoint, &path);
        match result {
            Ok(receipt) => {
                std::fs::rename(&path, path.with_extension("submitted"))?;
                message(format!("Report submitted: {}", receipt.report_id));
            }
            Err(error) => {
                message(format!("Report remains queued: {error:#}"));
            }
        }
    }
    Ok(())
}
fn upload_one(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    path: &Path,
) -> Result<DiagnosticReceiptV1> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_DIAGNOSTIC_BODY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= MAX_DIAGNOSTIC_BODY_BYTES,
        "queued report is too large"
    );
    let report: DiagnosticReportV1 = serde_json::from_slice(&bytes)?;
    report.validate().map_err(anyhow::Error::msg)?;
    let response = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(bytes)
        .send()?;
    anyhow::ensure!(
        response.status() == reqwest::StatusCode::ACCEPTED,
        "server returned HTTP {}",
        response.status()
    );
    let mut body = Vec::new();
    response.take(4097).read_to_end(&mut body)?;
    anyhow::ensure!(body.len() <= 4096, "receipt too large");
    let receipt: DiagnosticReceiptV1 = serde_json::from_slice(&body)?;
    use sha2::{Digest as _, Sha256};
    let expected = hex::encode(Sha256::digest(serde_json::to_vec(&report)?));
    anyhow::ensure!(
        receipt.schema_version == 1 && receipt.report_id == expected,
        "invalid report receipt"
    );
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report() -> DiagnosticReportV1 {
        DiagnosticReportV1 {
            schema_version: 1,
            kind: DiagnosticKindV1::Bug,
            description: "stuck".into(),
            engine_commit: "test".into(),
            platform: "test".into(),
            occurred_at_unix_ms: 0,
            backtrace: None,
            recent_log: String::new(),
            attachments: vec![],
            warnings: vec![],
        }
    }
    #[test]
    fn queue_is_durable_unique_and_roundtrips() {
        let root = tempfile::tempdir().unwrap();
        let first = persist(root.path(), &report()).unwrap();
        let second = persist(root.path(), &report()).unwrap();
        assert_ne!(first, second);
        let recovered: DiagnosticReportV1 =
            serde_json::from_reader(std::fs::File::open(first).unwrap()).unwrap();
        assert_eq!(recovered.description, "stuck");
    }
    #[test]
    fn replay_capture_excludes_unrelated_files_and_reports_missing_replay() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("mission.json"), "{}").unwrap();
        std::fs::write(root.path().join("ranked.json"), "private").unwrap();
        let mut report = report();
        collect_replay(&mut report, Some(root.path()));
        assert_eq!(report.attachments.len(), 1);
        assert_eq!(report.attachments[0].filename, "mission.json");
        collect_replay(&mut report, Some(&root.path().join("missing")));
        assert!(report.warnings.iter().any(|s| s.contains("missing")));
    }
}

#[cfg(test)]
mod upload_tests {
    use super::*;
    #[test]
    fn upload_requires_matching_receipt_and_keeps_failed_report() {
        let root = tempfile::tempdir().unwrap();
        let report = DiagnosticReportV1 {
            schema_version: 1,
            kind: DiagnosticKindV1::Bug,
            description: "stuck".into(),
            engine_commit: "test".into(),
            platform: "test".into(),
            occurred_at_unix_ms: 0,
            backtrace: None,
            recent_log: String::new(),
            attachments: vec![],
            warnings: vec![],
        };
        let path = persist(root.path(), &report).unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/api/v1/diagnostics", server.server_addr());
        use sha2::{Digest as _, Sha256};
        let id = hex::encode(Sha256::digest(serde_json::to_vec(&report).unwrap()));
        let expected_id = id.clone();
        let server_thread = std::thread::spawn(move || {
            for (status, id) in [(503, String::new()), (202, "bad-receipt".into()), (202, id)] {
                let mut request = server
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.url(), "/api/v1/diagnostics");
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let decoded: DiagnosticReportV1 = serde_json::from_str(&body).unwrap();
                assert_eq!(decoded.description, "stuck");
                request
                    .respond(
                        tiny_http::Response::from_string(
                            serde_json::to_string(&DiagnosticReceiptV1 {
                                schema_version: 1,
                                report_id: id,
                            })
                            .unwrap(),
                        )
                        .with_status_code(status),
                    )
                    .unwrap();
            }
        });
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        assert!(upload_one(&client, &endpoint, &path).is_err());
        assert!(path.exists());
        assert!(upload_one(&client, &endpoint, &path).is_err());
        assert!(path.exists());
        assert_eq!(
            upload_one(&client, &endpoint, &path).unwrap().report_id,
            expected_id
        );
        server_thread.join().unwrap();
    }
}

//! UI-independent, frame-polled leaderboard browsing and replay downloads.
//!
//! Presentation code supplies typed queries and consumes typed events. Replay
//! events are emitted only after MIME, length, digest, bounded compact decode,
//! current engine identity, and canonical re-encoding all succeed.

use crate::leaderboard_http::HttpTask;
use crate::leaderboard_service::{
    CanonicalReplayDownload, LeaderboardApi, LeaderboardServiceError, decode_board,
    decode_campaign_session_detail, decode_metadata, decode_replay_download, decode_run_detail,
};
use robin_run_protocol::{
    CampaignSessionDetailV1, LeaderboardMetadataV1, LeaderboardPageV1, LeaderboardQueryV1,
    OpaqueId, ReplayArtifactV1, RunDetailV1,
};
#[cfg(not(target_arch = "wasm32"))]
use std::io::Read as _;
use std::sync::Arc;

#[derive(Debug)]
enum BrowseTask {
    Metadata(HttpTask),
    Board {
        task: HttpTask,
        query: LeaderboardQueryV1,
    },
    RunDetail {
        task: HttpTask,
        run_id: OpaqueId,
    },
    CampaignSessionDetail {
        task: HttpTask,
        aggregate_run_id: OpaqueId,
        ordinal: u32,
    },
    ReplayDownload {
        task: HttpTask,
        run_id: OpaqueId,
        session_ordinal: Option<u32>,
        replay: ReplayArtifactV1,
    },
}

#[derive(Clone)]
pub struct VerifiedReplayDownload {
    pub suggested_filename: String,
    pub media_type: String,
    pub engine_hash: String,
    pub bytes: Arc<[u8]>,
}

impl std::fmt::Debug for VerifiedReplayDownload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedReplayDownload")
            .field("suggested_filename", &self.suggested_filename)
            .field("media_type", &self.media_type)
            .field("engine_hash", &self.engine_hash)
            .field("byte_length", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum LeaderboardBrowseEvent {
    Metadata(LeaderboardMetadataV1),
    Board(LeaderboardPageV1),
    RunDetail(RunDetailV1),
    CampaignSessionDetail(CampaignSessionDetailV1),
    Replay(VerifiedReplayDownload),
}

pub struct LeaderboardBrowser {
    api: LeaderboardApi,
    task: Option<BrowseTask>,
}

impl LeaderboardBrowser {
    pub fn new(api: LeaderboardApi) -> Self {
        Self { api, task: None }
    }

    pub fn is_busy(&self) -> bool {
        self.task.is_some()
    }

    pub fn begin_metadata(&mut self) -> Result<(), LeaderboardServiceError> {
        self.replace_task(BrowseTask::Metadata(self.api.metadata()?))
    }

    pub fn load_board(&mut self, query: LeaderboardQueryV1) -> Result<(), LeaderboardServiceError> {
        let task = self.api.board(&query)?;
        self.replace_task(BrowseTask::Board { task, query })
    }

    pub fn open_run(&mut self, run_id: OpaqueId) -> Result<(), LeaderboardServiceError> {
        let task = self.api.run_detail(&run_id)?;
        self.replace_task(BrowseTask::RunDetail { task, run_id })
    }

    pub fn open_campaign_session(
        &mut self,
        aggregate_run_id: OpaqueId,
        ordinal: u32,
    ) -> Result<(), LeaderboardServiceError> {
        let task = self
            .api
            .campaign_session_detail(&aggregate_run_id, ordinal)?;
        self.replace_task(BrowseTask::CampaignSessionDetail {
            task,
            aggregate_run_id,
            ordinal,
        })
    }

    pub fn download_replay(
        &mut self,
        run_id: OpaqueId,
        session_ordinal: Option<u32>,
        replay: ReplayArtifactV1,
    ) -> Result<(), LeaderboardServiceError> {
        let task = match session_ordinal {
            Some(ordinal) => self
                .api
                .campaign_session_replay(&run_id, ordinal, &replay)?,
            None => self.api.run_replay(&run_id, &replay)?,
        };
        self.replace_task(BrowseTask::ReplayDownload {
            task,
            run_id,
            session_ordinal,
            replay,
        })
    }

    pub fn poll(&mut self) -> Option<Result<LeaderboardBrowseEvent, LeaderboardServiceError>> {
        let result = self.task.as_ref().and_then(|task| match task {
            BrowseTask::Metadata(task) => task.try_take(),
            BrowseTask::Board { task, .. }
            | BrowseTask::RunDetail { task, .. }
            | BrowseTask::CampaignSessionDetail { task, .. }
            | BrowseTask::ReplayDownload { task, .. } => task.try_take(),
        })?;
        let task = self
            .task
            .take()
            .expect("completed leaderboard browse task remains installed");
        Some(match task {
            BrowseTask::Metadata(_) => {
                decode_metadata(result).map(LeaderboardBrowseEvent::Metadata)
            }
            BrowseTask::Board { query, .. } => {
                decode_board(result, &query).map(LeaderboardBrowseEvent::Board)
            }
            BrowseTask::RunDetail { run_id, .. } => {
                decode_run_detail(result, &run_id).map(LeaderboardBrowseEvent::RunDetail)
            }
            BrowseTask::CampaignSessionDetail {
                aggregate_run_id,
                ordinal,
                ..
            } => decode_campaign_session_detail(result, &aggregate_run_id, ordinal)
                .map(LeaderboardBrowseEvent::CampaignSessionDetail),
            BrowseTask::ReplayDownload {
                run_id,
                session_ordinal,
                replay,
                ..
            } => decode_replay_download(result, &replay)
                .map(|download| replay_event(run_id, session_ordinal, replay, download)),
        })
    }

    fn replace_task(&mut self, task: BrowseTask) -> Result<(), LeaderboardServiceError> {
        if self.task.is_some() {
            return Err(LeaderboardServiceError::InvalidProtocol(
                "another leaderboard browse request is already in progress".to_owned(),
            ));
        }
        self.task = Some(task);
        Ok(())
    }
}

fn replay_event(
    run_id: OpaqueId,
    session_ordinal: Option<u32>,
    replay: ReplayArtifactV1,
    download: CanonicalReplayDownload,
) -> LeaderboardBrowseEvent {
    let suffix = session_ordinal
        .map(|ordinal| format!("-session-{ordinal}"))
        .unwrap_or_default();
    LeaderboardBrowseEvent::Replay(VerifiedReplayDownload {
        suggested_filename: format!("{}{suffix}.rhrec", safe_filename_id(&run_id)),
        media_type: replay.artifact.media_type,
        engine_hash: download.engine_hash,
        bytes: download.bytes,
    })
}

fn safe_filename_id(id: &OpaqueId) -> String {
    url::form_urlencoded::byte_serialize(id.as_str().as_bytes()).collect()
}

/// Persist authenticated bytes with create-new semantics. An identical
/// existing file is treated as success; a different file is never replaced.
#[cfg(not(target_arch = "wasm32"))]
pub fn persist_verified_replay(
    download: &VerifiedReplayDownload,
    download_directory: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let mut components = std::path::Path::new(&download.suggested_filename).components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
        || download
            .suggested_filename
            .chars()
            .any(|character| matches!(character, '/' | '\\') || character.is_control())
    {
        return Err("replay filename escaped the selected directory".to_owned());
    }
    let path = download_directory.join(&download.suggested_filename);
    match crate::desktop_persistence::write_new_bytes(&path, download.bytes.as_ref()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !existing_download_matches(&path, download.bytes.as_ref())
                .map_err(|error| format!("read existing replay download: {error}"))?
            {
                return Err(format!(
                    "refusing to overwrite a different replay at {}",
                    path.display()
                ));
            }
        }
        Err(error) => return Err(format!("publish authenticated replay download: {error}")),
    }
    Ok(path)
}

#[cfg(not(target_arch = "wasm32"))]
fn existing_download_matches(path: &std::path::Path, expected: &[u8]) -> std::io::Result<bool> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(std::io::Error::other(
            "existing replay download is not a regular file",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // A path swapped after the metadata check must not follow a symlink
        // or block the UI waiting for a FIFO writer.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::other(
            "existing replay download is not a regular file",
        ));
    }
    if metadata.len() != expected.len() as u64 {
        return Ok(false);
    }
    let mut buffer = [0; 8192];
    for chunk in expected.chunks(buffer.len()) {
        file.read_exact(&mut buffer[..chunk.len()])?;
        if &buffer[..chunk.len()] != chunk {
            return Ok(false);
        }
    }
    // Reject a file extended after the length check.
    Ok(file.read(&mut buffer[..1])? == 0)
}

#[cfg(target_arch = "wasm32")]
pub fn trigger_verified_replay_download(
    download: &VerifiedReplayDownload,
) -> Result<String, String> {
    use wasm_bindgen::JsCast as _;

    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(download.bytes.as_ref()));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(&download.media_type);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(browser_download_error)?;
    let object_url =
        web_sys::Url::create_object_url_with_blob(&blob).map_err(browser_download_error)?;
    let result = (|| {
        let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_owned())?;
        let document = window
            .document()
            .ok_or_else(|| "browser document is unavailable".to_owned())?;
        let anchor = document
            .create_element("a")
            .map_err(browser_download_error)?
            .dyn_into::<web_sys::HtmlElement>()
            .map_err(|_| "browser replay download anchor is not an HTML element".to_owned())?;
        anchor
            .set_attribute("href", &object_url)
            .and_then(|()| anchor.set_attribute("download", &download.suggested_filename))
            .map_err(browser_download_error)?;
        anchor.click();
        Ok(format!("browser download: {}", download.suggested_filename))
    })();
    web_sys::Url::revoke_object_url(&object_url).map_err(browser_download_error)?;
    result
}

#[cfg(target_arch = "wasm32")]
fn browser_download_error(value: wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("browser rejected replay download: {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::RANKED_REPLAY_MEDIA_TYPE_V1;

    fn download(filename: &str, bytes: &[u8]) -> VerifiedReplayDownload {
        VerifiedReplayDownload {
            suggested_filename: filename.to_owned(),
            media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
            engine_hash: robin_replay_format::ENGINE_VERSION_HASH.to_owned(),
            bytes: Arc::from(bytes),
        }
    }

    #[test]
    fn debug_output_never_contains_replay_bytes() {
        let value = download("run.rhrec", b"sensitive replay bytes");
        let debug = format!("{value:?}");
        assert!(debug.contains("byte_length"));
        assert!(!debug.contains("sensitive replay bytes"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_delivery_never_overwrites_different_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let first = download("run.rhrec", b"first");
        let path = persist_verified_replay(&first, directory.path()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert_eq!(
            persist_verified_replay(&first, directory.path()).unwrap(),
            path
        );
        assert!(
            persist_verified_replay(&download("run.rhrec", b"other"), directory.path()).is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), b"first");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_delivery_compares_existing_files_in_bounded_chunks() {
        for length in [0, 8191, 8192, 8193, 16385] {
            let directory = tempfile::tempdir().unwrap();
            let bytes = vec![42; length];
            let first = download("run.rhrec", &bytes);
            let path = persist_verified_replay(&first, directory.path()).unwrap();
            assert_eq!(
                persist_verified_replay(&first, directory.path()).unwrap(),
                path
            );
            if length != 0 {
                let mut different = bytes.clone();
                different[length - 1] = 43;
                assert!(
                    persist_verified_replay(&download("run.rhrec", &different), directory.path())
                        .is_err()
                );
            }
            let longer = vec![42; length + 1];
            assert!(
                persist_verified_replay(&download("run.rhrec", &longer), directory.path()).is_err()
            );
            assert_eq!(std::fs::read(path).unwrap(), bytes);
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn native_delivery_rejects_symlinks_and_non_regular_destinations() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside.rhrec");
        std::fs::write(&outside, b"same").unwrap();
        let link = directory.path().join("link.rhrec");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(
            persist_verified_replay(&download("link.rhrec", b"same"), directory.path()).is_err()
        );
        std::fs::create_dir(directory.path().join("directory.rhrec")).unwrap();
        assert!(
            persist_verified_replay(&download("directory.rhrec", b"same"), directory.path())
                .is_err()
        );
        assert_eq!(std::fs::read(outside).unwrap(), b"same");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 3);
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn browser_download_failures_preserve_javascript_error_details() {
        assert_eq!(
            browser_download_error("string failure".into()),
            "string failure"
        );
        let error = js_sys::Error::new("download permission denied");
        assert!(browser_download_error(error.into()).contains("download permission denied"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_delivery_rejects_escaping_names() {
        let directory = tempfile::tempdir().unwrap();
        for filename in [
            "",
            ".",
            "..",
            "../run.rhrec",
            "dir\\run.rhrec",
            "/absolute.rhrec",
        ] {
            assert!(
                persist_verified_replay(&download(filename, b"x"), directory.path()).is_err(),
                "{filename:?}"
            );
        }
        #[cfg(windows)]
        assert!(persist_verified_replay(&download("C:run.rhrec", b"x"), directory.path()).is_err());
    }
}

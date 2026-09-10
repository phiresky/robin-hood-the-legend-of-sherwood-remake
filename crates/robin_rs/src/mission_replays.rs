//! Host-only links between immutable attempts and local recordings.
//! These paths never enter deterministic campaign/save state.

use robin_engine::campaign_history::MissionAttemptKey;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application-owned recording links and background scan. Runtime ownership cannot
/// be restored from a diagnostic serialization.
#[derive(Debug, Serialize)]
pub(crate) struct RecordingIndex {
    directory: Option<PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(skip)]
    active_path: std::sync::Mutex<Option<PathBuf>>,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(skip)]
    scan: std::sync::Mutex<ScanState>,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(skip)]
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl<'de> Deserialize<'de> for RecordingIndex {
    fn deserialize<D: serde::Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "recording index runtime ownership cannot be deserialized",
        ))
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default, Serialize)]
struct ScanState {
    #[serde(skip)]
    worker: Option<std::thread::JoinHandle<Result<(), String>>>,
    stopped: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl<'de> Deserialize<'de> for ScanState {
    fn deserialize<D: serde::Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "recording scan runtime ownership cannot be deserialized",
        ))
    }
}

impl RecordingIndex {
    pub(crate) fn disabled() -> Self {
        Self {
            directory: None,
            #[cfg(not(target_arch = "wasm32"))]
            active_path: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            scan: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            cancelled: Default::default(),
        }
    }

    /// The directory contains attempt links; recordings are scanned in its parent.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn native(directory: PathBuf) -> Self {
        let mut index = Self::disabled();
        index.directory = Some(directory);
        index
    }

    /// Starts one scan at a time. A completed scan must be observed before retrying.
    pub(crate) fn refresh_index(&self) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut scan = self
                .scan
                .lock()
                .map_err(|_| "recording scan lock poisoned")?;
            if scan.stopped {
                return Err("recording index is shut down".into());
            }
            if scan.worker.is_some() {
                return Ok(());
            }
            let Some(directory) = self.directory.clone() else {
                return Ok(());
            };
            let cancelled = self.cancelled.clone();
            scan.worker = Some(
                std::thread::Builder::new()
                    .name("recording-index".into())
                    .spawn(move || {
                        backfill(&directory, &cancelled).map_err(|error| error.to_string())
                    })
                    .map_err(|error| format!("Cannot start recording index: {error}"))?,
            );
        }
        Ok(())
    }

    /// Nonblocking completion polling. Failure, including worker panic, is visible
    /// to the caller and retires the worker so a later refresh can retry.
    pub(crate) fn take_completion(&self) -> Option<Result<(), String>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut scan = self.scan.lock().expect("recording scan lock poisoned");
            if scan
                .worker
                .as_ref()
                .is_some_and(|worker| worker.is_finished())
            {
                return Some(join_scan(
                    scan.worker.take().expect("finished worker exists"),
                ));
            }
        }
        None
    }

    /// Stops further work, cancels between files and joins the active scan.
    /// Parsing an already-open recording must finish before shutdown returns.
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
            // Even a poisoned owner must retire its worker instead of detaching
            // it when the application drops. Keep the failure observable.
            let (mut scan, poisoned) = match self.scan.lock() {
                Ok(scan) => (scan, false),
                Err(error) => {
                    tracing::error!("recording scan lock poisoned during shutdown");
                    (error.into_inner(), true)
                }
            };
            scan.stopped = true;
            if let Some(worker) = scan.worker.take() {
                join_scan(worker)?;
            }
            if poisoned {
                return Err("recording scan lock poisoned during shutdown".into());
            }
        }
        Ok(())
    }
}

impl Drop for RecordingIndex {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            tracing::warn!("Cannot shut down recording index: {error}");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn join_scan(worker: std::thread::JoinHandle<Result<(), String>>) -> Result<(), String> {
    worker
        .join()
        .map_err(|_| "recording index worker panicked".to_string())?
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct ReplayAttemptState {
    run: Option<u64>,
    sequence: u64,
    terminal_nonce: Option<u64>,
    terminal_count: usize,
    completed_at: Option<i64>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ReplayAttemptState {
    fn from_campaign(campaign: &robin_engine::campaign::Campaign) -> Result<Self, String> {
        Ok(Self {
            run: campaign.history_run_id(),
            sequence: campaign
                .latest_mission_attempt()
                .map_or(0, |attempt| attempt.sequence())
                .checked_add(1)
                .ok_or("Attempt sequence overflow")?,
            terminal_nonce: None,
            terminal_count: 0,
            completed_at: None,
        })
    }

    fn identity(self) -> Result<Option<(MissionAttemptKey, Option<i64>)>, String> {
        if self.terminal_count != 1 {
            return Ok(None);
        }
        let run = self
            .run
            .or(self.terminal_nonce)
            .ok_or("Recording has no campaign identity")?;
        Ok(Some((
            MissionAttemptKey {
                campaign_run_id: run,
                sequence: self.sequence,
            },
            self.completed_at,
        )))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn replay_attempt_identity(
    data: &robin_engine::replay::ReplayData,
) -> Result<Option<(MissionAttemptKey, Option<i64>)>, String> {
    use robin_engine::player_command::PlayerCommand;
    let campaign: robin_engine::campaign::Campaign =
        bitcode::decode(&data.header().campaign).map_err(|e| e.to_string())?;
    let mut state = ReplayAttemptState::from_campaign(&campaign)?;
    let mut markers = std::collections::BTreeMap::new();
    for ordinal in 0..data.frame_count() {
        if data.save_marker_for_frame(ordinal).is_some() {
            markers.insert(ordinal, state);
        }
        if let Some(load) = data.load_back_for_frame(ordinal) {
            if let Some(snapshot) = &load.snapshot {
                let save: crate::save_file::GameSaveFile =
                    serde_json::from_slice(&snapshot.payload).map_err(|error| error.to_string())?;
                state = ReplayAttemptState::from_campaign(save.engine.campaign())?;
            } else {
                state = *markers
                    .get(&load.to_frame)
                    .ok_or("attempt identity references a missing save marker")?;
            }
        }
        let frame = data
            .frame(ordinal)
            .expect("validated replay has every frame");
        for input in frame
            .input
            .commands
            .iter()
            .chain(&frame.input.post_commands)
        {
            if let PlayerCommand::ApplyQuitMissionUpdates {
                campaign_run_nonce,
                completed_at_unix_seconds,
                ..
            } = &input.player_input().command
            {
                state.terminal_count += 1;
                state.terminal_nonce = *campaign_run_nonce;
                state.completed_at = *completed_at_unix_seconds;
            }
        }
    }
    state.identity()
}

#[cfg(not(target_arch = "wasm32"))]
fn backfill(
    dir: &std::path::Path,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    let recordings = dir
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or("attempt index directory must have a parent")?;
    let entries = match std::fs::read_dir(recordings) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    // TODO: Index compact imports too when importing an artifact into a profile.
    for entry in entries {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let entry = entry?;
        let path = entry.path();
        let archive = path.is_dir() && path.join("mission.json").is_file();
        if !archive && !path.to_string_lossy().ends_with(".rhrec.jsonl") {
            continue;
        }
        let metadata = entry.metadata()?;
        // Do not block application shutdown opening a named pipe or device
        // merely because it has a recording suffix.
        if (!archive && !metadata.is_file()) || metadata.len() > 64 * 1024 * 1024 {
            continue;
        }
        let data = match crate::replay_format::load_replay_spec(
            path.to_str().ok_or("Non-UTF8 recording path")?,
        ) {
            Ok(data) => data,
            Err(error) => {
                tracing::debug!("Skipping unreadable recording {}: {error}", path.display());
                continue;
            }
        };
        let (key, completed_at) = match replay_attempt_identity(&data) {
            Ok(Some(identity)) => identity,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!("Cannot identify recording {}: {error}", path.display());
                continue;
            }
        };
        if find_in(dir, key, completed_at).is_some() {
            continue;
        }
        let file = stage_recording_link(
            dir,
            RecordingLink {
                key,
                completed_at,
                path: path.canonicalize()?,
            },
        )?;
        // A newly completed live attempt wins over an older background scan.
        match file.persist_noclobber(recording_link_path(dir, key)) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordingLink {
    key: MissionAttemptKey,
    completed_at: Option<i64>,
    path: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
fn recording_link_path(dir: &std::path::Path, key: MissionAttemptKey) -> PathBuf {
    dir.join(format!("{}-{}.json", key.campaign_run_id, key.sequence))
}

#[cfg(not(target_arch = "wasm32"))]
fn stage_recording_link(
    dir: &std::path::Path,
    link: RecordingLink,
) -> Result<tempfile::NamedTempFile, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(&mut file, &link)?;
    Ok(file)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn default_directory() -> PathBuf {
    if let Some(dir) = dirs::data_dir() {
        return dir.join("robin_hood/replays/attempts");
    }
    PathBuf::from("Data/Replays/attempts")
}

impl RecordingIndex {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn recording_started(&self, path: &std::path::Path) {
        if self.directory.is_none() {
            return;
        }
        match path.canonicalize() {
            Ok(path) => *self.active_path.lock().expect("recording path lock") = Some(path),
            Err(error) => {
                *self.active_path.lock().expect("recording path lock") = None;
                tracing::warn!("Cannot track mission recording: {error}");
            }
        }
    }

    pub(crate) fn recording_finished(&self, key: MissionAttemptKey, completed_at: Option<i64>) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(dir) = self.directory.as_ref() else {
                return;
            };
            let Some(path) = self.active_path.lock().expect("recording path lock").take() else {
                tracing::warn!("Completed attempt has no local recording path");
                return;
            };
            let save = || -> Result<(), Box<dyn std::error::Error>> {
                let file = stage_recording_link(
                    dir,
                    RecordingLink {
                        key,
                        completed_at,
                        path,
                    },
                )?;
                file.persist(recording_link_path(dir, key))?;
                Ok(())
            };
            if let Err(error) = save() {
                tracing::warn!("Cannot save mission recording link: {error}");
            }
        }
        // TODO: Persist browser recordings and open playback in an isolated browser session.
        #[cfg(target_arch = "wasm32")]
        let _ = (key, completed_at);
    }

    pub(crate) fn find(
        &self,
        key: MissionAttemptKey,
        completed_at: Option<i64>,
    ) -> Option<PathBuf> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.directory
                .as_ref()
                .and_then(|dir| find_in(dir, key, completed_at))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (key, completed_at);
            None
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn find_in(
    dir: &std::path::Path,
    key: MissionAttemptKey,
    completed_at: Option<i64>,
) -> Option<PathBuf> {
    let file = recording_link_path(dir, key);
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!("Cannot read recording link: {error}");
            return None;
        }
    };
    match serde_json::from_slice::<RecordingLink>(&bytes) {
        Ok(link)
            if link.key == key
                && link.completed_at == completed_at
                && (link.path.is_file() || link.path.join("mission.json").is_file()) =>
        {
            return Some(link.path);
        }
        Ok(_) => tracing::warn!("Recording link is stale or has the wrong attempt identity"),
        Err(error) => tracing::warn!("Invalid recording link: {error}"),
    }
    None
}

pub(crate) fn watch(
    path: &std::path::Path,
    expected: (MissionAttemptKey, Option<i64>),
) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !path.is_file() && !path.join("mission.json").is_file() {
            return Err("The recording file is no longer available.".into());
        }
        let data = crate::replay_format::load_replay_spec(
            path.to_str().ok_or("Recording path is not UTF-8")?,
        )
        .map_err(|error| format!("Cannot play this recording: {error}"))?;
        if replay_attempt_identity(&data)? != Some(expected) {
            return Err("The recording file does not match this play.".into());
        }
        // A separate viewer preserves the live session, recorder and paused state.
        let mut child =
            std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                .arg("--replay")
                .arg(path)
                .args(["--http-server", "0"])
                .spawn()
                .map_err(|e| format!("Cannot open replay: {e}"))?;
        std::thread::spawn(move || match child.wait() {
            Ok(status) if status.success() => {}
            result => tracing::warn!("Replay viewer ended with {result:?}"),
        });
        Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, expected);
        Err("Local replay playback requires the desktop game.".into())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use robin_engine::{
        campaign::Campaign,
        engine::{SimCommand, SimulationFrameInput},
        replay::{ReplayData, ReplayFile, ReplayFrame, ReplayHeader},
    };

    fn write_recording(path: &std::path::Path, nonce: u64) {
        let data = recording(&Campaign::default(), &[nonce]);
        std::fs::write(
            path,
            format!(
                "{}\n{}\n",
                serde_json::to_string(data.header()).unwrap(),
                serde_json::json!({"f": 0, "i": data.frame(0).unwrap()}),
            ),
        )
        .unwrap();
    }

    fn completion(index: &RecordingIndex) -> Result<(), String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(result) = index.take_completion() {
                return result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "recording scan did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn staging_a_recording_link_does_not_publish_and_drop_retires_the_temp_file() {
        let directory = tempfile::tempdir().unwrap();
        let attempts = directory.path().join("attempts");
        let key = MissionAttemptKey {
            campaign_run_id: 41,
            sequence: 7,
        };
        let path = directory.path().join("recording 雪.rhrec.jsonl");
        let file = stage_recording_link(
            &attempts,
            RecordingLink {
                key,
                completed_at: Some(200),
                path: path.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            recording_link_path(&attempts, key),
            attempts.join("41-7.json")
        );
        assert!(!recording_link_path(&attempts, key).exists());
        let decoded: RecordingLink =
            serde_json::from_slice(&std::fs::read(file.path()).unwrap()).unwrap();
        assert_eq!(decoded.key, key);
        assert_eq!(decoded.completed_at, Some(200));
        assert_eq!(decoded.path, path);
        let temporary_path = file.path().to_owned();
        drop(file);
        assert!(!temporary_path.exists());
        assert!(!recording_link_path(&attempts, key).exists());
    }

    #[test]
    fn refresh_retains_running_and_completed_workers_until_observed() {
        let directory = tempfile::tempdir().unwrap();
        let index = RecordingIndex::native(directory.path().join("attempts"));
        let (release, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            wait.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            Err("unobserved scan result".to_owned())
        });
        let worker_id = worker.thread().id();
        index.scan.lock().unwrap().worker = Some(worker);
        index.refresh_index().unwrap();
        assert_eq!(
            index
                .scan
                .lock()
                .unwrap()
                .worker
                .as_ref()
                .unwrap()
                .thread()
                .id(),
            worker_id
        );
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !index
            .scan
            .lock()
            .unwrap()
            .worker
            .as_ref()
            .unwrap()
            .is_finished()
        {
            assert!(
                std::time::Instant::now() < deadline,
                "test worker did not finish"
            );
            std::thread::yield_now();
        }
        index.refresh_index().unwrap();
        assert_eq!(
            index
                .scan
                .lock()
                .unwrap()
                .worker
                .as_ref()
                .unwrap()
                .thread()
                .id(),
            worker_id
        );
        assert_eq!(
            index.take_completion(),
            Some(Err("unobserved scan result".to_owned()))
        );
        assert!(index.take_completion().is_none());
        index.refresh_index().unwrap();
        completion(&index).unwrap();
    }

    #[test]
    fn background_backfill_and_active_recordings_are_application_local() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let first = RecordingIndex::native(first_dir.path().join("attempts"));
        let second = RecordingIndex::native(second_dir.path().join("attempts"));
        let key = MissionAttemptKey {
            campaign_run_id: 41,
            sequence: 1,
        };
        let first_path = first_dir.path().join("first.rhrec.jsonl");
        let second_path = second_dir.path().join("second.rhrec.jsonl");
        write_recording(&first_path, 41);
        write_recording(&second_path, 41);
        first.refresh_index().unwrap();
        completion(&first).unwrap();
        assert_eq!(
            first.find(key, Some(100)),
            Some(first_path.canonicalize().unwrap())
        );
        assert!(second.find(key, Some(100)).is_none());
        assert!(second.take_completion().is_none());

        first.recording_started(&first_path);
        second.recording_started(&second_path);
        first.recording_finished(key, Some(200));
        second.recording_finished(key, Some(300));
        assert_eq!(
            first.find(key, Some(200)),
            Some(first_path.canonicalize().unwrap())
        );
        assert_eq!(
            second.find(key, Some(300)),
            Some(second_path.canonicalize().unwrap())
        );
        assert!(first.find(key, Some(300)).is_none());
    }

    #[test]
    fn backfill_does_not_replace_a_live_attempt_link() {
        let dir = tempfile::tempdir().unwrap();
        let index = RecordingIndex::native(dir.path().join("attempts"));
        let old = dir.path().join("old.rhrec.jsonl");
        let live = dir.path().join("live.rhrec.jsonl");
        write_recording(&old, 41);
        write_recording(&live, 41);
        let key = MissionAttemptKey {
            campaign_run_id: 41,
            sequence: 1,
        };
        index.recording_started(&live);
        // Deliberately different completion time: lookup cannot short-circuit the
        // scan, so persist_noclobber must preserve the authoritative live link.
        index.recording_finished(key, Some(200));
        index.refresh_index().unwrap();
        completion(&index).unwrap();
        assert_eq!(
            index.find(key, Some(200)),
            Some(live.canonicalize().unwrap())
        );
        assert!(index.find(key, Some(100)).is_none());
    }

    #[test]
    fn failed_scan_is_observable_and_can_retry_after_repair() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = dir.path().join("recordings");
        std::fs::write(&recordings, "not a directory").unwrap();
        let index = RecordingIndex::native(recordings.join("attempts"));
        index.refresh_index().unwrap();
        assert!(completion(&index).is_err());
        assert!(index.take_completion().is_none());
        std::fs::remove_file(&recordings).unwrap();
        std::fs::create_dir(&recordings).unwrap();
        write_recording(&recordings.join("old.rhrec.jsonl"), 41);
        index.refresh_index().unwrap();
        completion(&index).unwrap();
        assert!(
            index
                .find(
                    MissionAttemptKey {
                        campaign_run_id: 41,
                        sequence: 1
                    },
                    Some(100)
                )
                .is_some()
        );
    }

    #[test]
    #[ignore = "requires LLVM codegen backend for panic unwinding; see docs/TESTING.md"]
    fn worker_panic_is_observable_and_does_not_wedge_retry() {
        let dir = tempfile::tempdir().unwrap();
        let index = RecordingIndex::native(dir.path().join("attempts"));
        index.scan.lock().unwrap().worker =
            Some(std::thread::spawn(|| panic!("injected scanner panic")));
        assert!(completion(&index).unwrap_err().contains("panicked"));
        index.refresh_index().unwrap();
        completion(&index).unwrap();
    }

    #[test]
    fn shutdown_and_drop_join_workers_and_prevent_new_scans() {
        let index = RecordingIndex::disabled();
        let cancelled = index.cancelled.clone();
        index.scan.lock().unwrap().worker = Some(std::thread::spawn(move || {
            while !cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::yield_now();
            }
            Err("injected worker failure during shutdown".into())
        }));
        assert!(
            index
                .shutdown()
                .unwrap_err()
                .contains("injected worker failure")
        );
        assert!(index.scan.lock().unwrap().worker.is_none());
        assert!(index.refresh_index().unwrap_err().contains("shut down"));
        index.shutdown().unwrap();

        let index = RecordingIndex::disabled();
        let cancelled = index.cancelled.clone();
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_finished = finished.clone();
        index.scan.lock().unwrap().worker = Some(std::thread::spawn(move || {
            while !cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::yield_now();
            }
            worker_finished.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }));
        drop(index);
        assert!(finished.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn cancelled_scan_does_not_publish_links_and_disabled_index_does_not_scan() {
        let dir = tempfile::tempdir().unwrap();
        write_recording(&dir.path().join("old.rhrec.jsonl"), 41);
        let attempts = dir.path().join("attempts");
        backfill(&attempts, &std::sync::atomic::AtomicBool::new(true)).unwrap();
        assert!(!attempts.exists());
        let index = RecordingIndex::disabled();
        index.refresh_index().unwrap();
        assert!(index.scan.lock().unwrap().worker.is_none());
        assert!(index.take_completion().is_none());
        assert!(serde_json::from_str::<RecordingIndex>("{}").is_err());
    }

    fn recording(campaign: &Campaign, nonces: &[u64]) -> ReplayData {
        ReplayData::try_from(ReplayFile {
            header: ReplayHeader {
                mission_id: "H01".into(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in("H01", "H01", "H01").unwrap(),
                rng_seed: 1, sim_config: Default::default(), spellforge_package: None,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
                campaign: bitcode::encode(campaign),
            },
            frames: [(0, ReplayFrame {
                timeline_before: 0, timeline_after: 1,
                input: SimulationFrameInput {
                    post_commands: nonces.iter().map(|nonce| SimCommand::host(robin_engine::player_command::PlayerCommand::ApplyQuitMissionUpdates {
                        exit_code: robin_engine::game_operation::GameCode::LevelFailed,
                        difficulty: Default::default(), completed_at_unix_seconds: Some(100), campaign_run_nonce: Some(*nonce),
                    })).collect(),
                    ..Default::default()
                }, host_controls: Vec::new(),
            })].into(),
            hashes: Default::default(), save_markers: Default::default(), load_backs: Default::default(),
        }).unwrap()
    }

    #[test]
    fn recording_identity_uses_campaign_sequence_and_terminal_nonce() {
        let mut campaign = Campaign::default();
        assert_eq!(
            replay_attempt_identity(&recording(&campaign, &[]))
                .unwrap()
                .map(|(key, _)| key),
            None
        );
        assert_eq!(
            replay_attempt_identity(&recording(&campaign, &[41, 42]))
                .unwrap()
                .map(|(key, _)| key),
            None
        );
        assert_eq!(
            replay_attempt_identity(&recording(&campaign, &[41]))
                .unwrap()
                .map(|(key, _)| key),
            Some(MissionAttemptKey {
                campaign_run_id: 41,
                sequence: 1
            })
        );
        campaign
            .missions
            .push(robin_engine::mission::Mission::new());
        campaign.record_mission_attempt(
            0,
            robin_engine::campaign_history::MissionAttemptOutcome::Lost,
            Some(10),
            Some(77),
            1,
            Default::default(),
            &Default::default(),
            None,
        );
        assert_eq!(
            replay_attempt_identity(&recording(&campaign, &[999]))
                .unwrap()
                .map(|(key, _)| key),
            Some(MissionAttemptKey {
                campaign_run_id: 77,
                sequence: 2
            })
        );
    }

    #[test]
    fn recording_identity_follows_reload_instead_of_counting_abandoned_terminals() {
        let first = recording(&Campaign::default(), &[41]);
        let mut file = robin_engine::replay::ReplayFile::from(&first);
        file.header.total_frames = 3;
        file.save_markers.insert(
            0,
            robin_engine::replay::ReplaySaveMarker {
                state_hash: 1,
                timeline_frame: 0,
            },
        );
        file.load_backs.insert(
            1,
            robin_engine::replay::ReplayLoadBack {
                to_frame: 0,
                is_continue: false,
                snapshot: None,
            },
        );
        file.frames.insert(
            1,
            robin_engine::replay::ReplayFrame {
                timeline_before: 0,
                timeline_after: 0,
                input: robin_engine::engine::SimulationFrameInput::default().with_hourglass(false),
                host_controls: Vec::new(),
            },
        );
        let final_attempt = recording(&Campaign::default(), &[42]);
        file.frames
            .insert(2, final_attempt.frame(0).unwrap().clone());
        let data = file.try_into().unwrap();
        assert_eq!(
            replay_attempt_identity(&data).unwrap(),
            Some((
                MissionAttemptKey {
                    campaign_run_id: 42,
                    sequence: 1
                },
                Some(100)
            ))
        );
        assert_eq!(
            replay_attempt_identity(&first).unwrap(),
            Some((
                MissionAttemptKey {
                    campaign_run_id: 41,
                    sequence: 1
                },
                Some(100)
            ))
        );
    }

    #[test]
    fn missing_or_invalid_recordings_report_errors_without_launching() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            watch(
                &dir.path().join("missing.rhrec.jsonl"),
                (
                    MissionAttemptKey {
                        campaign_run_id: 1,
                        sequence: 1
                    },
                    None
                )
            )
            .unwrap_err()
            .contains("no longer available")
        );
        let path = dir.path().join("invalid.rhrec.jsonl");
        std::fs::write(&path, "invalid replay").unwrap();
        assert!(
            watch(
                &path,
                (
                    MissionAttemptKey {
                        campaign_run_id: 1,
                        sequence: 1
                    },
                    None
                )
            )
            .unwrap_err()
            .contains("Cannot play this recording")
        );
    }

    #[test]
    fn a_replaced_recording_cannot_launch_a_different_play() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replaced.rhrec.jsonl");
        let data = recording(&Campaign::default(), &[41]);
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                serde_json::to_string(data.header()).unwrap(),
                serde_json::json!({"f": 0, "i": data.frame(0).unwrap()}),
            ),
        )
        .unwrap();
        let error = watch(
            &path,
            (
                MissionAttemptKey {
                    campaign_run_id: 42,
                    sequence: 1,
                },
                Some(100),
            ),
        )
        .unwrap_err();
        assert!(error.contains("does not match"), "{error}");
    }
}

//! Host-only links between immutable attempts and local recordings.
//! These paths never enter deterministic campaign/save state.

use robin_engine::campaign_history::MissionAttemptKey;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

static INDEX_UPDATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn take_index_updated() -> bool {
    INDEX_UPDATED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Backfill recordings made before attempt links existed, off the UI thread.
#[cfg(not(test))]
pub(crate) fn refresh_index() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static SCANNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if SCANNING.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        std::thread::spawn(|| {
            if let Err(error) = backfill() {
                tracing::warn!("Cannot index previous recordings: {error}");
            }
            INDEX_UPDATED.store(true, std::sync::atomic::Ordering::Relaxed);
            SCANNING.store(false, std::sync::atomic::Ordering::Relaxed);
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn replay_attempt_identity(
    data: &robin_engine::replay::ReplayData,
) -> Result<Option<(MissionAttemptKey, Option<i64>)>, String> {
    use robin_engine::player_command::PlayerCommand;
    let campaign: robin_engine::campaign::Campaign =
        bitcode::decode(&data.header().campaign).map_err(|e| e.to_string())?;
    let mut terminal_nonce = None;
    let mut terminal_count = 0;
    let mut completed_at = None;
    for frame in
        (0..data.frame_count()).map(|i| data.frame(i).expect("validated replay has every frame"))
    {
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
                terminal_count += 1;
                terminal_nonce = *campaign_run_nonce;
                completed_at = *completed_at_unix_seconds;
            }
        }
    }
    if terminal_count != 1 {
        return Ok(None);
    }
    let run = campaign
        .history_run_id()
        .or(terminal_nonce)
        .ok_or("Recording has no campaign identity")?;
    let sequence = campaign
        .latest_mission_attempt()
        .map_or(0, |attempt| attempt.sequence())
        .checked_add(1)
        .ok_or("Attempt sequence overflow")?;
    Ok(Some((
        MissionAttemptKey {
            campaign_run_id: run,
            sequence,
        },
        completed_at,
    )))
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(not(test))]
fn backfill() -> Result<(), Box<dyn std::error::Error>> {
    let dir = directory();
    let recordings = dir.parent().expect("attempt index directory has a parent");
    let entries = match std::fs::read_dir(recordings) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    // TODO: Index compact imports too when importing an artifact into a profile.
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.to_string_lossy().ends_with(".rhrec.jsonl")
            || entry.metadata()?.len() > 64 * 1024 * 1024
        {
            continue;
        }
        let data = match robin_engine::replay::ReplayData::from_file(
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
        if find(key, completed_at).is_some() {
            continue;
        }
        std::fs::create_dir_all(&dir)?;
        let mut file = tempfile::NamedTempFile::new_in(&dir)?;
        serde_json::to_writer(
            &mut file,
            &RecordingLink {
                key,
                completed_at,
                path: path.canonicalize()?,
            },
        )?;
        // A newly completed live attempt wins over an older background scan.
        match file
            .persist_noclobber(dir.join(format!("{}-{}.json", key.campaign_run_id, key.sequence)))
        {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordingLink {
    key: MissionAttemptKey,
    completed_at: Option<i64>,
    path: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
fn active_path() -> &'static std::sync::Mutex<Option<PathBuf>> {
    static PATH: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
        std::sync::OnceLock::new();
    PATH.get_or_init(Default::default)
}

#[cfg(not(target_arch = "wasm32"))]
fn directory() -> PathBuf {
    #[cfg(feature = "native-fs")]
    if let Some(dir) = dirs::data_dir() {
        return dir.join("robin_hood/replays/attempts");
    }
    PathBuf::from("Data/Replays/attempts")
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn recording_started(path: &std::path::Path) {
    match path.canonicalize() {
        Ok(path) => *active_path().lock().expect("recording path lock") = Some(path),
        Err(error) => {
            *active_path().lock().expect("recording path lock") = None;
            tracing::warn!("Cannot track mission recording: {error}");
        }
    }
}

pub(crate) fn recording_finished(key: MissionAttemptKey, completed_at: Option<i64>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(path) = active_path().lock().expect("recording path lock").take() else {
            tracing::warn!("Completed attempt has no local recording path");
            return;
        };
        let save = || -> Result<(), Box<dyn std::error::Error>> {
            let dir = directory();
            std::fs::create_dir_all(&dir)?;
            let mut file = tempfile::NamedTempFile::new_in(&dir)?;
            serde_json::to_writer(
                &mut file,
                &RecordingLink {
                    key,
                    completed_at,
                    path,
                },
            )?;
            file.persist(dir.join(format!("{}-{}.json", key.campaign_run_id, key.sequence)))?;
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

pub(crate) fn find(key: MissionAttemptKey, completed_at: Option<i64>) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let file = directory().join(format!("{}-{}.json", key.campaign_run_id, key.sequence));
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
                if link.key == key && link.completed_at == completed_at && link.path.is_file() =>
            {
                return Some(link.path);
            }
            Ok(_) => tracing::warn!("Recording link is stale or has the wrong attempt identity"),
            Err(error) => tracing::warn!("Invalid recording link: {error}"),
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (key, completed_at);
    None
}

pub(crate) fn watch(
    path: &std::path::Path,
    expected: (MissionAttemptKey, Option<i64>),
) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !path.is_file() {
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

//! Replay recorder/player and rollback-checker setup.
//!
//! Houses the `TeeWriter` adapter, the default replay path picker, the
//! `ReplayAndRollback` bundle, and `init_replay_and_rollback` itself.

use crate::rewind::RewindBuffer;
use crate::rollback_checker::RollbackChecker;
use robin_engine::engine::LevelAssets;
use robin_engine::replay::{ReplayPlayer, ReplayRecorder};
use robin_engine::replay_rankability::InputTaintKind;
use std::collections::BTreeSet;
use std::sync::Arc;

fn mission_start_input_taints(
    headless: bool,
    mission_start_reveal_all: bool,
    restarted: bool,
) -> BTreeSet<InputTaintKind> {
    let mut taints = BTreeSet::new();
    if headless {
        taints.insert(InputTaintKind::HeadlessAutomation);
    }
    if mission_start_reveal_all {
        taints.insert(InputTaintKind::DebugInputInjection);
    }
    if restarted {
        taints.insert(InputTaintKind::MissionRestart);
    }
    taints
}

/// `Write` adapter that forwards bytes to a primary sink (the
/// mission chunk on native or browser storage) and to the bounded segmented replay
/// spool used by the script-RPC `get-replay` endpoint.
///
/// Only used by `init_replay_and_rollback`; kept here (rather than in
/// `replay`) so the recorder itself stays filesystem-agnostic.
struct TeeWriter {
    primary: Box<dyn std::io::Write + Send>,
    mirror: crate::replay_service::ReplaySpoolWriter,
    skip_mirror_header: bool,
}

impl std::io::Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Reject known spool backpressure before changing the durable file. If
        // the primary performs a legitimate short write, mirror only the
        // accepted prefix and let `write_all` retry the remainder.
        let mirror_len = if self.skip_mirror_header {
            buf.iter()
                .position(|byte| *byte == b'\n')
                .map_or(0, |newline| buf.len() - newline - 1)
        } else {
            buf.len()
        };
        self.mirror.preflight_write(mirror_len)?;
        let written = match self.primary.write(buf) {
            Ok(written) => written,
            Err(error) => {
                self.mirror
                    .poison(format!("primary replay write failed: {error}"));
                return Err(error);
            }
        };
        if written == 0 && !buf.is_empty() {
            let error = std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "primary replay writer made no progress",
            );
            self.mirror
                .poison(format!("primary replay write failed: {error}"));
            return Err(error);
        }
        let accepted = &buf[..written];
        let mirrored = if self.skip_mirror_header {
            match accepted.iter().position(|byte| *byte == b'\n') {
                Some(end) => {
                    self.skip_mirror_header = false;
                    &accepted[end + 1..]
                }
                None => &[],
            }
        } else {
            accepted
        };
        std::io::Write::write_all(&mut self.mirror, mirrored)?;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Err(error) = self.primary.flush() {
            self.mirror
                .poison(format!("primary replay flush failed: {error}"));
            return Err(error);
        }
        std::io::Write::flush(&mut self.mirror)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn default_replay_path() -> String {
    use std::path::PathBuf;
    let dir = dirs::data_dir()
        .map(|d| d.join("robin_hood").join("replays"))
        .unwrap_or_else(|| PathBuf::from("Data/Replays"));
    // `%:z` → `+HH:MM`; we strip the inner colon so the whole stamp is
    // filesystem-safe (e.g. `2026-04-17T09-32-15+02-00`).
    let stamp = jiff::Zoned::now()
        .strftime("%Y-%m-%dT%H-%M-%S%:z")
        .to_string()
        .replace(':', "-");
    dir.join(format!(
        "{stamp}-{}-{}.mission",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("valid clock")
            .subsec_nanos()
    ))
    .to_string_lossy()
    .into_owned()
}

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
fn replay_debug_log_path(replay_path: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(replay_path);
    let filename = path
        .file_name()
        .map(|name| format!("{}.debug.log", name.to_string_lossy()))
        .unwrap_or_else(|| "replay.debug.log".to_string());
    path.with_file_name(filename)
}

/// Start a separate artifact after restoring the exact bootstrap checkpoint.
/// Never overwrite the terminal attempt, including an explicit --record path.
pub(super) fn restart_recording(
    control: &crate::replay_service::ReplayRecordingControl,
    _recording_index: &crate::mission_replays::RecordingIndex,
    header: robin_engine::replay::ReplayHeader,
) -> std::io::Result<ReplayRecorder> {
    let mirror = control.begin_recording();
    #[cfg(all(not(target_arch = "wasm32"), not(test)))]
    let primary: Box<dyn std::io::Write + Send> = {
        let default_path = std::path::PathBuf::from(default_replay_path());
        let directory = default_path
            .parent()
            .expect("default replay path has a directory");
        std::fs::create_dir_all(directory)?;
        let (file, path) = tempfile::Builder::new()
            .prefix("restart-")
            .suffix(crate::main_entry::RHREC_EXT)
            .tempfile_in(directory)?
            .keep()
            .map_err(|error| error.error)?;
        tracing::info!("Recording restarted replay → {}", path.display());
        _recording_index.recording_started(&path);
        let log_path = replay_debug_log_path(path.to_str().expect("replay directory is UTF-8"));
        if let Err(error) = crate::set_replay_log_file(&log_path) {
            tracing::warn!("Failed to create restarted replay debug log: {error}");
        }
        Box::new(file)
    };
    #[cfg(any(target_arch = "wasm32", test))]
    let primary: Box<dyn std::io::Write + Send> = Box::new(std::io::sink());
    ReplayRecorder::from_recording_header(
        Box::new(TeeWriter {
            primary,
            mirror,
            skip_mirror_header: false,
        }),
        header,
    )
}

pub(super) fn init_recording(
    replay_campaign: &robin_engine::campaign::Campaign,
    assets: &LevelAssets,
    args: &crate::main_entry::MissionLaunch,
    mission_id: &str,
    mission_assets: robin_engine::mission_assets::MissionAssetDescriptor,
    engine_rng_seed: u64,
    engine_sim_config: robin_engine::engine::SimConfig,
) -> Result<Option<crate::replay_recording::SharedReplayRecorder>, String> {
    // No recording while playing back (either source).
    let is_playing_back = args.replay_data.is_some() || args.replay.is_some();
    #[cfg(not(target_arch = "wasm32"))]
    let replay_path = if is_playing_back {
        None
    } else {
        Some(args.record.clone().unwrap_or_else(default_replay_path))
    };
    #[cfg(target_arch = "wasm32")]
    let replay_path = if is_playing_back {
        None
    } else {
        match crate::replay_archive::browser_recording_directory() {
            Ok(path) => Some(path),
            Err(error) => {
                return Err(format!("Browser replay storage is unavailable: {error:#}"));
            }
        }
    };
    // A fresh mission gets a fresh generation of the bounded spool. The
    // returned sole writer publishes only complete recorder flush boundaries.
    let rpc_spool = args.global_options.replay_recording().begin_recording();
    // One-shot mission-map rendering exits before the first simulation
    // frame, so producing an empty replay (and its debug log) would only be
    // an unrelated filesystem side effect of the capture tool.
    let mut recorder =
        if !should_record_local_replay(is_playing_back, args.mission_start_map_output.is_some()) {
            None
        } else {
            let path = replay_path.as_deref().expect("live recording path");
            let archive = crate::replay_archive::MissionArchive::create(std::path::Path::new(path));
            let (primary, archive) = archive
                .and_then(|archive| Ok((archive.writer()?, archive)))
                .map_err(|error| format!("Failed to create mission recording: {error:#}"))?;

            // `mission_id` (e.g. `"Dem_Lei_MP"`, `"Sherwood"`) is the
            // `.rhm` filename — stamped into the header so a later
            // `--replay` picks the right mission without threading the
            // campaign index through. `replay_campaign` is the exact clone made
            // immediately before Engine construction; the engine-owned campaign
            // may already have been changed by level initialization (Sherwood
            // clears its mission team after using it to spawn PCs).
            {
                let writer: Box<dyn std::io::Write + Send> = Box::new(TeeWriter {
                    primary,
                    mirror: rpc_spool,
                    skip_mirror_header: false,
                });
                let spellforge_package = assets
                    .attachments
                    .spellforge_runtime
                    .as_ref()
                    .map(|runtime| runtime.package().clone());
                match ReplayRecorder::with_writer_and_spellforge_package(
                    writer,
                    mission_id.to_string(),
                    mission_assets.clone(),
                    engine_rng_seed,
                    engine_sim_config,
                    replay_campaign,
                    spellforge_package,
                ) {
                    Ok(rec) => {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            args.global_options.recording_index().recording_started(
                                &archive.directory().join(archive.current_chunk()),
                            );
                            if let Err(error) = crate::set_replay_log_file(
                                &archive.directory().join("replay.debug.log"),
                            ) {
                                tracing::warn!("Failed to create replay log: {error}");
                            }
                        }
                        tracing::info!(
                            "Recording mission replay → {}",
                            archive.directory().display()
                        );
                        Some(crate::replay_recording::SharedReplayRecorder::archived(
                            rec, archive,
                        ))
                    }
                    Err(error) => {
                        return Err(format!("Failed to initialize replay recorder: {error}"));
                    }
                }
            }
        };

    if let Some(recorder) = recorder.as_mut() {
        for kind in mission_start_input_taints(
            args.headless,
            args.mission_start_reveal_all,
            args.mission_restart,
        ) {
            recorder.record_input_taint(kind, 0);
        }
    }
    args.global_options
        .replay_recording()
        .install_capture_recorder(recorder.clone());
    Ok(recorder)
}

#[cfg(test)]
pub(crate) fn root_writer(
    primary: Box<dyn std::io::Write + Send>,
    mirror: crate::replay_service::ReplaySpoolWriter,
) -> Box<dyn std::io::Write + Send> {
    Box::new(TeeWriter {
        primary,
        mirror,
        skip_mirror_header: false,
    })
}

pub(crate) fn continuation_writer(
    primary: Box<dyn std::io::Write + Send>,
    mirror: crate::replay_service::ReplaySpoolWriter,
) -> Box<dyn std::io::Write + Send> {
    Box::new(TeeWriter {
        primary,
        mirror,
        skip_mirror_header: true,
    })
}

/// Bundle of determinism-related mission state built by
/// [`init_replay_and_rollback`] — replay recorder, replay player,
/// rollback checker, and the hold-to-rewind snapshot buffer.
pub(super) struct ReplayAndRollback {
    pub(super) recording_control: crate::replay_service::ReplayRecordingControl,
    pub(super) recorder: Option<crate::replay_recording::SharedReplayRecorder>,
    pub(super) player: Option<ReplayPlayer>,
    pub(super) rollback_checker: Option<RollbackChecker>,
    pub(super) rewind_buffer: RewindBuffer,
    /// Final value of the "start paused" toggle — true if either
    /// `--start-paused` was passed on the command line, or a pending
    /// `load-replay` RPC call requested it.
    pub(super) start_paused: bool,
}

fn should_record_local_replay(is_playing_back: bool, mission_start_map: bool) -> bool {
    !is_playing_back && !mission_start_map
}

/// Wire up replay recording / playback, the runtime rollback checker,
/// and the hold-to-rewind snapshot buffer.
///
/// Record-by-default: when `--record` is omitted and we're not
/// replaying, drop a recording into `<data_dir>/robin_hood/replays/`
/// with an ISO-8601 timestamp name so every session can be re-run
/// deterministically later.  Pass `--record <path>` to override the
/// destination, or `--replay <path>` to disable recording entirely.
///
/// Replay seed/config/campaign metadata has already been applied before
/// Engine construction; this function only attaches playback/recording and
/// rollback instrumentation to that frame-0 state.
pub(super) fn init_replay_and_rollback(
    replay_campaign: &robin_engine::campaign::Campaign,
    assets: Arc<LevelAssets>,
    args: &crate::main_entry::MissionLaunch,
    mission_id: &str,
    mission_assets: robin_engine::mission_assets::MissionAssetDescriptor,
    engine_rng_seed: u64,
    engine_sim_config: robin_engine::engine::SimConfig,
    is_multiplayer: bool,
    prepared_recorder: Option<crate::replay_recording::SharedReplayRecorder>,
) -> Result<ReplayAndRollback, String> {
    // Every queued replay must be converted into `args.replay_data` before
    // mission construction. Reseeding an already-built Engine cannot recreate
    // random draws performed during level initialization.
    assert!(
        args.global_options
            .replay_launches()
            .pending_mission()
            .is_none(),
        "pending replay must be consumed and supplied before mission Engine construction"
    );
    assert!(
        args.replay.is_none(),
        "raw replay input must be decoded before mission Engine construction"
    );

    let is_playing_back = args.replay_data.is_some();
    let recorder = if prepared_recorder.is_some() {
        prepared_recorder
    } else {
        init_recording(
            replay_campaign,
            &assets,
            args,
            mission_id,
            mission_assets.clone(),
            engine_rng_seed,
            engine_sim_config,
        )?
    };
    let player = if let Some(data) = args.replay_data.clone() {
        assert_eq!(
            data.header().mission_assets,
            mission_assets,
            "replay mission asset descriptor does not match the assets mounted for playback"
        );
        let local_spellforge_package = assets
            .attachments
            .spellforge_runtime
            .as_ref()
            .map(|runtime| runtime.package());
        assert_eq!(
            data.header().spellforge_package.as_ref(),
            local_spellforge_package,
            "replay Spellforge package does not match the loaded mission package"
        );
        tracing::info!(
            "Loaded replay (decoded): mission `{}`, {} frames, seed {}",
            data.header().mission_id,
            data.frame_count(),
            data.header().rng_seed,
        );
        // No restore_rng_from_seed here: see EngineArgs setup in
        // `load_level_and_sprite_bank` — the engine RNG was already
        // seeded at construction with this header's seed.
        Some(ReplayPlayer::new(data))
    } else {
        None
    };

    assert_eq!(
        is_playing_back,
        player.is_some(),
        "requested replay must be attached before gameplay starts"
    );
    tracing::info!(
        playback = player.is_some(),
        recording = recorder.is_some(),
        "mission replay mode"
    );

    // Rollback checker rewinds 25 frames every sim frame and re-simulates
    // to verify determinism. Disabled during replay playback (no new
    // commands to verify), when `--rollback-check=false`, on wasm, and
    // in multiplayer. Multiplayer still logs real host/client desyncs
    // through authoritative state-hash comparison; the local rollback
    // checker is too expensive to run inside the live netcode loop.
    let rollback_checker = if args.rollback_check
        && player.is_none()
        && !cfg!(target_arch = "wasm32")
        && !is_multiplayer
    {
        let rollback_replay_path = args.record.clone();
        Some(RollbackChecker::new(assets, rollback_replay_path))
    } else {
        if is_multiplayer && args.rollback_check && player.is_none() {
            tracing::info!(
                "multiplayer: rollback checker disabled; using host state-hash desync logs"
            );
        }
        None
    };

    // Hold-to-rewind buffer keeps exponentially-spaced pre-tick sim
    // clones so BACKSPACE can replay the game backwards at normal
    // speed.  Disabled during replay playback because the replay path
    // owns the command stream.
    let rewind_buffer = RewindBuffer::new();

    Ok(ReplayAndRollback {
        recording_control: args.global_options.replay_recording(),
        recorder,
        player,
        rollback_checker,
        rewind_buffer,
        start_paused: args.start_paused,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_archive_creation_failure_rejects_mission_initialization() {
        let directory = tempfile::tempdir().unwrap();
        let blocker = directory.path().join("not-a-directory");
        std::fs::write(&blocker, b"occupied").unwrap();
        let args = crate::main_entry::MissionLaunch {
            config: crate::main_entry::CliArgs {
                record: Some(blocker.join("recording").to_string_lossy().into_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        let error = init_recording(
            &Default::default(),
            &LevelAssets::new(),
            &args,
            "Fixture",
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "Fixture", "Fixture", "Fixture",
            )
            .unwrap(),
            0,
            Default::default(),
        )
        .err()
        .expect("recording creation failure must reject setup");
        assert!(error.contains("Failed to create mission recording"));
        assert_eq!(std::fs::read(blocker).unwrap(), b"occupied");
    }
    use std::io::Write as _;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    #[should_panic(
        expected = "raw replay input must be decoded before mission Engine construction"
    )]
    fn replay_attachment_rejects_raw_input_before_recording_or_file_access() {
        let directory = tempfile::tempdir().unwrap();
        let save_root = directory.path().to_string_lossy().into_owned();
        let mut players =
            robin_engine::player_profile::PlayerProfileManager::new(save_root.clone());
        let player = players.create_profile(
            "Replay Attachment Test".into(),
            robin_engine::player_profile::DifficultyLevel::Medium,
        );
        players.set_active(player);
        let application_context = crate::host::ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&save_root),
            Default::default(),
            players,
            crate::key_config_store::KeyConfigStore::new(save_root),
            None,
        )
        .unwrap();
        let args = crate::main_entry::MissionLaunch {
            global_options: application_context,
            config: crate::main_entry::CliArgs {
                replay: Some("must-not-be-read.rhrec.jsonl".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        init_replay_and_rollback(
            &Default::default(),
            Arc::new(LevelAssets::new()),
            &args,
            "Fixture",
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "Fixture", "Fixture", "Fixture",
            )
            .unwrap(),
            0,
            Default::default(),
            false,
            None,
        );
    }

    struct ControlledPrimary {
        bytes: Arc<Mutex<Vec<u8>>>,
        max_write: usize,
        fail_write: Arc<AtomicBool>,
        fail_flush: Arc<AtomicBool>,
    }

    impl std::io::Write for ControlledPrimary {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.fail_write.swap(false, Ordering::SeqCst) {
                return Err(std::io::Error::other("injected primary write failure"));
            }
            let written = self.max_write.min(buf.len());
            self.bytes
                .lock()
                .expect("controlled replay primary poisoned")
                .extend_from_slice(&buf[..written]);
            Ok(written)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            if self.fail_flush.swap(false, Ordering::SeqCst) {
                Err(std::io::Error::other("injected primary flush failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn tee_mirrors_exact_primary_short_write_prefixes() {
        let service = crate::replay_service::ReplayService::default();
        let primary_bytes = Arc::new(Mutex::new(Vec::new()));
        let mut tee = TeeWriter {
            primary: Box::new(ControlledPrimary {
                bytes: Arc::clone(&primary_bytes),
                max_write: 3,
                fail_write: Arc::new(AtomicBool::new(false)),
                fail_flush: Arc::new(AtomicBool::new(false)),
            }),
            mirror: service.begin_recording(),
            skip_mirror_header: false,
        };
        tee.write_all(b"complete replay record\n").unwrap();
        tee.flush().unwrap();
        assert_eq!(
            primary_bytes
                .lock()
                .expect("controlled replay primary poisoned")
                .as_slice(),
            b"complete replay record\n"
        );
        assert_eq!(
            service.snapshot_bytes().unwrap(),
            b"complete replay record\n"
        );
    }

    #[test]
    fn headless_and_debug_mission_start_paths_are_independently_tainted() {
        assert!(mission_start_input_taints(false, false, false).is_empty());
        assert_eq!(
            mission_start_input_taints(true, false, false),
            BTreeSet::from([InputTaintKind::HeadlessAutomation])
        );
        assert_eq!(
            mission_start_input_taints(false, true, false),
            BTreeSet::from([InputTaintKind::DebugInputInjection])
        );
        assert_eq!(
            mission_start_input_taints(true, true, false),
            BTreeSet::from([
                InputTaintKind::HeadlessAutomation,
                InputTaintKind::DebugInputInjection,
            ])
        );
    }

    #[test]
    fn tee_never_publishes_primary_write_or_flush_failures() {
        let service = crate::replay_service::ReplayService::default();
        let primary_bytes = Arc::new(Mutex::new(Vec::new()));
        let mut tee = TeeWriter {
            primary: Box::new(ControlledPrimary {
                bytes: Arc::clone(&primary_bytes),
                max_write: usize::MAX,
                fail_write: Arc::new(AtomicBool::new(true)),
                fail_flush: Arc::new(AtomicBool::new(false)),
            }),
            mirror: service.begin_recording(),
            skip_mirror_header: false,
        };
        assert!(tee.write_all(b"rejected\n").is_err());
        let error = service.snapshot_bytes().unwrap_err();
        assert!(error.contains("primary replay write failed"), "{error}");
        assert!(
            primary_bytes
                .lock()
                .expect("controlled replay primary poisoned")
                .is_empty()
        );

        let mut tee = TeeWriter {
            primary: Box::new(ControlledPrimary {
                bytes: Arc::clone(&primary_bytes),
                max_write: 0,
                fail_write: Arc::new(AtomicBool::new(false)),
                fail_flush: Arc::new(AtomicBool::new(false)),
            }),
            mirror: service.begin_recording(),
            skip_mirror_header: false,
        };
        assert!(tee.write_all(b"zero progress\n").is_err());
        let error = service.snapshot_bytes().unwrap_err();
        assert!(error.contains("made no progress"), "{error}");
        assert!(
            primary_bytes
                .lock()
                .expect("controlled replay primary poisoned")
                .is_empty()
        );

        let mut tee = TeeWriter {
            primary: Box::new(ControlledPrimary {
                bytes: Arc::clone(&primary_bytes),
                max_write: usize::MAX,
                fail_write: Arc::new(AtomicBool::new(false)),
                fail_flush: Arc::new(AtomicBool::new(true)),
            }),
            mirror: service.begin_recording(),
            skip_mirror_header: false,
        };
        tee.write_all(b"accepted by primary\n").unwrap();
        assert!(tee.flush().is_err());
        assert_eq!(
            primary_bytes
                .lock()
                .expect("controlled replay primary poisoned")
                .as_slice(),
            b"accepted by primary\n"
        );
        let error = service.snapshot_bytes().unwrap_err();
        assert!(error.contains("primary replay flush failed"), "{error}");
        assert!(tee.flush().is_err(), "poisoned mirror must not recover");
    }

    #[test]
    fn restart_evidence_is_local_to_the_run_arguments() {
        let launch = crate::main_entry::MissionLaunch::default();
        let mut restarting = launch.clone();
        restarting.mission_restart = true;
        assert!(restarting.clone().mission_restart);
        assert_eq!(
            mission_start_input_taints(false, false, restarting.mission_restart),
            BTreeSet::from([InputTaintKind::MissionRestart])
        );
        assert!(mission_start_input_taints(false, false, launch.mission_restart).is_empty());
        assert!(!launch.mission_restart);
        assert!(!crate::main_entry::MissionLaunch::default().mission_restart);
        // Configuration files cannot supply internal restart evidence.
        let decoded: crate::main_entry::MissionLaunch =
            serde_json::from_str(r#"{"mission-restart":true}"#).unwrap();
        assert!(!decoded.mission_restart);
    }

    #[test]
    fn every_live_peer_records_the_same_canonical_replay_path() {
        assert!(should_record_local_replay(false, false));
        assert!(!should_record_local_replay(true, false));
        assert!(!should_record_local_replay(false, true));
    }
}

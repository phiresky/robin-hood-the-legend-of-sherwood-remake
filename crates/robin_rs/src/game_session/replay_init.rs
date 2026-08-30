//! Replay recorder/player and rollback-checker setup.
//!
//! Houses the `TeeWriter` adapter, the default replay path picker, the
//! `ReplayAndRollback` bundle, and `init_replay_and_rollback` itself.

use crate::rewind::RewindBuffer;
use crate::rollback_checker::RollbackChecker;
use robin_engine::engine::LevelAssets;
use robin_engine::replay::{ReplayPlayer, ReplayRecorder};
use std::sync::Arc;

/// `Write` adapter that forwards bytes to a primary sink (the
/// `.rhrec.jsonl` file on disk on native; [`std::io::sink`] on wasm
/// where the browser has no filesystem) and to the bounded segmented replay
/// spool used by the script-RPC `get-replay` endpoint.
///
/// Only used by `init_replay_and_rollback`; kept here (rather than in
/// `replay`) so the recorder itself stays filesystem-agnostic.
struct TeeWriter {
    primary: Box<dyn std::io::Write + Send>,
    mirror: crate::http_server::ReplaySpoolWriter,
}

impl std::io::Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Reject known spool backpressure before changing the durable file. If
        // the primary performs a legitimate short write, mirror only the
        // accepted prefix and let `write_all` retry the remainder.
        self.mirror.preflight_write(buf.len())?;
        let written = match self.primary.write(buf) {
            Ok(written) => written,
            Err(error) => {
                self.mirror
                    .poison(format!("primary replay write failed: {error}"));
                return Err(error);
            }
        };
        std::io::Write::write_all(&mut self.mirror, &buf[..written])?;
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
    #[cfg(feature = "native-fs")]
    let dir = dirs::data_dir()
        .map(|d| d.join("robin_hood").join("replays"))
        .unwrap_or_else(|| PathBuf::from("Data/Replays"));
    #[cfg(not(feature = "native-fs"))]
    let dir = PathBuf::from("Data/Replays");
    // `%:z` → `+HH:MM`; we strip the inner colon so the whole stamp is
    // filesystem-safe (e.g. `2026-04-17T09-32-15+02-00`).
    let stamp = jiff::Zoned::now()
        .strftime("%Y-%m-%dT%H-%M-%S%:z")
        .to_string()
        .replace(':', "-");
    dir.join(format!("{stamp}{}", crate::main_entry::RHREC_EXT))
        .to_string_lossy()
        .into_owned()
}

#[cfg(not(target_arch = "wasm32"))]
fn replay_debug_log_path(replay_path: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(replay_path);
    let filename = path
        .file_name()
        .map(|name| format!("{}.debug.log", name.to_string_lossy()))
        .unwrap_or_else(|| "replay.debug.log".to_string());
    path.with_file_name(filename)
}

/// Bundle of determinism-related mission state built by
/// [`init_replay_and_rollback`] — replay recorder, replay player,
/// rollback checker, and the hold-to-rewind snapshot buffer.
pub(super) struct ReplayAndRollback {
    pub(super) recorder: Option<ReplayRecorder>,
    pub(super) player: Option<ReplayPlayer>,
    pub(super) rollback_checker: Option<RollbackChecker>,
    pub(super) rewind_buffer: RewindBuffer,
    /// Final value of the "start paused" toggle — true if either
    /// `--start-paused` was passed on the command line, or a pending
    /// `load-replay` RPC call requested it.
    pub(super) start_paused: bool,
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
    args: &crate::main_entry::CliArgs,
    _mission_idx: usize,
    mission_id: &str,
    engine_rng_seed: u64,
    engine_sim_config: robin_engine::engine::SimConfig,
    is_multiplayer: bool,
    is_canonical_replay_owner: bool,
) -> ReplayAndRollback {
    // Every queued replay must be converted into `args.replay_data` before
    // mission construction. Reseeding an already-built Engine cannot recreate
    // random draws performed during level initialization.
    assert!(
        crate::http_server::peek_pending_replay_mission_id().is_none(),
        "pending replay must be consumed and supplied before mission Engine construction"
    );
    let pending_paused = false;

    // No recording while playing back (either source).
    let is_playing_back = args.replay_data.is_some() || args.replay.is_some();
    #[cfg(not(target_arch = "wasm32"))]
    let replay_path = if is_playing_back {
        None
    } else {
        Some(args.record.clone().unwrap_or_else(default_replay_path))
    };
    #[cfg(target_arch = "wasm32")]
    let replay_path: Option<String> = None;
    // A fresh mission gets a fresh generation of the bounded spool. The
    // returned sole writer publishes only complete recorder flush boundaries.
    let rpc_spool = crate::http_server::reset_replay_buffer();
    // One-shot mission-map rendering exits before the first simulation
    // frame, so producing an empty replay (and its debug log) would only be
    // an unrelated filesystem side effect of the capture tool.
    let recorder = if is_playing_back
        || args.mission_start_map_output.is_some()
        || !is_canonical_replay_owner
    {
        if is_multiplayer && !is_canonical_replay_owner {
            tracing::info!(
                "multiplayer: replay recording disabled on peer; the host owns the canonical ordered replay"
            );
        }
        None
    } else {
        // Native path owns an on-disk `.rhrec.jsonl` file so replays
        // survive across sessions.  On wasm the browser has no
        // filesystem — we fall back to `std::io::sink` for the
        // primary and rely exclusively on the mirror buffer (which
        // `get-replay` serializes straight back to the JS caller).
        #[cfg(not(target_arch = "wasm32"))]
        let primary: Option<Box<dyn std::io::Write + Send>> = {
            let path = replay_path
                .as_deref()
                .expect("native replay recording has a path");
            if let Some(parent) = std::path::Path::new(path).parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                tracing::error!("Failed to create replay dir {parent:?}: {e}");
                None
            } else {
                match std::fs::File::create(path) {
                    Ok(f) => {
                        tracing::info!("Recording replay → {path}");
                        let log_path = replay_debug_log_path(path);
                        if let Err(e) = crate::set_replay_log_file(&log_path) {
                            tracing::warn!(
                                "Failed to create replay debug log {}: {e}",
                                log_path.display()
                            );
                        }
                        Some(Box::new(f))
                    }
                    Err(e) => {
                        tracing::error!("Failed to create replay file {path}: {e}");
                        None
                    }
                }
            }
        };
        #[cfg(target_arch = "wasm32")]
        let primary: Option<Box<dyn std::io::Write + Send>> = {
            tracing::info!("Recording replay (in-memory only — wasm)");
            Some(Box::new(std::io::sink()))
        };

        // `mission_id` (e.g. `"Dem_Lei_MP"`, `"Sherwood"`) is the
        // `.rhm` filename — stamped into the header so a later
        // `--replay` picks the right mission without threading the
        // campaign index through. `replay_campaign` is the exact clone made
        // immediately before Engine construction; the engine-owned campaign
        // may already have been changed by level initialization (Sherwood
        // clears its mission team after using it to spawn PCs).
        primary.and_then(|primary| {
            let writer: Box<dyn std::io::Write + Send> = Box::new(TeeWriter {
                primary,
                mirror: rpc_spool,
            });
            match ReplayRecorder::with_writer(
                writer,
                mission_id.to_string(),
                engine_rng_seed,
                engine_sim_config,
                replay_campaign,
            ) {
                Ok(rec) => Some(rec),
                Err(e) => {
                    tracing::error!("Failed to initialize replay recorder: {e}");
                    None
                }
            }
        })
    };

    let player = if let Some(data) = args.replay_data.clone() {
        tracing::info!(
            "Loaded replay (decoded): mission `{}`, {} frames, seed {}",
            data.header.mission_id,
            data.frame_count(),
            data.header.rng_seed,
        );
        // No restore_rng_from_seed here: see EngineArgs setup in
        // `load_level_and_sprite_bank` — the engine RNG was already
        // seeded at construction with this header's seed.
        Some(ReplayPlayer::new(data))
    } else {
        args.replay
            .as_ref()
            .and_then(|spec| match crate::replay_format::load_replay_spec(spec) {
                Ok(data) => {
                    tracing::info!(
                        "Loaded replay: mission `{}`, {} frames, seed {}",
                        data.header.mission_id,
                        data.frame_count(),
                        data.header.rng_seed,
                    );
                    // No restore_rng_from_seed here: see EngineArgs
                    // setup in `load_level_and_sprite_bank` — the
                    // engine RNG was already seeded at construction
                    // with this header's seed.
                    Some(ReplayPlayer::new(data))
                }
                Err(e) => {
                    tracing::error!("Failed to load replay: {e}");
                    None
                }
            })
    };

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
        let rollback_replay_path = recorder.as_ref().and(replay_path.clone());
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

    ReplayAndRollback {
        recorder,
        player,
        rollback_checker,
        rewind_buffer,
        start_paused: args.start_paused || pending_paused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    fn replay_spool_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("replay spool test lock poisoned")
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
        let _serial = replay_spool_test_lock();
        let primary_bytes = Arc::new(Mutex::new(Vec::new()));
        let mut tee = TeeWriter {
            primary: Box::new(ControlledPrimary {
                bytes: Arc::clone(&primary_bytes),
                max_write: 3,
                fail_write: Arc::new(AtomicBool::new(false)),
                fail_flush: Arc::new(AtomicBool::new(false)),
            }),
            mirror: crate::http_server::reset_replay_buffer(),
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
            crate::http_server::replay_buffer_snapshot().unwrap(),
            b"complete replay record\n"
        );
    }

    #[test]
    fn tee_never_publishes_primary_write_or_flush_failures() {
        let _serial = replay_spool_test_lock();
        let primary_bytes = Arc::new(Mutex::new(Vec::new()));
        let mut tee = TeeWriter {
            primary: Box::new(ControlledPrimary {
                bytes: Arc::clone(&primary_bytes),
                max_write: usize::MAX,
                fail_write: Arc::new(AtomicBool::new(true)),
                fail_flush: Arc::new(AtomicBool::new(false)),
            }),
            mirror: crate::http_server::reset_replay_buffer(),
        };
        assert!(tee.write_all(b"rejected\n").is_err());
        let error = crate::http_server::replay_buffer_snapshot().unwrap_err();
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
                max_write: usize::MAX,
                fail_write: Arc::new(AtomicBool::new(false)),
                fail_flush: Arc::new(AtomicBool::new(true)),
            }),
            mirror: crate::http_server::reset_replay_buffer(),
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
        let error = crate::http_server::replay_buffer_snapshot().unwrap_err();
        assert!(error.contains("primary replay flush failed"), "{error}");
        assert!(tee.flush().is_err(), "poisoned mirror must not recover");
    }
}

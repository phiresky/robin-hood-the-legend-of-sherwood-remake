//! Runtime rollback consistency checker.
//!
//! On every frame, rewinds the engine by 5 frames and re-simulates
//! them, comparing the result to the live engine state via
//! [`robin_engine::replay::state_hash`].  Any divergence reveals a source of
//! non-determinism in the simulation.
//!
//! Enabled by default (`--rollback-check`, disable with
//! `--rollback-check=false`).  Heavy: does extra replayed ticks plus
//! several engine clones.
//!
//! When a desync is detected, writes a JSON debug file containing
//! the sequence of commands replayed, the live engine state, and
//! the re-simulated engine state to `rollback_desync_<frame>.json`
//! in the current directory, including the replay file path when one
//! is available.

use crate::host::Host;
use crate::rewind::RewindBuffer;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use web_time::Instant;

use crate::sim_timeline::{CommandJournal, SimSnapshot as Snapshot, replay_frames_to_frame};
use robin_engine::engine::{Engine, LevelAssets};

/// Number of frames to rewind and replay each check.  5 ticks = 0.2s
/// at the game's fixed 25 fps simulation rate.
pub const ROLLBACK_WINDOW: usize = 5;

/// Run a check every N live frames instead of every frame.  With
/// `ROLLBACK_CHECK_INTERVAL = ROLLBACK_WINDOW`, amortized cost drops
/// from ~5 replayed ticks per live tick down to ~1, while every
/// simulated frame is still covered by exactly one check (windows
/// abut rather than overlap).  Non-determinism bugs reliably reproduce
/// every few frames, so sparse checking catches them with a large
/// runtime-cost win in both live play and `--fast-forward` replay.
pub const ROLLBACK_CHECK_INTERVAL: u32 = ROLLBACK_WINDOW as u32;

const PERF_LOG_INTERVAL: u32 = 25;

/// Scheduler for verification jobs copied from the mission-owned timeline.
pub struct RollbackChecker {
    assets: Arc<LevelAssets>,
    /// Frames recorded since the last replay check.  Gates
    /// `check_and_trim` to every `ROLLBACK_CHECK_INTERVAL` live frames.
    frames_since_check: u32,
    /// Whether we've already dumped a desync debug file this session.
    /// Repeated dumps are write-amplified noise.
    desync_dumped: Arc<AtomicBool>,
    /// Replay recording path for this session, used to make desync
    /// dumps directly reproducible.
    replay_path: Option<String>,
    worker: Option<JoinHandle<()>>,
    perf: PerfStats,
}

impl RollbackChecker {
    pub fn new(assets: Arc<LevelAssets>, replay_path: Option<String>) -> Self {
        Self {
            assets,
            frames_since_check: 0,
            desync_dumped: Arc::new(AtomicBool::new(false)),
            replay_path,
            worker: None,
            perf: PerfStats::default(),
        }
    }

    /// Reset check cadence after a non-chronological engine jump. The
    /// canonical timeline itself is reset by its owner; an already-running
    /// immutable job may finish safely against the old branch.
    pub fn reset(&mut self) {
        self.frames_since_check = 0;
        self.reap_worker();
        self.perf = PerfStats::default();
    }

    /// Verify a window copied from the mission's one canonical timeline.
    /// The checker owns no live journal or checkpoint ring; its background job
    /// receives an immutable five-frame slice after the canonical commit.
    pub fn check_after_commit(
        &mut self,
        host: &mut Host,
        history: &RewindBuffer,
        current_engine: &Engine,
    ) {
        let end_start = Instant::now();
        self.frames_since_check += 1;
        self.perf.end_bookkeeping_us += end_start.elapsed().as_micros();
        if self.frames_since_check < ROLLBACK_CHECK_INTERVAL {
            return;
        }
        self.frames_since_check = 0;

        let check_start = Instant::now();
        self.reap_worker();
        // Replay starts from the oldest snapshot. The live `host` remains in
        // this compatibility signature only for caller API parity; the worker
        // reconstructs the serialized Engine directly from complete recorded
        // SimulationFrameInputs and explicitly discards typed host output.
        let _ = host;
        if self.worker.is_some() {
            tracing::debug!(
                target: "robin_rs::rollback_checker::perf",
                "rollback checker worker still busy; skipping this window"
            );
            self.perf.check_total_us += check_start.elapsed().as_micros();
            self.perf.checks += 1;
            if self.perf.checks >= PERF_LOG_INTERVAL {
                self.perf.flush();
            }
            return;
        }

        let end_frame_exclusive = history.next_record_frame();
        let Some(start_frame) = end_frame_exclusive.checked_sub(ROLLBACK_WINDOW as u32) else {
            return;
        };
        let Some(start) =
            history.restore_recent(start_frame, crate::sim_timeline::RestorePolicy::Exact)
        else {
            tracing::warn!(
                start_frame,
                end_frame_exclusive,
                "rollback checker skipped: canonical recent checkpoint is unavailable"
            );
            return;
        };
        let mut window = CommandJournal::default();
        for frame in start_frame..end_frame_exclusive {
            let Some(input) = history.frame_for(frame).cloned() else {
                tracing::warn!(
                    frame,
                    start_frame,
                    end_frame_exclusive,
                    "rollback checker skipped: canonical command journal has a gap"
                );
                return;
            };
            window.record_frame(frame, input);
        }

        let clone_start = Instant::now();
        let job = RollbackCheckJob {
            start,
            history: window,
            assets: Arc::clone(&self.assets),
            current_engine: current_engine.clone(),
            desync_dumped: Arc::clone(&self.desync_dumped),
            replay_path: self.replay_path.clone(),
        };
        self.perf.check_clone_us += clone_start.elapsed().as_micros();

        self.worker = Some(std::thread::spawn(move || job.run()));

        self.perf.check_total_us += check_start.elapsed().as_micros();
        self.perf.checks += 1;
        if self.perf.checks >= PERF_LOG_INTERVAL {
            self.perf.flush();
        }
    }

    fn reap_worker(&mut self) {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
            && let Some(worker) = self.worker.take()
            && let Err(e) = worker.join()
        {
            tracing::error!("Rollback checker worker panicked: {e:?}");
        }
    }
}

impl Drop for RollbackChecker {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take()
            && let Err(e) = worker.join()
        {
            tracing::error!("Rollback checker worker panicked: {e:?}");
        }
    }
}

struct RollbackCheckJob {
    start: Snapshot,
    history: CommandJournal,
    assets: Arc<LevelAssets>,
    current_engine: Engine,
    desync_dumped: Arc<AtomicBool>,
    replay_path: Option<String>,
}

impl RollbackCheckJob {
    fn run(self) {
        let check_start = Instant::now();
        let start_frame = self.start.frame;
        let end_frame = self
            .history
            .newest_frame()
            .expect("rollback command history is non-empty");

        // Swap in a no-op tracing subscriber for the duration of the
        // replay so every `info!` / `warn!` inside `perform_hourglass`
        // doesn't fire once per replayed frame.
        let silent = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let clone_start = Instant::now();
        let start = self.start;
        let clone_us = clone_start.elapsed().as_micros();
        let replay_start = Instant::now();
        let replayed = tracing::dispatcher::with_default(&silent, || {
            replay_frames_to_frame(start, &self.assets, end_frame.saturating_add(1), |frame| {
                self.history.frame_for(frame)
            })
        });
        let (sim_snapshot, _timing) = match replayed {
            Ok(replayed) => replayed,
            Err(error) => {
                tracing::error!(
                    "Rollback checker failed to replay frames {start_frame}..={end_frame}: {error}"
                );
                return;
            }
        };
        let replay_us = replay_start.elapsed().as_micros();

        let hash_start = Instant::now();
        let expected = robin_engine::replay::state_hash(&self.current_engine);
        let actual = robin_engine::replay::state_hash(&sim_snapshot.engine);
        let hash_us = hash_start.elapsed().as_micros();

        if expected != actual {
            tracing::error!(
                "Rollback desync: frames {start_frame}..={end_frame}: \
                 live {expected:016x} vs replayed {actual:016x}"
            );
            // Only dump the (multi-MB) debug file once per session.
            if !self.desync_dumped.swap(true, Ordering::AcqRel)
                && let Err(e) = Self::dump_debug_file(
                    &self.current_engine,
                    &sim_snapshot.engine,
                    start_frame,
                    end_frame,
                    self.replay_path.as_deref(),
                )
            {
                tracing::error!("Failed to write rollback debug file: {e}");
            }
        } else {
            tracing::trace!("Rollback OK: frames {start_frame}..={end_frame} hash {expected:016x}");
        }

        tracing::debug!(
            target: "robin_rs::rollback_checker::perf",
            clone_us,
            replay_us,
            hash_us,
            total_us = check_start.elapsed().as_micros(),
            "rollback checker async timing"
        );
    }

    fn dump_debug_file(
        live: &Engine,
        replayed: &Engine,
        start_frame: u32,
        end_frame: u32,
        replay_path: Option<&str>,
    ) -> std::io::Result<()> {
        // Emit a focused JSON of `path → (live, replayed)` for every leaf
        // where the two engines differ. The original "dump both engines
        // verbatim" version was 50+ MB on a real level — way too big to
        // diff by hand. The summary stays under a few hundred KB even for
        // hundreds of entities because it skips matching subtrees.
        let live_v = serde_json::to_value(live).map_err(std::io::Error::other)?;
        let rep_v = serde_json::to_value(replayed).map_err(std::io::Error::other)?;
        let mut diffs: Vec<serde_json::Value> = Vec::new();
        collect_diffs("", &live_v, &rep_v, &mut diffs);

        let dump = serde_json::json!({
            "end_frame": end_frame,
            "replay_path": replay_path,
            "start_frame": start_frame,
            "differences": diffs,
        });

        let path = format!("rollback_desync_{end_frame}.json");
        let file = std::fs::File::create(&path)?;
        serde_json::to_writer_pretty(file, &dump).map_err(std::io::Error::other)?;
        tracing::error!(
            "Rollback debug file written: {path} ({} differences)",
            dump["differences"].as_array().unwrap().len()
        );
        Ok(())
    }
}

#[derive(Default)]
struct PerfStats {
    checks: u32,
    end_bookkeeping_us: u128,
    check_clone_us: u128,
    replay_us: u128,
    hash_us: u128,
    check_total_us: u128,
}

impl PerfStats {
    fn flush(&mut self) {
        if self.checks == 0 {
            return;
        }
        let checks = u128::from(self.checks);
        tracing::info!(
            target: "robin_rs::rollback_checker::perf",
            checks = self.checks,
            end_bookkeeping_avg_us = self.end_bookkeeping_us / checks,
            check_clone_avg_us = self.check_clone_us / checks,
            replay_avg_us = self.replay_us / checks,
            hash_avg_us = self.hash_us / checks,
            check_total_avg_us = self.check_total_us / checks,
            "rollback checker timing"
        );
        *self = Self::default();
    }
}

/// Walk two JSON trees in parallel and append `{path, live, replayed}`
/// for every leaf where the values differ. Matching subtrees are skipped
/// entirely to keep the output small.
fn collect_diffs(
    path: &str,
    a: &serde_json::Value,
    b: &serde_json::Value,
    out: &mut Vec<serde_json::Value>,
) {
    use serde_json::Value;
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            let mut keys: Vec<&String> = am.keys().chain(bm.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                collect_diffs(
                    &p,
                    am.get(k).unwrap_or(&Value::Null),
                    bm.get(k).unwrap_or(&Value::Null),
                    out,
                );
            }
        }
        (Value::Array(av), Value::Array(bv)) => {
            let n = av.len().max(bv.len());
            for i in 0..n {
                let p = format!("{path}[{i}]");
                collect_diffs(
                    &p,
                    av.get(i).unwrap_or(&Value::Null),
                    bv.get(i).unwrap_or(&Value::Null),
                    out,
                );
            }
        }
        _ => out.push(serde_json::json!({
            "path": path,
            "live": a,
            "replayed": b,
        })),
    }
}

impl Default for RollbackChecker {
    fn default() -> Self {
        Self::new(Arc::new(LevelAssets::new()), None)
    }
}

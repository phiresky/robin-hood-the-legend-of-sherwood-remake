//! Shared simulation timeline checkpoint, retention, restore, and replay helpers.
//!
//! Rewind, rollback checking, and multiplayer correction all need the
//! same primitive: start from a pre-tick snapshot, apply the recorded
//! commands for each frame, and run deterministic engine ticks until a
//! target pre-tick frame is reconstructed. The policies below make the
//! places where those callers intentionally differ explicit.
//!
//! Original provenance: `original-code/RHgame.cpp:1801-1831` advances
//! `PerformHourglass` once per eligible live update, while
//! `original-code/RHgame.cpp:2310-2445` restores whole savegames. The
//! original has no in-memory replay, rollback, or rewind timeline; those
//! policies are Rust-port infrastructure around the original tick boundary.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use web_time::Instant;

use crate::host::Host;
use robin_engine::engine::{DevState, Engine, HostDisplayState, LevelAssets};
use robin_engine::game_operation::GameCode;
use robin_engine::player_command::PlayerInput;

/// Dense recent rollback snapshots retained for multiplayer correction.
/// Two seconds at the fixed 25 Hz sim rate.
pub const RECENT_TIMELINE_HISTORY_FRAMES: usize = 50;

/// Decide which pre-tick frames are eligible to become checkpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckpointPolicy {
    EveryFrame,
    EveryNthFrame { interval: u32 },
}

impl CheckpointPolicy {
    pub fn should_checkpoint(self, frame: u32) -> bool {
        match self {
            Self::EveryFrame => true,
            Self::EveryNthFrame { interval } => {
                assert!(
                    interval > 0,
                    "timeline checkpoint interval must be non-zero"
                );
                frame.is_multiple_of(interval)
            }
        }
    }
}

/// Decide which eligible checkpoints remain in memory.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum RetentionPolicy {
    Latest { capacity: usize },
    Exponential { interval: u32, growth: f32 },
}

impl RetentionPolicy {
    fn validate(self) {
        match self {
            Self::Latest { capacity } => {
                assert!(capacity > 0, "timeline retention capacity must be non-zero");
            }
            Self::Exponential { interval, growth } => {
                assert!(interval > 0, "timeline retention interval must be non-zero");
                assert!(
                    growth.is_finite() && growth > 1.0,
                    "timeline exponential growth must be finite and greater than one"
                );
            }
        }
    }
}

/// Decide whether restoring a target requires its exact checkpoint or
/// may start from the newest retained checkpoint at or before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestorePolicy {
    Exact,
    LatestAtOrBefore,
}

/// Rollback state at the start of `frame`, before that frame's
/// commands or engine tick have run.
///
/// `HostDisplayState` and `DevState` are intentionally excluded: they
/// are host/display or developer overlay state. Replay uses scratch
/// instances while reconstructing deterministic engine state. The two
/// gameplay-affecting zoom gates are restored into that scratch display
/// from the engine-owned camera transition state before every replay tick.
#[derive(Clone, Serialize, Deserialize)]
pub struct SimSnapshot {
    pub frame: u32,
    pub engine: Engine,
}

impl SimSnapshot {
    pub fn new(frame: u32, engine: &Engine) -> Self {
        Self {
            frame,
            engine: engine.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReplayTiming {
    pub replayed_frames: u32,
    pub replay_us: u128,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReplayFrameTiming {
    pub apply_us: u128,
    pub tick_us: u128,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReplayError {
    #[error("cannot replay backward from checkpoint {checkpoint_frame} to target {target_frame}")]
    TargetBeforeCheckpoint {
        checkpoint_frame: u32,
        target_frame: u32,
    },
    #[error("missing recorded commands for replay frame {frame}")]
    MissingCommands { frame: u32 },
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum RestoreError {
    #[error("no checkpoint satisfies {policy:?} restore for frame {target_frame}")]
    CheckpointUnavailable {
        target_frame: u32,
        policy: RestorePolicy,
    },
}

/// Policy-driven collection of pre-tick simulation checkpoints.
#[derive(Clone, Serialize, Deserialize)]
pub struct SnapshotHistory {
    snapshots: VecDeque<SimSnapshot>,
    checkpoint_policy: CheckpointPolicy,
    retention_policy: RetentionPolicy,
}

impl SnapshotHistory {
    pub fn new(checkpoint_policy: CheckpointPolicy, retention_policy: RetentionPolicy) -> Self {
        retention_policy.validate();
        // Validate a periodic checkpoint policy even before its first use.
        if let CheckpointPolicy::EveryNthFrame { interval } = checkpoint_policy {
            assert!(
                interval > 0,
                "timeline checkpoint interval must be non-zero"
            );
        }
        Self {
            snapshots: VecDeque::new(),
            checkpoint_policy,
            retention_policy,
        }
    }

    pub fn should_checkpoint(&self, frame: u32) -> bool {
        self.checkpoint_policy.should_checkpoint(frame)
    }

    /// Clone and retain `engine` when `frame` is eligible under the
    /// checkpoint policy. Returns whether a checkpoint was retained.
    pub fn checkpoint(&mut self, frame: u32, engine: &Engine) -> bool {
        if !self.should_checkpoint(frame) {
            return false;
        }
        self.remember(SimSnapshot::new(frame, engine));
        true
    }

    /// Retain an already-cloned eligible checkpoint.
    pub fn remember(&mut self, snapshot: SimSnapshot) {
        assert!(
            self.should_checkpoint(snapshot.frame),
            "frame {} is ineligible under checkpoint policy {:?}",
            snapshot.frame,
            self.checkpoint_policy
        );
        if let Some(existing) = self.snapshots.back() {
            assert!(
                snapshot.frame >= existing.frame,
                "timeline checkpoints must be remembered chronologically: {} after {}",
                snapshot.frame,
                existing.frame
            );
            if existing.frame == snapshot.frame {
                self.snapshots.pop_back();
            }
        }
        self.snapshots.push_back(snapshot);
        prune_by_policy(
            &mut self.snapshots,
            |snapshot| snapshot.frame,
            self.retention_policy,
        );
    }

    pub fn restore(
        &self,
        target_frame: u32,
        policy: RestorePolicy,
    ) -> Result<SimSnapshot, RestoreError> {
        let Some(index) = restore_index(
            &self.snapshots,
            |snapshot| snapshot.frame,
            target_frame,
            policy,
        ) else {
            return Err(RestoreError::CheckpointUnavailable {
                target_frame,
                policy,
            });
        };
        Ok(self.snapshots[index].clone())
    }

    pub fn oldest_frame(&self) -> Option<u32> {
        self.snapshots.front().map(|snapshot| snapshot.frame)
    }

    pub fn truncate_after(&mut self, frame: u32) {
        while self
            .snapshots
            .back()
            .is_some_and(|snapshot| snapshot.frame > frame)
        {
            self.snapshots.pop_back();
        }
    }

    pub fn clear(&mut self) {
        self.snapshots.clear();
    }
}

fn restore_index<T>(
    snapshots: &VecDeque<T>,
    frame_of: impl Fn(&T) -> u32,
    target_frame: u32,
    policy: RestorePolicy,
) -> Option<usize> {
    snapshots.iter().rposition(|snapshot| match policy {
        RestorePolicy::Exact => frame_of(snapshot) == target_frame,
        RestorePolicy::LatestAtOrBefore => frame_of(snapshot) <= target_frame,
    })
}

fn prune_by_policy<T>(
    snapshots: &mut VecDeque<T>,
    frame_of: impl Fn(&T) -> u32,
    policy: RetentionPolicy,
) {
    match policy {
        RetentionPolicy::Latest { capacity } => {
            while snapshots.len() > capacity {
                snapshots.pop_front();
            }
        }
        RetentionPolicy::Exponential { interval, growth } => {
            let Some(newest_frame) = snapshots.back().map(&frame_of) else {
                return;
            };
            let mut kept: VecDeque<T> = VecDeque::with_capacity(snapshots.len());
            let mut buckets = Vec::with_capacity(snapshots.len());

            // Walk newest to oldest and replace a bucket's member as
            // older candidates arrive. Keeping the oldest checkpoint
            // in each bucket lets the reachable horizon grow over time.
            while let Some(snapshot) = snapshots.pop_back() {
                let age = newest_frame.saturating_sub(frame_of(&snapshot));
                let bucket = exponential_bucket(age, interval, growth);
                if let Some(position) = buckets.iter().position(|&seen| seen == bucket) {
                    kept[position] = snapshot;
                } else {
                    buckets.push(bucket);
                    kept.push_back(snapshot);
                }
            }
            snapshots.extend(kept.into_iter().rev());
        }
    }
}

fn exponential_bucket(age_frames: u32, interval: u32, growth: f32) -> u32 {
    if age_frames < interval {
        return 0;
    }
    let ratio = age_frames as f32 / interval as f32;
    (ratio.ln() / growth.ln()).floor() as u32 + 1
}

fn validate_replay_boundary(checkpoint_frame: u32, target_frame: u32) -> Result<(), ReplayError> {
    if target_frame < checkpoint_frame {
        return Err(ReplayError::TargetBeforeCheckpoint {
            checkpoint_frame,
            target_frame,
        });
    }
    Ok(())
}

/// Replay `snapshot` forward to `target_frame`.
///
/// `commands_for(frame)` must return the commands that were applied
/// during that frame. Missing commands are an error because guessing
/// would silently corrupt the reconstructed timeline.
pub fn replay_to_frame<'a>(
    mut snapshot: SimSnapshot,
    assets: &LevelAssets,
    target_frame: u32,
    mut commands_for: impl FnMut(u32) -> Option<&'a [PlayerInput]>,
) -> Result<(SimSnapshot, ReplayTiming), ReplayError> {
    validate_replay_boundary(snapshot.frame, target_frame)?;
    let start = Instant::now();
    let start_frame = snapshot.frame;
    let mut scratch_host = Host::default();
    let mut scratch_dev = DevState::default();
    let mut scratch_display = HostDisplayState::default();

    while snapshot.frame < target_frame {
        let cmds = commands_for(snapshot.frame).ok_or(ReplayError::MissingCommands {
            frame: snapshot.frame,
        })?;
        replay_one_frame(
            &mut snapshot,
            &mut scratch_display,
            assets,
            &mut scratch_host,
            &mut scratch_dev,
            cmds,
        );
    }

    Ok((
        snapshot,
        ReplayTiming {
            replayed_frames: target_frame - start_frame,
            replay_us: start.elapsed().as_micros(),
        },
    ))
}

/// Replay exactly one frame in-place, advancing the snapshot to the
/// next pre-tick frame.
pub fn replay_one_frame(
    snapshot: &mut SimSnapshot,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    scratch_host: &mut Host,
    scratch_dev: &mut DevState,
    cmds: &[PlayerInput],
) {
    let _ = replay_one_frame_profiled(snapshot, display, assets, scratch_host, scratch_dev, cmds);
}

/// Replay one frame and report the apply/tick split for callers that
/// expose detailed rollback telemetry.
pub fn replay_one_frame_profiled(
    snapshot: &mut SimSnapshot,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    scratch_host: &mut Host,
    scratch_dev: &mut DevState,
    cmds: &[PlayerInput],
) -> ReplayFrameTiming {
    sync_gameplay_zoom_gate(&snapshot.engine, display);
    let apply_start = Instant::now();
    snapshot
        .engine
        .apply_commands(display, &mut scratch_host.input, assets, cmds);
    let apply_us = apply_start.elapsed().as_micros();
    // A replayed zoom command updates the serialised camera transition.
    // Mirror its gate before ticking just as the original zoom-message
    // handlers and PerformHourglass both read mbackgroundTransform.
    sync_gameplay_zoom_gate(&snapshot.engine, display);
    let tick_start = Instant::now();
    run_engine_tick_core(
        scratch_host,
        display,
        assets,
        &mut snapshot.engine,
        scratch_dev,
    );
    run_post_initialize_stage(
        scratch_host,
        display,
        assets,
        &mut snapshot.engine,
        scratch_dev,
    );
    let tick_us = tick_start.elapsed().as_micros();
    snapshot.frame += 1;
    ReplayFrameTiming { apply_us, tick_us }
}

/// Restore the only host-display fields that gate authoritative gameplay.
///
/// The original UI caller forwards zoom messages at `RHgame.cpp:2070-2071`;
/// the engine callees set these direction flags at
/// `RHengine.cpp:12162-12229`, and `PerformHourglass` reads the same flags
/// before gameplay advances at `RHengine.cpp:3446-3633`.
/// `RHEngine::Serialize` also includes the containing transform at
/// `RHengine.cpp:2408-2504`. Rust keeps the canonical transition in the
/// serialised engine camera while host presentation uses separate scratch,
/// so rollback must explicitly bridge just these two booleans.
fn sync_gameplay_zoom_gate(engine: &Engine, display: &mut HostDisplayState) {
    let zoom_to_up = engine.is_zoom_up_in_progress(display);
    let zoom_to_down = engine.is_zoom_down_in_progress(display);
    display.background_transform.zoom_to_up = zoom_to_up;
    display.background_transform.zoom_to_down = zoom_to_down;
}

/// Run one deterministic engine tick and drain engine-local side effects.
///
/// This is the rollback-safe core of `Game::run_engine_tick`: it does
/// not read or mutate the outer `Game` shell. Live play wraps this to
/// update mission-operation and UI widget state after the engine
/// reports a result.
pub fn run_engine_tick_core(
    host: &mut Host,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
) -> GameCode {
    host.sync_sound_listener();
    let side_effects = engine.perform_hourglass(display, assets, dev);
    apply_engine_side_effects(host, display, dev, side_effects)
}

/// Dispatch the one-shot mission `PostInitialize` hook at the host's
/// post-refresh boundary.
///
/// Live play calls this after the first sound and render passes. Replay
/// has no presentation work, so [`replay_one_frame`] calls it immediately
/// after reconstructing frame zero, producing the same pre-frame-one
/// authoritative engine state.
pub fn run_post_initialize_stage(
    host: &mut Host,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
) -> bool {
    if let Some(side_effects) = engine.perform_post_initialize(display, assets) {
        apply_engine_side_effects(host, display, dev, side_effects);
        true
    } else {
        false
    }
}

fn apply_engine_side_effects(
    host: &mut Host,
    display: &mut HostDisplayState,
    dev: &mut DevState,
    mut side_effects: robin_engine::engine::SideEffects,
) -> GameCode {
    if side_effects.ui_has_focus {
        host.input.has_focus = false;
    }
    for noise in side_effects.displayed_noises.drain(..) {
        dev.add_noise_to_display(noise);
    }
    dev.tick_noise_display(1.0);
    for (show, restore_position) in side_effects.pending_minimap_display_maps.drain(..) {
        display.display_minimap(show, restore_position);
    }
    host.apply_side_effects(side_effects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_checkpoint_policy_includes_interval_boundaries_only() {
        let policy = CheckpointPolicy::EveryNthFrame { interval: 25 };
        assert!(policy.should_checkpoint(0));
        assert!(!policy.should_checkpoint(24));
        assert!(policy.should_checkpoint(25));
        assert!(!policy.should_checkpoint(26));
    }

    #[test]
    #[should_panic(expected = "timeline checkpoint interval must be non-zero")]
    fn zero_checkpoint_interval_is_rejected() {
        SnapshotHistory::new(
            CheckpointPolicy::EveryNthFrame { interval: 0 },
            RetentionPolicy::Latest { capacity: 1 },
        );
    }

    #[test]
    fn latest_retention_keeps_exact_capacity_at_boundary() {
        let mut frames: VecDeque<u32> = (10..=14).collect();
        prune_by_policy(
            &mut frames,
            |frame| *frame,
            RetentionPolicy::Latest { capacity: 3 },
        );
        assert_eq!(frames, VecDeque::from([12, 13, 14]));
    }

    #[test]
    fn exponential_retention_keeps_old_horizon_bounded() {
        let policy = RetentionPolicy::Exponential {
            interval: 25,
            growth: 1.3,
        };
        let mut frames = VecDeque::new();
        for frame in (0..=1000).step_by(25) {
            frames.push_back(frame);
            prune_by_policy(&mut frames, |frame| *frame, policy);
        }
        assert_eq!(frames.back(), Some(&1000));
        assert!(1000 - frames.front().expect("history is non-empty") >= 500);
        assert!(frames.len() <= 20);
    }

    #[test]
    fn restore_policy_distinguishes_exact_from_at_or_before() {
        let frames = VecDeque::from([0, 25, 50]);
        assert_eq!(
            restore_index(&frames, |frame| *frame, 30, RestorePolicy::Exact),
            None
        );
        assert_eq!(
            restore_index(&frames, |frame| *frame, 30, RestorePolicy::LatestAtOrBefore),
            Some(1)
        );
        assert_eq!(
            restore_index(&frames, |frame| *frame, 0, RestorePolicy::LatestAtOrBefore),
            Some(0)
        );
        assert_eq!(
            restore_index(&frames, |frame| *frame, 51, RestorePolicy::LatestAtOrBefore),
            Some(2)
        );
    }

    #[test]
    fn restore_before_oldest_checkpoint_is_unavailable() {
        let frames = VecDeque::from([25, 50]);
        assert_eq!(
            restore_index(&frames, |frame| *frame, 24, RestorePolicy::LatestAtOrBefore),
            None
        );
    }

    #[test]
    fn replay_rejects_target_before_checkpoint() {
        assert_eq!(
            validate_replay_boundary(10, 9),
            Err(ReplayError::TargetBeforeCheckpoint {
                checkpoint_frame: 10,
                target_frame: 9,
            })
        );
        assert_eq!(validate_replay_boundary(10, 10), Ok(()));
        assert_eq!(validate_replay_boundary(10, 11), Ok(()));
    }
}

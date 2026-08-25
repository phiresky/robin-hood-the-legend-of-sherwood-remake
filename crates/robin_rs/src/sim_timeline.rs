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
/// instances while reconstructing deterministic engine state; no field on
/// either scratch owner is allowed to gate the Engine tick.
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

/// Chronological journal of the deterministic commands applied at each
/// simulation frame.
///
/// Rewind and rollback verification intentionally retain different amounts of
/// history, but they must agree on frame addressing, late-input edits, and
/// branch truncation.  Keeping those rules here prevents each consumer from
/// maintaining its own `oldest_frame + VecDeque` arithmetic.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommandJournal {
    commands: VecDeque<Vec<PlayerInput>>,
    /// Frame represented by `commands[0]`. When truncation empties a journal,
    /// this remains the frame at which its next branch must begin.
    oldest_frame: u32,
    /// Required frame for the next record. `None` only for a fresh or fully
    /// cleared journal whose first record establishes a new timeline anchor.
    next_frame: Option<u32>,
}

impl CommandJournal {
    /// Record one complete frame. Non-empty journals are strictly contiguous:
    /// a gap or duplicate means a caller crossed a timeline discontinuity
    /// without first clearing or truncating the journal.
    pub fn record(&mut self, frame: u32, commands: Vec<PlayerInput>) {
        if let Some(expected) = self.next_frame {
            assert_eq!(
                frame, expected,
                "timeline commands must be recorded contiguously"
            );
        }
        if self.commands.is_empty() && self.next_frame.is_none() {
            self.oldest_frame = frame;
        }
        self.commands.push_back(commands);
        self.next_frame = Some(
            frame
                .checked_add(1)
                .expect("simulation frame counter overflowed command journal"),
        );
    }

    pub fn commands_for(&self, frame: u32) -> Option<&[PlayerInput]> {
        let index = frame.checked_sub(self.oldest_frame)? as usize;
        self.commands.get(index).map(Vec::as_slice)
    }

    pub fn oldest_frame(&self) -> Option<u32> {
        (!self.commands.is_empty()).then_some(self.oldest_frame)
    }

    pub fn newest_frame(&self) -> Option<u32> {
        (!self.commands.is_empty()).then(|| {
            self.next_frame
                .expect("non-empty command journal has a next frame")
                - 1
        })
    }

    pub fn next_frame(&self) -> u32 {
        self.next_frame.unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Add an input to an already-recorded frame. Returns `false` when the
    /// requested frame is outside the retained journal.
    pub fn append_input(&mut self, frame: u32, input: PlayerInput) -> bool {
        let Some(index) = frame.checked_sub(self.oldest_frame) else {
            return false;
        };
        let Some(commands) = self.commands.get_mut(index as usize) else {
            return false;
        };
        commands.push(input);
        true
    }

    /// Discard commands for `frame` and its future, retaining the prefix that
    /// remains valid on a newly-created branch.
    pub fn truncate_from(&mut self, frame: u32) {
        let Some(index) = frame.checked_sub(self.oldest_frame) else {
            return;
        };
        let index = index as usize;
        if index < self.commands.len() {
            self.commands.truncate(index);
            self.next_frame = Some(frame);
            if self.commands.is_empty() {
                self.oldest_frame = frame;
            }
        }
    }

    /// Drop commands older than the earliest checkpoint that can still be
    /// restored.
    pub fn discard_before(&mut self, frame: u32) {
        while !self.commands.is_empty() && self.oldest_frame < frame {
            self.commands.pop_front();
            self.oldest_frame += 1;
        }
    }

    pub fn clear(&mut self) {
        self.commands.clear();
        self.oldest_frame = 0;
        self.next_frame = None;
    }
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

/// A checkpoint store and its command journal with one pre-tick frame
/// lifecycle.
///
/// `begin_frame` captures the optional pre-tick checkpoint. `commit_frame`
/// publishes that checkpoint and the commands together only after the tick
/// completes. An abandoned host iteration may call `begin_frame` again; the
/// previous pending capture was never authoritative and is replaced.
#[derive(Clone, Serialize, Deserialize)]
pub struct TimelineHistory {
    checkpoints: SnapshotHistory,
    commands: CommandJournal,
    pending: Option<PendingFrame>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PendingFrame {
    frame: u32,
    checkpoint: Option<SimSnapshot>,
}

impl TimelineHistory {
    pub fn new(checkpoint_policy: CheckpointPolicy, retention_policy: RetentionPolicy) -> Self {
        Self {
            checkpoints: SnapshotHistory::new(checkpoint_policy, retention_policy),
            commands: CommandJournal::default(),
            pending: None,
        }
    }

    pub fn begin_frame(&mut self, frame: u32, engine: &Engine) {
        let checkpoint = self
            .checkpoints
            .should_checkpoint(frame)
            .then(|| SimSnapshot::new(frame, engine));
        self.pending = Some(PendingFrame { frame, checkpoint });
    }

    /// Commit the open frame. Returns `false` only while the history has no
    /// checkpoint anchor yet; commands before that first checkpoint cannot be
    /// replayed and are deliberately not journaled.
    pub fn commit_frame(&mut self, commands: Vec<PlayerInput>) -> bool {
        let pending = self
            .pending
            .take()
            .expect("timeline frame committed without a matching begin_frame");

        if self.checkpoints.oldest_frame().is_none() && pending.checkpoint.is_none() {
            return false;
        }
        if let Some(checkpoint) = pending.checkpoint {
            self.checkpoints.remember(checkpoint);
        }
        self.commands.record(pending.frame, commands);
        if let Some(oldest_checkpoint) = self.checkpoints.oldest_frame() {
            self.commands.discard_before(oldest_checkpoint);
        }
        true
    }

    pub fn restore(
        &self,
        target_frame: u32,
        policy: RestorePolicy,
    ) -> Result<SimSnapshot, RestoreError> {
        self.checkpoints.restore(target_frame, policy)
    }

    pub fn commands_for(&self, frame: u32) -> Option<&[PlayerInput]> {
        self.commands.commands_for(frame)
    }

    pub fn append_input(&mut self, frame: u32, input: PlayerInput) -> bool {
        self.commands.append_input(frame, input)
    }

    pub fn oldest_checkpoint_frame(&self) -> Option<u32> {
        self.checkpoints.oldest_frame()
    }

    pub fn oldest_command_frame(&self) -> Option<u32> {
        self.commands.oldest_frame()
    }

    pub fn next_record_frame(&self) -> u32 {
        self.commands.next_frame()
    }

    pub fn truncate_future(&mut self, frame: u32) {
        // Preserve the old rewind contract: a target before the command
        // horizon cannot create a valid branch, so neither journal nor
        // checkpoints are changed.
        if self
            .commands
            .oldest_frame()
            .is_some_and(|oldest| frame < oldest)
        {
            return;
        }
        self.commands.truncate_from(frame);
        self.checkpoints.truncate_after(frame);
        self.pending = None;
    }

    pub fn clear(&mut self) {
        self.checkpoints.clear();
        self.commands.clear();
        self.pending = None;
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
    let apply_start = Instant::now();
    snapshot
        .engine
        .apply_commands(display, &mut scratch_host.input, assets, cmds);
    let apply_us = apply_start.elapsed().as_micros();
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

    #[test]
    fn command_journal_addresses_edits_and_branches_by_absolute_frame() {
        use robin_engine::player_command::{PlayerCommand, PlayerId};

        let mut journal = CommandJournal::default();
        journal.record(40, Vec::new());
        journal.record(41, Vec::new());
        journal.record(42, Vec::new());

        let late = PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown);
        assert!(journal.append_input(41, late));
        assert_eq!(journal.commands_for(41).map(<[_]>::len), Some(1));
        assert!(
            !journal.append_input(39, PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown))
        );

        journal.truncate_from(42);
        assert_eq!(journal.newest_frame(), Some(41));
        assert_eq!(journal.next_frame(), 42);
        assert!(journal.commands_for(42).is_none());

        journal.record(42, Vec::new());
        journal.discard_before(41);
        assert_eq!(journal.oldest_frame(), Some(41));
        assert_eq!(journal.commands_for(41).map(<[_]>::len), Some(1));
    }

    #[test]
    #[should_panic(expected = "timeline commands must be recorded contiguously")]
    fn command_journal_rejects_unannounced_discontinuity() {
        let mut journal = CommandJournal::default();
        journal.record(7, Vec::new());
        journal.record(9, Vec::new());
    }

    #[test]
    fn timeline_history_commits_checkpoint_and_commands_at_one_boundary() {
        use robin_engine::campaign::Campaign;
        use robin_engine::player_command::{PlayerCommand, PlayerId};

        let mut assets = LevelAssets::default();
        let engine = Engine::new_for_test(640.0, 480.0, Campaign::default(), &mut assets)
            .expect("fixture engine");
        let mut history = TimelineHistory::new(
            CheckpointPolicy::EveryFrame,
            RetentionPolicy::Latest { capacity: 2 },
        );
        let command = PlayerInput::new(PlayerId(1), PlayerCommand::CrouchDown);

        history.begin_frame(12, &engine);
        assert!(history.commit_frame(vec![command.clone()]));
        assert_eq!(history.oldest_checkpoint_frame(), Some(12));
        assert_eq!(history.oldest_command_frame(), Some(12));
        let recorded = history.commands_for(12).expect("frame-12 commands");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].player_id, command.player_id);
        assert_eq!(
            history
                .restore(12, RestorePolicy::Exact)
                .expect("frame-12 checkpoint")
                .frame,
            12
        );

        // Opening a replacement host iteration before either commits is safe:
        // neither pending capture was published into the timeline yet.
        history.begin_frame(13, &engine);
        history.begin_frame(13, &engine);
        assert!(history.commit_frame(Vec::new()));
        assert_eq!(history.next_record_frame(), 14);
    }
}

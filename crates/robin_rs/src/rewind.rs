//! Hold-to-rewind debug feature.
//!
//! Periodically captures rollback sim state (`Engine`) every
//! [`SNAPSHOT_INTERVAL`] frames (10 seconds), retaining every checkpoint
//! for the mission. While the rewind key is held, the main loop asks the
//! buffer for the state at `sim_frame - 1`; the buffer locates the
//! nearest snapshot at or before the target frame, restores it, and
//! replays complete authoritative frames to reconstruct the exact pre-tick state at
//! the target frame.
//!
//! The per-frame command journal is an instance of the same shared timeline
//! primitive used by [`crate::rollback_checker::RollbackChecker`]. It remains
//! independent of [`robin_engine::replay::ReplayRecorder`] (which writes JSONL
//! to disk), and has to cover the full span from the oldest retained snapshot
//! to "now", so checkpoints and the journal grow with mission duration.
//!
//! This is a dev / debug feature; bypasses the replay recorder and the
//! rollback checker while active (both would see the time-reversal as
//! a desync).
//!
//! Inspired by the "time rewind" feature in *Braid*.
//!
//! Older sparse checkpoints use bitcode + zstd, retaining one live engine.
//! All 50 dense recent checkpoints remain uncompressed. Active rewind sessions
//! separately cache up to 25 live states for consecutive backward steps.
//! Inputs share one command journal.

use std::collections::BTreeMap;

use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::player_command::PlayerInput;
use robin_engine::sim_timeline::{
    CheckpointPolicy, RestorePolicy, RetentionPolicy, SimSnapshot as Snapshot, SnapshotHistory,
    TimelineHistory, replay_authoritative_frame,
};

/// Ten seconds at the simulation rate of 25 frames per second.
pub const SNAPSHOT_INTERVAL: u32 = 250;

/// Bound live reconstruction memory independently of checkpoint spacing.
const SESSION_CACHE_FRAMES: u32 = 25;

/// Mission-owned canonical in-memory timeline: one per-frame command journal
/// plus dense multiplayer/checker and fixed-interval interactive-rewind
/// checkpoint tiers. The historical type name is retained because most of
/// its public operations are rewind-oriented, but no other live subsystem may
/// own a parallel command history.
pub struct RewindBuffer {
    /// Shared checkpoint + command-journal lifecycle. Rewind adds only its
    /// interactive seek cache and whole-mission retention policy around this
    /// reusable timeline primitive.
    history: TimelineHistory,
    /// Dense short-horizon checkpoint tier used by multiplayer correction
    /// and rollback verification. It shares `history`'s one canonical command
    /// journal instead of maintaining a parallel timeline store.
    recent_checkpoints: SnapshotHistory,
    pending_recent: Option<Snapshot>,
    /// Active rewind session cache — populated while BACKSPACE is
    /// held so consecutive rewind-steps reuse earlier replay work
    /// instead of re-cloning a snapshot and ticking forward from
    /// scratch each time.
    ///
    /// Pruned on every [`Self::rewind_to`] call to drop entries past
    /// the current target (rewind walks monotonically backward within
    /// a session). Only the last [`SESSION_CACHE_FRAMES`] reconstructed
    /// states are cached. Cleared entirely by [`Self::end_session`].
    session: Option<BTreeMap<u32, Snapshot>>,
}

impl RewindBuffer {
    pub fn new() -> Self {
        Self {
            history: TimelineHistory::new(
                CheckpointPolicy::EveryNthFrame {
                    interval: SNAPSHOT_INTERVAL,
                },
                RetentionPolicy::All,
            ),
            recent_checkpoints: SnapshotHistory::new(
                CheckpointPolicy::EveryFrame,
                RetentionPolicy::Latest {
                    capacity: robin_engine::sim_timeline::RECENT_TIMELINE_HISTORY_FRAMES,
                },
            ),
            pending_recent: None,
            session: None,
        }
    }

    /// Start a rewind session: subsequent [`Self::rewind_to`] calls
    /// will cache the most recent reconstructed states so walking backward
    /// across consecutive frames hits the cache instead of re-ticking
    /// from a snapshot.  Idempotent — safe to call while a session is
    /// already open.
    pub fn begin_session(&mut self) {
        if self.session.is_none() {
            self.session = Some(BTreeMap::new());
        }
    }

    /// End the current rewind session and drop any cached states.
    pub fn end_session(&mut self) {
        self.session = None;
    }

    /// Capture pre-tick state.  Call once per non-paused frame, before
    /// `engine.apply_commands` + tick, with `frame` equal to the
    /// current `sim_frame` (the frame about to be ticked).
    ///
    /// The snapshot is stashed in `pending` and only committed by
    /// [`Self::end_frame_input`] if this frame aligns to
    /// [`SNAPSHOT_INTERVAL`] — non-aligned frames still need to
    /// register their commands but don't add to the snapshot ring.
    pub fn begin_frame(&mut self, frame: u32, engine: &Engine) {
        self.history.begin_frame(frame, engine);
        self.pending_recent = Some(Snapshot::new(frame, engine));
    }

    /// Journal a seek tick without copying the network rollback cache.
    /// Periodic checkpoints and commands still support ordinary rewind.
    pub(crate) fn begin_seek_frame(&mut self, frame: u32, engine: &Engine) {
        self.history.begin_frame(frame, engine);
        self.pending_recent = None;
    }

    /// Anchor a freshly reset timeline at a whole-state adoption boundary.
    /// Snapshot joins and save/load can land between the sparse tier's normal
    /// periodic frames, but their very next command must still be journaled.
    pub fn seed_initial_anchor(&mut self, frame: u32, engine: &Engine) {
        self.history.seed_initial_anchor(frame, engine);
        self.recent_checkpoints.replace_with_anchor(frame, engine);
        self.pending_recent = None;
        self.session = None;
    }

    /// Finalize the frame: commit the pending snapshot (if any), push
    /// the frame's commands onto the log, and retain periodic checkpoints.
    pub fn end_frame_input(&mut self, input: robin_engine::engine::SimulationFrameInput) {
        if self.history.commit_frame_input(input)
            && let Some(snapshot) = self.pending_recent.take()
        {
            self.recent_checkpoints.remember(snapshot);
        }
    }

    /// Reconstruct the pre-tick sim state at `target_frame` by
    /// locating the closest starting point at or before `target_frame`
    /// — a session-cached state if one exists, otherwise the nearest
    /// retained snapshot — and replaying commands + ticks forward
    /// until we arrive.  Returns `None` when `target_frame` predates
    /// every retained snapshot or when we're missing a command entry
    /// along the way (shouldn't happen in practice, but guarded for
    /// safety).
    ///
    /// Replay advances only the snapshotted [`Engine`]. Typed host output is
    /// explicitly discarded, so reconstruction neither mutates live host state
    /// nor invents a second host/input/display owner.
    ///
    /// When a session is open (see [`Self::begin_session`]) the last 25
    /// intermediate states produced by the replay loop are cached so
    /// the next backward step (target_frame - 1) reuses the work.
    /// Entries past the current target are pruned here because
    /// rewind walks monotonically backward within a session.
    pub fn rewind_to(&mut self, assets: &LevelAssets, target_frame: u32) -> Option<Engine> {
        // Prune cache entries past the current target — they're the
        // "future" we've already rewound past and won't revisit.
        if let Some(cache) = &mut self.session
            && let Some(first_future_frame) = target_frame.checked_add(1)
        {
            cache.split_off(&first_future_frame);
        }

        // Fast path: target itself is cached.
        if let Some(hit) = self.session.as_ref().and_then(|c| c.get(&target_frame)) {
            return Some(hit.engine.clone());
        }

        // Pick the closest starting point ≤ target_frame.  A cached
        // state beats a retained snapshot when both are available.
        let mut snapshot = self
            .history
            .restore(assets, target_frame, RestorePolicy::LatestAtOrBefore)
            .ok()?;
        if let Some(cache) = &self.session
            && let Some((&cached_frame, cached)) = cache.range(..=target_frame).next_back()
            && cached_frame > snapshot.frame
        {
            snapshot = cached.clone();
        }

        while snapshot.frame < target_frame {
            let frame = self.history.frame_for(snapshot.frame)?;
            let _discarded_frame_output =
                replay_authoritative_frame(&mut snapshot, assets, frame).output;
            // Cache the state we just produced — it's the pre-tick
            // state for `frame + 1`.
            if let Some(cache) = &mut self.session
                && target_frame - snapshot.frame < SESSION_CACHE_FRAMES
            {
                cache.insert(snapshot.frame, snapshot.clone());
                while cache.len() > SESSION_CACHE_FRAMES as usize {
                    cache.pop_first();
                }
            }
        }

        Some(snapshot.engine)
    }

    /// How far back (in frames) the oldest retained snapshot reaches
    /// from the newest.  Used by the main loop to decide whether a
    /// rewind request has any chance of succeeding.
    pub fn oldest_reachable_frame(&self) -> Option<u32> {
        self.history.oldest_checkpoint_frame()
    }

    /// The frame number that [`Self::end_frame_input`] would next record.
    /// Equal to the latest-recorded frame's number + 1 (or
    /// [`Self::oldest_cmd_frame`] when the log is empty).
    ///
    /// Used by the main loop to detect the "auto-replay" window: any
    /// `sim_frame < next_record_frame()` is a frame the buffer
    /// already has a transaction for, so the player is currently replaying
    /// forward through previously-recorded authoritative input after a rewind.
    pub fn next_record_frame(&self) -> u32 {
        self.history.next_record_frame()
    }

    /// Frame number of the oldest entry in the command log.  Frames
    /// before this have rolled off the buffer and can no longer be
    /// targeted by [`Self::rewind_to`] / [`Self::splice_late_input`].
    pub fn oldest_cmd_frame(&self) -> u32 {
        self.history.oldest_command_frame().unwrap_or(0)
    }

    /// Compatibility view of the recorded pre-hourglass commands.
    pub fn commands_for(&self, frame: u32) -> Option<Vec<PlayerInput>> {
        self.history.commands_for(frame)
    }

    pub fn frame_for(&self, frame: u32) -> Option<&robin_engine::engine::SimulationFrameInput> {
        self.history.frame_for(frame)
    }

    pub fn checkpoint_recent(&mut self, frame: u32, engine: &Engine) {
        self.recent_checkpoints.checkpoint(frame, engine);
    }

    pub fn restore_recent(
        &self,
        assets: &LevelAssets,
        frame: u32,
        policy: RestorePolicy,
    ) -> Option<Snapshot> {
        self.recent_checkpoints.restore(assets, frame, policy).ok()
    }

    pub fn recent_checkpoints(&self) -> &SnapshotHistory {
        &self.recent_checkpoints
    }

    pub fn replace_recent_checkpoints(&mut self, checkpoints: SnapshotHistory) {
        self.recent_checkpoints = checkpoints;
    }

    pub fn clear_recent_checkpoints(&mut self) {
        self.recent_checkpoints.clear();
        self.pending_recent = None;
    }

    pub fn truncate_recent_after(&mut self, frame: u32) {
        self.recent_checkpoints.truncate_after(frame);
    }

    /// Append a late-arriving input into the buffer's command log at
    /// `frame`.  Used by the multiplayer rollback path: when a peer
    /// input arrives stamped with a `target_frame` already in the
    /// past, we splice it into the buffer so the subsequent
    /// `rewind_to(current_frame)` reconstructs the engine state with
    /// the late input woven in.
    ///
    /// Returns `true` when the input landed.  `false` means `frame` is
    /// outside the buffered range — either older than
    /// [`Self::oldest_cmd_frame`] (snapshot rolled off — input is
    /// permanently lost, so the caller must request a full authoritative
    /// snapshot) or
    /// past [`Self::next_record_frame`] (caller should queue it as a
    /// future input instead of trying to splice).
    pub fn splice_late_input(&mut self, frame: u32, input: PlayerInput) -> bool {
        if !self.history.append_input(frame, input) {
            return false;
        }
        // Both checkpoint tiers are derived from the edited command stream.
        // The pre-tick checkpoint at `frame` remains valid; every later dense
        // checkpoint must be reconstructed before it can be published again.
        self.recent_checkpoints.truncate_after(frame);
        self.pending_recent = None;
        // Interactive seek caches are also derived state. Reusing one after
        // editing an earlier command would bypass the late input just like a
        // stale retained checkpoint would.
        self.session = None;
        true
    }

    /// Discard every command entry at `frame` or later, and every
    /// snapshot whose frame is strictly greater than `frame`.  Called
    /// when the player interrupts the replayed post-rewind timeline
    /// with a new live input — the buffered future is now obsolete.
    ///
    /// The snapshot at exactly `frame` is retained: it's the pre-tick
    /// state for the frame that's diverging, which is still a valid
    /// rewind target.
    pub fn truncate_future(&mut self, frame: u32) {
        self.history.truncate_future(frame);
        self.recent_checkpoints.truncate_after(frame);
        if self
            .pending_recent
            .as_ref()
            .is_some_and(|snapshot| snapshot.frame != frame)
        {
            self.pending_recent = None;
        }
    }
}

impl Default for RewindBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoints_keep_ten_second_cadence_and_the_mission_start() {
        let (engine, assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        let mut buffer = RewindBuffer::new();
        assert_eq!(SNAPSHOT_INTERVAL, 250);
        for frame in 0..=1000 {
            buffer.history.begin_frame(frame, &engine);
            buffer.history.commit_frame_input(Default::default());
        }
        for frame in [0, 250, 500, 750, 1000] {
            assert!(
                buffer
                    .history
                    .restore(&assets, frame, RestorePolicy::Exact)
                    .is_ok()
            );
        }
        for frame in [25, 249, 251, 999] {
            assert!(
                buffer
                    .history
                    .restore(&assets, frame, RestorePolicy::Exact)
                    .is_err()
            );
        }
        buffer.begin_session();
        assert!(buffer.rewind_to(&assets, 249).is_some());
        let cache = buffer.session.as_ref().unwrap();
        assert_eq!(cache.len(), SESSION_CACHE_FRAMES as usize);
        assert_eq!(cache.first_key_value().unwrap().0, &225);
    }

    #[test]
    fn seek_journal_reconstructs_intermediate_frames_without_recent_copies() {
        let (mut engine, assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        let mut buffer = RewindBuffer::new();
        let mut hashes = Vec::new();
        for frame in 0..=SNAPSHOT_INTERVAL + 3 {
            hashes.push(robin_engine::replay::state_hash(&engine));
            buffer.begin_seek_frame(frame, &engine);
            let input = robin_engine::engine::SimulationFrameInput::default();
            engine.advance_frame(&assets, input.clone()).unwrap();
            buffer.end_frame_input(input);
        }
        assert!(
            buffer
                .restore_recent(&assets, 249, RestorePolicy::Exact)
                .is_none()
        );
        for frame in [253, 251, 249, 1, 0] {
            let restored = buffer
                .rewind_to(&assets, frame)
                .expect("sparse seek history remains rewindable");
            assert_eq!(
                robin_engine::replay::state_hash(&restored),
                hashes[frame as usize]
            );
        }
    }

    #[test]
    fn session_pruning_keeps_the_target_and_handles_the_maximum_frame() {
        let (engine, assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        let frames = [0, 4, u32::MAX];
        for target in frames {
            let mut buffer = RewindBuffer::new();
            buffer.session = Some(
                frames
                    .into_iter()
                    .map(|frame| (frame, Snapshot::new(frame, &engine)))
                    .collect(),
            );
            let restored = buffer
                .rewind_to(&assets, target)
                .expect("cached target exists");
            assert_eq!(
                robin_engine::replay::state_hash(&restored),
                robin_engine::replay::state_hash(&engine)
            );
            assert_eq!(
                buffer
                    .session
                    .as_ref()
                    .unwrap()
                    .keys()
                    .copied()
                    .collect::<Vec<_>>(),
                frames
                    .into_iter()
                    .filter(|frame| *frame <= target)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn adopted_state_between_sparse_boundaries_journals_immediately() {
        let (engine, assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        let frame = SNAPSHOT_INTERVAL + 7;
        let mut buffer = RewindBuffer::new();

        buffer.seed_initial_anchor(frame, &engine);
        buffer.begin_frame(frame, &engine);
        buffer.end_frame_input(robin_engine::engine::SimulationFrameInput::default());

        assert!(buffer.frame_for(frame).is_some());
        assert_eq!(buffer.oldest_cmd_frame(), frame);
        assert!(buffer.rewind_to(&assets, frame).is_some());
        assert!(
            buffer
                .restore_recent(&assets, frame, RestorePolicy::Exact)
                .is_some()
        );
    }

    #[test]
    fn rewind_during_active_zoom_matches_uninterrupted_gameplay_gate() {
        use crate::sim_timeline::{run_engine_tick_core, run_post_initialize_stage};
        use robin_engine::campaign::Campaign;
        use robin_engine::engine::{DevState, EngineStateRequest};
        use robin_engine::messenger::SimpleMessage;
        use robin_engine::player_command::PlayerCommand;

        let mut assets = LevelAssets::new();
        let mut engine = Engine::new_for_test_with_level_size(
            1024.0,
            768.0,
            Campaign::default(),
            &mut assets,
            4096.0,
            4096.0,
        )
        .expect("fixture engine");
        let mut host = crate::host::Host::default();
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::new(vec![
                    PlayerCommand::ChangeState(EngineStateRequest::ZoomingUp).into(),
                ])
                .with_hourglass(false),
            )
            .expect("zoom command admission");
        assert!(engine.is_zoom_up_in_progress());

        // Modifier messages complete at admission, including during a camera
        // transition. Rewind must retain that state and the active zoom gate.
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::no_hourglass().with_external_actions(
                    vec![robin_engine::engine::ExternalAction::SimpleMessage {
                        message: SimpleMessage::LockAlt,
                    }],
                ),
            )
            .expect("LockAlt message admission");
        assert!(engine.is_lock_alt());

        let mut rewind = RewindBuffer::new();
        let application_context = host.application_context().clone();
        let mut dev = DevState::default();
        for frame in 0..3 {
            rewind.begin_frame(frame, &engine);

            // Deliberately keep host scratch contradictory. The Engine-owned
            // camera transition is the only gameplay gate.
            host.frontend
                .presentation
                .engine_display
                .background_transform
                .zoom_to_up = false;
            host.frontend
                .presentation
                .engine_display
                .background_transform
                .zoom_to_down = true;
            run_engine_tick_core(
                &mut host.frontend,
                &mut host.audio,
                &mut host.effects,
                &application_context,
                host.transport.local_seat(),
                &assets,
                &mut engine,
                &mut dev,
            );
            run_post_initialize_stage(
                &mut host.frontend,
                &mut host.audio,
                &mut host.effects,
                &application_context,
                host.transport.local_seat(),
                &assets,
                &mut engine,
                &mut dev,
                &[],
            );

            rewind.end_frame_input(robin_engine::engine::SimulationFrameInput::default());
        }

        assert!(engine.is_zoom_up_in_progress());
        assert!(engine.is_lock_alt());

        let rewound = rewind
            .rewind_to(&assets, 3)
            .expect("frame 3 is reconstructable from the frame-0 checkpoint");
        assert_eq!(
            robin_engine::replay::state_hash(&rewound),
            robin_engine::replay::state_hash(&engine)
        );
        assert!(rewound.is_zoom_up_in_progress());
        assert!(rewound.is_lock_alt());
    }

    #[test]
    fn splice_late_input_appends_to_correct_frame() {
        use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};

        let mut buf = RewindBuffer::new();
        let (engine, _assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        for frame in 0..3 {
            buf.begin_frame(frame, &engine);
            buf.end_frame_input(robin_engine::engine::SimulationFrameInput::default());
        }

        let inp = PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown);
        assert!(buf.splice_late_input(1, inp.clone()));
        assert_eq!(buf.commands_for(1).map(|s| s.len()), Some(1));
        assert_eq!(buf.commands_for(0).map(|s| s.len()), Some(0));
        assert_eq!(buf.commands_for(2).map(|s| s.len()), Some(0));

        // Out-of-range frames return false without mutating.
        assert!(!buf.splice_late_input(99, inp.clone()));
        buf.truncate_future(0);
        assert!(!buf.splice_late_input(2, inp));
    }

    #[test]
    fn splice_late_input_drops_snapshots_derived_from_the_old_command_stream() {
        use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
        use robin_engine::sim_timeline::{RestoreError, RestorePolicy};

        let mut buf = RewindBuffer::new();
        let (engine, assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        for frame in 0..=SNAPSHOT_INTERVAL {
            buf.begin_frame(frame, &engine);
            buf.end_frame_input(robin_engine::engine::SimulationFrameInput::default());
        }
        assert!(
            buf.history
                .restore(&assets, SNAPSHOT_INTERVAL, RestorePolicy::Exact)
                .is_ok()
        );
        assert!(
            buf.restore_recent(&assets, SNAPSHOT_INTERVAL, RestorePolicy::Exact)
                .is_some()
        );
        buf.begin_session();
        let input = PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown);

        let edited_frame = SNAPSHOT_INTERVAL - 1;
        assert!(buf.splice_late_input(edited_frame, input));
        assert!(buf.session.is_none());
        assert!(matches!(
            buf.history
                .restore(&assets, SNAPSHOT_INTERVAL, RestorePolicy::Exact),
            Err(RestoreError::CheckpointUnavailable { .. })
        ));
        assert!(
            buf.history
                .restore(&assets, 0, RestorePolicy::Exact)
                .is_ok()
        );
        assert!(
            buf.restore_recent(&assets, edited_frame, RestorePolicy::Exact)
                .is_some()
        );
        assert!(
            buf.restore_recent(&assets, SNAPSHOT_INTERVAL, RestorePolicy::Exact)
                .is_none(),
            "dense checkpoints derived after the edit must be reconstructed"
        );
        assert!(buf.commands_for(SNAPSHOT_INTERVAL).is_some());
    }
}

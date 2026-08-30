//! Hold-to-rewind debug feature.
//!
//! Periodically clones rollback sim state (`Engine`) every
//! [`SNAPSHOT_INTERVAL`] frames and
//! retains them with exponential spacing per [`BUCKET_GROWTH`]
//! (≈25, 33, 42, 55, 72, 93, 121, 157, 204, 265, … frames back).  While the rewind key is held, the main loop asks the
//! buffer for the state at `sim_frame - 1`; the buffer locates the
//! nearest snapshot at or before the target frame, clones it, and
//! replays complete authoritative frames to reconstruct the exact pre-tick state at
//! the target frame.
//!
//! The per-frame command journal is an instance of the same shared timeline
//! primitive used by [`crate::rollback_checker::RollbackChecker`]. It remains
//! independent of [`robin_engine::replay::ReplayRecorder`] (which writes JSONL
//! to disk), and has to cover the full span from the oldest retained snapshot
//! to "now", so it grows with how far back the oldest bucket reaches — bounded
//! by the exponential retention.
//!
//! This is a dev / debug feature; bypasses the replay recorder and the
//! rollback checker while active (both would see the time-reversal as
//! a desync).
//!
//! Inspired by the "time rewind" feature in *Braid*.
//!
//! Memory cost: ~16 full state clones plus one `Vec<PlayerCommand>`
//! per tracked frame.  The Engine already clones cheaply enough that
//! the rollback checker does it on every frame, so this is fine.

use std::collections::BTreeMap;

use crate::sim_timeline::{
    CheckpointPolicy, RestorePolicy, RetentionPolicy, SimSnapshot as Snapshot, TimelineHistory,
    replay_authoritative_frame,
};
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::player_command::PlayerInput;

/// How often (in sim frames) to take a snapshot.  Matches the cadence
/// of the replay state-hash check so the two systems have similar
/// memory pressure.
pub const SNAPSHOT_INTERVAL: u32 = 25;

/// Growth factor between consecutive retained snapshots (measured as
/// multiples of `SNAPSHOT_INTERVAL` frames).  Each older bucket
/// targets `interval × BUCKET_GROWTH^i` frames back, so retained
/// distances become roughly 25, 33, 42, 55, 72, 93, 121, 157, 204,
/// 265, … frames.  A value of 2.0 would yield only 25, 50, 100, 200,
/// … — coarser history; 1.3 is dense enough to give smooth rewind
/// across multi-second spans without blowing up snapshot count
/// (still ~log1.3(span) snapshots total).
const BUCKET_GROWTH: f32 = 1.3;

/// Ring buffer of sim snapshots plus a per-frame command log, used to
/// reconstruct any recent frame by replaying forward from the nearest
/// snapshot.
pub struct RewindBuffer {
    /// Shared checkpoint + command-journal lifecycle. Rewind adds only its
    /// interactive seek cache and exponential-retention policy around this
    /// reusable timeline primitive.
    history: TimelineHistory,
    /// Active rewind session cache — populated while BACKSPACE is
    /// held so consecutive rewind-steps reuse earlier replay work
    /// instead of re-cloning a snapshot and ticking forward from
    /// scratch each time.
    ///
    /// Pruned on every [`Self::rewind_to`] call to drop entries past
    /// the current target (rewind walks monotonically backward within
    /// a session), so the cache size stays bounded by one
    /// [`SNAPSHOT_INTERVAL`] of states — plenty small even at its
    /// worst case.  Cleared entirely by [`Self::end_session`].
    session: Option<BTreeMap<u32, Snapshot>>,
}

impl RewindBuffer {
    pub fn new() -> Self {
        Self {
            history: TimelineHistory::new(
                CheckpointPolicy::EveryNthFrame {
                    interval: SNAPSHOT_INTERVAL,
                },
                RetentionPolicy::Exponential {
                    interval: SNAPSHOT_INTERVAL,
                    growth: BUCKET_GROWTH,
                },
            ),
            session: None,
        }
    }

    /// Start a rewind session: subsequent [`Self::rewind_to`] calls
    /// will cache every reconstructed state so walking backward
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
    /// [`Self::end_frame`] if this frame aligns to
    /// [`SNAPSHOT_INTERVAL`] — non-aligned frames still need to
    /// register their commands but don't add to the snapshot ring.
    pub fn begin_frame(&mut self, frame: u32, engine: &Engine, _assets: &LevelAssets) {
        self.history.begin_frame(frame, engine);
    }

    /// Anchor a freshly reset rollback journal at a whole-state adoption
    /// boundary. Authoritative reconnect snapshots can land between the
    /// sparse tier's ordinary 25-frame checkpoints.
    pub fn seed_initial_anchor(&mut self, frame: u32, engine: &Engine) {
        self.history.seed_initial_anchor(frame, engine);
        self.session = None;
    }

    /// Finalize the frame: commit the pending snapshot (if any), push
    /// the frame's commands onto the log, and prune the snapshot ring
    /// to exponential spacing.
    pub fn end_frame(&mut self, cmds: Vec<PlayerInput>) {
        self.history.commit_frame(cmds);
    }

    pub fn end_frame_input(&mut self, input: robin_engine::engine::SimulationFrameInput) {
        self.history.commit_frame_input(input);
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
    /// When a session is open (see [`Self::begin_session`]) every
    /// intermediate state produced by the replay loop is cached so
    /// the next backward step (target_frame - 1) reuses the work.
    /// Entries past the current target are pruned here because
    /// rewind walks monotonically backward within a session.
    pub fn rewind_to(&mut self, assets: &LevelAssets, target_frame: u32) -> Option<Engine> {
        // Prune cache entries past the current target — they're the
        // "future" we've already rewound past and won't revisit.
        if let Some(cache) = &mut self.session {
            cache.split_off(&(target_frame + 1));
        }

        // Fast path: target itself is cached.
        if let Some(hit) = self.session.as_ref().and_then(|c| c.get(&target_frame)) {
            return Some(hit.engine.clone());
        }

        // Pick the closest starting point ≤ target_frame.  A cached
        // state beats a retained snapshot when both are available.
        let mut snapshot = self
            .history
            .restore(target_frame, RestorePolicy::LatestAtOrBefore)
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
            if let Some(cache) = &mut self.session {
                cache.insert(snapshot.frame, snapshot.clone());
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

    /// The frame number that [`Self::end_frame`] would next record.
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
    /// permanently lost, the only safe response is a desync alarm) or
    /// past [`Self::next_record_frame`] (caller should queue it as a
    /// future input instead of trying to splice).
    pub fn splice_late_input(&mut self, frame: u32, input: PlayerInput) -> bool {
        if !self.history.append_input(frame, input) {
            return false;
        }
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
    fn adopted_snapshot_seeds_an_off_cadence_timeline_anchor() {
        use robin_engine::campaign::Campaign;

        let mut assets = LevelAssets::new();
        let engine = Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
            .expect("fixture engine");
        let frame = SNAPSHOT_INTERVAL + 7;
        let mut buffer = RewindBuffer::new();

        buffer.seed_initial_anchor(frame, &engine);
        buffer.begin_frame(frame, &engine, &assets);
        buffer.end_frame(Vec::new());

        assert!(buffer.frame_for(frame).is_some());
        assert_eq!(buffer.oldest_cmd_frame(), frame);
        assert_eq!(buffer.oldest_reachable_frame(), Some(frame));
    }

    #[test]
    fn rewind_during_active_zoom_matches_uninterrupted_gameplay_gate() {
        use crate::sim_timeline::{run_engine_tick_core, run_post_initialize_stage};
        use robin_engine::campaign::Campaign;
        use robin_engine::engine::{DevState, EngineStateRequest, HostDisplayState};
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
        let mut display = HostDisplayState::default();
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::new(vec![
                    PlayerCommand::ChangeState(EngineStateRequest::ZoomingUp).into(),
                ])
                .with_hourglass(false),
            )
            .expect("zoom command admission");
        assert!(engine.is_zoom_up_in_progress(&display));

        // LockAlt is handled after the zoom gate in PerformHourglass. It
        // therefore remains pending throughout these active transition
        // frames, making an incorrectly defaulted replay display observable.
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

        let mut rewind = RewindBuffer::new();
        let mut host = crate::host::Host::default();
        let mut dev = DevState::default();
        for frame in 0..3 {
            rewind.begin_frame(frame, &engine, &assets);

            // Deliberately keep host scratch contradictory. The Engine-owned
            // camera transition is the only gameplay gate.
            display.background_transform.zoom_to_up = false;
            display.background_transform.zoom_to_down = true;
            run_engine_tick_core(&mut host, &mut display, &assets, &mut engine, &mut dev);
            run_post_initialize_stage(&mut host, &mut display, &assets, &mut engine, &mut dev, &[]);

            rewind.end_frame(Vec::new());
        }

        assert!(engine.is_zoom_up_in_progress(&display));
        assert!(!engine.is_lock_alt());

        let rewound = rewind
            .rewind_to(&assets, 3)
            .expect("frame 3 is reconstructable from the frame-0 checkpoint");
        assert_eq!(
            robin_engine::replay::state_hash(&rewound),
            robin_engine::replay::state_hash(&engine)
        );
        assert!(!rewound.is_lock_alt());
    }

    #[test]
    fn splice_late_input_appends_to_correct_frame() {
        use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};

        let mut buf = RewindBuffer::new();
        let mut assets = LevelAssets::default();
        let engine = Engine::new_for_test(
            640.0,
            480.0,
            robin_engine::campaign::Campaign::default(),
            &mut assets,
        )
        .expect("fixture engine");
        for frame in 0..3 {
            buf.begin_frame(frame, &engine, &assets);
            buf.end_frame(Vec::new());
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
        use crate::sim_timeline::{RestoreError, RestorePolicy};
        use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};

        let mut buf = RewindBuffer::new();
        let mut assets = LevelAssets::default();
        let engine = Engine::new_for_test(
            640.0,
            480.0,
            robin_engine::campaign::Campaign::default(),
            &mut assets,
        )
        .expect("fixture engine");
        for frame in 0..=SNAPSHOT_INTERVAL {
            buf.begin_frame(frame, &engine, &assets);
            buf.end_frame(Vec::new());
        }
        assert!(
            buf.history
                .restore(SNAPSHOT_INTERVAL, RestorePolicy::Exact)
                .is_ok()
        );
        buf.begin_session();
        let input = PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown);

        assert!(buf.splice_late_input(1, input));
        assert!(buf.session.is_none());
        assert!(matches!(
            buf.history.restore(SNAPSHOT_INTERVAL, RestorePolicy::Exact),
            Err(RestoreError::CheckpointUnavailable { .. })
        ));
        assert!(buf.history.restore(0, RestorePolicy::Exact).is_ok());
        assert!(buf.commands_for(SNAPSHOT_INTERVAL).is_some());
    }
}

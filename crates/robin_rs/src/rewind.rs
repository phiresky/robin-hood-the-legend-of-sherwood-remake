//! Hold-to-rewind debug feature.
//!
//! Periodically clones rollback sim state (`Engine`) every
//! [`SNAPSHOT_INTERVAL`] frames and
//! retains them with exponential spacing per [`BUCKET_GROWTH`]
//! (≈25, 33, 42, 55, 72, 93, 121, 157, 204, 265, … frames back).  While the rewind key is held, the main loop asks the
//! buffer for the state at `sim_frame - 1`; the buffer locates the
//! nearest snapshot at or before the target frame, clones it, and
//! replays commands forward to reconstruct the exact pre-tick state at
//! the target frame.
//!
//! The per-frame command log kept here is independent of
//! [`crate::replay::ReplayRecorder`] (which writes JSONL to disk) and
//! [`crate::rollback_checker::RollbackChecker`] (which only keeps a
//! short 5-frame ring).  It has to cover the full span from the oldest
//! retained snapshot to "now", so it grows with how far back the
//! oldest bucket reaches — bounded by the exponential retention.
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

use std::collections::{BTreeMap, VecDeque};

use crate::engine::{DevState, Engine, HostDisplayState, LevelAssets};
use crate::player_command::PlayerInput;
use crate::sim_timeline::{
    CheckpointPolicy, RestorePolicy, RetentionPolicy, SimSnapshot as Snapshot, SnapshotHistory,
    replay_one_frame,
};

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
    /// Exponentially spaced snapshots, oldest first.
    snapshots: SnapshotHistory,
    /// One entry per simulated frame from [`Self::oldest_cmd_frame`]
    /// up to the most recently recorded frame, holding the commands
    /// applied during that frame.
    commands: VecDeque<Vec<PlayerInput>>,
    /// Frame number of `commands[0]`.  Undefined when
    /// `commands.is_empty()`.
    oldest_cmd_frame: u32,
    /// Pending pre-tick snapshot captured in `begin_frame`, consumed
    /// by `end_frame`.  None when
    /// begin_frame hasn't been called for the current frame yet (e.g.
    /// paused frame).
    pending: Option<Snapshot>,
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
            snapshots: SnapshotHistory::new(
                CheckpointPolicy::EveryNthFrame {
                    interval: SNAPSHOT_INTERVAL,
                },
                RetentionPolicy::Exponential {
                    interval: SNAPSHOT_INTERVAL,
                    growth: BUCKET_GROWTH,
                },
            ),
            commands: VecDeque::new(),
            oldest_cmd_frame: 0,
            pending: None,
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
        if self.snapshots.should_checkpoint(frame) {
            self.pending = Some(Snapshot::new(frame, engine));
        } else {
            self.pending = None;
        }
    }

    /// Finalize the frame: commit the pending snapshot (if any), push
    /// the frame's commands onto the log, and prune the snapshot ring
    /// to exponential spacing.
    pub fn end_frame(&mut self, cmds: Vec<PlayerInput>) {
        let frame = if let Some(snap) = self.pending.take() {
            let f = snap.frame;
            self.snapshots.remember(snap);
            f
        } else if let Some(back) = self.commands.back() {
            // No snapshot this frame; infer the frame number from the
            // tail of the command log so we stay contiguous.
            let _ = back;
            self.oldest_cmd_frame + self.commands.len() as u32
        } else {
            // Very first frame after startup and it didn't align to
            // SNAPSHOT_INTERVAL.  Nothing to anchor the command log
            // against, so drop the commands — without a snapshot we
            // couldn't rewind into them anyway.
            return;
        };

        if self.commands.is_empty() {
            self.oldest_cmd_frame = frame;
        }
        self.commands.push_back(cmds);

        // Trim commands older than the oldest retained snapshot; they
        // can never be needed for a rewind replay (we always start
        // from a snapshot that's at or before the target frame).
        if let Some(oldest) = self.snapshots.oldest_frame() {
            while self.oldest_cmd_frame < oldest && !self.commands.is_empty() {
                self.commands.pop_front();
                self.oldest_cmd_frame += 1;
            }
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
    /// Replay uses a scratch [`Host`] so it can't mutate the live host
    /// state — same pattern as [`crate::rollback_checker`].
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
            .snapshots
            .restore(target_frame, RestorePolicy::LatestAtOrBefore)
            .ok()?;
        if let Some(cache) = &self.session
            && let Some((&cached_frame, cached)) = cache.range(..=target_frame).next_back()
            && cached_frame > snapshot.frame
        {
            snapshot = cached.clone();
        }

        let mut scratch_host = crate::Host::default();
        let mut scratch_dev = DevState::default();
        let mut scratch_display = HostDisplayState::default();
        while snapshot.frame < target_frame {
            let cmd_idx = snapshot.frame.checked_sub(self.oldest_cmd_frame)? as usize;
            let cmds = self.commands.get(cmd_idx)?;
            replay_one_frame(
                &mut snapshot,
                &mut scratch_display,
                assets,
                &mut scratch_host,
                &mut scratch_dev,
                cmds,
            );
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
        self.snapshots.oldest_frame()
    }

    /// The frame number that [`Self::end_frame`] would next record.
    /// Equal to the latest-recorded frame's number + 1 (or
    /// [`Self::oldest_cmd_frame`] when the log is empty).
    ///
    /// Used by the main loop to detect the "auto-replay" window: any
    /// `sim_frame < next_record_frame()` is a frame the buffer
    /// already has commands for, so the player is currently replaying
    /// forward through previously-recorded inputs after a rewind.
    pub fn next_record_frame(&self) -> u32 {
        self.oldest_cmd_frame + self.commands.len() as u32
    }

    /// Frame number of the oldest entry in the command log.  Frames
    /// before this have rolled off the buffer and can no longer be
    /// targeted by [`Self::rewind_to`] / [`Self::splice_late_input`].
    pub fn oldest_cmd_frame(&self) -> u32 {
        self.oldest_cmd_frame
    }

    /// Recorded commands for `frame`, if present.
    pub fn commands_for(&self, frame: u32) -> Option<&[PlayerInput]> {
        let idx = frame.checked_sub(self.oldest_cmd_frame)? as usize;
        self.commands.get(idx).map(Vec::as_slice)
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
        let Some(idx) = frame.checked_sub(self.oldest_cmd_frame) else {
            return false;
        };
        let Some(slot) = self.commands.get_mut(idx as usize) else {
            return false;
        };
        slot.push(input);
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
        let Some(idx) = frame.checked_sub(self.oldest_cmd_frame) else {
            return;
        };
        while self.commands.len() > idx as usize {
            self.commands.pop_back();
        }
        self.snapshots.truncate_after(frame);
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
    fn splice_late_input_appends_to_correct_frame() {
        use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};

        let mut buf = RewindBuffer::new();
        // Manually seed a few frames of command logs so we can splice
        // without standing up a full Engine.  oldest_cmd_frame defaults
        // to 0, so we mark frames 0..=2 as recorded.
        buf.commands.push_back(Vec::new());
        buf.commands.push_back(Vec::new());
        buf.commands.push_back(Vec::new());
        // begin_session would normally manage oldest_cmd_frame; force
        // it here to match the seed above.
        buf.oldest_cmd_frame = 0;

        let inp = PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown);
        assert!(buf.splice_late_input(1, inp.clone()));
        assert_eq!(buf.commands_for(1).map(|s| s.len()), Some(1));
        assert_eq!(buf.commands_for(0).map(|s| s.len()), Some(0));
        assert_eq!(buf.commands_for(2).map(|s| s.len()), Some(0));

        // Out-of-range frames return false without mutating.
        assert!(!buf.splice_late_input(99, inp.clone()));
        // Below oldest_cmd_frame: also false.
        buf.oldest_cmd_frame = 5;
        assert!(!buf.splice_late_input(2, inp));
    }
}

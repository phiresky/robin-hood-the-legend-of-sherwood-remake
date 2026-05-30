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
use crate::sim_timeline::{SimSnapshot as Snapshot, replay_one_frame};

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
    snapshots: VecDeque<Snapshot>,
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
            snapshots: VecDeque::new(),
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
        if frame.is_multiple_of(SNAPSHOT_INTERVAL) {
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
            self.snapshots.push_back(snap);
            self.prune_exponential();
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
        if let Some(oldest) = self.snapshots.front().map(|s| s.frame) {
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
        let snap = self
            .snapshots
            .iter()
            .rev()
            .find(|s| s.frame <= target_frame)?;
        let mut snapshot = snap.clone();
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
        self.snapshots.front().map(|s| s.frame)
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
        while let Some(snap) = self.snapshots.back() {
            if snap.frame > frame {
                self.snapshots.pop_back();
            } else {
                break;
            }
        }
    }

    /// Drop snapshots so the retained set is exponentially spaced:
    /// newest is kept, and for each older snapshot we keep at most
    /// one per `bucket_for_age` bucket.
    ///
    /// Target distances-from-newest: roughly 25, 50, 100, 200, 400, …
    /// frames back.  When two snapshots land in the same bucket we
    /// keep the OLDER one — otherwise every new snapshot would push
    /// the whole retained window forward, and the oldest reachable
    /// frame would never grow beyond one bucket span.  Keeping the
    /// older member of each bucket lets snapshots age naturally into
    /// larger buckets until they eventually roll off the oldest edge.
    fn prune_exponential(&mut self) {
        let Some(newest_frame) = self.snapshots.back().map(|s| s.frame) else {
            return;
        };

        // kept[i] corresponds to buckets[i] — parallel Vecs give us
        // O(buckets) lookup without a hashmap allocation.
        let mut kept: Vec<Snapshot> = Vec::with_capacity(self.snapshots.len());
        let mut buckets: Vec<u32> = Vec::with_capacity(self.snapshots.len());

        // Walk newest → oldest.  For each bucket, overwrite the slot
        // with whichever snapshot we see later — we iterate oldest
        // last per bucket, so "later" == "older", which is what we
        // want to retain.
        while let Some(snap) = self.snapshots.pop_back() {
            let age = newest_frame.saturating_sub(snap.frame);
            let bucket = bucket_for_age(age);
            if let Some(pos) = buckets.iter().position(|&b| b == bucket) {
                kept[pos] = snap;
            } else {
                buckets.push(bucket);
                kept.push(snap);
            }
        }
        // kept is newest → oldest; put back in oldest → newest order.
        kept.reverse();
        self.snapshots.extend(kept);
    }
}

/// Map an age (in frames) to an exponential bucket index.  Ages below
/// [`SNAPSHOT_INTERVAL`] all land in bucket 0 (the newest snapshot
/// itself); from there each bucket covers a [`BUCKET_GROWTH`]-times
/// wider range of frames than the previous one.
fn bucket_for_age(age_frames: u32) -> u32 {
    if age_frames < SNAPSHOT_INTERVAL {
        return 0;
    }
    let ratio = age_frames as f32 / SNAPSHOT_INTERVAL as f32;
    // floor(log_BUCKET_GROWTH(ratio)) + 1
    (ratio.ln() / BUCKET_GROWTH.ln()).floor() as u32 + 1
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
    fn bucket_for_age_is_monotonic_and_dense() {
        // Ages below one interval collapse to bucket 0.
        assert_eq!(bucket_for_age(0), 0);
        assert_eq!(bucket_for_age(SNAPSHOT_INTERVAL - 1), 0);
        // Exactly one interval back → bucket 1 (the closest retained
        // snapshot slot).
        assert_eq!(bucket_for_age(SNAPSHOT_INTERVAL), 1);
        // Buckets are non-decreasing in age.
        let mut last = 0;
        for i in 0..=40 {
            let b = bucket_for_age(i * SNAPSHOT_INTERVAL);
            assert!(b >= last, "bucket decreased at i={i}: {last} → {b}");
            last = b;
        }
        // Adjacent ages at the low end should land in distinct
        // buckets (denser than a factor-of-2 scheme).
        assert_ne!(
            bucket_for_age(SNAPSHOT_INTERVAL),
            bucket_for_age(2 * SNAPSHOT_INTERVAL)
        );
        assert_ne!(
            bucket_for_age(2 * SNAPSHOT_INTERVAL),
            bucket_for_age(3 * SNAPSHOT_INTERVAL)
        );
    }

    /// Mirror of [`RewindBuffer::prune_exponential`] operating on bare
    /// frame numbers so the retention policy can be tested without
    /// instantiating real state clones.  Must be kept in sync with
    /// the real impl.
    fn prune_frames(frames: &mut Vec<u32>) {
        let Some(&newest) = frames.last() else { return };
        let mut kept: Vec<u32> = Vec::new();
        let mut buckets: Vec<u32> = Vec::new();
        while let Some(f) = frames.pop() {
            let age = newest.saturating_sub(f);
            let bucket = bucket_for_age(age);
            if let Some(pos) = buckets.iter().position(|&b| b == bucket) {
                kept[pos] = f;
            } else {
                buckets.push(bucket);
                kept.push(f);
            }
        }
        kept.reverse();
        frames.extend(kept);
    }

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

    #[test]
    fn pruning_retains_exponentially_old_history() {
        // Simulate 40 snapshot intervals == 1000 frames == 40 seconds
        // of game time.  After pruning each insert, the oldest
        // retained frame should still reach far back.
        let mut frames: Vec<u32> = Vec::new();
        for i in 0..=40 {
            frames.push(i * SNAPSHOT_INTERVAL);
            prune_frames(&mut frames);
        }
        let newest = *frames.last().unwrap();
        let oldest = *frames.first().unwrap();
        assert_eq!(newest, 40 * SNAPSHOT_INTERVAL);
        // With exponential retention we expect the oldest retained
        // frame to span at least half the full history — a flat
        // "keep newest per bucket" implementation loses everything
        // beyond one bucket span and would fail this assertion.
        assert!(
            newest - oldest >= 20 * SNAPSHOT_INTERVAL,
            "expected oldest to reach >=500 frames back; got {oldest} (newest={newest})"
        );
        // Bucket count is bounded by log_BUCKET_GROWTH(intervals) + 2.
        // At growth 1.3 across 40 intervals that's around 14 buckets
        // in steady state; leave some headroom for off-by-one.
        assert!(
            frames.len() <= 20,
            "too many snapshots retained: {}",
            frames.len()
        );
    }
}

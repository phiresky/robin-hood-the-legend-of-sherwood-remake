//! Shared simulation timeline replay helpers.
//!
//! Rewind, rollback checking, and multiplayer correction all need the
//! same primitive: start from a pre-tick snapshot, apply the recorded
//! commands for each frame, and run deterministic engine ticks until a
//! target pre-tick frame is reconstructed.  Keep that behavior in one
//! place so the remaining callers differ only in retention policy and
//! diagnostics.

use std::collections::VecDeque;
use web_time::Instant;

use crate::Host;
use crate::engine::{DevState, Engine, HostDisplayState, LevelAssets};
use crate::game_operation::GameCode;
use crate::player_command::PlayerInput;

/// Dense recent rollback snapshots retained for multiplayer correction.
/// Two seconds at the fixed 25 Hz sim rate.
pub const RECENT_TIMELINE_HISTORY_FRAMES: usize = 50;

/// Rollback state at the start of `frame`, before that frame's
/// commands or engine tick have run.
///
/// `HostDisplayState` and `DevState` are intentionally excluded: they
/// are host/display or developer overlay state. Replay uses scratch
/// instances while reconstructing deterministic engine state.
#[derive(Clone)]
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

#[derive(Clone, Debug, Default)]
pub struct ReplayTiming {
    pub replayed_frames: u32,
    pub replay_us: u128,
}

/// Dense recent timeline snapshots for short-horizon correction.
pub struct RecentTimelineHistory {
    snapshots: VecDeque<SimSnapshot>,
    capacity: usize,
}

impl RecentTimelineHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            snapshots: VecDeque::with_capacity(capacity + 1),
            capacity,
        }
    }

    pub fn remember(&mut self, snapshot: SimSnapshot) {
        if self
            .snapshots
            .back()
            .is_some_and(|existing| existing.frame == snapshot.frame)
        {
            self.snapshots.pop_back();
        }
        self.snapshots.push_back(snapshot);
        while self.snapshots.len() > self.capacity {
            self.snapshots.pop_front();
        }
    }

    pub fn get(&self, frame: u32) -> Option<SimSnapshot> {
        self.snapshots
            .iter()
            .rev()
            .find(|snapshot| snapshot.frame == frame)
            .cloned()
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

/// Replay `snapshot` forward to `target_frame`.
///
/// `commands_for(frame)` must return the commands that were applied
/// during that frame. Returning `None` aborts replay, because guessing
/// would silently corrupt the reconstructed timeline.
pub fn replay_to_frame<'a>(
    mut snapshot: SimSnapshot,
    assets: &LevelAssets,
    target_frame: u32,
    mut commands_for: impl FnMut(u32) -> Option<&'a [PlayerInput]>,
) -> Option<(SimSnapshot, ReplayTiming)> {
    let start = Instant::now();
    let start_frame = snapshot.frame;
    let mut scratch_host = Host::default();
    let mut scratch_dev = DevState::default();
    let mut scratch_display = HostDisplayState::default();

    while snapshot.frame < target_frame {
        let cmds = commands_for(snapshot.frame)?;
        replay_one_frame(
            &mut snapshot,
            &mut scratch_display,
            assets,
            &mut scratch_host,
            &mut scratch_dev,
            cmds,
        );
    }

    Some((
        snapshot,
        ReplayTiming {
            replayed_frames: target_frame.saturating_sub(start_frame),
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
    snapshot
        .engine
        .apply_commands(display, &mut scratch_host.input, assets, cmds);
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
    snapshot.frame += 1;
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
    mut side_effects: crate::engine::SideEffects,
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

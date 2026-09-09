//! Host effect application around the shared authoritative timeline.

pub use robin_engine::sim_timeline::*;

use crate::host::Host;
use robin_engine::engine::{DevState, Engine, HostDisplayState, LevelAssets};
use robin_engine::game_operation::GameCode;
use robin_engine::player_command::PlayerInput;

/// Run one deterministic engine tick and drain engine-local side effects.
///
/// This is the rollback-safe core of `Game::run_engine_tick`: it does
/// not read or mutate the outer `Game` shell. Live play wraps this to
/// update mission-operation and UI widget state after the engine
/// reports a result.
pub fn run_engine_frame_core(
    host: &mut Host,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
    frame: robin_engine::engine::SimulationFrameInput,
) -> robin_engine::engine::SimulationFrameOutput {
    host.sync_sound_listener();
    let camera_before = engine.director_camera_frame();
    let output = engine
        .advance_frame(assets, frame)
        .unwrap_or_else(|error| panic!("authoritative frame admission failed: {error}"));
    if let Some(failure) = &output.spellforge_abort {
        tracing::error!(
            kind = ?failure.kind,
            invocation = ?failure.invocation,
            traceback = ?failure.traceback,
            message = %failure.message,
            "Spellforge mission aborted; ending the mission and multiplayer session"
        );
    }
    apply_engine_side_effects(
        host,
        display,
        dev,
        output.events.clone().into_side_effects(),
    );
    apply_engine_side_effects(
        host,
        display,
        dev,
        output.post_boundary_events.clone().into_side_effects(),
    );
    if let Some(events) = output.post_initialize_events.clone() {
        apply_engine_side_effects(host, display, dev, events.into_side_effects());
    }
    host.frontend.viewport.advance_director_camera(
        camera_before,
        engine.director_camera_frame(),
        engine.director_camera_view_size(),
    );
    output
}

/// Compatibility helper for tests and setup callers which need one empty,
/// ungated hourglass. Production drivers pass their complete frame input to
/// [`run_engine_frame_core`].
pub fn run_engine_tick_core(
    host: &mut Host,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
) -> GameCode {
    run_engine_frame_core(
        host,
        display,
        assets,
        engine,
        dev,
        robin_engine::engine::SimulationFrameInput::default(),
    )
    .game_code()
}

/// Dispatch the one-shot mission `PostInitialize` hook at the host's
/// post-refresh boundary.
///
/// Live play calls this after the first sound and render passes. Replay
/// has no presentation work; its recorded complete frame explicitly carries
/// the post-initialization gate needed to reconstruct the authoritative state.
pub fn run_post_initialize_stage(
    host: &mut Host,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
    post_commands: &[PlayerInput],
) -> bool {
    run_post_initialize_stage_with_actions(
        host,
        display,
        assets,
        engine,
        dev,
        &[],
        post_commands,
        true,
    )
}

/// Replay already-recorded post-hourglass developer actions before admitting
/// new live RPC work at the same boundary.
pub fn run_post_external_action_stage(
    host: &mut Host,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
    actions: &[robin_engine::engine::ExternalAction],
) {
    run_engine_frame_core(
        host,
        display,
        assets,
        engine,
        dev,
        robin_engine::engine::SimulationFrameInput::no_hourglass()
            .with_post_external_actions(actions.to_vec()),
    );
}

pub fn run_post_initialize_stage_with_actions(
    host: &mut Host,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
    post_external_actions: &[robin_engine::engine::ExternalAction],
    post_commands: &[PlayerInput],
    run_post_initialize: bool,
) -> bool {
    run_engine_frame_core(
        host,
        display,
        assets,
        engine,
        dev,
        robin_engine::engine::SimulationFrameInput::no_hourglass()
            .with_post_external_actions(post_external_actions.to_vec())
            .with_post_commands(
                post_commands
                    .iter()
                    .cloned()
                    .map(robin_engine::engine::SimCommand::from)
                    .collect(),
            )
            .with_post_initialize(run_post_initialize),
    )
    .post_initialize_events
    .is_some()
}

fn apply_engine_side_effects(
    host: &mut Host,
    display: &mut HostDisplayState,
    dev: &mut DevState,
    mut side_effects: robin_engine::engine::SideEffects,
) -> GameCode {
    for event in side_effects.host_events.drain(..) {
        display.apply_host_event(&mut host.frontend.input, event);
    }
    if let Some(top_left) = display.take_pending_minimap_position() {
        side_effects.pending_minimap_position = Some(top_left);
    }
    if side_effects.ui_has_focus {
        host.frontend.input.has_focus = false;
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

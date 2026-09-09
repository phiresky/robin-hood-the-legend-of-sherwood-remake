//! Host effect application around the shared authoritative timeline.

pub use robin_engine::sim_timeline::*;

use crate::host::{ApplicationContext, HostAudio, HostEffectBatches, HostFrontend};
use robin_engine::engine::{DevState, Engine, HostDisplayState, LevelAssets};
use robin_engine::game_operation::GameCode;
use robin_engine::player_command::PlayerInput;

/// Run one deterministic engine tick and drain engine-local side effects.
///
/// This is the rollback-safe core of `Game::run_engine_tick`: it does
/// not read or mutate the outer `Game` shell. Live play wraps this to
/// update mission-operation and UI widget state after the engine
/// reports a result. Presentation domains are borrowed disjointly: this boundary
/// cannot replace transport, process services, or a snapshot manager, and the
/// frontend's display remains installed while effects are applied.
pub fn run_engine_frame_core(
    frontend: &mut HostFrontend,
    audio: &mut HostAudio,
    effects: &mut HostEffectBatches,
    application_context: &ApplicationContext,
    local_seat: robin_engine::player_command::PlayerId,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
    frame: robin_engine::engine::SimulationFrameInput,
) -> robin_engine::engine::SimulationFrameOutput {
    audio.sound.set_listen_point(
        frontend.viewport.sound_listen_point(),
        frontend.viewport.zoom_factor,
    );
    let camera_before = engine.director_camera_frame();
    let output = execute_frame(engine, assets, frame);
    // Even empty batches advance display lifetimes. Preserve pre-hourglass,
    // post-boundary, then optional PostInitialize delivery exactly once, before
    // advancing the host camera.
    for events in [
        Some(output.events.clone()),
        Some(output.post_boundary_events.clone()),
        output.post_initialize_events.clone(),
    ]
    .into_iter()
    .flatten()
    {
        let side_effects = prepare_display_effects(
            &mut frontend.engine_display,
            &mut frontend.input,
            dev,
            events.into_side_effects(),
        );
        frontend.apply_side_effects(
            side_effects,
            audio,
            effects,
            application_context,
            local_seat,
        );
    }
    frontend.viewport.advance_director_camera(
        camera_before,
        engine.director_camera_frame(),
        engine.director_camera_view_size(),
    );
    output
}

/// The deterministic boundary has no host, display, audio or transport access.
fn execute_frame(
    engine: &mut Engine,
    assets: &LevelAssets,
    frame: robin_engine::engine::SimulationFrameInput,
) -> robin_engine::engine::SimulationFrameOutput {
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
    output
}

/// Compatibility helper for tests and setup callers which need one empty,
/// ungated hourglass. Production drivers pass their complete frame input to
/// [`run_engine_frame_core`].
pub fn run_engine_tick_core(
    frontend: &mut HostFrontend,
    audio: &mut HostAudio,
    effects: &mut HostEffectBatches,
    application_context: &ApplicationContext,
    local_seat: robin_engine::player_command::PlayerId,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
) -> GameCode {
    run_engine_frame_core(
        frontend,
        audio,
        effects,
        application_context,
        local_seat,
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
    frontend: &mut HostFrontend,
    audio: &mut HostAudio,
    effects: &mut HostEffectBatches,
    application_context: &ApplicationContext,
    local_seat: robin_engine::player_command::PlayerId,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
    post_commands: &[PlayerInput],
) -> bool {
    run_post_initialize_stage_with_actions(
        frontend,
        audio,
        effects,
        application_context,
        local_seat,
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
    frontend: &mut HostFrontend,
    audio: &mut HostAudio,
    effects: &mut HostEffectBatches,
    application_context: &ApplicationContext,
    local_seat: robin_engine::player_command::PlayerId,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
    actions: &[robin_engine::engine::ExternalAction],
) {
    run_engine_frame_core(
        frontend,
        audio,
        effects,
        application_context,
        local_seat,
        assets,
        engine,
        dev,
        robin_engine::engine::SimulationFrameInput::no_hourglass()
            .with_post_external_actions(actions.to_vec()),
    );
}

pub fn run_post_initialize_stage_with_actions(
    frontend: &mut HostFrontend,
    audio: &mut HostAudio,
    effects: &mut HostEffectBatches,
    application_context: &ApplicationContext,
    local_seat: robin_engine::player_command::PlayerId,
    assets: &LevelAssets,
    engine: &mut Engine,
    dev: &mut DevState,
    post_external_actions: &[robin_engine::engine::ExternalAction],
    post_commands: &[PlayerInput],
    run_post_initialize: bool,
) -> bool {
    run_engine_frame_core(
        frontend,
        audio,
        effects,
        application_context,
        local_seat,
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

fn prepare_display_effects(
    display: &mut HostDisplayState,
    input: &mut robin_engine::engine::InputState,
    dev: &mut DevState,
    mut side_effects: robin_engine::engine::SideEffects,
) -> robin_engine::engine::SideEffects {
    for event in side_effects.host_events.drain(..) {
        display.apply_host_event(input, event);
    }
    if let Some(top_left) = display.take_pending_minimap_position() {
        side_effects.pending_minimap_position = Some(top_left);
    }
    if side_effects.ui_has_focus {
        input.controls.has_focus = false;
    }
    for noise in side_effects.displayed_noises.drain(..) {
        dev.add_noise_to_display(noise);
    }
    dev.tick_noise_display(1.0);
    for (show, restore_position) in side_effects.pending_minimap_display_maps.drain(..) {
        display.display_minimap(show, restore_position);
    }
    side_effects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_execution_has_only_disjoint_effect_authority() {
        let _: fn(
            &mut HostFrontend,
            &mut HostAudio,
            &mut HostEffectBatches,
            &ApplicationContext,
            robin_engine::player_command::PlayerId,
            &LevelAssets,
            &mut Engine,
            &mut DevState,
            robin_engine::engine::SimulationFrameInput,
        ) -> robin_engine::engine::SimulationFrameOutput = run_engine_frame_core;
    }

    #[test]
    fn live_and_post_action_boundaries_deliver_empty_batches_exactly_once() {
        let mut assets = LevelAssets::new();
        let mut engine = Engine::new_for_test_with_simulation(
            800.0,
            600.0,
            robin_engine::campaign::Campaign::default(),
            &mut assets,
            42,
            robin_engine::engine::SimConfig {
                script_enabled: false,
                ..Default::default()
            },
        )
        .expect("effect-boundary fixture");
        let mut host = crate::host::Host::scratch(800.0, 600.0);
        let application_context = host.application_context().clone();
        let mut dev = DevState::default();
        let output = run_engine_frame_core(
            &mut host.frontend,
            &mut host.audio,
            &mut host.effects,
            &application_context,
            host.transport.local_seat(),
            &assets,
            &mut engine,
            &mut dev,
            robin_engine::engine::SimulationFrameInput::no_hourglass(),
        );
        assert!(!output.hourglass_ran);
        assert!(output.post_initialize_events.is_none());
        assert_eq!(
            dev.noise_display_start_radius, 14,
            "two empty batches still advance display lifetime"
        );
        run_post_external_action_stage(
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
        assert_eq!(
            dev.noise_display_start_radius, 8,
            "post-action handling uses the same two-batch boundary, modulo twenty"
        );
        assert_eq!(engine.frame_counter(), 0);
    }

    #[test]
    fn deterministic_execution_has_no_host_effect_authority() {
        let _: fn(
            &mut Engine,
            &LevelAssets,
            robin_engine::engine::SimulationFrameInput,
        ) -> robin_engine::engine::SimulationFrameOutput = execute_frame;
    }

    #[test]
    fn display_preparation_consumes_focus_and_preserves_queued_host_effects() {
        let mut display = HostDisplayState::default();
        let mut input = robin_engine::engine::InputState::default();
        input.controls.has_focus = true;
        let mut dev = DevState::default();
        let prepared = prepare_display_effects(
            &mut display,
            &mut input,
            &mut dev,
            robin_engine::engine::SideEffects {
                ui_has_focus: true,
                pending_show_console: true,
                ..Default::default()
            },
        );
        assert!(!input.controls.has_focus);
        assert!(prepared.pending_show_console);
        assert!(prepared.ui_has_focus);
        assert_eq!(dev.noise_display_start_radius, 7);
    }
}

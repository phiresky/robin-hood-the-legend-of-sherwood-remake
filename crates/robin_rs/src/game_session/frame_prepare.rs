//! Interactive frame input and operation preparation.
//!
//! This phase owns the exclusive mission and application-service borrows until
//! it has finalized the deterministic command stream. No presentation borrow
//! escapes the phase or crosses into simulation.

use super::flow::{FrameControl, MissionExit, MissionServices};
use super::interactive::MissionInput;
use super::runtime::FrameContractStage;
use super::*;
use crate::game::Game;

/// Values produced by graphical network ingress at the frame boundary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FrameStart {
    pub(super) frame: MissionFrame,
    pub(super) mp_clock_pause: bool,
}

/// State handed from modal/recorder bookkeeping to presentation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FramePresentationState {
    pub(super) frame: MissionFrame,
    pub(super) rewind_active: bool,
    pub(super) consumed_buffered: bool,
    pub(super) shift_held: bool,
    pub(super) modal_rendered: bool,
}

/// Deterministic and presentation flags carried across the tick boundary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PreparedFrame {
    pub(super) frame: MissionFrame,
    pub(super) rewind_active: bool,
    pub(super) paused: bool,
    pub(super) consumed_buffered: bool,
    pub(super) shift_held: bool,
    pub(super) modal_rendered: bool,
    pub(super) step_forward_pressed: bool,
    pub(super) step_back_pressed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum FramePreparation {
    Ready(PreparedFrame),
    Control(FrameControl),
}

/// Apply multiplayer ingress and capture the deterministic pre-command
/// snapshot before any interactive input mutates the engine.
fn begin_interactive_frame(mission: &mut InteractiveMission) -> FrameStart {
    let InteractiveMission { runtime, frontend } = mission;
    let MissionRuntime {
        world,
        timeline: runtime,
        control,
    } = runtime;
    let MissionWorld {
        host,
        manager,
        assets,
        ..
    } = world;
    let manual_pause = &mut control.manual_pause;
    let hud = &mut frontend.hud;
    let presentation = &mut frontend.presentation;
    let mut frame = MissionFrame::new(crate::window::process_uptime_ms());
    runtime.begin_execution_trace(FrameContractStage::NetworkIngress);
    if let Some(start_at) = runtime.mp_start_gate {
        if current_epoch_ms() >= start_at {
            runtime.mp_start_gate = None;
            if !runtime.start_paused {
                *manual_pause = false;
            }
            tracing::info!("multiplayer: synchronized lobby start gate opened");
        } else {
            *manual_pause = true;
        }
    }

    // ── Multiplayer: drain incoming wire events ───────────────
    // - Future inputs queue in `pending_inputs[target_frame]`.
    // - Late inputs (target < sim_frame) splice into the rewind
    //   buffer and trigger a rollback to reconstruct the engine
    //   state with the late input woven in.  `drain_net_inputs`
    //   replaces the live rollback state when that fires.
    // - Inputs scheduled for `sim_frame` come back in the return
    //   value; we apply them and append to `frame.commands`.
    // - Authoritative state hashes from the host land in
    //   `runtime.peer_hashes`, drained below alongside the per-25-frame
    //   sampling tick.
    // Publishes the current sim_frame to the server's broadcast
    // pump so peer-input target frames are stamped against a
    // fresh cursor.
    if let Some(net) = host.net.as_ref() {
        net.publish_frame(manager.sim_frame);
    }
    let net_drain = drain_net_inputs(
        host,
        manager,
        assets.as_ref(),
        &mut runtime.rewind_buffer,
        &mut runtime.peer_hashes,
        &mut runtime.recent_timeline_history,
    );
    if net_drain.rewrote_sim_state
        && let Some(ref mut checker) = runtime.rollback_checker
    {
        checker.reset();
    }
    if let Some(rollback) = net_drain.rollback.clone() {
        runtime.last_mp_rollback = Some(rollback);
    }
    if let Some((_frame, start_epoch_ms)) = net_drain.begin_sim {
        runtime.mp_waiting_for_begin_sim = false;
        runtime.mp_start_gate = Some(start_epoch_ms);
        *manual_pause = true;
    }
    if runtime.mp_waiting_for_initial_snapshot && net_drain.received_initial_snapshot {
        runtime.mp_waiting_for_initial_snapshot = false;
        tracing::info!("multiplayer: initial snapshot received; client ready for start barrier");
    }
    if runtime.mp_waiting_for_initial_snapshot || runtime.mp_waiting_for_begin_sim {
        *manual_pause = true;
    }
    if host.net.is_some()
        && host.local_seat != engine_player_command::PlayerId::HOST
        && let Some((clock_frame, ms_until_next_frame)) = net_drain.latest_host_clock_sample
    {
        accept_host_frame_schedule(
            &mut runtime.mp_host_frame_schedule,
            clock_frame,
            ms_until_next_frame,
            manager.sim_frame,
        );
    }
    let mut mp_clock_pause = false;
    if host.net.is_some()
        && host.local_seat != engine_player_command::PlayerId::HOST
        && !runtime.mp_waiting_for_initial_snapshot
        && !runtime.mp_waiting_for_begin_sim
        && runtime.mp_start_gate.is_none()
    {
        if let Some(deadline_ms) =
            host_scheduled_frame_deadline_ms(runtime.mp_host_frame_schedule, manager.sim_frame)
        {
            let now_ms = crate::window::process_uptime_ms();
            let until_frame_ms = deadline_ms - i64::from(now_ms);
            if until_frame_ms > 0 {
                mp_clock_pause = true;
                if now_ms.saturating_sub(runtime.last_mp_clock_ahead_log_ms) >= 1000 {
                    runtime.last_mp_clock_ahead_log_ms = now_ms;
                    tracing::info!(
                        scheduled_frame = runtime.mp_host_frame_schedule.map(|(frame, _)| frame),
                        local_frame = manager.sim_frame,
                        until_frame_ms,
                        "multiplayer: local frame is ahead of host schedule; holding sim"
                    );
                }
            }
        } else {
            mp_clock_pause = true;
        }
    }
    let net_inputs = net_drain.inputs;
    if host.net.is_some() {
        runtime
            .recent_timeline_history
            .checkpoint(manager.sim_frame, &manager.engine);
    }
    if !net_inputs.is_empty() {
        manager.engine.apply_commands(
            &mut host.engine_display,
            &mut host.input,
            &assets,
            &net_inputs,
        );
        for inp in net_inputs {
            frame.commands.commands.push(inp);
        }
    }

    // Re-derive the corner HUD layout every frame so resolution
    // changes triggered from nested menus (options modal, Sherwood
    // flow, etc.) take effect without needing every call site to
    // plumb a mutable layout ref.  Cheap — just a few rect
    // arithmetic operations.
    hud.corner_layout = CornerHudLayout::for_resolution(
        presentation.renderer.screen_width() as u32,
        presentation.renderer.screen_height() as u32,
        &hud.corner_sprites,
    );
    hud.stature_layout = StatureHudLayout::for_resolution(
        presentation.renderer.screen_width() as u32,
        presentation.renderer.screen_height() as u32,
        &hud.stature_sprites,
    );

    // Refresh the host-cached back-to-front entity draw order from
    // the current engine state.  Consumed by this frame's input
    // handlers (hit-test via `find_focusable_entity`), render loop,
    // and titbit Z flush. This is the interactive-only driver; true
    // headless construction never reaches this presentation stage.
    host.draw_order = manager.engine.compute_display_order();

    // Enter the shared runtime's input phase. This captures the
    // rollback/rewind snapshots and replay state hash at the "start of
    // frame N — after N-1's tick, before N's commands" point,
    // before any event-loop handler (Resized → inline
    // `MinimapResize`, live input → local viewport edits, …)
    // mutates the engine.  The recorder write and the player
    // check MUST sample here in lockstep; if the recording
    // captures post-input state while the replay checks
    // pre-input state, every hash-carrying frame spuriously
    // desyncs.  The actual write is deferred to the recorder
    // block further down so the existing
    // `!rewind_active && !consumed_buffered` gating stays in
    // one place.
    runtime.open_frame(&mut frame, manager.sim_frame, &manager.engine, &assets);
    FrameStart {
        frame,
        mp_clock_pause,
    }
}

/// Apply process-side input and banner effects produced by save/load I/O.
/// These intentionally remain after operation processing and before replay or
/// multiplayer command injection.
fn apply_post_save_ui_state(
    callbacks: &mut RustCallbacks,
    game: &mut crate::game::Game,
    input: &mut MissionInput,
) {
    if std::mem::take(&mut callbacks.pending_reset_input) {
        input.reset_after_engine_request();
    }
    if let Some(kind) = callbacks.pending_save_banner.take() {
        let text = match kind {
            SaveBannerKind::Saved => "Game saved.",
            SaveBannerKind::Loaded => "Game loaded.",
        };
        // TODO(refactor): replace these literals with MT_MSG_GAME_SAVED and
        // MT_MSG_GAME_LOADED once the localized text ownership is explicit.
        game.display_message(text.to_string(), 100);
    }
}

/// Physical step-key state admitted by the input phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct StepShortcutInput {
    now_ms: u32,
    forward_held: bool,
    forward_hit: bool,
    back_held: bool,
    back_hit: bool,
    backspace_held: bool,
    unpause_hit: bool,
    gated: bool,
}

/// Step commands and manual-pause update produced from one key sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct StepShortcutOutput {
    forward: bool,
    back: bool,
    manual_pause: Option<bool>,
}

/// Apply the debug-step edge/repeat policy without borrowing mission state.
fn plan_step_shortcuts(
    input: StepShortcutInput,
    forward_repeat_at_ms: &mut Option<u32>,
    back_repeat_at_ms: &mut Option<u32>,
) -> StepShortcutOutput {
    const INITIAL_DELAY_MS: u32 = 160;
    const REPEAT_INTERVAL_MS: u32 = 40;

    let repeat = |held: bool, hit: bool, repeat_at_ms: &mut Option<u32>| -> bool {
        if !held {
            *repeat_at_ms = None;
            return false;
        }
        if hit {
            *repeat_at_ms = Some(input.now_ms.saturating_add(INITIAL_DELAY_MS));
            return true;
        }
        if let Some(next_ms) = *repeat_at_ms
            && input.now_ms >= next_ms
        {
            *repeat_at_ms = Some(input.now_ms.saturating_add(REPEAT_INTERVAL_MS));
            return true;
        }
        false
    };

    let forward = repeat(input.forward_held, input.forward_hit, forward_repeat_at_ms);
    let back = repeat(input.back_held, input.back_hit, back_repeat_at_ms) || input.backspace_held;
    if input.gated {
        return StepShortcutOutput {
            forward: false,
            back: false,
            manual_pause: None,
        };
    }
    let manual_pause = if input.unpause_hit {
        Some(false)
    } else if forward || back {
        Some(true)
    } else {
        None
    };
    StepShortcutOutput {
        forward,
        back,
        manual_pause,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct InputModifiers {
    ctrl: bool,
    shift: bool,
    alt: bool,
}

fn input_modifiers(keys: &std::collections::BTreeSet<winit::keyboard::KeyCode>) -> InputModifiers {
    use winit::keyboard::KeyCode;
    InputModifiers {
        ctrl: keys.contains(&KeyCode::ControlLeft) || keys.contains(&KeyCode::ControlRight),
        shift: keys.contains(&KeyCode::ShiftLeft) || keys.contains(&KeyCode::ShiftRight),
        alt: keys.contains(&KeyCode::AltLeft) || keys.contains(&KeyCode::AltRight),
    }
}

/// Apply only logical resolution changes. Ordinary WM resizes reconfigure the
/// swapchain but deliberately leave the fixed logical game resolution alone.
fn apply_frame_resizes(
    events: &[GameEvent],
    window: &mut GameWindow,
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    input: &mut MissionInput,
    hud: &mut super::interactive::MissionHud,
    presentation: &mut super::interactive::MissionPresentation,
    frame: &mut MissionFrame,
) {
    for event in events {
        let GameEvent::Resized(new_w, new_h) = *event else {
            continue;
        };
        presentation.renderer.configure_surface_size(new_w, new_h);
        if !matches!((new_w, new_h), (640, 480) | (800, 600) | (1024, 768)) {
            continue;
        }
        let w = new_w as f32;
        let h = new_h as f32;
        window.set_logical_size(new_w, new_h);
        host.viewport.set_screen_size(w, h);
        presentation.renderer.resize(new_w as u16, new_h as u16);
        input.resize(new_w, new_h, &host.key_config);
        if host.minimap_corner_size.x > 0.0 {
            let cmd = PlayerCommand::MinimapResize {
                base: engine_coordinates::ScreenPoint::new(w - 83.0, 38.0),
                corner_size: host.minimap_corner_size,
            };
            dispatch_local_command(host, &mut manager.engine, &mut frame.commands, assets, &cmd);
        }
        hud.resize(new_w, new_h);
    }
}

/// Apply host-only camera controls. These deliberately remain available while
/// deterministic replay or rewind suppresses simulation commands.
fn apply_host_view_input(
    host: &mut Host,
    engine: &Engine,
    mouse_position: engine_coordinates::ScreenPoint,
    keyboard_actions: &[GameAction],
    mouse_actions: &[GameAction],
    events: &[GameEvent],
    view_suppressed: bool,
    pan_suppressed: bool,
) {
    if !view_suppressed {
        for action in keyboard_actions.iter().chain(mouse_actions) {
            let scroll_suppressed_by_minimap = matches!(
                action,
                GameAction::ScrollUp
                    | GameAction::ScrollDown
                    | GameAction::ScrollLeft
                    | GameAction::ScrollRight
            ) && host.engine_display.minimap().drag_start();
            if scroll_suppressed_by_minimap {
                continue;
            }
            match action {
                GameAction::ScrollUp => apply_local_viewport_scroll(host, ScrollDirection::Up),
                GameAction::ScrollDown => apply_local_viewport_scroll(host, ScrollDirection::Down),
                GameAction::ScrollLeft => apply_local_viewport_scroll(host, ScrollDirection::Left),
                GameAction::ScrollRight => {
                    apply_local_viewport_scroll(host, ScrollDirection::Right)
                }
                GameAction::ZoomIn => host.viewport.zoom_by(2.0, Some(mouse_position)),
                GameAction::ZoomOut => host.viewport.zoom_by(0.5, Some(mouse_position)),
                _ => {}
            }
        }
    }

    if pan_suppressed || engine.user_locked() {
        return;
    }
    for event in events {
        if let GameEvent::ViewportPan { xrel, yrel } = *event {
            host.viewport
                .scroll_by(robin_engine::coordinates::ScreenVec::new(
                    -(xrel as f32),
                    -(yrel as f32),
                ));
            host.input.cancel_multi_selection();
        }
    }
}

/// Drain packets which arrived while the frame was processing local input.
/// This is the final network mutation boundary before state hashing and tick.
fn drain_pre_tick_network(
    runtime: &mut super::runtime::TimelineRuntime,
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    frame: &mut MissionFrame,
    manual_pause: &mut bool,
    mp_clock_pause: &mut bool,
    rewind_active: bool,
) {
    if host.net.is_none() || rewind_active {
        return;
    }

    runtime.trace(FrameContractStage::SecondNetworkDrain);
    if let Some(net) = host.net.as_ref() {
        net.publish_frame(manager.sim_frame);
    }
    let drain = drain_net_inputs(
        host,
        manager,
        assets,
        &mut runtime.rewind_buffer,
        &mut runtime.peer_hashes,
        &mut runtime.recent_timeline_history,
    );
    if drain.rewrote_sim_state
        && let Some(ref mut checker) = runtime.rollback_checker
    {
        checker.reset();
    }
    if let Some(rollback) = drain.rollback.clone() {
        runtime.last_mp_rollback = Some(rollback);
    }
    if let Some((_frame, start_epoch_ms)) = drain.begin_sim {
        runtime.mp_waiting_for_begin_sim = false;
        runtime.mp_start_gate = Some(start_epoch_ms);
        *manual_pause = true;
    }
    if runtime.mp_waiting_for_initial_snapshot && drain.received_initial_snapshot {
        runtime.mp_waiting_for_initial_snapshot = false;
        tracing::info!("multiplayer: initial snapshot received; client ready for start barrier");
    }
    if runtime.mp_waiting_for_initial_snapshot || runtime.mp_waiting_for_begin_sim {
        *manual_pause = true;
    }
    if host.net.is_some()
        && host.local_seat != engine_player_command::PlayerId::HOST
        && let Some((clock_frame, ms_until_next_frame)) = drain.latest_host_clock_sample
    {
        accept_host_frame_schedule(
            &mut runtime.mp_host_frame_schedule,
            clock_frame,
            ms_until_next_frame,
            manager.sim_frame,
        );
    }
    if host.net.is_some()
        && host.local_seat != engine_player_command::PlayerId::HOST
        && !runtime.mp_waiting_for_initial_snapshot
        && !runtime.mp_waiting_for_begin_sim
        && runtime.mp_start_gate.is_none()
    {
        if let Some(deadline_ms) =
            host_scheduled_frame_deadline_ms(runtime.mp_host_frame_schedule, manager.sim_frame)
        {
            let now_ms = crate::window::process_uptime_ms();
            let until_frame_ms = deadline_ms - i64::from(now_ms);
            if until_frame_ms > 0 {
                *mp_clock_pause = true;
                if now_ms.saturating_sub(runtime.last_mp_clock_ahead_log_ms) >= 1000 {
                    runtime.last_mp_clock_ahead_log_ms = now_ms;
                    tracing::info!(
                        scheduled_frame = runtime.mp_host_frame_schedule.map(|(frame, _)| frame),
                        local_frame = manager.sim_frame,
                        until_frame_ms,
                        "multiplayer: local frame is ahead of host schedule; holding sim"
                    );
                }
            }
        } else {
            *mp_clock_pause = true;
        }
    }
    if drain.rewrote_sim_state && host.net.is_some() {
        runtime
            .recent_timeline_history
            .checkpoint(manager.sim_frame, &manager.engine);
    }
    if !drain.inputs.is_empty() {
        manager.engine.apply_commands(
            &mut host.engine_display,
            &mut host.input,
            assets,
            &drain.inputs,
        );
        frame.commands.commands.extend(drain.inputs);
    }
}

/// Publish or verify the periodic multiplayer state hash after the second
/// network drain has made this frame's command set final.
fn process_pre_tick_state_hash(
    runtime: &mut super::runtime::TimelineRuntime,
    host: &Host,
    manager: &robin_engine::engine_manager::EngineManager,
) {
    if host.net.is_none()
        || !manager
            .sim_frame
            .is_multiple_of(crate::multiplayer::STATE_HASH_INTERVAL)
    {
        return;
    }
    if host.local_seat == engine_player_command::PlayerId::HOST
        && runtime.last_mp_state_hash_frame != Some(manager.sim_frame)
    {
        runtime.last_mp_state_hash_frame = Some(manager.sim_frame);
        let mp_hash_start = web_time::Instant::now();
        let live_hash_start = web_time::Instant::now();
        let local_hash = crate::replay::state_hash(&manager.engine);
        let live_hash_us = live_hash_start.elapsed().as_micros();
        runtime.pending_mp_state_hash = Some((manager.sim_frame, local_hash));
        tracing::debug!(
            frame = manager.sim_frame,
            total_us = mp_hash_start.elapsed().as_micros(),
            live_hash_us,
            "multiplayer hash frame timing"
        );
    } else if let Some(&host_hash) = runtime.peer_hashes.get(&manager.sim_frame) {
        let local_hash = crate::replay::state_hash(&manager.engine);
        if local_hash != host_hash {
            let last_rollback_path = runtime.last_mp_rollback.as_ref().map_or("none", |r| r.path);
            let last_rollback_earliest = runtime
                .last_mp_rollback
                .as_ref()
                .map_or(0, |r| r.earliest_frame);
            let last_rollback_target = runtime
                .last_mp_rollback
                .as_ref()
                .map_or(0, |r| r.target_frame);
            let last_rollback_replayed = runtime
                .last_mp_rollback
                .as_ref()
                .map_or(0, |r| r.replayed_frames);
            let last_rollback_total_us =
                runtime.last_mp_rollback.as_ref().map_or(0, |r| r.total_us);
            tracing::warn!(
                frame = manager.sim_frame,
                local = format!("{local_hash:016x}"),
                host = format!("{host_hash:016x}"),
                host_schedule_frame = runtime.mp_host_frame_schedule.map(|(frame, _)| frame),
                pending_input_frames = manager.pending_inputs.len(),
                last_rollback_path,
                last_rollback_earliest,
                last_rollback_target,
                last_rollback_replayed,
                last_rollback_total_us,
                "multiplayer DESYNC: local engine hash differs from host's"
            );
        } else {
            tracing::debug!(frame = manager.sim_frame, "multiplayer hash OK");
        }
    }
    runtime.peer_hashes.retain(|&f, _| f > manager.sim_frame);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PreTickPauseSources {
    pause_menu: bool,
    manual: bool,
    multiplayer_clock: bool,
    modal: bool,
}

fn pre_tick_is_paused(sources: PreTickPauseSources) -> bool {
    sources.pause_menu || sources.manual || sources.multiplayer_clock || sources.modal
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PreTickTimelineOutput {
    paused: bool,
    consumed_buffered: bool,
}

/// Admit replay commands and reconcile rewind history after all live/network
/// commands for the frame are known.
fn prepare_pre_tick_timeline(
    runtime: &mut super::runtime::TimelineRuntime,
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    frame: &mut MissionFrame,
    manual_pause: &mut bool,
    rewind_active: bool,
    mut paused: bool,
) -> Result<PreTickTimelineOutput, String> {
    if let Some(ref mut player) = runtime.replay_player
        && !paused
    {
        if player.is_finished() {
            if !runtime.replay_finished_logged {
                tracing::info!("Replay finished after {} frames", player.current_frame());
                runtime.replay_finished_logged = true;
            }
            *manual_pause = true;
            paused = true;
        } else {
            runtime.replay_finished_logged = false;
            frame.inject_replay_commands(player, host, manager, assets);
        }
    }

    let mut consumed_buffered = false;
    if !rewind_active && !paused && manager.sim_frame < runtime.rewind_buffer.next_record_frame() {
        let Some(recorded) = runtime.rewind_buffer.commands_for(manager.sim_frame) else {
            return Err(format!(
                "cannot replay frame {}: rewind command history starts at frame {}",
                manager.sim_frame,
                runtime.rewind_buffer.oldest_cmd_frame()
            ));
        };
        if runtime.replay_player.is_some() {
            consumed_buffered = true;
            tracing::trace!("Replay reused rewind-buffer frame {}", manager.sim_frame);
        } else if frame.commands.commands.is_empty() {
            let recorded: Vec<PlayerInput> = recorded.to_vec();
            manager.engine.apply_commands(
                &mut host.engine_display,
                &mut host.input,
                assets,
                &recorded,
            );
            frame.commands.commands = recorded;
            consumed_buffered = true;
            tracing::trace!("Auto-replay -> frame {}", manager.sim_frame);
        } else {
            tracing::trace!(
                "Auto-replay interrupted by live input; truncating buffer at {}",
                manager.sim_frame
            );
            runtime.rewind_buffer.truncate_future(manager.sim_frame);
        }
    }
    Ok(PreTickTimelineOutput {
        paused,
        consumed_buffered,
    })
}

/// Emit pointer-derived simulation commands only after replay/rewind/pause
/// admission is final, so they enter the same deterministic frame log.
fn dispatch_pre_tick_pointer_commands(
    runtime: &super::runtime::TimelineRuntime,
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    input: &MissionInput,
    frame: &mut MissionFrame,
    rewind_active: bool,
    paused: bool,
) {
    if runtime.replay_player.is_some() || rewind_active || paused {
        return;
    }
    let Some(mouse_map) = host.viewport.screen_to_map(input.threaded.position()) else {
        return;
    };

    if manager.engine.locker_active()
        && let Some(id) =
            manager
                .engine
                .find_focusable_npc(assets, mouse_map, engine_element::Focus::View)
    {
        let cmd = PlayerCommand::SelectFollowElement {
            entity_id: Some(id),
        };
        dispatch_local_command(host, &mut manager.engine, &mut frame.commands, assets, &cmd);
    }

    let bow_armed =
        manager.engine.selected_action_for_seat(host.local_seat) == engine_profiles::Action::Bow;
    if host.time_no_mouse_move != 0 || bow_armed {
        let cmd = PlayerCommand::PerformOrientation { mouse_map };
        dispatch_local_command(host, &mut manager.engine, &mut frame.commands, assets, &cmd);
    }
}

/// Dispatch simulation-affecting keyboard, pause-menu, and mouse input. The
/// caller admits this phase only when replay and rewind are inactive.
#[allow(clippy::too_many_arguments)]
async fn drive_live_gameplay_input(
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    game: &mut Game,
    assets: &robin_engine::engine::LevelAssets,
    dev: &mut robin_engine::engine::DevState,
    callbacks: &mut RustCallbacks,
    window: &mut GameWindow,
    presentation: &mut super::interactive::MissionPresentation,
    resources: &mut super::interactive::MissionResources,
    audio: &mut super::interactive::MissionAudio,
    input: &mut MissionInput,
    ui: &mut super::interactive::MissionUi,
    hud: &mut super::interactive::MissionHud,
    frame: &mut MissionFrame,
    events: &[GameEvent],
    keyboard_actions: &[GameAction],
    mouse_actions: &[GameAction],
    minimap_toggle_pressed: bool,
    modifiers: InputModifiers,
    pause_closed_this_frame: &mut bool,
) -> HandlerAction {
    let InputModifiers {
        ctrl: ctrl_held,
        shift: shift_held,
        alt: _,
    } = modifiers;
    let kb_actions = keyboard_actions;
    // Minimap accelerator key.
    // Suppressed while the console or pause menu has focus so the
    // toggle can't fire underneath modal UI.
    if minimap_toggle_pressed && !ui.console_overlay.is_visible() && ui.pause_menu.is_none() {
        let cmd = PlayerCommand::MinimapToggle;
        dispatch_local_command(
            host,
            &mut manager.engine,
            &mut frame.commands,
            &assets,
            &cmd,
        );
    }

    for action in kb_actions.iter().chain(mouse_actions.iter()) {
        // Console captures every other action while it has focus.
        if ui.console_overlay.is_visible() {
            continue;
        }
        match action {
            GameAction::DisplayConsole => {
                // Already handled above — swallow so we don't
                // hit the catch-all below.
            }
            GameAction::DisplayInfo => {
                // Toggle the host flag — the per-frame debug
                // overlay presentation.renderer polls `host.info_displayed`
                // to decide whether to draw FPS / mission
                // clock / music-mode bars.
                host.info_displayed = !host.info_displayed;
                tracing::debug!("DisplayInfo toggled: {}", host.info_displayed);
            }
            GameAction::DisplayMenu => {
                if ui.pause_menu.is_some() {
                    debug_assert!(ui.close_pause(input, presentation));
                    *pause_closed_this_frame = true;
                    callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                    // Resume play-time recording after the
                    // modal closes.
                    callbacks.start_play_time();
                } else {
                    // Suspend play-time recording before
                    // opening the modal so `MissionLength`
                    // doesn't count wall-clock spent in the
                    // pause menu.
                    callbacks.suspend_play_time();
                    if let Some(resources) = resources.menu.as_ref() {
                        ui.pause_menu = Some(PauseMenu::new(resources, ui.restart_allowed));
                    } else {
                        // Retry the resource load in case a transient presentation.renderer state
                        // prevented mission-start initialization. A pause menu still
                        // requires the real resources after this retry.
                        let fallback = IngameMenuResources::new(
                            &mut presentation.renderer,
                            host.shipping.as_deref(),
                        );
                        let res = required_menu_resources(
                            &fallback,
                            "opening the pause menu after resource reload",
                        );
                        ui.pause_menu = Some(PauseMenu::new(res, ui.restart_allowed));
                        resources.menu = fallback;
                    }
                    if ui.pause_menu.is_some() {
                        // Freeze the current screen so the
                        // pause-menu backdrop composites over
                        // a still frame instead of the live
                        // engine output.  Idempotent; the
                        // symmetric close-branch above calls
                        // `clear_frozen_scene`.
                        presentation.renderer.freeze_scene_for_modal();
                        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Menu));
                    }
                }
            }
            _ if ui.pause_menu.is_some() || *pause_closed_this_frame => {
                // Skip all other game actions while paused
                // and for the remainder of the frame if pause
                // was toggled off this frame, so actions
                // queued during pause don't fire the instant
                // the game resumes.
            }
            _ => {
                match action {
                    GameAction::SlowMotion => {
                        // Toggle the slow-motion pacing flag.
                        // The frame-pacing block multiplies
                        // the 40 ms frame target by 10 when
                        // set.  Pure host-side, not sim state.
                        host.slow_motion = !host.slow_motion;
                    }
                    GameAction::SwitchMaskedDisplay => {
                        // Toggle the "draw hidden" debug view.
                        // This is per-seat presentation state;
                        // script-visible outline display
                        // changes still come from sim-side
                        // `SetOutlineDisplay` commands.
                        host.input.draw_hidden = !host.input.draw_hidden;
                    }
                    // Scroll{Up,Down,Left,Right} and Zoom{In,Out}
                    // are handled by the always-on view-only
                    // input pass at the top of the frame so
                    // they work during replay/rewind.
                    GameAction::ScrollUp
                    | GameAction::ScrollDown
                    | GameAction::ScrollLeft
                    | GameAction::ScrollRight
                    | GameAction::ZoomIn
                    | GameAction::ZoomOut => {}
                    GameAction::SelectAll => {
                        let cmd = PlayerCommand::SelectAllPcs;
                        dispatch_local_command(
                            host,
                            &mut manager.engine,
                            &mut frame.commands,
                            &assets,
                            &cmd,
                        );
                    }
                    GameAction::UnselectAll => {
                        let cmd = PlayerCommand::UnselectAllPcs;
                        dispatch_local_command(
                            host,
                            &mut manager.engine,
                            &mut frame.commands,
                            &assets,
                            &cmd,
                        );
                    }
                    GameAction::SelectAction { index } => {
                        let selected = manager.engine.seat_selection(host.local_seat);
                        if selected.len() == 1 {
                            let pc_id = selected[0];
                            let cmd = PlayerCommand::SelectAction {
                                pc_id,
                                action_index: *index as u32,
                            };
                            dispatch_local_command(
                                host,
                                &mut manager.engine,
                                &mut frame.commands,
                                &assets,
                                &cmd,
                            );
                        }
                    }
                    GameAction::SelectCharacter { portrait_index } => {
                        let idx = *portrait_index as usize;
                        let cmd = if ctrl_held {
                            PlayerCommand::AssignQuickGroup { index: idx as u8 }
                        } else {
                            let has_group =
                                idx < 9 && !manager.engine.quick_select_group(idx).is_empty();
                            if has_group {
                                PlayerCommand::RecallQuickGroup { index: idx as u8 }
                            } else {
                                PlayerCommand::SelectByPortrait {
                                    portrait_index: *portrait_index as u32,
                                    append: false,
                                }
                            }
                        };
                        dispatch_local_command(
                            host,
                            &mut manager.engine,
                            &mut frame.commands,
                            &assets,
                            &cmd,
                        );
                    }
                    GameAction::QuickSave => {
                        // F9 (default binding).  The quick-
                        // save request rotates the previous
                        // QuickSave to ExQuickSave before
                        // writing — distinct from the generic
                        // `LevelSave` state-machine path.
                        //
                        // Defer the save until any active zoom
                        // finishes so the mid-zoom background
                        // isn't captured.
                        if !manager.engine.is_zoom_possible(&host.engine_display) {
                            game.quick_save_after_zoom = true;
                        } else {
                            let campaign = manager.engine.campaign();
                            let mission_id = current_mission_id(campaign, &assets.profile_manager);
                            callbacks.pending = Some(SaveLoadRequest::QuickSave { mission_id });
                        }
                    }
                    GameAction::QuickLoad => {
                        // F12 (default binding).  Loads the
                        // quick-save slot into the current
                        // engine, with a zoom-defer gate and a
                        // Shift+F12 → backup (ExQuickSave)
                        // shortcut.  The cross-mission
                        // confirmation modal is handled by
                        // `confirm_quickload_cross_mission`
                        // running before the per-frame
                        // `perform_pending_save_load` flush —
                        // it either drops the queued request
                        // (No) or rewrites it to
                        // `SaveLoadRequest::Load` so the
                        // cross-mission `PendingLevelLoad`
                        // routing performs the mission swap
                        // (Yes).
                        if !manager.engine.is_zoom_possible(&host.engine_display) {
                            game.quick_load_after_zoom = true;
                        } else {
                            callbacks.pending = Some(SaveLoadRequest::QuickLoad {
                                use_backup: shift_held,
                            });
                        }
                    }
                    GameAction::CrouchDown => {
                        // Prime the crouch-down focus latch
                        // before issuing the command so the
                        // down-arrow "pressed" overlay
                        // appears for the full transition.
                        // Snapshot the pre-command stature so
                        // the latch clears the first frame
                        // posture shifts.
                        let pre = manager.engine.retrieve_stature(None);
                        let cmd = PlayerCommand::CrouchDown;
                        dispatch_local_command(
                            host,
                            &mut manager.engine,
                            &mut frame.commands,
                            &assets,
                            &cmd,
                        );
                        game.stature_focus.latch_crouch_down(pre);
                    }
                    GameAction::StandUp => {
                        // Companion of CrouchDown above —
                        // primes the stand-up focus latch so
                        // the up-arrow holds pressed while
                        // the sim runs the stand-up animation.
                        let pre = manager.engine.retrieve_stature(None);
                        let cmd = PlayerCommand::StandUp;
                        dispatch_local_command(
                            host,
                            &mut manager.engine,
                            &mut frame.commands,
                            &assets,
                            &cmd,
                        );
                        game.stature_focus.latch_stand_up(pre);
                    }
                    GameAction::KeyControl => {
                        // Save the current action on every
                        // selected PC.  Used by the
                        // "move during action" modifier so
                        // ctrl-release can restore the action.
                        let cmd = PlayerCommand::KeyControl;
                        dispatch_local_command(
                            host,
                            &mut manager.engine,
                            &mut frame.commands,
                            &assets,
                            &cmd,
                        );
                    }
                    GameAction::KeyReleaseControl => {
                        // Restore each selected PC's saved
                        // action on ctrl-up.  The handler
                        // honours the macOS carve-out via
                        // `cfg(target_os = "macos")`.
                        let cmd = PlayerCommand::KeyReleaseControl;
                        dispatch_local_command(
                            host,
                            &mut manager.engine,
                            &mut frame.commands,
                            &assets,
                            &cmd,
                        );
                    }
                    GameAction::SwitchTask => {
                        // Emit a reset-input so held-key edges
                        // caught during an Alt+Tab / Ctrl+Esc
                        // task switch don't re-fire in-game
                        // when focus returns.  Route through
                        // the engine messenger so the drain
                        // handler applies the reset
                        // symmetrically with the hide-console
                        // path.
                        manager
                            .engine
                            .send_simple_message(engine_messenger::SimpleMessage::SwitchTask);
                    }
                    GameAction::Teleport => {
                        // F7 cheat — teleport every selected
                        // PC to the current mouse map point.
                        let mouse_screen = input.threaded.position();
                        if let Some(mouse_map) = host.viewport.screen_to_map(mouse_screen) {
                            if !manager.engine.seat_selection(host.local_seat).is_empty() {
                                // Resolve destination sector/layer
                                // via `get_sector_screen_accessible`
                                // and bail when it returns None.
                                // Doors / motion obstacles / empty
                                // cells are rejected up front rather
                                // than going through as the topmost
                                // hit.
                                let accessible = manager
                                    .engine
                                    .fast_grid()
                                    .get_sector_screen_accessible(mouse_map);
                                if let Some(sector_idx) = accessible.sector_idx {
                                    let cmd = PlayerCommand::TeleportSelectedToPoint {
                                        dest: mouse_map,
                                        layer: accessible.layer,
                                        sector: u16::try_from(u32::from(sector_idx))
                                            .ok()
                                            .and_then(engine_position_interface::SectorHandle::new),
                                    };
                                    dispatch_local_command(
                                        host,
                                        &mut manager.engine,
                                        &mut frame.commands,
                                        &assets,
                                        &cmd,
                                    );
                                }
                            } else if dev.debug.free_shadow_polygon {
                                // With no PCs selected and the
                                // shadow-polygon dev cheat on,
                                // reposition the free-floating
                                // shadow-polygon viewer at the
                                // mouse map point, 45 units
                                // above the impact surface.
                                // Non-sim dev state, handled
                                // host-side outside the replay
                                // pipeline.
                                let p3d = manager.engine.fast_grid().convert_2d_to_3d(
                                    mouse_map,
                                    engine_sight_obstacle::SIGHTOBSTACLE_MOUSE,
                                    manager.engine.sight_obstacles(&assets),
                                );
                                dev.cheat_free_shadow_polygon_pos =
                                    Some(engine_coordinates::WorldPoint3D {
                                        x: p3d.x,
                                        y: p3d.y,
                                        z: p3d.z + 45.0,
                                    });
                            }
                        }
                    }
                    GameAction::RecordQa => {
                        // F5 (default binding) — replay the
                        // corner-clock left-click behaviour:
                        // start / cycle the macro slot for
                        // the currently-selected PC(s).
                        if !game.is_sherwood {
                            dispatch_corner_button_left_click(
                                CornerButton::Clock,
                                manager,
                                game,
                                host,
                                &assets,
                                &mut frame.commands,
                            );
                        }
                    }
                    GameAction::PrintScreen => {
                        // Defer to the post-render drain so we
                        // capture the fully-composited frame
                        // rather than an incomplete in-progress draw
                        // queue. Ctrl matches the historical wide
                        // snapshot branch; Shift applies the 3x3
                        // median filter branch.
                        host.pending_print_screen =
                            Some(print_screen_request_from_modifiers(ctrl_held, shift_held));
                    }
                    _ => {
                        tracing::trace!("Game action: {:?}", action);
                    }
                }
            }
        }
    }

    match handle_pause_menu_events(
        &mut ui.pause_menu,
        pause_closed_this_frame,
        host,
        manager,
        game,
        &assets,
        callbacks,
        &mut *window,
        &mut presentation.renderer,
        &mut resources.cursor,
        &mut presentation.sprites.cursor_renderer,
        &resources.menu,
        &mut audio.backend,
        &audio.sample_loader,
        &mut input.threaded,
        &mut input.translator,
        &mut hud.sherwood_layout,
        &mut hud.zoom_layout,
        &hud.zoom_sprites,
        &mut frame.commands,
        &events,
    )
    .await
    {
        HandlerAction::Continue => {
            return HandlerAction::Continue;
        }
        HandlerAction::Exit(code) => {
            execute_app_effects(
                &mut callbacks.app_effects,
                &mut host.sound,
                &mut input.threaded,
                audio
                    .backend
                    .as_mut()
                    .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
            );
            return HandlerAction::Exit(code);
        }
        HandlerAction::Proceed => {}
    }

    handle_mouse_input(
        manager,
        host,
        &assets,
        &presentation.renderer,
        &presentation.sprites.portrait_cache,
        &mut frame.commands,
        &events,
        ui.pause_menu.as_ref(),
        *pause_closed_this_frame,
        shift_held,
        ctrl_held,
    );

    HandlerAction::Proceed
}

/// Collect input, drive operation/save flows, and finalize the pre-tick
/// command stream.
pub(super) struct InteractiveFramePreparation<'mission, 'services, 'app> {
    mission: &'mission mut InteractiveMission,
    services: &'services mut MissionServices<'app>,
    state: Option<PreparationPhaseState>,
}

struct PreparationPhaseState {
    frame: MissionFrame,
    mp_clock_pause: bool,
    pause_closed_this_frame: bool,
    rewind_active: bool,
    shift_held: bool,
    step_forward_pressed: bool,
    step_back_pressed: bool,
    modal_rendered_this_frame: bool,
}

impl<'mission, 'services, 'app> InteractiveFramePreparation<'mission, 'services, 'app> {
    pub(super) fn new(
        mission: &'mission mut InteractiveMission,
        services: &'services mut MissionServices<'app>,
    ) -> Self {
        Self {
            mission,
            services,
            state: None,
        }
    }

    /// Collect input, drive operation/save flows, and finalize the pre-tick
    /// command stream.
    pub(super) async fn run(mut self) -> Result<FramePreparation, String> {
        if let Some(control) = self.collect_input_and_menus().await? {
            return Ok(FramePreparation::Control(control));
        }
        if let Some(control) = self.process_operation_and_save().await? {
            return Ok(FramePreparation::Control(control));
        }
        self.finalize_pre_tick()
    }

    async fn collect_input_and_menus(&mut self) -> Result<Option<FrameControl>, String> {
        let mission = &mut *self.mission;
        let services = &mut *self.services;
        let window = &mut *services.window;
        let callbacks = &mut *services.callbacks;
        let profiles = services.profiles;
        let FrameStart {
            mut frame,
            mp_clock_pause,
        } = begin_interactive_frame(mission);
        let modal_rendered_this_frame = false;
        // Preserve the existing statement order while migrating ownership. These
        // are disjoint borrows from the two mission-lifetime roots, not secondary
        // state copies.
        let InteractiveMission { runtime, frontend } = mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            control,
        } = runtime;
        let MissionWorld {
            host,
            game,
            manager,
            assets,
            dev,
        } = world;
        let MissionControl {
            manual_pause,
            step_forward_repeat_at_ms,
            step_back_repeat_at_ms,
            ..
        } = control;
        let input = &mut frontend.input;
        let audio = &mut frontend.audio;
        let resources = &mut frontend.resources;
        let ui = &mut frontend.ui;
        let hud = &mut frontend.hud;
        let presentation = &mut frontend.presentation;

        match handle_sherwood_campaign_map_overlay(
            game,
            manager,
            host,
            &mut frame.commands,
            &assets,
            &mut *window,
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &mut resources.text,
            &mut ui.campaign_map,
            &mut resources.menu,
            &mut hud.sherwood_enable,
        )
        .await?
        {
            HandlerAction::Continue => {
                runtime.trace(FrameContractStage::EarlyRestart);
                return Ok(Some(FrameControl::RestartIteration));
            }
            HandlerAction::Exit(code) => {
                runtime.trace(FrameContractStage::Exit);
                return Ok(Some(FrameControl::Exit(MissionExit::new(code))));
            }
            HandlerAction::Proceed => {}
        }

        // Tracks whether the pause menu closed at any point this
        // frame.  Used to flush queued input so actions that would
        // have run while the menu was up don't leak into the resumed
        // game.
        let mut pause_closed_this_frame = false;

        // ── Input ──
        // Flattened gameplay modals still own their own one-frame
        // `poll_events()` call.  If the main loop drains the window
        // first, modal widgets only receive occasional synthetic state,
        // while global gameplay shortcuts (notably Escape/DisplayMenu)
        // can fire underneath the modal.  Leave the event queue intact
        // whenever a modal is active or queued so `tick_active_modal`
        // gets first chance at the raw input.
        let modal_input_active = ui.active_modal.is_some() || modal_state_pending(&host);
        if modal_input_active && ui.close_pause(input, presentation) {
            pause_closed_this_frame = true;
            callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
        }
        // Disjoint-borrow event poll: `event_pump`/`width`/`height` are
        // separate fields from `canvas`, which the presentation.renderer owns mutably.
        let mut events = if modal_input_active {
            Vec::new()
        } else {
            window.poll_events()
        };
        input.threaded.feed_events(&events);

        let rewind_active = handle_hold_to_rewind(
            manager,
            assets.as_ref(),
            &input.threaded,
            &mut runtime.rewind_buffer,
            &mut runtime.rollback_checker,
            &mut runtime.replay_player,
        );

        // Field-disjoint access to keep `presentation.renderer` (holding &mut *window)
        // alive through the event loop.  Skip gamepad command dispatch
        // during replay/rewind — see input_suppressed comment below.
        if runtime.replay_player.is_none() && !rewind_active {
            handle_gamepad_events(
                host,
                manager,
                &assets,
                &mut input.threaded,
                &mut frame.commands,
                &events,
                &mut window.active_gamepad,
            );
        }
        events.extend(input.threaded.drain_synthetic_events());

        // ── Handle window resize ──
        // Window-size changes don't change the game's logical render
        // resolution any more — `present()` letterboxes the fixed-size
        // offscreen RT into whatever shape the WM hands the swapchain.
        // The `host.viewport.set_screen_size` + `presentation.renderer.resize` below
        // are kept for the graphics-options menu's resolution change
        // path, which fakes a Resized event with the user-picked
        // logical size; under the new arch we should separate those,
        // but for now: only fire the full
        // logical-resize cascade if the new size matches one of the
        // menu's supported resolutions. Pure WM resizes drop through
        // and only the swapchain reconfigures.
        apply_frame_resizes(
            &events,
            window,
            host,
            manager,
            assets.as_ref(),
            input,
            hud,
            presentation,
            &mut frame,
        );

        if input.threaded.is_ended() {
            runtime.trace(FrameContractStage::Exit);
            return Ok(Some(FrameControl::Exit(MissionExit::new(GameCode::Quit))));
        }

        match handle_sherwood_hud_buttons(
            game,
            manager,
            host,
            &mut frame.commands,
            &assets,
            callbacks,
            &mut *window,
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &resources.menu,
            &events,
            &hud.sherwood_layout,
            &mut hud.sherwood_enable,
        )
        .await
        {
            HandlerAction::Continue => {
                runtime.trace(FrameContractStage::EarlyRestart);
                return Ok(Some(FrameControl::RestartIteration));
            }
            HandlerAction::Exit(code) => {
                runtime.trace(FrameContractStage::Exit);
                return Ok(Some(FrameControl::Exit(MissionExit::new(code))));
            }
            HandlerAction::Proceed => {}
        }

        // Suppress all mouse-driven HUD widget clicks while a replay
        // is playing back (recorded commands re-enter at the tick
        // boundary) or a rewind hold is active (live clicks
        // shouldn't perturb a reconstructed past state).  Without
        // this the user could click HUD buttons mid-replay and steer
        // the run.
        let input_suppressed = runtime.replay_player.is_some() || rewind_active;

        // Zoom HUD buttons (ZoomUp / ZoomDown).  Maps to
        // `EngineStateRequest::ZoomingUp/Down` — same path the
        // mouse wheel + keyboard bindings use.
        if !input_suppressed {
            let zoom_enable = ZoomButtonEnable::from_engine(&manager.engine, &host.engine_display);
            let mut zoom_btn_hit = None;
            for event in &events {
                if let GameEvent::MouseDown(mx, my, 1 /* left */, _) = *event
                    && let Some(btn) = hud.zoom_layout.hit_test(mx, my, zoom_enable)
                {
                    zoom_btn_hit = Some((btn, mx, my));
                    break;
                }
            }
            if let Some((btn, mx, my)) = zoom_btn_hit {
                let factor = match btn {
                    ZoomButton::ZoomUp => 2.0,
                    ZoomButton::ZoomDown => 0.5,
                };
                host.viewport.zoom_by(
                    factor,
                    Some(engine_coordinates::ScreenPoint::new(mx as f32, my as f32)),
                );
            }
        }

        // Corner HUD buttons (Clock / Sight / QuickStart).  Only
        // active on non-Sherwood missions.
        //
        // Left-click dispatches the activation message (record /
        // lock-alt / launch-all).  Right-click unlocks / deletes
        // macros.
        if !game.is_sherwood && !input_suppressed {
            let corner_enable = CornerButtonEnable::from_engine(&manager.engine);
            for event in &events {
                match *event {
                    GameEvent::MouseDown(mx, my, 1 /* left */, _) => {
                        let Some(btn) = hud.corner_layout.hit_test(mx, my, corner_enable) else {
                            continue;
                        };
                        dispatch_corner_button_left_click(
                            btn,
                            manager,
                            game,
                            host,
                            &assets,
                            &mut frame.commands,
                        );
                    }
                    GameEvent::MouseDown(mx, my, 3 /* right */, _) => {
                        let Some(btn) = hud.corner_layout.hit_test_geometric(mx, my) else {
                            continue;
                        };
                        dispatch_corner_button_right_click(
                            btn,
                            manager,
                            host,
                            &assets,
                            &mut frame.commands,
                        );
                    }
                    _ => {}
                }
            }

            // Stature up/down-arrow click dispatch.  Emits the same
            // PlayerCommand the keyboard path uses.  Clicking either
            // arrow also primes the focus-latch so the arrow stays
            // visually pressed while the sim runs the posture
            // transition.  Auto-clears when the aggregate stature
            // shifts.
            let stature = manager.engine.retrieve_stature(None);
            game.stature_focus.maybe_clear(stature);
            let stature_enable =
                StatureEnable::from_stature(stature).with_focus_latch(game.stature_focus);
            for event in &events {
                if let GameEvent::MouseDown(mx, my, 1 /* left */, _) = *event
                    && let Some(btn) = hud.stature_layout.hit_test(mx, my, stature_enable)
                {
                    let cmd = btn.as_command();
                    dispatch_local_command(
                        host,
                        &mut manager.engine,
                        &mut frame.commands,
                        &assets,
                        &cmd,
                    );
                    match btn {
                        StatureButton::Up => {
                            game.stature_focus.latch_stand_up(stature);
                        }
                        StatureButton::Down => {
                            game.stature_focus.latch_crouch_down(stature);
                        }
                    }
                }
            }
        }

        // Edge-check the minimap accelerator key BEFORE
        // `translate_keyboard` advances the translator's prev-key
        // buffer.  The widget holds the accelerator itself and
        // toggles on release.
        let minimap_toggle_pressed = {
            host.minimap_fast_key.is_some_and(|fast_key| {
                input
                    .translator
                    .was_key_released(fast_key, &input.threaded.keyboard_state().keys)
            })
        };

        // Step-debug keys: `.` (forward), `,` / Backspace (back), Enter
        // (unpause).  `.` and `,` step immediately on the press edge,
        // then repeat after a short hold delay so a normal key tap
        // advances exactly one frame but holding still scrubs. Backspace
        // keeps its held-state rewind scrub behavior.  Enter uses the
        // release edge so a held Enter doesn't spam-resume.  All checks
        // read physical keys rather than the bindable `GameAction` map.
        use winit::keyboard::KeyCode;
        let keys = &input.threaded.keyboard_state().keys;
        // Suppress these shortcuts when any modal input sink has focus
        // so `.` / `,` / Enter typed into the console, pause menu, or
        // text input don't accidentally freeze/step the sim.
        let step_keys_gated =
            ui.console_overlay.is_visible() || ui.pause_menu.is_some() || modal_input_active;
        let step_shortcuts = plan_step_shortcuts(
            StepShortcutInput {
                now_ms: frame.started_at_ms,
                forward_held: keys.contains(&KeyCode::Period),
                forward_hit: input.translator.was_key_pressed(KeyCode::Period, keys),
                back_held: keys.contains(&KeyCode::Comma),
                back_hit: input.translator.was_key_pressed(KeyCode::Comma, keys),
                backspace_held: keys.contains(&KeyCode::Backspace),
                unpause_hit: input.translator.was_key_released(KeyCode::Enter, keys),
                gated: step_keys_gated,
            },
            step_forward_repeat_at_ms,
            step_back_repeat_at_ms,
        );
        if let Some(paused) = step_shortcuts.manual_pause {
            *manual_pause = paused;
        }
        let step_forward_pressed = step_shortcuts.forward;
        let step_back_pressed = step_shortcuts.back;

        // Translate to game actions
        let mut kb_actions = input
            .translator
            .translate_keyboard(&input.threaded.keyboard_state().keys, TranslationFlags::ALL);
        if events
            .iter()
            .any(|event| matches!(event, GameEvent::MenuToggleRequested))
            || (ui.pause_menu.is_none()
                && !modal_input_active
                && events
                    .iter()
                    .any(|event| matches!(event, GameEvent::PauseRequested)))
        {
            kb_actions.push(GameAction::DisplayMenu);
        }
        let mouse_actions = if input.threaded.has_position() {
            input.translator.translate_mouse(
                input.threaded.position().x,
                input.threaded.position().y,
                input.threaded.wheel_delta(),
            )
        } else {
            Vec::new()
        };

        let modifiers = input_modifiers(&input.threaded.keyboard_state().keys);
        let InputModifiers {
            ctrl: _,
            shift: shift_held,
            alt: alt_held,
        } = modifiers;
        // Persist the alt state on `InputState` so subsystems that
        // don't otherwise see the platform modifier state can read it.
        host.input.is_alt = alt_held;

        handle_console_overlay_events(
            &mut ui.console_overlay,
            &mut manager.engine,
            &assets,
            host,
            dev,
            &events,
            &kb_actions,
            &mut input.translator,
        );

        // ── View-only input (scroll / zoom): always allowed ──
        // These mutate host-side viewport state only — never the sim —
        // so they're safe during replay playback and rewind, when the
        // user wants to pan/zoom around the paused world.  Suppressed
        // only when the console or the pause menu has focus.
        apply_host_view_input(
            host,
            &manager.engine,
            input.threaded.position(),
            &kb_actions,
            &mouse_actions,
            &events,
            ui.console_overlay.is_visible() || ui.pause_menu.is_some() || pause_closed_this_frame,
            ui.pause_menu.is_some() || pause_closed_this_frame,
        );

        // ── Skip all sim-affecting input during replay / rewind ──
        // Recorded commands are injected at the tick boundary instead
        // (replay), or suppressed entirely (rewind — live input
        // shouldn't perturb a state reconstructed from the past).
        if runtime.replay_player.is_none() && !rewind_active {
            match drive_live_gameplay_input(
                host,
                manager,
                game,
                assets.as_ref(),
                dev,
                callbacks,
                window,
                presentation,
                resources,
                audio,
                input,
                ui,
                hud,
                &mut frame,
                &events,
                &kb_actions,
                &mouse_actions,
                minimap_toggle_pressed,
                modifiers,
                &mut pause_closed_this_frame,
            )
            .await
            {
                HandlerAction::Continue => {
                    runtime.trace(FrameContractStage::EarlyRestart);
                    return Ok(Some(FrameControl::RestartIteration));
                }
                HandlerAction::Exit(code) => {
                    runtime.trace(FrameContractStage::Exit);
                    return Ok(Some(FrameControl::Exit(MissionExit::new(code))));
                }
                HandlerAction::Proceed => {}
            }
        }
        // ── Cross-mission QuickLoad confirmation modal ──
        // Quick-load prompts the
        // player with `MSG_REALLY_LOAD_QUICKSAVE` whenever the quicksave
        // header's mission ID differs from the running mission.  Run
        // the modal here, before the thumbnail capture and state-machine
        // drain — the helper either drops the pending request (No) or
        // rewrites it into a `Load` so the existing cross-mission
        // routing performs the mission swap (Yes).
        confirm_quickload_cross_mission(
            callbacks,
            &manager.engine,
            profiles,
            &host,
            &mut *window,
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &resources.menu,
        )
        .await;

        runtime.trace(FrameContractStage::InputAndMenus);

        self.state = Some(PreparationPhaseState {
            frame,
            mp_clock_pause,
            pause_closed_this_frame,
            rewind_active,
            shift_held,
            step_forward_pressed,
            step_back_pressed,
            modal_rendered_this_frame,
        });
        Ok(None)
    }

    async fn process_operation_and_save(&mut self) -> Result<Option<FrameControl>, String> {
        let services = &mut *self.services;
        let callbacks = &mut *services.callbacks;
        let profiles = services.profiles;
        let PreparationPhaseState {
            frame,
            mp_clock_pause,
            pause_closed_this_frame,
            rewind_active,
            shift_held,
            step_forward_pressed,
            step_back_pressed,
            modal_rendered_this_frame,
        } = self.state.take().expect("input phase must complete first");
        let InteractiveMission { runtime, frontend } = &mut *self.mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            ..
        } = runtime;
        let MissionWorld {
            host,
            game,
            manager,
            assets,
            dev,
        } = world;
        let input = &mut frontend.input;
        let audio = &mut frontend.audio;
        let resources = &mut frontend.resources;
        let ui = &mut frontend.ui;
        let hud = &mut frontend.hud;
        let presentation = &mut frontend.presentation;

        // ── Process game operations (save/load/quit/win/lose) ──
        runtime.trace(FrameContractStage::OperationAndSave);
        //
        // The Game state machine queues save/load intents on the
        // callbacks; `perform_pending_save_load` then flushes them to
        // disk with live engine access.
        let exit_code = game.process_operation(manager.engine.campaign(), profiles, callbacks);
        let pending_thumbnail = if callbacks
            .pending
            .as_ref()
            .is_some_and(|request| request.writes_save_payload())
            && !host.skip_render
            && !modal_rendered_this_frame
        {
            pre_render_engine_setup(manager, host, assets.as_ref(), &mut presentation.renderer);
            update_mouse_and_cursor(
                manager,
                host,
                &assets,
                &dev,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &input.threaded,
                &presentation.sprites.portrait_cache,
                shift_held,
                &mut hud.last_cursor_id,
            );
            let display_snapshot = host.engine_display.clone();
            let mut render_ctx = presentation.render_context(
                resources,
                hud,
                input,
                ui,
                game,
                RenderViewState {
                    shift_held,
                    rewind_active,
                    display_info_elapsed_secs:
                        <RustCallbacks as crate::game::GameCallbacks>::get_current_playing_time(
                            callbacks,
                            manager.engine.campaign(),
                        ),
                },
            );
            capture_save_thumbnail(
                &manager.engine,
                &display_snapshot,
                host,
                &assets,
                &dev,
                &mut render_ctx,
            )
        } else {
            None
        };
        if let Some(exit_code) = exit_code {
            // `RHGame::GameLoop` applies transition sound/input changes
            // before returning its terminal code. Execute them here so
            // the mission-local Host is not dropped with queued effects.
            execute_app_effects(
                &mut callbacks.app_effects,
                &mut host.sound,
                &mut input.threaded,
                audio
                    .backend
                    .as_mut()
                    .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
            );
            tracing::info!("Game exited with: {:?}", exit_code);
            // Flush any pending save before returning (e.g. the
            // quit-time continue save).
            let save_load_processed = perform_pending_save_load(
                host,
                game,
                callbacks,
                &mut manager.engine,
                assets.as_ref(),
                profiles,
                pending_thumbnail.clone(),
            );
            if save_load_processed && let Some(ref mut checker) = runtime.rollback_checker {
                checker.reset();
            }
            if let Some(sync) = callbacks.post_load_sync.take() {
                game.apply_post_load_sync(sync.is_continue);
                game.post_load_resolution_resync();
            }
            runtime.trace(FrameContractStage::Exit);
            return Ok(Some(FrameControl::Exit(MissionExit::new(exit_code))));
        }
        let save_load_processed = perform_pending_save_load(
            host,
            game,
            callbacks,
            &mut manager.engine,
            assets.as_ref(),
            profiles,
            pending_thumbnail,
        );
        if save_load_processed && let Some(ref mut checker) = runtime.rollback_checker {
            checker.reset();
        }

        // ── Cross-mission load: bubble up ──
        // `perform_pending_save_load` stashes a `PendingLevelLoad` when the
        // chosen slot targets a different mission than the one running. Force
        // the Game state machine into LevelLoad so `process_operation` exits
        // on the next iteration; the outer session loop will switch missions
        // and re-queue the Load on the fresh engine.
        if callbacks.pending_level_load.is_some() {
            game.operation.set(GameCode::LevelLoad);
            runtime.trace(FrameContractStage::Exit);
            return Ok(Some(FrameControl::Exit(MissionExit::new(
                GameCode::LevelLoad,
            ))));
        }

        // ── Post-load slot-type sync ──
        // Sync the continue-save flag and re-arm the campaign-map
        // overlay if the loaded save had it open.  `post_load_sync`
        // is armed by `perform_pending_save_load` after any Load
        // variant succeeds, threading the slot type back out of the
        // save-I/O layer.
        if let Some(sync) = callbacks.post_load_sync.take() {
            game.apply_post_load_sync(sync.is_continue);
            game.post_load_resolution_resync();
        }

        apply_post_save_ui_state(callbacks, game, input);

        self.state = Some(PreparationPhaseState {
            frame,
            mp_clock_pause,
            pause_closed_this_frame,
            rewind_active,
            shift_held,
            step_forward_pressed,
            step_back_pressed,
            modal_rendered_this_frame,
        });
        Ok(None)
    }

    fn finalize_pre_tick(&mut self) -> Result<FramePreparation, String> {
        let PreparationPhaseState {
            mut frame,
            mut mp_clock_pause,
            pause_closed_this_frame: _,
            rewind_active,
            shift_held,
            step_forward_pressed,
            step_back_pressed,
            modal_rendered_this_frame,
        } = self
            .state
            .take()
            .expect("operation/save phase must complete first");
        let InteractiveMission { runtime, frontend } = &mut *self.mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            control,
        } = runtime;
        let MissionWorld {
            host,
            game: _,
            manager,
            assets,
            ..
        } = world;
        let manual_pause = &mut control.manual_pause;
        let input = &mut frontend.input;
        let ui = &mut frontend.ui;

        // ── Replay: inject recorded commands + desync check ──
        // `ModalDismiss` commands are split out of the recorded stream
        // here and handed to the modal drain step further down, so the
        // interactive dialog / popup event loops are skipped during
        // playback. All other commands are sim-affecting and applied
        // immediately.
        // Freeze every sim-advancing step (replay playback, engine
        // tick, rewind-buffer commit, sim-frame increment) whenever the
        // user has asked to pause.  Under `--replay`, this means the
        // player's cursor on the recorded command stream stops too —
        // otherwise `--start-paused --replay` would still race through
        // the replay even though the tick was suppressed.
        let modal_pause = ui
            .active_modal
            .as_ref()
            .is_some_and(|modal| !modal.is_empty());

        // Drain once more at the last deterministic pre-tick boundary.
        // Packets can arrive after the top-of-loop drain while this
        // frame handles UI, local input, and modal work.  Applying due
        // inputs here keeps them on the same `sim_frame` without
        // mutating sim state at arbitrary points in the frame.
        drain_pre_tick_network(
            runtime,
            host,
            manager,
            assets.as_ref(),
            &mut frame,
            manual_pause,
            &mut mp_clock_pause,
            rewind_active,
        );

        // ── Multiplayer: state hash broadcast / verify ──
        // Sample after the final deterministic pre-tick network drain.
        // Inputs can arrive between the top-of-loop drain and this
        // boundary; hashing earlier can compare two machines that will
        // tick the same commands but sampled before/after a current-frame
        // input that just arrived.
        process_pre_tick_state_hash(runtime, host, manager);

        let paused = pre_tick_is_paused(PreTickPauseSources {
            pause_menu: ui.pause_menu.is_some(),
            manual: *manual_pause,
            multiplayer_clock: mp_clock_pause,
            modal: modal_pause,
        });
        let PreTickTimelineOutput {
            paused,
            consumed_buffered,
        } = prepare_pre_tick_timeline(
            runtime,
            host,
            manager,
            assets.as_ref(),
            &mut frame,
            manual_pause,
            rewind_active,
            paused,
        )?;

        dispatch_pre_tick_pointer_commands(
            runtime,
            host,
            manager,
            assets.as_ref(),
            input,
            &mut frame,
            rewind_active,
            paused,
        );

        if paused || rewind_active {
            runtime.trace(FrameContractStage::PausedOrRewind);
        }
        runtime.trace(FrameContractStage::PreTickCommands);
        Ok(FramePreparation::Ready(PreparedFrame {
            frame,
            rewind_active,
            paused,
            consumed_buffered,
            shift_held,
            modal_rendered: modal_rendered_this_frame,
            step_forward_pressed,
            step_back_pressed,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PreTickPauseSources, StepShortcutInput, StepShortcutOutput, plan_step_shortcuts,
        pre_tick_is_paused,
    };

    fn step_input(now_ms: u32) -> StepShortcutInput {
        StepShortcutInput {
            now_ms,
            forward_held: false,
            forward_hit: false,
            back_held: false,
            back_hit: false,
            backspace_held: false,
            unpause_hit: false,
            gated: false,
        }
    }

    #[test]
    fn step_shortcuts_preserve_edge_repeat_and_modal_gating() {
        let mut forward_repeat = None;
        let mut back_repeat = None;
        let first = plan_step_shortcuts(
            StepShortcutInput {
                forward_held: true,
                forward_hit: true,
                ..step_input(100)
            },
            &mut forward_repeat,
            &mut back_repeat,
        );
        assert_eq!(
            first,
            StepShortcutOutput {
                forward: true,
                back: false,
                manual_pause: Some(true),
            }
        );
        assert_eq!(forward_repeat, Some(260));

        let before_repeat = plan_step_shortcuts(
            StepShortcutInput {
                forward_held: true,
                ..step_input(259)
            },
            &mut forward_repeat,
            &mut back_repeat,
        );
        assert!(!before_repeat.forward);

        let gated_repeat = plan_step_shortcuts(
            StepShortcutInput {
                forward_held: true,
                gated: true,
                ..step_input(260)
            },
            &mut forward_repeat,
            &mut back_repeat,
        );
        assert_eq!(
            gated_repeat,
            StepShortcutOutput {
                forward: false,
                back: false,
                manual_pause: None,
            }
        );
        assert_eq!(forward_repeat, Some(300));
    }

    #[test]
    fn unpause_edge_wins_when_sampled_with_a_step() {
        let mut forward_repeat = None;
        let mut back_repeat = None;
        let output = plan_step_shortcuts(
            StepShortcutInput {
                forward_held: true,
                forward_hit: true,
                unpause_hit: true,
                ..step_input(400)
            },
            &mut forward_repeat,
            &mut back_repeat,
        );

        assert!(output.forward);
        assert_eq!(output.manual_pause, Some(false));
    }

    #[test]
    fn pre_tick_pause_combines_all_graphical_pause_sources() {
        let clear = PreTickPauseSources {
            pause_menu: false,
            manual: false,
            multiplayer_clock: false,
            modal: false,
        };
        assert!(!pre_tick_is_paused(clear));

        for paused in [
            PreTickPauseSources {
                pause_menu: true,
                ..clear
            },
            PreTickPauseSources {
                manual: true,
                ..clear
            },
            PreTickPauseSources {
                multiplayer_clock: true,
                ..clear
            },
            PreTickPauseSources {
                modal: true,
                ..clear
            },
        ] {
            assert!(pre_tick_is_paused(paused));
        }
    }
}

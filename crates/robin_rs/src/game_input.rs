//! Input resolution — translates mouse clicks and keyboard actions into
//! [`PlayerCommand`]s by reading engine state immutably.
//!
//! The input system **never** holds `&mut Engine`.  It reads positions,
//! selection, entity state, and focus-test results through `&Engine`,
//! then returns a `Vec<PlayerCommand>` that the game session feeds to
//! `Engine::apply_commands`.  This clean separation is the foundation
//! for deterministic replay and rollback networking.

use crate::host::Host;
use crate::mouse_way::{GestureCoachFeedback, MouseWayPattern};
use crate::shadow_polygon::ASPECT_RATIO;
use robin_engine::campaign as engine_campaign;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::MapPoint;
use robin_engine::element as engine_element;
use robin_engine::element::{ActionState, Command, Entity, EntityId, Focus, ListenPhase, Posture};
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::player_command::{
    CompositeSwordTechnique, GestureQuality, PlayerCommand, PlayerId, QueuedQuickActionCommand,
};
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::Action;
use robin_engine::sector as engine_sector;
use robin_engine::sector::SectorNumber;
use robin_engine::sequence::Field;
use robin_engine::sight_obstacle as engine_sight_obstacle;
use robin_engine::tactical_control::TacticalFormation;

// ─── Left-click resolution ──────────────────────────────────────────

/// Resolve a left-click at `map_pt` into player commands.
///
/// The engine is read-only; all mutations are expressed as commands.
#[cfg(test)]
fn resolve_left_click(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    map_pt: MapPoint,
    shift_held: bool,
    ctrl_held: bool,
    is_double: bool,
) -> Vec<PlayerCommand> {
    resolve_left_click_with_planning(
        host,
        engine,
        assets,
        map_pt,
        ClickModifiers {
            shift: shift_held,
            planning: shift_held,
            control: ctrl_held,
            double: is_double,
        },
    )
}

/// Resolve a click with Original physical-Shift behaviour separated from the
/// post-port planning modifier.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct ClickModifiers {
    pub shift: bool,
    pub planning: bool,
    pub control: bool,
    pub double: bool,
}

pub fn resolve_left_click_with_planning(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    map_pt: MapPoint,
    modifiers: ClickModifiers,
) -> Vec<PlayerCommand> {
    let ClickModifiers {
        shift: shift_held,
        planning: planning_held,
        control: ctrl_held,
        double: is_double,
    } = modifiers;
    let local_seat = host.transport.local_seat();
    let selected = engine.hero_selection(local_seat);
    let num_selected = selected.len();
    let tactical_selected = engine.tactical_selection(local_seat);

    // The optional allied-control layer keeps soldier selection separate from
    // the original PC selection so scripts and hero action bars retain their
    // five-PC assumptions. A direct click still feels like ordinary unit
    // selection and may coexist with heroes when Shift is held.
    if host.frontend.preferences().control_tactical_units()
        && let Some(soldier) = engine.find_tactically_controllable_unit(
            assets,
            &host.frontend.presentation.draw_order.ids,
            map_pt,
        )
    {
        let mut commands = Vec::new();
        if !shift_held {
            commands.push(PlayerCommand::UnselectAllPcs);
        }
        commands.push(PlayerCommand::SelectTacticalUnits {
            soldiers: vec![soldier],
            append: shift_held,
        });
        return commands;
    }

    // Pre-process for double-clicks: when an action is armed and any
    // selected PC's profile lacks the action or has it disabled, abort
    // the entire double-click.  The prior single-click already fired
    // through the regular left-button arm, so suppressing the
    // double-click here prevents the action / cancel-arm / repeat-
    // interact branches below from dispatching a second time.  The
    // "no PCs selected" case is implicit: `selected_action_for_seat`
    // returns `NoAction` when nothing is selected, so the `!= NoAction`
    // gate already excludes it.
    if is_double && !planning_held && num_selected > 0 {
        let pending_action = if planning_held {
            engine.planned_action_for_seat(local_seat)
        } else {
            engine.selected_action_for_seat(local_seat)
        };
        if pending_action != Action::NoAction {
            for &pc_id in selected {
                if !engine.is_pc_action_available(&assets.profile_manager, pc_id, pending_action) {
                    return vec![];
                }
            }
        }
    }

    // Action-specific click dispatch
    if num_selected > 0 {
        let selected_action = if planning_held {
            engine.planned_action_for_seat(local_seat)
        } else {
            engine.selected_action_for_seat(local_seat)
        };
        if selected_action != Action::NoAction {
            // Double-click-specific behaviour:
            //   - Whistle/Listen/Eat/Guzzle cancel the action instead
            //     of re-firing it.
            //   - Apple/Stone/Bow/Purse/WaspNest/Net while recording a
            //     macro (with ≥1 selected PC) commit the macro instead
            //     of dispatching a second action step — this prevents a
            //     stray extra step from leaking into the recording.
            if is_double {
                match selected_action {
                    Action::Whistle | Action::Listen | Action::Eat | Action::Guzzle => {
                        return vec![PlayerCommand::UnselectAllActions];
                    }
                    Action::Apple
                    | Action::Stone
                    | Action::Bow
                    | Action::Purse
                    | Action::WaspNest
                    | Action::Net
                        if engine.is_recording_macro() =>
                    {
                        return vec![PlayerCommand::StopRecordingMacro];
                    }
                    _ => {}
                }
            }
            let cmds = resolve_action_left_click(
                host,
                engine,
                assets,
                map_pt,
                local_seat,
                selected_action,
                is_double,
                planning_held,
            );
            return cmds;
        } else if is_double
            && (engine.is_alt_effective(&host.frontend.input) || engine.view_locked())
        {
            // No-action double-click with Alt or Locker held: swallow
            // the click (no run-move).  Without this, the GroupMove
            // fallback below would issue a running move on every
            // double-click regardless of modifiers.
            return vec![];
        }
    }

    // Double-click repeat-interact
    if is_double
        && num_selected > 0
        && let Some(cached) = host.frontend.input.gestures.element_old_click
    {
        let cmds = resolve_double_click_repeat(engine, assets, cached, local_seat);
        if !cmds.is_empty() {
            return cmds;
        }
    }

    // Clear the click cache at entry; every hit branch below
    // re-assigns it, while the map-click fallback leaves it as `None`.
    // Done once here so the clear doesn't get sprinkled across every
    // early-exit path.  The double-click replay above still reads the
    // cached value first.
    host.frontend.input.gestures.element_old_click = None;

    // No PCs selected: a controlled allied group can still engage a
    // sword-focusable target, select a hero, or move.
    if num_selected == 0 {
        if let Some(pc_id) = engine.find_focusable_entity(
            assets,
            &host.frontend.presentation.draw_order.ids,
            map_pt,
            Focus::Select,
        ) {
            host.frontend.input.gestures.element_old_click = Some(pc_id);
            let mut commands = Vec::new();
            if !shift_held && !tactical_selected.is_empty() {
                commands.push(PlayerCommand::ClearTacticalSelection);
            }
            commands.push(PlayerCommand::SelectPc {
                pc_id,
                append: shift_held,
            });
            return commands;
        }
        if host.frontend.preferences().control_tactical_units() && !tactical_selected.is_empty() {
            if let Some(target_id) = engine.find_focusable_entity(
                assets,
                &host.frontend.presentation.draw_order.ids,
                map_pt,
                Focus::Sword,
            ) {
                host.frontend.input.gestures.element_old_click = Some(target_id);
                return tactical_selected
                    .iter()
                    .copied()
                    .map(|actor| PlayerCommand::EnterSwordfight {
                        actor,
                        target: target_id,
                        running: false,
                    })
                    .collect();
            }
            return vec![PlayerCommand::MoveTacticalUnits {
                formation: selected_tactical_formation(engine, tactical_selected),
                soldiers: tactical_selected.to_vec(),
                destination: map_pt,
                running: is_double,
            }];
        }
        host.frontend.input.gestures.element_old_click = None;
        return vec![];
    }

    let is_swordfighting = is_selected_unit_swordfighting(&engine.presentation_view(), local_seat);

    // Unselected PC → select it
    if let Some(pc_id) = engine.find_focusable_pc(assets, map_pt, Focus::Select)
        && !selected.contains(&pc_id)
    {
        host.frontend.input.gestures.element_old_click = Some(pc_id);
        if ctrl_held {
            return vec![PlayerCommand::TogglePcSelection { pc_id }];
        } else {
            let mut commands = Vec::new();
            if !shift_held && !tactical_selected.is_empty() {
                commands.push(PlayerCommand::ClearTacticalSelection);
            }
            commands.push(PlayerCommand::SelectPc {
                pc_id,
                append: shift_held,
            });
            return commands;
        }
    }

    // Use-focusable entity (search/carry/tie) — single selection, not swordfighting
    if !is_swordfighting
        && num_selected == 1
        && let Some(target_id) = engine.find_focusable_entity(
            assets,
            &host.frontend.presentation.draw_order.ids,
            map_pt,
            Focus::Use,
        )
    {
        let pc_id = selected[0];
        // Scroll-attached NPC — opens a dialog.  Hands a composite
        // `LOCK_AI → turn ×2 → UNLOCK_AI → OPEN_SCROLL` sequence to the
        // PC; the engine-side helper `apply_scroll_read_with_seek`
        // builds the composite and prepends a seek as needed.
        if is_target_scroll_attached_npc(engine, target_id) {
            host.frontend.input.gestures.element_old_click = Some(target_id);
            return vec![PlayerCommand::LaunchScrollRead {
                actor: pc_id,
                target: target_id,
                running: is_double,
            }];
        }
        if let Some(cmd) = determine_use_command(engine, assets, pc_id, target_id) {
            host.frontend.input.gestures.element_old_click = Some(target_id);
            // A click on a coin forwards to the source purse when the
            // purse isn't yet taken — route the actual Take launch at
            // the purse id so its has-been-taken sweep fires on
            // arrival.
            let launch_target = match cmd {
                Command::Take => engine_api::coin_pickup_target(engine, target_id),
                _ => target_id,
            };
            // Net-specific double-click handling:
            //   double + !recording → MakePcFast; return (skip seek+take)
            //   double + recording  → run the seek+take with running gait
            // Only nets get this handling — other object types fall
            // through to the regular take path.
            let target_is_net = matches!(
                engine.get_entity(launch_target),
                Some(engine_element::Entity::Net(_))
            );
            let is_recording = engine.is_recording_macro();
            if is_double && target_is_net && cmd == Command::Take && !is_recording {
                return selected
                    .iter()
                    .map(|&pc| PlayerCommand::MakePcFast { pc_id: pc })
                    .collect();
            }
            let running = is_double && target_is_net && cmd == Command::Take && is_recording;
            let mut cmds = vec![PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: launch_target,
                command: cmd,
                running,
            }];
            // Stop macro recording after the registration on a Net
            // Take.  This lives outside the action-mode dispatch
            // because the Net Take comes through the no-action click
            // path.
            if is_recording && target_is_net && cmd == Command::Take {
                cmds.push(PlayerCommand::StopRecordingMacro);
            }
            return cmds;
        }
    }

    // Sword-focusable entity → engage in combat.
    // This path only runs on single-click, so the seek uses walking
    // animation (running=false).
    //
    // Soldier / non-soldier break: when the sword target is NOT a
    // soldier, only the first selected PC engages.  For soldiers every
    // selected PC piles on.
    if let Some(target_id) = engine.find_focusable_entity(
        assets,
        &host.frontend.presentation.draw_order.ids,
        map_pt,
        Focus::Sword,
    ) {
        host.frontend.input.gestures.element_old_click = Some(target_id);
        let target_is_soldier = engine
            .get_entity(target_id)
            .expect("same-frame sword focus must identify a live entity")
            .is_soldier();
        // Soldier targets accept the whole selection; other sword-focus targets
        // use only the leading hero. Borrow both selections until commands own IDs.
        let engager_limit = if target_is_soldier { usize::MAX } else { 1 };
        return selected
            .iter()
            .chain(tactical_selected)
            .copied()
            .take(engager_limit)
            .map(|pc_id| PlayerCommand::EnterSwordfight {
                actor: pc_id,
                target: target_id,
                running: false,
            })
            .collect();
    }

    // Nothing hit: move to clicked location.
    //
    // Single-click:
    //   if a patch is selected: unlocked → group-move to waypoint;
    //                           locked → HeroSpeak (unable to do something).
    //   else if valid sector position: group-move to clicked point.
    //   else: no-op.
    //
    // Double-click splits on macro-recording:
    //   recording → patch → GroupMove(waypoint, RUNNING); else sector →
    //               GroupMove(pt, RUNNING); else no-op.
    //   !recording → valid sector → MakePcFast per selected PC (no
    //                patch branch, no fresh seek).
    host.frontend.input.gestures.element_old_click = None;

    let is_recording = engine.is_recording_macro();

    // Non-recording double-click tail: MakePcFast, no fresh move.
    // The patch branch is intentionally ignored here — only the
    // recording arm honours the patch redirect.
    if is_double && !is_recording {
        if host.frontend.input.spatial_hit().valid_position_for_move
            && host
                .frontend
                .input
                .spatial_hit()
                .selected_sector_idx
                .is_some()
        {
            let mut commands: Vec<_> = selected
                .iter()
                .map(|&pc_id| PlayerCommand::MakePcFast { pc_id })
                .collect();
            // A box selection may contain both PCs and controllable allied
            // soldiers. The original-PC acceleration return above used to
            // discard the soldiers' half of that mixed selection, making the
            // gallery troops appear unable to run whenever Robin was boxed
            // with them.
            if host.frontend.preferences().control_tactical_units() && !tactical_selected.is_empty()
            {
                commands.push(PlayerCommand::MoveTacticalUnits {
                    formation: selected_tactical_formation(engine, tactical_selected),
                    soldiers: tactical_selected.to_vec(),
                    destination: map_pt,
                    running: true,
                });
            }
            return commands;
        }
        return vec![];
    }

    // Single-click path, plus the recording double-click (which
    // follows the same patch→GroupMove / sector→GroupMove ordering
    // with the running gait on double-click).
    if let Some(patch_idx) = host.frontend.input.spatial_hit().selected_patch_idx
        && let Some(patch) = engine
            .mission_script()
            .and_then(|_| engine.patches().get(patch_idx as usize))
    {
        if patch.locked {
            // Locked patch: the first selected PC speaks "unable to do
            // something" instead of moving.  The recording-double-click
            // arm skips the lock check entirely and just group-moves
            // the waypoint, so we bypass the HeroSpeak in that case.
            if !(is_double && is_recording) {
                if let Some(&pc_id) = selected.first() {
                    return vec![PlayerCommand::HeroSpeak {
                        pc_id,
                        expression: engine_api::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    }];
                }
                return vec![];
            }
        }
        let actors: Vec<EntityId> = selected.to_vec();
        // Mirror the original game's mouse-update substitution
        // selected-sector and selected-layer substitution: pass the patch's
        // proto-loaded `(sector, layer)` to perform_group_move so the
        // move targets the patch's underlying motion area instead of
        // whatever the spatial lookup at the waypoint happens to find
        // (which can pick the wrong layer or no sector at all).
        let goal_override = Some((
            engine_sector::SectorNumber::new(patch.sector as i16),
            patch.layer,
        ));
        let mut commands = vec![PlayerCommand::GroupMove {
            actors,
            destination: patch.waypoint,
            running: is_double,
            show_marker: false,
            goal_override,
            // `update_mouse` already resolved the patch to its exact
            // underlying FastFindGrid sector. Original carries that
            // sector reference into group movement; retaining only the public
            // sector number is ambiguous on maps that reuse numbers.
            goal_sector_index_override: host.frontend.input.spatial_hit().selected_sector_idx,
            door_route_override: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: Vec::new(),
        }];
        if host.frontend.preferences().control_tactical_units() && !tactical_selected.is_empty() {
            commands.push(PlayerCommand::MoveTacticalUnits {
                formation: selected_tactical_formation(engine, tactical_selected),
                soldiers: tactical_selected.to_vec(),
                destination: patch.waypoint,
                running: is_double,
            });
        }
        return commands;
    }

    // Sector-click branch — gated on both predicates.
    if !(host.frontend.input.spatial_hit().valid_position_for_move
        && host
            .frontend
            .input
            .spatial_hit()
            .selected_sector_idx
            .is_some())
    {
        return vec![];
    }

    let actors: Vec<EntityId> = selected.to_vec();
    let goal_sector_index = host.frontend.input.spatial_hit().selected_sector_idx;
    let goal_override = goal_sector_index.and_then(|idx| {
        engine
            .fast_grid()
            .level
            .sectors
            .get(usize::from(idx))
            .and_then(|sector| {
                let st = sector.sector_type;
                if st.is_door() || st.is_jump() {
                    None
                } else {
                    Some((
                        sector.sector_number,
                        host.frontend.input.spatial_hit().selected_layer,
                    ))
                }
            })
    });
    let mut commands = vec![PlayerCommand::GroupMove {
        actors,
        destination: map_pt,
        running: is_double,
        show_marker: true,
        goal_override,
        // Preserve the exact cursor-selected arena slot alongside the public
        // sector/layer, mirroring the original game's authoritative sector reference.
        goal_sector_index_override: goal_override.and(goal_sector_index),
        door_route_override: None,
        recorded_gate_routes: Vec::new(),
        recorded_failed_gate_routes: Vec::new(),
    }];
    if host.frontend.preferences().control_tactical_units() && !tactical_selected.is_empty() {
        commands.push(PlayerCommand::MoveTacticalUnits {
            formation: selected_tactical_formation(engine, tactical_selected),
            soldiers: tactical_selected.to_vec(),
            destination: map_pt,
            running: is_double,
        });
    }
    commands
}

/// Convert Shift-click results into non-live quick-action queue commands.
/// Selection/UI commands are deliberately left alone: Shift+click on a
/// portrait still extends selection, while world actions are recorded.
pub fn queue_shift_click_commands(
    commands: Vec<PlayerCommand>,
    action: Action,
    shift_held: bool,
) -> Vec<PlayerCommand> {
    if !shift_held {
        return commands;
    }
    commands
        .into_iter()
        .filter_map(|command| match command {
            command @ (PlayerCommand::GroupMove { .. }
            | PlayerCommand::MoveTacticalUnits { .. }
            | PlayerCommand::LaunchInteraction { .. }
            | PlayerCommand::LaunchGroundTarget { .. }
            | PlayerCommand::DropAleAt { .. }
            | PlayerCommand::LaunchSelfAbility { .. }
            | PlayerCommand::LaunchScrollRead { .. }
            | PlayerCommand::EnterSwordfight { .. }
            | PlayerCommand::SwordStrikeCmd { .. }
            | PlayerCommand::RaiseShieldWithDanger { .. }
            | PlayerCommand::CrouchDown
            | PlayerCommand::StandUp) => Some(PlayerCommand::QueueQuickAction {
                action,
                command: QueuedQuickActionCommand::from(command),
            }),
            PlayerCommand::MakePcFast { pc_id } => {
                Some(PlayerCommand::MakeQueuedActionFast { pc_id })
            }
            // These trailers only clear the live action arm. Planned actions
            // are intentionally sticky while Shift is held, and must never
            // unequip/interrupt the real PC.
            PlayerCommand::UnselectAllActions | PlayerCommand::CancelAction { .. } => None,
            // Selection changes remain UI intent while Shift is held. Do not
            // let any unclassified simulation command leak through live: new
            // click-result variants must opt into queuing explicitly above.
            command @ (PlayerCommand::SelectPc { .. }
            | PlayerCommand::TogglePcSelection { .. }
            | PlayerCommand::SelectTacticalUnits { .. }
            | PlayerCommand::SelectPlannedShieldProtected { .. }
            | PlayerCommand::ClearTacticalSelection
            | PlayerCommand::UnselectAllPcs
            | PlayerCommand::StopRecordingMacro) => Some(command),
            other => {
                tracing::warn!(?other, "suppressed unsupported live Shift-click command");
                None
            }
        })
        .collect()
}

/// Whether a live or planned shield action is waiting for its protected-PC
/// click. In the original game, `true` means the
/// protectee is still being chosen, while `false` means the next click must be
/// the world-space danger point.
pub(crate) fn is_choosing_shield_protectee(
    engine: &Engine,
    local_seat: PlayerId,
    actor: EntityId,
    is_planning: bool,
) -> bool {
    if is_planning {
        engine
            .planned_shield_protected_for_seat(local_seat, actor)
            .is_none()
    } else {
        engine.shield().is_protected
    }
}

/// Portrait healing accepts PCs whose life is strictly between zero and full.
/// Share this gate between click handling and its Yes/No cursor feedback.
pub(crate) fn is_valid_heal_portrait_target(engine: &Engine, target: EntityId) -> bool {
    engine
        .get_entity(target)
        .and_then(|entity| entity.pc_data())
        .is_some_and(|pc| pc.life_points > 0 && pc.life_points < 100)
}

/// Original-game shield-portrait target gate. Shield targeting requires
/// one selected bearer, and cannot protect a selected, dead, or inactive PC.
pub(crate) fn is_valid_shield_portrait_protectee(
    engine: &Engine,
    local_seat: PlayerId,
    target: EntityId,
) -> bool {
    let selection = engine.hero_selection(local_seat);
    selection.len() == 1
        && !selection.contains(&target)
        && engine
            .get_entity(target)
            .is_some_and(|entity| entity.is_pc() && entity.is_active() && !entity.is_dead())
}

/// Resolve the portrait half of Shield/Big Shield's two-click protocol.
///
/// `None` leaves the portrait click to ordinary portrait selection. `Some`
/// means the armed action consumed it; an empty vector is an invalid target,
/// while a one-command vector records the protectee without launching a
/// shield sequence. Once the protectee has been selected, portraits cannot
/// supply the required world-space danger point.
pub(crate) fn resolve_shield_portrait_click(
    engine: &Engine,
    local_seat: PlayerId,
    actor: EntityId,
    target: EntityId,
    is_planning: bool,
) -> Option<Vec<PlayerCommand>> {
    let action = if is_planning {
        engine.planned_action_for_seat(local_seat)
    } else {
        engine.selected_action_for_seat(local_seat)
    };
    if !matches!(action, Action::Shield | Action::BigShield)
        || !engine.hero_selection(local_seat).contains(&actor)
    {
        return None;
    }

    let target_is_valid = is_valid_shield_portrait_protectee(engine, local_seat, target);
    if !is_choosing_shield_protectee(engine, local_seat, actor, is_planning) {
        // Original's portrait action handler relinquishes a valid portrait in
        // the danger-point phase. Ordinary portrait selection may proceed,
        // but it must never synthesize RaiseShield without a world point.
        return if target_is_valid {
            None
        } else {
            Some(Vec::new())
        };
    }
    if !target_is_valid {
        return Some(Vec::new());
    }

    let command = if is_planning {
        PlayerCommand::SelectPlannedShieldProtected {
            actor,
            protected_pc: target,
        }
    } else {
        PlayerCommand::ShieldSelectProtected {
            actor,
            protected_pc: target,
        }
    };
    Some(vec![command])
}

fn selected_tactical_formation(engine: &Engine, soldiers: &[EntityId]) -> TacticalFormation {
    soldiers
        .iter()
        .find_map(|soldier| engine.tactical_order(*soldier).map(|order| order.formation))
        .unwrap_or_default()
}

/// Update the click-and-drag target cache after a successful action-mode
/// target lookup.
///
/// When a focusable victim is found, record it in `element_old_click`
/// (for double-click replay) and `target_drag` (so a follow-up drag
/// over the same victim doesn't retarget on every frame).  Once the
/// per-frame drag arms (Apple/Stone/Hit/Heal/Strangle/Lever) land,
/// this cache becomes the tripwire for the `ignore_next_left_click` /
/// `ignore_next_drag` handshake.
fn cache_click_and_drag_target(host: &mut Host, target_id: EntityId) {
    host.frontend.input.gestures.element_old_click = Some(target_id);
    host.frontend.input.gestures.target_drag = Some(target_id);
}

/// Resolve action-specific left-click (bow, hit, heal, etc.).
use actions::resolve_action_left_click;
mod actions;

/// Resolve the action-drag arm — fires the per-action launcher on the
/// first drag frame where a focusable target is acquired (Apple /
/// Stone / Hit / Hit-Hard / Heal / Lever / Strangle).
///
/// Mutates `host.frontend.input.gestures.target_drag` for click-and-drag dedup, and sets
/// `ignore_next_left_click` when a new drag target is acquired so the
/// MouseUp doesn't re-fire the command.  When a macro is recording,
/// additionally latches `ignore_next_drag` so subsequent drag frames
/// don't re-record the same action.
pub fn resolve_action_drag(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    map_pt: MapPoint,
) -> Vec<PlayerCommand> {
    let local_seat = host.transport.local_seat();
    if host.frontend.input.ignore_next_drag() {
        return vec![];
    }
    // Swordfighting PCs feed the mouse-way gesture recognizer on
    // drag, not the action arm.  The drag path already filters in the
    // caller; this defensive check is a safety net.
    if is_selected_unit_swordfighting(&engine.presentation_view(), local_seat) {
        return vec![];
    }

    let selected_action = engine.selected_action_for_seat(local_seat);
    let focus = match selected_action {
        Action::Apple => Focus::Apple,
        Action::Stone => Focus::Stone,
        Action::Hit | Action::HitHard => Focus::Hit,
        Action::Heal => Focus::Heal,
        Action::Lever => Focus::Lever,
        Action::Strangle => Focus::Strangle,
        _ => return vec![],
    };

    let Some(pc_id) = engine.hero_selection(local_seat).first().copied() else {
        return vec![];
    };
    let is_recording = engine.is_recording_macro();
    let valid_trajectory = host.frontend.trajectory_preview().is_valid();

    // Apple / Stone gate on a valid arc.
    if matches!(selected_action, Action::Apple | Action::Stone)
        && !valid_trajectory
        && !is_recording
    {
        host.frontend.input.gestures.target_drag = None;
        return vec![];
    }

    let target = match engine.find_focusable_entity(
        assets,
        &host.frontend.presentation.draw_order.ids,
        map_pt,
        focus,
    ) {
        Some(t) => t,
        None => {
            // No focus found: clear `target_drag` so a subsequent
            // re-hover re-fires the arm.
            host.frontend.input.gestures.target_drag = None;
            return vec![];
        }
    };

    // Dedup: when the same target is still under the cursor, skip — the
    // action only fires on the first frame a focus is acquired or when
    // it changes.
    if host.frontend.input.gestures.target_drag == Some(target) {
        return vec![];
    }

    host.frontend.input.gestures.target_drag = Some(target);
    host.frontend.input.gestures.element_old_click = Some(target);
    // Block the MouseUp click so it doesn't double-fire, and (when
    // recording a macro) block further drag frames so the macro stream
    // captures exactly one action step.
    host.frontend.input.drag_action_dispatched(is_recording);

    drag_interaction_commands(pc_id, target, selected_action, is_recording)
}

/// Construct the walking interaction and its action-specific completion policy.
fn drag_interaction_commands(
    actor: EntityId,
    target: EntityId,
    action: Action,
    is_recording: bool,
) -> Vec<PlayerCommand> {
    let (command, completion) = match action {
        // Thrown distractions stay armed outside macro recording.
        Action::Apple => (
            Command::ThrowApple,
            is_recording.then_some(PlayerCommand::StopRecordingMacro),
        ),
        Action::Stone => (
            Command::ThrowStone,
            is_recording.then_some(PlayerCommand::StopRecordingMacro),
        ),
        Action::Hit | Action::HitHard => (Command::HitCmd, None),
        Action::Strangle => (Command::StrangleCmd, None),
        Action::Heal | Action::Lever => (
            if action == Action::Heal {
                Command::HealCmd
            } else {
                Command::UseLever
            },
            Some(if is_recording {
                PlayerCommand::StopRecordingMacro
            } else {
                PlayerCommand::UnselectAllActions
            }),
        ),
        _ => panic!("unsupported drag interaction action: {action:?}"),
    };
    let mut commands = vec![PlayerCommand::LaunchInteraction {
        actor,
        target,
        command,
        running: false,
    }];
    commands.extend(completion);
    commands
}

/// Resolve double-click repeat-interact on a cached target.
fn resolve_double_click_repeat(
    engine: &Engine,
    assets: &LevelAssets,
    cached_target: EntityId,
    local_seat: PlayerId,
) -> Vec<PlayerCommand> {
    use robin_engine::element::Entity;

    #[derive(PartialEq)]
    enum Kind {
        Soldier,
        Civilian,
        Object,
    }
    let kind = match engine.get_entity(cached_target) {
        Some(Entity::Soldier(_)) => Kind::Soldier,
        Some(Entity::Civilian(_)) => Kind::Civilian,
        // Object / Net branches: a double-click on an in-flight
        // pickup-target accelerates the seek (MakePcFast) instead of
        // launching a fresh Take sequence (only when not recording a
        // macro).
        Some(Entity::Bonus(_) | Entity::Scroll(_) | Entity::Projectile(_) | Entity::Net(_)) => {
            Kind::Object
        }
        Some(Entity::Pc(_)) => return vec![], // no-op
        Some(_) | None => return vec![],
    };

    let selected_pcs = engine.hero_selection(local_seat);
    let selected_combatants = selected_units(engine, local_seat);
    // Tactical allies participate in combat, but civilian and pickup interactions
    // require a selected hero. An allied-only selection is valid, not a hero.
    if kind != Kind::Soldier && selected_pcs.is_empty() {
        return vec![];
    }

    match kind {
        Kind::Soldier => {
            // Double-click on an enemy accelerates the in-flight seek
            // rather than issuing a new one.  Only while recording a
            // macro does it fall through to a fresh seek with the
            // running gait.
            if engine.is_recording_macro() {
                selected_combatants
                    .map(|pc_id| PlayerCommand::EnterSwordfight {
                        actor: pc_id,
                        target: cached_target,
                        running: true,
                    })
                    .collect()
            } else {
                selected_combatants
                    .map(|pc_id| PlayerCommand::MakePcFast { pc_id })
                    .collect()
            }
        }
        Kind::Civilian => {
            let selected = selected_pcs;
            let pc_id = selected[0];
            let Some(cmd) = determine_use_command(engine, assets, pc_id, cached_target) else {
                return vec![];
            };
            selected
                .iter()
                .copied()
                .map(|pc_id| PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target: cached_target,
                    command: cmd,
                    running: false,
                })
                .collect()
        }
        Kind::Object => {
            let selected = selected_pcs;
            // Non-recording double-click: accelerate the in-flight
            // seek with MakePcFast.  Recording double-click falls
            // through to a fresh Take launched with the running gait
            // via the quick-action path in `LaunchInteraction`.
            if engine.is_recording_macro() {
                let pc_id = selected[0];
                let Some(cmd) = determine_use_command(engine, assets, pc_id, cached_target) else {
                    return vec![];
                };
                let launch_target = match cmd {
                    Command::Take => engine_api::coin_pickup_target(engine, cached_target),
                    _ => cached_target,
                };
                selected
                    .iter()
                    .copied()
                    .map(|pc_id| PlayerCommand::LaunchInteraction {
                        actor: pc_id,
                        target: launch_target,
                        command: cmd,
                        running: true,
                    })
                    .collect()
            } else {
                selected
                    .iter()
                    .copied()
                    .map(|pc_id| PlayerCommand::MakePcFast { pc_id })
                    .collect()
            }
        }
    }
}

// ─── Right-click resolution ─────────────────────────────────────────

/// Resolve a right-click into player commands.
pub fn resolve_right_click(engine: &Engine, local_seat: PlayerId) -> Vec<PlayerCommand> {
    let selected_combatants = selected_units(engine, local_seat);
    let clear_tactical = !engine.tactical_selection(local_seat).is_empty();
    let finish = |mut commands: Vec<PlayerCommand>| {
        if clear_tactical {
            commands.push(PlayerCommand::ClearTacticalSelection);
        }
        commands
    };

    // Swordfighting → parry. Building the commands also determines whether
    // this branch owns the click; no separate selection scan is needed.
    let mut parries = Vec::new();
    for pc_id in selected_combatants {
        let is_fighting = engine
            .get_entity(pc_id)
            .and_then(|e| e.human_data())
            .is_some_and(|h| !h.opponents.is_empty());
        if is_fighting {
            parries.push(PlayerCommand::LaunchSelfAbility {
                actor: pc_id,
                command: Command::ParrySword,
            });
        }
    }
    if !parries.is_empty() {
        return finish(parries);
    }

    let selected = engine.hero_selection(local_seat);
    let Some(&first_selected) = selected.first() else {
        return finish(Vec::new());
    };

    // Action selected → cancel
    let selected_action = engine.selected_action_for_seat(local_seat);
    match selected_action {
        Action::NoAction => {}
        Action::Strangle
        | Action::Heal
        | Action::Hit
        | Action::HitHard
        | Action::HelpToClimb
        | Action::Beggar => {
            let mut cmds = resolve_right_click_stop(engine, local_seat);
            cmds.push(PlayerCommand::UnselectAllActions);
            return finish(cmds);
        }
        Action::Bow => {
            // Right-click with Bow armed clears the queued shoot list
            // first — if anything was queued, drain it and keep Bow
            // armed.  Only an empty queue falls through to deselecting
            // the action.
            if engine.pc_has_pending_shoot_bow(first_selected) {
                return finish(vec![PlayerCommand::ClearShootList {
                    pc_id: first_selected,
                }]);
            }
            return finish(vec![PlayerCommand::UnselectAllActions]);
        }
        Action::Shield | Action::BigShield => {
            // Splits on action state:
            //   MovingShield → Stop (motion-cancel)
            //   HoldingShield | ParryingShield → LowerShield
            let mut cmds = Vec::new();
            for &pc_id in selected {
                let action_state = engine
                    .get_entity(pc_id)
                    .and_then(|e| e.actor_data())
                    .map(|a| a.action_state)
                    .unwrap_or(ActionState::Waiting);
                match action_state {
                    ActionState::MovingShield => {
                        cmds.push(PlayerCommand::StopPc { pc_id });
                    }
                    ActionState::HoldingShield | ActionState::ParryingShield => {
                        cmds.push(PlayerCommand::LaunchSelfAbility {
                            actor: pc_id,
                            command: Command::LowerShield,
                        });
                    }
                    _ => {}
                }
            }
            cmds.push(PlayerCommand::UnselectAllActions);
            return finish(cmds);
        }
        _ => {
            return finish(vec![PlayerCommand::UnselectAllActions]);
        }
    }

    // NoAction → posture-based stop
    finish(resolve_right_click_stop(engine, local_seat))
}

/// Resolve the posture-based stop for each selected PC.
///
/// For the corpse-carry / shoulders-carry / helping-climb postures
/// there's a two-way split: if the PC is in motion AND the sector is
/// not a building, just stop the current move; otherwise run the
/// pose-exit command (and for HelpingToClimb / CarryingOnShoulders the
/// else-branch is disabled, so the right-click is ignored).
fn resolve_right_click_stop(engine: &Engine, local_seat: PlayerId) -> Vec<PlayerCommand> {
    let mut cmds = Vec::new();
    for &pc_id in engine.hero_selection(local_seat) {
        let (posture, action_state, in_motion, sector_is_building) = match engine.get_entity(pc_id)
        {
            Some(e) => {
                let posture = e.element_data().posture();
                let action_state = e
                    .actor_data()
                    .map(|a| a.action_state)
                    .unwrap_or(ActionState::Waiting);
                // `is_in_motion` compares sprite goal vs. current map
                // position OR consults map-movement state. Reading
                // `action_state.is_moving()` would miss MovingSword /
                // MovingShield translation cases.
                let in_motion = e.is_in_motion();
                let sector_is_building = e
                    .element_data()
                    .sector()
                    .and_then(|s| {
                        let sn = SectorNumber::new(i16::from(s));
                        let idx = *engine.fast_grid().level.sector_number_map.get(&sn)?;
                        engine.fast_grid().level.sectors.get(idx)
                    })
                    .is_some_and(|s| s.sector_type.is_building());
                (posture, action_state, in_motion, sector_is_building)
            }
            None => continue,
        };

        // For the CarryingCorpse / OnShoulders / HelpingToClimb /
        // CarryingOnShoulders arms: when the PC is walking and not
        // inside a building, the right-click stops them instead of
        // triggering the posture-specific cancel.

        match posture {
            Posture::CarryingCorpse => {
                if in_motion && !sector_is_building {
                    cmds.push(PlayerCommand::StopPc { pc_id });
                } else {
                    cmds.push(PlayerCommand::LaunchSelfAbility {
                        actor: pc_id,
                        command: Command::DropCorpse,
                    });
                }
            }
            Posture::OnShoulders => {
                if in_motion && !sector_is_building {
                    cmds.push(PlayerCommand::StopPc { pc_id });
                } else {
                    cmds.push(PlayerCommand::LaunchSelfAbility {
                        actor: pc_id,
                        command: Command::ClimbDownFromShoulders,
                    });
                }
            }
            Posture::HelpingToClimb | Posture::CarryingOnShoulders => {
                // Only stop when in motion on open ground; the
                // `LeaveHelpingClimb` exit branch is disabled, so a
                // right-click on an idle helper or one inside a
                // building is ignored entirely.
                if in_motion && !sector_is_building {
                    cmds.push(PlayerCommand::StopPc { pc_id });
                }
            }
            Posture::Upright => match action_state {
                ActionState::HoldingShield | ActionState::MovingShield => {
                    cmds.push(PlayerCommand::LaunchSelfAbility {
                        actor: pc_id,
                        command: Command::LowerShield,
                    });
                }
                ActionState::AimingWithBow
                | ActionState::AimingWithBowUp
                | ActionState::AimingWithBowDown => {
                    // Don't interrupt bow aim
                }
                _ => {
                    cmds.push(PlayerCommand::StopPc { pc_id });
                }
            },
            _ => {
                cmds.push(PlayerCommand::StopPc { pc_id });
            }
        }
    }
    cmds
}

// ─── Swordfight gesture resolution ──────────────────────────────────

/// Resolve a swordfight mouse gesture into commands.
///
/// Returns commands if the gesture was consumed, empty if not.
/// "Consumed" is true for Attempt (unrecognised gesture), recognised
/// strikes, and clicks on sword-focusable targets.
pub fn resolve_swordfight(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    map_pt: MapPoint,
    is_left_button: bool,
) -> Vec<PlayerCommand> {
    let local_seat = host.transport.local_seat();
    if !is_selected_unit_swordfighting(&engine.presentation_view(), local_seat) {
        return vec![];
    }

    let mut cmds = Vec::new();
    let mut consumed = false;
    let mut feedback_recorded = false;
    let combat_rules = engine.sim_config();

    for pc_id in selected_units(engine, local_seat) {
        let Some(entity) = engine.get_entity(pc_id) else {
            continue;
        };
        let Some(human) = entity.human_data() else {
            continue;
        };

        if human.opponents.is_empty() {
            // Non-swordfighting PC: click on sword target = engage.
            // The seek walks (running=false) since this is the single-
            // click path.
            if is_left_button
                && let Some(target_id) = engine.find_focusable_entity(
                    assets,
                    &host.frontend.presentation.draw_order.ids,
                    map_pt,
                    Focus::Sword,
                )
            {
                host.frontend.input.gestures.element_old_click = Some(target_id);
                cmds.push(PlayerCommand::EnterSwordfight {
                    actor: pc_id,
                    target: target_id,
                    running: false,
                });
            }
            continue;
        }

        let element = entity.element_data();
        let direction = crate::shadow_polygon::sector_to_direction(element.direction());
        let facing_dir =
            robin_engine::coordinates::ScreenVec::new(direction[0], direction[1] * ASPECT_RATIO);
        let pc_screen = host
            .frontend
            .viewport
            .map_to_screen_unclamped(element.position_map());
        let evaluation = host.frontend.mouse_way().evaluate_detailed(
            pc_screen,
            facing_dir,
            combat_rules.more_combat_gestures,
        );
        let pattern = evaluation.pattern;
        tracing::trace!(
            "resolve_swordfight: pc={pc_id:?} pattern={pattern:?} quality={} similarity={} mw_pts={}",
            evaluation.quality.permille(),
            evaluation.similarity,
            host.frontend.mouse_way().len(),
        );

        if host
            .frontend
            .preferences()
            .gameplay_config()
            .combat_gesture_coach
            && !feedback_recorded
            && !matches!(pattern, MouseWayPattern::None)
            && let Some(bounds) = host.frontend.mouse_way().bounds()
        {
            let feedback_pattern = if combat_rules.more_combat_gestures
                && matches!(pattern, MouseWayPattern::Attempt)
            {
                evaluation
                    .nearest_composite
                    .map(MouseWayPattern::Composite)
                    .unwrap_or(pattern)
            } else {
                pattern
            };
            host.frontend
                .set_gesture_coach_feedback(Some(GestureCoachFeedback {
                    pattern: feedback_pattern,
                    quality: evaluation.quality,
                    bounds,
                    template_rotation: crate::mouse_way::display_template_rotation(
                        feedback_pattern,
                        facing_dir,
                    ),
                    created_at_ms: crate::window::process_uptime_ms(),
                }));
            feedback_recorded = true;
        }

        match pattern {
            MouseWayPattern::Attempt => {
                // Unrecognised gesture, but the swordfight path
                // claims it as consumed.
                consumed = true;
            }
            MouseWayPattern::None => {
                if !is_left_button {
                    continue;
                }
                let Some(target_id) = engine.find_focusable_entity(
                    assets,
                    &host.frontend.presentation.draw_order.ids,
                    map_pt,
                    Focus::Sword,
                ) else {
                    continue;
                };

                let already_opponent = human.opponents.contains(&target_id);

                host.frontend.input.gestures.element_old_click = Some(target_id);
                if already_opponent {
                    let with_seek = sword_strike_target_is_in_same_sector(engine, pc_id, target_id);
                    cmds.push(PlayerCommand::SwordStrikeCmd {
                        actor: pc_id,
                        target: target_id,
                        command: Command::SwordstrikeThrustA,
                        composite: None,
                        gesture_quality: GestureQuality::PERFECT,
                        with_seek,
                        seek_distance: with_seek.then(|| {
                            resolved_sword_seek_distance(
                                engine,
                                assets,
                                pc_id,
                                Command::SwordstrikeThrustA,
                                true,
                            )
                        }),
                    });
                } else {
                    // Same walking-seek behaviour as the
                    // non-swordfighting branch above.
                    cmds.push(PlayerCommand::EnterSwordfight {
                        actor: pc_id,
                        target: target_id,
                        running: false,
                    });
                }
            }
            recognised => {
                let Some(strike_cmd) = pattern_to_command(recognised) else {
                    continue;
                };
                let composite = pattern_to_composite(recognised);
                let target_id = *human
                    .opponents
                    .first()
                    .expect("engaged swordfighter must retain its principal opponent");

                let with_seek = command_supports_sword_seek(strike_cmd)
                    && sword_strike_target_is_in_same_sector(engine, pc_id, target_id);

                cmds.push(PlayerCommand::SwordStrikeCmd {
                    actor: pc_id,
                    target: target_id,
                    command: strike_cmd,
                    composite,
                    gesture_quality: if combat_rules.gesture_quality_damage {
                        evaluation.quality
                    } else {
                        GestureQuality::PERFECT
                    },
                    with_seek,
                    seek_distance: with_seek.then(|| {
                        resolved_sword_seek_distance(engine, assets, pc_id, strike_cmd, false)
                    }),
                });
            }
        }
    }

    // If Attempt was seen but no commands were generated, we still
    // need to signal "consumed" so the caller doesn't fall through to
    // the no-action path.
    if cmds.is_empty() && consumed {
        cmds.push(PlayerCommand::Noop);
    }
    cmds
}

fn selected_units(engine: &Engine, local_seat: PlayerId) -> impl Iterator<Item = EntityId> + '_ {
    engine
        .hero_selection(local_seat)
        .iter()
        .chain(engine.tactical_selection(local_seat).iter())
        .copied()
}

/// True when any selected hero or controllable allied soldier is currently
/// engaged in melee. The original engine query intentionally remains PC-only;
/// UI input uses this broader query for the optional allied-control layer.
pub fn is_selected_unit_swordfighting(
    engine: &engine_api::PresentationView<'_>,
    local_seat: PlayerId,
) -> bool {
    first_selected_swordfighter(engine, local_seat).is_some()
}

/// Borrow the first engaged selection in hero-then-tactical order.
pub(crate) fn first_selected_swordfighter<'world>(
    engine: &engine_api::PresentationView<'world>,
    local_seat: PlayerId,
) -> Option<&'world Entity> {
    engine
        .hero_selection(local_seat)
        .iter()
        .chain(engine.tactical_selection(local_seat))
        .filter_map(|&id| engine.get_entity(id))
        .find(|entity| {
            entity
                .human_data()
                .is_some_and(|human| !human.opponents.is_empty())
        })
}

fn sword_strike_target_is_in_same_sector(
    engine: &Engine,
    actor: EntityId,
    target: EntityId,
) -> bool {
    let actor_sector = engine
        .get_entity(actor)
        .unwrap_or_else(|| {
            panic!("sword-strike actor {actor:?} disappeared during input resolution")
        })
        .element_data()
        .sector();
    let target_sector = engine
        .get_entity(target)
        .unwrap_or_else(|| {
            panic!("sword-strike target {target:?} disappeared during input resolution")
        })
        .element_data()
        .sector();
    actor_sector == target_sector
}

/// Resolve the exact swordfight tolerance used by the original game.
/// The no-gesture click path deliberately uses the weapon's generic maximum
/// (the end-of-strike marker in the original game), while recognised A-E gestures use the
/// selected thrust's maximum. Keeping the resulting scalar in the resolved
/// command makes macro and rollback replay independent of later profile state.
fn resolved_sword_seek_distance(
    engine: &Engine,
    assets: &LevelAssets,
    actor_id: EntityId,
    command: Command,
    generic_maximum: bool,
) -> f32 {
    let entity = engine
        .get_entity(actor_id)
        .unwrap_or_else(|| panic!("sword seek owner {actor_id:?} disappeared"));
    let weapon_id = match entity {
        Entity::Pc(pc) => {
            assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| panic!("sword seek owner {actor_id:?} has no character profile"))
                .hth_weapon_id
        }
        Entity::Soldier(soldier) => {
            assets
                .profile_manager
                .get_soldier(soldier.soldier.soldier_profile_index)
                .unwrap_or_else(|| panic!("sword seek owner {actor_id:?} has no soldier profile"))
                .hth_weapon_id
        }
        _ => panic!("sword seek owner {actor_id:?} is not a PC or soldier"),
    };
    let weapon = assets
        .profile_manager
        .get_hth_weapon(weapon_id)
        .unwrap_or_else(|| panic!("sword seek owner {actor_id:?} has no HtH weapon profile"));
    sword_seek_distance_for_weapon(weapon, command, generic_maximum)
}

fn sword_seek_distance_for_weapon(
    weapon: &robin_engine::profiles::HtHWeaponProfile,
    command: Command,
    generic_maximum: bool,
) -> f32 {
    let maximum = if generic_maximum {
        weapon.distance[robin_engine::weapons::WeaponDistance::Maximal as usize]
    } else {
        let strike = robin_engine::weapons::SwordStrike::from_command(command)
            .unwrap_or_else(|| panic!("seek command {command:?} is not a normal sword strike"));
        weapon.thrusts[strike as usize].maximal_distance
    };
    0.9 * maximum as f32
}

// ─── Helpers (read-only) ────────────────────────────────────────────

/// Whether `target_id` is an NPC (Soldier / Civilian) with a currently
/// attached dialog scroll.  `Focus::Use` already gates the
/// `!is_out_of_order` precondition, so a direct `attached_scroll` read
/// on the entity is sufficient here.
fn is_target_scroll_attached_npc(engine: &Engine, target_id: EntityId) -> bool {
    match engine.get_entity(target_id) {
        Some(Entity::Soldier(s)) => s.npc.attached_scroll.is_some(),
        Some(Entity::Civilian(c)) => c.npc.attached_scroll.is_some(),
        _ => false,
    }
}

/// Determine which Use command to launch on a target entity.
fn determine_use_command(
    engine: &Engine,
    assets: &LevelAssets,
    pc_id: EntityId,
    target_id: EntityId,
) -> Option<Command> {
    let entity = engine.get_entity(target_id)?;

    // Targets are not humans: their Element::is_dead default is true. Resolve
    // their authored action filter before the corpse-search fallback below,
    // exactly like target command selection in the original game. Otherwise
    // every target click is incorrectly dispatched as SearchCmd.
    if let Entity::Target(target) = entity {
        let pc_has_search = engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            engine_profiles::Action::Search,
        );
        let pc_has_lever = engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            engine_profiles::Action::Lever,
        );
        let pc_is_vip = engine
            .get_entity(pc_id)
            .is_some_and(|pc| engine.is_entity_vip(assets, pc));
        return engine_api::target_interaction::target_use_command(
            target.target.action_filter,
            pc_has_search,
            pc_has_lever,
            pc_is_vip,
        );
    }

    // Object-class targets (Net, Bonus, Scroll, landed Projectile).
    // The engine-side `object_pickup_command` is the authoritative
    // implementation; this just calls straight through.
    if let Some(cmd) = engine_api::object_pickup_command(engine, assets, target_id, pc_id) {
        return Some(cmd);
    }

    // Scroll / Bonus / landed Projectile pickup — dispatches the Take
    // sequence per object type after the seek + Taking animation
    // completes.  `IsFocusable(Focus::Use)` already gated the status /
    // focus checks.
    if let Entity::Scroll(_) = entity {
        return Some(Command::Take);
    }
    if let Entity::Bonus(_) = entity {
        return Some(Command::Take);
    }
    if let Entity::Projectile(p) = entity
        && !p.projectile.flying
    {
        return Some(Command::Take);
    }

    let is_dead = entity.is_dead();
    let posture = entity.element_data().posture();
    let is_unconscious = entity.human_data().is_some_and(|h| h.unconscious);
    let is_tied = posture == Posture::Tied;

    // PC override fires before the human fallback.  When the target
    // PC is in HelpingToClimb posture and the selector PC has Jump,
    // dispatch the climb-up-on-shoulders sequence.
    // `is_entity_focusable(Focus::Use)` already gates the cursor on
    // `posture == HelpingToClimb && has_jump && !selector_swordfighting`,
    // so this arm just produces the matching Command.
    if matches!(entity, Entity::Pc(_)) && posture == Posture::HelpingToClimb {
        if engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            engine_profiles::Action::Jump,
        ) {
            return Some(Command::ClimbUpOnShoulders);
        }
        return None;
    }

    // Pay beggar.  When ransom < BEGGAR_SALARY the click silently
    // no-ops, even though the focus and cursor still light up (the
    // PAY_NO cursor variant).  The ransom check therefore lives on
    // the click side.
    if !is_dead
        && !is_unconscious
        && posture != Posture::Carried
        && matches!(entity, Entity::Civilian(c)
            if c.civilian.cached_civilian_type == robin_engine::profiles::CivilianType::Beggar
                && c.npc.attached_scroll.is_none())
    {
        let ransom = engine
            .campaign()
            .get_value(engine_campaign::CampaignValue::Ransom);
        if ransom >= engine_api::BEGGAR_SALARY {
            return Some(Command::Pay);
        }
        return None;
    }

    if engine.sim_config().enable_unbinding && is_tied && entity.is_npc() && !is_dead {
        let npc_money = match entity {
            Entity::Soldier(s) => s.npc.money,
            Entity::Civilian(c) => c.npc.money,
            _ => unreachable!("NPC human interaction target must be soldier or civilian"),
        };
        if npc_money != 0
            && engine.selected_pc_has_contextual_action(
                assets,
                Some(pc_id),
                engine_profiles::Action::Search,
            )
            && (!engine.is_entity_vip(assets, entity)
                || engine
                    .get_entity(pc_id)
                    .and_then(|selected| selected.pc_data())
                    .is_some_and(|pc| pc.robin))
        {
            return Some(Command::SearchCmd);
        }
        if engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            engine_profiles::Action::Tie,
        ) {
            return Some(Command::Untie);
        }
    }

    if is_dead {
        return Some(Command::SearchCmd);
    }
    if !is_dead && !is_unconscious && posture == Posture::Lying {
        return Some(Command::SearchCmd);
    }

    // Wake-Up arm.
    if is_unconscious
        && engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            engine_profiles::Action::Resuscitate,
        )
    {
        let selector_camp = engine
            .get_entity(pc_id)
            .unwrap_or_else(|| panic!("selected PC {pc_id:?} disappeared during WakeUp dispatch"))
            .camp();
        if entity.is_human() && engine.camps_are_allied(entity.camp(), selector_camp) {
            return Some(Command::WakeUp);
        }
    }

    // Take-Corpse arm before Tie, gated on carry-ability and not-heavy.
    if (is_unconscious || is_dead)
        && posture != Posture::Carried
        && !is_tied
        && engine.selected_pc_can_carry(assets, Some(pc_id))
    {
        let is_heavy = match entity {
            Entity::Soldier(s) => {
                assets
                    .profile_manager
                    .get_soldier(s.soldier.soldier_profile_index)
                    .expect("soldier corpse must reference an admitted soldier profile")
                    .heavy
            }
            _ => false,
        };
        if !is_heavy {
            return Some(Command::TakeCorpse);
        }
    }

    // Tie arm — gated on the selector having the Tie action.
    if is_unconscious
        && !is_tied
        && posture != Posture::Carried
        && engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            engine_profiles::Action::Tie,
        )
    {
        return Some(Command::TieCmd);
    }
    None
}

fn pattern_to_command(pattern: MouseWayPattern) -> Option<Command> {
    Some(match pattern {
        MouseWayPattern::ThrustA => Command::SwordstrikeThrustA,
        MouseWayPattern::ThrustB => Command::SwordstrikeThrustB,
        MouseWayPattern::ThrustC => Command::SwordstrikeThrustC,
        MouseWayPattern::ThrustD => Command::SwordstrikeThrustD,
        MouseWayPattern::ThrustE => Command::SwordstrikeThrustE,
        MouseWayPattern::ThrustF => Command::SwordstrikeThrustF,
        MouseWayPattern::ThrustG => Command::SwordstrikeThrustG,
        MouseWayPattern::ThrustH => Command::SwordstrikeThrustH,
        MouseWayPattern::ThrustI => Command::SwordstrikeThrustI,
        MouseWayPattern::Composite(technique) => technique.first_command(),
        MouseWayPattern::None | MouseWayPattern::Attempt => return None,
    })
}

fn pattern_to_composite(pattern: MouseWayPattern) -> Option<CompositeSwordTechnique> {
    match pattern {
        MouseWayPattern::Composite(technique) => Some(technique),
        _ => None,
    }
}

fn command_supports_sword_seek(command: Command) -> bool {
    matches!(
        command,
        Command::SwordstrikeThrustA
            | Command::SwordstrikeThrustB
            | Command::SwordstrikeThrustC
            | Command::SwordstrikeThrustD
            | Command::SwordstrikeThrustE
    )
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;

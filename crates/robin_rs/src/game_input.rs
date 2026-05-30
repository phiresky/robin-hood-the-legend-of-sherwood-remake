//! Input resolution — translates mouse clicks and keyboard actions into
//! [`PlayerCommand`]s by reading engine state immutably.
//!
//! The input system **never** holds `&mut Engine`.  It reads positions,
//! selection, entity state, and focus-test results through `&Engine`,
//! then returns a `Vec<PlayerCommand>` that the game session feeds to
//! `Engine::apply_commands`.  This clean separation is the foundation
//! for deterministic replay and rollback networking.

use crate::Host;
use crate::element::{ActionState, Camp, Command, Entity, EntityId, Focus, ListenPhase, Posture};
use crate::geo2d;
use crate::mouse_way::MouseWayPattern;
use crate::player_command::{PlayerCommand, PlayerId};
use crate::profiles::Action;
use crate::sector::SectorNumber;
use crate::sequence::Field;
use crate::shadow_polygon::ASPECT_RATIO;
use robin_engine::coordinates::MapPoint;
use robin_engine::engine::{Engine, LevelAssets};

// ─── Left-click resolution ──────────────────────────────────────────

/// Resolve a left-click at `map_pt` into player commands.
///
/// The engine is read-only; all mutations are expressed as commands.
pub fn resolve_left_click(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    map_pt: MapPoint,
    shift_held: bool,
    ctrl_held: bool,
    is_double: bool,
) -> Vec<PlayerCommand> {
    let local_seat = host.local_seat;
    let selected = engine.seat_selection(local_seat);
    let num_selected = selected.len();

    // Pre-process for double-clicks: when an action is armed and any
    // selected PC's profile lacks the action or has it disabled, abort
    // the entire double-click.  The prior single-click already fired
    // through the regular left-button arm, so suppressing the
    // double-click here prevents the action / cancel-arm / repeat-
    // interact branches below from dispatching a second time.  The
    // "no PCs selected" case is implicit: `selected_action_for_seat`
    // returns `NoAction` when nothing is selected, so the `!= NoAction`
    // gate already excludes it.
    if is_double && num_selected > 0 {
        let pending_action = engine.selected_action_for_seat(local_seat);
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
        let selected_action = engine.selected_action_for_seat(local_seat);
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
            );
            return cmds;
        } else if is_double && (engine.is_alt_effective(&host.input) || engine.locker_active()) {
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
        && let Some(cached) = host.input.element_old_click
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
    host.input.element_old_click = None;

    // No PCs selected: can only select a PC
    if num_selected == 0 {
        if let Some(pc_id) =
            engine.find_focusable_entity(assets, &host.draw_order.ids, map_pt, Focus::Select)
        {
            host.input.element_old_click = Some(pc_id);
            return vec![PlayerCommand::SelectPc {
                pc_id,
                append: shift_held,
            }];
        }
        host.input.element_old_click = None;
        return vec![];
    }

    let is_swordfighting = engine.is_seat_selection_swordfighting(local_seat);

    // Unselected PC → select it
    if let Some(pc_id) = engine.find_focusable_pc(assets, map_pt, Focus::Select)
        && !selected.contains(&pc_id)
    {
        host.input.element_old_click = Some(pc_id);
        if ctrl_held {
            return vec![PlayerCommand::TogglePcSelection { pc_id }];
        } else {
            return vec![PlayerCommand::SelectPc {
                pc_id,
                append: shift_held,
            }];
        }
    }

    // Use-focusable entity (search/carry/tie) — single selection, not swordfighting
    if !is_swordfighting
        && num_selected == 1
        && let Some(target_id) =
            engine.find_focusable_entity(assets, &host.draw_order.ids, map_pt, Focus::Use)
    {
        let pc_id = selected[0];
        // Scroll-attached NPC — opens a dialog.  Hands a composite
        // `LOCK_AI → turn ×2 → UNLOCK_AI → OPEN_SCROLL` sequence to the
        // PC; the engine-side helper `apply_scroll_read_with_seek`
        // builds the composite and prepends a seek as needed.
        if is_target_scroll_attached_npc(engine, target_id) {
            host.input.element_old_click = Some(target_id);
            return vec![PlayerCommand::LaunchScrollRead {
                actor: pc_id,
                target: target_id,
                running: is_double,
            }];
        }
        if let Some(cmd) = determine_use_command(engine, assets, pc_id, target_id) {
            host.input.element_old_click = Some(target_id);
            // A click on a coin forwards to the source purse when the
            // purse isn't yet taken — route the actual Take launch at
            // the purse id so its has-been-taken sweep fires on
            // arrival.
            let launch_target = match cmd {
                Command::Take => robin_engine::engine::coin_pickup_target(engine, target_id),
                _ => target_id,
            };
            // Net-specific double-click handling:
            //   double + !recording → MakePcFast; return (skip seek+take)
            //   double + recording  → run the seek+take with running gait
            // Only nets get this handling — other object types fall
            // through to the regular take path.
            let target_is_net = matches!(
                engine.get_entity(launch_target),
                Some(robin_engine::element::Entity::Net(_))
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
    if let Some(target_id) =
        engine.find_focusable_entity(assets, &host.draw_order.ids, map_pt, Focus::Sword)
    {
        host.input.element_old_click = Some(target_id);
        let target_is_soldier = engine
            .get_entity(target_id)
            .map(|e| e.is_soldier())
            .unwrap_or(false);
        let selected: Vec<EntityId> = selected.to_vec();
        let engagers: Vec<EntityId> = if target_is_soldier {
            selected
        } else {
            selected.into_iter().take(1).collect()
        };
        return engagers
            .into_iter()
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
    host.input.element_old_click = None;

    let is_recording = engine.is_recording_macro();

    // Non-recording double-click tail: MakePcFast, no fresh move.
    // The patch branch is intentionally ignored here — only the
    // recording arm honours the patch redirect.
    if is_double && !is_recording {
        if host.input.valid_position_for_move && host.input.selected_sector_idx.is_some() {
            return selected
                .iter()
                .map(|&pc_id| PlayerCommand::MakePcFast { pc_id })
                .collect();
        }
        return vec![];
    }

    // Single-click path, plus the recording double-click (which
    // follows the same patch→GroupMove / sector→GroupMove ordering
    // with the running gait on double-click).
    if let Some(patch_idx) = host.input.selected_patch_idx
        && let Some(patch) = engine
            .mission_script()
            .and_then(|s| s.game_host())
            .and_then(|h| h.patches.get(patch_idx as usize))
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
                        expression: robin_engine::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    }];
                }
                return vec![];
            }
        }
        let actors: Vec<EntityId> = selected.to_vec();
        let waypoint = geo2d::pt(patch.waypoint.x, patch.waypoint.y);
        // Mirror C++'s `update_mouse` substitution
        // (`mpSelectedSector = mFastGrid.GetSector(patch.sector)` /
        // `muwSelectedLayer = patch.layer`): pass the patch's
        // proto-loaded `(sector, layer)` to perform_group_move so the
        // move targets the patch's underlying motion area instead of
        // whatever the spatial lookup at the waypoint happens to find
        // (which can pick the wrong layer or no sector at all).
        let goal_override = Some((
            robin_engine::sector::SectorNumber::new(patch.sector as i16),
            patch.layer,
        ));
        return vec![PlayerCommand::GroupMove {
            actors,
            destination: waypoint.into(),
            running: is_double,
            show_marker: false,
            goal_override,
        }];
    }

    // Sector-click branch — gated on both predicates.
    if !(host.input.valid_position_for_move && host.input.selected_sector_idx.is_some()) {
        return vec![];
    }

    let actors: Vec<EntityId> = selected.to_vec();
    vec![PlayerCommand::GroupMove {
        actors,
        destination: map_pt.into(),
        running: is_double,
        show_marker: true,
        goal_override: None,
    }]
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
    host.input.element_old_click = Some(target_id);
    host.input.target_drag = Some(target_id);
}

/// Resolve action-specific left-click (bow, hit, heal, etc.).
fn resolve_action_left_click(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    map_pt: MapPoint,
    local_seat: PlayerId,
    action: Action,
    is_double: bool,
) -> Vec<PlayerCommand> {
    let draw_order = &host.draw_order.ids;
    let pc_id = match engine.seat_selection(local_seat).first().copied() {
        Some(id) => id,
        None => return vec![],
    };
    let is_recording = engine.is_recording_macro();
    let valid_trajectory = host.valid_trajectory;
    let selected_layer = host.input.selected_layer;

    // 2D → 3D projection of the mouse-map point onto the topmost
    // projection-area surface, used by the Purse/Wasp/Net ground-
    // target arms so the recorded titbit and `*_TARGET` sequence field
    // both land on the real 3D point instead of `z=0`.
    let convert_to_3d = |pt: MapPoint| -> robin_engine::coordinates::WorldPoint3D {
        let p3d = engine.fast_grid().convert_2d_to_3d(
            pt,
            robin_engine::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
            engine.sight_obstacles(assets),
        );
        robin_engine::coordinates::WorldPoint3D::from(p3d)
    };

    // Post-launch tail for actions that deselect on launch (non-
    // recording) or stop macro recording (recording).
    let commit_tail = |is_recording: bool| -> PlayerCommand {
        if is_recording {
            PlayerCommand::StopRecordingMacro
        } else {
            PlayerCommand::UnselectAllActions
        }
    };

    match action {
        Action::Bow => {
            // Bow runs several validation steps before launching the
            // shoot sequence.
            //
            // 1. Climbing or inside a building → drop the click.
            //    Recording bypasses this gate so a macro can still be
            //    recorded while climbing / inside a building.
            if !is_recording && engine.is_climbing_or_inside_building(pc_id) {
                return vec![];
            }

            let Some(target_id) =
                engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Bow)
            else {
                tracing::info!(
                    ?pc_id,
                    mouse_x = map_pt.x,
                    mouse_y = map_pt.y,
                    "Bow click rejected: no bow-focus target"
                );
                return vec![];
            };

            // 2. NPC target filter: VIP or civilian → drop.
            //    Applied in both the record and non-record branches.
            //    The record branch additionally gates on the target
            //    not being blipped, which we conservatively ignore —
            //    the downstream blipped check already makes the shot
            //    a no-op on replay.
            if let Some(target) = engine.get_entity(target_id)
                && target.is_npc()
                && (target.is_civilian() || engine.is_entity_vip(assets, target))
            {
                tracing::info!(
                    ?pc_id,
                    ?target_id,
                    "Bow click rejected: civilian or VIP target"
                );
                return vec![];
            }

            // 3. Shooter posture guard: AnonymousArcher (archers'
            //    contest) → hero speech + drop the click.  Only
            //    applied in the non-record branch.
            let Some(pc_entity) = engine.get_entity(pc_id) else {
                tracing::warn!(
                    ?pc_id,
                    ?target_id,
                    "Bow click rejected: selected PC entity missing"
                );
                return vec![];
            };
            let archer_posture = pc_entity.element_data().posture;
            if !is_recording && archer_posture == Posture::AnonymousArcher {
                tracing::info!(
                    ?pc_id,
                    ?target_id,
                    "Bow click rejected: anonymous archer cannot shoot"
                );
                return vec![PlayerCommand::HeroSpeak {
                    pc_id,
                    expression: robin_engine::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                }];
            }

            // 4. Range / LOS / shoot-mode validation.  Only applied
            //    in the non-record branch — a macro can be recorded
            //    even when the current LOS / range wouldn't allow the
            //    shot (replay re-evaluates).
            if !is_recording {
                let (bow_status, _shoot_mode) =
                    engine.can_shoot_with_bow_at(assets, pc_id, target_id);
                if bow_status != robin_engine::engine::input::BowTarget::Valid {
                    tracing::info!(
                        ?pc_id,
                        ?target_id,
                        ?bow_status,
                        "Bow click rejected: target not shootable"
                    );
                    return vec![];
                }
            }

            // Drag-target cache.  Bow goes through the shoot-list
            // queue rather than the click-and-drag arm, but caching
            // the target here lets the double-click-repeat path
            // (`resolve_double_click_repeat`) find a previous victim
            // to re-hit.
            cache_click_and_drag_target(host, target_id);
            let mut cmds = vec![PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: target_id,
                command: Command::ShootBow,
                running: is_double,
            }];
            tracing::info!(?pc_id, ?target_id, "Bow click dispatched ShootBow");
            // Stop macro recording after a recorded bow shot.  No
            // unselect trailer on the non-record branch — the launch
            // and return path leaves the action armed.
            if is_recording {
                cmds.push(PlayerCommand::StopRecordingMacro);
            }
            return cmds;
        }
        Action::Hit | Action::HitHard => {
            // The seek uses the running gait on double-click and
            // passes the no-transitions / seek-stop-NPC flags (handled
            // inside `apply_interaction_with_seek`).
            if let Some(target_id) =
                engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Hit)
            {
                // Cache the drag target so a follow-up double-click
                // repeats on the same victim.
                cache_click_and_drag_target(host, target_id);
                let mut cmds = vec![PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target: target_id,
                    command: Command::HitCmd,
                    running: is_double,
                }];
                // The pipeline records the quick-action step inside
                // `apply_command`; this trailer closes the recording.
                // No unselect — Hit deliberately stays armed after
                // launch.
                if is_recording {
                    cmds.push(PlayerCommand::StopRecordingMacro);
                }
                return cmds;
            }
        }
        Action::Apple => {
            // Drop the click on an invalid trajectory unless recording
            // a macro.
            if !valid_trajectory && !is_recording {
                return vec![];
            }
            if let Some(target_id) =
                engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Apple)
            {
                // Drag-target caching — used by the double-click
                // repeat path to replay the hit on a cached victim.
                cache_click_and_drag_target(host, target_id);
                // Apple throws have no seek: action_distance=0 same-
                // sector is already handled by
                // `apply_interaction_with_seek`, and the cross-sector
                // ranged bypass in that helper makes the throw fire
                // regardless of distance.
                let mut cmds = vec![PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target: target_id,
                    command: Command::ThrowApple,
                    running: false,
                }];
                // No UnselectAllActions here — Apple deliberately
                // stays armed after launch.  Only the recording branch
                // closes the macro.
                if is_recording {
                    cmds.push(PlayerCommand::StopRecordingMacro);
                }
                return cmds;
            }
        }
        Action::Stone => {
            // Trajectory gate — drop the click on an invalid arc
            // unless recording.
            if !valid_trajectory && !is_recording {
                return vec![];
            }
            if let Some(target_id) =
                engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Stone)
            {
                cache_click_and_drag_target(host, target_id);
                let mut cmds = vec![PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target: target_id,
                    command: Command::ThrowStone,
                    running: false,
                }];
                if is_recording {
                    cmds.push(PlayerCommand::StopRecordingMacro);
                }
                return cmds;
            }
        }
        Action::Heal => {
            // Heal unselects the action unconditionally after launch;
            // the recording path additionally closes the macro.  The
            // SEEK_IN_BUILDINGS flag is handled inside
            // `apply_interaction_with_seek`.
            if let Some(target_id) =
                engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Heal)
            {
                cache_click_and_drag_target(host, target_id);
                let mut cmds = vec![
                    PlayerCommand::LaunchInteraction {
                        actor: pc_id,
                        target: target_id,
                        command: Command::HealCmd,
                        running: is_double,
                    },
                    PlayerCommand::UnselectAllActions,
                ];
                if is_recording {
                    cmds.push(PlayerCommand::StopRecordingMacro);
                }
                return cmds;
            }
        }
        Action::Whistle => {
            // Launch the whistle ability then either deselect
            // the action or stop macro recording.
            return vec![
                PlayerCommand::LaunchSelfAbility {
                    actor: pc_id,
                    command: Command::WhistleCmd,
                },
                commit_tail(is_recording),
            ];
        }
        Action::Strangle => {
            // Run gait on double-click, no-transitions / seek-stop-NPC
            // flags (handled in `apply_interaction_with_seek`).
            if let Some(target_id) =
                engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Strangle)
            {
                cache_click_and_drag_target(host, target_id);
                let mut cmds = vec![PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target: target_id,
                    command: Command::StrangleCmd,
                    running: is_double,
                }];
                // Same pattern as Hit: close the macro on the
                // recording branch, no unselect — Strangle deliberately
                // stays armed.
                if is_recording {
                    cmds.push(PlayerCommand::StopRecordingMacro);
                }
                return cmds;
            }
        }
        Action::Net => {
            // Trajectory gate.
            if !valid_trajectory && !is_recording {
                return vec![];
            }
            let mut cmds = vec![PlayerCommand::LaunchGroundTarget {
                actor: pc_id,
                target_pos: convert_to_3d(map_pt),
                command: Command::ThrowNet,
                target_field: Field::NetTarget,
                // Net titbits are hard-coded to layer 0 regardless of
                // the currently selected layer.
                titbit_layer: 0,
            }];
            if is_recording {
                cmds.push(PlayerCommand::StopRecordingMacro);
            }
            return cmds;
        }
        Action::WaspNest => {
            // Trajectory gate.
            if !valid_trajectory && !is_recording {
                return vec![];
            }
            let mut cmds = vec![PlayerCommand::LaunchGroundTarget {
                actor: pc_id,
                target_pos: convert_to_3d(map_pt),
                command: Command::ThrowWaspNest,
                target_field: Field::WaspNestTarget,
                // Wasp nest titbits are placed on the currently
                // selected layer.
                titbit_layer: selected_layer,
            }];
            if is_recording {
                cmds.push(PlayerCommand::StopRecordingMacro);
            }
            return cmds;
        }
        Action::Purse => {
            // Trajectory gate — unconditional, applies even while
            // recording a macro.
            if !valid_trajectory {
                return vec![];
            }
            return vec![
                PlayerCommand::LaunchGroundTarget {
                    actor: pc_id,
                    target_pos: convert_to_3d(map_pt),
                    command: Command::ThrowPurse,
                    target_field: Field::PurseTarget,
                    // Place the titbit on the currently selected layer
                    // (same as Wasp) so it stays under the mouse when
                    // the selected layer differs from the PC's.
                    titbit_layer: selected_layer,
                },
                commit_tail(is_recording),
            ];
        }
        Action::Shield | Action::BigShield => {
            // Two-step protocol keyed on `ShieldState::is_protected`:
            //   first click  (is_protected=true):
            //     pick a focusable PC via `Focus::Shield`, store it in
            //     `protected_pc`, flip `is_protected = false`.
            //   second click (is_protected=false):
            //     read a 3D danger point from the click, flip
            //     `is_protected = true`, build
            //     `Seek(protected_pc, 50) → RaiseShield(DangerPoint=...)`,
            //     refresh the `DangerPoint` titbit, and deselect.
            //
            // `set_pc_action` resets the ShieldState when the action is
            // armed.
            let shield = engine.shield();
            if shield.is_protected {
                // First click — pick the PC to protect.  No sequence
                // is launched; returning empty here lets the click be
                // consumed without falling through to the GroupMove
                // tail of `resolve_left_click`.
                if let Some(target_id) = engine.find_focusable_pc(assets, map_pt, Focus::Shield) {
                    return vec![PlayerCommand::ShieldSelectProtected {
                        actor: pc_id,
                        protected_pc: target_id,
                    }];
                }
                return vec![];
            }
            // Second click — resolve the danger point and launch the
            // shield sequence.  The protected PC was stashed in the
            // first click.  If for some reason the invariant has been
            // broken (no protected PC stored), fall through to the
            // nothing-happens branch so the click is harmlessly
            // consumed.
            let Some(protected_pc) = shield.protected_pc else {
                return vec![];
            };
            return vec![
                PlayerCommand::RaiseShieldWithDanger {
                    actor: pc_id,
                    protected_pc,
                    danger_point: map_pt.into(),
                    danger_point_layer: selected_layer,
                },
                commit_tail(is_recording),
            ];
        }
        Action::Eat | Action::Guzzle => {
            return vec![
                PlayerCommand::LaunchSelfAbility {
                    actor: pc_id,
                    command: Command::EatCmd,
                },
                commit_tail(is_recording),
            ];
        }
        Action::Listen => {
            // Check disabled
            let disabled = engine
                .get_entity(pc_id)
                .and_then(|e| match e {
                    Entity::Pc(pc) => Some(&pc.pc),
                    _ => None,
                })
                .map(|pc| {
                    let i = Action::Listen as usize;
                    pc.disabled_actions.get(i).copied().unwrap_or(false)
                        || pc.disabled_actions_temp.get(i).copied().unwrap_or(false)
                })
                .unwrap_or(false);
            if disabled {
                return vec![]; // consumed but no command
            }

            let listen_phase = engine
                .get_entity(pc_id)
                .and_then(|e| e.actor_data())
                .map(|a| a.listen_phase)
                .unwrap_or(ListenPhase::Inactive);
            // Deliberate behaviour change: this code emits a toggle —
            // a click while listening emits `LeaveListen`, matching
            // player expectation from the HUD state, instead of
            // re-emitting `EnterListen` and relying on the ability to
            // short-circuit when already active.
            let cmd = match listen_phase {
                ListenPhase::Inactive => Command::EnterListen,
                ListenPhase::EnterTransition | ListenPhase::CountingDown => Command::LeaveListen,
                ListenPhase::ExitTransition => return vec![],
            };
            return vec![
                PlayerCommand::LaunchSelfAbility {
                    actor: pc_id,
                    command: cmd,
                },
                commit_tail(is_recording),
            ];
        }
        Action::Lever => {
            // Interaction on a focusable lever (FX target or hookable
            // mobile), followed by deselecting the action.
            if let Some(target_id) =
                engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Lever)
            {
                cache_click_and_drag_target(host, target_id);
                return vec![
                    PlayerCommand::LaunchInteraction {
                        actor: pc_id,
                        target: target_id,
                        command: Command::UseLever,
                        running: is_double,
                    },
                    commit_tail(is_recording),
                ];
            }
        }
        Action::Beggar => {
            // Posture-keyed arms:
            //   SimulatingBeggar + double + !recording → MakePcFast
            //   SimulatingBeggar + single              → fall through to walk
            //   default                                → EnterBeggar (+ deselect on double)
            let posture = engine
                .get_entity(pc_id)
                .map(|e| e.element_data().posture)
                .unwrap_or(Posture::Undefined);
            if posture == Posture::SimulatingBeggar {
                if is_double && !is_recording {
                    return vec![PlayerCommand::MakePcFast { pc_id }];
                }
                // Fall through to the generic no-action path (walk).
                return vec![];
            }
            // Default posture → EnterBeggar.  Deliberate divergence
            // from the reference: in the double-click non-recording
            // case the reference deselects without launching a
            // sequence; here we emit the launch unconditionally and
            // let stealth-transition validation reject it — the
            // observable delta is an extra posture-precondition
            // rejection in that one case.
            return vec![
                PlayerCommand::LaunchSelfAbility {
                    actor: pc_id,
                    command: Command::EnterBeggar,
                },
                commit_tail(is_recording),
            ];
        }
        Action::HelpToClimb => {
            // Posture branches:
            //   HelpingToClimb | CarryingOnShoulders → walk-through to
            //     click (no-action path), or MakePcFast on double-click.
            //   default → launch EnterHelpingClimb (+ deselect).
            let posture = engine
                .get_entity(pc_id)
                .map(|e| e.element_data().posture)
                .unwrap_or(Posture::Undefined);
            if matches!(
                posture,
                Posture::HelpingToClimb | Posture::CarryingOnShoulders
            ) {
                if is_double && !is_recording {
                    return vec![PlayerCommand::MakePcFast { pc_id }];
                }
                // Walk the carried partner to the clicked spot — fall
                // through to the generic no-action GroupMove path.
                return vec![];
            }
            return vec![
                PlayerCommand::LaunchSelfAbility {
                    actor: pc_id,
                    command: Command::EnterHelpingClimb,
                },
                commit_tail(is_recording),
            ];
        }
        Action::Ale => {
            // Build a seek with a post-seek `DropAle` element: walk or
            // run the selected PC to the cursor, then play the ale-
            // drop animation to materialise a bottle at the PC's feet.
            //
            // The `DropAleAt` command handler constructs the
            // Move → DropAle sequence; the engine tick's
            // `Command::DropAle` arm then spawns the bottle and
            // decrements `Action::Ale` ammo.
            if !engine.is_mouse_sector_valid_for_ground_target(map_pt.into()) {
                return vec![];
            }
            return vec![
                PlayerCommand::DropAleAt {
                    actor: pc_id,
                    target_pos: map_pt.into(),
                    running: is_double,
                },
                commit_tail(is_recording),
            ];
        }
        _ => {}
    }

    vec![]
}

/// Resolve the action-drag arm — fires the per-action launcher on the
/// first drag frame where a focusable target is acquired (Apple /
/// Stone / Hit / Hit-Hard / Heal / Lever / Strangle).
///
/// Mutates `host.input.target_drag` for click-and-drag dedup, and sets
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
    let local_seat = host.local_seat;
    if host.input.ignore_next_drag {
        return vec![];
    }
    // Swordfighting PCs feed the mouse-way gesture recognizer on
    // drag, not the action arm.  The drag path already filters in the
    // caller; this defensive check is a safety net.
    if engine.is_seat_selection_swordfighting(local_seat) {
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

    let Some(pc_id) = engine.seat_selection(local_seat).first().copied() else {
        return vec![];
    };
    let is_recording = engine.is_recording_macro();
    let valid_trajectory = host.valid_trajectory;

    // Apple / Stone gate on a valid arc.
    if matches!(selected_action, Action::Apple | Action::Stone)
        && !valid_trajectory
        && !is_recording
    {
        host.input.target_drag = None;
        return vec![];
    }

    let target = match engine.find_focusable_entity(assets, &host.draw_order.ids, map_pt, focus) {
        Some(t) => t,
        None => {
            // No focus found: clear `target_drag` so a subsequent
            // re-hover re-fires the arm.
            host.input.target_drag = None;
            return vec![];
        }
    };

    // Dedup: when the same target is still under the cursor, skip — the
    // action only fires on the first frame a focus is acquired or when
    // it changes.
    if host.input.target_drag == Some(target) {
        return vec![];
    }

    host.input.target_drag = Some(target);
    host.input.element_old_click = Some(target);
    // Block the MouseUp click so it doesn't double-fire, and (when
    // recording a macro) block further drag frames so the macro stream
    // captures exactly one action step.
    host.input.ignore_next_left_click = true;
    if is_recording {
        host.input.ignore_next_drag = true;
    }

    let commit_tail = |is_recording: bool| -> PlayerCommand {
        if is_recording {
            PlayerCommand::StopRecordingMacro
        } else {
            PlayerCommand::UnselectAllActions
        }
    };

    // Drag always uses the walking animation regardless of double-
    // click state.
    match selected_action {
        Action::Apple => {
            let mut cmds = vec![PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target,
                command: Command::ThrowApple,
                running: false,
            }];
            // No unselect — Apple deliberately stays armed.  Only the
            // recording branch closes the macro.
            if is_recording {
                cmds.push(PlayerCommand::StopRecordingMacro);
            }
            cmds
        }
        Action::Stone => {
            let mut cmds = vec![PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target,
                command: Command::ThrowStone,
                running: false,
            }];
            if is_recording {
                cmds.push(PlayerCommand::StopRecordingMacro);
            }
            cmds
        }
        Action::Hit | Action::HitHard => {
            vec![PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target,
                command: Command::HitCmd,
                running: false,
            }]
        }
        Action::Heal => {
            vec![
                PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target,
                    command: Command::HealCmd,
                    running: false,
                },
                commit_tail(is_recording),
            ]
        }
        Action::Lever => {
            vec![
                PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target,
                    command: Command::UseLever,
                    running: false,
                },
                commit_tail(is_recording),
            ]
        }
        Action::Strangle => {
            vec![PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target,
                command: Command::StrangleCmd,
                running: false,
            }]
        }
        _ => vec![],
    }
}

/// Resolve double-click repeat-interact on a cached target.
fn resolve_double_click_repeat(
    engine: &Engine,
    assets: &LevelAssets,
    cached_target: EntityId,
    local_seat: PlayerId,
) -> Vec<PlayerCommand> {
    use crate::element::Entity;

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

    let selected: Vec<EntityId> = engine.seat_selection(local_seat).to_vec();
    if selected.is_empty() {
        return vec![];
    }

    match kind {
        Kind::Soldier => {
            // Double-click on an enemy accelerates the in-flight seek
            // rather than issuing a new one.  Only while recording a
            // macro does it fall through to a fresh seek with the
            // running gait.
            if engine.is_recording_macro() {
                selected
                    .into_iter()
                    .map(|pc_id| PlayerCommand::EnterSwordfight {
                        actor: pc_id,
                        target: cached_target,
                        running: true,
                    })
                    .collect()
            } else {
                selected
                    .into_iter()
                    .map(|pc_id| PlayerCommand::MakePcFast { pc_id })
                    .collect()
            }
        }
        Kind::Civilian => {
            let pc_id = selected[0];
            let Some(cmd) = determine_use_command(engine, assets, pc_id, cached_target) else {
                return vec![];
            };
            selected
                .into_iter()
                .map(|pc_id| PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target: cached_target,
                    command: cmd,
                    running: false,
                })
                .collect()
        }
        Kind::Object => {
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
                    Command::Take => {
                        robin_engine::engine::coin_pickup_target(engine, cached_target)
                    }
                    _ => cached_target,
                };
                selected
                    .into_iter()
                    .map(|pc_id| PlayerCommand::LaunchInteraction {
                        actor: pc_id,
                        target: launch_target,
                        command: cmd,
                        running: true,
                    })
                    .collect()
            } else {
                selected
                    .into_iter()
                    .map(|pc_id| PlayerCommand::MakePcFast { pc_id })
                    .collect()
            }
        }
    }
}

// ─── Right-click resolution ─────────────────────────────────────────

/// Resolve a right-click into player commands.
pub fn resolve_right_click(engine: &Engine, local_seat: PlayerId) -> Vec<PlayerCommand> {
    let selected = engine.seat_selection(local_seat);
    if selected.is_empty() {
        return vec![];
    }

    // Swordfighting → parry
    if engine.is_seat_selection_swordfighting(local_seat) {
        let mut cmds = Vec::new();
        for &pc_id in selected {
            let is_fighting = engine
                .get_entity(pc_id)
                .and_then(|e| e.human_data())
                .is_some_and(|h| !h.opponents.is_empty());
            if is_fighting {
                cmds.push(PlayerCommand::LaunchSelfAbility {
                    actor: pc_id,
                    command: Command::ParrySword,
                });
            }
        }
        return cmds;
    }

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
            return cmds;
        }
        Action::Bow => {
            // Right-click with Bow armed clears the queued shoot list
            // first — if anything was queued, drain it and keep Bow
            // armed.  Only an empty queue falls through to deselecting
            // the action.
            let first = selected.first().copied();
            if let Some(pc_id) = first
                && engine.pc_has_pending_shoot_bow(pc_id)
            {
                return vec![PlayerCommand::ClearShootList { pc_id }];
            }
            return vec![PlayerCommand::UnselectAllActions];
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
            return cmds;
        }
        _ => {
            return vec![PlayerCommand::UnselectAllActions];
        }
    }

    // NoAction → posture-based stop
    resolve_right_click_stop(engine, local_seat)
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
    for &pc_id in engine.seat_selection(local_seat) {
        let (posture, action_state, in_motion, sector_is_building) = match engine.get_entity(pc_id)
        {
            Some(e) => {
                let posture = e.element_data().posture;
                let action_state = e
                    .actor_data()
                    .map(|a| a.action_state)
                    .unwrap_or(ActionState::Waiting);
                // `is_in_motion` compares sprite goal vs. current map
                // position OR consults `IsMovingMap`.  Reading
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
    let local_seat = host.local_seat;
    if !engine.is_seat_selection_swordfighting(local_seat) {
        return vec![];
    }

    let mut cmds = Vec::new();
    let mut consumed = false;

    for &pc_id in engine.seat_selection(local_seat) {
        let Some((is_sword, pos_map, facing_dir)) = engine.get_entity(pc_id).and_then(|entity| {
            let h = entity.human_data()?;
            let is_sword = !h.opponents.is_empty();
            let elem = entity.element_data();
            let pos = geo2d::pt(elem.position_map().x, elem.position_map().y);
            let dir_sector = elem.direction();
            let dir_arr = crate::shadow_polygon::sector_to_direction(dir_sector);
            let facing = geo2d::pt(dir_arr[0], dir_arr[1] * ASPECT_RATIO);
            Some((is_sword, pos, facing))
        }) else {
            continue;
        };

        if !is_sword {
            // Non-swordfighting PC: click on sword target = engage.
            // The seek walks (running=false) since this is the single-
            // click path.
            if is_left_button
                && let Some(target_id) =
                    engine.find_focusable_entity(assets, &host.draw_order.ids, map_pt, Focus::Sword)
            {
                host.input.element_old_click = Some(target_id);
                cmds.push(PlayerCommand::EnterSwordfight {
                    actor: pc_id,
                    target: target_id,
                    running: false,
                });
            }
            continue;
        }

        let pc_screen = host.viewport.map_to_screen_unclamped(pos_map.into());
        let pattern = host.mouse_way.evaluate(pc_screen, facing_dir);
        tracing::trace!(
            "resolve_swordfight: pc={pc_id:?} pattern={pattern:?} mw_pts={}",
            host.mouse_way.len(),
        );

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
                    &host.draw_order.ids,
                    map_pt,
                    Focus::Sword,
                ) else {
                    continue;
                };

                let already_opponent = engine
                    .get_entity(pc_id)
                    .and_then(|e| e.human_data())
                    .map(|h| h.opponents.contains(&target_id))
                    .unwrap_or(false);

                host.input.element_old_click = Some(target_id);
                if already_opponent {
                    cmds.push(PlayerCommand::SwordStrikeCmd {
                        actor: pc_id,
                        target: target_id,
                        command: Command::SwordstrikeThrustA,
                        with_seek: true,
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
                let principal = engine
                    .get_entity(pc_id)
                    .and_then(|e| e.human_data())
                    .and_then(|h| h.opponents.first().copied());
                let Some(target_id) = principal else { continue };

                let with_seek = matches!(
                    recognised,
                    MouseWayPattern::ThrustA
                        | MouseWayPattern::ThrustB
                        | MouseWayPattern::ThrustC
                        | MouseWayPattern::ThrustD
                        | MouseWayPattern::ThrustE
                );

                cmds.push(PlayerCommand::SwordStrikeCmd {
                    actor: pc_id,
                    target: target_id,
                    command: strike_cmd,
                    with_seek,
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

// ─── Helpers (read-only) ────────────────────────────────────────────

/// Whether `target_id` is an NPC (Soldier / Civilian) with a currently
/// attached dialog scroll.  `Focus::Use` already gates the
/// `!is_out_of_order` precondition, so a direct `scroll_attached` read
/// on the entity is sufficient here.
fn is_target_scroll_attached_npc(engine: &Engine, target_id: EntityId) -> bool {
    match engine.get_entity(target_id) {
        Some(Entity::Soldier(s)) => s.npc.scroll_attached,
        Some(Entity::Civilian(c)) => c.npc.scroll_attached,
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

    // Object-class targets (Net, Bonus, Scroll, landed Projectile).
    // The engine-side `object_pickup_command` is the authoritative
    // implementation; this just calls straight through.
    if let Some(cmd) = robin_engine::engine::object_pickup_command(engine, assets, target_id, pc_id)
    {
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
    let posture = entity.element_data().posture;
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
            robin_engine::profiles::Action::Jump,
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
            if c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar
                && !c.npc.scroll_attached)
    {
        let ransom = engine
            .campaign()
            .map(|c| c.get_value(robin_engine::campaign::CampaignValue::Ransom))
            .unwrap_or(0);
        if ransom >= robin_engine::engine::BEGGAR_SALARY {
            return Some(Command::Pay);
        }
        return None;
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
            robin_engine::profiles::Action::Resuscitate,
        )
    {
        let target_pc_or_same_camp = match entity {
            Entity::Pc(_) => true,
            Entity::Soldier(s) => s.soldier.cached_camp == Camp::Royalists,
            _ => false,
        };
        if target_pc_or_same_camp {
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
            Entity::Soldier(s) => assets
                .profile_manager
                .get_soldier(s.soldier.soldier_profile_index)
                .map(|p| p.heavy)
                .unwrap_or(false),
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
            robin_engine::profiles::Action::Tie,
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
        MouseWayPattern::None | MouseWayPattern::Attempt => return None,
    })
}

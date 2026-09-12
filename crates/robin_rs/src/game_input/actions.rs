//! Action-specific click decisions; ordering and command tails stay explicit.
use super::*;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ActionClickContext {
    pc_id: EntityId,
    map_pt: MapPoint,
    local_seat: PlayerId,
    is_double: bool,
    is_planning: bool,
    is_recording: bool,
    is_deferred: bool,
    valid_trajectory: bool,
    selected_layer: u16,
}
impl ActionClickContext {
    fn commit_tail(&self) -> PlayerCommand {
        if self.is_recording {
            PlayerCommand::StopRecordingMacro
        } else {
            PlayerCommand::UnselectAllActions
        }
    }
    fn to_3d(
        &self,
        engine: &Engine,
        assets: &LevelAssets,
        point: MapPoint,
    ) -> engine_coordinates::WorldPoint3D {
        engine.fast_grid().convert_2d_to_3d(
            point,
            engine_sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
            engine.sight_obstacles(assets),
        )
    }
}

pub(super) fn resolve_action_left_click(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    map_pt: MapPoint,
    local_seat: PlayerId,
    action: Action,
    is_double: bool,
    is_planning: bool,
) -> Vec<PlayerCommand> {
    let pc_id = match engine.hero_selection(local_seat).first().copied() {
        Some(id) => id,
        None => return vec![],
    };
    let is_recording = engine.is_recording_macro();
    let is_deferred = is_recording || is_planning;
    let valid_trajectory = host.frontend.trajectory_preview().is_valid();
    let selected_layer = host.frontend.input.spatial_hit().selected_layer;

    let context = ActionClickContext {
        pc_id,
        map_pt,
        local_seat,
        is_double,
        is_planning,
        is_recording,
        is_deferred,
        valid_trajectory,
        selected_layer,
    };
    match action {
        Action::Bow => click_bow(host, engine, assets, &context),
        Action::Hit | Action::HitHard => click_hit(host, engine, assets, &context),
        Action::Apple => click_apple(host, engine, assets, &context),
        Action::Stone => click_stone(host, engine, assets, &context),
        Action::Heal => click_heal(host, engine, assets, &context),
        Action::Whistle => click_whistle(host, engine, assets, &context),
        Action::Strangle => click_strangle(host, engine, assets, &context),
        Action::Net => click_net(host, engine, assets, &context),
        Action::WaspNest => click_wasp_nest(host, engine, assets, &context),
        Action::Purse => click_purse(host, engine, assets, &context),
        Action::Shield | Action::BigShield => click_shield(host, engine, assets, &context),
        Action::Eat | Action::Guzzle => click_eat(host, engine, assets, &context),
        Action::Listen => click_listen(host, engine, assets, &context),
        Action::Lever => click_lever(host, engine, assets, &context),
        Action::Beggar => click_beggar(host, engine, assets, &context),
        Action::HelpToClimb => click_help_to_climb(host, engine, assets, &context),
        Action::Ale => click_ale(host, engine, assets, &context),
        _ => vec![],
    }
}

fn click_bow(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_double = context.is_double;
    let is_recording = context.is_recording;
    let is_deferred = context.is_deferred;
    let draw_order = &host.frontend.presentation.draw_order.ids;
    // Bow runs several validation steps before launching the
    // shoot sequence.
    //
    // 1. Climbing or inside a building → drop the click.
    //    Recording bypasses this gate so a macro can still be
    //    recorded while climbing / inside a building.
    if !is_deferred && engine.is_climbing_or_inside_building(pc_id) {
        return vec![];
    }

    let Some(target_id) = engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Bow)
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
    let archer_posture = pc_entity.element_data().posture();
    if !is_deferred && archer_posture == Posture::AnonymousArcher {
        tracing::info!(
            ?pc_id,
            ?target_id,
            "Bow click rejected: anonymous archer cannot shoot"
        );
        return vec![PlayerCommand::HeroSpeak {
            pc_id,
            expression: engine_api::melee::HERO_UNABLE_TO_DO_SOMETHING,
        }];
    }

    // 4. Range / LOS / shoot-mode validation.  Only applied
    //    in the non-record branch — a macro can be recorded
    //    even when the current LOS / range wouldn't allow the
    //    shot (replay re-evaluates).
    if !is_deferred {
        let (bow_status, _shoot_mode) = engine.can_shoot_with_bow_at(assets, pc_id, target_id);
        if bow_status != engine_api::input::BowTarget::Valid {
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
    cmds
}

fn click_hit(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_double = context.is_double;
    let is_recording = context.is_recording;
    let draw_order = &host.frontend.presentation.draw_order.ids;
    // The seek uses the running gait on double-click and
    // passes the no-transitions / seek-stop-NPC flags (handled
    // inside `apply_interaction_with_seek`).
    if let Some(target_id) = engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Hit) {
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

    vec![]
}

fn click_apple(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_recording = context.is_recording;
    let is_deferred = context.is_deferred;
    let valid_trajectory = context.valid_trajectory;
    let draw_order = &host.frontend.presentation.draw_order.ids;
    // Drop the click on an invalid trajectory unless recording
    // a macro.
    if !valid_trajectory && !is_deferred {
        return vec![];
    }
    if let Some(target_id) = engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Apple)
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

    vec![]
}

fn click_stone(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_recording = context.is_recording;
    let is_deferred = context.is_deferred;
    let valid_trajectory = context.valid_trajectory;
    let selected_layer = context.selected_layer;
    let draw_order = &host.frontend.presentation.draw_order.ids;
    // Trajectory gate — drop the click on an invalid arc
    // unless recording.
    if !valid_trajectory && !is_deferred {
        return vec![];
    }
    if let Some(target_id) = engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Stone)
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
    // Optional extension: the same Stone inventory action can target
    // valid ground. It remains a real projectile and consumes one
    // stone; only the impact's AI stimulus differs from an ordinary
    // entity-targeted throw.
    if engine.sim_config().item_gameplay.stone_ground_distraction
        && engine.is_mouse_sector_valid_for_ground_target(map_pt)
        && (is_deferred
            || (valid_trajectory
                && engine.is_in_range_for_projectile(assets, pc_id, map_pt, Action::Stone, None)))
    {
        let mut cmds = vec![PlayerCommand::LaunchGroundTarget {
            actor: pc_id,
            target_pos: context.to_3d(engine, assets, map_pt),
            command: Command::ThrowStone,
            target_field: Field::NoiseDistractionTarget,
            titbit_layer: selected_layer,
        }];
        if is_recording {
            cmds.push(PlayerCommand::StopRecordingMacro);
        }
        return cmds;
    }

    vec![]
}

fn click_heal(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_double = context.is_double;
    let is_recording = context.is_recording;
    let draw_order = &host.frontend.presentation.draw_order.ids;
    // Heal unselects the action unconditionally after launch;
    // the recording path additionally closes the macro.  The
    // SEEK_IN_BUILDINGS flag is handled inside
    // `apply_interaction_with_seek`.
    if let Some(target_id) = engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Heal) {
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

    vec![]
}

fn click_whistle(
    _host: &mut Host,
    _engine: &Engine,
    _assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    // Launch the whistle ability then either deselect
    // the action or stop macro recording.
    vec![
        PlayerCommand::LaunchSelfAbility {
            actor: pc_id,
            command: Command::WhistleCmd,
        },
        context.commit_tail(),
    ]
}

fn click_strangle(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_double = context.is_double;
    let is_recording = context.is_recording;
    let draw_order = &host.frontend.presentation.draw_order.ids;
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

    vec![]
}

fn click_net(
    _host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_recording = context.is_recording;
    let is_deferred = context.is_deferred;
    let valid_trajectory = context.valid_trajectory;
    // Trajectory gate.
    if !valid_trajectory && !is_deferred {
        return vec![];
    }
    let mut cmds = vec![PlayerCommand::LaunchGroundTarget {
        actor: pc_id,
        target_pos: context.to_3d(engine, assets, map_pt),
        command: Command::ThrowNet,
        target_field: Field::NetTarget,
        // Net titbits are hard-coded to layer 0 regardless of
        // the currently selected layer.
        titbit_layer: 0,
    }];
    if is_recording {
        cmds.push(PlayerCommand::StopRecordingMacro);
    }
    cmds
}

fn click_wasp_nest(
    _host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_recording = context.is_recording;
    let is_deferred = context.is_deferred;
    let valid_trajectory = context.valid_trajectory;
    let selected_layer = context.selected_layer;
    // Trajectory gate.
    if !valid_trajectory && !is_deferred {
        return vec![];
    }
    let mut cmds = vec![PlayerCommand::LaunchGroundTarget {
        actor: pc_id,
        target_pos: context.to_3d(engine, assets, map_pt),
        command: Command::ThrowWaspNest,
        target_field: Field::WaspNestTarget,
        // Wasp nest titbits are placed on the currently
        // selected layer.
        titbit_layer: selected_layer,
    }];
    if is_recording {
        cmds.push(PlayerCommand::StopRecordingMacro);
    }
    cmds
}

fn click_purse(
    _host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_deferred = context.is_deferred;
    let valid_trajectory = context.valid_trajectory;
    let selected_layer = context.selected_layer;
    // Live throws require the currently displayed trajectory.
    // Deferred recording/planning validates from the eventual
    // execution position instead of the real PC's present position.
    if !valid_trajectory && !is_deferred {
        return vec![];
    }
    vec![
        PlayerCommand::LaunchGroundTarget {
            actor: pc_id,
            target_pos: context.to_3d(engine, assets, map_pt),
            command: Command::ThrowPurse,
            target_field: Field::PurseTarget,
            // Place the titbit on the currently selected layer
            // (same as Wasp) so it stays under the mouse when
            // the selected layer differs from the PC's.
            titbit_layer: selected_layer,
        },
        context.commit_tail(),
    ]
}

fn click_shield(
    _host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let local_seat = context.local_seat;
    let is_planning = context.is_planning;
    let selected_layer = context.selected_layer;
    if is_planning {
        if let Some(protected_pc) = engine.planned_shield_protected_for_seat(local_seat, pc_id) {
            return vec![PlayerCommand::RaiseShieldWithDanger {
                actor: pc_id,
                protected_pc,
                danger_point: context.to_3d(engine, assets, map_pt),
                danger_point_layer: selected_layer,
            }];
        }
        if let Some(target_id) = engine.find_focusable_pc(assets, map_pt, Focus::Shield) {
            return vec![PlayerCommand::SelectPlannedShieldProtected {
                actor: pc_id,
                protected_pc: target_id,
            }];
        }
        return vec![];
    }
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
    vec![
        PlayerCommand::RaiseShieldWithDanger {
            actor: pc_id,
            protected_pc,
            danger_point: context.to_3d(engine, assets, map_pt),
            danger_point_layer: selected_layer,
        },
        context.commit_tail(),
    ]
}

fn click_eat(
    _host: &mut Host,
    _engine: &Engine,
    _assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    vec![
        PlayerCommand::LaunchSelfAbility {
            actor: pc_id,
            command: Command::EatCmd,
        },
        context.commit_tail(),
    ]
}

fn click_listen(
    _host: &mut Host,
    engine: &Engine,
    _assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
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
    // A click while EnterListen owns its non-interruptable chain emits
    // LeaveListen. Sequence arbitration postpones it until EnterListen
    // finishes; after the actor returns to Waiting, the restamped
    // LeaveListen fails MUST_BE_LISTENING and becomes Impossible.
    let cmd = match listen_phase {
        ListenPhase::Inactive => Command::EnterListen,
        ListenPhase::EnterTransition | ListenPhase::CountingDown => Command::LeaveListen,
        ListenPhase::ExitTransition => return vec![],
    };
    vec![
        PlayerCommand::LaunchSelfAbility {
            actor: pc_id,
            command: cmd,
        },
        context.commit_tail(),
    ]
}

fn click_lever(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_double = context.is_double;
    let draw_order = &host.frontend.presentation.draw_order.ids;
    // Interaction on a focusable lever (FX target or hookable
    // mobile), followed by deselecting the action.
    if let Some(target_id) = engine.find_focusable_entity(assets, draw_order, map_pt, Focus::Lever)
    {
        cache_click_and_drag_target(host, target_id);
        return vec![
            PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: target_id,
                command: Command::UseLever,
                running: is_double,
            },
            context.commit_tail(),
        ];
    }

    vec![]
}

fn click_beggar(
    _host: &mut Host,
    engine: &Engine,
    _assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let is_double = context.is_double;
    let is_recording = context.is_recording;
    // Posture-keyed arms:
    //   SimulatingBeggar + double + !recording → MakePcFast
    //   SimulatingBeggar + single              → fall through to walk
    //   default                                → EnterBeggar (+ deselect on double)
    let posture = engine
        .get_entity(pc_id)
        .map(|e| e.element_data().posture())
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
    vec![
        PlayerCommand::LaunchSelfAbility {
            actor: pc_id,
            command: Command::EnterBeggar,
        },
        context.commit_tail(),
    ]
}

fn click_help_to_climb(
    _host: &mut Host,
    engine: &Engine,
    _assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let is_double = context.is_double;
    let is_recording = context.is_recording;
    // Posture branches:
    //   HelpingToClimb | CarryingOnShoulders → walk-through to
    //     click (no-action path), or MakePcFast on double-click.
    //   default → launch EnterHelpingClimb (+ deselect).
    let posture = engine
        .get_entity(pc_id)
        .map(|e| e.element_data().posture())
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
    vec![
        PlayerCommand::LaunchSelfAbility {
            actor: pc_id,
            command: Command::EnterHelpingClimb,
        },
        context.commit_tail(),
    ]
}

fn click_ale(
    _host: &mut Host,
    engine: &Engine,
    _assets: &LevelAssets,
    context: &ActionClickContext,
) -> Vec<PlayerCommand> {
    let pc_id = context.pc_id;
    let map_pt = context.map_pt;
    let is_double = context.is_double;
    // Build a seek with a post-seek `DropAle` element: walk or
    // run the selected PC to the cursor, then play the ale-
    // drop animation to materialise a bottle at the PC's feet.
    //
    // The `DropAleAt` command handler constructs the
    // Move → DropAle sequence; the engine tick's
    // `Command::DropAle` arm then spawns the bottle and
    // decrements `Action::Ale` ammo.
    if !engine.is_mouse_sector_valid_for_ground_target(map_pt) {
        return vec![];
    }
    vec![
        PlayerCommand::DropAleAt {
            actor: pc_id,
            target_pos: map_pt,
            running: is_double,
            already_authorized: false,
            goal_override: None,
            goal_sector_index_override: None,
            recorded_gate_path: None,
        },
        context.commit_tail(),
    ]
}

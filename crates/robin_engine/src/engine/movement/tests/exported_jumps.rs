use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum JumpDispatch {
    Isolated,
    Approach,
    Click,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JumpStance {
    Upright,
    Sword,
    Shoulders,
    Vertical,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct JumpArrival {
    final_distance: f32,
    completed_turning_startup: bool,
    turning_distance_loss: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    trajectory: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Default)]
struct StartupWalkAudit {
    start: Option<MapPoint>,
    raw_distance: f32,
    turning_loss: f32,
    invalid_step: bool,
    finished: bool,
}

impl StartupWalkAudit {
    // A short move can consist solely of its finite startup animation. Turning
    // reduces travel without extending that animation. Accept only its measured
    // distance loss, never a general near-goal tolerance or a collision shortfall.
    fn observe(&mut self, before: MapPoint, sprite: &crate::sprite::Sprite) {
        if self.finished || sprite.last_action != OrderType::TransitionWaitingUprightWalkingUpright
        {
            return;
        }
        self.start.get_or_insert(before);
        let raw = sprite.current_frame_distance();
        let position = &sprite.position_iface;
        let effective = if raw > 0. && position.get_direction() != position.get_direction_goal() {
            (raw * 0.6).max(0.7)
        } else {
            raw
        };
        self.raw_distance += raw;
        self.turning_loss += raw - effective;
        self.invalid_step |= ((position.map_position() - before).length() - effective).abs()
            > 0.001
            || position.is_deviated()
            || position.blocked_count != 0;
        self.finished = sprite.last_motion_state == Some(crate::sprite::MotionState::Terminated);
    }

    fn completed_while_turning(&self, sprite: &crate::sprite::Sprite, goal: MapPoint) -> bool {
        let Some(start) = self.start else {
            return false;
        };
        let delta = goal - start;
        let distance = delta.length();
        let moved = self.raw_distance - self.turning_loss;
        if !self.finished
            || self.invalid_step
            || self.turning_loss <= 0.
            || distance <= 0.
            || self.raw_distance + 0.001 < distance
            || moved >= distance
        {
            return false;
        }
        let expected = MapPoint::new(
            start.x + delta.x * moved / distance,
            start.y + delta.y * moved / distance,
        );
        (sprite.position_iface.map_position() - expected).length() < 0.001
    }
}

#[test]
fn short_startup_finishes_its_animation_with_only_the_turning_distance_loss() {
    for (turn, expected_distance) in [(0, 4.), (1, 3.2), (4, 2.4)] {
        let (mut engine, mut assets) = compiled_walkway(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/asset-navigation-copies.level.json"
        )));
        let sector = crate::position_interface::SectorHandle::from_number(
            engine.world.fast_grid.level.sectors[0].sector_number,
        )
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap());
        let start = MapPoint::new(400., 300.);
        let goal = MapPoint::new(404., 300.);
        let owner = walking_pc(&mut engine, &mut assets, start, 0, sector);
        let action = OrderType::TransitionWaitingUprightWalkingUpright;
        let script = crate::sprite_script::SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 2.,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 4,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![2; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
        };
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[action as usize] = 0;
        let element = engine.ent_mut(owner).element_data_mut();
        let position = element.sprite.position_iface.clone();
        element.sprite =
            crate::sprite::Sprite::new(Arc::new(vec![script; 16]), Arc::new(conversion));
        element.sprite.position_iface = position;
        element.set_direction_instantly((vector_to_sector_0_to_15(1., 0.) + turn) & 15);
        element.sprite.position_iface.set_anti_collision_on(false);
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingUpright,
        );
        let mut order =
            crate::order::Order::new(action, goal.x, goal.y, engine.orders.allocate_order_id());
        order.compute_direction = true;
        movement.orders.push_back(order);
        let sequence = engine.t_launch_in_progress(&assets, movement);
        engine.select_sequence_element(owner, Some((sequence, 0)));
        let mut audit = StartupWalkAudit::default();
        for _ in 0..2 {
            let before = engine.ent(owner).element_data().position_map();
            engine.t_tick_actor_owner_envelopes(&assets);
            audit.observe(before, &engine.ent(owner).element_data().sprite);
        }
        let sprite = &engine.ent(owner).element_data().sprite;
        assert_eq!(
            sprite.last_motion_state,
            Some(crate::sprite::MotionState::Terminated)
        );
        assert!(
            (sprite.position_iface.map_position().x - start.x - expected_distance).abs() < 0.001
        );
        assert_eq!(audit.completed_while_turning(sprite, goal), turn != 0);
    }
}

#[test]
#[ignore = "requires exported jump pairs and ROBIN_CLIMB_RHS"]
fn exported_jumps_complete_sprite_dispatch_and_land_on_receivers() {
    audit_jump_dispatch(JumpDispatch::Isolated, JumpStance::Upright);
}

#[test]
fn empty_shoulder_carrier_recovers_only_at_termination_and_preserves_newer_actions() {
    use crate::profiles::Action;
    let recovery = OrderType::TransitionWaitingCarryingOnShouldersWaitingUpright;
    for (selected, action) in [
        (false, Action::HelpToClimb),
        (false, Action::Net),
        (true, Action::HelpToClimb),
        (true, Action::Net),
    ] {
        let (mut engine, mut assets) = compiled_walkway(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/asset-navigation-copies.level.json"
        )));
        let sector = crate::position_interface::SectorHandle::from_number(
            engine.world.fast_grid.level.sectors[0].sector_number,
        )
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap());
        let owner = walking_pc(
            &mut engine,
            &mut assets,
            MapPoint::new(400., 300.),
            0,
            sector,
        );
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        let mut scripts = vec![];
        for animation in [
            recovery,
            OrderType::WaitingUpright,
            OrderType::WaitingCarryingOnShoulders,
        ] {
            conversion[animation as usize] = scripts.len() as u16;
            scripts.extend(vec![
                crate::sprite_script::SpriteScript {
                    action_id: animation as u16,
                    action_done: if animation == recovery { 1 } else { 2 },
                    average_speed: 0.,
                    hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
                    sum_distance: 0,
                    frame_ids: vec![1, 2, 3],
                    delays: vec![0; 3],
                    distances: vec![0; 3],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
                    sound_ids: vec![0; 3],
                };
                16
            ]);
        }
        let element = engine.ent_mut(owner).element_data_mut();
        let position = element.sprite.position_iface.clone();
        element.sprite = crate::sprite::Sprite::new(Arc::new(scripts), Arc::new(conversion));
        element.sprite.position_iface = position;
        engine.set_entity_posture(owner, Posture::CarryingOnShoulders);
        engine.ent_mut(owner).actor_data_mut().unwrap().action_state = ActionState::Waiting;
        engine.pc_mut(owner).current_action = action;
        if selected {
            engine.players.seats[0].selection.push(owner);
            engine.players.seats[0].selected_action = action;
        }
        let sim = crate::sim_rng::test_context();
        engine.launch_element(
            TickCtx::new(&sim, &assets),
            SequenceElement::new(1, Command::LeaveHelpingClimb, Some(owner)),
        );
        let mut saw_done = false;
        for _ in 0..12 {
            engine.control.frame_counter += 1;
            engine.t_hourglass_phase_sequences(&assets);
            engine.t_tick_actor_owner_envelopes(&assets);
            let entity = engine.ent(owner);
            let element = entity.element_data();
            if element.sprite.last_action == recovery
                && element.sprite.last_motion_state == Some(crate::sprite::MotionState::Done)
            {
                saw_done = true;
                assert_eq!(element.posture(), Posture::CarryingOnShoulders);
                assert_eq!(entity.pc_data().unwrap().current_action, action);
            }
            if element.posture() == Posture::Upright {
                break;
            }
        }
        assert!(saw_done, "the animation must reach DONE before TERMINATED");
        assert_eq!(engine.ent(owner).element_data().posture(), Posture::Upright);
        assert_eq!(
            engine.ent(owner).actor_data().unwrap().action_state,
            ActionState::Waiting
        );
        let expected = if selected && action == Action::Net {
            Action::Net
        } else {
            Action::NoAction
        };
        assert_eq!(engine.pc(owner).current_action, expected);
        if selected {
            assert_eq!(engine.players.seats[0].selected_action, expected);
        }
        engine.t_hourglass_phase_sequences(&assets);
        engine.t_tick_actor_owner_envelopes(&assets);
        let (sequence, index) = engine.entities().current_element_for_actor(owner).unwrap();
        assert_eq!(
            engine.seq().get_element(sequence, index).unwrap().command,
            Command::Wait
        );
    }
}

#[test]
fn exported_roof_jump_overlays_select_the_current_connection_and_require_jump_skill() {
    let (mut engine, mut assets) = compiled_walkway(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-jump-rotated-roofs.level.json"
    )));
    assert_eq!(engine.world.fast_grid.level.jump_lines.len(), 4);
    for index in 0..4 {
        let source = engine.world.fast_grid.level.jump_lines[index].clone();
        let destination = engine.world.fast_grid.level.jump_lines
            [source.associated_line_index.unwrap() as usize]
            .clone();
        let handle = |line: &crate::jump_line::JumpLine| {
            let index = line.sector_index.unwrap();
            crate::position_interface::SectorHandle::from_number(
                engine.world.fast_grid.level.sectors[usize::from(index)].sector_number,
            )
            .with_arena_index(index)
        };
        let source_sector = handle(&source);
        let destination_sector = handle(&destination);
        let vector = source.vector();
        let inset = crate::coordinates::MapVec::new(
            -vector.y * 2. / vector.length(),
            vector.x * 2. / vector.length(),
        );
        let start = source.get_middle_point() - inset;
        let goal = destination.get_middle_point() + inset;
        let owner = walking_pc(&mut engine, &mut assets, start, source.layer, source_sector);
        engine.ent_mut(owner).pc_data_mut().unwrap().has_jump = true;
        resolve_jump_click(&engine, owner, start, goal, index, destination_sector).unwrap();
        engine.ent_mut(owner).pc_data_mut().unwrap().has_jump = false;
        assert!(!engine.is_jumpable(index as u32, owner, true));
        assert!(
            resolve_jump_click(&engine, owner, start, goal, index, destination_sector).is_err()
        );
    }
}

fn resolve_jump_click(
    engine: &EngineInner,
    owner: EntityId,
    start: MapPoint,
    goal: MapPoint,
    index: usize,
    destination_sector: crate::position_interface::SectorHandle,
) -> Result<(), String> {
    let hit = engine.world.fast_grid.get_sector_screen(goal, start);
    let hit_index = hit
        .sector_idx
        .ok_or_else(|| format!("click misses all sectors: {goal:?}"))?;
    let selected = &engine.world.fast_grid.level.sectors[usize::from(hit_index)];
    if !selected.sector_type.is_jump() {
        return Err(format!(
            "click does not select jump overlay: {goal:?}, sector={:?}",
            selected.sector_number
        ));
    }
    let chosen = engine.get_nearest_jumpable_jump_line(
        owner,
        u32::from(hit_index),
        start,
        goal,
        true,
        Some(u16::from(destination_sector)),
    );
    if chosen != Some(index as u32) {
        return Err(format!(
            "click selects wrong jump: expected={index}, chosen={chosen:?}"
        ));
    }
    Ok(())
}

#[test]
#[ignore = "requires exported jump pairs and ROBIN_CLIMB_RHS"]
fn exported_jumps_walk_to_launch_and_continue_after_landing() {
    audit_jump_dispatch(JumpDispatch::Approach, JumpStance::Upright);
}

#[test]
#[ignore = "requires exported long-jump pairs and ROBIN_CLIMB_RHS"]
fn exported_jumps_complete_sword_dispatch_and_preserve_combat() {
    audit_jump_dispatch(JumpDispatch::Isolated, JumpStance::Sword);
}

#[test]
#[ignore = "requires exported jump pairs and ROBIN_CLIMB_RHS"]
fn exported_jumps_resolve_player_clicks_and_complete_the_route() {
    audit_jump_dispatch(JumpDispatch::Click, JumpStance::Upright);
}

#[test]
#[ignore = "requires exported jump pairs, ROBIN_CLIMB_RHS and ROBIN_CARRIER_RHS"]
fn exported_jumps_from_shoulders_land_and_release_the_carrier() {
    audit_jump_dispatch(JumpDispatch::Isolated, JumpStance::Shoulders);
}

#[test]
#[ignore = "requires exported jump pairs, ROBIN_CLIMB_RHS and ROBIN_CARRIER_RHS"]
fn exported_jump_clicks_route_the_carrier_then_land_the_rider() {
    audit_jump_dispatch(JumpDispatch::Click, JumpStance::Shoulders);
}

#[test]
#[ignore = "requires exported vertical jump pairs and complete rider/helper sprites"]
fn exported_vertical_jumps_climb_with_help_and_descend_upright() {
    audit_jump_dispatch(JumpDispatch::Isolated, JumpStance::Vertical);
}

#[test]
#[ignore = "requires exported vertical jump pairs and complete rider/helper sprites"]
fn exported_vertical_jumps_resolve_clicks_and_complete_the_route() {
    audit_jump_dispatch(JumpDispatch::Click, JumpStance::Vertical);
}

fn audit_jump_dispatch(mode: JumpDispatch, stance: JumpStance) {
    let sprite = super::exported_stairs::complete_climb_sprite();
    let carrier_sprite =
        matches!(stance, JumpStance::Shoulders | JumpStance::Vertical).then(|| {
            let path = std::env::var("ROBIN_CARRIER_RHS")
                .expect("path to a complete LittleJohn RHS sprite");
            super::exported_stairs::complete_character_sprite(
                std::path::Path::new(&path),
                "Petit Jean",
            )
        });
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let mut results = vec![];
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
        let approach_depth = result["approach_depth"].as_f64().unwrap_or(8.) as f32;
        assert!(approach_depth.is_finite() && approach_depth > 0.);
        // Click inside the exported landing band. Its outer edge rounds to the
        // native integer grid; the full-depth movement goal can lie outside it.
        let approach_depth = if mode == JumpDispatch::Click {
            approach_depth / 2.
        } else {
            approach_depth
        };
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let dims = &descriptor["walkable_polygon"][2];
        let (engine, assets) = compiled_walkway_with_dimensions(
            &bytes,
            (
                dims[0].as_f64().unwrap() as f32 + 1.,
                dims[1].as_f64().unwrap() as f32 + 1.,
            ),
        );
        for index in 0..engine.world.fast_grid.level.jump_lines.len() {
            let line = &engine.world.fast_grid.level.jump_lines[index];
            let destination = &engine.world.fast_grid.level.jump_lines
                [line.associated_line_index.unwrap() as usize];
            let climbing = destination.z_a > line.z_a;
            let case_stance = if stance == JumpStance::Vertical {
                assert!(!line.long_jump_forced);
                assert!((destination.z_a - line.z_a).abs() >= 60.);
                assert_eq!(destination.helper_needed, climbing);
                if climbing {
                    JumpStance::Shoulders
                } else {
                    JumpStance::Upright
                }
            } else {
                stance
            };
            for t in if mode == JumpDispatch::Click {
                [0.05, 0.25, 0.5, 0.75, 0.95]
            } else {
                [0., 0.25, 0.5, 0.75, 1.]
            } {
                if stance == JumpStance::Vertical && mode == JumpDispatch::Click && !climbing {
                    let vector = line.vector();
                    let inset = crate::coordinates::MapVec::new(
                        -vector.y * approach_depth / vector.length(),
                        vector.x * approach_depth / vector.length(),
                    );
                    let offset = crate::coordinates::MapVec::new(vector.x * t, vector.y * t);
                    let start = line.point_a + offset - inset;
                    let goal = destination.point_b + offset + inset;
                    let hit = engine.world.fast_grid.get_sector_screen(goal, start);
                    if hit.sector_idx == line.sector_index && line.layer > destination.layer {
                        let lower =
                            engine
                                .world
                                .fast_grid
                                .get_sector(goal, start, destination.layer);
                        let crate::fast_find_grid::SectorHit::Found { sector_idx, .. } = lower
                        else {
                            panic!("occluded lower landing must still have a jump overlay");
                        };
                        assert!(
                            engine.world.fast_grid.level.sectors[usize::from(sector_idx)]
                                .sector_type
                                .is_jump()
                        );
                        let upper_sector = line.sector_index.unwrap();
                        let upper_handle = crate::position_interface::SectorHandle::from_number(
                            engine.world.fast_grid.level.sectors[usize::from(upper_sector)]
                                .sector_number,
                        )
                        .with_arena_index(upper_sector);
                        assert!(
                            engine
                                .get_projection_area_index(&assets, upper_handle, line.layer, goal)
                                .is_some()
                        );
                        results.push(serde_json::json!({
                            "file": file, "line": index, "t": t, "passed": true,
                            "outcome": "source-platform-occludes-lower-click",
                            "arrival": null, "error": null,
                        }));
                        continue;
                    }
                }
                let outcome = dispatch_jump(
                    engine.clone(),
                    assets.clone(),
                    &sprite,
                    index,
                    t,
                    mode,
                    approach_depth,
                    case_stance,
                    carrier_sprite
                        .as_ref()
                        .filter(|_| case_stance == JumpStance::Shoulders),
                );
                results.push(serde_json::json!({
                    "file": file, "line": index, "t": t,
                    "outcome": "traversal",
                    "approach_depth": approach_depth,
                    "passed": outcome.is_ok(), "error": outcome.as_ref().err(),
                    "arrival": outcome.as_ref().ok(),
                }));
            }
        }
    }
    let failed = results
        .iter()
        .filter(|result| result["passed"] != true)
        .count();
    let traversals = results
        .iter()
        .filter(|result| result["outcome"] == "traversal")
        .count();
    let occlusions = results.len() - traversals;
    let report = serde_json::json!({
        "stance": match stance { JumpStance::Sword => "sword", JumpStance::Shoulders => "shoulders", JumpStance::Upright => "upright", JumpStance::Vertical => "assisted-ascent-upright-descent" },
        "scope": match mode {
            JumpDispatch::Click => "player-click-resolution-and-walk-jump-walk-not-rendering",
            JumpDispatch::Approach => "walk-jump-walk-sequence-not-click-authorization-or-rendering",
            JumpDispatch::Isolated => "sprite-dispatch-and-landing-not-click-approach-or-rendering",
        },
        "complete": traversals > 0 && failed == 0,
        "traversal_cases": traversals, "occlusion_cases": occlusions, "results": results,
    });
    let path = directory.join(
        if stance == JumpStance::Vertical && mode == JumpDispatch::Click {
            "actor-jump-vertical-click-report.json"
        } else if stance == JumpStance::Vertical {
            "actor-jump-vertical-report.json"
        } else if stance == JumpStance::Shoulders && mode == JumpDispatch::Click {
            "actor-jump-shoulders-click-report.json"
        } else if stance == JumpStance::Shoulders {
            "actor-jump-shoulders-report.json"
        } else if stance == JumpStance::Sword {
            "actor-jump-sword-report.json"
        } else if mode == JumpDispatch::Click {
            "actor-jump-click-report.json"
        } else if mode == JumpDispatch::Approach {
            "actor-jump-approach-report.json"
        } else {
            "actor-jump-landing-report.json"
        },
    );
    std::fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(traversals > 0);
    assert_eq!(
        failed,
        0,
        "{failed}/{} jump landings failed; see {}",
        results.len(),
        path.display()
    );
}

fn dispatch_jump(
    mut engine: EngineInner,
    mut assets: LevelAssets,
    sprite: &crate::sprite::Sprite,
    index: usize,
    t: f32,
    mode: JumpDispatch,
    approach_depth: f32,
    stance: JumpStance,
    carrier_sprite: Option<&crate::sprite::Sprite>,
) -> Result<JumpArrival, String> {
    let approach = mode != JumpDispatch::Isolated;
    let source = engine.world.fast_grid.level.jump_lines[index].clone();
    let destination_index = source.associated_line_index.unwrap();
    let destination = engine.world.fast_grid.level.jump_lines[destination_index as usize].clone();
    let handle = |line: &crate::jump_line::JumpLine| {
        let index = line.sector_index.unwrap();
        crate::position_interface::SectorHandle::from_number(
            engine.world.fast_grid.level.sectors[usize::from(index)].sector_number,
        )
        .with_arena_index(index)
    };
    let source_sector = handle(&source);
    let destination_sector = handle(&destination);
    let offset = crate::coordinates::MapVec::new(source.vector().x * t, source.vector().y * t);
    let normal = crate::coordinates::MapVec::new(-source.vector().y, source.vector().x);
    let distance = if approach {
        approach_depth / source.vector().length()
    } else {
        0.
    };
    let inset = crate::coordinates::MapVec::new(normal.x * distance, normal.y * distance);
    let start = source.point_a + offset - inset;
    let goal = destination.point_b + offset + inset;
    if approach {
        let footprint = engine
            .world
            .fast_grid
            .try_move_box_half_diagonal(0)
            .unwrap();
        for (label, edge, interior, layer) in [
            ("launch", source.point_a + offset, start, source.layer),
            (
                "landing",
                destination.point_b + offset,
                goal,
                destination.layer,
            ),
        ] {
            if !engine
                .world
                .fast_grid
                .is_reachable_thick(edge, interior, layer, footprint)
            {
                return Err(format!(
                    "{label} approach cannot fit the native actor: edge={edge:?}, interior={interior:?}, depth={approach_depth}"
                ));
            }
        }
    }
    let landing_receiver =
        engine.get_projection_area_index(&assets, destination_sector, destination.layer, goal);
    if landing_receiver.is_none() && (destination.z_a != 0. || destination.z_b != 0.) {
        return Err(format!("elevated landing point has no receiver: {goal:?}"));
    }
    let receiver = engine.get_projection_area_index(&assets, source_sector, source.layer, start);
    if receiver.is_none() && (source.z_a != 0. || source.z_b != 0.) {
        return Err(format!("elevated launch point has no receiver: {start:?}"));
    }
    let owner = walking_pc(&mut engine, &mut assets, start, source.layer, source_sector);
    engine.ent_mut(owner).pc_data_mut().unwrap().has_jump = true;
    let opponent = (stance == JumpStance::Sword).then(|| {
        let opponent = walking_pc(
            &mut engine,
            &mut assets,
            MapPoint::new(start.x - 100., start.y - 100.),
            source.layer,
            source_sector,
        );
        engine.human_mut(owner).opponents.push(opponent);
        engine.human_mut(opponent).opponents.push(owner);
        engine.ent_mut(owner).actor_data_mut().unwrap().action_state =
            crate::element_kinds::ActionState::WaitingSword;
        opponent
    });
    let element = engine.ent_mut(owner).element_data_mut();
    let position = element.sprite.position_iface.clone();
    element.sprite = sprite.clone();
    element.sprite.position_iface = position;
    engine.set_obstacle_and_material(&assets, owner, receiver);
    if destination.helper_needed && engine.is_jumpable(index as u32, owner, true) {
        return Err("helper-required destination accepted an unassisted actor".into());
    }
    let carrier = carrier_sprite.map(|sprite| {
        let carrier = walking_pc(&mut engine, &mut assets, start, source.layer, source_sector);
        let element = engine.ent_mut(carrier).element_data_mut();
        let position = element.sprite.position_iface.clone();
        element.sprite = sprite.clone();
        element.sprite.position_iface = position;
        engine.set_obstacle_and_material(&assets, carrier, receiver);
        engine.set_entity_posture(carrier, Posture::CarryingOnShoulders);
        engine.set_entity_posture(owner, Posture::OnShoulders);
        engine
            .ent_mut(carrier)
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::Waiting;
        engine.ent_mut(owner).actor_data_mut().unwrap().action_state = ActionState::Waiting;
        let pc = engine.ent_mut(carrier).pc_data_mut().unwrap();
        pc.carried = Some(owner);
        pc.set_live_carried_posture(Posture::OnShoulders);
        engine.human_mut(owner).carrier = Some(carrier);
        carrier
    });
    if !engine.is_jumpable(index as u32, owner, true) {
        return Err("prepared connection rejects its required actor posture".into());
    }
    let mut jump = SequenceElement::new_generic(1, Command::JumpCmd, Some(owner));
    jump.set_property(
        crate::sequence::Field::JumplineSource,
        crate::sequence::FieldValue::Integer(index as u32),
    );
    jump.set_property(
        crate::sequence::Field::JumplineDestination,
        crate::sequence::FieldValue::Integer(destination_index),
    );
    let sim = crate::sim_rng::test_context();
    let sequence = if mode == JumpDispatch::Click {
        resolve_jump_click(&engine, owner, start, goal, index, destination_sector)?;
        engine.perform_group_move(
            TickCtx::new(&sim, &assets),
            &[owner],
            goal,
            false,
            false,
            None,
            None,
            None,
            &[],
            &[],
        );
        None
    } else if approach {
        let source_id = crate::jump_line::JumpLineIndex::new(index as u32).unwrap();
        let destination_id = crate::jump_line::JumpLineIndex::new(destination_index).unwrap();
        Some(
            engine
                .launch_gate_movement_sequence(
                    TickCtx::new(&sim, &assets),
                    &mut vec![],
                    crate::engine::movement::GateRouteRequest {
                        entity_id: owner,
                        source_sector: None,
                        gate_path: vec![],
                        goal: crate::engine::movement::GoalShape::Line {
                            line_index: source_id,
                            midpoint: source.get_middle_point(),
                            tolerance: 0.,
                        },
                        goal_layer: source.layer,
                        base_action: OrderType::WalkingUpright,
                        move_after_last_door: true,
                        speed_factor: 1.,
                        initial_flags: crate::sequence::MoveFlags::empty(),
                        prefix_elements: vec![],
                        tail_elements: crate::engine::movement::build_line_jump_click_tail(
                            owner,
                            OrderType::WalkingUpright,
                            source_id,
                            destination_id,
                            goal,
                            destination.layer,
                            1.,
                        ),
                        append_arrival_speech: false,
                        append_recovery: false,
                    },
                )
                .ok_or("walk-jump-walk route construction failed")?,
        )
    } else {
        let sequence = engine.t_launch_in_progress(&assets, jump);
        if !engine.start_jump(
            TickCtx::new(&sim, &assets),
            owner,
            crate::sequence::SequenceElementRef::new(sequence, 0),
        ) {
            return Err("jump dispatch rejected the connection".into());
        }
        engine.select_sequence_element(owner, Some((sequence, 0)));
        Some(sequence)
    };
    let mut flew = false;
    let mut sword_flew = false;
    let mut shoulder_launched = false;
    let vertical = !source.long_jump_forced
        && (destination.z_a - source.z_a).abs() >= if carrier.is_some() { 100. } else { 60. };
    let expected_flight = if !vertical {
        if stance == JumpStance::Sword {
            OrderType::JumpingLongSword
        } else {
            OrderType::JumpingLong
        }
    } else if destination.z_a > source.z_a {
        OrderType::JumpingUp
    } else {
        OrderType::JumpingDown
    };
    let mut expected_flight_seen = false;
    let mut startup = StartupWalkAudit::default();
    let mut trajectory = std::env::var_os("ROBIN_TRACE_JUMP").map(|_| Vec::new());
    for _ in 0..1000 {
        engine.control.frame_counter += 1;
        if approach || carrier.is_some() {
            engine.t_hourglass_phase_sequences(&assets);
            engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
        }
        let before = engine.ent(owner).element_data().position_map();
        engine.t_tick_actor_owner_envelopes(&assets);
        let element = engine.ent(owner).element_data();
        if let Some(trajectory) = &mut trajectory {
            let position = element.position();
            trajectory.push(serde_json::json!({
                "action": format!("{:?}", element.sprite.last_action),
                "motion": format!("{:?}", element.sprite.last_motion_state),
                "position": [position.x, position.y, position.z],
            }));
        }
        if approach && flew {
            startup.observe(before, &element.sprite);
        }
        flew |= element.posture() == Posture::Flying;
        sword_flew |= element.sprite.last_action == OrderType::JumpingLongSword;
        shoulder_launched |= element.sprite.last_action
            == if vertical {
                OrderType::TransitionWaitingOnShouldersJumpingUp
            } else {
                OrderType::TransitionWaitingOnShouldersJumpingLong
            };
        expected_flight_seen |= element.sprite.last_action == expected_flight;
        let finished = sequence.is_some_and(|sequence| {
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .is_none_or(|element| element.orders.is_empty())
        });
        let finished_selection = engine
            .entities()
            .current_element_for_actor(owner)
            .and_then(|(id, index)| engine.seq().get_element(id, index))
            .is_none_or(|selected| selected.orders.is_empty() || selected.command == Command::Wait);
        let completed_turning_startup = approach
            && finished_selection
            && startup.completed_while_turning(&element.sprite, goal);
        let final_distance = (element.position_map() - goal).length();
        if flew
            && if approach {
                finished_selection
            } else {
                finished
            }
            && element.posture() == Posture::Upright
            && (final_distance < 0.1 || completed_turning_startup)
        {
            if element.sector() != Some(destination_sector) || element.layer() != destination.layer
            {
                return Err(format!(
                    "wrong landing sector/layer: {:?}/{}",
                    element.sector(),
                    element.layer()
                ));
            }
            actor_receiver_result(
                &engine,
                &assets,
                owner,
                destination_sector,
                destination.layer,
                element.position_map(),
            )?;
            if !expected_flight_seen {
                return Err(format!("jump did not execute {expected_flight:?}"));
            }
            if let Some(carrier) = carrier {
                if !shoulder_launched
                    || engine.ent(owner).human_data().unwrap().carrier.is_some()
                    || engine.ent(carrier).pc_data().unwrap().carried.is_some()
                {
                    return Err(format!(
                        "shoulder jump did not release the carrier: launched={shoulder_launched}"
                    ));
                }
                if engine.ent(carrier).element_data().posture() != Posture::Upright {
                    continue;
                }
                if engine
                    .entities()
                    .current_element_for_actor(carrier)
                    .and_then(|(id, index)| engine.seq().get_element(id, index))
                    .is_some_and(|selected| {
                        !selected.orders.is_empty() && selected.command != Command::Wait
                    })
                {
                    continue;
                }
                if engine.ent(carrier).element_data().sector() != Some(source_sector) {
                    return Err("carrier left the source sector".into());
                }
                actor_receiver_result(
                    &engine,
                    &assets,
                    carrier,
                    source_sector,
                    source.layer,
                    engine.ent(carrier).element_data().position_map(),
                )?;
            }
            if let Some(opponent) = opponent {
                let actor = engine.ent(owner);
                if !sword_flew
                    || actor.actor_data().unwrap().action_state
                        != crate::element_kinds::ActionState::WaitingSword
                    || !actor.human_data().unwrap().opponents.contains(&opponent)
                    || !engine
                        .ent(opponent)
                        .human_data()
                        .unwrap()
                        .opponents
                        .contains(&owner)
                {
                    return Err(format!(
                        "sword jump did not preserve combat: sword_flew={sword_flew}, state={:?}",
                        actor.actor_data().unwrap().action_state
                    ));
                }
            }
            return Ok(JumpArrival {
                final_distance,
                completed_turning_startup,
                turning_distance_loss: startup.turning_loss,
                trajectory,
            });
        }
    }
    let selected = engine
        .entities()
        .current_element_for_actor(owner)
        .and_then(|(id, index)| engine.seq().get_element(id, index));
    Err(format!(
        "jump stalled: flew={flew}, position={:?}, posture={:?}, goal={goal:?}, startup={startup:?}, selected={selected:?}, carrier={:?}",
        engine.ent(owner).element_data().position(),
        engine.ent(owner).element_data().posture(),
        carrier.map(|id| (
            engine.ent(id).element_data().posture(),
            engine.ent(id).actor_data().unwrap().action_state,
            engine.ent(id).element_data().sprite.last_motion_state,
            engine
                .entities()
                .current_element_for_actor(id)
                .and_then(|(sequence, index)| engine.seq().get_element(sequence, index))
        ))
    ))
}

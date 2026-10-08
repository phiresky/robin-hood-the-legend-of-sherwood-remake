use super::*;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct JumpArrival {
    final_distance: f32,
    completed_turning_startup: bool,
    turning_distance_loss: f32,
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
    audit_jump_dispatch(false);
}

#[test]
#[ignore = "requires exported jump pairs and ROBIN_CLIMB_RHS"]
fn exported_jumps_walk_to_launch_and_continue_after_landing() {
    audit_jump_dispatch(true);
}

fn audit_jump_dispatch(approach: bool) {
    let sprite = super::exported_stairs::complete_climb_sprite();
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
            for t in [0., 0.25, 0.5, 0.75, 1.] {
                let outcome = dispatch_jump(
                    engine.clone(),
                    assets.clone(),
                    &sprite,
                    index,
                    t,
                    approach,
                    approach_depth,
                );
                results.push(serde_json::json!({
                    "file": file, "line": index, "t": t,
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
    let report = serde_json::json!({
        "scope": if approach { "upright-walk-jump-walk-sequence-not-click-authorization-or-rendering" } else { "upright-sprite-dispatch-and-landing-not-click-approach-or-rendering" },
        "complete": !results.is_empty() && failed == 0, "results": results,
    });
    let path = directory.join(if approach {
        "actor-jump-approach-report.json"
    } else {
        "actor-jump-landing-report.json"
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(!results.is_empty());
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
    approach: bool,
    approach_depth: f32,
) -> Result<JumpArrival, String> {
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
    engine
        .get_projection_area_index(&assets, destination_sector, destination.layer, goal)
        .ok_or_else(|| format!("landing point has no receiver: {goal:?}"))?;
    let receiver = engine
        .get_projection_area_index(&assets, source_sector, source.layer, start)
        .ok_or_else(|| format!("launch point has no receiver: {start:?}"))?;
    let owner = walking_pc(&mut engine, &mut assets, start, source.layer, source_sector);
    let element = engine.ent_mut(owner).element_data_mut();
    let position = element.sprite.position_iface.clone();
    element.sprite = sprite.clone();
    element.sprite.position_iface = position;
    engine.set_obstacle_and_material(&assets, owner, Some(receiver));
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
    let sequence = if approach {
        let source_id = crate::jump_line::JumpLineIndex::new(index as u32).unwrap();
        let destination_id = crate::jump_line::JumpLineIndex::new(destination_index).unwrap();
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
            .ok_or("walk-jump-walk route construction failed")?
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
        sequence
    };
    let mut flew = false;
    let mut startup = StartupWalkAudit::default();
    for _ in 0..1000 {
        engine.control.frame_counter += 1;
        if approach {
            engine.t_hourglass_phase_sequences(&assets);
            engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
        }
        let before = engine.ent(owner).element_data().position_map();
        engine.t_tick_actor_owner_envelopes(&assets);
        let element = engine.ent(owner).element_data();
        if approach && flew {
            startup.observe(before, &element.sprite);
        }
        flew |= element.posture() == Posture::Flying;
        let finished = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .is_none_or(|element| element.orders.is_empty());
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
            return Ok(JumpArrival {
                final_distance,
                completed_turning_startup,
                turning_distance_loss: startup.turning_loss,
            });
        }
    }
    let selected = engine
        .entities()
        .current_element_for_actor(owner)
        .and_then(|(id, index)| engine.seq().get_element(id, index));
    Err(format!(
        "jump stalled: flew={flew}, position={:?}, posture={:?}, goal={goal:?}, startup={startup:?}, selected={selected:?}",
        engine.ent(owner).element_data().position(),
        engine.ent(owner).element_data().posture()
    ))
}

// Exercise authored ordinary passages before activation, after activation and after reset.
#[test]
#[ignore = "requires passage_controls probes in ROBIN_ASSET_MAP_DIAGNOSTICS and ROBIN_CLIMB_RHS"]
fn exported_ordinary_passage_controls_support_actor_round_trips() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let sprite = complete_climb_sprite();
    let sim = crate::sim_rng::test_context();
    let mut results = Vec::new();
    for entry in manifest["results"].as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let dims = &descriptor["walkable_polygon"][2];
        for probe in entry["passage_controls"].as_array().unwrap() {
            let (mut engine, assets) = compiled_walkway_with_dimensions(
                &bytes,
                (
                    dims[0].as_f64().unwrap() as f32 + 1.,
                    dims[1].as_f64().unwrap() as f32 + 1.,
                ),
            );
            let patches: Vec<_> = probe["patches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|patch| {
                    let index = patch["index"].as_u64().unwrap();
                    assert_eq!(
                        descriptor["asset_geometry"]["movement_transitions"][index as usize]["id"],
                        patch["id"]
                    );
                    crate::patch::PatchIndex::new(u32::try_from(index).unwrap()).unwrap()
                })
                .collect();
            let fixed_state = probe["fixed_state"].as_str();
            assert!(fixed_state.is_none_or(|state| matches!(state, "open" | "closed")));
            assert!(if fixed_state.is_some() {
                patches.is_empty()
            } else {
                !patches.is_empty()
            });
            let doors = probe["doors"].as_array().unwrap();
            assert!(!doors.is_empty());
            let states =
                fixed_state.map_or_else(|| vec!["closed", "open", "reset"], |state| vec![state]);
            for state in states {
                if state == "open"
                    && let Some(unlocks) = probe["unlock_before_open"].as_array()
                {
                    for door in unlocks {
                        let index = u32::try_from(door.as_u64().unwrap()).unwrap();
                        assert!(
                            engine.script_domains.interactables.doors[index as usize].unlockable
                        );
                        engine
                            .execute_unlock_door_done(crate::gate::DoorIndex::new(index).unwrap());
                    }
                }
                for &patch in &patches {
                    if state == "open" {
                        engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                    } else if state == "reset" {
                        engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                    }
                }
                for door in doors {
                    let index = usize::try_from(door.as_u64().unwrap()).unwrap();
                    let runtime = &engine.script_domains.interactables.doors[index];
                    // A rights change can affect NPCs while leaving PC access open.
                    let rights = &probe["rights_by_door"][index.to_string()][state];
                    let expected = if rights.is_null() {
                        state == "open"
                    } else {
                        for (key, actual) in [
                            ("locked_pc", runtime.locked_pc),
                            ("locked_npc_villain", runtime.locked_npc_villain),
                            ("locked_npc_civilian", runtime.locked_npc_civilian),
                            ("unlockable", runtime.unlockable),
                        ] {
                            assert_eq!(Some(actual), rights[key].as_bool(), "{file}: door {index}, {state}, {key}");
                        }
                        !rights["locked_pc"].as_bool().unwrap()
                    };
                    assert!(matches!(
                        runtime.door_type,
                        crate::gate::DoorType::Default | crate::gate::DoorType::Gate
                    ));
                    let mut gate_search = Vec::new();
                    if probe["reject_alternatives"] == true {
                        let auth = crate::gate::ActorAuthInfo {
                            kind: crate::element::ElementKind::ActorPc,
                            pc_auth_bit: 1,
                            has_lockpick: false,
                            has_climb: false,
                            has_jump: false,
                            is_rider: false,
                            posture: Posture::Upright,
                        };
                        for direct in [true, false] {
                            let (
                                source,
                                source_sector,
                                source_index,
                                goal,
                                goal_sector,
                                goal_index,
                            ) = if direct {
                                (
                                    runtime.point_out,
                                    runtime.sector_out,
                                    runtime.sector_out_index,
                                    runtime.point_in,
                                    runtime.sector_in,
                                    runtime.sector_in_index,
                                )
                            } else {
                                (
                                    runtime.point_in,
                                    runtime.sector_in,
                                    runtime.sector_in_index,
                                    runtime.point_out,
                                    runtime.sector_out,
                                    runtime.sector_out_index,
                                )
                            };
                            let path = crate::gate::find_path_gates_with_sector_indices(
                                &engine.script_domains.interactables.doors,
                                (source.x, source.y),
                                source_sector.into(),
                                source_index,
                                (goal.x, goal.y),
                                goal_sector.into(),
                                goal_index,
                                Some(&auth),
                                false,
                                &|_| true,
                                &|number| {
                                    engine
                                        .world
                                        .fast_grid
                                        .level
                                        .sectors
                                        .iter()
                                        .find(|sector| sector.sector_number == number)
                                        .and_then(|sector| sector.lift_type)
                                },
                            );
                            gate_search.push(serde_json::json!({
                                "direct": direct, "route_available": path.is_some(),
                                "passed": path.is_some() == expected,
                            }));
                        }
                    }
                    let outcome = walk_exported_lift_with_tick(
                        engine.clone(),
                        assets.clone(),
                        index,
                        index,
                        Some(&sprite),
                        |_, _, _| {},
                    );
                    let player_clicks: Vec<_> = if expected || probe["reject_alternatives"] == true
                    {
                        [true, false].into_iter().map(|direct| {
                            let outcome = click_exported_passage(
                                engine.clone(), assets.clone(), index, direct, expected, &sprite,
                            );
                            serde_json::json!({"direct": direct, "passed": outcome.is_ok(), "outcome": outcome})
                        }).collect()
                    } else {
                        Vec::new()
                    };
                    let npc_round_trips: Vec<_> = if rights.is_null() {
                        Vec::new()
                    } else {
                        [
                            ("soldier", "locked_npc_villain", StairDispatch::SoldierPrepared),
                            ("civilian", "locked_npc_civilian", StairDispatch::CivilianPrepared),
                        ].into_iter().map(|(kind, lock, dispatch)| {
                            let expected = !rights[lock].as_bool().unwrap();
                            let outcome = walk_exported_lift_route(
                                engine.clone(), assets.clone(), index, index, Some(&sprite), dispatch,
                                |_, _, _| {},
                            );
                            serde_json::json!({"actor": kind, "expected_round_trip": expected,
                                "passed": outcome == Ok(expected), "outcome": outcome})
                        }).collect()
                    };
                    results.push(serde_json::json!({
                        "file": file, "door": index, "state": state,
                        "expected_round_trip": expected,
                        "passed": outcome == Ok(expected) && gate_search.iter().all(|result| result["passed"] == true)
                            && player_clicks.iter().all(|result| result["passed"] == true)
                            && npc_round_trips.iter().all(|result| result["passed"] == true),
                        "npc_round_trips": npc_round_trips,
                        "player_clicks": player_clicks,
                        "gate_search": gate_search,
                        "outcome": outcome,
                    }));
                }
            }
        }
    }
    let complete = !results.is_empty() && results.iter().all(|result| result["passed"] == true);
    let report = directory.join("actor-passage-control-report.json");
    std::fs::write(&report, serde_json::to_vec_pretty(&serde_json::json!({
        "scope": "ordinary-passage-round-trips-player-clicks-and-optional-closed-route-rejection",
        "complete": complete, "results": results,
    })).unwrap()).unwrap();
    assert!(complete, "see {}", report.display());
}

fn click_exported_passage(
    mut engine: EngineInner,
    mut assets: LevelAssets,
    index: usize,
    direct: bool,
    expected_crossing: bool,
    sprite: &crate::sprite::Sprite,
) -> Result<(), String> {
    let door = engine.script_domains.interactables.doors[index].clone();
    let mut source = door.clone();
    let mut target = door.clone();
    // The landing selector reads only the outside endpoint fields. Keep the
    // actual runtime door unchanged and let player dispatch discover the route.
    if direct {
        target.point_out = door.point_in;
        target.sector_out = door.sector_in;
        target.sector_out_index = door.sector_in_index;
        target.layer_out = door.layer_in;
    } else {
        source.point_out = door.point_in;
        source.sector_out = door.sector_in;
        source.sector_out_index = door.sector_in_index;
        source.layer_out = door.layer_in;
    }
    let start = stair_landing_click(&engine, source.point_out, &source, false)?;
    let goal = stair_landing_click(&engine, start, &target, false)?;
    let source_sector = crate::position_interface::SectorHandle::from_number(source.sector_out)
        .with_arena_index(source.sector_out_index.unwrap());
    let target_sector = crate::position_interface::SectorHandle::from_number(target.sector_out)
        .with_arena_index(target.sector_out_index.unwrap());
    if source_sector == target_sector {
        return Err("ordinary passage does not separate two sectors".into());
    }
    let owner = walking_pc(
        &mut engine,
        &mut assets,
        start,
        source.layer_out,
        source_sector,
    );
    let element = engine.ent_mut(owner).element_data_mut();
    let position = element.sprite.position_iface.clone();
    element.sprite = sprite.clone();
    element.sprite.position_iface = position;
    let receiver =
        engine.get_projection_area_index(&assets, source_sector, source.layer_out, start);
    engine.set_obstacle_and_material(&assets, owner, receiver);
    let sim = crate::sim_rng::test_context();
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
    let mut previous = start;
    let mut stationary = 0;
    for _ in 0..2000 {
        engine.control.frame_counter += 1;
        engine.t_hourglass_phase_sequences(&assets);
        engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
        engine.t_tick_actor_owner_envelopes(&assets);
        let element = engine.ent(owner).element_data();
        let point = element.position_map();
        let sector = element.sector().ok_or("passage actor lost sector")?;
        if !expected_crossing && sector != source_sector {
            return Err(format!(
                "closed passage click left source sector: {sector:?}"
            ));
        }
        let passing = crate::engine::ai::selected_actor_is_passing_door(
            &engine.entities(),
            &engine.seq(),
            owner,
        );
        if !passing {
            actor_receiver_result(&engine, &assets, owner, sector, element.layer(), point)?;
        }
        if !passing
            && sector == target_sector
            && element.layer() == target.layer_out
            && (point - goal).length() < 0.02
        {
            return Ok(());
        }
        stationary = if previous == point { stationary + 1 } else { 0 };
        previous = point;
        if stationary >= 200 {
            if !expected_crossing && !passing && element.layer() == source.layer_out {
                return Ok(());
            }
            return Err(format!(
                "player passage stalled: {point:?}, goal {goal:?}, sector {sector:?}, target {target_sector:?}"
            ));
        }
    }
    Err(format!(
        "player passage exceeded movement budget: {previous:?}, goal {goal:?}"
    ))
}

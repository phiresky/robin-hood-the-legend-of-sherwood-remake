use super::*;

#[test]
#[ignore = "requires exported jump pairs and ROBIN_CLIMB_RHS"]
fn exported_jumps_complete_sprite_dispatch_and_land_on_receivers() {
    let sprite = super::exported_stairs::complete_climb_sprite();
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let mut results = vec![];
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
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
                let outcome = dispatch_jump(engine.clone(), assets.clone(), &sprite, index, t);
                results.push(serde_json::json!({
                    "file": file, "line": index, "t": t,
                    "passed": outcome.is_ok(), "error": outcome.err(),
                }));
            }
        }
    }
    let failed = results
        .iter()
        .filter(|result| result["passed"] != true)
        .count();
    let report = serde_json::json!({
        "scope": "upright-sprite-dispatch-and-landing-not-click-approach-or-rendering",
        "complete": !results.is_empty() && failed == 0, "results": results,
    });
    let path = directory.join("actor-jump-landing-report.json");
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
) -> Result<(), String> {
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
    let start = source.point_a + offset;
    let goal = destination.point_b + offset;
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
    let sequence = engine.t_launch_in_progress(&assets, jump);
    let sim = crate::sim_rng::test_context();
    if !engine.start_jump(
        TickCtx::new(&sim, &assets),
        owner,
        crate::sequence::SequenceElementRef::new(sequence, 0),
    ) {
        return Err("jump dispatch rejected the connection".into());
    }
    engine.select_sequence_element(owner, Some((sequence, 0)));
    let mut flew = false;
    for _ in 0..1000 {
        engine.control.frame_counter += 1;
        engine.t_tick_actor_owner_envelopes(&assets);
        let element = engine.ent(owner).element_data();
        flew |= element.posture() == Posture::Flying;
        let finished = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .is_none_or(|element| element.orders.is_empty());
        if flew
            && finished
            && element.posture() == Posture::Upright
            && (element.position_map() - goal).length() < 0.1
        {
            if element.sector() != Some(destination_sector) || element.layer() != destination.layer
            {
                return Err(format!(
                    "wrong landing sector/layer: {:?}/{}",
                    element.sector(),
                    element.layer()
                ));
            }
            return actor_receiver_result(
                &engine,
                &assets,
                owner,
                destination_sector,
                destination.layer,
                element.position_map(),
            );
        }
    }
    Err(format!(
        "jump stalled: flew={flew}, position={:?}, posture={:?}, goal={goal:?}",
        engine.ent(owner).element_data().position(),
        engine.ent(owner).element_data().posture()
    ))
}

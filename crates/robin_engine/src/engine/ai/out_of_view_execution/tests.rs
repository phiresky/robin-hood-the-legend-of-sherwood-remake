use super::*;
use crate::coordinates::{GroundPoint, WorldPoint3D};
use crate::element::{Camp, Detectable, DetectableType};
use crate::engine::test_support::actors::make_test_ai_soldier;

fn fixture() -> (EngineInner, LevelAssets, [EntityId; 3]) {
    let mut engine = EngineInner::new();
    let sector = crate::engine::test_support::ensure_ordinary_sector(&mut engine, 1, 0);
    let ids = [Camp::Lacklandists, Camp::Royalists, Camp::Royalists].map(|camp| {
        let mut entity = make_test_ai_soldier(camp);
        entity.element_data_mut().set_sector(Some(sector));
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(1546.6582, 318.29956, 0.0));
        engine.add_test_entity(entity)
    });
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = 920;
    let [owner, primary, lost] = ids;
    let entity = engine.world.entities.get_mut(owner).unwrap();
    entity.element_data_mut().set_direction_instantly(14);
    entity.ai_actor_data_mut().unwrap().view_radius = 100;
    entity.ai_actor_data_mut().unwrap().stare_point = GroundPoint::new(1500.6963, 344.66223);
    entity
        .human_data_mut()
        .unwrap()
        .opponents
        .add_principal(lost, None);
    entity.ai_actor_data_mut().unwrap().detectable_lists[DetectableType::Enemy as usize] =
        vec![Detectable {
            element: Some(primary),
            detectable_type: DetectableType::Enemy,
            seen_now: true,
            ..Default::default()
        }];
    let ai = engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("out-of-view fixture"));
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingSwordfight;
    ai.base.primary_target = Some(AiEntityHandle::new(primary.index()));
    ai.list_them = vec![primary.index(), lost.index()];
    (engine, assets, ids)
}

#[test]
fn perpendicular_stare_uses_literal_direction_and_raw_ground_coordinates() {
    let (mut engine, _, [owner, _, _]) = fixture();
    for elevation in [0.0, 160.0] {
        let actor = engine.world.entities.get_mut(owner).unwrap();
        actor
            .element_data_mut()
            .set_position(WorldPoint3D::new(1546.6582, 318.29956, elevation));
        actor.ai_actor_data_mut().unwrap().stare_point.y = 344.66223;
        assert!(
            !engine.live_enemy_is_behind_me(owner),
            "elevation {elevation}"
        );
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .ai_actor_data_mut()
            .unwrap()
            .stare_point
            .y = 344.67224;
        assert!(
            engine.live_enemy_is_behind_me(owner),
            "elevation {elevation}"
        );
    }
}

#[test]
fn out_of_view_compares_ai_primary_instead_of_actor_principal() {
    let (mut engine, assets, [owner, _, lost]) = fixture();
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .ai_actor_data_mut()
        .unwrap()
        .stare_point
        .y = 344.67224;
    let sim = crate::sim_rng::test_context();
    let seed = sim.seed();
    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(!engine.execute_ai_out_of_view(
        &sim,
        &assets,
        owner,
        &Stimulus::with_human(StimulusType::EventOutOfView, lost.index())
    ));
    let queries = crate::sight_obstacle::take_parity_visibility_capture();
    assert!(
        queries.is_empty(),
        "non-primary loss must skip the 360-degree query"
    );
    assert_eq!(sim.seed(), seed);
    assert_eq!(
        engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("ignored loss"))
            .missed_pc,
        None
    );
}

#[test]
fn perpendicular_non_primary_loss_rebuilds_from_current_detectables() {
    let (mut engine, assets, [owner, primary, lost]) = fixture();
    engine.execute_ai_out_of_view(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        &Stimulus::with_human(StimulusType::EventOutOfView, lost.index()),
    );
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("processed loss"));
    assert_eq!(ai.list_them, vec![primary.index()]);
    assert_eq!(ai.missed_pc, Some(AiEntityHandle::new(lost.index())));
    assert!(ai.pc_missed);
}

#[test]
fn removed_detectable_forecasts_the_current_stimulus_target_lazily() {
    let (mut engine, assets, [owner, primary, lost]) = fixture();
    engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("stationary observation"))
        .base
        .current_substate = Substate::AttackingObserve;
    let point = WorldPoint3D::new(1226.1754, 315.8716, 0.0);
    engine
        .world
        .entities
        .get_mut(lost)
        .unwrap()
        .element_data_mut()
        .set_position(point);
    let expected = engine.live_ai_position(lost);
    assert_ne!(expected, engine.live_ai_position(primary));
    engine.execute_ai_out_of_view(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        &Stimulus::with_human(StimulusType::EventOutOfView, lost.index()),
    );
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("live target forecast"));
    assert_eq!(ai.missed_pc, Some(AiEntityHandle::new(lost.index())));
    assert_eq!(ai.base.seek_position, expected);
    assert_eq!(
        ai.base.primary_target,
        Some(AiEntityHandle::new(primary.index()))
    );
}

//! Bow-shot unit and parity regression tests.

use super::*;
use crate::coordinates::{MapVec, SpriteFrameOffset, SpriteLocalPoint};
use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};
use crate::engine::TickCtx;
use crate::engine::test_support::actors::TestActor;
use crate::sequence::SequenceElementRef;
use crate::sequence::SequenceId;
use crate::sprite_script::SpriteScript;

fn begin_test_bow_shot(
    entities: &mut Entities,
    sequences: &mut SequenceManager,
    owner: EntityId,
    target: EntityId,
    sequence: SequenceId,
    element: usize,
    once: bool,
    ammo: u32,
    mode: Option<ShootMode>,
    next_order_id: &mut u32,
) -> BeginShotResult {
    entities
        .get_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .selected_sequence_element =
        Some(crate::sequence::SequenceElementRef::new(sequence, element));
    super::begin_bow_shot(
        entities,
        sequences,
        owner,
        target,
        SequenceElementRef::new(sequence, element),
        once,
        ammo,
        mode,
        next_order_id,
    )
}

fn run_test_bow_owner(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    sequences: &mut SequenceManager,
    owner: EntityId,
    frozen: bool,
) {
    let Some(selected) = entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .selected_sequence_element
    else {
        return;
    };
    let sequence = selected.sequence_id;
    let Some(order) = sequences
        .get_element(sequence, selected.element_index)
        .and_then(SequenceElement::current_order)
    else {
        return;
    };
    let order_id = order.order_id;
    let mut engine = crate::engine::EngineInner::new();
    engine.world.entities = std::mem::take(entities);
    engine.orders.sequence_manager = std::mem::replace(sequences, SequenceManager::new());
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .selected_sequence_element = Some(crate::sequence::SequenceElementRef::new(
        sequence,
        selected.element_index,
    ));
    engine.publish_selected_order_as_installed(owner);
    engine.control.set_actors_frozen(frozen);
    engine.tick_bow_shot_for(
        TickCtx::new(sim, &crate::engine::LevelAssets::new()),
        owner,
        order_id,
    );
    *entities = engine.world.entities;
    *sequences = engine.orders.sequence_manager;
}

fn test_bow_done_pulse(entities: &Entities, owner: EntityId) -> bool {
    entities.get(owner).unwrap().sprite().last_motion_state == Some(SpriteMotionState::Done)
}

fn bow_owner_engine(
    entities: Entities,
    owner: EntityId,
) -> (crate::engine::EngineInner, crate::engine::LevelAssets) {
    let (mut engine, mut assets) = projectile_engine(entities);
    let pc = engine.world.entities.get(owner).unwrap().pc_data().unwrap();
    let profile_index = usize::from(pc.profile_index);
    let description_index = pc.campaign_description_index.unwrap() as usize;
    engine.mission_domain.campaign.characters[description_index]
        .status
        .num_arrows = 10;
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.characters[profile_index].shooting_weapon_id = 1;
    profiles.characters[profile_index].shooting = 100;
    profiles.bows.push(crate::profiles::BowProfile {
        normal_shoot: crate::profiles::BowShootMode {
            range: 2000,
            ..Default::default()
        },
        ..Default::default()
    });
    (engine, assets)
}
fn test_selected_bow(entities: &Entities, sequences: &SequenceManager, owner: EntityId) -> bool {
    let Some(selected) = entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .selected_sequence_element
    else {
        return false;
    };
    sequences
        .get_element(selected.sequence_id, selected.element_index)
        .and_then(SequenceElement::current_order)
        .is_some_and(|order| is_active_bow_order(order.order_type))
}

trait TestEntityIndexAccess {
    fn get_at_index(&self, index: u32) -> Option<(EntityId, &Entity)>;
    fn get_mut_at_index(&mut self, index: u32) -> Option<(EntityId, &mut Entity)>;
}

impl TestEntityIndexAccess for Entities {
    fn get_at_index(&self, index: u32) -> Option<(EntityId, &Entity)> {
        self.get_legacy_slot(index)
    }

    fn get_mut_at_index(&mut self, index: u32) -> Option<(EntityId, &mut Entity)> {
        self.get_legacy_slot_mut(index)
    }
}

fn entity_table(slots: Vec<Option<Entity>>) -> Entities {
    let mut entities = Entities::new();
    for slot in slots {
        entities.push(slot);
    }
    entities
}

fn projectile_engine(
    entities: Entities,
) -> (crate::engine::EngineInner, crate::engine::LevelAssets) {
    let mut engine = crate::engine::EngineInner::new();
    engine.world.entities = entities;
    for (_, projectile) in engine.world.entities.projectiles_mut() {
        // These fixtures launch from ground-level actors even when the
        // projectile itself temporarily has no current navigation layer.
        projectile.projectile.trajectory_origin_layer = crate::position_interface::Layer::new(0);
    }
    let creation_orders = engine
        .world
        .entities
        .occupied()
        .map(|(id, _)| (id, id.index()))
        .collect();
    let next = engine
        .world
        .entities
        .occupied()
        .map(|(id, _)| id.index())
        .max()
        .map_or(0, |index| index + 1);
    engine
        .world
        .install_original_creation_orders(creation_orders, next);
    let mut assets = engine.test_runtime_assets();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    (engine, assets)
}

fn make_arrow_target(x: f32, y: f32) -> Entity {
    let mut element =
        crate::engine::test_support::extra_engine_combat::test_element(ElementKind::Target, true);
    element.set_position_map(MapPoint { x, y });
    element.set_position(WorldPoint3D { x, y, z: 0.0 });
    Entity::Target(ElementTarget {
        element,
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::ARROW,
            ..TargetData::default()
        },
    })
}

/// Test helper — launch a `ShootBow` sequence element and return
/// `(sequence_manager, seq_id, elem_idx)` so tests can hand the
/// triple to `begin_bow_shot` and exact-owner execution.
fn launch_test_shoot_element(
    shooter: EntityId,
    target: EntityId,
) -> (SequenceManager, SequenceId, usize) {
    let mut sm = SequenceManager::new();
    let elem = build_shoot_bow_element(shooter, target);
    let seq_id = sm.insert_element(elem);
    sm.start_sequence_level(seq_id);
    // Model the running bow element independently of instruction dispatch.
    sm.get_element_mut(seq_id, 0).unwrap().state = crate::sequence::SequenceState::InProgress;
    sm.get_sequence_mut(seq_id)
        .unwrap()
        .increase_elements_in_progress();
    sm.rebuild_indices();
    (sm, seq_id, 0)
}

fn set_test_action_state_after_transition(
    sm: &mut SequenceManager,
    elem_ref: SequenceElementRef,
    action_state: ActionState,
) {
    sm.get_element_at_mut(elem_ref)
        .unwrap()
        .action_state_after_transition = action_state;
}

fn bind_test_bow_release_rows(entity: &mut Entity, order_type: OrderType) {
    let mut conversion = crate::engine::test_support::unmapped_conversion();
    let base_row = 0u16;
    conversion[order_type as usize] = base_row;

    let mut scripts = Vec::with_capacity(16);
    for _direction in 0..16 {
        scripts.push(SpriteScript {
            action_id: order_type as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: SpriteLocalPoint::new(2.0, 3.0),
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0, 0, 0],
        });
    }

    let sprite = &mut entity.element_data_mut().sprite;
    sprite.scripts = std::sync::Arc::new(scripts);
    sprite.conversion = std::sync::Arc::new(conversion);
}

fn make_pc(x: f32, y: f32) -> Entity {
    TestActor::pc(Posture::Undefined)
        .map_position(MapPoint { x, y })
        .build()
}

fn make_anonymous_pc(x: f32, y: f32) -> Entity {
    let mut pc = make_pc(x, y);
    pc.element_data_mut()
        .publish_order_posture(Posture::AnonymousArcher);
    pc
}

fn make_soldier(x: f32, y: f32) -> Entity {
    make_soldier_with_camp(x, y, crate::element::Camp::Royalists)
}

fn make_soldier_with_camp(x: f32, y: f32, camp: crate::element::Camp) -> Entity {
    TestActor::soldier(Posture::Undefined)
        .map_position(MapPoint { x, y })
        .life_points(100)
        .camp(camp)
        .build()
}

/// Savegame_Nescafe/Profile_002/Continue replay-007 reaches these exact
/// bits at Original frame 590. `new - increment` shifts the old X by one
/// bit and fails the strict range gate; the movement step's saved old position hits.
#[test]
fn existing_arrow_collision_uses_new_move_old_position() {
    let mut victim = make_pc(1_040.648_1, 1_915.162_7);
    victim
        .element_data_mut()
        .set_position(WorldPoint3D::new(1_040.648_1, 1_915.162_7, 0.0));
    let shooter = make_soldier_with_camp(772.0, 1796.0, crate::element::Camp::Lacklandists);

    let mut element = crate::engine::test_support::extra_engine_combat::test_element(
        ElementKind::ObjectProjectile,
        true,
    );
    let saved_old = WorldPoint3D::new(987.105_4, 1_922.524_8, 68.750_26);
    element.set_position(saved_old);
    element.set_position_map_preserving_3d(MapPoint::new(987.105_4, 1_853.774_5));
    let arrow = Entity::Projectile(ElementProjectile {
        element,
        object: ObjectData {
            associated_action: Action::Bow,
            object_type: ObjectType::Arrow,
            animation: Animation::ObjectFlying,
            ..ObjectData::default()
        },
        projectile: ProjectileData {
            flying: true,
            trajectory_frame_count: 1,
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D::new(1_070.693_6, 1_911.031_5, 0.0),
                time: 1,
            }],
            velocity_increment: WorldVec3D::new(53.542_618, -7.362_060_5, -43.750_25),
            damage: 10,
            ..ProjectileData::default()
        },
    });
    let mut entities = entity_table(vec![Some(victim), Some(shooter), Some(arrow)]);
    let victim_id = entities.get_at_index(0).expect("victim slot").0;
    let shooter_id = entities.get_at_index(1).expect("shooter slot").0;
    let (_, arrow) = entities.get_mut_at_index(2).expect("arrow slot");
    let Entity::Projectile(arrow) = arrow else {
        panic!("arrow slot did not retain projectile")
    };
    arrow.object.reference = Some(victim_id);
    arrow.projectile.shooter = Some(shooter_id);

    let integrated = WorldPoint3D::new(
        saved_old.x + arrow.projectile.velocity_increment.x,
        saved_old.y + arrow.projectile.velocity_increment.y,
        saved_old.z + arrow.projectile.velocity_increment.z,
    );
    let reconstructed_old = WorldPoint3D::new(
        integrated.x - arrow.projectile.velocity_increment.x,
        integrated.y - arrow.projectile.velocity_increment.y,
        integrated.z - arrow.projectile.velocity_increment.z,
    );
    assert_ne!(
        reconstructed_old.x, saved_old.x,
        "fixture must exercise the non-reversible f32 boundary"
    );
    let norm = |from: WorldPoint3D, to: WorldPoint3D| {
        let delta = to - from;
        delta.norm()
    };
    let belt = WorldPoint3D::new(1_040.648_1, 1_915.162_7, 25.0);
    assert!(norm(reconstructed_old, belt) > norm(reconstructed_old, integrated));
    assert!(norm(saved_old, belt) <= norm(saved_old, integrated));

    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let actor_order: Vec<EntityId> = entities.actors().map(|(id, _)| id.into()).collect();
    let old_position = {
        let Entity::Projectile(arrow) = entities.get_mut(arrow_id).unwrap() else {
            unreachable!()
        };
        if let Some(old) = arrow.projectile.launch_segment_start.take() {
            old
        } else {
            let old = arrow.element.position();
            arrow.advance_projectile_hourglass();
            old
        }
    };
    let hit = projectile_human_victim(
        &entities,
        &actor_order,
        &crate::diplomacy::DiplomacyState::default(),
        arrow_id,
        old_position,
    );
    assert_eq!(hit, Some(victim_id));
}

#[test]
fn begin_bow_shot_sets_shooter_state() {
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        None,
        &mut 1u32,
    );
    assert_eq!(result, BeginShotResult::Started);

    let actor = entities
        .get_at_index(0)
        .map(|(_, entity)| entity)
        .unwrap()
        .actor_data()
        .unwrap();
    assert_eq!(
        actor.action_state,
        ActionState::Waiting,
        "shoot-bow translation must not force the actor's action state before queued bow orders run"
    );
    assert!(
        sm.get_element(seq_id, elem_idx)
            .unwrap()
            .orders
            .iter()
            .any(|order| order.order_type == OrderType::ShootingWithBow
                && order.target_actor == Some(target_id.index()))
    );
    // Should have: shoot order + reload order (and possibly transition orders)
    assert!(sm.get_element(seq_id, elem_idx).unwrap().orders.len() >= 2);
}

#[test]
#[should_panic(expected = "bow shot translation lost sequence element")]
fn begin_bow_shot_rejects_a_missing_sequence_element_as_corrupt_state() {
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let mut sequences = SequenceManager::new();

    let _ = begin_test_bow_shot(
        &mut entities,
        &mut sequences,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        SequenceId(999),
        0,
        false,
        10,
        None,
        &mut 1u32,
    );
}

#[test]
fn unbound_bow_sprite_does_not_synthesize_a_release_pulse() {
    let sim = crate::sim_rng::test_context();
    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let target = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let (mut sequences, sequence, element) = launch_test_shoot_element(owner, target);
    begin_test_bow_shot(
        &mut entities,
        &mut sequences,
        owner,
        target,
        sequence,
        element,
        false,
        10,
        Some(ShootMode::Normal),
        &mut 1,
    );
    let order = sequences
        .get_element(sequence, element)
        .unwrap()
        .current_order()
        .unwrap()
        .order_id;
    for _ in 0..4 {
        run_test_bow_owner(&sim, &mut entities, &mut sequences, owner, false);
    }
    assert!(!test_bow_done_pulse(&entities, owner));
    assert_eq!(
        sequences
            .get_element(sequence, element)
            .unwrap()
            .current_order()
            .unwrap()
            .order_id,
        order
    );
}

#[test]
fn bow_done_pulse_fires_once_and_stays_consumed_after_state_clone() {
    let sim = crate::sim_rng::test_context();
    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let target = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let mut shooter = make_pc(0.0, 0.0);
    shooter
        .element_data_mut()
        .publish_order_posture(Posture::Upright);
    shooter.actor_data_mut().unwrap().action_state = ActionState::AimingWithBow;
    bind_test_bow_release_rows(&mut shooter, OrderType::ShootingWithBow);
    let target_actor = TestActor::soldier(Posture::Upright)
        .map_position(MapPoint::new(50.0, 0.0))
        .life_points(100)
        .build();
    let mut entities = entity_table(vec![Some(shooter), Some(target_actor)]);
    let (mut sequences, sequence, element) = launch_test_shoot_element(owner, target);
    begin_test_bow_shot(
        &mut entities,
        &mut sequences,
        owner,
        target,
        sequence,
        element,
        false,
        10,
        Some(ShootMode::Normal),
        &mut 1,
    );
    let orders = &mut sequences.get_element_mut(sequence, element).unwrap().orders;
    orders.truncate(1);
    orders.push_back(Order::new(
        OrderType::WaitingUpright,
        0.0,
        0.0,
        std::num::NonZeroU32::new(999).unwrap(),
    ));

    let (mut engine, assets) = bow_owner_engine(entities, owner);
    engine.orders.sequence_manager = sequences;
    assert_eq!(
        engine.can_shoot_with_bow_at(&assets, owner, target).0,
        crate::engine::input::BowTarget::Valid
    );
    let mut pulse_count = 0;
    let mut restored = None;
    for _ in 0..12 {
        engine.tick_one_actor_animation_action_change_slot(TickCtx::new(&sim, &assets), owner);
        if test_bow_done_pulse(&engine.world.entities, owner) {
            pulse_count += 1;
            restored = Some(engine.clone());
        }
    }
    assert_eq!(
        pulse_count, 1,
        "one authored animation yields one release pulse"
    );
    let mut engine = restored.expect("captured state immediately after DONE");
    for _ in 0..8 {
        engine.tick_one_actor_animation_action_change_slot(TickCtx::new(&sim, &assets), owner);
        assert!(
            !test_bow_done_pulse(&engine.world.entities, owner),
            "restored animation must not release again"
        );
    }
}

#[test]
fn tick_bow_shots_detaches_when_sequence_has_advanced_past_bow_orders() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        None,
        &mut 1u32,
    );
    assert_eq!(result, BeginShotResult::Started);
    let orders = &mut sm.get_element_mut(seq_id, elem_idx).unwrap().orders;
    orders.clear();
    let mut next_order_id = 1000;
    orders.push_back(Order::new(
        OrderType::WalkingUpright,
        0.0,
        0.0,
        crate::order::alloc_order_id(&mut next_order_id),
    ));

    run_test_bow_owner(
        sim,
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        false,
    );

    assert!(!test_bow_done_pulse(
        &entities,
        entities.actors().next().unwrap().0.into()
    ));

    assert!(
        !test_selected_bow(&entities, &sm, EntityId::Pc(crate::entity_id::PcId(0))),
        "shoot-list ownership ends once the sequence has no bow orders left"
    );
}

#[test]
fn single_owner_tick_preserves_replaced_other_actor_shot() {
    let sim_context = crate::sim_rng::test_context();
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_pc(5.0, 0.0)),
        Some(make_soldier(50.0, 0.0)),
    ]);
    let first = EntityId::Pc(crate::entity_id::PcId(0));
    let other = EntityId::Pc(crate::entity_id::PcId(1));
    let target = EntityId::Soldier(crate::entity_id::SoldierId(2));
    let mut sm = SequenceManager::new();
    let first_seq = sm.insert_element(build_shoot_bow_element(first, target));
    sm.start_sequence_level(first_seq);
    sm.get_element_mut(first_seq, 0).unwrap().state = crate::sequence::SequenceState::InProgress;
    sm.rebuild_indices();
    let other_seq = sm.insert_element(build_shoot_bow_element(other, target));
    sm.start_sequence_level(other_seq);
    sm.get_element_mut(other_seq, 0).unwrap().state = crate::sequence::SequenceState::InProgress;
    sm.rebuild_indices();
    let mut next_order_id = 1;
    assert_eq!(
        begin_test_bow_shot(
            &mut entities,
            &mut sm,
            first,
            target,
            first_seq,
            0,
            false,
            10,
            None,
            &mut next_order_id,
        ),
        BeginShotResult::Started
    );
    assert_eq!(
        begin_test_bow_shot(
            &mut entities,
            &mut sm,
            other,
            target,
            other_seq,
            0,
            false,
            10,
            None,
            &mut next_order_id,
        ),
        BeginShotResult::Started
    );
    let replacement = crate::sequence::SequenceElementRef::new(
        sm.insert_element(build_shoot_bow_element(other, target)),
        0,
    );

    // A synchronous operation has replaced another actor's shot before this
    // owner resumes. The production owner-only tick must not overwrite it.
    entities
        .get_mut(other)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .selected_sequence_element = Some(replacement);

    run_test_bow_owner(&sim_context, &mut entities, &mut sm, first, false);

    assert_eq!(
        entities
            .get(other)
            .unwrap()
            .actor_data()
            .unwrap()
            .selected_sequence_element,
        Some(replacement),
        "single-owner bow execution must preserve a synchronous cross-actor replacement"
    );
}

#[test]
fn frozen_owner_bow_initialises_direction_without_advancing_sprite_or_order() {
    let sim = crate::sim_rng::test_context();
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(40.0, 0.0))]);
    let shooter = EntityId::Pc(crate::entity_id::PcId(0));
    let target = EntityId::Soldier(crate::entity_id::SoldierId(1));
    entities
        .get_mut(shooter)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::AimingWithBow;
    let mut sm = SequenceManager::new();
    let seq = sm.insert_element(build_shoot_bow_element(shooter, target));
    sm.start_sequence_level(seq);
    sm.get_element_mut(seq, 0).unwrap().state = crate::sequence::SequenceState::InProgress;
    sm.rebuild_indices();
    let mut next_order_id = 1;
    assert_eq!(
        begin_test_bow_shot(
            &mut entities,
            &mut sm,
            shooter,
            target,
            seq,
            0,
            false,
            10,
            None,
            &mut next_order_id
        ),
        BeginShotResult::Started
    );
    let order = sm
        .get_element(seq, 0)
        .unwrap()
        .current_order()
        .unwrap()
        .clone();
    let before_sprite = entities.get(shooter).unwrap().sprite().clone();
    // The shoot order samples its target only while the owner slot has
    // the execute order in its initialising window; arm it the way the
    // production Execute path does.
    entities
        .get_mut(shooter)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = true;

    run_test_bow_owner(&sim, &mut entities, &mut sm, shooter, true);

    assert!(!test_bow_done_pulse(
        &entities,
        entities.actors().next().unwrap().0.into()
    ));

    let entity = entities.get(shooter).unwrap();
    assert_eq!(
        i16::from(entity.position_iface().get_direction_goal()),
        crate::position_interface::vector_to_sector_0_to_15_iso(40.0, 0.0)
    );
    assert_eq!(
        entity.sprite().last_processed_order_id,
        before_sprite.last_processed_order_id
    );
    assert_eq!(
        sm.get_element(seq, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .order_id,
        order.order_id
    );
}

#[test]
fn tick_bow_shots_waits_behind_pre_shoot_setup_order() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        None,
        &mut 1u32,
    );
    assert_eq!(result, BeginShotResult::Started);
    let mut next_order_id = 1000;
    sm.get_element_mut(seq_id, elem_idx).unwrap().orders.insert(
        0,
        Order::new(
            OrderType::TransitionWaitingUprightBoredWaitingUpright,
            0.0,
            0.0,
            crate::order::alloc_order_id(&mut next_order_id),
        ),
    );

    run_test_bow_owner(
        sim,
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        false,
    );

    assert!(!test_bow_done_pulse(
        &entities,
        entities.actors().next().unwrap().0.into()
    ));

    assert!(
        sm.get_element(seq_id, elem_idx)
            .unwrap()
            .orders
            .iter()
            .any(|order| is_shoot_order(order.order_type)),
        "pre-shoot setup orders should not cancel the pending bow shot"
    );
}

#[test]
fn tick_bow_shots_detaches_before_trailing_non_bow_order() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut entities = entity_table(vec![
        Some(
            TestActor::pc(Posture::Upright)
                .map_position(MapPoint::new(0.0, 0.0))
                .build(),
        ),
        Some(
            TestActor::soldier(Posture::Upright)
                .map_position(MapPoint::new(50.0, 0.0))
                .life_points(100)
                .build(),
        ),
    ]);
    entities
        .get_mut(EntityId::Pc(crate::entity_id::PcId(0)))
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::AimingWithBow;
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    bind_test_bow_release_rows(
        entities
            .get_mut_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap(),
        OrderType::ShootingWithBow,
    );
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        Some(ShootMode::Normal),
        &mut 1u32,
    );
    assert_eq!(result, BeginShotResult::Started);

    let mut next_order_id = 1000;
    let orders = &mut sm.get_element_mut(seq_id, elem_idx).unwrap().orders;
    orders.clear();
    orders.push_back(Order::new(
        OrderType::ShootingWithBow,
        0.0,
        0.0,
        crate::order::alloc_order_id(&mut next_order_id),
    ));
    orders.push_back(Order::new(
        OrderType::TransitionWaitingUprightBoredWaitingUpright,
        0.0,
        0.0,
        crate::order::alloc_order_id(&mut next_order_id),
    ));

    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let (mut engine, assets) = bow_owner_engine(entities, owner);
    engine.orders.sequence_manager = sm;
    assert_eq!(
        engine.can_shoot_with_bow_at(&assets, owner, target_id).0,
        crate::engine::input::BowTarget::Valid
    );
    let mut released = false;
    for _ in 0..64 {
        engine.tick_one_actor_animation_action_change_slot(
            TickCtx::new(sim, &assets),
            EntityId::Pc(crate::entity_id::PcId(0)),
        );
        released |= test_bow_done_pulse(
            &engine.world.entities,
            EntityId::Pc(crate::entity_id::PcId(0)),
        );
        if !test_selected_bow(
            &engine.world.entities,
            &engine.orders.sequence_manager,
            EntityId::Pc(crate::entity_id::PcId(0)),
        ) {
            break;
        }
    }

    assert!(released);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress
    );
    assert!(
        !test_selected_bow(
            &engine.world.entities,
            &engine.orders.sequence_manager,
            EntityId::Pc(crate::entity_id::PcId(0))
        ),
        "active bow-shot driver should detach after the final bow order"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .unwrap()
            .current_order()
            .unwrap()
            .order_type,
        OrderType::TransitionWaitingUprightBoredWaitingUpright
    );
}

#[test]
#[should_panic(expected = "bow release requires an aiming action")]
fn bow_release_rejects_non_aiming_action_state() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        None,
        &mut 1u32,
    );
    assert_eq!(result, BeginShotResult::Started);
    let facing = crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 0.0);
    let shooter = entities
        .get_mut_at_index(0)
        .map(|(_, entity)| entity)
        .unwrap();
    shooter.element_data_mut().set_direction_instantly(facing);
    shooter.actor_data_mut().unwrap().action_state = ActionState::Waiting;
    bind_test_bow_release_rows(shooter, OrderType::ShootingWithBow);

    for _ in 0..16 {
        run_test_bow_owner(
            sim,
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            false,
        );
    }
}

#[test]
fn begin_bow_shot_keeps_current_aim_state_until_transition_pulse() {
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    entities
        .get_mut_at_index(0)
        .map(|(_, entity)| entity)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::AimingWithBow;
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    set_test_action_state_after_transition(
        &mut sm,
        SequenceElementRef::new(seq_id, elem_idx),
        ActionState::AimingWithBow,
    );

    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        Some(ShootMode::Long),
        &mut 1u32,
    );

    assert_eq!(result, BeginShotResult::Started);
    let actor = entities
        .get_at_index(0)
        .map(|(_, entity)| entity)
        .unwrap()
        .actor_data()
        .unwrap();
    assert_eq!(actor.action_state, ActionState::AimingWithBow);
    assert!(
        sm.get_element(seq_id, elem_idx)
            .unwrap()
            .orders
            .iter()
            .any(|order| order.order_type == OrderType::ShootingWithBowUp)
    );
    let orders: Vec<OrderType> = sm
        .get_element(seq_id, elem_idx)
        .unwrap()
        .orders
        .iter()
        .map(|o| o.order_type)
        .collect();
    assert_eq!(orders[0], OrderType::TransitionRaisingBow);
    assert_eq!(orders[1], OrderType::ShootingWithBowUp);
}

#[test]
fn begin_bow_shot_uses_action_state_after_transition_for_setup_orders() {
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    set_test_action_state_after_transition(
        &mut sm,
        SequenceElementRef::new(seq_id, elem_idx),
        ActionState::AimingWithBow,
    );

    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        Some(ShootMode::Long),
        &mut 1u32,
    );

    assert_eq!(result, BeginShotResult::Started);
    let orders: Vec<OrderType> = sm
        .get_element(seq_id, elem_idx)
        .unwrap()
        .orders
        .iter()
        .map(|o| o.order_type)
        .collect();
    assert_eq!(
        orders[0],
        OrderType::TransitionRaisingBow,
        "the original game delays the action-state change, so a first long shot after equip/load still raises the bow before shooting"
    );
    assert_eq!(orders[1], OrderType::ShootingWithBowUp);
}

#[test]
fn begin_bow_shot_accepts_active_target_that_died_while_aiming() {
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    if let Some((_, Entity::Soldier(s))) = entities.get_mut_at_index(1) {
        s.npc.life_points = 0; // dead
    }
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        None,
        &mut 1u32,
    );
    assert_eq!(result, BeginShotResult::Started);
    assert!(
        sm.get_element(seq_id, elem_idx)
            .unwrap()
            .orders
            .iter()
            .any(|order| matches!(
                order.order_type,
                OrderType::ShootingWithBow | OrderType::ShootingWithBowUp
            ))
    );
}

#[test]
fn begin_bow_shot_accepts_retained_inactive_human_target() {
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    if let Some((_, Entity::Soldier(target))) = entities.get_mut_at_index(1) {
        target.element.active = false;
    }
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        None,
        &mut 1u32,
    );

    assert_eq!(result, BeginShotResult::Started);
    assert!(
        sm.get_element(seq_id, elem_idx)
            .unwrap()
            .orders
            .iter()
            .any(|order| matches!(
                order.order_type,
                OrderType::ShootingWithBow | OrderType::ShootingWithBowUp
            )),
        "a retained inactive human remains a valid shot target in Original"
    );
}

#[test]
fn begin_bow_shot_accepts_arrow_fx_target() {
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_arrow_target(50.0, 0.0)),
    ]);
    let target_id = EntityId::Target(crate::entity_id::TargetId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        None,
        &mut 1u32,
    );
    assert_eq!(result, BeginShotResult::Started);
    assert!(matches!(sm.get_element(seq_id, elem_idx).unwrap().data,
        SequenceElementData::Interaction { antagonist: Some(target) } if target == target_id));
}

#[test]
fn begin_bow_shot_uses_anonymous_shoot_orders() {
    let mut entities = entity_table(vec![
        Some(make_anonymous_pc(0.0, 0.0)),
        Some(make_soldier(50.0, 0.0)),
    ]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        Some(ShootMode::Normal),
        &mut 1u32,
    );

    assert_eq!(result, BeginShotResult::Started);
    let orders: Vec<OrderType> = sm
        .get_element(seq_id, elem_idx)
        .unwrap()
        .orders
        .iter()
        .map(|o| o.order_type)
        .collect();
    assert_eq!(orders[0], OrderType::ShootingWithBowAnonymous);
    assert_eq!(orders[1], OrderType::TransitionLoadingBowAnonymous);
}

#[test]
fn begin_bow_shot_preserves_facing_until_shoot_order_initialization() {
    let mut target = make_arrow_target(50.0, 120.0);
    target
        .element_data_mut()
        .set_position(WorldPoint3D::new(50.0, 120.0, 100.0));
    let mut entities = entity_table(vec![Some(make_pc(0.0, 100.0)), Some(target)]);
    entities
        .get_mut_at_index(0)
        .map(|(_, entity)| entity)
        .unwrap()
        .element_data_mut()
        .set_direction_goal(7);
    let target_id = EntityId::Target(crate::entity_id::TargetId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        None,
        &mut 1u32,
    );

    assert_eq!(result, BeginShotResult::Started);
    let direction_goal = i16::from(
        entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .element_data()
            .sprite
            .position_iface
            .get_direction_goal(),
    );
    assert_eq!(direction_goal, 7);
}

#[test]
fn shoot_initialization_samples_fx_target_gameplay_ground_y_once() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut target = make_arrow_target(50.0, 120.0);
    target
        .element_data_mut()
        .set_position(WorldPoint3D::new(50.0, 120.0, 100.0));
    let mut entities = entity_table(vec![Some(make_pc(0.0, 100.0)), Some(target)]);
    entities
        .get_mut(EntityId::Pc(crate::entity_id::PcId(0)))
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::AimingWithBow;
    let target_id = EntityId::Target(crate::entity_id::TargetId(1));
    bind_test_bow_release_rows(
        entities
            .get_mut_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap(),
        OrderType::ShootingWithBow,
    );
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

    let result = begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        Some(ShootMode::Normal),
        &mut 1u32,
    );
    assert_eq!(result, BeginShotResult::Started);
    entities
        .get_mut_at_index(0)
        .map(|(_, entity)| entity)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = true;

    run_test_bow_owner(
        sim,
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        false,
    );

    let direction_goal = i16::from(
        entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .element_data()
            .sprite
            .position_iface
            .get_direction_goal(),
    );
    assert_eq!(
        direction_goal,
        crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 20.0)
    );
    assert_ne!(
        direction_goal,
        crate::position_interface::vector_to_sector_0_to_15_iso(50.0, -80.0)
    );

    entities
        .get_mut_at_index(0)
        .map(|(_, entity)| entity)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = false;
    entities
        .get_mut(target_id)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(-100.0, -100.0, 0.0));
    run_test_bow_owner(
        sim,
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        false,
    );
    assert_eq!(
        i16::from(
            entities
                .get_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        direction_goal,
        "the live target is sampled once per shooting order"
    );
}

#[test]
fn leaning_out_shot_initializes_from_live_map_positions_and_holds_while_turning() {
    let sim = crate::sim_rng::test_context();
    let mut target = make_pc(50.0, 20.0);
    target
        .element_data_mut()
        .set_position(WorldPoint3D::new(50.0, 120.0, 100.0));
    let mut shooter = make_soldier(0.0, 0.0);
    shooter
        .element_data_mut()
        .publish_order_posture(Posture::LeaningOut);
    shooter.actor_data_mut().unwrap().action_state = ActionState::AimingWithBowDown;
    shooter.element_data_mut().set_direction_instantly(14);
    bind_test_bow_release_rows(&mut shooter, OrderType::ShootingWithBowLeaningOut);

    let shooter_id = EntityId::Soldier(crate::entity_id::SoldierId(0));
    let target_id = EntityId::Pc(crate::entity_id::PcId(1));
    let mut entities = entity_table(vec![Some(shooter), Some(target)]);
    let (mut sm, seq_id, elem_idx) = launch_test_shoot_element(shooter_id, target_id);
    assert_eq!(
        begin_test_bow_shot(
            &mut entities,
            &mut sm,
            shooter_id,
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            Some(ShootMode::Down),
            &mut 1u32,
        ),
        BeginShotResult::Started
    );
    entities
        .get_mut(shooter_id)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = true;

    run_test_bow_owner(&sim, &mut entities, &mut sm, shooter_id, false);
    assert!(!test_bow_done_pulse(
        &entities,
        entities.actors().next().unwrap().0.into()
    ));

    let shooter = entities.get(shooter_id).unwrap();
    let expected_goal = crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 20.0);
    assert_ne!(
        expected_goal,
        crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 120.0),
        "the fixture must distinguish map position from ground position"
    );
    assert_eq!(
        i16::from(shooter.position_iface().get_direction_goal()),
        expected_goal
    );
    assert_eq!(i16::from(shooter.position_iface().get_direction()), 13);
    assert_eq!(shooter.sprite().current_row, 13);
    assert_eq!(shooter.sprite().current_frame, 0);
}

#[test]
fn tick_bow_shots_fires_arrow_and_returns_to_aiming() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    entities
        .get_mut(EntityId::Pc(crate::entity_id::PcId(0)))
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::AimingWithBow;
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    bind_test_bow_release_rows(
        entities
            .get_mut_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap(),
        OrderType::ShootingWithBow,
    );
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

    begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        None,
        &mut 1u32,
    );

    // Tick through the facing freeze, then the shoot row's action-done pulse.
    let mut released = false;
    for _ in 0..24 {
        run_test_bow_owner(
            sim,
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            false,
        );
        released |= test_bow_done_pulse(&entities, EntityId::Pc(crate::entity_id::PcId(0)));
        if released {
            break;
        }
    }
    assert!(released, "expected the release pulse");
    assert_eq!(
        sm.get_element(seq_id, elem_idx).unwrap().state,
        crate::sequence::SequenceState::InProgress
    );

    // Shooter should now be in AimingWithBow (sustained aim).
    let actor = entities
        .get_at_index(0)
        .map(|(_, entity)| entity)
        .unwrap()
        .actor_data()
        .unwrap();
    assert_eq!(actor.action_state, ActionState::AimingWithBow);
    assert!(test_selected_bow(
        &entities,
        &sm,
        EntityId::Pc(crate::entity_id::PcId(0))
    ));
    assert!(test_bow_done_pulse(
        &entities,
        EntityId::Pc(crate::entity_id::PcId(0))
    ));
}

#[test]
fn compute_initial_throw_velocity_flat_shot() {
    let to_target = WorldVec3D::new(100.0, 0.0, 0.0);
    // Flat shot: flight_time = (0.003 * 100) + 1 = 1
    let vel = compute_initial_throw_velocity(to_target, 0.001, MASS_ARROW_FLAT, 1, None);
    // With flight_time == 1: velocity = 0.5 * to_target
    assert!((vel.x - 50.0).abs() < 0.01);
}

#[test]
fn compute_initial_throw_velocity_high_shot() {
    let to_target = WorldVec3D::new(100.0, 0.0, 0.0);
    let apex = 10.0; // distance / 10
    let vel = compute_initial_throw_velocity(to_target, apex, MASS_ARROW_HIGH, 0, None);
    // Should have a positive Z component (upward arc).
    assert!(vel.z > 0.0, "high shot should arc upward, got z={}", vel.z);
    // X should be positive (toward target).
    assert!(vel.x > 0.0);
}

#[test]
fn compute_trajectory_produces_arc() {
    let start = WorldPoint3D::new(0.0, 0.0, 40.0);
    let vel = compute_initial_throw_velocity(
        WorldVec3D::new(100.0, 0.0, -10.0),
        10.0,
        MASS_ARROW_HIGH,
        0,
        None,
    );
    let traj = compute_trajectory_ballistic(start, vel, MASS_ARROW_HIGH, false, None);
    assert!(!traj.is_empty(), "trajectory should have waypoints");
    // All points should have time == TIME_FLYSEGMENT.
    for pt in &traj {
        assert_eq!(pt.time, TIME_FLYSEGMENT);
    }
    // First point should be ahead of start in X.
    assert!(traj[0].position.x > start.x);
}

#[test]
fn projectile_impact_time_uses_euclidean_distance_ratio() {
    let position = WorldPoint3D::new(0.0, 0.0, 0.0);
    let new_position = WorldPoint3D::new(4.0, 0.0, 0.0);
    // Collision geometry can return a point off the intended segment.
    // Original measures the full 3D distance to that point: sqrt(8) / 4
    // rounds a four-frame segment to three frames.  The old dot
    // projection measured only 2 / 4 and incorrectly produced two.
    let impact = WorldPoint3D::new(2.0, 2.0, 0.0);
    let ratio = projectile_impact_ratio(position, new_position, impact);
    let impact_time = ((TIME_FLYSEGMENT as f32 * ratio + 0.5) as u16).max(1);
    let projected_time = ((TIME_FLYSEGMENT as f32 * 0.5 + 0.5) as u16).max(1);

    assert_eq!(impact_time, 3);
    assert_eq!(projected_time, 2);
}

#[test]
fn projectile_near_impact_still_uses_one_frame_minimum() {
    let ratio = projectile_impact_ratio(
        WorldPoint3D::new(0.0, 0.0, 0.0),
        WorldPoint3D::new(4.0, 0.0, 0.0),
        WorldPoint3D::new(0.01, 0.0, 0.0),
    );
    let impact_time = ((TIME_FLYSEGMENT as f32 * ratio + 0.5) as u16).max(1);
    assert_eq!(impact_time, 1);
}

#[test]
fn spawn_arrow_creates_flying_projectile_with_trajectory() {
    let traj = vec![
        TrajectoryPoint {
            position: WorldPoint3D::new(25.0, 0.0, 45.0),
            time: 4,
        },
        TrajectoryPoint {
            position: WorldPoint3D::new(50.0, 0.0, 40.0),
            time: 4,
        },
    ];
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory: traj,
        initial_velocity: WorldVec3D::new(0.0, 1.0, 0.0),
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Soldier(crate::entity_id::SoldierId(1)),
            WorldPoint3D::new(0.0, 0.0, 40.0),
            MapPoint::new(50.0, 0.0),
        )
    });
    match arrow {
        Entity::Projectile(p) => {
            assert!(p.projectile.flying);
            assert_eq!(p.projectile.trajectory.len(), 1);
            assert_eq!(p.projectile.launch_segment_start.map(|p| p.x), Some(0.0));
            assert_eq!(p.projectile.damage, 30);
            assert_eq!(p.object.object_type, ObjectType::Arrow);
            assert_eq!(
                p.element.direction(),
                0,
                "projectile sprite facing stays at its element-constructor default"
            );
            assert_ne!(
                p.projectile.flight_direction, 0,
                "gameplay flight direction is stored separately from sprite facing"
            );
            let position = p.element.sprite.position_iface.v48_serialized_state();
            assert_eq!(position.posture, Posture::Upright);
            assert_eq!(position.old_posture, Posture::Upright);
            assert_eq!(position.computed_position.bits(), 7);
            assert_eq!(position.computed_increment.bits(), 2);
            assert_eq!(position.increment, p.projectile.velocity_increment);
            assert_eq!(
                p.element.sprite.last_processed_order_id,
                u32::from(u16::MAX) + 1
            );
        }
        _ => panic!("expected ElementProjectile"),
    }
}

#[test]
fn spawn_arrow_stores_shooter_map_position_as_trajectory_origin() {
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory_origin: MapPoint::new(100.0, 0.0),
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(50.0, 40.0, 40.0),
            time: 2,
        }],
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Pc(crate::entity_id::PcId(1)),
            WorldPoint3D::new(100.0, 40.0, 40.0),
            MapPoint::new(50.0, 0.0),
        )
    });

    let Entity::Projectile(p) = arrow else {
        panic!("spawn_arrow should create projectile");
    };
    assert_eq!(p.projectile.start_of_trajectory_x, 100.0);
    assert_eq!(p.projectile.start_of_trajectory_y, 0.0);
}

#[test]
fn tick_arrows_follows_trajectory_and_hits() {
    // Place a soldier at (50, 0) (belt lives at Z=25, the
    // default belt elevation for an upright human).  The
    // trajectory arcs from the bow height down to belt height at
    // the soldier's XY — the per-segment 3D hit check picks the
    // soldier up on the final waypoint.
    let traj = vec![
        TrajectoryPoint {
            position: WorldPoint3D::new(20.0, 0.0, 35.0),
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D::new(40.0, 0.0, 30.0),
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D::new(50.0, 0.0, 25.0),
            time: 2,
        },
    ];
    let entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_soldier_with_camp(
            50.0,
            0.0,
            crate::element::Camp::Lacklandists,
        )),
        Some(spawn_arrow(SpawnArrowParams {
            trajectory: traj,
            ..SpawnArrowParams::test_flat(
                EntityId::Pc(crate::entity_id::PcId(0)),
                EntityId::Pc(crate::entity_id::PcId(1)),
                WorldPoint3D::new(0.0, 0.0, 40.0),
                MapPoint::new(50.0, 0.0),
            )
        })),
    ]);

    let (mut engine, assets) = projectile_engine(entities);
    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let victim = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let sim = crate::sim_rng::test_context();
    for _ in 0..20 {
        engine.tick_existing_projectile(TickCtx::new(&sim, &assets), arrow_id);
        if projectile_activation_seen(&engine, victim, Command::ReceiveArrowDamage) {
            break;
        }
    }
    let damage = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .find(|element| {
            element.owner == Some(victim) && element.command == Command::ReceiveArrowDamage
        })
        .expect("arrow registers damage on its victim");
    assert!(matches!(
        damage.data,
        SequenceElementData::Damage { damage: 30, .. }
    ));
}

#[test]
fn tick_arrows_human_hit_reports_old_position_and_victim_impact_anchor() {
    let traj = vec![
        TrajectoryPoint {
            position: WorldPoint3D::new(20.0, 0.0, 35.0),
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D::new(40.0, 0.0, 30.0),
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D::new(50.0, 0.0, 25.0),
            time: 2,
        },
    ];
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory: traj,
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Soldier(crate::entity_id::SoldierId(1)),
            WorldPoint3D::new(0.0, 0.0, 40.0),
            MapPoint::new(50.0, 0.0),
        )
    });
    let entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_soldier_with_camp(
            50.0,
            0.0,
            crate::element::Camp::Lacklandists,
        )),
        Some(arrow),
    ]);

    let (mut engine, assets) = projectile_engine(entities);
    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let victim = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let sim = crate::sim_rng::test_context();
    let mut impact_old = None;
    for _ in 0..20 {
        let old = engine
            .world
            .entities
            .get(arrow_id)
            .unwrap()
            .element_data()
            .position();
        engine.tick_existing_projectile(TickCtx::new(&sim, &assets), arrow_id);
        if projectile_activation_seen(&engine, victim, Command::ReceiveArrowDamage) {
            impact_old = Some(old);
            break;
        }
    }
    let old = impact_old.expect("arrow damages its victim");
    let arrow = engine.world.entities.get(arrow_id).unwrap();
    assert_eq!(
        arrow.element_data().position(),
        old,
        "consumed projectile rewinds to its previous position"
    );
    let impact = engine
        .world
        .entities
        .get(victim)
        .unwrap()
        .element_data()
        .position();
    assert_eq!(impact.x, 50.0);
    assert_eq!(impact.y, 0.0);
    assert!(old.x < impact.x);
    assert!(old.y.abs() < 0.01);
    assert!(old.z >= 25.0);
}

#[test]
fn tick_arrow_resolves_spawn_primed_segment_only_for_requested_arrow() {
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(50.0, 0.0, 0.0),
            time: 1,
        }],
        initial_velocity: WorldVec3D::new(1.0, 0.0, -0.25),
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Target(crate::entity_id::TargetId(1)),
            WorldPoint3D::new(0.0, 0.0, 40.0),
            MapPoint::new(50.0, 0.0),
        )
    });
    let mut other_arrow = spawn_arrow(SpawnArrowParams {
        trajectory_origin: MapPoint::new(0.0, 0.0),
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(1010.0, 0.0, 40.0),
            time: 1,
        }],
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Target(crate::entity_id::TargetId(1)),
            WorldPoint3D::new(1000.0, 0.0, 40.0),
            MapPoint::new(50.0, 0.0),
        )
    });
    let Entity::Projectile(p) = &mut other_arrow else {
        panic!("spawn_arrow should create projectile");
    };
    p.projectile.launch_segment_start = Some(WorldPoint3D::new(1000.0, 0.0, 40.0));

    let entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_arrow_target(50.0, 0.0)),
        Some(arrow),
        Some(other_arrow),
    ]);

    let (mut engine, assets) = projectile_engine(entities);
    engine.tick_new_projectile_once(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        EntityId::Projectile(crate::entity_id::ProjectileId(2)),
    );
    assert!(projectile_activation_seen(
        &engine,
        EntityId::Target(crate::entity_id::TargetId(1)),
        Command::ActivateArrow
    ));
    let Some(Entity::Projectile(p)) = engine
        .world
        .entities
        .get_at_index(3)
        .map(|(_, entity)| entity)
    else {
        panic!("other arrow should remain present");
    };
    assert!(
        p.projectile.launch_segment_start.is_some(),
        "filtered tick must not consume another projectile's primed segment"
    );
}

#[test]
fn tick_arrows_prefilters_friendly_candidate_before_selecting_victim() {
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(100.0, 0.0, 25.0),
            time: 1,
        }],
        ..SpawnArrowParams::test_flat(
            EntityId::Soldier(crate::entity_id::SoldierId(0)),
            EntityId::Soldier(crate::entity_id::SoldierId(2)),
            WorldPoint3D::new(0.0, 0.0, 25.0),
            MapPoint::new(100.0, 0.0),
        )
    });
    let mut entities = entity_table(vec![
        Some(make_soldier_with_camp(
            0.0,
            0.0,
            crate::element::Camp::Royalists,
        )),
        Some(make_soldier_with_camp(
            20.0,
            0.0,
            crate::element::Camp::Royalists,
        )),
        Some(make_soldier_with_camp(
            80.0,
            0.0,
            crate::element::Camp::Lacklandists,
        )),
        Some(arrow),
    ]);

    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(3));
    let actor_order: Vec<EntityId> = entities.actors().map(|(id, _)| id.into()).collect();
    let old_position = {
        let Entity::Projectile(arrow) = entities.get_mut(arrow_id).unwrap() else {
            unreachable!()
        };
        if let Some(old) = arrow.projectile.launch_segment_start.take() {
            old
        } else {
            let old = arrow.element.position();
            arrow.advance_projectile_hourglass();
            old
        }
    };
    let hit = projectile_human_victim(
        &entities,
        &actor_order,
        &crate::diplomacy::DiplomacyState::default(),
        arrow_id,
        old_position,
    );
    assert_eq!(
        hit,
        Some(EntityId::Soldier(crate::entity_id::SoldierId(2))),
        "friendly candidate is skipped before the valid later victim"
    );
}

#[test]
fn enabled_diplomacy_protects_neutral_soldiers_from_pc_arrows() {
    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let victim_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(100.0, 0.0, 25.0),
            time: 1,
        }],
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            victim_id,
            WorldPoint3D::new(0.0, 0.0, 25.0),
            MapPoint::new(100.0, 0.0),
        )
    });
    let entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_soldier_with_camp(
            80.0,
            0.0,
            crate::element::Camp::Lacklandists,
        )),
        Some(arrow),
    ]);
    let diplomacy = crate::diplomacy::DiplomacyState::from_definition(
        true,
        true,
        Some(&crate::diplomacy::DiplomacyDefinition {
            player_coalition: vec![0],
            relationships: vec![crate::diplomacy::DiplomacyRule {
                first: 0,
                second: 1,
                relationship: crate::diplomacy::Relationship::Neutral,
            }],
        }),
    )
    .unwrap();

    let Entity::Projectile(arrow) = entities.get(arrow_id).unwrap() else {
        unreachable!()
    };
    let old_position = arrow.projectile.launch_segment_start.unwrap();
    let hit = projectile_human_victim(
        &entities,
        &[EntityId::Pc(crate::entity_id::PcId(0)), victim_id],
        &diplomacy,
        arrow_id,
        old_position,
    );
    assert_eq!(
        hit, None,
        "neutral actors are excluded before hit selection"
    );
}

#[test]
fn tick_arrows_selects_last_eligible_human_in_actor_order() {
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(100.0, 0.0, 25.0),
            time: 1,
        }],
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Soldier(crate::entity_id::SoldierId(1)),
            WorldPoint3D::new(0.0, 0.0, 25.0),
            MapPoint::new(100.0, 0.0),
        )
    });
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        // The earlier actor is farther downrange than the later actor so
        // this proves registry order, not nearest/farthest geometry.
        Some(make_soldier(80.0, 0.0)),
        Some(make_soldier(60.0, 0.0)),
        Some(arrow),
    ]);

    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(3));
    let actor_order: Vec<EntityId> = entities.actors().map(|(id, _)| id.into()).collect();
    let old_position = {
        let Entity::Projectile(arrow) = entities.get_mut(arrow_id).unwrap() else {
            unreachable!()
        };
        if let Some(old) = arrow.projectile.launch_segment_start.take() {
            old
        } else {
            let old = arrow.element.position();
            arrow.advance_projectile_hourglass();
            old
        }
    };
    let hit = projectile_human_victim(
        &entities,
        &actor_order,
        &crate::diplomacy::DiplomacyState::default(),
        arrow_id,
        old_position,
    );
    assert_eq!(
        hit,
        Some(EntityId::Soldier(crate::entity_id::SoldierId(2))),
        "last eligible actor replaces earlier farther candidate"
    );
}

#[test]
fn ordered_projectile_scan_uses_actor_registry_not_entity_slots() {
    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(3));
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(100.0, 0.0, 25.0),
            time: 1,
        }],
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Soldier(crate::entity_id::SoldierId(1)),
            WorldPoint3D::new(0.0, 0.0, 25.0),
            MapPoint::new(100.0, 0.0),
        )
    });
    let mut world = crate::engine::state::WorldState::new();
    world.entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        // Slot order visits Soldier1 before Soldier2. Original creation
        // order for the representative is the reverse, so Soldier1 is
        // the final eligible actor and must replace Soldier2.
        Some(make_soldier(60.0, 0.0)),
        Some(make_soldier(80.0, 0.0)),
        Some(arrow),
    ]);
    let pc = EntityId::Pc(crate::entity_id::PcId(0));
    let soldier_1 = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let soldier_2 = EntityId::Soldier(crate::entity_id::SoldierId(2));
    world.actor_registry_ids = vec![pc, soldier_1, soldier_2];
    world.fighter_registry_ids = world.actor_registry_ids.clone();
    world.install_original_creation_orders(
        std::collections::BTreeMap::from([
            (pc, 100),
            (soldier_2, 101),
            (soldier_1, 102),
            (arrow_id, 103),
        ]),
        104,
    );
    let actor_order = world.actor_registry_ids.clone();
    assert_eq!(actor_order, [pc, soldier_2, soldier_1]);

    let Entity::Projectile(arrow) = world.entities.get(arrow_id).unwrap() else {
        unreachable!()
    };
    let old_position = arrow.projectile.launch_segment_start.unwrap();
    let hit = projectile_human_victim(
        &world.entities,
        &actor_order,
        &crate::diplomacy::DiplomacyState::default(),
        arrow_id,
        old_position,
    );
    assert_eq!(
        hit,
        Some(soldier_1),
        "last eligible actor follows creation order, not entity slot"
    );
}

#[test]
fn ordered_projectile_scan_uses_first_shield_in_actor_registry_order() {
    use crate::element::ActionState;

    let make_holder = || {
        let mut holder = make_soldier(50.0, 0.0);
        let actor = holder.actor_data_mut().unwrap();
        actor.action_state = ActionState::HoldingShield;
        actor.shield_obstacle = Some(Box::new(compute_shield_obstacle(
            MapPoint::new(50.0, 0.0),
            0.0,
            4,
            &shield_params_for_soldier(20, 40),
        )));
        holder.element_data_mut().set_direction_instantly(4);
        holder
    };
    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(3));
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(50.0, 0.0, 40.0),
            time: 2,
        }],
        initial_velocity: WorldVec3D::new(-1.0, 0.0, 0.0),
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Soldier(crate::entity_id::SoldierId(1)),
            WorldPoint3D::new(100.0, 0.0, 40.0),
            MapPoint::new(50.0, 0.0),
        )
    });
    let entities = entity_table(vec![
        Some(make_pc(100.0, 0.0)),
        Some(make_holder()),
        Some(make_holder()),
        Some(arrow),
    ]);
    let soldier_1 = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let soldier_2 = EntityId::Soldier(crate::entity_id::SoldierId(2));
    let actor_order = [
        EntityId::Pc(crate::entity_id::PcId(0)),
        soldier_2,
        soldier_1,
    ];

    let Entity::Projectile(arrow) = entities.get(arrow_id).unwrap() else {
        unreachable!()
    };
    let hit = projectile_shield_holder(
        &entities,
        &actor_order,
        arrow.projectile.launch_segment_start.unwrap(),
        WorldPoint3D::new(50.0, 0.0, 40.0),
        arrow.projectile.velocity_increment,
    );
    assert_eq!(hit, Some(soldier_2));
}

#[test]
fn tick_arrows_leaning_eye_hit_can_be_replaced_by_later_eligible_human() {
    let [lean_x, lean_y] = crate::position_interface::sector_to_vector_iso(0);
    let arrow_y = lean_y * 40.0;
    let arrow_old = WorldPoint3D::new(lean_x * 40.0, arrow_y, 45.0);
    let arrow_new = WorldPoint3D::new(120.0 + lean_x * 40.0, arrow_y, 45.0);
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: arrow_old,
        trajectory_origin: MapPoint::new(0.0, 0.0),
        target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
        target_pos: MapPoint::new(120.0, 0.0),
        trajectory: vec![TrajectoryPoint {
            position: arrow_new,
            time: 1,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D::new(1.0, 0.0, 0.0),
    });
    let mut earlier = make_soldier(80.0, 0.0);
    earlier
        .element_data_mut()
        .publish_order_posture(Posture::LeaningOut);
    earlier.element_data_mut().set_direction_instantly(0);
    // The flight line is at the leaning eye height (z=45); the belt is
    // at z=25, outside HIT_DISTANCE, so only the eye retry can hit.
    let mut later = make_soldier(60.0, 0.0);
    later
        .element_data_mut()
        .publish_order_posture(Posture::LeaningOut);
    later.element_data_mut().set_direction_instantly(0);
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, -200.0)),
        Some(earlier),
        Some(later),
        Some(arrow),
    ]);

    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(3));
    let actor_order: Vec<EntityId> = entities.actors().map(|(id, _)| id.into()).collect();
    let old_position = {
        let Entity::Projectile(arrow) = entities.get_mut(arrow_id).unwrap() else {
            unreachable!()
        };
        if let Some(old) = arrow.projectile.launch_segment_start.take() {
            old
        } else {
            let old = arrow.element.position();
            arrow.advance_projectile_hourglass();
            old
        }
    };
    let hit = projectile_human_victim(
        &entities,
        &actor_order,
        &crate::diplomacy::DiplomacyState::default(),
        arrow_id,
        old_position,
    );
    assert_eq!(hit, Some(EntityId::Soldier(crate::entity_id::SoldierId(2))));
}

#[test]
fn tick_arrows_stationary_projectile_does_not_hit_human() {
    let mut arrow_element = crate::engine::test_support::extra_engine_combat::test_element(
        ElementKind::ObjectProjectile,
        true,
    );
    arrow_element.set_position_map(MapPoint::new(50.0, -25.0));
    arrow_element.set_position(WorldPoint3D::new(50.0, 0.0, 25.0));
    let arrow = Entity::Projectile(ElementProjectile {
        element: arrow_element,
        object: ObjectData {
            associated_action: Action::Bow,
            object_type: ObjectType::Arrow,
            animation: Animation::ObjectFlying,
            quantity: 1,
            reference: Some(EntityId::Pc(crate::entity_id::PcId(1))),
            ..ObjectData::default()
        },
        projectile: ProjectileData {
            shooter: Some(EntityId::Pc(crate::entity_id::PcId(0))),
            flying: true,
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D::new(50.0, 0.0, 25.0),
                time: 1,
            }],
            damage: 30,
            ..ProjectileData::default()
        },
    });
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_soldier_with_camp(
            50.0,
            0.0,
            crate::element::Camp::Lacklandists,
        )),
        Some(arrow),
    ]);

    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let actor_order: Vec<EntityId> = entities.actors().map(|(id, _)| id.into()).collect();
    let old_position = {
        let Entity::Projectile(arrow) = entities.get_mut(arrow_id).unwrap() else {
            unreachable!()
        };
        if let Some(old) = arrow.projectile.launch_segment_start.take() {
            old
        } else {
            let old = arrow.element.position();
            arrow.advance_projectile_hourglass();
            old
        }
    };
    let hit = projectile_human_victim(
        &entities,
        &actor_order,
        &crate::diplomacy::DiplomacyState::default(),
        arrow_id,
        old_position,
    );
    assert_eq!(
        hit, None,
        "stationary projectiles cannot select a human victim"
    );
}

#[test]
fn tick_arrows_without_shooter_does_not_hit_human() {
    let mut arrow = spawn_arrow(SpawnArrowParams {
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(50.0, 0.0, 25.0),
            time: 1,
        }],
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Pc(crate::entity_id::PcId(1)),
            WorldPoint3D::new(0.0, 0.0, 40.0),
            MapPoint::new(50.0, 0.0),
        )
    });
    if let Entity::Projectile(proj) = &mut arrow {
        proj.projectile.shooter = None;
    }
    let mut entities = entity_table(vec![None, Some(make_soldier(50.0, 0.0)), Some(arrow)]);

    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let actor_order: Vec<EntityId> = entities.actors().map(|(id, _)| id.into()).collect();
    let old_position = {
        let Entity::Projectile(arrow) = entities.get_mut(arrow_id).unwrap() else {
            unreachable!()
        };
        if let Some(old) = arrow.projectile.launch_segment_start.take() {
            old
        } else {
            let old = arrow.element.position();
            arrow.advance_projectile_hourglass();
            old
        }
    };
    let hit = projectile_human_victim(
        &entities,
        &actor_order,
        &crate::diplomacy::DiplomacyState::default(),
        arrow_id,
        old_position,
    );
    assert_eq!(hit, None, "a missing shooter excludes all human victims");
}

/// An arrow whose shooter dies mid-flight keeps hunting victims.
///
/// Original-game victim selection holds the shooter through the
/// shooter reference stored at construction, aborts only when that reference
/// is absent, and otherwise merely asks the shooter for
/// soldier, camp, and player-character checks. A corpse still
/// answers all three, so the shot stays lethal. Rust used to resolve the
/// shooter inside the *hittable-victim* snapshot, which drops dead,
/// lying, netted and tied humans, and then skipped the whole scan.
#[test]
fn tick_arrows_dead_shooter_still_hits_human() {
    use crate::element::Posture;

    let mut shooter = make_pc(0.0, 0.0);
    shooter.set_posture(Posture::Dead);

    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(50.0, 0.0, 25.0),
            time: 2,
        }],
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Soldier(crate::entity_id::SoldierId(1)),
            WorldPoint3D::new(0.0, 0.0, 25.0),
            MapPoint::new(50.0, 0.0),
        )
    });

    let mut entities = entity_table(vec![
        Some(shooter),
        Some(make_soldier(50.0, 0.0)),
        Some(arrow),
    ]);

    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let hit = first_projectile_victim(&mut entities, arrow_id, 10);
    assert_eq!(hit, Some(EntityId::Soldier(crate::entity_id::SoldierId(1))));
}

/// Apple projectile flying through an APPLE-filtered FX target
/// yields a `Command::ActivateApple` activation on tick.
#[test]
fn tick_arrows_apple_projectile_activates_apple_target() {
    use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

    let target_pos = MapPoint::new(50.0, 0.0);
    let mut target_element =
        crate::engine::test_support::extra_engine_combat::test_element(ElementKind::Target, true);
    target_element.set_position_map(target_pos);
    // `compute_target_center` reads the 3D position; real loaded
    // targets set both, but `ElementData::default()` leaves position
    // at origin so we mirror position_map.
    target_element.set_position(WorldPoint3D::new(target_pos.x, target_pos.y, 0.0));
    let target = Entity::Target(ElementTarget {
        element: target_element,
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::APPLE,
            ..TargetData::default()
        },
    });

    let trajectory = vec![
        TrajectoryPoint {
            position: WorldPoint3D::new(25.0, 0.0, 10.0),
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D::new(50.0, 0.0, 0.0),
            time: 2,
        },
    ];
    let mut apple_element = crate::engine::test_support::extra_engine_combat::test_element(
        ElementKind::ObjectProjectile,
        true,
    );
    apple_element.set_position_map(MapPoint::new(0.0, 0.0));
    apple_element.set_position(WorldPoint3D::new(0.0, 0.0, 20.0));
    let apple = Entity::Projectile(ElementProjectile {
        element: apple_element,
        object: ObjectData {
            associated_action: Action::Apple,
            object_type: ObjectType::Apple,
            animation: Animation::ObjectFlying,
            quantity: 1,
            reference: Some(EntityId::Target(crate::entity_id::TargetId(0))),
            ..ObjectData::default()
        },
        projectile: ProjectileData {
            shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
            flying: true,
            trajectory,
            ..ProjectileData::default()
        },
    });

    let entities = entity_table(vec![Some(target), Some(apple), Some(make_pc(0.0, 0.0))]);

    let (mut engine, assets) = projectile_engine(entities);
    let projectile = EntityId::Projectile(crate::entity_id::ProjectileId(1));
    let target = EntityId::Target(crate::entity_id::TargetId(0));
    for _ in 0..20 {
        engine.tick_existing_projectile(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            projectile,
        );
        if projectile_activation_seen(&engine, target, Command::ActivateApple) {
            break;
        }
    }
    assert!(
        projectile_activation_seen(&engine, target, Command::ActivateApple),
        "apple projectile should activate APPLE-filter target with ActivateApple"
    );
    assert!(engine.feedback.pending_side_effects.sounds.iter().any(|sound| matches!(
        sound, crate::engine::SoundCommand::Fx { fx_id: 509, position, .. } if *position == target_pos
    )));
}

fn projectile_activation_seen(
    engine: &crate::engine::EngineInner,
    target: EntityId,
    command: Command,
) -> bool {
    engine
        .orders
        .sequence_manager
        .sequences_iter()
        .any(|sequence| {
            sequence
                .elements
                .iter()
                .any(|element| element.owner == Some(target) && element.command == command)
        })
}

/// The original game uses current-position range gating for
/// FX targets: a target just beyond the old->new segment endpoint
/// can still be hit when it is within one movement length of the
/// arrow's current position.  This catches short final segments
/// that would otherwise land without activating scripted targets.
#[test]
fn tick_arrows_arrow_target_uses_current_position_range_gate() {
    use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

    let mut target_element =
        crate::engine::test_support::extra_engine_combat::test_element(ElementKind::Target, true);
    target_element.set_position_map(MapPoint::new(40.0, 0.0));
    target_element.set_position(WorldPoint3D::new(40.0, 0.0, 0.0));
    let target = Entity::Target(ElementTarget {
        element: target_element,
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::ARROW,
            ..TargetData::default()
        },
    });

    let mut arrow_element = crate::engine::test_support::extra_engine_combat::test_element(
        ElementKind::ObjectProjectile,
        true,
    );
    arrow_element.set_position_map(MapPoint::new(0.0, 0.0));
    arrow_element.set_position(WorldPoint3D::new(0.0, 0.0, 0.0));
    let arrow = Entity::Projectile(ElementProjectile {
        element: arrow_element,
        object: ObjectData {
            associated_action: Action::Bow,
            object_type: ObjectType::Arrow,
            animation: Animation::ObjectFlying,
            quantity: 1,
            reference: Some(EntityId::Target(crate::entity_id::TargetId(0))),
            ..ObjectData::default()
        },
        projectile: ProjectileData {
            shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
            flying: true,
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D::new(25.0, 0.0, 0.0),
                time: 1,
            }],
            damage: 30,
            ..ProjectileData::default()
        },
    });

    let entities = entity_table(vec![Some(target), Some(arrow), Some(make_pc(0.0, 0.0))]);
    let (mut engine, assets) = projectile_engine(entities);
    engine.tick_existing_projectile(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        EntityId::Projectile(crate::entity_id::ProjectileId(1)),
    );
    assert!(
        projectile_activation_seen(
            &engine,
            EntityId::Target(crate::entity_id::TargetId(0)),
            Command::ActivateArrow
        ),
        "arrow should activate target using the original game's current-position range gate"
    );
    let Entity::Projectile(arrow) = engine.world.entities.get_at_index(1).unwrap().1 else {
        panic!("expected arrow");
    };
    assert!(!arrow.projectile.flying);
}

/// A stopped projectile cannot activate a target with its degenerate flight line.
#[test]
fn tick_arrows_stationary_projectile_does_not_radius_hit_fx_target() {
    use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

    let mut target_element =
        crate::engine::test_support::extra_engine_combat::test_element(ElementKind::Target, true);
    target_element.set_position_map(MapPoint::new(10.0, 0.0));
    target_element.set_position(WorldPoint3D::new(10.0, 0.0, 0.0));
    let target = Entity::Target(ElementTarget {
        element: target_element,
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::ARROW,
            ..TargetData::default()
        },
    });

    let mut arrow_element = crate::engine::test_support::extra_engine_combat::test_element(
        ElementKind::ObjectProjectile,
        true,
    );
    arrow_element.set_position_map(MapPoint::new(0.0, 0.0));
    arrow_element.set_position(WorldPoint3D::new(0.0, 0.0, 0.0));
    let arrow = Entity::Projectile(ElementProjectile {
        element: arrow_element,
        object: ObjectData {
            associated_action: Action::Bow,
            object_type: ObjectType::Arrow,
            animation: Animation::ObjectFlying,
            quantity: 1,
            reference: Some(EntityId::Pc(crate::entity_id::PcId(0))),
            ..ObjectData::default()
        },
        projectile: ProjectileData {
            shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
            flying: true,
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D::new(0.0, 0.0, 0.0),
                time: 1,
            }],
            damage: 30,
            ..ProjectileData::default()
        },
    });

    let entities = entity_table(vec![Some(target), Some(arrow), Some(make_pc(0.0, 0.0))]);
    let (mut engine, assets) = projectile_engine(entities);
    engine.tick_existing_projectile(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        EntityId::Projectile(crate::entity_id::ProjectileId(1)),
    );
    assert!(
        !projectile_activation_seen(
            &engine,
            EntityId::Target(crate::entity_id::TargetId(0)),
            Command::ActivateArrow
        ),
        "stationary projectile must not activate nearby FX target by radius"
    );
}

#[test]
fn projectile_target_collision_has_no_stationary_or_short_segment_epsilon() {
    for (movement, target_x, expected_hit) in [
        (0.0, 0.0, false),
        (0.0, 0.005, false),
        (0.0001, 0.0001, true),
        (0.0001, 0.0002, true),
        (0.0001, 0.0003, false),
    ] {
        let mut element = ElementData::default();
        element.kind = ElementKind::ObjectProjectile;
        element.active = true;
        element.set_position(WorldPoint3D::new(movement, 0.0, 0.0));
        let projectile = Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type: ObjectType::Arrow,
                ..ObjectData::default()
            },
            projectile: ProjectileData::default(),
        });
        let entities = entity_table(vec![
            Some(projectile),
            Some(make_arrow_target(target_x, 0.0)),
        ]);
        let hit = projectile_target_victim(
            &entities,
            EntityId::Projectile(crate::entity_id::ProjectileId(0)),
            WorldPoint3D::new(0.0, 0.0, 0.0),
        );
        assert_eq!(
            hit.is_some(),
            expected_hit,
            "movement={movement}, target={target_x}"
        );
    }
}

#[test]
fn projectile_target_collision_keeps_first_eligible_sparse_slot() {
    let mut element = ElementData::default();
    element.kind = ElementKind::ObjectProjectile;
    element.active = true;
    element.set_position(WorldPoint3D::new(10.0, 0.0, 0.0));
    let projectile = Entity::Projectile(ElementProjectile {
        element,
        object: ObjectData {
            object_type: ObjectType::Arrow,
            ..ObjectData::default()
        },
        projectile: ProjectileData::default(),
    });
    let mut inactive = make_arrow_target(10.0, 0.0);
    inactive.element_data_mut().active = false;
    let mut wrong_filter = make_arrow_target(10.0, 0.0);
    let Entity::Target(target) = &mut wrong_filter else {
        unreachable!()
    };
    target.target.action_filter = TargetFilter::APPLE;
    let entities = entity_table(vec![
        Some(projectile),
        Some(inactive),
        None,
        Some(make_pc(10.0, 0.0)),
        Some(wrong_filter),
        Some(make_arrow_target(15.0, 0.0)),
        Some(make_arrow_target(10.0, 0.0)),
    ]);
    assert_eq!(
        projectile_target_victim(
            &entities,
            EntityId::Projectile(crate::entity_id::ProjectileId(0)),
            WorldPoint3D::new(0.0, 0.0, 0.0),
        ),
        Some((
            EntityId::Target(crate::entity_id::TargetId(5)),
            Command::ActivateArrow
        )),
    );
}

#[test]
fn tick_arrows_has_no_artificial_lifetime_timeout() {
    let trajectory = (1..=320)
        .map(|i| TrajectoryPoint {
            position: WorldPoint3D::new(i as f32 * 10.0, 0.0, 40.0),
            time: 1,
        })
        .collect();
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory,
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Soldier(crate::entity_id::SoldierId(1)),
            WorldPoint3D::new(0.0, 0.0, 40.0),
            MapPoint::new(3200.0, 0.0),
        )
    });
    let entities = entity_table(vec![Some(make_pc(0.0, -100.0)), Some(arrow)]);

    let (mut engine, assets) = projectile_engine(entities);
    let projectile = engine.world.entities.get_at_index(1).unwrap().0;
    let mut despawn_frame = None;
    for frame in 0..260 {
        engine.tick_existing_projectile(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            projectile,
        );
        if !engine.world.entities.get(projectile).unwrap().is_active() {
            despawn_frame = Some(frame);
            break;
        }
    }

    assert_eq!(
        despawn_frame, None,
        "projectile lifetime is trajectory-driven, not capped at 250 frames"
    );
    match engine.world.entities.get(projectile).unwrap() {
        Entity::Projectile(p) => assert!(p.projectile.flying),
        _ => panic!("expected projectile"),
    }
}

/// Apple projectile flying through a target that does NOT have the
/// APPLE filter leaves `fx_target_hit` unset — no activation is
/// launched.
#[test]
fn tick_arrows_apple_projectile_ignores_non_apple_target() {
    use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

    let mut target_element =
        crate::engine::test_support::extra_engine_combat::test_element(ElementKind::Target, true);
    target_element.set_position_map(MapPoint::new(50.0, 0.0));
    target_element.set_position(WorldPoint3D::new(50.0, 0.0, 0.0));
    let target = Entity::Target(ElementTarget {
        element: target_element,
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::ARROW,
            ..TargetData::default()
        },
    });

    let trajectory = vec![TrajectoryPoint {
        position: WorldPoint3D::new(50.0, 0.0, 0.0),
        time: 2,
    }];
    let apple = Entity::Projectile(ElementProjectile {
        element: crate::engine::test_support::extra_engine_combat::test_element(
            ElementKind::ObjectProjectile,
            true,
        ),
        object: ObjectData {
            associated_action: Action::Apple,
            object_type: ObjectType::Apple,
            animation: Animation::ObjectFlying,
            ..ObjectData::default()
        },
        projectile: ProjectileData {
            shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
            flying: true,
            trajectory,
            ..ProjectileData::default()
        },
    });

    let entities = entity_table(vec![Some(target), Some(apple), Some(make_pc(100.0, 100.0))]);
    let (mut engine, assets) = projectile_engine(entities);
    let projectile = engine.world.entities.get_at_index(1).unwrap().0;
    engine.tick_existing_projectile(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        projectile,
    );
    assert!(
        !projectile_activation_seen(
            &engine,
            EntityId::Target(crate::entity_id::TargetId(0)),
            Command::ActivateApple
        ),
        "the original game ignores nonmatching target filters before the target can burst"
    );
}

/// Apple impact on an FX target sets the burst animation + decay
/// row and leaves grounded animation/removal to the derived owner path.
#[test]
fn tick_arrows_apple_bursts_then_leaves_grounded_tail_to_virtual_owner() {
    use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

    let mut target_element =
        crate::engine::test_support::extra_engine_combat::test_element(ElementKind::Target, true);
    target_element.set_position_map(MapPoint::new(10.0, 0.0));
    let target = Entity::Target(ElementTarget {
        element: target_element,
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::APPLE,
            ..TargetData::default()
        },
    });
    let apple = Entity::Projectile(ElementProjectile {
        element: crate::engine::test_support::extra_engine_combat::test_element(
            ElementKind::ObjectProjectile,
            true,
        ),
        object: ObjectData {
            object_type: ObjectType::Apple,
            animation: Animation::ObjectFlying,
            ..ObjectData::default()
        },
        projectile: ProjectileData {
            shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
            flying: true,
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D::new(10.0, 0.0, 0.0),
                time: 1,
            }],
            ..ProjectileData::default()
        },
    });
    let entities = entity_table(vec![Some(target), Some(apple), Some(make_pc(0.0, 0.0))]);

    // First tick: apple reaches target, bursts.
    let (mut engine, assets) = projectile_engine(entities);
    let projectile = engine.world.entities.get_at_index(1).unwrap().0;
    engine.tick_existing_projectile(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        projectile,
    );
    assert!(
        projectile_activation_seen(
            &engine,
            EntityId::Target(crate::entity_id::TargetId(0)),
            Command::ActivateApple
        ) && engine.world.entities.get(projectile).unwrap().is_active(),
        "apple must NOT despawn on impact frame — it bursts first"
    );
    let proj_after = engine.world.entities.get(projectile).unwrap();
    match proj_after {
        Entity::Projectile(p) => {
            assert!(!p.projectile.flying);
            assert_eq!(p.object.animation, Animation::ObjectBursting);
        }
        _ => panic!("expected apple projectile"),
    }

    let impact_sound_count = engine.feedback.pending_side_effects.sounds.len();
    engine.tick_existing_projectile(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        projectile,
    );
    assert_eq!(
        engine.feedback.pending_side_effects.sounds.len(),
        impact_sound_count,
        "grounded updates must not repeat impact feedback"
    );
}

/// Apple impact yields impact FX 509; stone yields 508; arrow hit
/// without shield yields no impact FX (silent).
#[test]
fn tick_arrows_impact_fx_per_projectile_type() {
    fn spawn_projectile_at_impact(obj: ObjectType) -> Entity {
        let mut element = crate::engine::test_support::extra_engine_combat::test_element(
            ElementKind::ObjectProjectile,
            true,
        );
        element.set_position_map(MapPoint::new(0.0, 0.0));
        element.set_position(WorldPoint3D::new(0.0, 0.0, 0.0));
        Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type: obj,
                animation: Animation::ObjectFlying,
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId::Pc(crate::entity_id::PcId(1))),
                flying: true,
                // Empty trajectory → immediate "trajectory exhausted".
                trajectory: Vec::new(),
                ..ProjectileData::default()
            },
        })
    }

    let fx_for = |obj: ObjectType| -> Option<u32> {
        let entities = entity_table(vec![
            Some(spawn_projectile_at_impact(obj)),
            Some(make_pc(100.0, 0.0)),
        ]);
        let (mut engine, assets) = projectile_engine(entities);
        let projectile = engine.world.entities.get_at_index(0).unwrap().0;
        engine.tick_existing_projectile(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            projectile,
        );
        engine
            .feedback
            .pending_side_effects
            .sounds
            .iter()
            .find_map(|sound| match sound {
                crate::engine::SoundCommand::Fx { fx_id, .. } => Some(*fx_id),
                _ => None,
            })
    };
    assert_eq!(fx_for(ObjectType::Apple), Some(509));
    assert_eq!(fx_for(ObjectType::Stone), Some(508));
    assert_eq!(fx_for(ObjectType::Arrow), None);
}

/// `spawn_apple` builds a flying apple projectile with Apple
/// object_type and a ballistic trajectory.
#[test]
fn spawn_apple_creates_flying_apple_projectile() {
    let start = WorldPoint3D::new(0.0, 0.0, 40.0);
    let end = WorldPoint3D::new(100.0, 0.0, 20.0);
    let apple = spawn_apple(
        EntityId::Pc(crate::entity_id::PcId(0)),
        start,
        end,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        None,
        0,
        None,
    );
    match apple {
        Entity::Projectile(p) => {
            assert!(p.projectile.flying);
            assert_eq!(p.object.object_type, ObjectType::Apple);
            assert_eq!(p.object.associated_action, Action::Apple);
            assert_eq!(p.object.animation, Animation::ObjectFlying);
            assert_eq!(
                p.projectile.shooter,
                Some(EntityId::Pc(crate::entity_id::PcId(0)))
            );
            assert_eq!(
                p.object.reference,
                Some(EntityId::Pc(crate::entity_id::PcId(1)))
            );
            assert!(!p.projectile.trajectory.is_empty());
        }
        _ => panic!("expected apple projectile"),
    }
}

#[test]
fn apply_arrow_hit_wounds_soldier() {
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let died = apply_arrow_hit(
        &mut entities,
        EntityId::Soldier(crate::entity_id::SoldierId(1)),
        EntityId::Pc(crate::entity_id::PcId(0)),
        30,
        0,
    );
    assert!(!died, "30 damage shouldn't kill a 100hp soldier");

    let life = match entities.get_at_index(1).map(|(_, entity)| entity).unwrap() {
        Entity::Soldier(s) => s.npc.life_points,
        _ => unreachable!(),
    };
    assert_eq!(life, 70);
}

#[test]
fn apply_arrow_hit_kills_soldier_at_low_hp() {
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    if let Some((_, Entity::Soldier(s))) = entities.get_mut_at_index(1) {
        s.npc.life_points = 5;
    }
    let died = apply_arrow_hit(
        &mut entities,
        EntityId::Soldier(crate::entity_id::SoldierId(1)),
        EntityId::Pc(crate::entity_id::PcId(0)),
        30,
        0,
    );
    assert!(died);
    let life = match entities.get_at_index(1).map(|(_, entity)| entity).unwrap() {
        Entity::Soldier(s) => s.npc.life_points,
        _ => unreachable!(),
    };
    assert_eq!(life, 0);
}

#[test]
fn build_shoot_bow_element_produces_interaction_element() {
    let elem = build_shoot_bow_element(
        EntityId::Pc(crate::entity_id::PcId(0)),
        EntityId::Pc(crate::entity_id::PcId(1)),
    );
    assert_eq!(elem.command, Command::ShootBow);
    match &elem.data {
        SequenceElementData::Interaction { antagonist } => {
            assert_eq!(*antagonist, Some(EntityId::Pc(crate::entity_id::PcId(1))));
        }
        other => panic!("expected Interaction, got {:?}", other),
    }
}

#[test]
fn hit_chance_bias_scales_with_skill() {
    // The focused fixture supplies an explicit deterministic context.
    crate::sim_rng::with_seed(1, |sim| {
        if let Some(bias) = roll_hit_and_compute_bias(sim, 0, 90) {
            // Miss with 90 skill → very small bias.
            assert!(bias.x.abs() < 1.0);
            assert!(bias.y.abs() < 1.0);
            assert!(bias.z.abs() < 1.0);
        }
    });
}

#[test]
fn bow_miss_skill_factor_uses_unclamped_capacity() {
    assert_eq!(bow_miss_skill_factor(0), 1.0);
    assert_eq!(bow_miss_skill_factor(100), 0.0);
    assert_eq!(bow_miss_skill_factor(150), -0.5);
}

#[test]
fn shoot_mode_from_action_state_mapping() {
    assert!(matches!(
        shoot_mode_from_action_state(ActionState::AimingWithBow),
        ShootMode::Normal
    ));
    assert!(matches!(
        shoot_mode_from_action_state(ActionState::AimingWithBowUp),
        ShootMode::Long
    ));
    assert!(matches!(
        shoot_mode_from_action_state(ActionState::AimingWithBowDown),
        ShootMode::Down
    ));
}

#[test]
fn bow_point_order_types_are_non_anonymous_gameplay_ids() {
    assert_eq!(
        bow_point_order_type_for_mode(ShootMode::Normal),
        OrderType::ShootingWithBow
    );
    assert_eq!(
        bow_point_order_type_for_mode(ShootMode::Long),
        OrderType::ShootingWithBowUp
    );
    assert_eq!(
        bow_point_order_type_for_mode(ShootMode::Down),
        OrderType::ShootingWithBowLeaningOut
    );
}

#[test]
fn aim_transitions_from_up_to_normal() {
    let t = aim_transition_orders(ActionState::AimingWithBowUp, ShootMode::Normal, false);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0], OrderType::TransitionLoweringBow);
}

#[test]
fn aim_transitions_from_down_to_long() {
    let t = aim_transition_orders(ActionState::AimingWithBowDown, ShootMode::Long, false);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0], OrderType::TransitionRaisingBowLeaningOut);
    assert_eq!(t[1], OrderType::TransitionRaisingBow);
}

#[test]
fn aim_transitions_use_anonymous_raise_lower_orders() {
    let normal = aim_transition_orders(ActionState::AimingWithBowUp, ShootMode::Normal, true);
    assert_eq!(normal, vec![OrderType::TransitionLoweringBowAnonymous]);

    let long = aim_transition_orders(ActionState::AimingWithBow, ShootMode::Long, true);
    assert_eq!(long, vec![OrderType::TransitionRaisingBowAnonymous]);
}

#[rstest::rstest]
#[case::unequip_sets_waiting(
    ActionState::AimingWithBow,
    OrderType::TransitionUnequipBow,
    ActionState::Waiting
)]
#[case::equip_sets_aiming(
    ActionState::Waiting,
    OrderType::TransitionEquipBow,
    ActionState::AimingWithBow
)]
#[case::unload_sets_waiting(
    ActionState::AimingWithBowDown,
    OrderType::TransitionUnloadBow,
    ActionState::Waiting
)]
fn bow_transition_sets_action_state_on_animation_start(
    #[case] initial: ActionState,
    #[case] transition: OrderType,
    #[case] expected: ActionState,
) {
    let mut pc = make_pc(0.0, 0.0);
    pc.actor_data_mut().unwrap().action_state = initial;

    apply_bow_transition_state_side_effect(&mut pc, transition, SpriteMotionState::Start);

    assert_eq!(
        pc.actor_data().unwrap().action_state,
        expected,
        "the bow transition sets its action state when motion starts"
    );
}

fn tick_active_pc_equip_start(script_driven: bool) -> Action {
    let sim = crate::sim_rng::test_context();
    let shooter = EntityId::Pc(crate::entity_id::PcId(0));
    let target = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let mut pc = make_pc(0.0, 0.0);
    bind_test_bow_release_rows(&mut pc, OrderType::TransitionEquipBow);
    let mut entities = entity_table(vec![Some(pc), Some(make_soldier(50.0, 0.0))]);
    let mut sm = SequenceManager::new();
    let mut element = build_shoot_bow_element(shooter, target);
    element.script_driven = script_driven;
    element.data = SequenceElementData::Interaction { antagonist: None };
    let mut next_order_id = 1;
    let order_id = crate::order::alloc_order_id(&mut next_order_id);
    element.orders.push_back(Order::new(
        OrderType::TransitionEquipBow,
        0.0,
        0.0,
        order_id,
    ));
    let sequence_id = sm.insert_element(element);
    sm.start_sequence_level(sequence_id);
    sm.get_element_mut(sequence_id, 0).unwrap().state = crate::sequence::SequenceState::InProgress;
    sm.rebuild_indices();
    entities
        .get_mut(shooter)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .selected_sequence_element = Some(crate::sequence::SequenceElementRef::new(sequence_id, 0));

    run_test_bow_owner(&sim, &mut entities, &mut sm, shooter, false);
    assert_eq!(
        entities
            .get(shooter)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        ActionState::AimingWithBow,
        "the specialized owner must still apply the equip START state before its callback"
    );
    entities
        .get(shooter)
        .unwrap()
        .pc_data()
        .unwrap()
        .current_action
}

#[test]
fn selected_bow_pc_equip_start_requests_bow_action_restitution() {
    assert_eq!(tick_active_pc_equip_start(false), Action::Bow);
}

#[test]
fn selected_bow_script_pc_equip_start_suppresses_bow_action_restitution() {
    assert_eq!(tick_active_pc_equip_start(true), Action::NoAction);
}

#[test]
fn equip_and_unload_are_active_bow_transition_orders() {
    assert!(is_bow_transition_order(OrderType::TransitionEquipBow));
    assert!(is_bow_transition_order(
        OrderType::TransitionEquipBowAnonymous
    ));
    assert!(is_bow_transition_order(OrderType::TransitionUnloadBow));
    assert!(is_bow_transition_order(
        OrderType::TransitionUnloadBowAnonymous
    ));
}

#[test]
fn leaning_out_bow_transitions_update_posture_like_soldier_execute() {
    let mut soldier = make_soldier(0.0, 0.0);
    soldier.actor_data_mut().unwrap().action_state = ActionState::AimingWithBow;

    apply_bow_transition_state_side_effect(
        &mut soldier,
        OrderType::TransitionLoweringBowLeaningOut,
        SpriteMotionState::Done,
    );
    assert_eq!(soldier.element_data().posture(), Posture::LeaningOut);
    assert_eq!(
        soldier.actor_data().unwrap().action_state,
        ActionState::AimingWithBowDown
    );

    apply_bow_transition_state_side_effect(
        &mut soldier,
        OrderType::TransitionRaisingBowLeaningOut,
        SpriteMotionState::Done,
    );
    assert_eq!(soldier.element_data().posture(), Posture::Upright);
    assert_eq!(
        soldier.actor_data().unwrap().action_state,
        ActionState::AimingWithBow
    );
}

#[test]
fn down_bow_shot_release_keeps_leaning_out_posture() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut pc = make_pc(0.0, 0.0);
    pc.element_data_mut()
        .publish_order_posture(Posture::LeaningOut);
    bind_test_bow_release_rows(&mut pc, OrderType::ShootingWithBowLeaningOut);
    pc.actor_data_mut().unwrap().action_state = ActionState::AimingWithBowDown;
    let mut target = make_soldier(50.0, 0.0);
    target
        .element_data_mut()
        .publish_order_posture(Posture::LeaningOut);
    let mut entities = entity_table(vec![Some(pc), Some(target)]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

    begin_test_bow_shot(
        &mut entities,
        &mut sm,
        EntityId::Pc(crate::entity_id::PcId(0)),
        target_id,
        seq_id,
        elem_idx,
        false,
        10,
        Some(ShootMode::Down),
        &mut 1u32,
    );

    let mut released = false;
    for _ in 0..16 {
        run_test_bow_owner(
            sim,
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            false,
        );
        released |= test_bow_done_pulse(&entities, EntityId::Pc(crate::entity_id::PcId(0)));
        if released {
            break;
        }
    }
    assert!(released);
    assert_eq!(
        entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .element_data()
            .posture(),
        Posture::LeaningOut
    );
    assert_eq!(
        entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        ActionState::AimingWithBow
    );
}

#[test]
fn compute_bow_point_offsets() {
    // 3D position: x=10, y=20 (map_y + elevation), z=0 (ground level)
    let pos = WorldPoint3D::new(10.0, 20.0, 0.0);
    let hand = MapPoint::new(pos.x, pos.y);
    let pt = compute_bow_point(pos, ShootMode::Normal, 0, hand);
    assert_eq!(pt.z, BOW_Z_OFFSET_NORMAL);
    assert_eq!(pt.x, 10.0); // no lateral shift for normal

    let pt_long = compute_bow_point(pos, ShootMode::Long, 0, hand);
    assert_eq!(pt_long.z, BOW_Z_OFFSET_LONG);

    // Down shot should shift laterally by 20 units in direction.
    let pt_down = compute_bow_point(pos, ShootMode::Down, 4, hand);
    assert_eq!(pt_down.z, BOW_Z_OFFSET_NORMAL);
    // Sector 4 = east (+x), so x shifts by ~20
    assert!(pt_down.x > pos.x + 15.0, "down-shot should shift x");

    let diagonal = compute_bow_point(pos, ShootMode::Down, 10, hand);
    let [iso_x, iso_y] = crate::position_interface::sector_to_vector_iso(10);
    let (_, unscaled_y) = crate::element::direction_vector_16(10);
    assert_ne!(iso_y, unscaled_y);
    assert_eq!(diagonal.x, hand.x + iso_x * 20.0);
    assert_eq!(diagonal.y, hand.y + iso_y * 20.0);

    // With non-zero elevation, Z should be elevation + offset,
    // and Y should have elevation added (isometric projection
    // adds elevation into the hand Y).
    let elevated_pos = WorldPoint3D::new(10.0, 50.0, 30.0);
    let elevated_hand = MapPoint::new(elevated_pos.x, elevated_pos.y);
    let pt_elev = compute_bow_point(elevated_pos, ShootMode::Normal, 0, elevated_hand);
    assert_eq!(pt_elev.z, 30.0 + BOW_Z_OFFSET_NORMAL);
    assert_eq!(pt_elev.y, 50.0 + 30.0); // map_y + elevation
}

// ═══════════════════════════════════════════════════════════════
//  Projectile pipeline parity tests
//
//  Verification of the projectile-tick branches: hit-an-actor,
//  hit-a-shield (deflect + fall), miss-and-fall, and the wasp-nest
//  throw impact path.
// ═══════════════════════════════════════════════════════════════

fn trajectory_into_material_test_wall(
    material_sectors: Vec<crate::material_sectors::MaterialSector>,
    water_zones: &crate::water_zones::WaterZones,
) -> (
    Vec<TrajectoryPoint>,
    Option<crate::position_interface::ObstacleHandle>,
    bool,
    bool,
) {
    let mut obstacle = compute_shield_obstacle(
        MapPoint::new(0.0, 0.0),
        0.0,
        4,
        &ShieldParams {
            pre_offset: 0.0,
            width: 100.0,
            depth: 5.0,
            height: 100.0,
            z_offset: 0.0,
        },
    );
    obstacle.set_flag(crate::sight_obstacle::SIGHTOBSTACLE_SHIELD, false);
    obstacle.material = 2; // STONE
    obstacle.material_sectors = material_sectors;
    let obstacles = [obstacle];
    let mut grid = crate::fast_find_grid::FastFindGrid::default();
    grid.size_map(4, 4);
    grid.allocate_layers(1);
    {
        let mut level = (*grid.level).clone();
        level.map_bbox =
            crate::coordinates::MapBBox::from_coords(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
        grid.level = std::sync::Arc::new(level);
    }
    // The 3D raycast reads its candidates out of the grid, exactly as
    // original-game impact reachability does, so the wall has to be
    // registered the way level loading registers real obstacles.
    grid.add_obstacle_index(
        crate::sight_obstacle::SightObstacleIndex::new(0).expect("obstacle index 0"),
        obstacles[0].projection_area_ref().map(|area| area.layer),
        &obstacles[0].box_ground,
    );
    let check = TrajectoryObstacleCheck {
        fast_find_grid: &grid,
        sight_obstacles: crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles),
        water_zones: Some(water_zones),
    };
    let (trajectory, obstacle, impact, hole, _) = compute_trajectory_ballistic_with_terminal_impact(
        // Begin far enough behind the thin wall to retain at least one
        // free-flight waypoint before impact. Original's
        // Fall-into-hole trajectory creation deliberately needs two points before
        // it can derive the approach line and append a far-edge point.
        WorldPoint3D::new(-40.0, 0.0, 25.0),
        WorldVec3D::new(10.0, 0.0, 0.0),
        MASS_ARROW_FLAT,
        false,
        Some(&check),
    );
    (trajectory, obstacle, impact, hole)
}

fn test_water_zone(points: Vec<MapPoint>) -> crate::water_zones::WaterZone {
    let mut bounding_box = crate::coordinates::MapBBox::new();
    for &point in &points {
        bounding_box.expand_point(point);
    }
    crate::water_zones::WaterZone {
        points,
        bounding_box,
        material: crate::sound_cache::Material::Hole,
    }
}

fn test_material_sector(
    points: Vec<MapPoint>,
    material: crate::element::GameMaterial,
) -> crate::material_sectors::MaterialSector {
    let mut bounding_box = crate::coordinates::MapBBox::new();
    for &point in &points {
        bounding_box.expand_point(point);
    }
    crate::material_sectors::MaterialSector {
        points,
        bounding_box,
        material,
    }
}

#[test]
fn raised_dry_terminal_obstacle_ignores_projected_global_hole() {
    let water_zones = crate::water_zones::WaterZones {
        zones: vec![test_water_zone(vec![
            MapPoint::new(-1000.0, -1000.0),
            MapPoint::new(1000.0, -1000.0),
            MapPoint::new(1000.0, 1000.0),
            MapPoint::new(-1000.0, 1000.0),
        ])],
    };

    let (_, terminal_obstacle, terminal_impact, terminal_lands_in_hole) =
        trajectory_into_material_test_wall(vec![], &water_zones);

    assert_eq!(terminal_obstacle.map(|index| index.get()), Some(0));
    assert!(terminal_impact);
    assert!(
        !terminal_lands_in_hole,
        "the global ground hole must not leak through an exact dry obstacle impact"
    );
}

#[test]
fn terminal_obstacle_hole_extends_through_exact_local_polygon() {
    let water_zones = crate::water_zones::WaterZones {
        zones: vec![test_water_zone(vec![
            MapPoint::new(-1000.0, -1000.0),
            MapPoint::new(1000.0, -1000.0),
            MapPoint::new(1000.0, 1000.0),
            MapPoint::new(-1000.0, 1000.0),
        ])],
    };
    let local_hole = test_material_sector(
        vec![
            MapPoint::new(-10_000.0, -50.0),
            MapPoint::new(10_000.0, -50.0),
            MapPoint::new(10_000.0, 50.0),
            MapPoint::new(-10_000.0, 50.0),
        ],
        crate::element::GameMaterial::Hole,
    );

    let (trajectory, terminal_obstacle, terminal_impact, terminal_lands_in_hole) =
        trajectory_into_material_test_wall(vec![local_hole.clone()], &water_zones);

    assert_eq!(terminal_obstacle.map(|index| index.get()), Some(0));
    assert!(terminal_impact);
    assert!(terminal_lands_in_hole);
    assert!(
        trajectory.len() >= 3,
        "fixture must retain free flight, terminal impact, and the appended hole exit"
    );
    let impact = trajectory[trajectory.len() - 2].position.to_map();
    assert!(local_hole.contains(impact));
    assert!(
        water_zones.landing_is_in_hole(impact),
        "fixture must genuinely overlap local and global hole polygons"
    );
    let exit = trajectory.last().unwrap().position.to_map();
    assert!(
        (exit.y - 50.0).abs() < 0.01,
        "far-edge extension must use the local obstacle hole (y=50), got {exit:?}"
    );
}

#[test]
fn arrow_trajectory_retains_exact_terminal_obstacle_identity() {
    let mut obstacle = compute_shield_obstacle(
        MapPoint::new(0.0, 0.0),
        0.0,
        4,
        &ShieldParams {
            pre_offset: 0.0,
            width: 100.0,
            depth: 5.0,
            height: 100.0,
            z_offset: 0.0,
        },
    );
    // The trajectory raycast skips shield obstacles entirely (shield
    // blocking is the per-arrow shield-holder test, not the obstacle
    // grid), so make this wall a plain solid to stay visible to it.
    obstacle.set_flag(crate::sight_obstacle::SIGHTOBSTACLE_SHIELD, false);
    let obstacles = [obstacle];
    // The trajectory raycast forces a bare ground impact for any origin
    // outside the level's map bbox, and a default grid has an empty
    // (hyperspace) bbox — give the flight path an open field instead.
    let mut grid = crate::fast_find_grid::FastFindGrid::default();
    grid.size_map(4, 4);
    grid.allocate_layers(1);
    {
        let mut level = (*grid.level).clone();
        level.map_bbox =
            crate::coordinates::MapBBox::from_coords(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
        grid.level = std::sync::Arc::new(level);
    }
    // The raycast pulls its candidates from the grid, so the wall has to
    // be registered there the way level loading registers real obstacles.
    grid.add_obstacle_index(
        crate::sight_obstacle::SightObstacleIndex::new(0).expect("obstacle index 0"),
        obstacles[0].projection_area_ref().map(|area| area.layer),
        &obstacles[0].box_ground,
    );
    let check = TrajectoryObstacleCheck {
        fast_find_grid: &grid,
        sight_obstacles: crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles),
        water_zones: None,
    };

    let (trajectory, terminal_obstacle) = compute_trajectory_ballistic_with_terminal_obstacle(
        WorldPoint3D::new(0.0, 0.0, 25.0),
        WorldVec3D::new(10.0, 0.0, 0.0),
        MASS_ARROW_FLAT,
        false,
        Some(&check),
    );

    assert!(!trajectory.is_empty());
    assert_eq!(terminal_obstacle.map(|index| index.get()), Some(0));
}

#[test]
fn arrow_trajectory_reports_exact_ground_impact_without_an_obstacle() {
    let mut grid = crate::fast_find_grid::FastFindGrid::default();
    {
        let mut level = (*grid.level).clone();
        level.map_bbox =
            crate::coordinates::MapBBox::from_coords(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
        grid.level = std::sync::Arc::new(level);
    }
    let check = TrajectoryObstacleCheck {
        fast_find_grid: &grid,
        sight_obstacles: crate::sight_obstacle::ObstacleList::empty(),
        water_zones: None,
    };

    let (
        trajectory,
        terminal_obstacle,
        terminal_impact,
        terminal_lands_in_hole,
        terminal_lands_in_water,
    ) = compute_trajectory_ballistic_with_terminal_impact(
        WorldPoint3D::new(0.0, 0.0, 25.0),
        WorldVec3D::new(10.0, 0.0, 0.0),
        MASS_ARROW_HIGH,
        false,
        Some(&check),
    );

    assert!(terminal_impact);
    assert_eq!(terminal_obstacle, None);
    assert!(!terminal_lands_in_hole);
    assert!(!terminal_lands_in_water);
    assert_eq!(trajectory.last().unwrap().position.z, 0.0);
}

#[test]
fn bare_ground_hole_is_propagated_from_terminal_trajectory_impact() {
    let mut grid = crate::fast_find_grid::FastFindGrid::default();
    {
        let mut level = (*grid.level).clone();
        level.map_bbox =
            crate::coordinates::MapBBox::from_coords(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
        grid.level = std::sync::Arc::new(level);
    }
    let water_zones = crate::water_zones::WaterZones {
        zones: vec![crate::water_zones::WaterZone {
            points: vec![
                MapPoint::new(-1000.0, -1000.0),
                MapPoint::new(1000.0, -1000.0),
                MapPoint::new(1000.0, 1000.0),
                MapPoint::new(-1000.0, 1000.0),
            ],
            bounding_box: crate::coordinates::MapBBox::from_coords(
                -1000.0, -1000.0, 1000.0, 1000.0,
            ),
            material: crate::sound_cache::Material::Hole,
        }],
    };
    let check = TrajectoryObstacleCheck {
        fast_find_grid: &grid,
        sight_obstacles: crate::sight_obstacle::ObstacleList::empty(),
        water_zones: Some(&water_zones),
    };

    let (_, terminal_obstacle, terminal_impact, terminal_lands_in_hole, terminal_lands_in_water) =
        compute_trajectory_ballistic_with_terminal_impact(
            WorldPoint3D::new(0.0, 0.0, 25.0),
            WorldVec3D::new(10.0, 0.0, 0.0),
            MASS_ARROW_HIGH,
            false,
            Some(&check),
        );

    assert!(terminal_impact);
    assert_eq!(terminal_obstacle, None);
    assert!(terminal_lands_in_hole);
    assert!(!terminal_lands_in_water);

    let (_, bounce_obstacle, bounce_impact, bounce_lands_in_hole, _) =
        compute_trajectory_ballistic_bounce_with_terminal(
            WorldPoint3D::new(0.0, 0.0, 25.0),
            WorldVec3D::new(10.0, 0.0, 0.0),
            MASS_COIN,
            false,
            Some(&check),
            BOUNCE_COIN,
        );
    assert!(bounce_impact);
    assert_eq!(bounce_obstacle, None);
    assert!(
        bounce_lands_in_hole,
        "bounce integration must use the same scoped terminal material resolver"
    );
}

#[test]
fn bare_ground_water_is_retained_for_arrow_terminal_lifecycle() {
    let mut grid = crate::fast_find_grid::FastFindGrid::default();
    {
        let mut level = (*grid.level).clone();
        level.map_bbox =
            crate::coordinates::MapBBox::from_coords(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
        grid.level = std::sync::Arc::new(level);
    }
    let water_zones = crate::water_zones::WaterZones {
        zones: vec![crate::water_zones::WaterZone {
            points: vec![
                MapPoint::new(-1000.0, -1000.0),
                MapPoint::new(1000.0, -1000.0),
                MapPoint::new(1000.0, 1000.0),
                MapPoint::new(-1000.0, 1000.0),
            ],
            bounding_box: crate::coordinates::MapBBox::from_coords(
                -1000.0, -1000.0, 1000.0, 1000.0,
            ),
            material: crate::sound_cache::Material::Water,
        }],
    };
    let check = TrajectoryObstacleCheck {
        fast_find_grid: &grid,
        sight_obstacles: crate::sight_obstacle::ObstacleList::empty(),
        water_zones: Some(&water_zones),
    };

    let (_, terminal_obstacle, terminal_impact, terminal_hole, terminal_water, _) =
        compute_trajectory_ballistic_impl(
            WorldPoint3D::new(0.0, 0.0, 25.0),
            WorldVec3D::new(10.0, 0.0, 0.0),
            MASS_ARROW_HIGH,
            false,
            Some(&check),
            None,
        );

    assert!(terminal_impact);
    assert_eq!(terminal_obstacle, None);
    assert!(!terminal_hole);
    assert!(terminal_water);
}

#[test]
fn falling_arrow_trajectory_transfers_terminal_water_to_dive_state() {
    let mut grid = crate::fast_find_grid::FastFindGrid::default();
    {
        let mut level = (*grid.level).clone();
        level.map_bbox =
            crate::coordinates::MapBBox::from_coords(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
        grid.level = std::sync::Arc::new(level);
    }
    let water_zones = crate::water_zones::WaterZones {
        zones: vec![crate::water_zones::WaterZone {
            points: vec![
                MapPoint::new(-1000.0, -1000.0),
                MapPoint::new(1000.0, -1000.0),
                MapPoint::new(1000.0, 1000.0),
                MapPoint::new(-1000.0, 1000.0),
            ],
            bounding_box: crate::coordinates::MapBBox::from_coords(
                -1000.0, -1000.0, 1000.0, 1000.0,
            ),
            material: crate::sound_cache::Material::Water,
        }],
    };
    let check = TrajectoryObstacleCheck {
        fast_find_grid: &grid,
        sight_obstacles: crate::sight_obstacle::ObstacleList::empty(),
        water_zones: Some(&water_zones),
    };
    let mut arrow = refresh_test_arrow();
    arrow
        .element
        .set_position(WorldPoint3D::new(0.0, 0.0, 25.0));
    arrow.projectile.dive = false;

    make_arrow_falling_down(&mut arrow, false, Some(&check));

    assert!(arrow.projectile.falling);
    assert!(arrow.projectile.dive);
    assert!(!arrow.projectile.disappear);

    let dry_zones = crate::water_zones::WaterZones::new();
    let dry_check = TrajectoryObstacleCheck {
        water_zones: Some(&dry_zones),
        ..check
    };
    make_arrow_falling_down(&mut arrow, false, Some(&dry_check));
    assert!(
        arrow.projectile.dive,
        "trajectory calculation does not clear an earlier dive flag when a ricochet recomputes a dry fall"
    );
}

/// A projectile that passes close to a target on the ground (not
/// airborne) still misses when the target's posture is one of the
/// "untargetable" postures.  Spot-check one of them
/// (`Posture::Lying`) to confirm the filter actually prunes the
/// snapshot.
/// Advance `arrow_id` for up to `frames` frames and return the first human
/// victim its swept segment reports.
fn first_projectile_victim(
    entities: &mut Entities,
    arrow_id: EntityId,
    frames: usize,
) -> Option<EntityId> {
    let actor_order: Vec<EntityId> = entities.actors().map(|(id, _)| id.into()).collect();
    let mut hit = None;
    for _ in 0..frames {
        let old_position = {
            let Entity::Projectile(arrow) = entities.get_mut(arrow_id).unwrap() else {
                unreachable!()
            };
            if let Some(old) = arrow.projectile.launch_segment_start.take() {
                old
            } else {
                let old = arrow.element.position();
                if arrow.advance_projectile_hourglass() {
                    break;
                }
                old
            }
        };
        hit = projectile_human_victim(
            entities,
            &actor_order,
            &crate::diplomacy::DiplomacyState::default(),
            arrow_id,
            old_position,
        );
        if hit.is_some() {
            break;
        }
    }
    hit
}

#[test]
fn tick_arrows_skips_lying_victim() {
    use crate::element::Posture;

    let mut soldier = make_soldier(50.0, 0.0);
    soldier.set_posture(Posture::Lying);

    // Arrow trajectory aimed directly at where the belt would be
    // if the soldier were upright — but since it's lying, no hit.
    let trajectory = vec![TrajectoryPoint {
        position: WorldPoint3D::new(50.0, 0.0, 25.0),
        time: 2,
    }];
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory,
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Pc(crate::entity_id::PcId(1)),
            WorldPoint3D::new(0.0, 0.0, 25.0),
            MapPoint::new(50.0, 0.0),
        )
    });

    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(soldier), Some(arrow)]);

    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let hit = first_projectile_victim(&mut entities, arrow_id, 10);
    assert_eq!(hit, None);
}

/// Arrow that sails past a target in 3D does not hit it even when
/// their 2D projections coincide.  Previously the 2D point check
/// falsely reported a hit on any arrow passing directly over a
/// target; the 3D line-segment check does not.  Regression test
/// for that gap.
#[test]
fn tick_arrows_does_not_hit_when_arcing_overhead() {
    // Arrow stays well above the soldier's belt (Z=25).
    let trajectory = vec![
        TrajectoryPoint {
            position: WorldPoint3D::new(30.0, 0.0, 80.0),
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D::new(60.0, 0.0, 78.0),
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D::new(90.0, 0.0, 76.0),
            time: 2,
        },
    ];
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory,
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Pc(crate::entity_id::PcId(1)),
            WorldPoint3D::new(0.0, 0.0, 82.0),
            MapPoint::new(90.0, 0.0),
        )
    });

    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_soldier(50.0, 0.0)),
        Some(arrow),
    ]);

    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let hit = first_projectile_victim(&mut entities, arrow_id, 20);
    assert_eq!(hit, None);
}

/// Arrow that shares the soldier's 2D column but passes at belt
/// height hits; trajectory comes down to the belt then continues
/// past.  Complement to [`tick_arrows_does_not_hit_when_arcing_overhead`].
#[test]
fn tick_arrows_hits_through_belt_column() {
    let trajectory = vec![
        TrajectoryPoint {
            position: WorldPoint3D::new(50.0, 0.0, 25.0),
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D::new(80.0, 0.0, 20.0),
            time: 2,
        },
    ];
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory,
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Pc(crate::entity_id::PcId(1)),
            WorldPoint3D::new(0.0, 0.0, 30.0),
            MapPoint::new(80.0, 0.0),
        )
    });
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_soldier(20.0, 0.0)),
        Some(arrow),
    ]);
    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let hit = first_projectile_victim(&mut entities, arrow_id, 20);
    assert_eq!(hit, Some(EntityId::Soldier(crate::entity_id::SoldierId(1))));
}

/// Shield intersection flips the projectile into the falling state
/// and emits a `shield_hit` result.  The projectile keeps flying
/// on a new deflected trajectory toward the ground — it must not
/// despawn on the same tick.
#[test]
fn tick_arrows_inactive_shield_hit_deflects_and_keeps_flying() {
    use crate::element::ActionState;

    // Shield holder facing east (sector 4 = +X), toward the arrow
    // which is flying westward from bow_point (100,…) to target
    // (50,…).  The shield quad projects forward in the holder's
    // facing direction, so the arrow's path intersects it.
    let mut shield_holder = make_soldier(50.0, 0.0);
    {
        let actor = shield_holder.actor_data_mut().unwrap();
        actor.action_state = ActionState::HoldingShield;
        let params = shield_params_for_soldier(20, 40);
        let obs = compute_shield_obstacle(MapPoint::new(50.0, 0.0), 0.0, 4, &params);
        actor.shield_obstacle = Some(obs.into());
    }
    shield_holder.element_data_mut().set_direction_instantly(4);
    // The original game scans every actor and checks only whether a shield is held. Saved
    // mission actors can remain inactive while retaining that state and
    // must still block a projectile crossing their serialized shield.
    shield_holder.element_data_mut().active = false;

    // Arrow flying from +X toward the shield holder at Z=40 —
    // mid-shield height for `shield_params_for_soldier(20, 40)`
    // which places the quad between Z=30 and Z=50.  The holder
    // stands at ground Y=0, so the arrow shares that ground Y and
    // clears the quad only on height, which the Z extent decides.
    let trajectory = vec![TrajectoryPoint {
        position: WorldPoint3D::new(50.0, 0.0, 40.0),
        time: 2,
    }];
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory,
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Soldier(crate::entity_id::SoldierId(1)),
            WorldPoint3D::new(100.0, 0.0, 40.0),
            MapPoint::new(50.0, 0.0),
        )
    });

    let entities = entity_table(vec![
        Some(make_pc(100.0, 0.0)),
        Some(shield_holder),
        Some(arrow),
    ]);

    let (mut engine, assets) = projectile_engine(entities);
    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    for _ in 0..10 {
        engine.tick_existing_projectile(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            arrow_id,
        );
        if let Entity::Projectile(p) = engine.world.entities.get(arrow_id).unwrap() {
            if p.projectile.falling {
                break;
            }
        }
    }
    assert!(projectile_activation_seen(
        &engine,
        EntityId::Soldier(crate::entity_id::SoldierId(1)),
        Command::ParryShield
    ));
    assert!(
        engine
            .world
            .entities
            .get(arrow_id)
            .unwrap()
            .element_data()
            .active,
        "shield deflection does not deactivate the projectile"
    );
    // The projectile should be flagged as falling, and the hit
    // check must now skip (falling arrows pass through bodies).
    match engine
        .world
        .entities
        .get_at_index(2)
        .map(|(_, entity)| entity)
        .unwrap()
    {
        Entity::Projectile(p) => {
            assert!(
                p.projectile.falling,
                "shield deflection flips arrow into falling state"
            );
            assert!(
                p.projectile.flying,
                "falling arrow still visually flying (arcs to ground)"
            );
            assert_ne!(
                p.element.position(),
                (WorldPoint3D::new(50.0, 40.0, 40.0)),
                "falling advances the trajectory immediately"
            );
        }
        _ => panic!("expected projectile"),
    }
}

#[test]
fn serialized_shield_state_restores_exact_collision_geometry() {
    let original = compute_shield_obstacle(
        MapPoint::new(1214.0, 415.0),
        0.0,
        4,
        &shield_params_for_soldier(40, 20),
    );
    let mut saved = crate::element::HumanShieldState::default();
    for (saved_point, point) in saved.points.iter_mut().zip(&original.obstacle_points) {
        saved_point.obstacle = [point.x, point.y, point.z_top, point.z_bottom];
        saved_point.polygon = MapPoint::new(point.x, point.y);
    }
    let world_point = |point: [f32; 3]| WorldPoint3D::new(point[0], point[1], point[2]);
    saved.top_plane.origin = world_point(original.top_plane_points[0]);
    saved.top_plane.a = world_point(original.top_plane_points[1]);
    saved.top_plane.b = world_point(original.top_plane_points[2]);
    saved.bottom_plane.origin = world_point(original.bottom_plane_points[0]);
    saved.bottom_plane.a = world_point(original.bottom_plane_points[1]);
    saved.bottom_plane.b = world_point(original.bottom_plane_points[2]);
    saved.on_ground = original.on_ground;

    let restored = shield_obstacle_from_serialized_state(&saved);
    assert_eq!(restored.obstacle_points, original.obstacle_points);
    assert_eq!(restored.top_plane_points, original.top_plane_points);
    assert_eq!(restored.bottom_plane_points, original.bottom_plane_points);
    assert_eq!(restored.box_3d_min, original.box_3d_min);
    assert_eq!(restored.box_3d_max, original.box_3d_max);
    let y = 0.5 * (original.box_3d_min[1] + original.box_3d_max[1]);
    let z = 0.5 * (original.box_3d_min[2] + original.box_3d_max[2]);
    let ray = (
        [original.box_3d_max[0] + 10.0, y, z],
        [original.box_3d_min[0] - 10.0, y, z],
    );
    assert!(original.is_blocking_ray_3d(ray.0, ray.1));
    assert_eq!(
        restored.is_blocking_ray_3d(ray.0, ray.1),
        original.is_blocking_ray_3d(ray.0, ray.1)
    );
}

#[test]
fn projectile_uses_stale_shield_until_explicit_refresh() {
    fn run(
        explicit_refresh: bool,
    ) -> (Option<EntityId>, ([f32; 3], [f32; 3]), ([f32; 3], [f32; 3])) {
        let mut holder = make_pc(50.0, 0.0);
        holder.element_data_mut().set_direction_instantly(4);
        holder.actor_data_mut().unwrap().action_state = ActionState::HoldingShield;

        // Authoritative geometry deliberately retained from an old
        // position. A projectile tick must not silently move it to the
        // actor's current position/facing.
        let stale = compute_shield_obstacle(
            MapPoint::new(-50.0, 100.0),
            0.0,
            4,
            &shield_params_for_pc(false),
        );
        holder.actor_data_mut().unwrap().shield_obstacle = Some(stale.into());
        if explicit_refresh {
            let mut profiles = ProfileManager::new();
            profiles
                .characters
                .push(crate::profiles::CharacterProfile::default());
            refresh_retained_shield_obstacle(&mut holder, &profiles);
        }

        let before_obstacle = holder
            .actor_data()
            .unwrap()
            .shield_obstacle
            .as_ref()
            .unwrap();
        let before = (before_obstacle.box_3d_min, before_obstacle.box_3d_max);
        let arrow = spawn_arrow(SpawnArrowParams {
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D::new(50.0, 0.0, 40.0),
                time: 2,
            }],
            initial_velocity: WorldVec3D::new(-1.0, 0.0, 0.0),
            ..SpawnArrowParams::test_flat(
                EntityId::Pc(crate::entity_id::PcId(0)),
                EntityId::Pc(crate::entity_id::PcId(1)),
                WorldPoint3D::new(100.0, 0.0, 40.0),
                MapPoint::new(50.0, 0.0),
            )
        });
        let entities = entity_table(vec![Some(make_pc(100.0, 0.0)), Some(holder), Some(arrow)]);
        let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
        let Entity::Projectile(arrow) = entities.get(arrow_id).unwrap() else {
            unreachable!()
        };
        let actor_order: Vec<EntityId> = entities.actors().map(|(id, _)| id.into()).collect();
        let shield_hit = projectile_shield_holder(
            &entities,
            &actor_order,
            arrow.projectile.launch_segment_start.unwrap(),
            WorldPoint3D::new(50.0, 0.0, 40.0),
            arrow.projectile.velocity_increment,
        );
        let after_obstacle = entities
            .get_at_index(1)
            .unwrap()
            .1
            .actor_data()
            .unwrap()
            .shield_obstacle
            .as_ref()
            .unwrap();
        let after = (after_obstacle.box_3d_min, after_obstacle.box_3d_max);
        (shield_hit, before, after)
    }

    let (stale_hit, stale_before, stale_after) = run(false);
    assert_eq!(
        stale_hit, None,
        "stale retained box must not block the arrow"
    );
    assert_eq!(
        stale_after, stale_before,
        "projectile processing must not recompute retained shield geometry"
    );

    let (fresh_hit, fresh_before, fresh_after) = run(true);
    assert_eq!(
        fresh_hit,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        "an explicit shield refresh must publish geometry that blocks the arrow"
    );
    assert_eq!(fresh_after, fresh_before);
}

#[test]
fn diagonal_soldier_shield_uses_update_box_normalization() {
    // Savegame_nicouzouf/Profile_001/Savegame_020/replay-006, frame 584:
    // this segment passes beside Soldier 58's retained sector-15 shield.
    // Treating the raw compass vector as box updating's already-normalized
    // input widens/rotates the quad onto the arrow and invents a parry.
    let obstacle = compute_shield_obstacle(
        MapPoint::new(867.4834, 406.6131),
        0.0,
        15,
        &shield_params_for_soldier(40, 50),
    );
    let current = [853.1472, 379.94678, 25.0];
    let old = [824.3604, 383.21008, 28.75];

    assert!(!obstacle.is_blocking_ray_3d(current, old));
}

#[test]
fn non_shield_arrow_ricochet_advances_immediately() {
    crate::sim_rng::with_seed(1, |sim| {
        // Two waypoints: the spawn primer consumes the first segment, so
        // the ricochet still sees a queued waypoint and derives its fall
        // sector from live flight rather than the orientation cache.
        let trajectory = vec![
            TrajectoryPoint {
                position: WorldPoint3D::new(25.0, 0.0, 0.0),
                time: 1,
            },
            TrajectoryPoint {
                position: WorldPoint3D::new(50.0, 0.0, 0.0),
                time: 2,
            },
        ];
        let arrow = spawn_arrow(SpawnArrowParams {
            trajectory,
            ..SpawnArrowParams::test_flat(
                EntityId::Pc(crate::entity_id::PcId(0)),
                EntityId::Pc(crate::entity_id::PcId(1)),
                WorldPoint3D::new(0.0, 0.0, 0.0),
                MapPoint::new(50.0, 0.0),
            )
        });
        let mut projectile = match arrow {
            Entity::Projectile(p) => p,
            _ => panic!("expected arrow projectile"),
        };
        projectile.element.set_direction_instantly(4);
        let impact_position = projectile.element.position();

        make_arrow_falling_down(&mut projectile, false, None);

        assert!(projectile.projectile.falling);
        assert!(projectile.projectile.flying);
        assert_eq!(
            projectile.projectile.falling_direction, 12,
            "armor ricochet reverses the flight sector for the fall"
        );
        assert_ne!(
            projectile.element.position(),
            impact_position,
            "falling advances armor ricochets too"
        );

        // The tumble visual is a presentation pass: it renders on the
        // deferred refresh before the next tick, not during
        // falling-motion setup itself.
        refresh_arrow_after_previous_hourglass(sim, &mut projectile);
        assert_eq!(
            projectile.element.sprite.current_row, 12,
            "impact-frame render uses the first falling sector"
        );
        assert!((3..=5).contains(&projectile.element.sprite.current_frame));
        assert_eq!(
            projectile.projectile.falling_direction, 10,
            "falling refresh rotates the next tumble sector by -2"
        );
    });
}

#[test]
fn shield_ricochet_with_empty_trajectory_finishes_nested_hourglass() {
    // A shield catches a segment crossing just below ground. The deflection
    // has no flight left, so its nested update must still finish the impact.
    let endpoint = WorldPoint3D::new(98.988_8, 861.410_2, -0.000_000_953_674_3);
    let Entity::Projectile(arrow) = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: endpoint,
        trajectory_origin: endpoint.to_map(),
        target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
        target_pos: endpoint.to_map(),
        trajectory: vec![],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D::new(-47.394_653, 46.451_09, -7.129_664),
    }) else {
        panic!("spawn_arrow returned a non-projectile entity");
    };
    let mut holder = make_soldier(140.006_48, 588.663_45);
    holder.element_data_mut().set_direction_instantly(15);
    let actor = holder.actor_data_mut().unwrap();
    actor.action_state = ActionState::HoldingShield;
    actor.shield_obstacle = Some(Box::new(compute_shield_obstacle(
        MapPoint::new(140.006_48, 588.663_45),
        0.0,
        15,
        &shield_params_for_soldier(40, 50),
    )));
    let (mut engine, assets) = projectile_engine(entity_table(vec![
        Some(make_pc(100.0, 800.0)),
        Some(holder),
        Some(Entity::Projectile(arrow)),
    ]));
    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let Entity::Projectile(arrow) = engine.world.entities.get_mut(arrow_id).unwrap() else {
        unreachable!();
    };
    arrow.element.set_position(endpoint);
    arrow
        .element
        .set_position_map_preserving_3d(endpoint.to_map());
    arrow.element.set_direction_instantly(10);
    arrow.projectile.trajectory.clear();
    arrow.projectile.trajectory_runtime.clear();
    arrow.projectile.trajectory_frame_count = 0;
    arrow.projectile.launch_segment_start =
        Some(WorldPoint3D::new(146.383_45, 814.959_1, 7.129_663_5));
    arrow.projectile.velocity_increment = WorldVec3D::new(-47.394_653, 46.451_09, -7.129_664);
    arrow.projectile.flying = true;

    assert!(engine.tick_new_projectile_once(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        arrow_id,
    ));
    assert!(projectile_activation_seen(
        &engine,
        EntityId::Soldier(crate::entity_id::SoldierId(1)),
        Command::ParryShield,
    ));
    let Entity::Projectile(arrow) = engine.world.entities.get(arrow_id).unwrap() else {
        unreachable!();
    };
    assert!(arrow.projectile.falling);
    let position = arrow.element.position();
    assert_eq!(position.x.to_bits(), endpoint.x.to_bits());
    assert_eq!(position.y.to_bits(), endpoint.y.to_bits());
    assert_eq!(position.z.to_bits(), 0.001_f32.to_bits());
    assert_eq!(arrow.element.sprite.position_iface.old_position(), endpoint);
    assert_eq!(arrow.element.optional_layer(), None);
    assert_eq!(arrow.element.sector(), None);
    assert!(!arrow.projectile.flying);
    assert_eq!(arrow.projectile.trajectory_frame_count, u16::MAX);
    assert_eq!(arrow.projectile.velocity_increment, WorldVec3D::ZERO);
    assert_eq!(
        arrow.element.sprite.position_iface.map_position()
            - arrow.element.sprite.position_iface.old_map_position(),
        MapVec::new(0.0, -0.000_976_562_5)
    );
}

#[test]
fn ground_crossing_is_attributed_to_first_front_facing_shield() {
    let mut holder = make_soldier(140.006_48, 588.663_45);
    holder.element_data_mut().set_direction_instantly(15);
    let actor = holder.actor_data_mut().expect("soldier actor data");
    actor.action_state = ActionState::HoldingShield;
    actor.shield_obstacle = Some(Box::new(compute_shield_obstacle(
        MapPoint::new(140.006_48, 588.663_45),
        0.0,
        15,
        &shield_params_for_soldier(40, 50),
    )));
    let entities = entity_table(vec![Some(holder)]);
    let holder_id = entities.get_at_index(0).expect("shield holder slot").0;

    let old = WorldPoint3D::new(146.383_45, 814.959_1, 7.129_663_5);
    let new = WorldPoint3D::new(98.988_8, 861.410_2, -0.000_000_953_674_3);
    let increment = WorldVec3D::new(-47.394_653, 46.451_09, -7.129_664);
    let obstacle = entities
        .get(holder_id)
        .and_then(Entity::actor_data)
        .and_then(|actor| actor.shield_obstacle.as_ref())
        .expect("retained shield obstacle");
    assert!(
        !obstacle.is_blocking_ray_3d([new.x, new.y, new.z], [old.x, old.y, old.z]),
        "fixture must prove the shield geometry itself is far from the arrow"
    );

    assert_eq!(
        projectile_shield_holder(&entities, &[holder_id], old, new, increment),
        Some(holder_id),
        "reachability tests the ground crossing before the shield obstacle list"
    );
}

/// An arrow that runs out of trajectory without hitting anything
/// stops flying on the landing tick and retains its grounded presentation.
#[test]
fn tick_arrows_miss_and_land_despawns() {
    let trajectory = vec![TrajectoryPoint {
        position: WorldPoint3D::new(10.0, 0.0, 0.0),
        time: 1,
    }];
    let arrow = spawn_arrow(SpawnArrowParams {
        trajectory,
        ..SpawnArrowParams::test_flat(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Pc(crate::entity_id::PcId(0)),
            WorldPoint3D::new(0.0, 0.0, 5.0),
            MapPoint::new(10.0, 0.0),
        )
    });
    // No other humans in range — arrow will fly out and land.
    let entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(arrow)]);

    let (mut engine, assets) = projectile_engine(entities);
    let projectile = engine.world.entities.get_at_index(1).unwrap().0;
    let mut landed = false;
    for _ in 0..10 {
        engine.tick_existing_projectile(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            projectile,
        );
        let Entity::Projectile(arrow) = engine.world.entities.get(projectile).unwrap() else {
            panic!("expected arrow");
        };
        landed = !arrow.projectile.flying;
        if landed {
            break;
        }
    }
    assert!(landed, "arrow that misses should stop flying on landing");
    assert!(
        engine.world.entities.get(projectile).unwrap().is_active(),
        "grounded arrow survives until presentation retirement"
    );
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());
}

#[test]
fn one_waypoint_falling_arrow_into_hole_disappears_without_ground_snap() {
    let endpoint = WorldPoint3D::new(10.0, 0.0, -0.5);
    let endpoint_map = endpoint.to_map();
    let water_zones = crate::water_zones::WaterZones {
        zones: vec![crate::water_zones::WaterZone {
            points: vec![
                MapPoint::new(0.0, -10.0),
                MapPoint::new(20.0, -10.0),
                MapPoint::new(20.0, 10.0),
                MapPoint::new(0.0, 10.0),
            ],
            bounding_box: crate::coordinates::MapBBox::from_coords(0.0, -10.0, 20.0, 10.0),
            material: crate::sound_cache::Material::Hole,
        }],
    };
    assert!(water_zones.landing_is_in_hole(endpoint_map));

    let Entity::Projectile(mut arrow) = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D::new(0.0, 0.0, 5.0),
        trajectory_origin: MapPoint::new(0.0, 0.0),
        target: EntityId::Pc(crate::entity_id::PcId(0)),
        target_pos: endpoint_map,
        trajectory: vec![],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D::new(1.0, 0.0, 0.0),
    }) else {
        panic!("spawn_arrow returned a non-projectile entity");
    };
    arrow.projectile.trajectory = vec![TrajectoryPoint {
        position: endpoint,
        time: 1,
    }];
    // Match falling setup's fresh trajectory result. The empty
    // spawn above exhausted its placeholder trajectory and left the runtime
    // counter at the retired sentinel; Original starts this replacement
    // trajectory from its first point instead.
    arrow.projectile.trajectory_frame_count = 0;
    arrow.projectile.flying = true;
    arrow.projectile.launch_segment_start = None;
    preserve_falling_hole_disappearance(&mut arrow, true);
    assert!(
        arrow.projectile.disappear,
        "hole-fall setup marks even a one-waypoint trajectory"
    );

    preserve_falling_hole_disappearance(&mut arrow, false);
    assert!(
        arrow.projectile.disappear,
        "recomputing a dry falling trajectory must preserve an existing disappear flag"
    );

    arrow.advance_trajectory_one_frame();
    assert_eq!(arrow.element.position().z.to_bits(), endpoint.z.to_bits());
    let entities = entity_table(vec![
        Some(make_pc(100.0, 100.0)),
        Some(Entity::Projectile(arrow)),
    ]);
    let (mut engine, assets) = projectile_engine(entities);
    let projectile = engine.world.entities.get_at_index(1).unwrap().0;
    let retain = engine.tick_existing_projectile(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        projectile,
    );
    assert!(
        !retain,
        "terminal projectile asks its concrete owner to retire it"
    );
    let Entity::Projectile(arrow) = engine.world.entities.get(projectile).unwrap() else {
        panic!("falling arrow changed concrete entity kind");
    };
    assert!(!arrow.projectile.flying);
    assert_eq!(
        arrow.element.position().z.to_bits(),
        endpoint.z.to_bits(),
        "disappearance returns before the obstacle impact's +0.001 elevation snap"
    );
    assert!(!arrow.element.sprite.position_iface.is_moving());
}

#[test]
fn falling_arrow_into_water_retires_without_ground_snap() {
    let endpoint = WorldPoint3D::new(10.0, 0.0, -0.000_001_907_348_6);
    let Entity::Projectile(mut arrow) = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D::new(0.0, 0.0, 5.0),
        trajectory_origin: MapPoint::new(0.0, 0.0),
        target: EntityId::Pc(crate::entity_id::PcId(0)),
        target_pos: endpoint.to_map(),
        trajectory: vec![],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D::new(1.0, 0.0, 0.0),
    }) else {
        panic!("spawn_arrow returned a non-projectile entity");
    };
    arrow.element.set_position(endpoint);
    arrow
        .element
        .set_position_map_preserving_3d(endpoint.to_map());
    arrow.projectile.trajectory.clear();
    arrow.projectile.trajectory_frame_count = 0;
    arrow.projectile.launch_segment_start = None;
    arrow.projectile.falling = true;
    arrow.projectile.flying = true;
    arrow.projectile.dive = true;

    let entities = entity_table(vec![
        Some(make_pc(100.0, 100.0)),
        Some(Entity::Projectile(arrow)),
    ]);
    let (mut engine, assets) = projectile_engine(entities);
    let projectile = engine.world.entities.get_at_index(1).unwrap().0;
    let retain = engine.tick_existing_projectile(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        projectile,
    );
    assert!(
        !retain,
        "terminal projectile asks its concrete owner to retire it"
    );
    let Entity::Projectile(arrow) = engine.world.entities.get(projectile).unwrap() else {
        panic!("falling arrow changed concrete entity kind");
    };
    assert!(!arrow.projectile.flying);
    assert_eq!(arrow.element.position().z.to_bits(), endpoint.z.to_bits());
    assert!(!arrow.element.sprite.position_iface.is_moving());
    assert!(!arrow.element.sprite.position_iface.is_moving_map());
}

/// Wasp nest thrown at a ground target bursts (`flying == false`)
/// once its bounce trajectory is exhausted.  Unlike arrows, the
/// nest keeps a projectile slot for the post-impact wasp swarm
/// spawn — here we just assert it stops flying.
#[test]
fn spawn_wasp_nest_lands_and_stops_flying() {
    let throw_pos = WorldPoint3D::new(0.0, 0.0, 50.0);
    let target_pos = WorldPoint3D::new(80.0, 0.0, 0.0);
    let nest = spawn_wasp_nest(
        EntityId::Pc(crate::entity_id::PcId(0)),
        throw_pos,
        target_pos,
        0,
        None,
    );

    match &nest {
        Entity::Projectile(p) => {
            assert!(p.projectile.flying, "nest starts flying");
            assert_eq!(p.object.object_type, ObjectType::BonusWaspNest);
            assert!(
                !p.projectile.trajectory.is_empty(),
                "wasp nest must produce a ballistic trajectory"
            );
        }
        _ => panic!("expected projectile"),
    }

    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(nest)]);
    // Wasp nests are skipped by `tick_arrows` (their impact burst +
    // swarm spawn lives on the engine in `tick_wasp_nests`).  Drive
    // the trajectory directly here via `advance_trajectory_one_frame`;
    // bouncing nests can produce the full 50-waypoint trajectory
    // (~100 ticks at TIME_FLYSEGMENT=2), so 300 iterations is a
    // generous bound.
    for _ in 0..300 {
        if let Some(Entity::Projectile(p)) = entities.get_mut_at_index(1).map(|(_, entity)| entity)
        {
            if !p.projectile.flying {
                break;
            }
            p.advance_trajectory_one_frame();
        }
    }
    let p = match entities.get_at_index(1).map(|(_, entity)| entity).unwrap() {
        Entity::Projectile(p) => p,
        _ => panic!("nest entity lost"),
    };
    assert!(
        !p.projectile.flying,
        "wasp nest must stop flying once its trajectory is exhausted"
    );
}

#[test]
fn self_priming_thrown_object_paths_are_advanced_exactly_once_by_spawn() {
    let thrower = EntityId::Pc(crate::entity_id::PcId(0));
    let start = WorldPoint3D::new(0.0, 0.0, 20.0);
    let end = WorldPoint3D::new(200.0, 0.0, 0.0);
    let thrown = [
        spawn_net(thrower, start, end, 0, None),
        spawn_wasp_nest(thrower, start, end, 0, None),
        spawn_apple(thrower, start, end, Some(thrower), None, 0, None),
        spawn_stone(thrower, start, end, Some(thrower), None, 0, None),
    ];
    for (index, entity) in thrown.into_iter().enumerate() {
        let (position, frame_count) = match entity {
            Entity::Projectile(projectile) => (
                projectile.element.position(),
                projectile.projectile.frame_count,
            ),
            Entity::Net(net) => (net.element.position(), net.projectile.frame_count),
            _ => unreachable!(),
        };
        assert_ne!(
            position, start,
            "throw path {index} omitted its explicit primer"
        );
        assert_eq!(
            frame_count, 1,
            "throw path {index} advanced more than once before insertion"
        );
    }
}

#[test]
fn purse_and_coin_constructors_defer_their_virtual_primer_to_engine_owner() {
    let thrower = EntityId::Pc(crate::entity_id::PcId(0));
    let start = WorldPoint3D::new(0.0, 0.0, 20.0);
    let end = WorldPoint3D::new(200.0, 0.0, 0.0);
    for entity in [
        spawn_purse(thrower, start, end, 0, None),
        spawn_coin(
            None,
            start,
            end,
            crate::position_interface::Layer::new(0),
            None,
            None,
            APEX_BEGGAR_COIN,
            None,
        ),
    ] {
        let Entity::Projectile(projectile) = entity else {
            unreachable!()
        };
        assert_eq!(projectile.element.position(), start);
        assert_eq!(projectile.projectile.frame_count, 0);
    }
}

#[test]
fn projectile_landing_retains_exact_sector_identity() {
    let exact_sector = crate::position_interface::SectorHandle::new(7)
        .expect("test sector")
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(41).expect("test arena"));
    let mut element = ElementData::default();

    apply_projectile_landing_resolution(
        &mut element,
        crate::fast_find_grid::ProjectileLandingResolution {
            obstacle_index: None,
            obstacle_plane: None,
            layer: crate::position_interface::Layer::new(2),
            sector: Some(exact_sector),
            blocked_by_motion_obstacle: false,
        },
        None,
    );

    assert_eq!(element.sector(), Some(exact_sector));
    assert_eq!(
        element.sector().and_then(|sector| sector.arena_index()),
        exact_sector.arena_index(),
        "landing membership must retain the original game's exact sector identity"
    );
}

fn refresh_test_arrow() -> ElementProjectile {
    let mut element = crate::engine::test_support::extra_engine_combat::test_element(
        ElementKind::ObjectProjectile,
        true,
    );
    element.sprite.current_row = 9;
    element.sprite.current_frame = 2;
    ElementProjectile {
        element,
        object: ObjectData {
            object_type: ObjectType::Arrow,
            animation: Animation::ObjectFlying,
            ..Default::default()
        },
        projectile: ProjectileData {
            flying: true,
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D::new(10.0, 0.0, 100.0),
                time: 4,
            }],
            // Deliberately horizontal: Refresh must use the next queued
            // point rather than this current-segment increment.
            velocity_increment: WorldVec3D::new(1.0, 0.0, 0.0),
            ..Default::default()
        },
    }
}

#[test]
fn arrow_refresh_is_deferred_and_uses_next_waypoint_pitch() {
    let mut arrow = refresh_test_arrow();
    assert_eq!(
        (
            arrow.element.sprite.current_row,
            arrow.element.sprite.current_frame
        ),
        (9, 2)
    );

    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);

    // The +X ground direction lies in compass sector 4 (the original
    // sector partition puts (0,-1) in sector 0 and (1,0) in sector 4).
    assert_eq!(arrow.projectile.last_orientation_sector, 4);
    assert_eq!(arrow.projectile.last_orientation_azimuth, 60);
    assert_eq!(
        (
            arrow.element.sprite.current_row,
            arrow.element.sprite.current_frame
        ),
        (4, 8)
    );
}

#[test]
fn arrow_refresh_zero_length_queued_endpoint_resets_orientation_like_i386() {
    let mut arrow = refresh_test_arrow();
    let endpoint = arrow.projectile.trajectory[0].position;
    arrow.element.set_position(endpoint);
    arrow.projectile.last_orientation_sector = 8;
    arrow.projectile.last_orientation_azimuth = -60;

    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);

    assert_eq!(arrow.projectile.last_orientation_sector, 0);
    assert_eq!(arrow.projectile.last_orientation_azimuth, 0);
    assert_eq!(
        (
            arrow.element.sprite.current_row,
            arrow.element.sprite.current_frame
        ),
        (0, 4)
    );
}

#[test]
fn nested_dialogue_refresh_publishes_new_arrow_in_creation_frame() {
    // QuickSave frame 35731 creates an arrow during the actor update and
    // then executes PlayDialog from the sequence-manager tick. The
    // dialogue's nested game refresh exposes the orientation before
    // frame recording instead of waiting for the ordinary deferred pass.
    let mut arrow = refresh_test_arrow();
    arrow.element.sprite.current_row = 0;
    arrow.element.sprite.current_frame = 0;

    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);

    assert_eq!(
        (
            arrow.element.sprite.current_row,
            arrow.element.sprite.current_frame
        ),
        (4, 8)
    );
}

#[test]
fn falling_arrow_refresh_consumes_exactly_one_draw_and_rotates_afterward() {
    let mut arrow = refresh_test_arrow();
    arrow.projectile.falling = true;
    arrow.projectile.falling_direction = 6;

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow)
    });

    assert_eq!(draws, vec![crate::sim_rng::RngSite::ArrowFallingFrame]);
    assert_eq!(arrow.element.sprite.current_row, 6);
    assert!((3..=5).contains(&arrow.element.sprite.current_frame));
    assert_eq!(arrow.projectile.falling_direction, 4);
}

#[test]
fn live_flying_arrow_with_world_movement_reuses_orientation_cache() {
    let mut arrow = refresh_test_arrow();
    arrow.projectile.trajectory.clear();
    arrow.projectile.trajectory_frame_count = 0;
    arrow.projectile.last_orientation_sector = 7;
    arrow.projectile.last_orientation_azimuth = -30;
    arrow
        .element
        .sprite
        .position_iface
        .set_old_position(WorldPoint3D::new(-1.0, 0.0, 0.0));

    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);

    assert!(arrow.element.active);
    assert_eq!(
        (
            arrow.element.sprite.current_row,
            arrow.element.sprite.current_frame
        ),
        (7, 3)
    );

    // The exhausted trajectory's next update stops flight and snaps
    // the landing height. Original exposes that movement for one more
    // active snapshot. The following stopped projectile tick owns
    // movement initialization; refreshing only observes that snapshot and retires it.
    arrow.projectile.flying = false;
    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);
    assert!(arrow.element.active);
    assert!(arrow.element.sprite.position_iface.is_moving());
    arrow.element.sprite.position_iface.new_move();
    assert!(!arrow.element.sprite.position_iface.is_moving());
    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);
    assert!(!arrow.element.active);
}

#[test]
fn stopped_fx_hit_refresh_waits_for_hourglass_new_move_before_retirement() {
    let mut arrow = refresh_test_arrow();
    arrow.projectile.trajectory.clear();
    arrow.projectile.trajectory_frame_count = 0;
    arrow.projectile.flying = false;
    arrow
        .element
        .sprite
        .position_iface
        .set_old_position(WorldPoint3D::new(-1.0, 0.0, 0.0));

    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);

    assert!(
        arrow.element.active,
        "Refresh must preserve the moving snapshot exposed by successful HitTarget"
    );
    assert!(arrow.element.sprite.position_iface.is_moving());

    // Original-game projectile updates apply movement before checking
    // the flying flag, even for an arrow already stopped by its target hit.
    arrow.element.sprite.position_iface.new_move();
    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);
    assert!(!arrow.element.active);
}

#[test]
fn stopped_moving_empty_trajectory_ignores_retained_counter_until_settled() {
    let mut arrow = refresh_test_arrow();
    arrow.projectile.trajectory.clear();
    arrow.projectile.falling = false;
    arrow.projectile.flying = false;
    arrow.projectile.trajectory_frame_count = 3;
    // A successful HitTarget stops flight and deletes the trajectory but
    // leaves the current segment's counter and movement intact.
    arrow
        .element
        .sprite
        .position_iface
        .set_old_position(WorldPoint3D::new(-1.0, 0.0, 0.0));

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow)
    });

    assert!(arrow.element.active);
    assert!(draws.is_empty());

    arrow.element.sprite.position_iface.new_move();
    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);
    assert!(!arrow.element.active);
}

#[test]
fn stopped_settled_empty_trajectory_retires_with_retained_counter() {
    let mut arrow = refresh_test_arrow();
    arrow.projectile.trajectory.clear();
    arrow.projectile.falling = false;
    arrow.projectile.flying = false;
    arrow.projectile.trajectory_frame_count = 3;

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow)
    });

    assert!(!arrow.element.active);
    assert!(draws.is_empty());
}

#[test]
fn non_falling_target_hit_with_leftover_countdown_exposes_stopped_snapshot() {
    let mut arrow = refresh_test_arrow();
    arrow.projectile.trajectory.clear();
    arrow.projectile.flying = false;
    arrow.projectile.falling = false;
    arrow.projectile.trajectory_frame_count = 1;
    arrow
        .element
        .sprite
        .position_iface
        .set_old_position(WorldPoint3D::new(-1.0, 0.0, 0.0));

    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);
    assert!(
        arrow.element.active,
        "HitTarget movement keeps the arrow alive for this Refresh"
    );
    assert!(arrow.element.sprite.position_iface.is_moving());

    // The next stopped projectile tick, not presentation refresh, owns movement bookkeeping.
    arrow.element.sprite.position_iface.new_move();
    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);
    assert!(
        !arrow.element.active,
        "the following stationary Refresh retires the arrow"
    );
}

#[test]
fn settled_flying_endpoint_retires_without_another_falling_frame() {
    let mut arrow = refresh_test_arrow();
    arrow.projectile.trajectory.clear();
    arrow.projectile.trajectory_frame_count = 0;
    arrow.projectile.falling = true;
    let published_sprite = (
        arrow.element.sprite.current_row,
        arrow.element.sprite.current_frame,
    );

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow)
    });

    assert!(!arrow.element.active);
    assert!(draws.is_empty());
    assert_eq!(
        (
            arrow.element.sprite.current_row,
            arrow.element.sprite.current_frame,
        ),
        published_sprite,
        "settled retirement preserves the endpoint sprite published by the preceding Refresh"
    );
    assert!(!arrow.element.sprite.position_iface.is_moving());
    assert!(!arrow.element.sprite.position_iface.is_moving_map());
}

#[test]
fn moving_falling_endpoint_survives_refresh_even_with_settled_sprite_cache() {
    let mut arrow = refresh_test_arrow();
    arrow.projectile.trajectory.clear();
    arrow.projectile.trajectory_frame_count = 0;
    arrow.projectile.falling = true;
    arrow
        .element
        .sprite
        .position_iface
        .set_old_position(WorldPoint3D::new(-1.0, 0.0, 0.0));

    // Original-game arrow refresh compares old and current positions, which
    // are the 3D position-interface values. The separately serialized
    // sprite-space cache is not part of its retirement decision.
    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow)
    });

    assert!(arrow.element.active);
    assert!(arrow.element.sprite.position_iface.is_moving());
    assert_eq!(draws, vec![crate::sim_rng::RngSite::ArrowFallingFrame]);
}

#[test]
fn non_falling_flying_arrow_retires_after_stationary_final_waypoint() {
    let mut arrow = refresh_test_arrow();
    arrow.projectile.trajectory.clear();
    arrow.projectile.trajectory_frame_count = 0;
    arrow.projectile.flying = true;
    arrow.projectile.falling = false;

    refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);

    assert!(!arrow.element.active);
    assert!(arrow.projectile.flying);
    assert!(!arrow.element.sprite.position_iface.is_moving());
    assert_eq!(arrow.element.position().z, 0.0);
}

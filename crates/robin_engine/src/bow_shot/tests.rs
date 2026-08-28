//! Bow-shot unit and parity regression tests.

use super::*;
use crate::coordinates::{MapVec, SpriteFrameOffset, SpriteLocalPoint};
use crate::element::{
    ActorData, ElementKind, ElementTarget, FxData, HumanData, TargetData, TargetFilter,
};
use crate::element::{ActorPc, ActorSoldier, NpcData, PcData, SoldierData};
use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

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

fn make_pc(x: f32, y: f32) -> Entity {
    let mut element = ElementData {
        kind: ElementKind::ActorPc,
        active: true,
        ..ElementData::default()
    };
    element.set_position_map(MapPoint { x, y });
    Entity::Pc(ActorPc {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData::default(),
    })
}

fn make_anonymous_pc(x: f32, y: f32) -> Entity {
    let mut pc = make_pc(x, y);
    pc.element_data_mut().posture = Posture::AnonymousArcher;
    pc
}

fn make_soldier(x: f32, y: f32) -> Entity {
    make_soldier_with_camp(x, y, crate::element::Camp::Royalists)
}

fn make_soldier_with_camp(x: f32, y: f32, camp: crate::element::Camp) -> Entity {
    let mut element = ElementData {
        kind: ElementKind::ActorSoldier,
        active: true,
        ..ElementData::default()
    };
    element.set_position_map(MapPoint { x, y });
    let npc = NpcData {
        life_points: 100,
        ..Default::default()
    };
    Entity::Soldier(ActorSoldier {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        npc,
        soldier: SoldierData {
            cached_camp: camp,
            ..SoldierData::default()
        },
    })
}

/// Savegame_Nescafe/Profile_002/Continue replay-007 reaches these exact
/// bits at Original frame 590. `new - increment` shifts the old X by one
/// bit and fails the strict range gate; NewMove's saved old position hits.
#[test]
fn existing_arrow_collision_uses_new_move_old_position() {
    let mut victim = make_pc(1_040.648_1, 1_915.162_7);
    victim
        .element_data_mut()
        .set_position(WorldPoint3D::new(1_040.648_1, 1_915.162_7, 0.0));
    let shooter = make_soldier_with_camp(772.0, 1796.0, crate::element::Camp::Lacklandists);

    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        ..ElementData::default()
    };
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

    let results = tick_arrows(
        &crate::sim_rng::test_context(),
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );
    assert!(
        results
            .iter()
            .any(|result| result.hit_target == Some(victim_id))
    );
}

fn make_arrow_target(x: f32, y: f32) -> Entity {
    let mut element = ElementData {
        kind: ElementKind::Target,
        active: true,
        ..ElementData::default()
    };
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
/// triple to `begin_bow_shot` / `tick_bow_shots`.
fn launch_test_shoot_element(
    shooter: EntityId,
    target: EntityId,
) -> (SequenceManager, SequenceId, usize) {
    let mut sm = SequenceManager::new();
    let elem = build_shoot_bow_element(shooter, target);
    let seq_id = sm.launch_element(elem);
    // Transition the element to InProgress so `current_element_for_actor`
    // finds it — the engine does this as part of the hourglass dispatch,
    // which the tests skip.
    sm.element_in_progress(seq_id, 0);
    (sm, seq_id, 0)
}

fn set_test_action_state_after_transition(
    sm: &mut SequenceManager,
    seq_id: SequenceId,
    elem_idx: usize,
    action_state: ActionState,
) {
    sm.get_element_mut(seq_id, elem_idx)
        .unwrap()
        .action_state_after_transition = action_state;
}

fn bind_test_bow_release_rows(entity: &mut Entity, order_type: OrderType) {
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
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

#[test]
fn begin_bow_shot_sets_shooter_state() {
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    let result = begin_bow_shot(
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
        "C++ ShootBow translation must not force the actor's action state before queued bow orders run"
    );
    assert!(actor.active_shot.is_active());
    assert_eq!(actor.active_shot.target, Some(target_id));
    assert_eq!(actor.active_shot.shoot_mode, Some(ShootMode::Normal));
    // Should have: shoot order + reload order (and possibly transition orders)
    assert!(sm.get_element(seq_id, elem_idx).unwrap().orders.len() >= 2);
}

#[test]
#[should_panic(expected = "bow shot translation lost sequence element")]
fn begin_bow_shot_rejects_a_missing_sequence_element_as_corrupt_state() {
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let mut sequences = SequenceManager::new();

    let _ = begin_bow_shot(
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
fn todo_shot_retranslation_clears_only_its_own_execution_latch() {
    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let mut sm = SequenceManager::new();
    let seq_id = sm.launch_element(build_shoot_bow_element(owner, target_id));
    let elem_idx = 0;
    assert_eq!(
        sm.get_element(seq_id, elem_idx).unwrap().state,
        crate::sequence::SequenceState::Todo,
        "cross-postponed elements are refreshed to Todo before dispatch"
    );
    entities
        .get_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_shot = ActiveShot {
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(std::num::NonZeroU32::new(77).unwrap()),
        released: true,
        shoot_mode: Some(ShootMode::Normal),
    };

    clear_matching_retranslated_shot(&mut entities, owner, SequenceId(999), elem_idx);
    assert!(
        entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot
            .is_active(),
        "an unrelated sequence must retain its active shot"
    );

    clear_matching_retranslated_shot(&mut entities, owner, seq_id, elem_idx);
    assert!(
        !entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot
            .is_active(),
        "the resumed element must release its stale execution latch before translation"
    );

    let mut next_order_id = 100;
    assert_eq!(
        begin_bow_shot(
            &mut entities,
            &mut sm,
            owner,
            target_id,
            seq_id,
            elem_idx,
            false,
            1,
            Some(ShootMode::Normal),
            &mut next_order_id,
        ),
        BeginShotResult::Started,
        "the resumed Original element must translate as a fresh execution"
    );
}

#[test]
fn tick_bow_shots_detaches_when_sequence_has_advanced_past_bow_orders() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    let result = begin_bow_shot(
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

    let events = tick_bow_shots(sim, &mut entities, &mut sm);

    assert!(events.fired.is_empty());
    assert!(events.completed.is_empty());
    assert!(
        !entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot
            .is_active(),
        "C++ shoot-list ownership ends once the sequence has no bow orders left"
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
    let first_seq = sm.launch_element(build_shoot_bow_element(first, target));
    sm.element_in_progress(first_seq, 0);
    let other_seq = sm.launch_element(build_shoot_bow_element(other, target));
    sm.element_in_progress(other_seq, 0);
    let mut next_order_id = 1;
    assert_eq!(
        begin_bow_shot(
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
        begin_bow_shot(
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
    let mut replacement = entities
        .get(other)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_shot;
    replacement.released = true;
    let selected_order = sm
        .current_order_for_actor(first)
        .expect("first bow order selected")
        .2
        .order_id;

    CROSS_ACTOR_SHOT_REPLACEMENT.set(Some((other, replacement)));

    tick_bow_shot_for_owner(
        &sim_context,
        &mut entities,
        &mut sm,
        first,
        selected_order,
        false,
    );

    assert_eq!(
        entities
            .get(other)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot,
        replacement,
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
    let seq = sm.launch_element(build_shoot_bow_element(shooter, target));
    sm.element_in_progress(seq, 0);
    let mut next_order_id = 1;
    assert_eq!(
        begin_bow_shot(
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
    let order = sm.current_order_for_actor(shooter).unwrap().2.clone();
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

    let events =
        tick_bow_shot_for_owner(&sim, &mut entities, &mut sm, shooter, order.order_id, true);

    assert!(events.fired.is_empty());
    assert!(events.completed.is_empty());
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
        sm.current_order_for_actor(shooter).unwrap().2.order_id,
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
    let result = begin_bow_shot(
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
    sm.get_element_mut(seq_id, elem_idx)
        .unwrap()
        .orders
        .push_front(Order::new(
            OrderType::TransitionWaitingUprightBoredWaitingUpright,
            0.0,
            0.0,
            crate::order::alloc_order_id(&mut next_order_id),
        ));

    let events = tick_bow_shots(sim, &mut entities, &mut sm);

    assert!(events.fired.is_empty());
    assert!(events.completed.is_empty());
    assert!(
        entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot
            .is_active(),
        "pre-shoot setup orders should not cancel the pending bow shot"
    );
}

#[test]
fn tick_bow_shots_detaches_before_trailing_non_bow_order() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
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
    let result = begin_bow_shot(
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

    let mut fired = Vec::new();
    let mut completed = Vec::new();
    for _ in 0..64 {
        let events = tick_bow_shots(sim, &mut entities, &mut sm);
        fired.extend(events.fired);
        completed.extend(events.completed);
        if !entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot
            .is_active()
        {
            break;
        }
    }

    assert_eq!(fired.len(), 1);
    assert!(completed.is_empty());
    assert!(
        !entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot
            .is_active(),
        "active bow-shot driver should detach after the final bow order"
    );
    assert_eq!(
        sm.get_element(seq_id, elem_idx)
            .unwrap()
            .current_order()
            .unwrap()
            .order_type,
        OrderType::TransitionWaitingUprightBoredWaitingUpright
    );
}

#[test]
#[should_panic(expected = "active bow shot missing resolved shoot mode")]
fn tick_bow_shots_panics_on_missing_resolved_shoot_mode() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
    let result = begin_bow_shot(
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
    shooter.actor_data_mut().unwrap().active_shot.shoot_mode = None;

    let _ = tick_bow_shots(sim, &mut entities, &mut sm);
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
    set_test_action_state_after_transition(&mut sm, seq_id, elem_idx, ActionState::AimingWithBow);

    let result = begin_bow_shot(
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
    assert_eq!(actor.active_shot.shoot_mode, Some(ShootMode::Long));
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
    set_test_action_state_after_transition(&mut sm, seq_id, elem_idx, ActionState::AimingWithBow);

    let result = begin_bow_shot(
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
        "C++ uses ActionStateAfterTransition, so a first long shot after equip/load still raises the bow before shooting"
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
    let result = begin_bow_shot(
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

    let result = begin_bow_shot(
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
    let result = begin_bow_shot(
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
    assert_eq!(
        entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot
            .target,
        Some(target_id)
    );
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

    let result = begin_bow_shot(
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
    target.element_data_mut().set_position(WorldPoint3D {
        x: 50.0,
        y: 120.0,
        z: 100.0,
    });
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

    let result = begin_bow_shot(
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
fn shoot_initialization_samples_fx_target_cxx_ground_y_once() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut target = make_arrow_target(50.0, 120.0);
    target.element_data_mut().set_position(WorldPoint3D {
        x: 50.0,
        y: 120.0,
        z: 100.0,
    });
    let mut entities = entity_table(vec![Some(make_pc(0.0, 100.0)), Some(target)]);
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

    let result = begin_bow_shot(
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

    tick_bow_shots(sim, &mut entities, &mut sm);

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
        .set_position(WorldPoint3D {
            x: -100.0,
            y: -100.0,
            z: 0.0,
        });
    tick_bow_shots(sim, &mut entities, &mut sm);
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
    target.element_data_mut().set_position(WorldPoint3D {
        x: 50.0,
        y: 120.0,
        z: 100.0,
    });
    let mut shooter = make_soldier(0.0, 0.0);
    shooter.element_data_mut().posture = Posture::LeaningOut;
    shooter.actor_data_mut().unwrap().action_state = ActionState::AimingWithBowDown;
    shooter.element_data_mut().set_direction_instantly(14);
    bind_test_bow_release_rows(&mut shooter, OrderType::ShootingWithBowLeaningOut);

    let shooter_id = EntityId::Soldier(crate::entity_id::SoldierId(0));
    let target_id = EntityId::Pc(crate::entity_id::PcId(1));
    let mut entities = entity_table(vec![Some(shooter), Some(target)]);
    let (mut sm, seq_id, elem_idx) = launch_test_shoot_element(shooter_id, target_id);
    assert_eq!(
        begin_bow_shot(
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

    let events = tick_bow_shots(&sim, &mut entities, &mut sm);
    assert!(events.fired.is_empty());

    let shooter = entities.get(shooter_id).unwrap();
    let expected_goal = crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 20.0);
    assert_ne!(
        expected_goal,
        crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 120.0),
        "the fixture must distinguish PositionMap from PositionGround"
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

    begin_bow_shot(
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
    let mut fired = Vec::new();
    let mut completed = Vec::new();
    for _ in 0..24 {
        let events = tick_bow_shots(sim, &mut entities, &mut sm);
        fired.extend(events.fired);
        completed.extend(events.completed);
        if !fired.is_empty() {
            break;
        }
    }
    assert_eq!(fired.len(), 1, "expected one fired shot");
    assert!(
        completed.is_empty(),
        "release should not terminate the sequence before the visual orders finish"
    );
    let r = &fired[0];
    assert_eq!(r.shooter, EntityId::Pc(crate::entity_id::PcId(0)));
    assert_eq!(r.target, target_id);
    assert_eq!(r.target_pos.x, 50.0);

    // Shooter should now be in AimingWithBow (sustained aim).
    let actor = entities
        .get_at_index(0)
        .map(|(_, entity)| entity)
        .unwrap()
        .actor_data()
        .unwrap();
    assert_eq!(actor.action_state, ActionState::AimingWithBow);
    assert!(actor.active_shot.is_active());
    assert!(actor.active_shot.released);
}

#[test]
fn compute_initial_throw_velocity_flat_shot() {
    let to_target = WorldVec3D {
        x: 100.0,
        y: 0.0,
        z: 0.0,
    };
    // Flat shot: flight_time = (0.003 * 100) + 1 = 1
    let vel = compute_initial_throw_velocity(to_target, 0.001, MASS_ARROW_FLAT, 1, None);
    // With flight_time == 1: velocity = 0.5 * to_target
    assert!((vel.x - 50.0).abs() < 0.01);
}

#[test]
fn compute_initial_throw_velocity_high_shot() {
    let to_target = WorldVec3D {
        x: 100.0,
        y: 0.0,
        z: 0.0,
    };
    let apex = 10.0; // distance / 10
    let vel = compute_initial_throw_velocity(to_target, apex, MASS_ARROW_HIGH, 0, None);
    // Should have a positive Z component (upward arc).
    assert!(vel.z > 0.0, "high shot should arc upward, got z={}", vel.z);
    // X should be positive (toward target).
    assert!(vel.x > 0.0);
}

#[test]
fn compute_trajectory_produces_arc() {
    let start = WorldPoint3D {
        x: 0.0,
        y: 0.0,
        z: 40.0,
    };
    let vel = compute_initial_throw_velocity(
        WorldVec3D {
            x: 100.0,
            y: 0.0,
            z: -10.0,
        },
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
            position: WorldPoint3D {
                x: 25.0,
                y: 0.0,
                z: 45.0,
            },
            time: 4,
        },
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 40.0,
            },
            time: 4,
        },
    ];
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
        target_pos: MapPoint { x: 50.0, y: 0.0 },
        trajectory: traj,
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
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
        }
        _ => panic!("expected ElementProjectile"),
    }
}

#[test]
fn spawn_arrow_stores_shooter_map_position_as_trajectory_origin() {
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 100.0,
            y: 40.0,
            z: 40.0,
        },
        trajectory_origin: MapPoint { x: 100.0, y: 0.0 },
        target: EntityId::Pc(crate::entity_id::PcId(1)),
        target_pos: MapPoint { x: 50.0, y: 0.0 },
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 40.0,
                z: 40.0,
            },
            time: 2,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });

    let Entity::Projectile(p) = arrow else {
        panic!("spawn_arrow should create projectile");
    };
    assert_eq!(p.projectile.start_of_trajectory_x, 100.0);
    assert_eq!(p.projectile.start_of_trajectory_y, 0.0);
}

#[test]
fn tick_arrows_follows_trajectory_and_hits() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // Place a soldier at (50, 0) (belt lives at Z=25, the
    // default belt elevation for an upright human).  The
    // trajectory arcs from the bow height down to belt height at
    // the soldier's XY — the per-segment 3D hit check picks the
    // soldier up on the final waypoint.
    let traj = vec![
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 20.0,
                y: 0.0,
                z: 35.0,
            },
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 40.0,
                y: 0.0,
                z: 30.0,
            },
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 25.0,
            },
            time: 2,
        },
    ];
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_soldier(50.0, 0.0)),
        Some(spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: traj,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        })),
    ]);

    let mut hit = None;
    for _ in 0..20 {
        let results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );
        for r in &results {
            if r.hit_target.is_some() {
                hit = r.hit_target;
                assert_eq!(r.damage, 30);
                break;
            }
        }
        if hit.is_some() {
            break;
        }
    }
    assert_eq!(
        hit,
        Some(EntityId::Soldier(crate::entity_id::SoldierId(1))),
        "arrow should reach target"
    );
}

#[test]
fn tick_arrows_human_hit_reports_old_position_and_victim_impact_anchor() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let traj = vec![
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 20.0,
                y: 0.0,
                z: 35.0,
            },
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 40.0,
                y: 0.0,
                z: 30.0,
            },
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 25.0,
            },
            time: 2,
        },
    ];
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
        target_pos: MapPoint { x: 50.0, y: 0.0 },
        trajectory: traj,
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_soldier(50.0, 0.0)),
        Some(arrow),
    ]);

    let mut hit = None;
    for _ in 0..20 {
        let results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );
        hit = results.into_iter().find(|result| {
            result.hit_target == Some(EntityId::Soldier(crate::entity_id::SoldierId(1)))
        });
        if hit.is_some() {
            break;
        }
    }

    let hit = hit.expect("arrow should reach human target");
    assert_eq!(hit.impact_pos, MapPoint { x: 50.0, y: 0.0 });
    let old_pos = hit
        .human_hit_old_position
        .expect("human hit should carry previous projectile position");
    assert!(old_pos.x < hit.impact_pos.x);
    assert!((old_pos.y - 0.0).abs() < 0.01);
    assert!(old_pos.z >= 25.0);
}

#[test]
fn tick_arrow_resolves_spawn_primed_segment_only_for_requested_arrow() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Target(crate::entity_id::TargetId(1)),
        target_pos: MapPoint { x: 50.0, y: 0.0 },
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 0.0,
            },
            time: 1,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: -0.25,
        },
    });
    let mut other_arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 1000.0,
            y: 0.0,
            z: 40.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Target(crate::entity_id::TargetId(1)),
        target_pos: MapPoint { x: 50.0, y: 0.0 },
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 1010.0,
                y: 0.0,
                z: 40.0,
            },
            time: 1,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    let Entity::Projectile(p) = &mut other_arrow else {
        panic!("spawn_arrow should create projectile");
    };
    p.projectile.launch_segment_start = Some(WorldPoint3D {
        x: 1000.0,
        y: 0.0,
        z: 40.0,
    });

    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_arrow_target(50.0, 0.0)),
        Some(arrow),
        Some(other_arrow),
    ]);

    let results = tick_arrow(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
        None,
        EntityId::Projectile(crate::entity_id::ProjectileId(2)),
    );

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].arrow,
        EntityId::Projectile(crate::entity_id::ProjectileId(2))
    );
    assert_eq!(
        results[0].fx_target_hit,
        Some((
            EntityId::Target(crate::entity_id::TargetId(1)),
            Command::ActivateArrow
        ))
    );

    let Some(Entity::Projectile(p)) = entities.get_at_index(3).map(|(_, entity)| entity) else {
        panic!("other arrow should remain present");
    };
    assert!(
        p.projectile.launch_segment_start.is_some(),
        "filtered tick must not consume another projectile's primed segment"
    );
}

#[test]
fn tick_arrows_prefilters_friendly_candidate_before_selecting_victim() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Soldier(crate::entity_id::SoldierId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 25.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Soldier(crate::entity_id::SoldierId(2)),
        target_pos: MapPoint { x: 100.0, y: 0.0 },
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 100.0,
                y: 0.0,
                z: 25.0,
            },
            time: 1,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
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

    let results = tick_arrows(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );
    assert!(
        results
            .iter()
            .all(|r| r.hit_target != Some(EntityId::Soldier(crate::entity_id::SoldierId(1)))),
        "same-camp soldier must be filtered before hit selection"
    );
    assert!(
        results
            .iter()
            .any(|r| r.hit_target == Some(EntityId::Soldier(crate::entity_id::SoldierId(2)))),
        "arrow should continue to the valid victim behind the filtered candidate"
    );
}

#[test]
fn enabled_diplomacy_protects_neutral_soldiers_from_pc_arrows() {
    let sim = &crate::sim_rng::test_context();
    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(2));
    let victim_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 25.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: victim_id,
        target_pos: MapPoint { x: 100.0, y: 0.0 },
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 100.0,
                y: 0.0,
                z: 25.0,
            },
            time: 1,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    let mut entities = entity_table(vec![
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

    let results = tick_arrow_in_actor_order_with_diplomacy(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
        None,
        arrow_id,
        &[EntityId::Pc(crate::entity_id::PcId(0)), victim_id],
        &diplomacy,
    );

    assert!(
        results
            .iter()
            .all(|result| result.hit_target != Some(victim_id)),
        "neutral actors must be filtered before projectile hit selection"
    );
}

#[test]
fn tick_arrows_selects_last_eligible_human_in_actor_order() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 25.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
        target_pos: MapPoint { x: 100.0, y: 0.0 },
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 100.0,
                y: 0.0,
                z: 25.0,
            },
            time: 1,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        // The earlier actor is farther downrange than the later actor so
        // this proves registry order, not nearest/farthest geometry.
        Some(make_soldier(80.0, 0.0)),
        Some(make_soldier(60.0, 0.0)),
        Some(arrow),
    ]);

    let results = tick_arrows(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );

    assert!(
        results.iter().any(|result| {
            result.hit_target == Some(EntityId::Soldier(crate::entity_id::SoldierId(2)))
        }),
        "Original retains the last eligible human visited by marrayActors"
    );
    assert!(
        results.iter().all(|result| {
            result.hit_target != Some(EntityId::Soldier(crate::entity_id::SoldierId(1)))
        }),
        "an earlier eligible human must be replaced by a later one"
    );
}

#[test]
fn ordered_projectile_scan_uses_actor_registry_not_entity_slots() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(3));
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 25.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
        target_pos: MapPoint { x: 100.0, y: 0.0 },
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 100.0,
                y: 0.0,
                z: 25.0,
            },
            time: 1,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
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
    world.install_original_creation_orders(
        std::collections::BTreeMap::from([
            (pc, 100),
            (soldier_2, 101),
            (soldier_1, 102),
            (arrow_id, 103),
        ]),
        104,
    );
    let actor_order = world.actor_registry_order();
    assert_eq!(actor_order, [pc, soldier_2, soldier_1]);

    let results = tick_arrow_in_actor_order(
        sim,
        &mut world.entities,
        crate::sight_obstacle::ObstacleList::empty(),
        None,
        arrow_id,
        &actor_order,
    );

    assert!(results.iter().any(|result| {
        result.hit_target == Some(EntityId::Soldier(crate::entity_id::SoldierId(1)))
    }));
    assert!(results.iter().all(|result| {
        result.hit_target != Some(EntityId::Soldier(crate::entity_id::SoldierId(2)))
    }));
}

#[test]
fn ordered_projectile_scan_uses_first_shield_in_actor_registry_order() {
    crate::sim_rng::with_seed(1, |sim| {
        use crate::element::ActionState;

        let make_holder = || {
            let mut holder = make_soldier(50.0, 0.0);
            let actor = holder.actor_data_mut().unwrap();
            actor.action_state = ActionState::HoldingShield;
            actor.shield_obstacle = Some(compute_shield_obstacle(
                MapPoint { x: 50.0, y: 0.0 },
                0.0,
                4,
                &shield_params_for_soldier(20, 40),
            ));
            holder.element_data_mut().set_direction_instantly(4);
            holder
        };
        let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(3));
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D::new(100.0, 0.0, 40.0),
            trajectory_origin: MapPoint { x: 100.0, y: 0.0 },
            target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D::new(50.0, 0.0, 40.0),
                time: 2,
            }],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D::new(-1.0, 0.0, 0.0),
        });
        let mut entities = entity_table(vec![
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

        let mut shield_hit = None;
        for _ in 0..10 {
            for result in tick_arrow_in_actor_order(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
                None,
                arrow_id,
                &actor_order,
            ) {
                shield_hit = shield_hit.or(result.shield_hit);
            }
            if shield_hit.is_some() {
                break;
            }
        }

        assert_eq!(shield_hit, Some(soldier_2));
    });
}

#[test]
fn tick_arrows_leaning_eye_hit_can_be_replaced_by_later_eligible_human() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let [lean_x, lean_y] = crate::position_interface::sector_to_vector_iso(0);
    let arrow_y = lean_y * 40.0;
    let arrow_old = WorldPoint3D {
        x: lean_x * 40.0,
        y: arrow_y,
        z: 45.0,
    };
    let arrow_new = WorldPoint3D {
        x: 120.0 + lean_x * 40.0,
        y: arrow_y,
        z: 45.0,
    };
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: arrow_old,
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
        target_pos: MapPoint { x: 120.0, y: 0.0 },
        trajectory: vec![TrajectoryPoint {
            position: arrow_new,
            time: 1,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    let mut earlier = make_soldier(80.0, 0.0);
    earlier.element_data_mut().posture = Posture::LeaningOut;
    earlier.element_data_mut().set_direction_instantly(0);
    // The flight line is at the leaning eye height (z=45); the belt is
    // at z=25, outside HIT_DISTANCE, so only the eye retry can hit.
    let mut later = make_soldier(60.0, 0.0);
    later.element_data_mut().posture = Posture::LeaningOut;
    later.element_data_mut().set_direction_instantly(0);
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, -200.0)),
        Some(earlier),
        Some(later),
        Some(arrow),
    ]);

    let results = tick_arrows(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );

    assert!(results.iter().any(|result| {
        result.hit_target == Some(EntityId::Soldier(crate::entity_id::SoldierId(2)))
    }));
    assert!(results.iter().all(|result| {
        result.hit_target != Some(EntityId::Soldier(crate::entity_id::SoldierId(1)))
    }));
}

#[test]
fn tick_arrows_stationary_projectile_does_not_hit_human() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut arrow_element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        ..ElementData::default()
    };
    arrow_element.set_position_map(MapPoint { x: 50.0, y: -25.0 });
    arrow_element.set_position(WorldPoint3D {
        x: 50.0,
        y: 0.0,
        z: 25.0,
    });
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
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
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

    let results = tick_arrows(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );
    assert!(
        results.iter().all(|r| r.hit_target.is_none()),
        "C++ FindHumanVictim returns no victim when projectile is not moving"
    );
}

#[test]
fn tick_arrows_without_shooter_does_not_hit_human() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Pc(crate::entity_id::PcId(1)),
        target_pos: MapPoint { x: 50.0, y: 0.0 },
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 25.0,
            },
            time: 1,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    if let Entity::Projectile(proj) = &mut arrow {
        proj.projectile.shooter = None;
    }
    let mut entities = entity_table(vec![None, Some(make_soldier(50.0, 0.0)), Some(arrow)]);

    let results = tick_arrows(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );
    assert!(
        results.iter().all(|r| r.hit_target.is_none()),
        "C++ FindHumanVictim returns no victim when projectile has no shooter"
    );
}

/// An arrow whose shooter dies mid-flight keeps hunting victims.
///
/// `RHElementProjectile::FindHumanVictim` holds the shooter through the
/// raw `mpShooter` pointer stored at construction
/// (`original-code/RHelementprojectile.cpp:103`), aborts only on
/// `mpShooter == NULL` (`:1801`), and otherwise merely asks him for
/// `IsSoldier()` / `GetCamp()` / `IsPC()` (`:1833-1857`). A corpse still
/// answers all three, so the shot stays lethal. Rust used to resolve the
/// shooter inside the *hittable-victim* snapshot, which drops dead,
/// lying, netted and tied humans, and then skipped the whole scan.
#[test]
fn tick_arrows_dead_shooter_still_hits_human() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::Posture;

    let mut shooter = make_pc(0.0, 0.0);
    shooter.set_posture(Posture::Dead);

    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 25.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
        target_pos: MapPoint { x: 50.0, y: 0.0 },
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 25.0,
            },
            time: 2,
        }],
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });

    let mut entities = entity_table(vec![
        Some(shooter),
        Some(make_soldier(50.0, 0.0)),
        Some(arrow),
    ]);

    let mut any_hit = None;
    for _ in 0..10 {
        for r in tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        ) {
            if r.hit_target.is_some() {
                any_hit = r.hit_target;
            }
        }
    }
    assert_eq!(
        any_hit,
        Some(EntityId::Soldier(crate::entity_id::SoldierId(1))),
        "a dead shooter's arrow must still resolve its human victim"
    );
}

/// Apple projectile flying through an APPLE-filtered FX target
/// yields a `Command::ActivateApple` activation on tick.
#[test]
fn tick_arrows_apple_projectile_activates_apple_target() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

    let target_pos = MapPoint { x: 50.0, y: 0.0 };
    let mut target_element = ElementData {
        kind: ElementKind::Target,
        active: true,
        ..ElementData::default()
    };
    target_element.set_position_map(target_pos);
    // `compute_target_center` reads the 3D position; real loaded
    // targets set both, but `ElementData::default()` leaves position
    // at origin so we mirror position_map.
    target_element.set_position(WorldPoint3D {
        x: target_pos.x,
        y: target_pos.y,
        z: 0.0,
    });
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
            position: WorldPoint3D {
                x: 25.0,
                y: 0.0,
                z: 10.0,
            },
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 0.0,
            },
            time: 2,
        },
    ];
    let mut apple_element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        ..ElementData::default()
    };
    apple_element.set_position_map(MapPoint { x: 0.0, y: 0.0 });
    apple_element.set_position(WorldPoint3D {
        x: 0.0,
        y: 0.0,
        z: 20.0,
    });
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

    let mut entities = entity_table(vec![Some(target), Some(apple), Some(make_pc(0.0, 0.0))]);

    let mut activation = None;
    let mut impact = None;
    for _ in 0..20 {
        for r in tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        ) {
            if let Some(hit) = r.fx_target_hit {
                activation = Some(hit);
                impact = Some((r.impact_fx, r.impact_pos));
                break;
            }
        }
        if activation.is_some() {
            break;
        }
    }
    assert_eq!(
        activation,
        Some((
            EntityId::Target(crate::entity_id::TargetId(0)),
            Command::ActivateApple
        )),
        "apple projectile should activate APPLE-filter target with ActivateApple"
    );
    assert_eq!(impact, Some((Some(509), target_pos)));
}

/// C++ `FindTargetVictim` uses current-position range gating for
/// FX targets: a target just beyond the old->new segment endpoint
/// can still be hit when it is within one movement length of the
/// arrow's current position.  This catches short final segments
/// that would otherwise land without activating scripted targets.
#[test]
fn tick_arrows_arrow_target_uses_current_position_range_gate() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

    let mut target_element = ElementData {
        kind: ElementKind::Target,
        active: true,
        ..ElementData::default()
    };
    target_element.set_position_map(MapPoint { x: 40.0, y: 0.0 });
    target_element.set_position(WorldPoint3D {
        x: 40.0,
        y: 0.0,
        z: 0.0,
    });
    let target = Entity::Target(ElementTarget {
        element: target_element,
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::ARROW,
            ..TargetData::default()
        },
    });

    let mut arrow_element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        ..ElementData::default()
    };
    arrow_element.set_position_map(MapPoint { x: 0.0, y: 0.0 });
    arrow_element.set_position(WorldPoint3D {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
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
                position: WorldPoint3D {
                    x: 25.0,
                    y: 0.0,
                    z: 0.0,
                },
                time: 1,
            }],
            damage: 30,
            ..ProjectileData::default()
        },
    });

    let mut entities = entity_table(vec![Some(target), Some(arrow), Some(make_pc(0.0, 0.0))]);
    let results = tick_arrows(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );

    assert!(
        results.iter().any(|r| {
            r.fx_target_hit
                == Some((
                    EntityId::Target(crate::entity_id::TargetId(0)),
                    Command::ActivateArrow,
                ))
                && r.despawn
        }),
        "arrow should activate target using C++ current-position range gate"
    );
}

/// C++ stationary FX-target checks still require
/// `vtRange.Norm() <= range`, so a projectile with zero movement
/// cannot activate a nearby target unless it is exactly centered on
/// it. Rust used to fall back to `HIT_DISTANCE`, which could fire
/// scripted targets from a stopped projectile.
#[test]
fn tick_arrows_stationary_projectile_does_not_radius_hit_fx_target() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

    let mut target_element = ElementData {
        kind: ElementKind::Target,
        active: true,
        ..ElementData::default()
    };
    target_element.set_position_map(MapPoint { x: 10.0, y: 0.0 });
    target_element.set_position(WorldPoint3D {
        x: 10.0,
        y: 0.0,
        z: 0.0,
    });
    let target = Entity::Target(ElementTarget {
        element: target_element,
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::ARROW,
            ..TargetData::default()
        },
    });

    let mut arrow_element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        ..ElementData::default()
    };
    arrow_element.set_position_map(MapPoint { x: 0.0, y: 0.0 });
    arrow_element.set_position(WorldPoint3D {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
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
                position: WorldPoint3D {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                time: 1,
            }],
            damage: 30,
            ..ProjectileData::default()
        },
    });

    let mut entities = entity_table(vec![Some(target), Some(arrow), Some(make_pc(0.0, 0.0))]);
    let results = tick_arrows(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );

    assert!(
        results.iter().all(|r| r.fx_target_hit.is_none()),
        "stationary projectile must not activate nearby FX target by radius"
    );
}

#[test]
fn tick_arrows_has_no_artificial_lifetime_timeout() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let trajectory = (1..=320)
        .map(|i| TrajectoryPoint {
            position: WorldPoint3D {
                x: i as f32 * 10.0,
                y: 0.0,
                z: 40.0,
            },
            time: 1,
        })
        .collect();
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
        target_pos: MapPoint { x: 3200.0, y: 0.0 },
        trajectory,
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    let mut entities = entity_table(vec![Some(make_pc(0.0, -100.0)), Some(arrow)]);

    let mut despawn_frame = None;
    for frame in 0..260 {
        let results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );
        if results.iter().any(|r| r.despawn) {
            despawn_frame = Some(frame);
            break;
        }
    }

    assert_eq!(
        despawn_frame, None,
        "C++ projectile lifetime is trajectory-driven, not capped at 250 frames"
    );
    match entities.get_at_index(1).map(|(_, entity)| entity).unwrap() {
        Entity::Projectile(p) => assert!(p.projectile.flying),
        _ => panic!("expected projectile"),
    }
}

/// Apple projectile flying through a target that does NOT have the
/// APPLE filter leaves `fx_target_hit` unset — no activation is
/// launched.
#[test]
fn tick_arrows_apple_projectile_ignores_non_apple_target() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

    let mut target_element = ElementData {
        kind: ElementKind::Target,
        active: true,
        ..ElementData::default()
    };
    target_element.set_position_map(MapPoint { x: 50.0, y: 0.0 });
    target_element.set_position(WorldPoint3D {
        x: 50.0,
        y: 0.0,
        z: 0.0,
    });
    let target = Entity::Target(ElementTarget {
        element: target_element,
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::ARROW,
            ..TargetData::default()
        },
    });

    let trajectory = vec![TrajectoryPoint {
        position: WorldPoint3D {
            x: 50.0,
            y: 0.0,
            z: 0.0,
        },
        time: 2,
    }];
    let apple = Entity::Projectile(ElementProjectile {
        element: ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        },
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

    let mut entities = entity_table(vec![Some(target), Some(apple)]);
    let results = tick_arrows(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );
    assert!(
        results.is_empty(),
        "C++ FindTargetVictim ignores nonmatching target filters before HitTarget can burst"
    );
}

/// Apple impact on an FX target sets the burst animation + decay
/// row and leaves grounded animation/removal to the derived owner path.
#[test]
fn tick_arrows_apple_bursts_then_leaves_grounded_tail_to_virtual_owner() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

    let mut target_element = ElementData {
        kind: ElementKind::Target,
        active: true,
        ..ElementData::default()
    };
    target_element.set_position_map(MapPoint { x: 10.0, y: 0.0 });
    let target = Entity::Target(ElementTarget {
        element: target_element,
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::APPLE,
            ..TargetData::default()
        },
    });
    let apple = Entity::Projectile(ElementProjectile {
        element: ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        },
        object: ObjectData {
            object_type: ObjectType::Apple,
            animation: Animation::ObjectFlying,
            ..ObjectData::default()
        },
        projectile: ProjectileData {
            shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
            flying: true,
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0,
                },
                time: 1,
            }],
            ..ProjectileData::default()
        },
    });
    let mut entities = entity_table(vec![Some(target), Some(apple), Some(make_pc(0.0, 0.0))]);

    // First tick: apple reaches target, bursts.
    let impact_results = tick_arrows(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );
    assert!(
        impact_results
            .iter()
            .any(|r| r.fx_target_hit.is_some() && !r.despawn),
        "apple must NOT despawn on impact frame — it bursts first"
    );
    let proj_after = entities.get_at_index(1).map(|(_, entity)| entity).unwrap();
    match proj_after {
        Entity::Projectile(p) => {
            assert!(!p.projectile.flying);
            assert_eq!(p.object.animation, Animation::ObjectBursting);
        }
        _ => panic!("expected apple projectile"),
    }

    let grounded_base_results = tick_arrows(
        sim,
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );
    assert!(
        grounded_base_results.is_empty(),
        "Projectile::Hourglass must not duplicate the derived landed animation/removal"
    );
}

/// Apple impact yields impact FX 509; stone yields 508; arrow hit
/// without shield yields no impact FX (silent).
#[test]
fn tick_arrows_impact_fx_per_projectile_type() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    fn spawn_projectile_at_impact(obj: ObjectType) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        };
        element.set_position_map(MapPoint { x: 0.0, y: 0.0 });
        element.set_position(WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
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
        let mut entities = entity_table(vec![
            Some(spawn_projectile_at_impact(obj)),
            Some(make_pc(100.0, 0.0)),
        ]);
        let results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );
        results.into_iter().find_map(|r| r.impact_fx)
    };
    assert_eq!(fx_for(ObjectType::Apple), Some(509));
    assert_eq!(fx_for(ObjectType::Stone), Some(508));
    assert_eq!(fx_for(ObjectType::Arrow), None);
}

/// `spawn_apple` builds a flying apple projectile with Apple
/// object_type and a ballistic trajectory.
#[test]
fn spawn_apple_creates_flying_apple_projectile() {
    let start = WorldPoint3D {
        x: 0.0,
        y: 0.0,
        z: 40.0,
    };
    let end = WorldPoint3D {
        x: 100.0,
        y: 0.0,
        z: 20.0,
    };
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
fn bow_point_order_types_are_non_anonymous_cxx_compute_bow_point_ids() {
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

#[test]
fn unequip_bow_sets_waiting_on_animation_start() {
    let mut pc = make_pc(0.0, 0.0);
    pc.actor_data_mut().unwrap().action_state = ActionState::AimingWithBow;

    apply_bow_transition_state_side_effect(
        &mut pc,
        OrderType::TransitionUnequipBow,
        SpriteMotionState::Start,
    );

    assert_eq!(
        pc.actor_data().unwrap().action_state,
        ActionState::Waiting,
        "C++ TransitionUnequipBow sets Waiting on RHMOTION_START"
    );
}

#[test]
fn equip_bow_sets_aiming_on_animation_start() {
    let mut pc = make_pc(0.0, 0.0);
    pc.actor_data_mut().unwrap().action_state = ActionState::Waiting;

    apply_bow_transition_state_side_effect(
        &mut pc,
        OrderType::TransitionEquipBow,
        SpriteMotionState::Start,
    );

    assert_eq!(
        pc.actor_data().unwrap().action_state,
        ActionState::AimingWithBow,
        "C++ TransitionEquipBow sets AimingWithBow on RHMOTION_START"
    );
}

fn tick_active_pc_equip_start(script_driven: bool) -> BowTickEvents {
    let sim = crate::sim_rng::test_context();
    let shooter = EntityId::Pc(crate::entity_id::PcId(0));
    let target = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let mut pc = make_pc(0.0, 0.0);
    bind_test_bow_release_rows(&mut pc, OrderType::TransitionEquipBow);
    let mut entities = entity_table(vec![Some(pc), Some(make_soldier(50.0, 0.0))]);
    let mut sm = SequenceManager::new();
    let mut element = build_shoot_bow_element(shooter, target);
    element.script_driven = script_driven;
    let mut next_order_id = 1;
    let order_id = crate::order::alloc_order_id(&mut next_order_id);
    element.orders.push_back(Order::new(
        OrderType::TransitionEquipBow,
        0.0,
        0.0,
        order_id,
    ));
    let sequence_id = sm.launch_element(element);
    sm.element_in_progress(sequence_id, 0);
    entities
        .get_mut(shooter)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_shot = ActiveShot {
        sequence_id: Some(sequence_id),
        element_index: 0,
        target: Some(target),
        order_id: Some(order_id),
        released: false,
        shoot_mode: Some(ShootMode::Normal),
    };

    let events = tick_bow_shots(&sim, &mut entities, &mut sm);
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
    events
}

#[test]
fn active_shot_pc_equip_start_requests_bow_action_restitution() {
    let events = tick_active_pc_equip_start(false);

    assert_eq!(
        events.pc_equip_actions,
        [EntityId::Pc(crate::entity_id::PcId(0))]
    );
}

#[test]
fn active_shot_script_pc_equip_start_suppresses_bow_action_restitution() {
    let events = tick_active_pc_equip_start(true);

    assert!(
        events.pc_equip_actions.is_empty(),
        "script-driven equip transitions must preserve the PC's toolbar action"
    );
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
fn unload_bow_sets_waiting_on_animation_start() {
    let mut pc = make_pc(0.0, 0.0);
    pc.actor_data_mut().unwrap().action_state = ActionState::AimingWithBowDown;

    apply_bow_transition_state_side_effect(
        &mut pc,
        OrderType::TransitionUnloadBow,
        SpriteMotionState::Start,
    );

    assert_eq!(
        pc.actor_data().unwrap().action_state,
        ActionState::Waiting,
        "C++ TransitionUnloadBow sets Waiting on RHMOTION_START"
    );
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
    assert_eq!(soldier.element_data().posture, Posture::LeaningOut);
    assert_eq!(
        soldier.actor_data().unwrap().action_state,
        ActionState::AimingWithBowDown
    );

    apply_bow_transition_state_side_effect(
        &mut soldier,
        OrderType::TransitionRaisingBowLeaningOut,
        SpriteMotionState::Done,
    );
    assert_eq!(soldier.element_data().posture, Posture::Upright);
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
    pc.element_data_mut().posture = Posture::LeaningOut;
    bind_test_bow_release_rows(&mut pc, OrderType::ShootingWithBowLeaningOut);
    let mut target = make_soldier(50.0, 0.0);
    target.element_data_mut().posture = Posture::LeaningOut;
    let mut entities = entity_table(vec![Some(pc), Some(target)]);
    let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let (mut sm, seq_id, elem_idx) =
        launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

    begin_bow_shot(
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

    let mut fired = Vec::new();
    for _ in 0..16 {
        fired.extend(tick_bow_shots(sim, &mut entities, &mut sm).fired);
        if !fired.is_empty() {
            break;
        }
    }
    assert_eq!(fired.len(), 1);
    assert_eq!(
        entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .element_data()
            .posture,
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
    let pos = WorldPoint3D {
        x: 10.0,
        y: 20.0,
        z: 0.0,
    };
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
    let elevated_pos = WorldPoint3D {
        x: 10.0,
        y: 50.0,
        z: 30.0,
    };
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
    // `RHFastFindGrid::IsReachableImpact` does, so the wall has to be
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
        // AddTrajectoryFallIntoHole deliberately needs two points before
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
        WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 25.0,
        },
        WorldVec3D {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        },
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
        WorldVec3D {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        },
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

    make_arrow_falling_down(
        &crate::sim_rng::test_context(),
        &mut arrow,
        false,
        Some(&check),
    );

    assert!(arrow.projectile.falling);
    assert!(arrow.projectile.dive);
    assert!(!arrow.projectile.disappear);

    let dry_zones = crate::water_zones::WaterZones::new();
    let dry_check = TrajectoryObstacleCheck {
        water_zones: Some(&dry_zones),
        ..check
    };
    make_arrow_falling_down(
        &crate::sim_rng::test_context(),
        &mut arrow,
        false,
        Some(&dry_check),
    );
    assert!(
        arrow.projectile.dive,
        "ComputeTrajectory does not clear an earlier mbDive when a ricochet recomputes a dry fall"
    );
}

/// A projectile that passes close to a target on the ground (not
/// airborne) still misses when the target's posture is one of the
/// "untargetable" postures.  Spot-check one of them
/// (`Posture::Lying`) to confirm the filter actually prunes the
/// snapshot.
#[test]
fn tick_arrows_skips_lying_victim() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::Posture;

    let mut soldier = make_soldier(50.0, 0.0);
    soldier.set_posture(Posture::Lying);

    // Arrow trajectory aimed directly at where the belt would be
    // if the soldier were upright — but since it's lying, no hit.
    let trajectory = vec![TrajectoryPoint {
        position: WorldPoint3D {
            x: 50.0,
            y: 0.0,
            z: 25.0,
        },
        time: 2,
    }];
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 25.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Pc(crate::entity_id::PcId(1)),
        target_pos: MapPoint { x: 50.0, y: 0.0 },
        trajectory,
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });

    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(soldier), Some(arrow)]);

    let mut any_hit = None;
    for _ in 0..10 {
        for r in tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        ) {
            if r.hit_target.is_some() {
                any_hit = r.hit_target;
                break;
            }
        }
    }
    assert!(
        any_hit.is_none(),
        "arrow must not hit a lying soldier (posture filter)"
    );
}

/// Arrow that sails past a target in 3D does not hit it even when
/// their 2D projections coincide.  Previously the 2D point check
/// falsely reported a hit on any arrow passing directly over a
/// target; the 3D line-segment check does not.  Regression test
/// for that gap.
#[test]
fn tick_arrows_does_not_hit_when_arcing_overhead() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // Arrow stays well above the soldier's belt (Z=25).
    let trajectory = vec![
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 30.0,
                y: 0.0,
                z: 80.0,
            },
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 60.0,
                y: 0.0,
                z: 78.0,
            },
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 90.0,
                y: 0.0,
                z: 76.0,
            },
            time: 2,
        },
    ];
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 82.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Pc(crate::entity_id::PcId(1)),
        target_pos: MapPoint { x: 90.0, y: 0.0 },
        trajectory,
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });

    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_soldier(50.0, 0.0)),
        Some(arrow),
    ]);

    let mut any_hit = None;
    for _ in 0..20 {
        for r in tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        ) {
            if r.hit_target.is_some() {
                any_hit = r.hit_target;
            }
        }
        if any_hit.is_some() {
            break;
        }
    }
    assert!(
        any_hit.is_none(),
        "arrow arcing 50+ units above a soldier's belt must not register a hit"
    );
}

/// Arrow that shares the soldier's 2D column but passes at belt
/// height hits; trajectory comes down to the belt then continues
/// past.  Complement to [`tick_arrows_does_not_hit_when_arcing_overhead`].
#[test]
fn tick_arrows_hits_through_belt_column() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let trajectory = vec![
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 25.0,
            },
            time: 2,
        },
        TrajectoryPoint {
            position: WorldPoint3D {
                x: 80.0,
                y: 0.0,
                z: 20.0,
            },
            time: 2,
        },
    ];
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 30.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Pc(crate::entity_id::PcId(1)),
        target_pos: MapPoint { x: 80.0, y: 0.0 },
        trajectory,
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    let mut entities = entity_table(vec![
        Some(make_pc(0.0, 0.0)),
        Some(make_soldier(20.0, 0.0)),
        Some(arrow),
    ]);
    let mut hit = None;
    for _ in 0..20 {
        for r in tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        ) {
            if r.hit_target.is_some() {
                hit = r.hit_target;
            }
        }
        if hit.is_some() {
            break;
        }
    }
    assert_eq!(hit, Some(EntityId::Soldier(crate::entity_id::SoldierId(1))));
}

/// Shield intersection flips the projectile into the falling state
/// and emits a `shield_hit` result.  The projectile keeps flying
/// on a new deflected trajectory toward the ground — it must not
/// despawn on the same tick.
#[test]
fn tick_arrows_shield_hit_deflects_and_keeps_flying() {
    crate::sim_rng::with_seed(1, |sim| {
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
            let obs = compute_shield_obstacle(MapPoint { x: 50.0, y: 0.0 }, 0.0, 4, &params);
            actor.shield_obstacle = Some(obs);
        }
        shield_holder.element_data_mut().set_direction_instantly(4);

        // Arrow flying from +X toward the shield holder at Z=40 —
        // mid-shield height for `shield_params_for_soldier(20, 40)`
        // which places the quad between Z=30 and Z=50.  The holder
        // stands at ground Y=0, so the arrow shares that ground Y and
        // clears the quad only on height, which the Z extent decides.
        let trajectory = vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 40.0,
            },
            time: 2,
        }];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 100.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: MapPoint { x: 100.0, y: 0.0 },
            target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });

        let mut entities = entity_table(vec![
            Some(make_pc(100.0, 0.0)),
            Some(shield_holder),
            Some(arrow),
        ]);

        // Advance ticks until the shield_hit fires.
        let mut shield_hit = None;
        let mut despawn_seen = false;
        for _ in 0..10 {
            for r in tick_arrows(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            ) {
                if let Some(holder) = r.shield_hit {
                    shield_hit = Some(holder);
                    despawn_seen = r.despawn;
                }
            }
            if shield_hit.is_some() {
                break;
            }
        }
        assert_eq!(
            shield_hit,
            Some(EntityId::Soldier(crate::entity_id::SoldierId(1))),
            "arrow must report shield hit on the holder"
        );
        assert!(
            !despawn_seen,
            "shield-hit arrow keeps flying (falling) on same tick"
        );

        // The projectile should be flagged as falling, and the hit
        // check must now skip (falling arrows pass through bodies).
        match entities.get_at_index(2).map(|(_, entity)| entity).unwrap() {
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
                    (WorldPoint3D {
                        x: 50.0,
                        y: 40.0,
                        z: 40.0
                    }),
                    "C++ MakeFallingDown advances the falling trajectory immediately"
                );
            }
            _ => panic!("expected projectile"),
        }
    });
}

#[test]
fn projectile_uses_stale_shield_until_explicit_refresh() {
    fn run(
        explicit_refresh: bool,
    ) -> (Option<EntityId>, ([f32; 3], [f32; 3]), ([f32; 3], [f32; 3])) {
        let sim = crate::sim_rng::test_context();
        let mut holder = make_pc(50.0, 0.0);
        holder.element_data_mut().set_direction_instantly(4);
        holder.actor_data_mut().unwrap().action_state = ActionState::HoldingShield;

        // Authoritative geometry deliberately retained from an old
        // position. A projectile tick must not silently move it to the
        // actor's current position/facing.
        let stale = compute_shield_obstacle(
            MapPoint { x: -50.0, y: 100.0 },
            0.0,
            4,
            &shield_params_for_pc(false),
        );
        holder.actor_data_mut().unwrap().shield_obstacle = Some(stale);
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
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 100.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: MapPoint { x: 100.0, y: 0.0 },
            target: EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 40.0,
                },
                time: 2,
            }],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        let mut entities = entity_table(vec![Some(make_pc(100.0, 0.0)), Some(holder), Some(arrow)]);
        let mut shield_hit = None;
        for _ in 0..10 {
            for result in tick_arrows(
                &sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            ) {
                shield_hit = shield_hit.or(result.shield_hit);
            }
        }
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
    // Treating the raw compass vector as UpdateBox's already-normalized
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
                position: WorldPoint3D {
                    x: 25.0,
                    y: 0.0,
                    z: 0.0,
                },
                time: 1,
            },
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 0.0,
                },
                time: 2,
            },
        ];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        let mut projectile = match arrow {
            Entity::Projectile(p) => p,
            _ => panic!("expected arrow projectile"),
        };
        projectile.element.set_direction_instantly(4);
        let impact_position = projectile.element.position();

        make_arrow_falling_down(sim, &mut projectile, false, None);

        assert!(projectile.projectile.falling);
        assert!(projectile.projectile.flying);
        assert_eq!(
            projectile.projectile.falling_direction, 12,
            "armor ricochet reverses the flight sector for the fall"
        );
        assert_ne!(
            projectile.element.position(),
            impact_position,
            "C++ MakeFallingDown calls Hourglass for armor ricochets too"
        );

        // The tumble visual is a presentation pass: it renders on the
        // deferred refresh before the next hourglass, not inside
        // MakeFallingDown itself.
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
    crate::sim_rng::with_seed(1, |sim| {
        // Savegame_linux2/Profile_002/Savegame_017/replay-016, frame 566:
        // the arrow reaches a ground endpoint a fraction below zero.  Its
        // shield-deflection trajectory is empty, but Original's nested
        // Hourglass still executes HitObstacle and publishes the ground snap.
        let endpoint = WorldPoint3D::new(98.988_8, 861.410_2, -0.000_000_953_674_3);
        let Entity::Projectile(mut arrow) = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: endpoint,
            trajectory_origin: endpoint.to_map(),
            target: EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: endpoint.to_map(),
            trajectory: vec![],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D::new(-47.394_653, 46.451_09, -7.129_664),
        }) else {
            panic!("spawn_arrow returned a non-projectile entity");
        };
        arrow.element.set_position(endpoint);
        arrow
            .element
            .set_position_map_preserving_3d(endpoint.to_map());
        arrow.element.set_direction_instantly(10);
        arrow.projectile.trajectory.clear();
        arrow.projectile.trajectory_frame_count = 0;
        arrow.projectile.launch_segment_start = None;
        arrow.projectile.flying = true;

        make_arrow_falling_down(sim, &mut arrow, true, None);

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
    });
}

#[test]
fn ground_crossing_is_attributed_to_first_front_facing_shield() {
    let mut holder = make_soldier(140.006_48, 588.663_45);
    holder.element_data_mut().set_direction_instantly(15);
    let actor = holder.actor_data_mut().expect("soldier actor data");
    actor.action_state = ActionState::HoldingShield;
    actor.shield_obstacle = Some(compute_shield_obstacle(
        MapPoint::new(140.006_48, 588.663_45),
        0.0,
        15,
        &shield_params_for_soldier(40, 50),
    ));
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
        projectile_shield_holder(&entities, None, old, new, increment),
        Some(holder_id),
        "Original IsReachable tests the ground crossing before the shield obstacle list"
    );
}

/// An arrow that runs out of trajectory without hitting anything
/// stops flying on the landing tick and despawns.
#[test]
fn tick_arrows_miss_and_land_despawns() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let trajectory = vec![TrajectoryPoint {
        position: WorldPoint3D {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        },
        time: 1,
    }];
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(crate::entity_id::PcId(0)),
        bow_point: WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 5.0,
        },
        trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
        target: EntityId::Pc(crate::entity_id::PcId(0)),
        target_pos: MapPoint { x: 10.0, y: 0.0 },
        trajectory,
        damage: 30,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    // No other humans in range — arrow will fly out and land.
    let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(arrow)]);

    let mut despawn = false;
    for _ in 0..10 {
        for r in tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        ) {
            if r.despawn && r.hit_target.is_none() && r.shield_hit.is_none() {
                despawn = true;
            }
        }
        if despawn {
            break;
        }
    }
    assert!(
        despawn,
        "arrow that misses should land and despawn without hit_target / shield_hit"
    );
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
    // Mirror MakeFallingDown's fresh ComputeTrajectory result. The empty
    // spawn above exhausted its placeholder trajectory and left the runtime
    // counter at the retired sentinel; Original starts this replacement
    // trajectory from its first point instead.
    arrow.projectile.trajectory_frame_count = 0;
    arrow.projectile.flying = true;
    arrow.projectile.launch_segment_start = None;
    preserve_falling_hole_disappearance(&mut arrow, true);
    assert!(
        arrow.projectile.disappear,
        "AddTrajectoryFallIntoHole marks even a one-waypoint trajectory"
    );

    preserve_falling_hole_disappearance(&mut arrow, false);
    assert!(
        arrow.projectile.disappear,
        "recomputing a dry falling trajectory must preserve an existing disappear flag"
    );

    arrow.advance_trajectory_one_frame();
    assert_eq!(arrow.element.position().z.to_bits(), endpoint.z.to_bits());
    let mut entities = entity_table(vec![
        Some(make_pc(100.0, 100.0)),
        Some(Entity::Projectile(arrow)),
    ]);
    let results = tick_arrows(
        &crate::sim_rng::test_context(),
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );

    assert!(results.iter().any(|result| result.despawn));
    let Entity::Projectile(arrow) = entities.get_at_index(1).unwrap().1 else {
        panic!("falling arrow changed concrete entity kind");
    };
    assert!(!arrow.projectile.flying);
    assert_eq!(
        arrow.element.position().z.to_bits(),
        endpoint.z.to_bits(),
        "mbDisappear returns before HitObstacle's +0.001 elevation snap"
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

    let mut entities = entity_table(vec![
        Some(make_pc(100.0, 100.0)),
        Some(Entity::Projectile(arrow)),
    ]);
    let results = tick_arrows(
        &crate::sim_rng::test_context(),
        &mut entities,
        crate::sight_obstacle::ObstacleList::empty(),
    );

    assert!(results.iter().any(|result| result.despawn));
    let Entity::Projectile(arrow) = entities.get_at_index(1).unwrap().1 else {
        panic!("falling arrow changed concrete entity kind");
    };
    assert!(!arrow.element.active);
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
    let throw_pos = WorldPoint3D {
        x: 0.0,
        y: 0.0,
        z: 50.0,
    };
    let target_pos = WorldPoint3D {
        x: 80.0,
        y: 0.0,
        z: 0.0,
    };
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

fn refresh_test_arrow() -> ElementProjectile {
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        ..Default::default()
    };
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
    // QuickSave frame 35731 creates an arrow during actor Hourglass and
    // then executes PlayDialog from SequenceManager::Hourglass. The
    // dialogue's nested RHGame::Refresh exposes the orientation before
    // RecordFrame instead of waiting for the ordinary deferred pass.
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

    // The exhausted trajectory's next Hourglass stops flight and snaps
    // the landing height. Original exposes that movement for one more
    // active snapshot. The following stopped Projectile::Hourglass owns
    // NewMove; Refresh only observes that snapshot and then retires it.
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

    // RHElementProjectile::Hourglass calls NewMove before checking
    // mbFlying, even for an arrow already stopped by HitTarget.
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

    // The next stopped Projectile::Hourglass, not Refresh, owns NewMove.
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

    // RHElementArrow::Refresh compares GetOldPosition/GetPosition, which
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

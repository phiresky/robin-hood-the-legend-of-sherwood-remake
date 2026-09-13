use super::*;
use crate::coordinates::WorldPoint3D;
use crate::element::{
    ActorData, ActorSoldier, ElementBonus, ElementData, ElementKind, ElementProjectile, HumanData,
    NpcData, ObjectData, ObjectType, Posture, ProjectileData, SoldierData,
};
use crate::sequence::SequenceElement;

fn make_soldier_at(x: f32, y: f32) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
        initial_element.kind = ElementKind::ActorSoldier;
        initial_element.active = true;
        initial_element
    };
    element.set_position(WorldPoint3D { x, y, z: 0.0 });
    element.set_position_map(crate::coordinates::MapPoint { x, y });
    element.set_direction_instantly(0);
    Entity::Soldier(ActorSoldier {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: SoldierData::default(),
    })
}

fn make_pc_at(x: f32, y: f32) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
        initial_element.kind = ElementKind::ActorPc;
        initial_element.active = true;
        initial_element
    };
    element.set_position(WorldPoint3D { x, y, z: 0.0 });
    element.set_position_map(crate::coordinates::MapPoint { x, y });
    Entity::Soldier(ActorSoldier {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: SoldierData::default(),
    })
}

fn make_projectile_object_at(object_type: ObjectType, x: f32, y: f32) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::default();
        initial_element.kind = ElementKind::ObjectProjectile;
        initial_element.active = true;
        initial_element
    };
    element.set_position(WorldPoint3D { x, y, z: 0.0 });
    element.set_position_map(crate::coordinates::MapPoint { x, y });
    Entity::Projectile(ElementProjectile {
        element,
        object: ObjectData {
            object_type,
            ..ObjectData::default()
        },
        projectile: ProjectileData::default(),
    })
}

fn make_bonus_object_at(object_type: ObjectType, x: f32, y: f32) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::default();
        initial_element.kind = if object_type == ObjectType::Ale {
            ElementKind::ObjectOther
        } else {
            ElementKind::ObjectBonus
        };
        initial_element.active = true;
        initial_element
    };
    element.set_position(WorldPoint3D { x, y, z: 0.0 });
    element.set_position_map(crate::coordinates::MapPoint { x, y });
    Entity::Bonus(ElementBonus {
        element,
        object: ObjectData {
            object_type,
            ..ObjectData::default()
        },
    })
}

fn launch_interaction_and_tick(
    command: Command,
    actor: Entity,
    antagonist: Entity,
) -> (EngineInner, EntityId) {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let actor_id = engine.add_test_entity(actor);
    let antagonist_id = engine.add_test_entity(antagonist);
    engine.launch_element(SequenceElement::new_interaction(
        1,
        command,
        Some(actor_id),
        Some(antagonist_id),
    ));

    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    super::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    assert_eq!(
        engine
            .get_entity(actor_id)
            .expect("interaction actor present")
            .element_data()
            .direction(),
        0,
        "the sequence-manager dispatch follows the entity loop, so its new order cannot turn the actor on the launch frame"
    );
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    (engine, actor_id)
}

#[test]
fn soldier_taking_sets_goal_and_turns_toward_antagonist() {
    let (engine, actor_id) = launch_interaction_and_tick(
        Command::Take,
        make_soldier_at(0.0, 0.0),
        make_projectile_object_at(ObjectType::Purse, 10.0, 0.0),
    );

    let actor = engine.get_entity(actor_id).unwrap();
    assert_eq!(actor.element_data().direction(), 1);
}

#[test]
fn soldier_drinking_ale_turns_toward_existing_goal() {
    let mut soldier = make_soldier_at(0.0, 0.0);
    soldier.element_data_mut().set_direction_goal(1);
    let (engine, actor_id) = launch_interaction_and_tick(
        Command::DrinkAle,
        soldier,
        make_bonus_object_at(ObjectType::Ale, 100.0, 0.0),
    );

    let actor = engine.get_entity(actor_id).unwrap();
    assert_eq!(actor.element_data().direction(), 1);
}

#[test]
fn crouched_pc_take_uses_stamped_crouched_animation() {
    let mut pc = make_pc_at(0.0, 0.0);
    pc.element_data_mut()
        .publish_order_posture(Posture::Crouched);
    let (engine, actor_id) = launch_interaction_and_tick(
        Command::Take,
        pc,
        make_bonus_object_at(ObjectType::BonusPurse, 10.0, 0.0),
    );

    assert_eq!(
        engine
            .get_entity(actor_id)
            .expect("crouched PC remains present")
            .actor_data()
            .expect("crouched PC retains actor data")
            .installed_order
            .as_ref()
            .map(|order| order.order_type),
        Some(OrderType::TakingCrouched),
        "PC Translate(Take) must use the interaction element's Crouched post-transition stamp"
    );
}

#[test]
fn nearby_pc_does_not_pick_up_bonus_without_take_command() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    engine.add_test_entity(make_pc_at(100.0, 100.0));
    let bonus_id =
        engine.add_test_entity(make_bonus_object_at(ObjectType::BonusPurse, 100.0, 100.0));

    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    super::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    let bonus = engine.get_entity(bonus_id).unwrap();
    assert!(bonus.element_data().active);
    assert!(!bonus.object_data().unwrap().taken);
}

use super::scenarios::assets_with_test_pc_profile;
use super::*;
use crate::engine::tick::capture_projectile_derived_tails;

/// Give every live test PC its required campaign-description identity.
///
/// Fixtures that drive engine passes directly (without going through
/// `complete_test_runtime_fixture`) still need each PC linked to a campaign
/// character entry — the runtime resolves coma/ammo state through that link
/// and treats a missing one as corrupted data.
pub(super) fn attach_test_campaign_identities(engine: &mut EngineInner) {
    let campaign = &mut engine.mission_domain.campaign;
    for (_, pc) in engine.world.entities.pcs_mut() {
        if pc.pc.campaign_description_index.is_some() {
            continue;
        }
        let idx = campaign.characters.len();
        pc.pc.campaign_description_index = Some(idx as u32);
        campaign.characters.push(crate::campaign::PcDescription {
            character_profile_idx: Some(pc.pc.profile_index),
            ..Default::default()
        });
    }
}

fn immortal_pc_hit_by_creation_ordered_arrow(pc_before_arrow: bool) -> i16 {
    use crate::bow_shot::{SpawnArrowParams, spawn_arrow};
    use crate::coordinates::{WorldPoint3D, WorldVec3D};
    use crate::element::Posture;
    use crate::entity_id::PcId;

    let mut engine = EngineInner::new();

    let mut shooter = make_test_soldier(Posture::Upright);
    shooter
        .element_data_mut()
        .set_position_map(MapPoint { x: 0.0, y: 0.0 });
    let Entity::Soldier(shooter_soldier) = &mut shooter else {
        unreachable!();
    };
    shooter_soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
    shooter_soldier.npc.life_points = 100;
    let shooter_id = engine.add_test_entity(shooter);

    let victim_id = EntityId::Pc(PcId(if pc_before_arrow { 1 } else { 2 }));
    let make_arrow = || {
        spawn_arrow(SpawnArrowParams {
            shooter: shooter_id,
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: victim_id,
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: vec![crate::element::TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 2,
            }],
            damage: 10,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        })
    };
    let mut victim = make_test_pc(Posture::Upright);
    victim
        .element_data_mut()
        .set_position_map(MapPoint { x: 50.0, y: 0.0 });
    let Entity::Pc(victim_pc) = &mut victim else {
        unreachable!();
    };
    victim_pc.pc.life_points = 74;
    victim_pc.pc.immortal = true;

    if pc_before_arrow {
        assert_eq!(engine.add_entity(victim), victim_id);
        engine.add_test_entity(make_arrow());
    } else {
        engine.add_test_entity(make_arrow());
        assert_eq!(engine.add_entity(victim), victim_id);
    }

    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    let Some(Entity::Pc(victim)) = engine.get_entity(victim_id) else {
        panic!("victim PC missing after projectile frame");
    };
    victim.pc.life_points
}

fn corpse_exit_initialization_fixture(
    active_drop: bool,
    command: crate::element::Command,
) -> (EngineInner, EntityId, EntityId, crate::sequence::SequenceId) {
    use crate::element::Posture;
    use crate::movement::{AbilityKind, ActiveAbility};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let body = engine.add_test_entity(make_test_soldier(Posture::Carried));
    let carrier = engine.add_test_entity(make_test_pc(Posture::CarryingCorpse));
    {
        let carrier_entity = engine.get_entity_mut(carrier).unwrap();
        carrier_entity.pc_data_mut().unwrap().carried = Some(body);
        carrier_entity
            .pc_data_mut()
            .unwrap()
            .set_live_carried_posture(Posture::Lying);
        carrier_entity
            .element_data_mut()
            .set_direction_instantly(13);
    }
    {
        let body_entity = engine.get_entity_mut(body).unwrap();
        body_entity.human_data_mut().unwrap().carrier = Some(carrier);
        body_entity.actor_data_mut().unwrap().execution_frozen = true;
        body_entity.element_data_mut().set_direction_instantly(4);
    }

    let transition = OrderType::TransitionCarryingCorpseWaitingUpright;
    let script = SpriteScript {
        action_id: transition as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2],
        delays: vec![10, 10],
        distances: vec![0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
        sound_ids: vec![0, 0],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[transition as usize] = 0;
    engine
        .get_entity_mut(carrier)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    engine
        .get_entity_mut(carrier)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(13);

    let order_id = engine.orders.allocate_order_id();
    let mut element = SequenceElement::new(1, command, Some(carrier));
    let mut transition_order = Order::new(transition, 0.0, 0.0, order_id);
    transition_order.compute_direction = false;
    element.orders.push_back(transition_order);
    if command == crate::element::Command::EnterSwordfight {
        let raising_id = engine.orders.allocate_order_id();
        element.orders.push_back(Order::new(
            OrderType::TransitionRaisingSword,
            0.0,
            0.0,
            raising_id,
        ));
    }
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    if active_drop {
        engine
            .get_entity_mut(carrier)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_ability = ActiveAbility {
            kind: Some(AbilityKind::Drop),
            sequence_id: Some(sequence),
            element_index: 0,
            target: Some(body),
            order_id: Some(order_id),
            done_effect_applied: false,
            strangle_initialized: false,
        };
    }
    (engine, carrier, body, sequence)
}

/// Original-game corpse dropping stamps the body's facing
/// at `carrier_direction + 12` and then clears the carried relationship, whose
/// setting direction to the carrier's direction
/// direction update moves only the *goal*
/// to the carrier's own heading. The
/// dropped body therefore ends the drop with a goal exactly four
/// sectors ahead of its facing.

/// Original-game action selection closes each selected PC's synchronous stop
/// before the next engine update. If that Stop interrupts a mid-grab
/// `TakeCorpse`, the body's Wait is therefore installed before its own actor
/// slot and executes in that same frame.

fn install_owner_selected_test_melee(
    engine: &mut EngineInner,
    attacker: EntityId,
    target: EntityId,
    order_type: crate::order::OrderType,
    past_action_done: bool,
) {
    install_owner_selected_test_melee_frames(
        engine,
        attacker,
        target,
        order_type,
        past_action_done,
        3,
    )
}

/// `install_owner_selected_test_melee` with a configurable animation length.
/// Sweeping strikes need frames after the action-done tick: the sweep only
/// rotates and tests victims while the strike animation is still playing.
fn install_owner_selected_test_melee_frames(
    engine: &mut EngineInner,
    attacker: EntityId,
    target: EntityId,
    order_type: crate::order::OrderType,
    past_action_done: bool,
    animation_frames: usize,
) {
    let sequence =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::SwordstrikeThrustA,
                Some(attacker),
            ));
    let order_id = engine.orders.allocate_order_id();
    let mut order = crate::order::Order::new(order_type, 0.0, 0.0, order_id);
    order.antagonist = Some(target);
    engine
        .orders
        .sequence_manager
        .push_order_on(sequence, 0, order);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    bind_test_action_point(
        engine,
        attacker,
        order_type,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    let sim = crate::sim_rng::test_context();
    let entity = engine
        .get_entity_mut(attacker)
        .expect("selected melee test attacker exists");
    let mut script = entity.element_data().sprite.scripts[0].clone();
    script.action_done = 1;
    script.frame_ids = (1..=animation_frames as u32).collect();
    script.delays = vec![0; animation_frames];
    script.distances = vec![0; animation_frames];
    script.offsets = vec![crate::coordinates::SpriteFrameOffset::ZERO; animation_frames];
    script.sound_ids = vec![0; animation_frames];
    entity.element_data_mut().sprite.scripts = std::sync::Arc::new(vec![script; 16]);
    let direction = entity.element_data().direction() as u16;
    let sprite = &mut entity.element_data_mut().sprite;
    assert_eq!(
        sprite.perform_action(
            &sim,
            Some(order_id),
            order_type,
            direction,
            crate::sprite::FrameProgression::Default,
            false,
        ),
        crate::sprite::MotionState::Start
    );
    while sprite.frames_from_now_till_action_done() > 0 {
        assert_eq!(
            sprite.perform_action(
                &sim,
                Some(order_id),
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            ),
            crate::sprite::MotionState::InProgress
        );
    }
    if past_action_done {
        assert_eq!(
            sprite.perform_action(
                &sim,
                Some(order_id),
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            ),
            crate::sprite::MotionState::Done
        );
    }
}

fn chained_straight_strike_target_life(interrupter_first: bool) -> i16 {
    use crate::coordinates::WorldPoint3D;
    use crate::element::Posture;
    use crate::profiles::{CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile};
    use crate::weapons::SwordStrike;

    fn position(entity: &mut Entity, x: f32) {
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position(WorldPoint3D { x, y: 0.0, z: 0.0 });
        entity
            .element_data_mut()
            .set_position_map(MapPoint { x, y: 0.0 });
    }

    let mut engine = EngineInner::new();
    let mut interrupter = make_test_pc(Posture::Upright);
    position(&mut interrupter, 0.0);
    let mut chained_attacker = make_test_soldier(Posture::Upright);
    position(&mut chained_attacker, 20.0);
    let Entity::Soldier(soldier) = &mut chained_attacker else {
        unreachable!();
    };
    soldier.npc.life_points = 1;
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test soldier has enemy AI")
        .hth_weapon_id = 1;
    soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
    let mut final_target = make_test_pc(Posture::Upright);
    position(&mut final_target, 40.0);
    let Entity::Pc(pc) = &mut final_target else {
        unreachable!();
    };
    pc.pc.life_points = 50;

    let (interrupter_id, chained_attacker_id) = if interrupter_first {
        (
            engine.add_test_entity(interrupter),
            engine.add_test_entity(chained_attacker),
        )
    } else {
        let chained_attacker_id = engine.add_test_entity(chained_attacker);
        let interrupter_id = engine.add_test_entity(interrupter);
        (interrupter_id, chained_attacker_id)
    };
    let final_target_id = engine.add_test_entity(final_target);
    engine
        .get_entity_mut(chained_attacker_id)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .me = chained_attacker_id.index();

    for (attacker, target) in [
        (interrupter_id, chained_attacker_id),
        (chained_attacker_id, final_target_id),
    ] {
        install_owner_selected_test_melee(
            &mut engine,
            attacker,
            target,
            crate::order::OrderType::StrikingStraightSword,
            false,
        );
    }
    attach_test_campaign_identities(&mut engine);

    let mut profiles = ProfileManager::new();
    let mut weapon = HtHWeaponProfile::default();
    weapon.thrusts[SwordStrike::A as usize].minimal_distance = 0;
    weapon.thrusts[SwordStrike::A as usize].maximal_distance = 100;
    weapon.thrusts[SwordStrike::A as usize].cutting = 100;
    profiles.hth_weapons.push(weapon);
    profiles.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        ..SoldierProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };
    crate::sim_rng::with_seed(0xA_B_C, |sim| {
        let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        engine.tick_actor_owner_envelopes(sim, &assets, &positions);
        assert_strike_damage_deferred_then_drain(
            &mut engine,
            sim,
            &assets,
            chained_attacker_id,
            final_target_id,
            interrupter_first,
        );
    });

    let Entity::Pc(target) = engine
        .get_entity(final_target_id)
        .expect("final chained-strike target present")
    else {
        panic!("final chained-strike target must be a PC");
    };
    target.pc.life_points
}

/// After the entity envelope pass a strike must only have registered its
/// `ReceiveSwordDamage` elements — no HP mutation happens until the sequence
/// manager drains them, and registration follows the attackers'
/// creation-slot order. Drains the manager phase before returning.
fn assert_strike_damage_deferred_then_drain(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    chained_attacker_id: EntityId,
    final_target_id: EntityId,
    interrupter_first: bool,
) {
    assert_eq!(
        strike_life_points(engine, chained_attacker_id, final_target_id),
        (1, 50),
        "strike resolution must not mutate lives before the manager phase"
    );

    let damage_owners = registered_sword_damage_owners(engine);
    let expected = if interrupter_first {
        vec![chained_attacker_id, final_target_id]
    } else {
        vec![final_target_id, chained_attacker_id]
    };
    assert_eq!(
        damage_owners, expected,
        "damage registration must follow the attackers' creation-slot order"
    );

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(sim, &mut display, assets);
}

/// `(chained attacker soldier life, final target PC life)` for the chained
/// strike fixtures.
fn strike_life_points(
    engine: &EngineInner,
    chained_attacker_id: EntityId,
    final_target_id: EntityId,
) -> (i16, i16) {
    let chained_life = match engine
        .get_entity(chained_attacker_id)
        .expect("chained attacker present after envelope pass")
    {
        Entity::Soldier(soldier) => soldier.npc.life_points,
        _ => panic!("chained attacker must be a soldier"),
    };
    let final_life = match engine
        .get_entity(final_target_id)
        .expect("final target present after envelope pass")
    {
        Entity::Pc(pc) => pc.pc.life_points,
        _ => panic!("final target must be a PC"),
    };
    (chained_life, final_life)
}

/// Owners of every registered `ReceiveSwordDamage` element, in the manager's
/// sequence launch order.
fn registered_sword_damage_owners(engine: &EngineInner) -> Vec<EntityId> {
    engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.command == crate::element::Command::ReceiveSwordDamage)
        .filter_map(|element| element.owner)
        .collect()
}

#[derive(Clone, Copy)]
enum NonstraightInterrupt {
    Lateral,
    Push,
}

fn chained_nonstraight_strike_lives(
    interrupt: NonstraightInterrupt,
    interrupter_first: bool,
) -> (i16, i16) {
    use crate::coordinates::{MapVec, MoveBox, WorldPoint3D};
    use crate::element::Posture;
    use crate::profiles::{
        CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile, WeaponThrustDirection,
        WeaponThrustKind,
    };
    use crate::weapons::SwordStrike;

    fn position(entity: &mut Entity, x: f32, y: f32) {
        let element = entity.element_data_mut();
        element.active = true;
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
        element.set_position_map(MapPoint { x, y });
        element.set_direction_instantly(0);
        element
            .sprite
            .position_iface
            .set_move_box(MoveBox::from_corners(
                MapVec::new(-5.0, -5.0),
                MapVec::new(5.0, 5.0),
            ));
    }

    let mut engine = EngineInner::new();
    let mut interrupter = make_test_pc(Posture::Upright);
    position(&mut interrupter, 0.0, 100.0);
    let mut chained_attacker = make_test_soldier(Posture::Upright);
    position(&mut chained_attacker, 0.0, 50.0);
    let Entity::Soldier(soldier) = &mut chained_attacker else {
        unreachable!();
    };
    soldier.npc.life_points = 1;
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test soldier has enemy AI")
        .hth_weapon_id = 1;
    soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
    let mut final_target = make_test_pc(Posture::Upright);
    // Remain within the chained attacker's 100-unit straight range but
    // outside the interrupter's 100x100 push rectangle (half-width 50).
    position(&mut final_target, 60.0, 50.0);
    let Entity::Pc(pc) = &mut final_target else {
        unreachable!();
    };
    pc.pc.life_points = 50;

    let (interrupter_id, chained_attacker_id) = if interrupter_first {
        (
            engine.add_test_entity(interrupter),
            engine.add_test_entity(chained_attacker),
        )
    } else {
        let chained_attacker_id = engine.add_test_entity(chained_attacker);
        let interrupter_id = engine.add_test_entity(interrupter);
        (interrupter_id, chained_attacker_id)
    };
    let final_target_id = engine.add_test_entity(final_target);
    engine
        .get_entity_mut(chained_attacker_id)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .me = chained_attacker_id.index();

    install_owner_selected_test_melee(
        &mut engine,
        chained_attacker_id,
        final_target_id,
        crate::order::OrderType::StrikingStraightSword,
        false,
    );

    match interrupt {
        NonstraightInterrupt::Lateral => {
            // Face the victim so the sweep's profile-derived arc passes over
            // its sector; the strike's Done tick initializes the sweep from
            // the thrust profile and it rotates only while the strike
            // animation is still playing, so give it a long tail.
            let facing = crate::position_interface::vector_to_sector_0_to_15(0.0, -1.0);
            engine
                .get_entity_mut(interrupter_id)
                .expect("lateral attacker present")
                .element_data_mut()
                .set_direction_instantly(facing);
            install_owner_selected_test_melee_frames(
                &mut engine,
                interrupter_id,
                chained_attacker_id,
                crate::order::OrderType::StrikingLeftSword,
                false,
                20,
            );
        }
        NonstraightInterrupt::Push => {
            install_owner_selected_test_melee(
                &mut engine,
                interrupter_id,
                chained_attacker_id,
                crate::order::OrderType::StrikingLeftSword,
                false,
            );
        }
    }
    attach_test_campaign_identities(&mut engine);

    let mut profiles = ProfileManager::new();
    let mut weapon = HtHWeaponProfile::default();
    let straight = &mut weapon.thrusts[SwordStrike::A as usize];
    straight.minimal_distance = 0;
    straight.maximal_distance = 100;
    straight.cutting = 100;
    let nonstraight = &mut weapon.thrusts[SwordStrike::D as usize];
    nonstraight.kind = match interrupt {
        NonstraightInterrupt::Lateral => WeaponThrustKind::Lateral,
        NonstraightInterrupt::Push => WeaponThrustKind::PushAside,
    };
    nonstraight.direction = WeaponThrustDirection::LeftToRight;
    nonstraight.minimal_distance = 0;
    nonstraight.maximal_distance = 100;
    nonstraight.repulsion = 100;
    nonstraight.cutting = 100;
    // Sweep geometry: start 45 degrees before the attacker's facing and
    // rotate 30 degrees per frame, so the arc crosses the faced victim
    // within a few in-progress animation ticks.
    nonstraight.initial_angle = 45;
    nonstraight.rotation_angle = 30;
    profiles.hth_weapons.push(weapon);
    profiles.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        ..SoldierProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };
    crate::sim_rng::with_seed(0xD_E_F, |sim| {
        let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        // A lateral sweep only reaches its victim's sector after several
        // rotation steps, so keep running envelope passes until both damage
        // elements are registered. No HP may mutate before the manager
        // phase drains them.
        let mut registered = Vec::new();
        for _ in 0..32 {
            engine.tick_actor_owner_envelopes(sim, &assets, &positions);
            assert_eq!(
                strike_life_points(&engine, chained_attacker_id, final_target_id),
                (1, 50),
                "strike resolution must not mutate lives before the manager phase"
            );
            registered = registered_sword_damage_owners(&engine);
            if registered.len() == 2 {
                break;
            }
        }
        registered.sort();
        let mut expected = vec![chained_attacker_id, final_target_id];
        expected.sort();
        assert_eq!(
            registered, expected,
            "both strikes must register their damage before the manager phase"
        );
        let mut display = HostDisplayState::default();
        engine.hourglass_phase_sequences(sim, &mut display, &assets);
    });

    let Entity::Pc(target) = engine
        .get_entity(final_target_id)
        .expect("final chained-strike target present")
    else {
        panic!("final chained-strike target must be a PC");
    };
    let final_target_life = target.pc.life_points;
    let Entity::Soldier(chained_attacker) = engine
        .get_entity(chained_attacker_id)
        .expect("interrupted chained attacker remains present")
    else {
        panic!("chained attacker must remain a soldier");
    };
    (final_target_life, chained_attacker.npc.life_points)
}

mod combat;
mod movement;
mod orders;
mod perception;
mod projectiles;
/// Rollback determinism: clone the engine mid-run, advance the clone and
/// the original the same number of ticks, and verify they end up in the
/// same state. This is the foundation test for rollback multiplayer — if
/// it ever fails, determinism is broken somewhere in the tick path.
///
/// We advance past `frame_counter % 25 == 0` (the script-hourglass
/// boundary) a few times to exercise the scripted slow path as well as
/// the regular frame path, and we seed the RNG to a non-zero state so
/// any RNG consumer during the tick would diverge between seeded and
/// un-seeded paths.
///
/// This will grow as more sim surface comes online — right now there are
/// no entities, so it mostly exercises frame counters, script ticks,
/// chorus timer, mission state, and the RNG/sound-queue plumbing.

/// The original game loop calls mission post-initialization only after its
/// first forced refresh and sound update. Keep the
/// engine tick and that host-owned boundary observably separate: frame
/// zero must finish without flipping the serialized one-shot flag, and
/// the explicit post-refresh stage must flip it without advancing time.
mod state;

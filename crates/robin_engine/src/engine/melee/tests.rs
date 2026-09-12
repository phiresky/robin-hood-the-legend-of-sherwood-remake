use super::*;
use crate::ai::AiEntityHandle;
use crate::coordinates::WorldPoint3D;
use crate::element::ActiveFlight;
use crate::scb::{ClassEntry, SCB_VERSION, ScbFile};

fn make_engine() -> EngineInner {
    let mut engine = EngineInner::new();
    // Every PC built by `make_pc` carries campaign-description index 0,
    // so the campaign character table needs a matching entry backing the
    // required live-PC identity.
    engine.mission_domain.campaign.characters = vec![crate::campaign::PcDescription {
        character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
        ..Default::default()
    }];
    engine
}

fn empty_mission_script() -> crate::engine::types::MissionScript {
    let startup = crate::engine::test_support::asm::empty_startup_class("melee_test.scs".into());
    crate::engine::types::MissionScript::from_scb(ScbFile {
        version: SCB_VERSION,
        classes: vec![startup],
    })
    .expect("minimal StartUp script must load")
}

fn make_soldier(
    pos: WorldPoint3D,
    sector: Option<crate::position_interface::SectorHandle>,
) -> Entity {
    let mut entity = crate::engine::test_support::actors::make_test_ai_soldier(
        crate::element::Camp::Lacklandists,
    );
    entity.position_iface_mut().clear_pathfinder_index();
    entity.element_data_mut().active = true;
    entity.element_data_mut().set_position(pos);
    entity
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::from_world_xyz(
            pos.x, pos.y, pos.z,
        ));
    entity
        .position_iface_mut()
        .set_sector_topology(sector, sector.and_then(|sector| sector.arena_index()));
    entity.npc_data_mut().expect("soldier fixture").life_points = 50;
    entity
}

fn make_pc(pos: WorldPoint3D, sector: Option<crate::position_interface::SectorHandle>) -> Entity {
    let mut entity = crate::engine::test_support::actors::make_test_pc(Posture::Upright);
    entity.position_iface_mut().clear_pathfinder_index();
    entity.element_data_mut().active = true;
    entity.element_data_mut().set_position(pos);
    entity
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::from_world_xyz(
            pos.x, pos.y, pos.z,
        ));
    entity
        .position_iface_mut()
        .set_sector_topology(sector, sector.and_then(|sector| sector.arena_index()));
    let pc = entity.pc_data_mut().expect("PC fixture");
    pc.life_points = 50;
    pc.profile_index = crate::profiles::CharacterProfileIdx(0);
    pc.campaign_description_index = Some(0);
    entity
}

fn action_test_assets(actions: [crate::profiles::Action; 3]) -> LevelAssets {
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        actions,
        ..Default::default()
    });
    profiles
        .soldiers
        .push(crate::profiles::SoldierProfile::default());
    LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    }
}

fn make_civilian(pos: WorldPoint3D) -> Entity {
    let mut entity = crate::engine::test_support::actors::make_test_civilian(Posture::Upright);
    entity.position_iface_mut().clear_pathfinder_index();
    entity.element_data_mut().active = true;
    entity.element_data_mut().set_position(pos);
    entity
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::from_world_xyz(
            pos.x, pos.y, pos.z,
        ));
    entity.npc_data_mut().expect("civilian fixture").life_points = 100;
    entity
}

/// Set up a live falling-hit Execute flight on `flyer` so the per-frame
/// `tick_push_flights` sweep fires `apply_domino_effect`.
fn give_flight(
    engine: &mut EngineInner,
    flyer: EntityId,
    antagonist: EntityId,
    inc_x: f32,
    inc_y: f32,
    frames: u16,
) {
    engine
        .get_entity_mut(flyer)
        .expect("test flight owner exists")
        .element_data_mut()
        .sprite
        .scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
        frame_ids: vec![0, 1],
        ..Default::default()
    }]);
    let flyer_pos = engine
        .get_entity(flyer)
        .unwrap()
        .element_data()
        .position_map();

    // Combat flight belongs to the live falling order's execution
    // arm. Mirror that lifecycle instead of manufacturing an orphaned
    // `active_flight`, which production correctly holds until the order is
    // current and its START edge has changed posture to Flying.
    let damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveHitDamage,
        Some(flyer),
        Some(antagonist),
        1,
        0,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    let order_id = engine.push_new_order(
        sequence,
        0,
        crate::order::OrderType::FallingHitUpright,
        0.0,
        0.0,
    );
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    if let Some(entity) = engine.world.entities.get_mut(flyer) {
        entity.set_posture(Posture::Flying);
        let actor = entity
            .actor_data_mut()
            .expect("combat flight owner must be an actor");
        actor.installed_order = Some(crate::element::InstalledActorOrder {
            order_id,
            order_type: crate::order::OrderType::FallingHitUpright,
        });
        actor.active_flight = Some(ActiveFlight {
            increment_x: inc_x,
            increment_y: inc_y,
            goal_x: flyer_pos.x + inc_x * frames as f32,
            goal_y: flyer_pos.y + inc_y * frames as f32,
            frames_remaining: frames,
            antagonist: Some(antagonist),
            ..Default::default()
        });
    }
}

fn count_domino_hits_for(engine: &EngineInner, victim: EntityId, hitter: EntityId) -> usize {
    engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|s| s.elements.iter())
        .filter(|e| {
            e.command == Command::ReceiveHitDamage
                && e.owner == Some(victim)
                && match &e.data {
                    SequenceElementData::Damage {
                        origin,
                        damage,
                        concussion,
                        is_harder_hit,
                        ..
                    } => {
                        *origin == Some(hitter)
                            && *damage == 0
                            && *concussion == DOMINO_DAMAGE
                            && !*is_harder_hit
                    }
                    _ => false,
                }
        })
        .count()
}

fn initialized_hit_flight_delta(
    engine: &EngineInner,
    victim: EntityId,
) -> crate::coordinates::MapPoint {
    let victim = engine.get_entity(victim).unwrap();
    let flight = victim
        .actor_data()
        .unwrap()
        .active_flight
        .as_ref()
        .expect("unobstructed falling hit must initialize a flight");
    let position = victim.element_data().position_map();
    crate::coordinates::MapPoint::new(flight.goal_x - position.x, flight.goal_y - position.y)
}

fn authorize_test_hit_flight(engine: &mut EngineInner, victim: EntityId) {
    engine.world.fast_grid_mut().size_map(4, 4);
    engine.world.fast_grid_mut().allocate_layers(1);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -5.0, -5.0, 5.0, 5.0,
        ));
}

fn assets_with_sword_profile(energy: u16, max_distance: u16) -> LevelAssets {
    assets_with_sword_profile_effects(energy, max_distance, 4, 0)
}

fn assets_with_sword_profile_effects(
    energy: u16,
    max_distance: u16,
    cutting: u16,
    stunning: u16,
) -> LevelAssets {
    let mut profile_manager = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] = max_distance;
    weapon.thrusts[SwordStrike::A as usize].energy = energy;
    weapon.thrusts[SwordStrike::A as usize].minimal_distance = 0;
    weapon.thrusts[SwordStrike::A as usize].maximal_distance = max_distance;
    weapon.thrusts[SwordStrike::A as usize].cutting = cutting;
    weapon.thrusts[SwordStrike::A as usize].stunning = stunning;
    profile_manager.hth_weapons.push(weapon);
    profile_manager
        .characters
        .push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
    profile_manager
        .soldiers
        .push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            fighting: 20,
            ..crate::profiles::SoldierProfile::default()
        });

    LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    }
}

fn make_enemy_strike_pair(
    engine: &mut EngineInner,
    pending_consideration: bool,
) -> (EntityId, EntityId) {
    let attacker = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        soldier.actor.action_state = ActionState::WaitingSword;
        soldier.human.opponents.push(target);
        let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
            unreachable!()
        };
        ai.base.current_state = crate::ai::AiState::Attacking;
        ai.base.current_substate = crate::ai::Substate::AttackingSwordfight;
        ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(target.index()));
        ai.hth_weapon_id = 1;
        ai.pending_sword_strike_consideration = pending_consideration;
    }
    {
        let target_entity = engine.get_entity_mut(target).unwrap();
        target_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        target_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        // Strike selection owns a required sprite timing read for
        // its principal opponent.  Keep this shared synthetic duel
        // fixture structurally valid instead of relying on Sprite's
        // asset-less default row.
        target_entity.element_data_mut().sprite.scripts =
            std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 1,
                frame_ids: vec![0, 1, 2],
                delays: vec![1, 1, 1],
                distances: vec![0, 0, 0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
                sound_ids: vec![0, 0, 0],
                ..Default::default()
            }]);
    }
    (attacker, target)
}

fn make_enemy_ai_hero_strike_pair(engine: &mut EngineInner) -> (EntityId, EntityId) {
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    for (owner, opponent, camp, authorize_strike) in [
        (attacker, target, crate::element::Camp::Custom(2), true),
        (target, attacker, crate::element::Camp::Custom(3), false),
    ] {
        let Entity::Pc(pc) = engine.get_entity_mut(owner).unwrap() else {
            unreachable!()
        };
        pc.actor.action_state = ActionState::WaitingSword;
        pc.human.opponents.push(opponent);
        pc.pc.cached_camp = camp;
        pc.pc.command_interface = crate::human_control::CommandInterface::None;
        pc.pc.mission_role = crate::human_control::MissionRole::Combatant;
        pc.pc.combat_stance = crate::human_control::CombatStance::Aggressive;
        let mut ai = crate::ai_enemy::EnemyAi::new(owner.index());
        ai.base.current_state = crate::ai::AiState::Attacking;
        ai.base.current_substate = crate::ai::Substate::AttackingSwordfight;
        ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(opponent.index()));
        ai.hth_weapon_id = 1;
        ai.pending_sword_strike_consideration = authorize_strike;
        pc.pc.ai = Some(Box::new(crate::element::AiActorData {
            ai_brain: crate::element::AiBrain::Enemy(Box::new(ai)),
            ..Default::default()
        }));
        pc.element.sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            action_done: 1,
            frame_ids: vec![0, 1, 2],
            delays: vec![1, 1, 1],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0, 0, 0],
            ..Default::default()
        }]);
        pc.element.sprite.conversion =
            std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }

    (attacker, target)
}

fn assets_with_nonstraight_profile(
    strike: SwordStrike,
    kind: crate::profiles::WeaponThrustKind,
) -> LevelAssets {
    let mut profile_manager = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    let thrust = &mut weapon.thrusts[strike as usize];
    thrust.kind = kind;
    thrust.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
    thrust.minimal_distance = 0;
    thrust.maximal_distance = 100;
    thrust.initial_angle = 0;
    thrust.final_angle = 180;
    thrust.rotation_angle = 90;
    thrust.repulsion = 100;
    thrust.cutting = 100;
    profile_manager.hth_weapons.push(weapon);
    profile_manager
        .characters
        .push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
    profile_manager
        .soldiers
        .push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });

    LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    }
}

fn soldier_life(engine: &EngineInner, soldier_id: EntityId) -> i16 {
    match engine
        .get_entity(soldier_id)
        .expect("test soldier must remain present")
    {
        Entity::Soldier(soldier) => soldier.npc.life_points,
        _ => panic!("test victim must be a soldier"),
    }
}

fn install_test_melee_order(
    engine: &mut EngineInner,
    attacker: EntityId,
    target: EntityId,
    strike: SwordStrike,
    past_action_done: bool,
) -> crate::engine::tick::MeleeOwnerSelection {
    let order_type = strike_to_animation(strike);
    let sequence = engine.orders.sequence_manager.launch_element(
        crate::sequence::SequenceElement::new_interaction(
            1,
            strike.to_command(),
            Some(attacker),
            Some(target),
        ),
    );
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

    let script = crate::sprite_script::SpriteScript {
        action_id: order_type as u16,
        action_done: 1,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0, 0, 0],
        ..Default::default()
    };
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    conversion[order_type as usize] = 0;
    let entity = engine.get_entity_mut(attacker).unwrap();
    let position_iface = entity.element_data().sprite.position_iface.clone();
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    sprite.position_iface = position_iface;
    entity.element_data_mut().sprite = sprite;
    let direction = entity.element_data().direction() as u16;
    let sim = crate::sim_rng::test_context();
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
    crate::engine::tick::MeleeOwnerSelection {
        seq_id: sequence,
        elem_idx: 0,
        order_id,
    }
}

/// `SwordstrikeThrustA` promotes both principal opponents before
/// the strike, so clicking a secondary opponent during a
/// swordfight switches the primary target.

/// Build a cross-sector `EnterSwordfight` dispatch where `crowding`
/// fighters from the owner's sector already engage the opponent, and the
/// element carries no jump line.  Returns the engine, the owner and the
/// launched sequence id after dispatch.
fn dispatch_crowded_cross_sector_swordfight(
    crowding: usize,
) -> (EngineInner, EntityId, EntityId, crate::sequence::SequenceId) {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner_sector = crate::position_interface::SectorHandle::new(1);
    let opponent_sector = crate::position_interface::SectorHandle::new(2);
    assert_ne!(owner_sector, opponent_sector);

    let owner = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        owner_sector,
    ));
    let opponent = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 60.0,
            y: 100.0,
            z: 0.0,
        },
        opponent_sector,
    ));
    for index in 0..crowding {
        let fighter = engine.add_test_entity(make_soldier(
            WorldPoint3D {
                x: index as f32 * 10.0,
                y: 120.0,
                z: 0.0,
            },
            owner_sector,
        ));
        engine
            .get_entity_mut(opponent)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(fighter);
    }
    assert_eq!(
        number_of_table_swordfight_opponents(
            &engine.world.entities,
            opponent,
            i16::from(owner_sector.unwrap()),
        ),
        crowding as u32,
    );

    let mut element =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    element.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    // Deliberately no `Field::JumplineDestination`: the original's
    // The jump line is absent here, which must not switch the occupancy gate
    // off.
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(element);
    let seq_id = engine.launch_sequence(sequence);

    engine.dispatch_enter_swordfight(
        &sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        seq_id,
        0,
    );
    (engine, owner, opponent, seq_id)
}

mod combat;
/// Original-game actor translation case
/// Entering a swordfight
/// runs the cross-sector occupancy gate for every unprepared element that
/// has an opponent—the presence check for a jump line guards only the inner
/// slot-search half.  So a jump-line-less element still interrupts when
/// The swordfight-opponent count reports 3+ fighters on our side,
/// and the PC never reaches the `TransitionRaisingSword` order.

/// Counterpart of the gate above: with fewer than 3 fighters already on
/// our side and no jump line, the original falls straight through the
/// jump-line-present block and enters the swordfight normally
/// during swordfight entry.

/// Bud-Spencer-style line of three: PC punches the first soldier,
/// who is launched along +X into a second soldier directly in
/// front, and a third soldier behind the second. The flight tick
/// should fire a domino RECEIVE_HIT_DAMAGE on both downstream
/// soldiers, citing the PC as origin.

/// The domino effect measures the literal world X/Y ground plane. An
/// elevated victim can therefore be inside the 15-unit world radius even
/// when projecting elevation into map Y would put it outside the radius.

/// Actors behind the flight vector (negative dot product) are
/// outside the punch arc and must not take damage.

/// The Chebyshev pre-filter (maximum norm < `DOMINO_DISTANCE`) and the
/// Euclidean check both have to fire. Place a candidate just past
/// the radius and assert it is skipped.

/// Non-upright actors (lying, dead, etc.) are excluded — they're
/// already on the ground and the upright-only filter rejects them.

/// Rolling and ladder/wall flights set `antagonist = None`, so the
/// per-frame sweep skips them entirely. Verify by giving the flyer
/// a None-antagonist flight even though there's a candidate
/// directly in the flight path.

/// Regression: cheat-driven `apply_concussion` on a PC must seed
/// `concussion_healing_timeout` with the PC profile's `wake_up`,
/// not the soldier fallback constant.  Before the asset-context
/// plumbing landed, the cheat path hard-coded
/// `SOLDIER_CONCUSSION_HEALING_SPEED` because `&LevelAssets`
/// wasn't reachable from `dispatch_console_command`.
mod movement;
mod orders;
mod perception;
mod projectiles;
mod state;

#[cfg(test)]
mod suite {
    use super::super::*;
    use crate::element::{
        ActionState, ActorData, ActorPc, ActorSoldier, Command, ElementData, ElementKind, Entity,
        HumanData, NpcData, PcData, Posture, SoldierData,
    };
    use crate::order::Order;
    use crate::sequence::{
        MoveFlags, SequenceElement, SequenceElementData, SequencePriority, SequenceState,
    };
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    fn make_test_pc(posture: Posture) -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    fn assets_with_test_pc_profile() -> LevelAssets {
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles
            .characters
            .push(crate::profiles::CharacterProfile::default());
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile::default());
        LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::new()
        }
    }

    fn shield_movement_sprite() -> crate::sprite::Sprite {
        let directional_script = |action: OrderType| SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 8.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 8,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![8],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut scripts = Vec::with_capacity(64);
        for action in [
            OrderType::WalkingShield,
            OrderType::WalkingBackwardsShield,
            OrderType::StrafingRightShield,
            OrderType::StrafingLeftShield,
        ] {
            scripts.extend(vec![directional_script(action); 16]);
        }
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[OrderType::WalkingShield as usize] = 0;
        conversion[OrderType::WalkingBackwardsShield as usize] = 16;
        conversion[OrderType::StrafingRightShield as usize] = 32;
        conversion[OrderType::StrafingLeftShield as usize] = 48;
        crate::sprite::Sprite::new(
            std::sync::Arc::new(scripts),
            std::sync::Arc::new(conversion),
        )
    }

    fn install_pc_walking_shield(
        engine: &mut EngineInner,
        start: MapPoint,
        destination: MapPoint,
        action_state: ActionState,
    ) -> EntityId {
        let mut pc = make_test_pc(Posture::Upright);
        pc.element_data_mut().sprite = shield_movement_sprite();
        pc.element_data_mut().sprite.position_iface.set_move_box(
            crate::coordinates::MoveBox::from_coords(-4.0, -4.0, 4.0, 4.0),
        );
        pc.element_data_mut()
            .sprite
            .position_iface
            .set_anti_collision_on(false);
        pc.element_data_mut().set_position_map(start);
        pc.element_data_mut().set_direction_instantly(4);
        pc.actor_data_mut().unwrap().action_state = action_state;
        let owner = engine.add_entity(pc);

        let mut movement = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingWithShield,
        );
        movement.orders.push_back(Order::test_new(
            OrderType::WalkingWithShield,
            destination.x,
            destination.y,
        ));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        owner
    }

    fn dispatch_shield_movement(
        owner_is_pc: bool,
        live_action_state: ActionState,
        stamped_action_state: ActionState,
        install_shield_row: bool,
        on_lift: bool,
    ) -> OrderType {
        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(140.0, 100.0);
        let mut element = ElementData {
            kind: if owner_is_pc {
                ElementKind::ActorPc
            } else {
                ElementKind::ActorSoldier
            },
            active: true,
            posture: Posture::Upright,
            sprite: if install_shield_row {
                shield_movement_sprite()
            } else {
                crate::sprite::Sprite::default()
            },
            ..ElementData::default()
        };
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        element.sprite.position_iface.set_anti_collision_on(false);
        element
            .sprite
            .position_iface
            .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
        element.set_position_map(start);
        if on_lift {
            use crate::fast_find_grid::GridSector;
            use crate::sector::{LiftType, SectorNumber, SectorType};

            let sector_number = SectorNumber::new(1);
            element.set_sector(Some(
                crate::position_interface::SectorHandle::new(1).unwrap(),
            ));
            let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
            level.sector_number_map.insert(sector_number, 0);
            level.sectors.push(GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: SectorType::LIFT,
                layer: 0,
                sector_number,
                door_index: None,
                lift_type: Some(LiftType::Stairs),
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            });
        }
        let actor = ActorData {
            action_state: live_action_state,
            ..ActorData::default()
        };
        let owner = if owner_is_pc {
            engine.add_entity(Entity::Pc(ActorPc {
                element,
                actor,
                human: HumanData::default(),
                pc: PcData::default(),
            }))
        } else {
            engine.add_entity(Entity::Soldier(ActorSoldier {
                element,
                actor,
                human: HumanData::default(),
                npc: NpcData::default(),
                soldier: SoldierData::default(),
            }))
        };

        let mut movement =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
        movement.posture_after_transition = Posture::Upright;
        movement.action_state_after_transition = stamped_action_state;
        let SequenceElementData::Movement {
            destination: goal, ..
        } = &mut movement.data
        else {
            unreachable!("new_movement must create movement data")
        };
        *goal = destination;
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        let sim = crate::sim_rng::test_context();
        assert!(matches!(
            engine.try_dispatch_move_path(
                &sim,
                &LevelAssets::new(),
                owner,
                sequence,
                0,
                destination,
                OrderType::WalkingUpright,
            ),
            MovePathOutcome::Success
        ));
        let movement = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("dispatched shield movement must remain registered");
        let SequenceElementData::Movement { action, .. } = &movement.data else {
            panic!("dispatched shield movement changed data kind")
        };
        *action
    }

    #[test]
    fn soldier_live_shield_state_rewrites_new_upright_path_request() {
        assert_eq!(
            dispatch_shield_movement(
                false,
                ActionState::MovingShield,
                ActionState::Waiting,
                false,
                false,
            ),
            OrderType::WalkingWithShield,
            "Actor::DetermineMovementAnimation reads Soldier 61's live shield state, not the older sequence stamp"
        );
    }

    #[test]
    fn pc_stamped_shield_override_still_rewrites_upright_path_request() {
        assert_eq!(
            dispatch_shield_movement(
                true,
                ActionState::Waiting,
                ActionState::HoldingShield,
                false,
                true,
            ),
            OrderType::WalkingWithShield,
            "the PC override owns its stamped shield arm before sprite-row checks and base lift translation"
        );
    }

    #[test]
    fn terminal_pc_walking_with_shield_stamps_holding_shield() {
        let mut engine = EngineInner::new();
        let assets = assets_with_test_pc_profile();
        let owner = install_pc_walking_shield(
            &mut engine,
            MapPoint::new(100.0, 100.0),
            MapPoint::new(100.0, 100.0),
            ActionState::Waiting,
        );

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::HoldingShield
        );
    }

    #[test]
    fn frozen_all_pc_walking_with_shield_still_refreshes_retained_box() {
        let mut engine = EngineInner::new();
        let assets = assets_with_test_pc_profile();
        let mut pc = make_test_pc(Posture::Upright);
        pc.element_data_mut().sprite = shield_movement_sprite();
        pc.element_data_mut()
            .set_position_map(MapPoint::new(100.0, 100.0));
        pc.element_data_mut().set_direction_instantly(4);
        pc.actor_data_mut().unwrap().action_state = ActionState::Waiting;
        let stale = crate::bow_shot::compute_shield_obstacle(
            MapPoint::new(-50.0, 100.0),
            0.0,
            4,
            &crate::bow_shot::shield_params_for_pc(false),
        );
        pc.actor_data_mut().unwrap().shield_obstacle = Some(stale);
        let owner = engine.add_entity(pc);

        let mut movement = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingWithShield,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::WalkingWithShield, 140.0, 100.0));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine.set_actors_frozen(true);

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            entity.element_data().position_map(),
            MapPoint::new(100.0, 100.0)
        );
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::MovingShield,
            "FrozenAll suppresses sprite motion, but not the PC WalkingWithShield Execute state stamp"
        );
        let actual = entity
            .actor_data()
            .unwrap()
            .shield_obstacle
            .as_ref()
            .unwrap();
        let expected = crate::bow_shot::compute_shield_obstacle(
            MapPoint::new(100.0, 100.0),
            0.0,
            4,
            &crate::bow_shot::shield_params_for_pc(false),
        );
        assert_eq!(actual.box_3d_min, expected.box_3d_min);
        assert_eq!(actual.box_3d_max, expected.box_3d_max);
    }

    fn install_blocked_upright_movement(
        action: OrderType,
        initial_action_state: ActionState,
    ) -> (EngineInner, EntityId, crate::sequence::SequenceId) {
        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(140.0, 100.0);
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 10.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 10,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![10],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            sprite: crate::sprite::Sprite::new(
                std::sync::Arc::new(vec![script; 16]),
                std::sync::Arc::new(conversion),
            ),
            ..ElementData::default()
        };
        element.sprite.position_iface.set_anti_collision_on(false);
        element
            .sprite
            .position_iface
            .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
        element.set_position_map(start);
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: initial_action_state,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(1, Command::Move, Some(owner), action);
        movement.priority = SequencePriority::Normal;
        movement
            .orders
            .push_back(Order::new(action, destination.x, destination.y, order_id));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        let owner_entity = engine.get_entity_mut(owner).unwrap();
        owner_entity.actor_data_mut().unwrap().active_movement = ActiveMovement::new(sequence, 0);
        owner_entity
            .element_data_mut()
            .sprite
            .last_processed_order_id = order_id.get();
        let position_iface = owner_entity.position_iface_mut();
        position_iface.set_map_goal(destination);
        position_iface.compute_increment_all(true);
        position_iface.blocked_count = 51;

        (engine, owner, sequence)
    }

    fn assert_blocked_upright_movement_state(
        action: OrderType,
        initial_action_state: ActionState,
        expected_action_state: ActionState,
    ) {
        let (mut engine, owner, sequence) =
            install_blocked_upright_movement(action, initial_action_state);

        engine.tick_entity_movement(
            &crate::sim_rng::test_context(),
            &assets_with_test_pc_profile(),
        );

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            expected_action_state
        );
        assert_eq!(
            entity.actor_data().unwrap().active_movement,
            ActiveMovement::none(),
            "the aborted movement tracker must still detach"
        );
        assert_eq!(entity.position_iface().blocked_count, 0);
        assert!(entity.position_iface().box_blocked.0.is_none());
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            SequenceState::Impossible,
            "Actor::Hourglass must still reject the blocked element"
        );
    }

    #[test]
    fn blocked_walking_upright_retains_moving_state_while_tearing_down_movement() {
        assert_blocked_upright_movement_state(
            OrderType::WalkingUpright,
            ActionState::Moving,
            ActionState::Moving,
        );
    }

    #[test]
    fn blocked_running_upright_still_applies_its_unconditional_moving_fast_state() {
        assert_blocked_upright_movement_state(
            OrderType::RunningUpright,
            ActionState::Waiting,
            ActionState::MovingFast,
        );
    }

    fn install_sword_movement_for_kind(
        force: bool,
        soldier: bool,
    ) -> (
        EngineInner,
        EntityId,
        crate::sequence::SequenceId,
        std::num::NonZeroU32,
        MapPoint,
    ) {
        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(140.0, 100.0);
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position_map(start);

        let walking_script = SpriteScript {
            action_id: OrderType::WalkingSword as u16,
            action_done: 0,
            average_speed: 10.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 10,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![10],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let directional_script = |action: OrderType| SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 17.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 17,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![17],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut scripts = vec![walking_script; 16];
        scripts.extend(vec![
            directional_script(OrderType::WalkingBackwardsSword);
            16
        ]);
        scripts.extend(vec![directional_script(OrderType::StrafingRightSword); 16]);
        scripts.extend(vec![directional_script(OrderType::StrafingLeftSword); 16]);
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[OrderType::WalkingSword as usize] = 0;
        conversion[OrderType::WalkingBackwardsSword as usize] = 16;
        conversion[OrderType::StrafingRightSword as usize] = 32;
        conversion[OrderType::StrafingLeftSword as usize] = 48;
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(scripts),
            std::sync::Arc::new(conversion),
        );
        element.sprite.position_iface.set_anti_collision_on(false);
        element.set_position_map(start);

        let actor = ActorData {
            action_state: ActionState::MovingSword,
            ..ActorData::default()
        };
        let owner = if soldier {
            element.kind = ElementKind::ActorSoldier;
            let mut npc = NpcData::default();
            let mut enemy_ai = crate::ai_enemy::EnemyAi::new(0);
            enemy_ai.hth_weapon_id = 1;
            npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(enemy_ai));
            engine.add_entity(Entity::Soldier(ActorSoldier {
                element,
                actor,
                human: HumanData::default(),
                npc,
                soldier: SoldierData::default(),
            }))
        } else {
            engine.add_entity(Entity::Pc(ActorPc {
                element,
                actor,
                human: HumanData::default(),
                pc: PcData {
                    life_points: 50,
                    ..PcData::default()
                },
            }))
        };

        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingWithSword,
        );
        movement.priority = SequencePriority::Normal;
        movement.orders.push_back(Order::new(
            OrderType::WalkingWithSword,
            destination.x,
            destination.y,
            order_id,
        ));
        let SequenceElementData::Movement { flags, .. } = &mut movement.data else {
            unreachable!("new_movement must create movement data")
        };
        if force {
            *flags |= MoveFlags::FORCE_SWORD_MOVEMENT;
        }
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_movement = ActiveMovement::new(sequence, 0);

        (engine, owner, sequence, order_id, start)
    }

    fn install_sword_movement(
        force: bool,
    ) -> (
        EngineInner,
        EntityId,
        crate::sequence::SequenceId,
        std::num::NonZeroU32,
        MapPoint,
    ) {
        install_sword_movement_for_kind(force, false)
    }

    #[test]
    fn lowered_actor_still_aborts_unrewritten_resumed_sword_move() {
        let (mut engine, owner, movement_sequence, order_id, _start) =
            install_sword_movement(false);
        let movement = engine
            .orders
            .sequence_manager
            .get_element_mut(movement_sequence, 0)
            .unwrap();
        movement.command = Command::MoveOk;
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::Waiting;

        let aborted = engine.abort_orphaned_sword_movement(
            &crate::sim_rng::test_context(),
            &assets_with_test_pc_profile(),
            owner,
            MovementOwnerSelection {
                seq_id: movement_sequence,
                elem_idx: 0,
                order_id,
            },
        );

        assert!(
            aborted,
            "Human::Execute has no non-sword action-state exception for an unrewritten sword order"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .state,
            SequenceState::Impossible,
            "the captured resumed MoveOk must be rejected after Execute returns ABORTED"
        );
    }

    #[test]
    fn evaluate_opponents_rewrite_survives_postpone_as_untranslated_upright_move() {
        let (mut engine, owner, movement_sequence, _order_id, _start) =
            install_sword_movement(false);
        let sim = crate::sim_rng::test_context();
        let assets = assets_with_test_pc_profile();

        engine.evaluate_opponents(&sim, &assets, owner);

        let quit_sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .find_map(|sequence| {
                sequence.elements.first().and_then(|element| {
                    (element.owner == Some(owner) && element.command == Command::QuitSwordfight)
                        .then_some(sequence.id)
                })
            })
            .expect("EvaluateOpponents must register QuitSwordfight");
        engine.engine_postpone(quit_sequence, 0, movement_sequence, 0);

        let movement = engine
            .orders
            .sequence_manager
            .get_element(movement_sequence, 0)
            .expect("rewritten movement must remain registered behind QuitSwordfight");
        let SequenceElementData::Movement { action, .. } = movement.data else {
            panic!("rewritten movement changed data kind")
        };
        assert_eq!(action, OrderType::WalkingUpright);
        assert_eq!(movement.command, Command::Move);
        assert_eq!(movement.state, SequenceState::Postponed);
        assert!(
            movement.orders.is_empty(),
            "Original postponement deletes the old sword order so resume retranslates the rewritten upright action"
        );
    }

    #[test]
    fn blocked_terminal_sword_execute_marks_impossible_without_entering_waiting_sword() {
        let (mut engine, owner, movement_sequence, order_id, _start) = install_sword_movement(true);
        let mut opponent_element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        opponent_element.set_position_map(MapPoint::new(100.0, 50.0));
        let opponent = engine.add_entity(Entity::Pc(ActorPc {
            element: opponent_element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(opponent);

        let owner_entity = engine.get_entity_mut(owner).unwrap();
        // The replay's strafe order was already initialized before its
        // blocked counter crossed the threshold. A fresh fixture order calls
        // Sprite::initialize_motion_order and resets that counter, so install
        // the matching live sprite-order identity and cached trajectory first.
        owner_entity
            .element_data_mut()
            .sprite
            .last_processed_order_id = order_id.get();
        let position_iface = owner_entity.position_iface_mut();
        position_iface.set_map_goal(MapPoint::new(140.0, 100.0));
        position_iface.compute_increment_all(true);
        position_iface.blocked_count = 51;

        engine.tick_entity_movement(
            &crate::sim_rng::test_context(),
            &assets_with_test_pc_profile(),
        );

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::MovingSword,
            "the Human Execute ABORTED arm does not normalize a live sword-motion state"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .sprite
                .last_action,
            OrderType::StrafingLeftSword,
            "the regression drives the same terminal strafe family as Soldier 57"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .state,
            SequenceState::Impossible,
            "Actor::Hourglass must still reject the blocked movement element"
        );
    }

    #[test]
    fn nonforced_sword_movement_without_opponents_aborts_before_motion_and_quits_once() {
        let (mut engine, owner, movement_sequence, order_id, start) = install_sword_movement(false);
        let sim = crate::sim_rng::test_context();
        let assets = assets_with_test_pc_profile();

        engine.tick_entity_movement(&sim, &assets);

        let owner_entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            owner_entity.element_data().position_map(),
            start,
            "the rejected sword movement must not reach PerformMotion"
        );
        assert_ne!(
            owner_entity.element_data().sprite.last_processed_order_id,
            order_id.get(),
            "the rejected order must not initialize the sprite"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .state,
            SequenceState::Impossible
        );
        let quit_elements = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| {
                element.owner == Some(owner) && element.command == Command::QuitSwordfight
            })
            .collect::<Vec<_>>();
        assert_eq!(
            quit_elements.len(),
            1,
            "one rejected Execute invocation must launch exactly one QuitSwordfight"
        );
        assert_eq!(
            quit_elements[0].state,
            SequenceState::Todo,
            "Human::Execute registers QuitSwordfight; the later manager Hourglass owns Actor::Instruct"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner),
            None,
            "the rejected movement is already gone but deferred QuitSwordfight is not selected until manager dispatch"
        );

        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .continuation
            .motion_state = crate::sprite::MotionState::Aborted;
        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .continuation
                .motion_state,
            crate::sprite::MotionState::InProgress,
            "the later accepted Actor::Instruct must overwrite Execute's ABORTED result"
        );
        assert_eq!(
            engine.actor_order_type(owner),
            Some(OrderType::TransitionLoweringSword)
        );
    }

    #[test]
    fn npc_orphaned_sword_movement_stops_linked_turn_before_quit() {
        let (mut engine, owner, movement_sequence, _order_id, _start) =
            install_sword_movement_for_kind(false, true);
        let sim = crate::sim_rng::test_context();
        let assets = assets_with_test_pc_profile();
        engine
            .get_entity_mut(owner)
            .unwrap()
            .position_iface_mut()
            .set_direction_instantly(crate::position_interface::Direction::from_raw(10));

        let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(owner));
        turn.priority = SequencePriority::Normal;
        turn.set_property(
            crate::sequence::Field::Direction,
            crate::sequence::FieldValue::Integer(9),
        );
        let turn_sequence = engine.orders.sequence_manager.launch_element(turn);
        engine
            .orders
            .sequence_manager
            .postpone_element(turn_sequence, 0);
        engine
            .orders
            .sequence_manager
            .get_element_mut(movement_sequence, 0)
            .unwrap()
            .cross_postponed = Some((turn_sequence, 0));

        let unrelated_sequence = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::LookLeft, Some(owner)));

        let ((), cards) = crate::engine::soldier_helpers::capture_condolation_cards(|| {
            engine.tick_entity_movement(&sim, &assets);
        });

        assert_eq!(
            cards
                .iter()
                .filter(|(card_owner, _)| *card_owner == owner)
                .map(|(_, command)| *command)
                .take(2)
                .collect::<Vec<_>>(),
            vec![Command::Move, Command::Turn],
            "Movement::StopMovement must deliver the selected movement card before base Stop reaches the linked Turn"
        );

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(turn_sequence, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted,
            "Stop(Injury) must close the linked Turn before QuitSwordfight is registered"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .cross_postponed,
            None,
            "the interrupted Turn must not remain inheritable by QuitSwordfight"
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .is_registered_to_go(unrelated_sequence, 0),
            "the exact-root stop must preserve unrelated pending owner work"
        );
        let quit = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner) && element.command == Command::QuitSwordfight
            })
            .expect("the orphan guard must register QuitSwordfight");
        assert_eq!(
            quit.cross_postponed, None,
            "QuitSwordfight must not inherit the interrupted Turn"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .get_direction_goal()
                .as_u8(),
            10,
            "an unexecuted direction-9 Turn must not overwrite the live direction goal"
        );
        let events = engine
            .get_entity(owner)
            .unwrap()
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| entry.line_type == crate::ai::LogLineType::Event)
            .map(|entry| entry.info)
            .collect::<Vec<_>>();
        let turn_card = events
            .iter()
            .position(|event| *event == crate::ai::StimulusType::EventDone as u16)
            .expect("interrupting the linked Turn must synchronously send its condolence card");
        let quit_event = events
            .iter()
            .position(|event| *event == crate::ai::StimulusType::EventQuitSwordfight as u16)
            .expect("the orphan guard must notify the soldier brain about quitting");
        assert!(
            turn_card < quit_event,
            "the Turn condolence callback must close before EVENT_QUIT_SWORDFIGHT"
        );
    }

    #[test]
    fn pc_pinch_abort_cancels_terminal_pop_before_impossible() {
        let (mut engine, _owner, movement_sequence, _order_id, _start) =
            install_sword_movement(false);
        let unrelated = crate::sequence::SequenceId(movement_sequence.0 + 1);
        let mut order_pops = vec![(movement_sequence, 0), (unrelated, 0)];

        cancel_aborted_order_pop(&mut order_pops, movement_sequence, 0);
        assert_eq!(order_pops, vec![(unrelated, 0)]);

        engine
            .orders
            .sequence_manager
            .element_impossible(movement_sequence, 0);
        for (seq_id, elem_idx) in order_pops {
            if engine
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some()
            {
                engine.do_next_order(seq_id, elem_idx);
            }
        }
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .state,
            SequenceState::Impossible,
            "the PC Execute ABORTED result must not be overwritten by the nested motion's queued TERMINATED pop"
        );
    }

    #[test]
    fn forced_sword_movement_without_opponents_still_performs_motion() {
        let (mut engine, owner, movement_sequence, order_id, start) = install_sword_movement(true);

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());

        let owner_entity = engine.get_entity(owner).unwrap();
        assert_ne!(
            owner_entity.element_data().position_map(),
            start,
            "FORCE_SWORD_MOVEMENT must retain the movement Execute path"
        );
        assert_eq!(
            owner_entity.element_data().sprite.last_processed_order_id,
            order_id.get()
        );
        assert_eq!(
            owner_entity.element_data().sprite.last_action,
            OrderType::WalkingSword,
            "a non-soldier without opponents takes FaceOpponent's explicit WalkingSword fallback, not a directional row computed from a self-position sentinel"
        );
        assert_eq!(
            owner_entity.element_data().position_map(),
            MapPoint::new(start.x + 6.0, start.y),
            "the WalkingSword frame distance must be used instead of an accidental backward/strafe distance"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .state,
            SequenceState::InProgress
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .all(|element| {
                    element.owner != Some(owner) || element.command != Command::QuitSwordfight
                }),
            "forced movement must not launch QuitSwordfight"
        );
    }

    #[test]
    fn sword_movement_with_colocated_opponent_preserves_zero_facing_vector() {
        let (mut engine, owner, movement_sequence, _order_id, _start) =
            install_sword_movement(true);

        engine
            .orders
            .sequence_manager
            .get_element_mut(movement_sequence, 0)
            .and_then(|element| element.orders.front_mut())
            .expect("test movement keeps its selected order")
            .compute_direction = false;

        {
            let owner_element = engine.get_entity_mut(owner).unwrap().element_data_mut();
            owner_element.set_direction_instantly(7);
            owner_element.set_direction_goal(9);
        }

        let mut opponent_element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        opponent_element.set_position(
            engine
                .get_entity(owner)
                .expect("test owner exists")
                .element_data()
                .position(),
        );
        let opponent = engine.add_entity(Entity::Pc(ActorPc {
            element: opponent_element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                life_points: 50,
                ..PcData::default()
            },
        }));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(opponent);

        engine.tick_entity_movement(
            &crate::sim_rng::test_context(),
            &assets_with_test_pc_profile(),
        );

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .sprite
                .last_action,
            OrderType::WalkingBackwardsSword,
            "FaceOpponent passes a co-located opponent's literal zero vector to Angle, which resolves to PI"
        );
        let owner_element = engine.get_entity(owner).unwrap().element_data();
        assert_eq!(
            owner_element
                .sprite
                .position_iface
                .get_direction_goal()
                .as_u8(),
            0,
            "FaceOpponent passes its literal zero vector through GetSector0to15 and SetDirection"
        );
        assert_eq!(
            owner_element.direction(),
            6,
            "Turn must rotate one step from direction 7 toward the zero-vector goal 0"
        );
    }

    #[test]
    fn combat_seek_applies_face_opponent_and_perform_seek_turns() {
        let (mut engine, owner, movement_sequence, _order_id, start) = install_sword_movement(true);
        let (face_x, face_y) = crate::element::direction_vector_16(11);
        let opponent_position = MapPoint::new(
            start.x + face_x * 30.0,
            start.y + face_y * crate::position_interface::ASPECT_RATIO * 30.0,
        );
        assert_eq!(
            crate::position_interface::vector_to_sector_0_to_15_iso(
                opponent_position.x - start.x,
                opponent_position.y - start.y,
            ),
            11,
        );

        let mut opponent_element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        opponent_element.set_position_map(opponent_position);
        let opponent = engine.add_entity(Entity::Pc(ActorPc {
            element: opponent_element,
            actor: ActorData::default(),
            human: HumanData {
                opponents: vec![owner].into(),
                ..HumanData::default()
            },
            pc: PcData {
                life_points: 50,
                ..PcData::default()
            },
        }));

        {
            let entity = engine.get_entity_mut(owner).unwrap();
            entity.human_data_mut().unwrap().opponents.push(opponent);
            let actor = entity.actor_data_mut().unwrap();
            actor.seek_target = Some(opponent);
            actor.last_seek_target_position = opponent_position;
            actor.seek_distance = 0.0;
            let position = entity.position_iface_mut();
            let mut state = position.v48_serialized_state();
            state.direction = crate::position_interface::Direction::from_raw(10);
            state.direction_goal = crate::position_interface::Direction::from_raw(11);
            state.anti_collision_on = false;
            state.deviated = true;
            state.direction_count = 0;
            position.restore_v48_serialized_state(state);
        }
        let movement = engine
            .orders
            .sequence_manager
            .get_element_mut(movement_sequence, 0)
            .unwrap();
        let SequenceElementData::Movement { flags, element, .. } = &mut movement.data else {
            unreachable!("fixture movement changed data kind")
        };
        flags.insert(MoveFlags::SEEK);
        *element = Some(opponent);

        let sim = crate::sim_rng::test_context();
        let assets = assets_with_test_pc_profile();
        engine.tick_entity_movement(&sim, &assets);

        let first = engine.get_entity(owner).unwrap().position_iface();
        assert_eq!(first.get_direction().as_u8(), 10);
        assert_eq!(first.v48_serialized_state().direction_count, 2);

        engine.tick_entity_movement(&sim, &assets);

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .get_direction()
                .as_u8(),
            11,
            "FaceOpponent's first Turn must rotate once the prior frame's two-call anti-vibration count is stable"
        );
    }

    #[test]
    fn postponed_forced_move_resuming_after_sword_lowered_walks_upright() {
        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(140.0, 100.0);
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        element.sprite.position_iface.set_anti_collision_on(false);
        element
            .sprite
            .position_iface
            .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
        element.set_position_map(start);
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                // QuitSwordfight's lowering animation has completed before
                // the postponed Move is instructed again.
                action_state: ActionState::Waiting,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData {
                life_points: 50,
                ..PcData::default()
            },
        }));

        let mut movement = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingWithSword,
        );
        movement.priority = SequencePriority::Normal;
        movement.posture_after_transition = Posture::Upright;
        movement.action_state_after_transition = ActionState::Waiting;
        let SequenceElementData::Movement {
            destination: stored_destination,
            flags,
            ..
        } = &mut movement.data
        else {
            unreachable!("new_movement must create movement data")
        };
        *stored_destination = destination;
        *flags |= MoveFlags::FORCE_SWORD_MOVEMENT;
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        let sim = crate::sim_rng::test_context();

        assert!(matches!(
            engine.try_dispatch_move_path(
                &sim,
                &LevelAssets::new(),
                owner,
                sequence,
                0,
                destination,
                OrderType::WalkingWithSword,
            ),
            MovePathOutcome::Success
        ));

        let movement = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("dispatched postponed movement must remain registered");
        let SequenceElementData::Movement { action, .. } = &movement.data else {
            panic!("dispatched movement changed data kind")
        };
        assert_eq!(*action, OrderType::WalkingUpright);
        assert_eq!(
            movement.orders.front().map(|order| order.order_type),
            Some(OrderType::TransitionWaitingUprightWalkingUpright)
        );
        assert_eq!(
            movement.action_state_after_transition,
            ActionState::Waiting,
            "FORCE is an Execute guard, not a translated action-state override"
        );
        assert!(matches!(
            &movement.data,
            SequenceElementData::Movement { flags, .. }
                if flags.contains(MoveFlags::FORCE_SWORD_MOVEMENT)
        ));
        assert!(
            !is_sword_motion_context(
                ActionState::Waiting,
                None,
                OrderType::TransitionWaitingUprightWalkingUpright,
            ),
            "FORCE on the owning element must not reroute an ordinary transition through FaceOpponent"
        );
    }

    #[test]
    fn ordinary_successor_does_not_inherit_sword_movement_start_side_effects() {
        assert!(
            !is_sword_motion_context(
                ActionState::MovingSword,
                Some(OrderType::WalkingUpright),
                OrderType::WalkingUpright,
            ),
            "a concrete ordinary door successor must not re-enter Human's sword-facing Execute arm"
        );
        assert!(
            !executes_sword_movement_action(
                Some(OrderType::WalkingUpright),
                OrderType::WalkingUpright,
            ),
            "an ordinary door successor must execute the ordinary walking START arm"
        );
        assert!(executes_sword_movement_action(
            Some(OrderType::WalkingWithSword),
            OrderType::WalkingWithSword,
        ));
        assert!(executes_sword_movement_action(
            None,
            OrderType::RunningWithSword,
        ));
    }
}

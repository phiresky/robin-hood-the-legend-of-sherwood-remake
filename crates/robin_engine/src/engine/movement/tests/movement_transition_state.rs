#[cfg(test)]
mod suite {
    use super::super::*;
    use crate::element::{
        ActionState, ActiveDoorPass, ActorData, ActorPc, ActorSoldier, AiBrain, Camp, Command,
        ElementData, ElementKind, Entity, HumanData, NpcData, PcData, Posture, SoldierData,
    };
    use crate::order::{AiOrderIntent, Order};
    use crate::sequence::{
        MoveFlags, Sequence, SequenceElement, SequenceElementData, SequencePriority, SequenceState,
    };
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    #[test]
    fn perform_seek_registers_enter_swordfight_before_outer_terminal_provoke() {
        // Nescafe/Profile001+003/Savegame000/replay021: the PC's sword
        // movement reaches its seek target while carrying an EnterSwordfight
        // post-seek sequence. Original StartPostSeekSequence runs inside
        // PerformSeek, so EnterSwordfight reaches the manager FIFO before the
        // surrounding Human Execute arm registers Provoke.
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        let target = engine.add_entity(Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        }));

        let mut post_seek = Sequence::new();
        let mut enter = SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
        enter.set_property(
            crate::sequence::Field::Opponent,
            crate::sequence::FieldValue::Element(target),
        );
        post_seek.append_element(enter);
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.seek_target = Some(target);
            actor.post_seek_sequence = Some(post_seek.into_post_seek());
        }

        let movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingWithSword,
        );
        let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
        let registered = engine.orders.sequence_manager.hourglass();
        assert_eq!(registered.len(), 1);
        engine
            .orders
            .sequence_manager
            .element_in_progress(movement_sequence, 0);

        let reentrant =
            engine.launch_perform_seek_arrivals(&sim, &assets, vec![(owner, movement_sequence, 0)]);
        assert_eq!(reentrant, vec![owner]);
        engine.launch_sword_movement_termination_provoke(owner);

        let commands = engine
            .orders
            .sequence_manager
            .v48_elements_to_go()
            .into_iter()
            .filter_map(|(sequence_id, element_index)| {
                engine
                    .orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
            })
            .filter(|element| element.owner == Some(owner))
            .map(|element| element.command)
            .collect::<Vec<_>>();
        assert_eq!(commands, vec![Command::EnterSwordfight, Command::Provoke]);
    }

    #[test]
    fn map_exit_move_bypasses_ordinary_level_bounds_preflight() {
        fn make_owner(engine: &mut EngineInner) -> EntityId {
            let mut element = ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            };
            element.set_position_map(MapPoint::new(90.0, 90.0));
            let npc = NpcData {
                ai: crate::element::AiActorData {
                    ai_brain: AiBrain::Enemy(Box::default()),
                    ..Default::default()
                },
                ..Default::default()
            };
            engine.add_entity(Entity::Soldier(ActorSoldier {
                element,
                actor: ActorData::default(),
                human: HumanData::default(),
                npc,
                soldier: SoldierData::default(),
            }))
        }

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        let destination = crate::ai::Position {
            x: 100.0,
            y: 130.0,
            sector: crate::position_interface::SectorHandle::new(0),
            level: 0,
        };

        let mut map_exit = EngineInner::new();
        map_exit.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(100.0, 100.0);
        let map_owner = make_owner(&mut map_exit);
        map_exit
            .get_entity_mut(map_owner)
            .unwrap()
            .ai_controller_mut()
            .unwrap()
            .run_to_map_exit(destination);
        map_exit.launch_pending_orders_for_npc_mode(&sim, &assets, map_owner, false);
        let launched = map_exit.drain_pending_move_requests_for_owner(&sim, map_owner);
        assert_eq!(launched.len(), 1, "RHMOVE_MAP must launch through PointOut");
        let element = map_exit
            .orders
            .sequence_manager
            .get_element(launched[0], 0)
            .expect("map-exit movement must remain registered for manager Hourglass");
        let SequenceElementData::Movement {
            destination: actual,
            flags,
            ..
        } = &element.data
        else {
            panic!("map-exit sequence must contain movement")
        };
        assert_eq!(*actual, MapPoint::new(destination.x, destination.y));
        assert!(flags.contains(MoveFlags::MAP));
        assert!(
            !map_exit
                .get_entity(map_owner)
                .unwrap()
                .ai_controller()
                .unwrap()
                .couldnt_reachpoint
        );

        for ordinary_destination in [MapPoint::new(100.0, 90.0), MapPoint::new(90.0, 130.0)] {
            let mut ordinary = EngineInner::new();
            ordinary.feedback.cutscene_camera.level_size =
                crate::coordinates::MapSize::new(100.0, 100.0);
            let ordinary_owner = make_owner(&mut ordinary);
            ordinary
                .get_entity_mut(ordinary_owner)
                .unwrap()
                .ai_controller_mut()
                .unwrap()
                .outbox
                .actor
                .orders
                .push(AiOrderIntent::new(
                    OrderType::RunningUpright,
                    ordinary_destination.x,
                    ordinary_destination.y,
                ));
            ordinary.launch_pending_orders_for_npc_mode(&sim, &assets, ordinary_owner, false);
            assert!(
                ordinary
                    .drain_pending_move_requests_for_owner(&sim, ordinary_owner)
                    .is_empty(),
                "ordinary GoTo at or outside the level must retain the existing rejection"
            );
            assert!(
                ordinary
                    .get_entity(ordinary_owner)
                    .unwrap()
                    .ai_controller()
                    .unwrap()
                    .couldnt_reachpoint
            );
        }
    }

    fn run_stale_sword_crenel_transition() -> (u8, u8) {
        use crate::fast_find_grid::GridSector;
        use crate::gate::{Door, DoorIndex, DoorType};
        use crate::sector::{LiftType, SectorNumber, SectorType};

        let mut engine = EngineInner::new();
        let transition = OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel;
        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 7,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1; 8],
            delays: vec![0; 8],
            distances: vec![0; 8],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 8],
            sound_ids: vec![0; 8],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[transition as usize] = 0;
        let start = MapPoint::new(100.0, 100.0);
        let goal = MapPoint::new(108.0, 106.0);
        let lift_sector = SectorNumber::new(7);

        let mut opponent = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        opponent
            .element_data_mut()
            .set_position_map(MapPoint::new(120.0, 90.0));
        let opponent = engine.add_entity(opponent);

        let mut owner_entity = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData {
                action_state: ActionState::MovingSword,
                execute_order_initialising: true,
                active_door_pass: Some(ActiveDoorPass {
                    door_index: DoorIndex::new(0).expect("valid door index"),
                    direct: true,
                    position_direct: true,
                    steps: Default::default(),
                    triggers_fired: 0,
                    current_action: transition,
                    current_reverse: false,
                    saved_action_state: None,
                }),
                ..ActorData::default()
            },
            human: HumanData {
                opponents: vec![opponent].into(),
                ..HumanData::default()
            },
            pc: PcData::default(),
        });
        owner_entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        owner_entity.element_data_mut().set_position_map(start);
        owner_entity.element_data_mut().set_direction_instantly(2);
        owner_entity
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(7));
        owner_entity
            .position_iface_mut()
            .set_anti_collision_on(false);
        let owner = engine.add_entity(owner_entity);

        {
            let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
            level.sector_number_map.insert(lift_sector, 0);
            level.sectors.push(GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: SectorType::LIFT,
                layer: 0,
                sector_number: lift_sector,
                door_index: Some(0),
                lift_type: Some(LiftType::Wall),
                lift_direction: 15,
                force_crouched: false,
                building_index: None,
                low_exit_point: Some(start),
                high_exit_point: Some(goal),
                lowest_door_index: Some(0),
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            });
        }
        engine.script_domains.interactables.doors.push(Door {
            door_type: DoorType::LiftHighCrenel,
            sector_in: lift_sector,
            point_in: goal,
            ..Door::default()
        });

        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(owner),
            OrderType::WalkingWithSword,
        );
        movement
            .orders
            .push_back(Order::new(transition, goal.x, goal.y, order_id));
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

        let _ = engine.tick_entity_movement_owner(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            Some(MovementOwnerSelection {
                seq_id: sequence,
                elem_idx: 0,
                order_id,
            }),
        );
        let pi = engine.get_entity(owner).unwrap().position_iface();
        (pi.get_direction().as_u8(), pi.get_direction_goal().as_u8())
    }

    #[test]
    fn crenel_transition_does_not_inherit_stale_sword_facing_context() {
        assert!(matches!(
            ActionState::MovingSword,
            ActionState::MovingSword | ActionState::MovingFastSword
        ));
        assert!(!order_uses_distance_motion(
            OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
        ));
        assert!(
            climb_lift_type(OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel).is_some()
        );
        assert!(
            !is_sword_motion_context(
                ActionState::MovingSword,
                Some(OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel),
                OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel,
            ),
            "the base Actor crenel transition must retain its lift-facing goal instead of entering Human::FaceOpponent"
        );
        let (direction, goal) = run_stale_sword_crenel_transition();
        assert_eq!(
            direction, 3,
            "the trace-shaped Execute must turn toward the climb goal, from direction 2 to 3"
        );
        assert_ne!(
            goal, 1,
            "the opponent-facing writer selected by the old classification must not replace the climb trajectory"
        );
    }

    #[test]
    fn door_distance_successor_dispatches_its_literal_non_sword_action() {
        assert!(order_uses_distance_motion(OrderType::WalkingUpright));
        assert!(
            !is_sword_motion_context(
                ActionState::MovingSword,
                Some(OrderType::WalkingUpright),
                OrderType::WalkingUpright,
            ),
            "PassDoor's concrete WalkingUpright order enters base Actor::Execute"
        );
        assert!(
            is_sword_motion_context(ActionState::MovingSword, None, OrderType::WalkingUpright,),
            "ordinary movement still receives the translation-time sword rewrite"
        );
        assert!(is_sword_motion_context(
            ActionState::Waiting,
            None,
            OrderType::WalkingWithSword,
        ));
        assert!(!is_sword_motion_context(
            ActionState::MovingSword,
            None,
            OrderType::ClimbingWallDown,
        ));
        assert!(!is_sword_motion_context(
            ActionState::MovingSword,
            None,
            OrderType::WalkingCrouched,
        ));
    }

    #[test]
    fn stale_sword_state_does_not_face_opponent_before_plain_door_walk() {
        use crate::gate::{Door, DoorIndex};

        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(108.0, 106.0);
        let action = OrderType::WalkingUpright;
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 1.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 16,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![1; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;

        // The opponent lies in sector 0. The translated door step, however,
        // inherited sector 15 from the preceding door route and must execute
        // Actor's literal WALKING_UPRIGHT arm before PerformMotion replaces
        // the trajectory cache for the new destination.
        let mut opponent = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        opponent
            .element_data_mut()
            .set_position_map(MapPoint::new(100.0, 80.0));
        let opponent = engine.add_entity(opponent);

        let mut owner_entity = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData {
                action_state: ActionState::MovingSword,
                execute_order_initialising: true,
                active_door_pass: Some(ActiveDoorPass {
                    door_index: DoorIndex::new(0).expect("valid door index"),
                    direct: true,
                    position_direct: true,
                    steps: Default::default(),
                    triggers_fired: 0,
                    current_action: action,
                    current_reverse: false,
                    saved_action_state: None,
                }),
                ..ActorData::default()
            },
            human: HumanData {
                opponents: vec![opponent].into(),
                ..HumanData::default()
            },
            pc: PcData::default(),
        });
        owner_entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        owner_entity.element_data_mut().set_position_map(start);
        owner_entity.element_data_mut().set_direction_instantly(0);
        owner_entity.element_data_mut().set_direction_goal(15);
        owner_entity
            .position_iface_mut()
            .set_anti_collision_on(false);
        let owner = engine.add_entity(owner_entity);
        engine
            .script_domains
            .interactables
            .doors
            .push(Door::default());

        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(1, Command::PassDoor, Some(owner), action);
        let mut order = Order::new(action, destination.x, destination.y, order_id);
        order.compute_direction = false;
        movement.orders.push_back(order);
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

        let _ = engine.tick_entity_movement_owner(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            Some(MovementOwnerSelection {
                seq_id: sequence,
                elem_idx: 0,
                order_id,
            }),
        );
        let sprite = &engine.get_entity(owner).unwrap().element_data().sprite;
        assert_eq!(sprite.position_iface.get_direction().as_u8(), 15);
        assert_eq!(sprite.current_row, 15);
    }

    #[test]
    fn same_action_transition_arrival_applies_terminated_state_before_advancing() {
        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let first_destination = MapPoint::new(101.5, 100.0);
        let second_destination = MapPoint::new(110.0, 100.0);
        let transition = OrderType::TransitionWaitingUprightRunningUpright;

        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 2,
            average_speed: 1.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 4,
            frame_ids: vec![1, 2, 3, 4],
            delays: vec![0; 4],
            distances: vec![1; 4],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 4],
            sound_ids: vec![0; 4],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[transition as usize] = 0;

        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
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
                action_state: ActionState::Waiting,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let first_order_id = engine.orders.allocate_order_id();
        let second_order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        movement.priority = SequencePriority::Normal;
        movement.orders.push_back(Order::new(
            transition,
            first_destination.x,
            first_destination.y,
            first_order_id,
        ));
        movement.orders.push_back(Order::new(
            transition,
            second_destination.x,
            second_destination.y,
            second_order_id,
        ));
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

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());
        let first_tick_forecast = engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .get_forecasted_movement();
        assert_ne!(
            first_tick_forecast,
            crate::coordinates::WorldVec3D::ZERO,
            "a moving startup transition must restore the forecast that its action change reset"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::Waiting,
            "a nonterminal startup-transition tick must retain Waiting"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_id,
            first_order_id
        );

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::Waiting,
            "the transition remains nonterminal while its turning slowdown leaves the goal ahead"
        );

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::MovingFast,
            "arrival changed InProgress to Terminated after PerformMotion, so the transition Execute side effect must observe the final state"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_id,
            second_order_id,
            "the terminated transition must advance to its same-action successor"
        );
    }

    #[test]
    fn exhausted_stop_transition_clears_goal_before_queued_point_goto_promotion() {
        let mut engine = EngineInner::new();
        let old_goal = MapPoint::new(263.0, 794.0);
        let new_goal = MapPoint::new(363.0, 794.0);
        let stop_transition = OrderType::TransitionRunningUprightWaitingUpright;
        let start_transition = OrderType::TransitionWaitingUprightRunningUpright;
        let script = |action: OrderType| SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![0; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
        };
        let moving_script = SpriteScript {
            action_id: OrderType::RunningUpright as u16,
            action_done: 1,
            average_speed: 1.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 2,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![1; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
        };
        let mut scripts = Vec::with_capacity(48);
        scripts.extend(vec![script(stop_transition); 16]);
        scripts.extend(vec![script(start_transition); 16]);
        scripts.extend(vec![moving_script; 16]);
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[stop_transition as usize] = 0;
        conversion[start_transition as usize] = 16;
        conversion[OrderType::RunningUpright as usize] = 32;

        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(scripts),
            std::sync::Arc::new(conversion),
        );
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
        element.set_position_map(MapPoint::new(300.0, 794.0));
        let npc = NpcData {
            ai: crate::element::AiActorData {
                ai_brain: AiBrain::Enemy(Box::default()),
                ..Default::default()
            },
            ..Default::default()
        };
        let owner = engine.add_entity(Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData {
                action_state: ActionState::MovingFast,
                ..ActorData::default()
            },
            human: HumanData::default(),
            npc,
            soldier: SoldierData::default(),
        }));

        let mut outgoing = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        outgoing.priority = SequencePriority::Normal;
        outgoing.orders.push_back(Order::new(
            OrderType::RunningUpright,
            old_goal.x,
            old_goal.y,
            engine.orders.allocate_order_id(),
        ));
        let outgoing_sequence = engine.orders.sequence_manager.launch_element(outgoing);
        engine
            .orders
            .sequence_manager
            .element_in_progress(outgoing_sequence, 0);
        {
            let entity = engine.get_entity_mut(owner).unwrap();
            entity.actor_data_mut().unwrap().active_movement =
                ActiveMovement::new(outgoing_sequence, 0);
            entity.position_iface_mut().set_map_goal(old_goal);
            entity.ai_controller_mut().unwrap().outbox.actor.halt = true;
        }

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        engine.launch_pending_orders_for_npc_mode(&sim, &assets, owner, false);

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .map_goal(),
            old_goal,
            "StopAll retains the selected movement goal while its stop transition is live"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(outgoing_sequence, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_type,
            stop_transition
        );
        // Exercise the source-relevant postponed-successor boundary explicitly:
        // once StopAll has built its live StopMovement transition, keep that
        // transition authoritative while the ordinary point GoTo is instructed.
        // The terminal DoNextOrder below must therefore release the GoTo
        // synchronously, rather than letting the fixture interrupt the stop
        // transition before there is an exhaustion boundary to test.
        engine
            .orders
            .sequence_manager
            .get_element_mut(outgoing_sequence, 0)
            .unwrap()
            .priority = SequencePriority::Script;

        {
            let ai = engine
                .get_entity_mut(owner)
                .unwrap()
                .ai_controller_mut()
                .unwrap();
            let mut goto = AiOrderIntent::new(OrderType::RunningUpright, new_goal.x, new_goal.y);
            goto.move_flags = MoveFlags::STRAIGHT.bits() as u16;
            ai.outbox.actor.orders.push(goto);
        }
        engine.launch_pending_orders_for_npc_mode(&sim, &assets, owner, false);
        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        // `engine_postpone` intentionally drops the ordinary handoff cache;
        // model the later queue-time replacement snapshot that exposed the
        // replay bug only after the real GoTo has passed through Instruct and
        // become the outgoing transition's postponed successor.
        let outgoing_element_id = engine
            .orders
            .sequence_manager
            .get_element(outgoing_sequence, 0)
            .unwrap()
            .id;
        let replacement_handle = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner)
                    && element.id != outgoing_element_id
                    && element.data.is_movement()
            })
            .map(|element| element.id)
            .expect("queued point GoTo must register a replacement movement");
        let (replacement_sequence, replacement_index) = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| {
                sequence
                    .elements
                    .iter()
                    .enumerate()
                    .map(move |(index, element)| (sequence.id, index, element.id))
            })
            .find_map(|(sequence, index, id)| {
                (id == replacement_handle).then_some((sequence, index))
            })
            .expect("replacement movement handle must remain registered");
        engine
            .orders
            .sequence_manager
            .get_element_mut(replacement_sequence, replacement_index)
            .unwrap()
            .retained_movement_goal = Some(old_goal);

        let replacement = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner)
                    && element.id
                        != engine
                            .orders
                            .sequence_manager
                            .get_element(outgoing_sequence, 0)
                            .unwrap()
                            .id
                    && element.data.is_movement()
            })
            .expect("queued point GoTo must register a replacement movement");
        assert_eq!(
            replacement.retained_movement_goal,
            Some(old_goal),
            "the regression must carry the stale snapshot that could resurrect the exhausted goal"
        );
        assert_eq!(
            replacement.state,
            crate::sequence::SequenceState::Postponed,
            "the queued point GoTo must wait behind the live StopAll transition"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner),
            Some((outgoing_sequence, 0)),
            "the stop transition must still own the actor before its terminal tick"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .map_goal(),
            old_goal,
            "queuing the point GoTo does not end the live stop transition"
        );

        engine.tick_entity_movement(&sim, &assets);
        engine.tick_entity_movement(&sim, &assets);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(outgoing_sequence, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::Terminated
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .map_goal(),
            MapPoint::ZERO,
            "the selected outgoing Move condolence must remain observable on the replacement-promotion frame"
        );

        engine.hourglass_phase_sequences(&sim, &mut display, &assets);
        // Consume the replacement's authored start transition. Stop at the
        // exact boundary where its point-movement order is selected but has
        // not yet executed; the following tick is its first Execute.
        for _ in 0..4 {
            let point_move_selected = engine
                .orders
                .sequence_manager
                .get_element(replacement_sequence, replacement_index)
                .and_then(|element| element.current_order())
                .is_some_and(|order| {
                    order.order_type == OrderType::RunningUpright
                        && (order.target_x - new_goal.x).abs() <= 0.02
                        && (order.target_y - new_goal.y).abs() <= 0.02
                });
            if point_move_selected {
                break;
            }
            engine.tick_entity_movement(&sim, &assets);
        }
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(replacement_sequence, replacement_index)
                .unwrap()
                .current_order()
                .unwrap()
                .order_type,
            OrderType::RunningUpright,
            "fixture must reach the selected point-movement order before its first Execute"
        );
        let selected_point_order = engine
            .orders
            .sequence_manager
            .get_element(replacement_sequence, replacement_index)
            .unwrap()
            .current_order()
            .unwrap();
        assert!(
            (selected_point_order.target_x - new_goal.x).abs() <= 0.02
                && (selected_point_order.target_y - new_goal.y).abs() <= 0.02,
            "fixture must select the authored destination endpoint, not the source-side transition-distance continuation"
        );
        engine.tick_entity_movement(&sim, &assets);
        let installed_goal = engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal();
        assert!(
            (installed_goal.x - new_goal.x).abs() <= 0.02
                && (installed_goal.y - new_goal.y).abs() <= 0.02,
            "the promoted point GoTo installs its destination on first Execute"
        );
    }

    #[test]
    fn stale_nonselected_final_pop_does_not_clear_live_replacement_goal() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        let stale_goal = MapPoint::new(263.0, 794.0);
        let replacement_goal = MapPoint::new(363.0, 794.0);

        let mut stale = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        stale.orders.push_back(Order::test_new(
            OrderType::RunningUpright,
            stale_goal.x,
            stale_goal.y,
        ));
        let stale_sequence = engine.orders.sequence_manager.launch_element(stale);
        engine
            .orders
            .sequence_manager
            .element_in_progress(stale_sequence, 0);

        let mut replacement = SequenceElement::new_movement(
            1,
            Command::MoveWaiting,
            Some(owner),
            OrderType::RunningUpright,
        );
        replacement.retained_movement_goal = Some(replacement_goal);
        replacement.orders.push_back(Order::test_new(
            OrderType::Freezing,
            replacement_goal.x,
            replacement_goal.y,
        ));
        let replacement_sequence = engine.orders.sequence_manager.launch_element(replacement);
        engine
            .orders
            .sequence_manager
            .set_translating_element(Some((
                owner,
                crate::sequence::SequenceElementRef::new(replacement_sequence, 0),
            )));
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner),
            Some((replacement_sequence, 0)),
            "control requires the replacement to be authoritative before the stale pop drains"
        );
        engine
            .get_entity_mut(owner)
            .unwrap()
            .position_iface_mut()
            .set_map_goal(replacement_goal);
        let stale_orders_before_pop = engine
            .orders
            .sequence_manager
            .get_element(stale_sequence, 0)
            .unwrap()
            .orders
            .len();
        assert_eq!(
            stale_orders_before_pop, 1,
            "control requires a live final stale order for the queued pop"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(stale_sequence, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::InProgress,
            "control requires a stale InProgress owner, not prior terminal teardown"
        );

        engine.pop_selected_movement_order(stale_sequence, 0);
        engine.orders.sequence_manager.set_translating_element(None);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(stale_sequence, 0)
                .unwrap()
                .orders
                .len(),
            stale_orders_before_pop,
            "the stale queued pop must not perform any further order teardown after replacement selection"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(replacement_sequence, 0)
                .unwrap()
                .retained_movement_goal,
            Some(replacement_goal),
            "a stale pop must not erase the authoritative replacement's retained goal"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .map_goal(),
            replacement_goal,
            "a stale pop must leave the authoritative replacement goal untouched"
        );
    }

    #[test]
    fn terminal_movement_handoff_advances_live_move_waiting_order_without_seek_metadata() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Crouched,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        let mut outgoing = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingCrouched,
        );
        outgoing.orders.push_back(Order::test_new(
            OrderType::TransitionWalkingCrouchedWaitingCrouched,
            867.70776,
            2471.1958,
        ));
        let outgoing_sequence = engine.orders.sequence_manager.launch_element(outgoing);
        engine
            .orders
            .sequence_manager
            .element_in_progress(outgoing_sequence, 0);
        let mut waiting = SequenceElement::new_movement(
            1,
            Command::MoveWaiting,
            Some(owner),
            OrderType::WalkingCrouched,
        );
        waiting
            .orders
            .push_back(Order::test_new(OrderType::Freezing, 867.70776, 2471.1958));
        let sequence = engine.orders.sequence_manager.launch_element(waiting);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        let mut completed_parallel =
            SequenceElement::new(2, Command::SpeakHeroReachDestination, Some(owner));
        completed_parallel.state = SequenceState::Terminated;
        engine
            .orders
            .sequence_manager
            .get_sequence_mut(sequence)
            .unwrap()
            .elements
            .push(completed_parallel);
        engine
            .orders
            .sequence_manager
            .set_translating_element(Some((
                owner,
                crate::sequence::SequenceElementRef::new(sequence, 0),
            )));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .installed_order = Some(crate::element::InstalledActorOrder {
            order_id: engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_id,
            order_type: OrderType::Freezing,
        });
        engine
            .orders
            .pending_path_requests
            .enqueue(PendingPathRequest::test_request(owner, sequence, 0));

        assert!(engine.live_pending_freezing_order(owner));
        assert!(
            !engine.live_move_has_completed_parallel_element(owner),
            "a completed later-level element must not consume an ordinary postponed Move"
        );
        engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, 1)
            .unwrap()
            .command_level = 1;
        assert!(engine.live_move_has_completed_parallel_element(owner));
        engine.advance_live_order_after_terminal_handoff(owner);
        engine.orders.sequence_manager.set_translating_element(None);

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("terminated movement replacement remains inspectable");
        assert!(element.orders.is_empty());
        assert_eq!(element.state, SequenceState::Terminated);
        assert!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .installed_order
                .is_none(),
            "the actor's live Freezing order is the one Hourglass advances"
        );
        assert!(
            engine.orders.pending_path_requests.ignore_next_path,
            "terminating the live MoveWaiting must retain and invalidate its pathfinder head"
        );
        assert_eq!(
            engine.orders.pending_path_requests.waiting.len(),
            1,
            "Original CancelPathRequest retains the logical head until its completion slot"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(outgoing_sequence, 0)
                .unwrap()
                .orders
                .len(),
            1,
            "the captured outgoing order remains stale; Hourglass advances the live replacement"
        );
    }

    #[test]
    fn terminal_group_move_handoff_finds_completed_outgoing_sibling() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Crouched,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let mut outgoing = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingCrouched,
        );
        outgoing.state = SequenceState::Terminated;
        let outgoing_sequence = engine.orders.sequence_manager.launch_element(outgoing);
        let mut completed_sibling =
            SequenceElement::new(2, Command::SpeakHeroReachDestination, Some(owner));
        completed_sibling.state = SequenceState::Terminated;
        engine
            .orders
            .sequence_manager
            .get_sequence_mut(outgoing_sequence)
            .unwrap()
            .elements
            .push(completed_sibling);

        let mut replacement = SequenceElement::new_movement(
            1,
            Command::MoveWaiting,
            Some(owner),
            OrderType::WalkingCrouched,
        );
        replacement.state = SequenceState::InProgress;
        replacement
            .orders
            .push_back(Order::test_new(OrderType::Freezing, 867.70776, 2471.1958));
        let replacement_sequence = engine.orders.sequence_manager.launch_element(replacement);
        engine
            .orders
            .sequence_manager
            .set_translating_element(Some((
                owner,
                crate::sequence::SequenceElementRef::new(replacement_sequence, 0),
            )));

        assert!(engine.recent_terminal_move_has_completed_sibling(owner));
    }

    #[test]
    fn terminal_pc_stop_transition_keeps_mouse_orientation_goal() {
        let mut engine = EngineInner::new();
        let transition = OrderType::TransitionWalkingUprightWaitingUpright;
        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![0; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[transition as usize] = 0;

        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        element.sprite.position_iface.set_anti_collision_on(false);
        element.set_position_map(MapPoint::new(100.0, 100.0));
        element.set_direction_instantly(6);
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: ActionState::Waiting,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingUpright,
        );
        movement.priority = SequencePriority::Normal;
        movement.orders.clear();
        let order_id = engine.orders.allocate_order_id();
        movement
            .orders
            .push_back(Order::new(transition, 500.0, 428.0, order_id));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        let registered = engine.orders.sequence_manager.hourglass();
        assert_eq!(registered.len(), 1);
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

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        engine.tick_entity_movement(&sim, &assets);
        assert_eq!(
            i16::from(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .position_iface()
                    .get_direction_goal()
            ),
            6,
            "the stop transition must initially own the outgoing movement direction"
        );

        // PerformOrientation runs before the next engine frame and turns once;
        // the transition's Execute performs the second turn before terminating.
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.element_data_mut().set_direction_goal(4);
        entity.position_iface_mut().turn();
        engine.tick_entity_movement(&sim, &assets);

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(i16::from(entity.position_iface().get_direction()), 4);
        assert_eq!(
            i16::from(entity.position_iface().get_direction_goal()),
            4,
            "terminal Move cleanup must not resurrect the outgoing movement facing"
        );
        assert_eq!(entity.position_iface().map_goal(), MapPoint::ZERO);
    }

    #[test]
    fn new_terminal_pc_stop_transition_replaces_stale_direction_goal() {
        let mut engine = EngineInner::new();
        let transition = OrderType::TransitionWalkingUprightWaitingUpright;
        let destination = MapPoint::new(101.0, 104.0);
        assert_eq!(
            vector_to_sector_0_to_15(destination.x - 100.0, destination.y - 100.0),
            7,
            "fixture movement vector must reproduce QuickSave r011's direction goal"
        );
        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[transition as usize] = 0;

        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        element.sprite.position_iface.set_anti_collision_on(false);
        element.set_position_map(MapPoint::new(100.0, 100.0));
        element.sprite.position_iface.set_map_goal(destination);
        element.sprite.position_iface.compute_increment_all(true);
        element.set_direction_goal(0);
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: ActionState::Waiting,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingUpright,
        );
        movement.priority = SequencePriority::Normal;
        movement.orders.clear();
        let order_id = engine.orders.allocate_order_id();
        movement.orders.push_back(Order::new(
            transition,
            destination.x,
            destination.y,
            order_id,
        ));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        let registered = engine.orders.sequence_manager.hourglass();
        assert_eq!(registered.len(), 1);
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

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            entity.element_data().sprite.last_processed_order_id,
            order_id.get(),
            "regression must execute the newly initialized transition"
        );
        assert_eq!(
            i16::from(entity.position_iface().get_direction_goal()),
            7,
            "new-order ComputeIncrementAll must replace rather than restore the stale goal"
        );
    }

    #[test]
    fn terminal_door_pass_goal_clear_follows_crossing_recompute() {
        let mut entity = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        let destination = MapPoint::new(1447.3046, 620.1602);
        let terminal_position = MapPoint::new(1446.2887, 620.40155);
        {
            let pi = entity.position_iface_mut();
            pi.set_map_position(terminal_position);
            pi.set_map_goal(destination);
            // CrossElevationLine invalidates the cached trajectory before
            // Actor::CheckForLineCrossing rebuilds it against mpOrder's live
            // destination.
            pi.compute_increment_all(true);
        }
        let crossing_increment = entity.position_iface().get_increment_map();
        assert!(
            crossing_increment.x > 0.9 && crossing_increment.y < -0.2,
            "fixture must retain Save042's eastward terminal trajectory"
        );

        clear_terminal_door_pass_goal(&mut entity);

        assert_eq!(entity.position_iface().map_goal(), MapPoint::ZERO);
        assert_eq!(
            entity.position_iface().raw_increment_map(),
            crossing_increment,
            "terminal condolation clears the goal only after crossing has rebuilt the cached increment"
        );
        assert!(
            !entity.position_iface().is_increment_map_computed(),
            "clearing the completed goal must invalidate, but not overwrite, Original's cached increment"
        );
        let origin_length = (terminal_position.x * terminal_position.x
            + terminal_position.y * terminal_position.y)
            .sqrt();
        let origin_increment = MapVec::new(
            -terminal_position.x / origin_length,
            -terminal_position.y / origin_length,
        );
        assert_ne!(
            crossing_increment, origin_increment,
            "crossing must not rebuild the trajectory from the cleared (0, 0) sentinel"
        );
    }

    fn install_terminal_interaction_seek(
        command: Command,
        distance: f32,
    ) -> (EngineInner, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        let transition = OrderType::TransitionWalkingUprightWaitingUpright;
        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![0; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[transition as usize] = 0;
        conversion[OrderType::TransitionWaitingUprightWalkingUpright as usize] = 16;

        let mut owner_element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        owner_element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 32]),
            std::sync::Arc::new(conversion),
        );
        owner_element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        owner_element
            .sprite
            .position_iface
            .set_anti_collision_on(false);
        owner_element
            .sprite
            .position_iface
            .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
        owner_element.set_sector(Some(
            crate::position_interface::SectorHandle::new(1).unwrap(),
        ));
        owner_element.set_position_map(MapPoint::new(100.0, 100.0));
        owner_element.set_direction_goal(7);
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: owner_element,
            actor: ActorData {
                action_state: ActionState::Moving,
                seek_distance: 34.0,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let mut target_element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: if command == Command::TieCmd {
                Posture::Lying
            } else {
                Posture::Upright
            },
            ..ElementData::default()
        };
        target_element.sprite.position_iface.set_move_box(
            crate::coordinates::MoveBox::from_coords(-4.0, -4.0, 4.0, 4.0),
        );
        target_element
            .sprite
            .position_iface
            .set_anti_collision_on(false);
        target_element.set_sector(Some(
            crate::position_interface::SectorHandle::new(1).unwrap(),
        ));
        target_element.set_position_map(MapPoint::new(100.0 + distance, 100.0));
        let target = engine.add_entity(Entity::Soldier(ActorSoldier {
            element: target_element,
            actor: ActorData::default(),
            human: HumanData {
                unconscious: command == Command::TieCmd,
                ..HumanData::default()
            },
            npc: NpcData::default(),
            soldier: SoldierData {
                cached_camp: Camp::Lacklandists,
                ..SoldierData::default()
            },
        }));

        let mut interaction = Sequence::new();
        interaction.append_element(SequenceElement::new_interaction(
            1,
            command,
            Some(owner),
            Some(target),
        ));
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.seek_target = Some(target);
        actor.last_seek_target_position = MapPoint::new(100.0 + distance, 100.0);
        actor.post_seek_sequence = Some(interaction.into_post_seek());

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingUpright,
        );
        movement.priority = SequencePriority::Normal;
        if let SequenceElementData::Movement {
            flags,
            element,
            sector,
            tolerance,
            destination,
            ..
        } = &mut movement.data
        {
            *flags = MoveFlags::SEEK;
            // Reproduce the copied terminal-transition shape from the
            // schema-14 controls: FinalTol no longer carries the movement
            // element target, while PerformSeek still owns seek_target on
            // ActorData.
            *element = None;
            *sector = crate::position_interface::SectorHandle::new(1);
            *tolerance = 34.0;
            *destination = MapPoint::new(100.0 + distance, 100.0);
        }
        movement.orders.clear();
        let order_id = engine.orders.allocate_order_id();
        movement
            .orders
            .push_back(Order::new(transition, 100.0, 100.0, order_id));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        let registered = engine.orders.sequence_manager.hourglass();
        assert_eq!(
            registered.len(),
            1,
            "fixture must consume its launch registration"
        );
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
        (engine, owner, target)
    }

    fn finish_terminal_seek_tick(engine: &mut EngineInner, owner: EntityId) {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        for _ in 0..64 {
            engine.tick_entity_movement(&sim, &assets);
            engine.hourglass_phase_sequences(
                &sim,
                &mut crate::engine::HostDisplayState::default(),
                &assets,
            );
            if engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .post_seek_sequence
                .is_none()
            {
                return;
            }
        }
        panic!(
            "terminal seek transition did not finish: order={:?}, post_seek_present={}",
            engine.actor_order_type(owner),
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .post_seek_sequence
                .is_some()
        );
    }

    #[test]
    fn terminal_pc_hit_seek_outside_init_range_aborts_without_visible_hit() {
        let (mut engine, owner, target) = install_terminal_interaction_seek(Command::HitCmd, 55.8);
        finish_terminal_seek_tick(&mut engine, owner);
        assert_ne!(engine.actor_order_type(owner), Some(OrderType::Hitting));
        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert!(actor.post_seek_sequence.is_none());
        assert!(actor.seek_target.is_none());
        assert!(actor.active_movement.sequence_id.is_none());
        let expected = vector_to_sector_0_to_15(
            engine.get_entity(target).unwrap().ground_position().x
                - engine.get_entity(owner).unwrap().ground_position().x,
            engine.get_entity(target).unwrap().ground_position().y
                - engine.get_entity(owner).unwrap().ground_position().y,
        );
        assert_eq!(
            i16::from(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .position_iface()
                    .get_direction_goal(),
            ),
            expected
        );
    }

    #[test]
    fn terminal_pc_hit_seek_at_init_range_launches_hit_normally() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::HitCmd, 40.0);
        finish_terminal_seek_tick(&mut engine, owner);
        assert_eq!(engine.actor_order_type(owner), Some(OrderType::Hitting));
    }

    #[test]
    fn terminal_non_seek_move_does_not_launch_stale_actor_post_seek() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::HitCmd, 16.0);
        let (seq_id, elem_idx) = {
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        let SequenceElementData::Movement { flags, .. } = &mut engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .expect("terminal movement fixture lost its selected element")
            .data
        else {
            panic!("terminal movement fixture lost movement data")
        };
        *flags = MoveFlags::empty();

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        for _ in 0..64 {
            engine.tick_entity_movement(&sim, &assets);
            engine.hourglass_phase_sequences(
                &sim,
                &mut crate::engine::HostDisplayState::default(),
                &assets,
            );
            if engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_movement
                .sequence_id
                .is_none()
            {
                break;
            }
        }

        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert!(actor.active_movement.sequence_id.is_none());
        assert!(
            actor.post_seek_sequence.is_some(),
            "ordinary Move must not consume stale actor-owned post-seek state"
        );
        assert_ne!(engine.actor_order_type(owner), Some(OrderType::Hitting));
    }

    #[test]
    fn looped_seek_stop_transition_rechecks_final_order_after_deleting_followers() {
        let (mut engine, owner, target) = install_terminal_interaction_seek(Command::HitCmd, 40.0);
        let (seq_id, elem_idx) = {
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        let transition = OrderType::TransitionWalkingUprightWaitingUpright;
        let order_ids = [
            engine.orders.allocate_order_id(),
            engine.orders.allocate_order_id(),
            engine.orders.allocate_order_id(),
        ];
        let element = engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .unwrap();
        let SequenceElementData::Movement {
            element: seek_target,
            ..
        } = &mut element.data
        else {
            panic!("seek regression lost movement data")
        };
        *seek_target = Some(target);
        element.orders.clear();
        for (destination_x, order_id) in [110.0, 120.0, 140.0].into_iter().zip(order_ids) {
            element
                .orders
                .push_back(Order::new(transition, destination_x, 100.0, order_id));
        }
        assert_eq!(element.orders.len(), 3);
        assert_ne!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .position_map(),
            MapPoint::new(110.0, 100.0),
            "the first transition must loop before reaching its destination"
        );

        finish_terminal_seek_tick(&mut engine, owner);

        assert_eq!(
            engine.actor_order_type(owner),
            Some(OrderType::Hitting),
            "PerformSeek must observe that TillLastFrame deleted all followers and launch the attached post-seek"
        );
    }

    #[test]
    fn looped_seek_start_transition_refreshes_before_copied_stop_transition() {
        let (mut engine, owner, target) = install_terminal_interaction_seek(Command::HealCmd, 70.0);
        let (seq_id, elem_idx) = {
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        let stale_target = MapPoint::new(140.0, 100.0);
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.last_seek_target_position = stale_target;
            actor.seek_refresh_wait = 22;
            actor.wait_time = 22;
        }
        let order_ids = [
            engine.orders.allocate_order_id(),
            engine.orders.allocate_order_id(),
        ];
        let element = engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .unwrap();
        let SequenceElementData::Movement {
            element: seek_target,
            destination,
            tolerance,
            ..
        } = &mut element.data
        else {
            panic!("seek regression lost movement data")
        };
        *seek_target = Some(target);
        *destination = stale_target;
        *tolerance = 17.0;
        element.orders.clear();
        element.orders.push_back(Order::new(
            OrderType::TransitionWaitingUprightWalkingUpright,
            120.0,
            100.0,
            order_ids[0],
        ));
        element.orders.push_back(Order::new(
            OrderType::TransitionWalkingUprightWaitingUpright,
            stale_target.x,
            stale_target.y,
            order_ids[1],
        ));

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        for _ in 0..8 {
            engine.tick_entity_movement(&sim, &assets);
            if engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .seek_refresh_wait
                == 25
            {
                break;
            }
        }

        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert_eq!(actor.seek_refresh_wait, 25);
        assert_eq!(actor.wait_time, 25);
        assert_eq!(
            actor.last_seek_target_position,
            engine
                .get_entity(target)
                .unwrap()
                .element_data()
                .position_map()
        );
        assert_eq!(actor.action_state, ActionState::Moving);
        assert_ne!(
            engine.actor_order_type(owner),
            Some(OrderType::TransitionWalkingUprightWaitingUpright),
            "PerformSeek must refresh before executing the copied stale stop transition"
        );
    }

    #[test]
    fn terminal_pc_hit_in_live_range_precedes_expired_stale_seek_refresh() {
        let (mut engine, owner, target) = install_terminal_interaction_seek(Command::HitCmd, 20.0);
        let (seq_id, elem_idx) = {
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        let target_position = engine
            .get_entity(target)
            .unwrap()
            .element_data()
            .position_map();
        let stale_position = MapPoint::new(target_position.x + 100.0, target_position.y);
        let SequenceElementData::Movement {
            element,
            destination,
            ..
        } = &mut engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .unwrap()
            .data
        else {
            panic!("terminal seek fixture lost movement data")
        };
        *element = Some(target);
        *destination = stale_position;
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.seek_refresh_wait = 0;
            actor.wait_time = 0;
            actor.last_seek_target_position = stale_position;
        }

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        assert!(
            !engine.tick_refresh_seek_for_owner(&sim, &assets, owner),
            "PerformSeek must observe the live in-range target before testing stale-route refresh"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .seek_refresh_wait,
            0,
            "the suppressed refresh must not rearm the 25-frame timer"
        );

        finish_terminal_seek_tick(&mut engine, owner);
        assert_eq!(engine.actor_order_type(owner), Some(OrderType::Hitting));
    }

    #[test]
    fn terminal_pc_tie_seek_outside_init_range_publishes_tie_before_execute_validation() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::TieCmd, 55.8);
        finish_terminal_seek_tick(&mut engine, owner);

        assert_eq!(engine.actor_order_type(owner), Some(OrderType::Tying));
        let entity = engine.get_entity(owner).unwrap();
        let actor = entity.actor_data().unwrap();
        assert!(actor.post_seek_sequence.is_none());
        assert!(actor.seek_target.is_none());
        assert!(actor.active_movement.sequence_id.is_none());
        assert_eq!(
            i16::from(entity.position_iface().get_direction_goal()),
            7,
            "Tie translation itself must not run the next Hourglass's initialization turn"
        );
    }

    #[test]
    fn terminal_pc_tie_seek_at_init_range_launches_tie_normally() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::TieCmd, 40.0);
        finish_terminal_seek_tick(&mut engine, owner);
        assert_eq!(engine.actor_order_type(owner), Some(OrderType::Tying));
    }

    #[test]
    fn stopped_short_point_seek_does_not_launch_post_seek() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::HitCmd, 40.0);
        let seek_sector = crate::position_interface::SectorHandle::new(2).unwrap();
        let (sequence_id, element_index) = {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.seek_target = None;
            actor.continuation.seek_to_point = true;
            actor.continuation.seek_layer = 0;
            actor.continuation.seek_sector =
                Some(crate::actor_state::ActorSeekSector::Position(seek_sector));
            let mut post_seek = Sequence::new();
            post_seek.append_element(SequenceElement::new(1, Command::DropAle, Some(owner)));
            actor.post_seek_sequence = Some(post_seek.into_post_seek());
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        let movement = engine
            .orders
            .sequence_manager
            .get_element_mut(sequence_id, element_index)
            .unwrap();
        let SequenceElementData::Movement { sector, .. } = &mut movement.data else {
            panic!("point-seek fixture must retain its movement element")
        };
        *sector = Some(seek_sector);

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        for _ in 0..4 {
            engine.tick_entity_movement(&sim, &assets);
            engine.hourglass_phase_sequences(
                &sim,
                &mut crate::engine::HostDisplayState::default(),
                &assets,
            );
            if engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_movement
                .sequence_id
                .is_none()
            {
                break;
            }
        }

        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert!(actor.active_movement.sequence_id.is_none());
        assert!(
            actor.post_seek_sequence.is_some(),
            "a stop transition ending outside the point seek's sector must not launch its stale post-seek"
        );
        assert_ne!(engine.actor_order_type(owner), Some(OrderType::DroppingAle));
    }

    #[test]
    fn translated_point_seek_terminal_in_matching_sector_launches_drop_ale() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::HitCmd, 40.0);
        let (old_sequence, old_index) = {
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        engine.orders.sequence_manager.element_interrupted(
            old_sequence,
            old_index,
            crate::sequence::CascadeFlags::FOLLOWING,
        );
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.active_movement.clear();
            actor.post_seek_sequence = None;
        }
        engine.dispatch_condolations(&crate::sim_rng::test_context(), &LevelAssets::new());

        let seek_sector = crate::position_interface::SectorHandle::new(1).unwrap();
        let seek_layer = 3;
        let destination = MapPoint::new(100.0, 100.0);
        let mut post_seek = Sequence::new();
        post_seek.append_element(SequenceElement::new(1, Command::DropAle, Some(owner)));
        let mut seek =
            SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
        seek.priority = SequencePriority::Normal;
        let SequenceElementData::Movement {
            destination: stored_destination,
            sector,
            layer,
            post_seek_sequence,
            ..
        } = &mut seek.data
        else {
            unreachable!("Seek must have movement data")
        };
        *stored_destination = destination;
        *sector = Some(seek_sector);
        *layer = seek_layer;
        *post_seek_sequence = Some(post_seek.into_post_seek());

        let transient = engine.orders.sequence_manager.launch_element(seek);
        engine.hourglass_phase_sequences(
            &crate::sim_rng::test_context(),
            &mut crate::engine::HostDisplayState::default(),
            &LevelAssets::new(),
        );

        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert!(actor.continuation.seek_to_point);
        assert_eq!(actor.continuation.seek_layer, seek_layer);
        assert_eq!(
            actor.continuation.seek_sector,
            Some(crate::actor_state::ActorSeekSector::Position(seek_sector))
        );
        assert!(actor.post_seek_sequence.is_some());

        let (movement_sequence, movement_index) = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .filter(|sequence| sequence.id != transient)
            .find_map(|sequence| {
                sequence
                    .elements
                    .iter()
                    .enumerate()
                    .find_map(|(index, element)| {
                        (element.owner == Some(owner)
                            && element.data.is_movement()
                            && element.state == SequenceState::InProgress)
                            .then_some((sequence.id, index))
                    })
            })
            .expect("Translate(SEEK) must launch a concrete MOVE|SEEK replacement");
        let transition = OrderType::TransitionWalkingUprightWaitingUpright;
        let order_id = engine.orders.allocate_order_id();
        let follower_order_id = engine.orders.allocate_order_id();
        let movement = engine
            .orders
            .sequence_manager
            .get_element_mut(movement_sequence, movement_index)
            .unwrap();
        movement.command = Command::MoveOk;
        let SequenceElementData::Movement { sector, .. } = &mut movement.data else {
            panic!("translated point Seek replacement lost movement data")
        };
        *sector = crate::position_interface::SectorHandle::new(2);
        movement.orders.clear();
        movement.orders.push_back(Order::new(
            transition,
            destination.x,
            destination.y,
            order_id,
        ));
        movement.orders.push_back(Order::new(
            transition,
            destination.x,
            destination.y,
            follower_order_id,
        ));
        // Point-target PerformSeek keys the terminal handoff on the absence
        // of another movement order, not on this movement element being the
        // final element of its sequence. Keep a later sibling present to
        // cover that distinction.
        engine
            .orders
            .sequence_manager
            .get_sequence_mut(movement_sequence)
            .unwrap()
            .append_element(SequenceElement::new(2, Command::Wait, Some(owner)));
        {
            let entity = engine.get_entity_mut(owner).unwrap();
            entity.actor_data_mut().unwrap().active_movement =
                ActiveMovement::new(movement_sequence, movement_index);
            entity.position_iface_mut().set_map_goal(destination);
        }

        finish_terminal_seek_tick(&mut engine, owner);
        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert!(actor.post_seek_sequence.is_none());
        assert!(actor.active_movement.sequence_id.is_none());
        assert_eq!(engine.actor_order_type(owner), Some(OrderType::DroppingAle));
    }
}

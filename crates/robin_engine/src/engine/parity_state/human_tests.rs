//! Frozen pre-refactor JSON encoder, independent of the typed human/PC records.
use super::*;

#[test]
fn human_and_pc_records_match_frozen_encoder_with_populated_frontiers() {
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::{
        ActorPc, ActorSoldier, ElementData, ElementKind, Entity, HumanData, PcData,
    };
    use crate::profiles::{Action, CharacterProfile, CharacterProfileIdx};
    const COMPONENTS: [&str; 7] = [
        "human_continuation",
        "human_structure",
        "pc_core",
        "pc_qa",
        "pc_interface",
        "pc_portrait",
        "pc_tail",
    ];

    for populated in [false, true] {
        let mut inner = EngineInner::new();
        let mut assets = LevelAssets::new();
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles.characters.push(CharacterProfile {
            actions: [Action::Net, Action::Bow, Action::NoAction],
            ..Default::default()
        });
        assets.profile_manager = std::sync::Arc::new(profiles);
        inner
            .mission_domain
            .campaign
            .characters
            .push(crate::campaign::PcDescription {
                character_profile_idx: Some(CharacterProfileIdx(0)),
                instanced: true,
                status: crate::pc_status::PcStatus {
                    num_nets: 17,
                    num_arrows: 29,
                    ..Default::default()
                },
            });
        let mut soldier_element = ElementData::default();
        soldier_element.kind = ElementKind::ActorSoldier;
        let opponent = inner.add_test_entity(Entity::Soldier(ActorSoldier {
            element: soldier_element,
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        }));
        let mut human = HumanData::default();
        let mut pc = PcData {
            campaign_description_index: Some(0),
            profile_index: CharacterProfileIdx(0),
            // The parity boundary describes initialized PCs, whose profile
            // supplies exactly three permanent and temporary action slots.
            disabled_actions: vec![false; 3],
            disabled_actions_temp: vec![false; 3],
            ..Default::default()
        };
        if populated {
            human.already_detectable_body = true;
            human.concussion_healing_timeout = 2;
            human.tiredness = 3;
            human.concussion_of_the_brain = 4;
            human.parry_counter = 5;
            human.detectable_list_index = 6;
            human.invulnerable = true;
            human.last_motion_was_step_back_in_combat = true;
            human.smalltalk_initiative = true;
            human.received_smalltalk_initiative = true;
            human.smalltalk_hint_opponent = Some(opponent);
            human.relative_fighting_ability = 7;
            human.hollow_man = true;
            human.killed_by_accident = true;
            human.stuck_under_nets_counter = 8;
            human.sword_strike_boredom = vec![u16::MAX, 7, 2, 7];
            human.carrier = Some(opponent);
            human.small_repulsive_radius = true;
            human.running_hulk = 9;
            human.time_hulk = 10;
            human.hulk_level = 11;
            human.hulk_direction = true;
            human.hulk_speed = f32::from_bits(0x7fc0_1234);
            std::sync::Arc::make_mut(&mut inner.world.fast_grid_mut().level)
                .jump_lines
                .push(crate::jump_line::JumpLine::new(
                    MapPoint::new(-0.0, 3.5),
                    MapPoint::new(17.0, -11.0),
                    0.0,
                    0.0,
                ));
            human.opponents = crate::element::SwordfightOpponents::from_pairs([
                (
                    opponent,
                    Some(crate::jump_line::JumpLineIndex::new(0).unwrap()),
                ),
                (opponent, None),
            ]);
            human.repulsive_point.position = MapPoint::new(-0.0, 7.5);
            human.repulsive_point.concave = true;
            human.repulsive_point.limit_left = MapPoint::new(-3.0, 4.0);
            human.repulsive_point.limit_right = MapPoint::new(9.0, -2.0);
            human.repulsive_point.action_radius = f32::INFINITY;
            human.repulsive_point.force_a = -0.0;
            human.repulsive_point.force_b = 17.5;
            human.repulsive_point.radius = f32::NEG_INFINITY;
            human.repulsive_point.id = 47;
            human.repulsive_point.affects_pcs = true;
            human.repulsive_point.affects_civilians = true;
            human.repulsive_point.affects_animals = true;
            for (i, point) in human.shield.points.iter_mut().enumerate() {
                point.obstacle = [i as f32, -0.0, f32::INFINITY, -11.25];
                point.polygon = MapPoint::new(i as f32, -2.5);
            }
            human.shield.top_plane.a = WorldPoint3D::new(1.0, 2.0, 3.0);
            human.shield.top_plane.b = WorldPoint3D::new(4.0, 5.0, 6.0);
            human.shield.top_plane.normal = WorldPoint3D::new(-0.0, 7.0, 8.0);
            human.shield.top_plane.origin = WorldPoint3D::new(9.0, 10.0, 11.0);
            human.shield.top_plane.u = WorldPoint3D::new(12.0, 13.0, 14.0);
            human.shield.top_plane.v = WorldPoint3D::new(15.0, 16.0, 17.0);
            human.shield.top_plane.az = 18.0;
            human.shield.top_plane.bz = 19.0;
            human.shield.top_plane.dz = 20.0;
            human.shield.top_plane.d = 21.0;
            human.shield.bottom_plane.d = -27.5;
            human.shield.box_3d = [1.0, -0.0, 3.0, 4.0, 5.0, 6.0];
            human.shield.ground_box.top_left = MapPoint::new(-4.0, 7.0);
            human.shield.ground_box.bottom_right = MapPoint::new(17.0, 29.0);
            human.shield.ground_box.bounds_are_set = true;
            human.shield.screen_box.bottom_right = MapPoint::new(37.0, -13.0);
            human.shield.on_ground = true;
            human.sword_sweep.victims = vec![opponent, opponent];
            human.sword_sweep.initial_angle = -0.0;
            human.sword_sweep.current_angle = 23.5;
            human.sword_sweep.final_angle = f32::from_bits(0x7fc0_4321);
            let sequence = inner.orders.sequence_manager.launch_element(
                crate::sequence::SequenceElement::new_generic(
                    1,
                    crate::element::Command::WaitTimer,
                    Some(opponent),
                ),
            );
            human.pending_shoots = vec![crate::sequence::SequenceElementRef::new(sequence, 0)];
            pc.playable = true;
            pc.beam_me_index = -3;
            pc.already_selected = true;
            pc.belt_seen = true;
            pc.feet_seen = true;
            pc.head_seen = true;
            pc.immortal = true;
            pc.fried_psykokwack = true;
            pc.list_index = 7;
            pc.teleport_counter = 53;
            pc.current_action = Action::Net;
            pc.saved_action = Action::Bow;
            pc.disabled_actions = vec![true, false, true];
            pc.disabled_actions_temp = vec![false, true, false];
            pc.position_before_teleport = MapPoint::new(-0.0, 37.25);
            pc.quick_action_special_counts[0] = 13;
            pc.quick_action_buttons[1] = 19;
            pc.quick_action_interactors[2] = Some(opponent);
            pc.titbits[0] = Some(crate::titbit::TitbitId::new(23).unwrap());
            pc.quick_action_sequences[0] = Some(crate::sequence::Sequence::new());
            pc.quick_seek_sequences[2] = Some(crate::sequence::Sequence::new());
            pc.interface_hidden = true;
            pc.portrait.burned = true;
            pc.portrait.open = true;
            pc.life_points = -17;
            pc.trumpet_enabled = true;
            pc.portrait.quick_icons[0].titbit_id = Some(crate::titbit::TitbitId::new(31).unwrap());
            pc.portrait.quick_icons[0].running = true;
            pc.carried = Some(opponent);
            pc.carried_posture = 7;
            pc.shield_danger_point = WorldPoint3D::new(3.0, -0.0, 17.5);
            pc.shield_protected = Some(opponent);
            pc.shield_protector = Some(opponent);
            pc.guard = Some(opponent);
            pc.time_till_reinforcement = 59;
            pc.last_ammo_dropping_position = MapPoint::new(5.0, 7.0);
            pc.last_dropped_ammo = Some(opponent);
            pc.update_last_dropped_ammo = true;
            pc.last_dropping_direction = 17;
        }
        let mut element = ElementData::default();
        element.kind = ElementKind::ActorPc;
        let id = inner.add_test_entity(Entity::Pc(ActorPc {
            element,
            actor: Default::default(),
            human,
            pc,
        }));
        let engine = Engine {
            inner,
            bootstrap_open: false,
        };
        let before = crate::replay::state_hash(&engine);
        for entity_id in [opponent, id] {
            let expected = engine.original_human_frontier(entity_id, &assets);
            let actual = engine.parity_entity_runtime_state(entity_id, &assets);
            for key in COMPONENTS {
                assert_eq!(
                    actual.get(key),
                    expected.get(key),
                    "{key}, populated={populated}"
                );
                if let Some(value) = expected.get(key) {
                    assert_eq!(
                        serde_json::to_vec(&actual[key]).unwrap(),
                        serde_json::to_vec(value).unwrap()
                    );
                }
            }
        }
        assert_eq!(
            before,
            crate::replay::state_hash(&engine),
            "projection must remain read-only"
        );
    }
}

impl Engine {
    fn original_human_frontier(&self, id: EntityId, assets: &LevelAssets) -> serde_json::Value {
        use serde_json::{Value, json};
        let entity = self.inner.world.entities.get(id).expect("oracle entity");
        let entity_ref = |id: EntityId| {
            use crate::element::EntityIdKind;
            let kind = match id.kind() {
                EntityIdKind::Pc => "pc",
                EntityIdKind::Soldier => "soldier",
                EntityIdKind::Civilian => "civilian",
                EntityIdKind::Fx => "fx",
                EntityIdKind::Target => "target",
                EntityIdKind::Bonus => "bonus",
                EntityIdKind::Scroll => "scroll",
                EntityIdKind::Projectile => "projectile",
                EntityIdKind::Net => "net",
            };
            json!({"kind":kind,"index":id.index()})
        };
        let float = |value: f32| json!({ "bits": value.to_bits(), "value": value });
        let point2 = |x: f32, y: f32| json!({ "x": float(x), "y": float(y) });
        let point3 =
            |x: f32, y: f32, z: f32| json!({ "x": float(x), "y": float(y), "z": float(z) });
        let jump_line = |index: Option<u32>| {
            let index = index?;
            let line = self
                .inner
                .world
                .fast_grid
                .level
                .jump_lines
                .get(usize::try_from(index).expect("parity enemy jump-line index exceeds usize"))
                .unwrap_or_else(|| panic!("parity enemy references missing jump line {index}"));
            Some(
                json!({ "a": point2(line.point_a.x, line.point_a.y), "b": point2(line.point_b.x, line.point_b.y) }),
            )
        };
        let sector = |handle: Option<crate::position_interface::SectorHandle>| {
            handle.map(|handle| {
                let level = &self.inner.world.fast_grid.level;
                let arena_index = handle.arena_index().map_or_else(
                    || {
                        let public = crate::sector::SectorNumber::new(i16::from(handle));
                        level.sector_number_map.get(&public).copied().unwrap_or_else(|| {
                            panic!(
                                "parity position for {id:?} references missing public sector {handle}"
                            )
                        })
                    },
                    usize::from,
                );
                let sector = level.sectors.get(arena_index).unwrap_or_else(|| {
                    panic!(
                        "parity position for {id:?} references missing sector arena index {arena_index} (public {handle})"
                    )
                });
                assert_eq!(
                    u16::from(sector.sector_number),
                    handle.get(),
                    "parity position for {id:?} sector arena index {arena_index} has public number {}, expected {handle}",
                    sector.sector_number.get(),
                );
                sector.sector_number.get()
            })
        };
        let human_continuation = entity.human_data().map(|human| {
            json!({
                "already_detectable_body": human.already_detectable_body,
                "concussion_healing_timeout": human.concussion_healing_timeout,
                "tiredness": human.tiredness,
                "concussion": human.concussion_of_the_brain,
                "parry_counter": human.parry_counter,
                "detectable_list_index": human.detectable_list_index,
                "invulnerable": human.invulnerable,
                "last_motion_was_step_back": human.last_motion_was_step_back_in_combat,
                "smalltalk_initiative": human.smalltalk_initiative,
                "received_smalltalk_initiative": human.received_smalltalk_initiative,
                "smalltalk_hint": human.smalltalk_hint as u32,
                "smalltalk_hint_opponent": human.smalltalk_hint_opponent.map_or(Value::Null, entity_ref),
                "relative_fighting_ability": human.relative_fighting_ability,
                "hollow_man": human.hollow_man,
                "killed_by_accident": human.killed_by_accident,
                "stuck_under_nets_counter": human.stuck_under_nets_counter,
                "sword_strike_boredom": &human.sword_strike_boredom,
                "carrier": human.carrier.map_or(Value::Null, entity_ref),
                "small_repulsive_radius": human.small_repulsive_radius,
                "hulk": {
                    "running": human.running_hulk, "time": human.time_hulk,
                    "level": human.hulk_level, "direction": human.hulk_direction,
                    "speed": float(human.hulk_speed),
                },
            })
        });
        let human_structure = entity.human_data().map(|human| {
            let opponents = human
                .opponents
                .iter_with_jump_lines()
                .map(|(opponent, line)| json!({
                    "entity": entity_ref(opponent), "jump_line": jump_line(line.map(u32::from)),
                }))
                .collect::<Vec<_>>();
            let repulsive = &human.repulsive_point;
            let shield = &human.shield;
            let plane = |value: &crate::element::HumanPlaneState| json!({
                "a": point3(value.a.x, value.a.y, value.a.z),
                "b": point3(value.b.x, value.b.y, value.b.z),
                "normal": point3(value.normal.x, value.normal.y, value.normal.z),
                "origin": point3(value.origin.x, value.origin.y, value.origin.z),
                "u": point3(value.u.x, value.u.y, value.u.z),
                "v": point3(value.v.x, value.v.y, value.v.z),
                "az": float(value.az), "bz": float(value.bz),
                "dz": float(value.dz), "d": float(value.d),
            });
            let box2_state = |value: crate::element::HumanBoundingBox2State| json!({
                "top_left": point2(value.top_left.x, value.top_left.y),
                "bottom_right": point2(value.bottom_right.x, value.bottom_right.y),
                "bounds_are_set": value.bounds_are_set,
            });
            let sequence_ordinals: std::collections::BTreeMap<_, _> = self
                .inner
                .orders
                .sequence_manager
                .sequences_iter()
                .enumerate()
                .map(|(ordinal, sequence)| (sequence.id, ordinal))
                .collect();
            let sequence_ref = |value: crate::sequence::SequenceElementRef| {
                let sequence = sequence_ordinals.get(&value.sequence_id).copied().unwrap_or_else(|| {
                    panic!("parity human pending shoot points outside sequence manager: {value:?}")
                });
                json!({ "sequence": sequence, "element": value.element_index })
            };
            json!({
                "opponents": opponents,
                "repulsive_point": {
                    "position": point2(repulsive.position.x, repulsive.position.y),
                    "concave": repulsive.concave,
                    "limit_left": point2(repulsive.limit_left.x, repulsive.limit_left.y),
                    "limit_right": point2(repulsive.limit_right.x, repulsive.limit_right.y),
                    "action_radius": float(repulsive.action_radius),
                    "force_a": float(repulsive.force_a), "force_b": float(repulsive.force_b),
                    "radius": float(repulsive.radius), "id": repulsive.id,
                    "affects_pcs": repulsive.affects_pcs,
                    "affects_soldiers": repulsive.affects_soldiers,
                    "affects_civilians": repulsive.affects_civilians,
                    "affects_animals": repulsive.affects_animals,
                },
                "building": sector(human.building_sector),
                "shield": {
                    "points": shield.points.iter().map(|value| json!({
                        "obstacle": value.obstacle.map(float),
                        "polygon": point2(value.polygon.x, value.polygon.y),
                    })).collect::<Vec<_>>(),
                    "top_plane": plane(&shield.top_plane),
                    "bottom_plane": plane(&shield.bottom_plane),
                    "box_3d": shield.box_3d.map(float),
                    "ground_box": box2_state(shield.ground_box),
                    "screen_box": box2_state(shield.screen_box),
                    "on_ground": shield.on_ground,
                },
                "sword_sweep": {
                    "victims": human.sword_sweep.victims.iter().copied().map(entity_ref).collect::<Vec<_>>(),
                    "initial_angle": float(human.sword_sweep.initial_angle),
                    "current_angle": float(human.sword_sweep.current_angle),
                    "final_angle": float(human.sword_sweep.final_angle),
                },
                "pending_shoots": human.pending_shoots.iter().copied().map(sequence_ref).collect::<Vec<_>>(),
            })
        });
        let pc_core = entity.pc_data().map(|pc| {
            const ACTIONS: usize = 3;
            assert_eq!(
                pc.disabled_actions.len(),
                ACTIONS,
                "PC {id:?} parity projection has {} permanent action flags, expected {ACTIONS}",
                pc.disabled_actions.len()
            );
            assert_eq!(
                pc.disabled_actions_temp.len(),
                ACTIONS,
                "PC {id:?} parity projection has {} temporary action flags, expected {ACTIONS}",
                pc.disabled_actions_temp.len()
            );
            let campaign_description_index = pc.campaign_description_index.unwrap_or_else(|| {
                panic!("PC {id:?} parity projection has no campaign description index")
            });
            json!({
                "work_icon": pc.work_icon as u32,
                "campaign_description_index": campaign_description_index,
                "playable": pc.playable,
                "beam_me_index": pc.beam_me_index,
                "already_selected": pc.already_selected,
                "belt_seen": pc.belt_seen,
                "feet_seen": pc.feet_seen,
                "head_seen": pc.head_seen,
                "immortal": pc.immortal,
                "fried_psykokwack": pc.fried_psykokwack,
                "list_index": pc.list_index,
                "teleport_counter": pc.teleport_counter,
                "current_action": pc.current_action as u32,
                "saved_action": pc.saved_action as u32,
                "disabled_actions": pc.disabled_actions,
                "disabled_actions_temp": pc.disabled_actions_temp,
                "position_before_teleport": point2(
                    pc.position_before_teleport.x,
                    pc.position_before_teleport.y,
                ),
            })
        });
        let pc_qa = entity.pc_data().map(|pc| {
            const QA_SLOTS: usize = crate::macro_store::NUMBER_OF_QA_MEMORY;
            for (name, length) in [
                ("types", pc.quick_action_types.len()),
                ("actions", pc.quick_action_sequences.len()),
                ("seeks", pc.quick_seek_sequences.len()),
                ("special-counts", pc.quick_action_special_counts.len()),
                ("buttons", pc.quick_action_buttons.len()),
                ("interactors", pc.quick_action_interactors.len()),
                ("titbits", pc.titbits.len()),
            ] {
                assert_eq!(
                    length, QA_SLOTS,
                    "PC {id:?} parity projection has {length} {name}, expected {QA_SLOTS}"
                );
            }
            (0..QA_SLOTS)
                .map(|slot| {
                    json!({
                        "special_count": pc.quick_action_special_counts[slot],
                        "quickito": pc.quick_action_types[slot] as u32,
                        "titbit": pc.titbits[slot].map(crate::titbit::TitbitId::get),
                        "button": pc.quick_action_buttons[slot],
                        "interactor": pc.quick_action_interactors[slot].map_or(Value::Null, entity_ref),
                        "action_size": pc.quick_action_sequences[slot].as_ref().map(|sequence| sequence.len()),
                        "seek_size": pc.quick_seek_sequences[slot].as_ref().map(|sequence| sequence.len()),
                    })
                })
                .collect::<Vec<_>>()
        });
        let pc_interface = entity.pc_data().map(|pc| {
            json!({
                "playable": pc.playable,
                "displayed": !pc.interface_hidden,
            })
        });
        let pc_portrait = entity.pc_data().map(|pc| {
            let profile = assets
                .profile_manager
                .get_character(pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "PC {id:?} portrait has missing profile {}",
                        pc.profile_index
                    )
                });
            let description = self
                .inner
                .pc_description_for_pc_data(pc)
                .unwrap_or_else(|| panic!("PC {id:?} portrait has no campaign description"));
            let quantities = profile
                .actions
                .map(|action| description.status.get_ammo(action));
            json!({
                "quantities": quantities,
                "two_buttons_mode": profile.actions[2] == crate::profiles::Action::NoAction,
                "displayed": !pc.interface_hidden,
                "burned": pc.portrait.burned,
                "open": pc.portrait.open,
                "life_level": float(f32::from(pc.life_points)),
                "trumpet_enabled": pc.trumpet_enabled,
                "quick_icons": pc.portrait.quick_icons.iter().map(|icon| json!({
                    "titbit": icon.titbit_id.map(crate::titbit::TitbitId::get),
                    "running": icon.running,
                })).collect::<Vec<_>>(),
            })
        });
        let pc_tail = entity.pc_data().map(|pc| {
            json!({
                "carried": pc.carried.map_or(Value::Null, entity_ref),
                "carried_posture": pc.carried_posture,
                "shield_danger_point": point3(
                    pc.shield_danger_point.x,
                    pc.shield_danger_point.y,
                    pc.shield_danger_point.z,
                ),
                "shield_protected": pc.shield_protected.map_or(Value::Null, entity_ref),
                "shield_protector": pc.shield_protector.map_or(Value::Null, entity_ref),
                "guard": pc.guard.map_or(Value::Null, entity_ref),
                "time_till_reinforcement": pc.time_till_reinforcement,
                "last_ammo_dropping_position": point2(
                    pc.last_ammo_dropping_position.x,
                    pc.last_ammo_dropping_position.y,
                ),
                "last_dropped_ammo": pc.last_dropped_ammo.map_or(Value::Null, entity_ref),
                "update_last_dropped_ammo": pc.update_last_dropped_ammo,
                "last_dropping_direction": pc.last_dropping_direction,
            })
        });

        let mut result = serde_json::Map::new();
        for (key, value) in [
            ("human_continuation", human_continuation),
            ("human_structure", human_structure),
            ("pc_core", pc_core),
            ("pc_qa", pc_qa.map(|qa| json!(qa))),
            ("pc_interface", pc_interface),
            ("pc_portrait", pc_portrait),
            ("pc_tail", pc_tail),
        ] {
            if let Some(value) = value {
                result.insert(key.into(), value);
            }
        }
        Value::Object(result)
    }
}

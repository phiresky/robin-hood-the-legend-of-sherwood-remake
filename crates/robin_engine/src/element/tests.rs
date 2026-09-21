use super::*;

#[test]
fn entity_slots_fit_within_two_kibibytes() {
    assert!(std::mem::size_of::<Option<Entity>>() <= 2048);
}

#[test]
fn frozen_actor_execute_selection_keeps_independent_installed_identity() {
    let previous = std::num::NonZeroU32::new(10).unwrap();
    let selected = std::num::NonZeroU32::new(11).unwrap();
    let mut sequences = crate::sequence::SequenceManager::new();
    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let mut element = crate::sequence::SequenceElement::new(1, Command::Wait, Some(owner));
    element.push_order(crate::order::Order::new(
        crate::order::OrderType::WaitingUpright,
        0.0,
        0.0,
        previous,
    ));
    let sequence = sequences.insert_element(element);
    let installed = InstalledActorOrder::new(
        crate::sequence::SequenceElementRef::new(sequence, 0),
        sequences
            .get_element(sequence, 0)
            .unwrap()
            .current_order()
            .unwrap(),
    );
    let mut actor = ActorData {
        execution_frozen: true,
        installed_order: Some(installed),
        ..ActorData::default()
    };
    actor.select_execute_order(selected);
    assert!(actor.execute_order_initialising);
    assert_eq!(actor.last_execute_order_id, Some(selected));
    assert_eq!(actor.installed_order, Some(installed));
    assert!(actor.execution_frozen);
    actor.select_execute_order(selected);
    assert!(!actor.execute_order_initialising);
    actor.select_execute_order(previous);
    assert!(actor.execute_order_initialising);
    assert_eq!(actor.installed_order, Some(installed));
}

#[test]
fn hit_seek_abort_preserves_independent_actor_latches() {
    let mut actor = ActorData {
        wait_time: 99,
        seek_target: Some(EntityId::Pc(crate::entity_id::PcId(42))),
        post_seek_sequence: Some(crate::sequence::Sequence::new().into_post_seek()),
        selected_sequence_element: Some(crate::sequence::SequenceElementRef::new(
            crate::sequence::SequenceId(9),
            2,
        )),
        seek_distance: 23.5,
        last_seek_target_position: MapPoint::new(14.0, 27.0),
        last_execute_order_id: std::num::NonZeroU32::new(17),
        execute_order_initialising: true,
        execution_frozen: true,
        ..ActorData::default()
    };
    // Retain a complete expected value so any accidental reset of an
    // unrelated field becomes visible, including future stored latches.
    let mut expected = actor.clone();

    expected.seek_target = None;
    expected.post_seek_sequence = None;

    actor.abort_out_of_range_hit_seek();
    assert_eq!(bitcode::encode(&actor), bitcode::encode(&expected));
}

#[test]
fn belt_point_uses_retained_position_during_elevation_crossing_callback() {
    use crate::position_interface::{ObstacleHandle, PlaneZCoeffs};

    let mut pc = Entity::Pc(ActorPc {
        element: ElementData {
            kind: ElementKind::ActorPc,
            posture: Posture::Upright,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData::default(),
    });
    let position = pc.position_iface_mut();
    position.set_obstacle(
        Some(ObstacleHandle::new(1).unwrap()),
        Some(PlaneZCoeffs {
            az: 0.0,
            bz: 0.0,
            dz: 36.0,
        }),
    );
    position.set_map_position(MapPoint::new(198.0, 282.0));
    let outgoing = position.get_position();
    position.set_obstacle(
        Some(ObstacleHandle::new(2).unwrap()),
        Some(PlaneZCoeffs {
            az: 0.0,
            bz: 0.0,
            dz: 35.5,
        }),
    );
    let mut state = position.v48_serialized_state();
    state.position = outgoing;
    state
        .computed_position
        .remove(crate::position_interface::PositionComputed::THREE_D);
    position.restore_v48_serialized_state(state);

    assert_eq!(
        pc.compute_belt_point(),
        Some(WorldPoint3D::new(198.0, 318.0, 61.0)),
        "belt-point calculation must preserve Original's raw world-position bytes"
    );
}

#[test]
fn detection_point_uses_retained_position_during_elevation_crossing_callback() {
    use crate::position_interface::{ObstacleHandle, PlaneZCoeffs};

    let mut soldier = Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            posture: Posture::Upright,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: SoldierData {
            rider: true,
            ..SoldierData::default()
        },
    });
    let position = soldier.position_iface_mut();
    position.set_obstacle(
        Some(ObstacleHandle::new(1).unwrap()),
        Some(PlaneZCoeffs {
            az: 0.0,
            bz: 0.0,
            dz: 36.0,
        }),
    );
    position.set_map_position(MapPoint::new(198.0, 282.0));
    let outgoing = position.get_position();
    position.set_obstacle(
        Some(ObstacleHandle::new(2).unwrap()),
        Some(PlaneZCoeffs {
            az: 0.0,
            bz: 0.0,
            dz: 35.5,
        }),
    );
    let mut state = position.v48_serialized_state();
    state.position = outgoing;
    state
        .computed_position
        .remove(crate::position_interface::PositionComputed::THREE_D);
    position.restore_v48_serialized_state(state);

    assert_eq!(
        soldier.compute_detection_point(),
        Some(WorldPoint3D::new(198.0, 318.0, 96.0)),
        "detection-point calculation must preserve Original's raw world-position bytes"
    );
}

#[test]
fn delayed_positions_preserve_original_map_then_world_priority() {
    let mut element = ElementData::default();
    element.set_position(WorldPoint3D::new(10.0, 20.0, 3.0));
    element.set_position_map_delayed(MapPoint::new(40.0, 50.0));
    element.set_position_delayed(WorldPoint3D::new(70.0, 80.0, 9.0));

    let (old_map, new_map, _) = element
        .apply_next_delayed_position()
        .expect("map queue should apply first");
    assert_eq!(old_map, MapPoint::from_world_xyz(10.0, 20.0, 3.0));
    assert_eq!(new_map, MapPoint::new(40.0, 50.0));
    assert!(!element.position_map_delayed);
    assert!(element.position_delayed);
    assert_eq!(element.delayed_map_position, MapPoint::new(40.0, 50.0));

    let (_, second_map, _) = element
        .apply_next_delayed_position()
        .expect("world queue should remain for the next frame");
    assert_eq!(element.position(), WorldPoint3D::new(70.0, 80.0, 9.0));
    assert_eq!(second_map, MapPoint::from_world_xyz(70.0, 80.0, 9.0));
    assert!(!element.position_delayed);
    assert_eq!(
        element.delayed_position,
        WorldPoint3D::new(70.0, 80.0, 9.0),
        "Original clears only the flag and retains the serialized point"
    );
}

fn object_data(object_type: ObjectType) -> ObjectData {
    ObjectData {
        object_type,
        ..Default::default()
    }
}

fn element_data(kind: ElementKind) -> ElementData {
    ElementData {
        kind,
        ..Default::default()
    }
}

#[test]
fn original_hourglass_mapping_covers_every_rust_concrete_class() {
    use OriginalHourglassClass as Class;

    let cases = [
        (
            Entity::Pc(ActorPc {
                element: element_data(ElementKind::ActorPc),
                actor: ActorData::default(),
                human: HumanData::default(),
                pc: PcData::default(),
            }),
            Class::ActorPc,
        ),
        (
            Entity::Soldier(ActorSoldier {
                element: element_data(ElementKind::ActorSoldier),
                actor: ActorData::default(),
                human: HumanData::default(),
                npc: NpcData::default(),
                soldier: SoldierData::default(),
            }),
            Class::ActorSoldier,
        ),
        (
            Entity::Civilian(ActorCivilian {
                element: element_data(ElementKind::ActorCivilian),
                actor: ActorData::default(),
                human: HumanData::default(),
                npc: NpcData::default(),
                civilian: CivilianData::default(),
            }),
            Class::ActorCivilian,
        ),
        (
            Entity::Fx(ElementFx {
                element: element_data(ElementKind::Fx),
                fx: FxData::default(),
            }),
            Class::Fx,
        ),
        (
            Entity::Fx(ElementFx {
                element: element_data(ElementKind::Fx),
                fx: FxData {
                    mobile_index: Some(0),
                    ..Default::default()
                },
            }),
            Class::FxMasked,
        ),
        (
            Entity::Target(ElementTarget {
                element: element_data(ElementKind::Target),
                fx: FxData::default(),
                target: TargetData::default(),
            }),
            Class::Target,
        ),
        (
            Entity::Bonus(ElementBonus {
                element: element_data(ElementKind::ObjectBonus),
                object: object_data(ObjectType::BonusAmulet),
            }),
            Class::Bonus,
        ),
        (
            Entity::Bonus(ElementBonus {
                element: element_data(ElementKind::ObjectOther),
                object: object_data(ObjectType::Ale),
            }),
            Class::Ale,
        ),
        (
            Entity::Bonus(ElementBonus {
                element: element_data(ElementKind::ObjectBonus),
                object: object_data(ObjectType::Cape),
            }),
            Class::Cape,
        ),
        (
            Entity::Scroll(ElementScroll {
                element: element_data(ElementKind::ObjectScroll),
                object: object_data(ObjectType::Scroll),
                ..Default::default()
            }),
            Class::Scroll,
        ),
    ];
    for (entity, expected) in cases {
        assert_eq!(entity.original_hourglass_class(), expected);
    }

    let projectile_cases = [
        (ObjectType::Arrow, Class::Arrow),
        (ObjectType::Apple, Class::Apple),
        (ObjectType::Stone, Class::Stone),
        (ObjectType::Purse, Class::Purse),
        (ObjectType::Coin, Class::Coin),
        (ObjectType::WaspNest, Class::WaspNest),
        (ObjectType::BonusWaspNest, Class::WaspNest),
        (ObjectType::Wasp, Class::Wasp),
    ];
    for (object_type, expected) in projectile_cases {
        let entity = Entity::Projectile(ElementProjectile {
            element: element_data(ElementKind::ObjectProjectile),
            object: object_data(object_type),
            projectile: ProjectileData::default(),
        });
        assert_eq!(
            entity.original_hourglass_class(),
            expected,
            "{object_type:?}"
        );
    }
    for object_type in [ObjectType::Net, ObjectType::BonusNet] {
        let entity = Entity::Net(ElementNet {
            element: element_data(ElementKind::ObjectNet),
            object: object_data(object_type),
            projectile: ProjectileData::default(),
            net: NetData::default(),
        });
        assert_eq!(entity.original_hourglass_class(), Class::Net);
    }
}

#[test]
#[should_panic(expected = "variant/ElementKind invariant failed")]
fn original_hourglass_mapping_rejects_mismatched_element_kind() {
    Entity::Projectile(ElementProjectile {
        element: element_data(ElementKind::ObjectBonus),
        object: object_data(ObjectType::Arrow),
        projectile: ProjectileData::default(),
    })
    .original_hourglass_class();
}

#[test]
#[should_panic(expected = "no original-game concrete-kind mapping")]
fn original_hourglass_mapping_rejects_unproved_variant_object_pair() {
    Entity::Bonus(ElementBonus {
        element: element_data(ElementKind::ObjectBonus),
        object: object_data(ObjectType::Coin),
    })
    .original_hourglass_class();
}

#[test]
fn entity_bonus_object_type_mapping_is_exhaustive_and_original_evidenced() {
    use OriginalBonusConcreteClass as Class;
    let cases = [
        (ObjectType::None, Class::Unsupported),
        (ObjectType::VirtualJumper, Class::Unsupported),
        (ObjectType::VirtualListen, Class::Unsupported),
        (ObjectType::Ale, Class::Ale),
        (ObjectType::Apple, Class::Unsupported),
        (ObjectType::Arrow, Class::Unsupported),
        (ObjectType::Stone, Class::Unsupported),
        (ObjectType::Purse, Class::Unsupported),
        (ObjectType::Coin, Class::Unsupported),
        (ObjectType::Net, Class::Unsupported),
        (ObjectType::Wasp, Class::Unsupported),
        (ObjectType::WaspNest, Class::Unsupported),
        (ObjectType::Scroll, Class::Unsupported),
        (ObjectType::Cape, Class::Cape),
        (ObjectType::BonusAmulet, Class::Bonus),
        (ObjectType::BonusAle, Class::Bonus),
        (ObjectType::BonusApple, Class::Bonus),
        (ObjectType::BonusArrow, Class::Bonus),
        (ObjectType::BonusBlazon, Class::Bonus),
        (ObjectType::BonusLambLeg, Class::Bonus),
        (ObjectType::BonusNet, Class::Bonus),
        (ObjectType::BonusPlants, Class::Bonus),
        (ObjectType::BonusPurse, Class::Bonus),
        (ObjectType::BonusRansom, Class::Bonus),
        (ObjectType::BonusStone, Class::Bonus),
        (ObjectType::BonusWaspNest, Class::Bonus),
        (ObjectType::BonusAmpulla, Class::Bonus),
        (ObjectType::BonusCoronationSpoon, Class::Bonus),
        (ObjectType::BonusRichardsCrown, Class::Bonus),
        (ObjectType::BonusRoyalSeal, Class::Bonus),
        (ObjectType::BonusRoyalSceptre, Class::Bonus),
        (ObjectType::BonusDomesdayBook, Class::Bonus),
        (ObjectType::BonusSwordOfTheState, Class::Bonus),
    ];
    for (object_type, expected) in cases {
        let bonus = ElementBonus {
            object: ObjectData {
                object_type,
                ..Default::default()
            },
            element: ElementData::default(),
        };
        assert_eq!(bonus.original_concrete_class(), expected, "{object_type:?}");
    }
}

#[test]
fn element_kind_type_checks() {
    assert!(ElementKind::ActorPc.is_actor());
    assert!(ElementKind::ActorPc.is_human());
    assert!(ElementKind::ActorPc.is_pc());
    assert!(!ElementKind::ActorPc.is_npc());

    assert!(ElementKind::ActorSoldier.is_actor());
    assert!(ElementKind::ActorSoldier.is_human());
    assert!(ElementKind::ActorSoldier.is_npc());
    assert!(ElementKind::ActorSoldier.is_soldier());
    assert!(!ElementKind::ActorSoldier.is_civilian());

    assert!(ElementKind::ActorCivilian.is_civilian());
    assert!(ElementKind::ActorCivilian.is_npc());

    assert!(ElementKind::Fx.is_fx());
    assert!(ElementKind::Target.is_fx());
    assert!(!ElementKind::Fx.is_actor());

    assert!(ElementKind::ObjectBonus.is_object());
    assert!(ElementKind::ObjectBonus.is_bonus());
    assert!(ElementKind::ObjectProjectile.is_projectile());
    assert!(ElementKind::ObjectNet.is_projectile());
}

#[test]
fn posture_checks() {
    assert!(Posture::Dead.is_dead());
    assert!(Posture::DeadBack.is_dead());
    assert!(!Posture::Upright.is_dead());

    assert!(Posture::Lying.is_lying());
    assert!(Posture::Tied.is_lying());
    assert!(!Posture::Upright.is_lying());
}

#[test]
fn target_sprite_visual_anchor_uses_preserved_3d_position() {
    let mut element = ElementData {
        kind: ElementKind::Target,
        ..ElementData::default()
    };
    element.set_position(WorldPoint3D::new(120.0, 80.0, 30.0));
    element.set_position_map_preserving_3d(MapPoint::new(10.0, 20.0));
    let target = Entity::Target(ElementTarget {
        element,
        fx: FxData::default(),
        target: TargetData::default(),
    });

    assert_eq!(
        target.sprite_visual_map_position(),
        MapPoint::new(120.0, 50.0)
    );
}

#[test]
fn ground_position_uses_stored_world_xy_not_projected_map_position() {
    let mut element = ElementData {
        kind: ElementKind::Target,
        ..ElementData::default()
    };
    element.set_position(WorldPoint3D::new(120.0, 80.0, 30.0));
    element.set_position_map_preserving_3d(MapPoint::new(10.0, 20.0));
    let target = Entity::Target(ElementTarget {
        element,
        fx: FxData::default(),
        target: TargetData::default(),
    });

    assert_eq!(target.ground_position(), GroundPoint::new(120.0, 80.0));
}

#[test]
fn target_hotspots_use_the_exact_cached_sprite_top_left() {
    let mut element = ElementData {
        kind: ElementKind::Target,
        ..ElementData::default()
    };
    element.sprite.center = crate::coordinates::SpriteAnchor::new(30.0, 140.0);
    element.set_position(WorldPoint3D::new(2821.0, 727.355, 416.355));
    element
        .sprite
        .position_iface
        .set_cached_sprite_position(MapPoint::new(2791.0, 171.0));
    element.set_position_map_preserving_3d(MapPoint::new(2823.0, 312.0));
    let target = Entity::Target(ElementTarget {
        element,
        fx: FxData::default(),
        target: TargetData::default(),
    });

    assert_eq!(
        target.gameplay_sprite_position(),
        crate::coordinates::SpriteTopLeft::new(2791.0, 171.0)
    );
}

/// Corpse-transition guard: a dead corpse can only flip to
/// `Carried`; every other posture write on a `Dead` / `DeadBack`
/// sprite is silently dropped.
#[test]
fn set_posture_undead_guard() {
    // Alive → any posture is applied normally.
    let mut elem = ElementData {
        posture: Posture::Upright,
        ..Default::default()
    };
    elem.set_posture(Posture::Lying);
    assert_eq!(elem.posture, Posture::Lying);
    elem.set_posture(Posture::Crouched);
    assert_eq!(elem.posture, Posture::Crouched);

    // Dead + non-Carried → silently dropped (no undead!).
    let mut dead = ElementData {
        posture: Posture::Dead,
        ..Default::default()
    };
    dead.set_posture(Posture::Upright);
    assert_eq!(
        dead.posture,
        Posture::Dead,
        "stun on corpse must not revive"
    );
    dead.set_posture(Posture::Lying);
    assert_eq!(dead.posture, Posture::Dead);
    dead.set_posture(Posture::Crouched);
    assert_eq!(dead.posture, Posture::Dead);

    // Dead + Carried → allowed (pickup corpse).
    dead.set_posture(Posture::Carried);
    assert_eq!(dead.posture, Posture::Carried);

    // Same semantics for DeadBack.
    let mut dead_back = ElementData {
        posture: Posture::DeadBack,
        ..Default::default()
    };
    dead_back.set_posture(Posture::Upright);
    assert_eq!(dead_back.posture, Posture::DeadBack);
    dead_back.set_posture(Posture::Carried);
    assert_eq!(dead_back.posture, Posture::Carried);

    // Once flipped to Carried the entity is no longer "dead
    // posture", so normal writes apply again — Carried lifts the
    // lock.
    dead_back.set_posture(Posture::Dead);
    assert_eq!(dead_back.posture, Posture::Dead);
}

#[test]
fn order_posture_publication_preserves_sprite_and_unconditional_semantics() {
    let mut element = ElementData::from_initial_posture(Posture::Upright);
    let sprite_before = element.sprite.position_iface.v48_serialized_state();
    element.publish_order_posture(Posture::Dead);
    assert_eq!(element.posture(), Posture::Dead);
    let sprite_after = element.sprite.position_iface.v48_serialized_state();
    assert_eq!(sprite_after.posture, sprite_before.posture);
    assert_eq!(sprite_after.old_posture, sprite_before.old_posture);
    element.publish_order_posture(Posture::Upright);
    assert_eq!(element.posture(), Posture::Upright);
    element.publish_order_posture(Posture::Carried);
    assert_eq!(element.posture(), Posture::Carried);
    assert_eq!(
        element.sprite.position_iface.v48_serialized_state().posture,
        sprite_before.posture
    );
}

#[test]
fn explicit_posture_apis_preserve_legacy_state_hash_and_wire_bytes() {
    use robin_util::state_hash::StateHash;
    use std::hash::Hasher;

    let mut legacy = ElementData {
        posture: Posture::DeadBack,
        ..Default::default()
    };
    let mut explicit = ElementData::from_initial_posture(Posture::DeadBack);
    for posture in [
        Posture::Upright,
        Posture::Lying,
        Posture::Dead,
        Posture::Carried,
    ] {
        // Historical order publication changed only this field, even
        // when the source posture was a corpse. Keep the exact encoding.
        legacy.posture = posture;
        explicit.publish_order_posture(posture);
        assert_eq!(bitcode::encode(&legacy), bitcode::encode(&explicit));
        let mut legacy_hash = std::collections::hash_map::DefaultHasher::new();
        let mut explicit_hash = std::collections::hash_map::DefaultHasher::new();
        legacy.state_hash(&mut legacy_hash);
        explicit.state_hash(&mut explicit_hash);
        assert_eq!(legacy_hash.finish(), explicit_hash.finish());
    }
}

#[test]
fn saved_posture_sources_remain_independent_until_explicit_v48_restore() {
    let element = ElementData::from_initial_posture(Posture::Dead);
    let sprite_posture = element.sprite.position_iface.v48_serialized_state().posture;
    let encoded = serde_json::to_vec(&element).expect("encode element save");
    let mut restored: ElementData = serde_json::from_slice(&encoded).expect("decode element save");
    assert_eq!(restored.posture(), Posture::Dead);
    assert_eq!(
        restored
            .sprite
            .position_iface
            .v48_serialized_state()
            .posture,
        sprite_posture
    );
    let mut position = restored.sprite.position_iface.v48_serialized_state();
    position.posture = Posture::Upright;
    restored.restore_v48_position_and_posture(position);
    assert_eq!(restored.posture(), Posture::Upright);
    assert_eq!(
        restored
            .sprite
            .position_iface
            .v48_serialized_state()
            .posture,
        Posture::Upright
    );
}

#[test]
fn entity_set_posture_updates_authoritative_posture() {
    let mut element = ElementData {
        posture: Posture::Upright,
        ..Default::default()
    };
    let mut saved_position = element.sprite.position_iface.v48_serialized_state();
    saved_position.posture = Posture::Upright;
    saved_position.old_posture = Posture::Upright;
    element
        .sprite
        .position_iface
        .restore_v48_serialized_state(saved_position);
    let mut entity = Entity::Soldier(ActorSoldier {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: SoldierData::default(),
    });

    entity.set_posture(Posture::Crouched);

    assert_eq!(entity.element_data().posture, Posture::Crouched);
    assert_eq!(entity.posture(), Posture::Crouched);
    let position = entity.position_iface().v48_serialized_state();
    assert_eq!(position.posture, Posture::Crouched);
    assert_eq!(position.old_posture, Posture::Upright);

    entity.set_posture(Posture::Dead);
    entity.set_posture(Posture::Upright);

    assert_eq!(entity.element_data().posture, Posture::Dead);
    assert_eq!(entity.posture(), Posture::Dead);
    assert_eq!(
        entity.position_iface().v48_serialized_state().posture,
        Posture::Dead,
        "a rejected corpse transition must not advance position posture"
    );
}

#[test]
fn posture_allows_transition_to() {
    // Non-dead → any.
    assert!(Posture::Upright.allows_transition_to(Posture::Lying));
    assert!(Posture::Lying.allows_transition_to(Posture::Upright));
    assert!(Posture::Crouched.allows_transition_to(Posture::Dead));
    // Dead → Carried only.
    assert!(Posture::Dead.allows_transition_to(Posture::Carried));
    assert!(Posture::DeadBack.allows_transition_to(Posture::Carried));
    assert!(!Posture::Dead.allows_transition_to(Posture::Upright));
    assert!(!Posture::Dead.allows_transition_to(Posture::Lying));
    assert!(!Posture::DeadBack.allows_transition_to(Posture::Dead));
}

#[test]
fn action_state_groups() {
    assert!(ActionState::Moving.is_moving());
    assert!(ActionState::MovingFast.is_moving());
    assert!(!ActionState::Waiting.is_moving());

    assert!(ActionState::AimingWithBow.is_bow());
    assert!(ActionState::AimingWithBowUp.is_bow());
    assert!(!ActionState::Waiting.is_bow());

    assert!(ActionState::WaitingSword.is_sword());
    assert!(ActionState::ParryingSwordLow.is_sword());
    assert!(!ActionState::Waiting.is_sword());

    assert!(ActionState::HoldingShield.is_shield());
    assert!(ActionState::ParryingShield.is_shield());
    assert!(ActionState::MovingShield.is_shield());
}

#[test]
fn camp_enemy() {
    assert!(Camp::Royalists.is_hostile_to(Camp::Lacklandists));
    assert!(Camp::Lacklandists.is_hostile_to(Camp::Royalists));
    assert!(!Camp::Royalists.is_hostile_to(Camp::Royalists));
    let ten_teams: Vec<_> = (0..10).map(Camp::from_allegiance_id).collect();
    for (left_index, left) in ten_teams.iter().enumerate() {
        for (right_index, right) in ten_teams.iter().enumerate() {
            assert_eq!(left.is_hostile_to(*right), left_index != right_index);
        }
    }
}

#[test]
fn invalid_camp_hostility_remains_nonhostile_after_warning() {
    assert!(!Camp::Error.is_hostile_to(Camp::Royalists));
}

#[test]
fn command_swordstrike() {
    assert!(Command::SwordstrikeThrustA.is_swordstrike());
    assert!(Command::SwordstrikeThrustI.is_swordstrike());
    assert!(!Command::Move.is_swordstrike());
    assert!(!Command::SwordstrikeTired.is_swordstrike());
}

#[test]
fn entity_serde_roundtrip() {
    let pc = ActorPc {
        element: ElementData {
            kind: ElementKind::ActorPc,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData::default(),
    };

    let entity = Entity::Pc(pc);
    let json = serde_json::to_string(&entity).unwrap();
    let back: Entity = serde_json::from_str(&json).unwrap();

    assert!(back.is_pc());
    assert!(back.is_actor());
    assert!(back.is_human());
    assert!(!back.is_npc());
}

#[test]
fn entity_sub_data_accessors() {
    let soldier = Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData {
            life_points: 75,
            ..NpcData::default()
        },
        soldier: SoldierData::default(),
    });

    assert!(soldier.element_data().active);
    assert!(soldier.actor_data().is_some());
    assert!(soldier.human_data().is_some());
    assert!(soldier.npc_data().is_some());
    assert!(soldier.object_data().is_none());
}

#[test]
fn entity_is_dead_dispatch() {
    let alive = Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData {
            life_points: 50,
            ..NpcData::default()
        },
        soldier: SoldierData::default(),
    });
    assert!(!alive.is_dead());

    let dead = Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData {
            life_points: 0,
            ..NpcData::default()
        },
        soldier: SoldierData::default(),
    });
    assert!(dead.is_dead());

    // Non-actors default to dead = true
    let fx = Entity::Fx(ElementFx {
        element: ElementData {
            kind: ElementKind::Fx,
            ..ElementData::default()
        },
        fx: FxData::default(),
    });
    assert!(fx.is_dead());
}

#[test]
fn bonus_is_relic() {
    let mut bonus = ElementBonus {
        element: ElementData {
            kind: ElementKind::ObjectBonus,
            ..ElementData::default()
        },
        object: ObjectData {
            object_type: ObjectType::BonusRichardsCrown,
            ..ObjectData::default()
        },
    };
    assert!(bonus.is_relic());

    bonus.object.object_type = ObjectType::BonusArrow;
    assert!(!bonus.is_relic());
}

#[test]
fn trait_human_on_soldier() {
    let s = ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData {
            life_points: 50,
            ..NpcData::default()
        },
        soldier: SoldierData {
            cached_max_life_points: 80,
            cached_camp: Camp::Lacklandists,
            ..SoldierData::default()
        },
    };

    assert_eq!(Human::life_points(&s), 50);
    assert!(!s.is_out_of_order());
    assert_eq!(s.camp(), Camp::Lacklandists);
    assert!(s.is_able_to_fight());
}

#[test]
fn trait_human_on_pc_uses_authored_camp() {
    let pc = ActorPc {
        element: ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData {
            cached_camp: Camp::Custom(7),
            ..PcData::default()
        },
    };

    assert_eq!(Human::camp(&pc), Camp::Custom(7));
    assert_eq!(Entity::Pc(pc).camp(), Camp::Custom(7));
}

#[test]
fn trait_human_dead_soldier() {
    let s = ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData {
            life_points: 0,
            ..NpcData::default()
        },
        soldier: SoldierData::default(),
    };

    assert!(Element::is_dead(&s));
    assert!(s.is_out_of_order());
    assert!(!s.is_able_to_fight());
}

#[test]
fn trait_pc_immortal() {
    let pc = ActorPc {
        element: ElementData {
            kind: ElementKind::ActorPc,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData {
            immortal: true,
            ..PcData::default()
        },
    };

    assert!(Element::is_immortal(&pc));
    assert!(!pc.is_robin()); // robin defaults to false
}

// ═══════════════════════════════════════════════════════════════
//  Cross-module integration tests
// ═══════════════════════════════════════════════════════════════

#[test]
fn entity_sprite_reference() {
    use crate::sprite::Sprite;

    let mut entity = Entity::Fx(ElementFx {
        element: ElementData {
            kind: ElementKind::Fx,
            ..ElementData::default()
        },
        fx: FxData::default(),
    });

    // Every entity carries a sprite by default (non-Option).
    assert_eq!(entity.sprite().current_frame, 0);

    // Re-attach a fresh sprite.
    entity.element_data_mut().sprite = Sprite::default();
    assert_eq!(entity.sprite().current_frame, 0);
}

#[test]
fn entity_grid_cell_tracking() {
    let mut data = ElementData {
        kind: ElementKind::ActorPc,
        ..ElementData::default()
    };
    data.set_position_map(MapPoint { x: 200.0, y: 300.0 });

    data.update_grid_cell();
    // 200/64 = 3, 300/64 = 4
    assert_eq!(data.grid_cell, Some((3, 4)));
}

#[test]
fn lying_stars_point_uses_floored_sprite_top_left_plus_hotspot() {
    use crate::sprite::Sprite;
    use crate::sprite_script::SpriteScript;
    use std::sync::Arc;

    let mut sprite = Sprite {
        center: crate::coordinates::SpriteAnchor { x: 30.0, y: 70.0 },
        scripts: Arc::new(vec![SpriteScript {
            hotspot: crate::coordinates::SpriteLocalPoint::new(46.0, 18.0),
            ..SpriteScript::default()
        }]),
        ..Sprite::default()
    };
    sprite.current_row = 0;

    let mut element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Lying,
        sprite,
        ..ElementData::default()
    };
    element.set_position_map(MapPoint {
        x: 200.75,
        y: 300.75,
    });

    let entity = Entity::Soldier(ActorSoldier {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: SoldierData::default(),
    });

    let stars = entity.compute_stars_point().unwrap();

    assert_eq!(stars.x, 216.0);
    assert_eq!(stars.y - stars.z, 248.0);
    assert_eq!(stars.z, 5.0);
}

#[test]
fn npc_ai_controller_reference() {
    use crate::ai::AiState;
    use crate::ai_enemy::EnemyAi;

    let entity_id = EntityId::Pc(crate::entity_id::PcId(42));
    let mut enemy_ai = EnemyAi::new(7);
    enemy_ai.base.owner_entity_id = Some(entity_id);
    assert_eq!(enemy_ai.base.me, 7);
    assert_eq!(enemy_ai.base.owner_entity_id, Some(entity_id));
    assert_eq!(enemy_ai.base.current_state, AiState::Default);

    let mut soldier = ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: SoldierData::default(),
    };

    // Attach AI brain
    enemy_ai.base.current_state = AiTopState::Attacking;
    enemy_ai.base.current_substate = AiSubstate::AttackingSwordfight;
    soldier.npc.ai_brain = AiBrain::Enemy(Box::new(enemy_ai));

    // Access via Entity enum
    let entity = Entity::Soldier(soldier);
    assert_eq!(entity.ai_state(), Some(AiTopState::Attacking));

    let ai_ref = entity.ai_controller().unwrap();
    assert_eq!(ai_ref.owner_entity_id, Some(entity_id));

    // Can also access the enemy-specific subclass
    assert!(entity.enemy_ai().is_some());
}

#[test]
fn entity_cross_module_serde_roundtrip() {
    use crate::ai_enemy::EnemyAi;

    // Verify that entities with the new fields still serialize/deserialize.
    let mut enemy_ai = EnemyAi::new(0);
    enemy_ai.base.current_state = AiTopState::Seeking;
    let soldier = Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData {
            ai: AiActorData {
                ai_brain: AiBrain::Enemy(Box::new(enemy_ai)),
                ..AiActorData::default()
            },
            ..NpcData::default()
        },
        soldier: SoldierData::default(),
    });

    let json = serde_json::to_string(&soldier).unwrap();
    let back: Entity = serde_json::from_str(&json).unwrap();

    assert!(back.is_soldier());
    assert_eq!(back.ai_state(), Some(AiTopState::Seeking));
    // Sprite is now serialised (no serde-skip), so it survives round-trip.
    assert_eq!(back.sprite().current_frame, 0);
}

#[test]
fn swordfight_opponents_preserve_record_pairing_during_mutation() {
    let first = EntityId::Pc(crate::entity_id::PcId(11));
    let second = EntityId::Soldier(crate::entity_id::SoldierId(12));
    let first_line = JumpLineIndex::new(21).unwrap();
    let second_line = JumpLineIndex::new(22).unwrap();
    let promoted_line = JumpLineIndex::new(23).unwrap();
    let ignored_line = JumpLineIndex::new(24).unwrap();
    let mut opponents =
        SwordfightOpponents::from_pairs([(first, Some(first_line)), (second, Some(second_line))]);

    assert!(!opponents.add_principal(second, Some(promoted_line)));
    assert_eq!(opponents.ids(), vec![second, first]);
    assert_eq!(opponents.jump_line(0), Some(promoted_line));
    assert_eq!(opponents.jump_line(1), Some(first_line));

    // The original game leaves the existing principal's line alone when adding an opponent.
    assert!(!opponents.add_principal(second, Some(ignored_line)));
    assert_eq!(opponents.jump_line(0), Some(promoted_line));

    assert!(opponents.remove(first));
    assert_eq!(opponents.ids(), vec![second]);
    assert_eq!(opponents.jump_line(0), Some(promoted_line));
}

#[test]
fn swordfight_opponents_standalone_serde_is_an_ordered_record_sequence() {
    let first = SwordfightOpponent::new(
        EntityId::Pc(crate::entity_id::PcId(25)),
        Some(JumpLineIndex::new(35).unwrap()),
    );
    let second = SwordfightOpponent::new(EntityId::Soldier(crate::entity_id::SoldierId(26)), None);
    let entries = vec![first, second];
    let opponents = SwordfightOpponents::from_entries(entries.clone());

    let json = serde_json::to_value(&opponents).unwrap();
    assert!(json.is_array(), "private aggregate storage must not leak");
    assert_eq!(json, serde_json::to_value(&entries).unwrap());
    assert_eq!(
        serde_json::from_value::<SwordfightOpponents>(json).unwrap(),
        opponents
    );

    let binary = bitcode::encode(&opponents);
    let decoded: SwordfightOpponents = bitcode::decode(&binary).unwrap();
    assert_eq!(decoded, opponents);
}

#[test]
fn human_opponents_round_trip_preserves_ordered_pairs() {
    let first = EntityId::Pc(crate::entity_id::PcId(31));
    let second = EntityId::Soldier(crate::entity_id::SoldierId(32));
    let line = JumpLineIndex::new(41).unwrap();
    let human = HumanData {
        sorting_distance: 123.0,
        opponents: SwordfightOpponents::from_pairs([(first, Some(line)), (second, None)]),
        ..HumanData::default()
    };
    let value = serde_json::to_value(&human).unwrap();
    assert_eq!(
        value["opponents"],
        serde_json::to_value(&human.opponents).unwrap()
    );
    let round_trip: HumanData = serde_json::from_value(value).unwrap();
    assert_eq!(round_trip.opponents, human.opponents);
    assert_eq!(round_trip.sorting_distance, 0.0);
    let binary = bitcode::encode(&human);
    let binary_round_trip: HumanData = bitcode::decode(&binary).unwrap();
    assert_eq!(binary_round_trip.opponents, human.opponents);
}

#[test]
fn entity_position_iface_accessor() {
    let pc = Entity::Pc(ActorPc {
        element: ElementData {
            kind: ElementKind::ActorPc,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData::default(),
    });

    // PI now always exists (it lives on every sprite).
    assert_eq!(
        pc.position_iface().get_direction(),
        crate::position_interface::Direction::NORTH
    );
}

#[test]
fn action_slot_disabled_combines_sparse_permanent_and_temporary_state() {
    let mut pc = PcData::default();
    assert!(!pc.action_slot_disabled(0));
    pc.disabled_actions = vec![false, true];
    pc.disabled_actions_temp = vec![true];
    assert!(pc.action_slot_disabled(0));
    assert!(pc.action_slot_disabled(1));
    assert!(!pc.action_slot_disabled(2));
}

#[test]
fn entity_is_unconscious_reads_human_flag_and_is_false_for_non_humans() {
    let mut soldier = Entity::Soldier(ActorSoldier {
        element: element_data(ElementKind::ActorSoldier),
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: SoldierData::default(),
    });
    assert!(!soldier.is_unconscious());
    soldier.human_data_mut().unwrap().unconscious = true;
    assert!(soldier.is_unconscious());

    let mut pc = Entity::Pc(ActorPc {
        element: element_data(ElementKind::ActorPc),
        actor: ActorData::default(),
        human: HumanData {
            unconscious: true,
            ..HumanData::default()
        },
        pc: PcData::default(),
    });
    assert!(pc.is_unconscious());
    pc.human_data_mut().unwrap().unconscious = false;
    assert!(!pc.is_unconscious());

    // Non-humans have no consciousness to lose: never unconscious.
    let fx = Entity::Fx(ElementFx {
        element: element_data(ElementKind::Fx),
        fx: FxData::default(),
    });
    assert!(fx.human_data().is_none());
    assert!(!fx.is_unconscious());
}

#[test]
fn entity_is_vip_reads_enemy_ai_flag_only_for_soldiers() {
    use crate::ai_enemy::EnemyAi;

    let mut soldier = Entity::Soldier(ActorSoldier {
        element: element_data(ElementKind::ActorSoldier),
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: SoldierData::default(),
    });
    // No enemy brain attached yet: not a VIP.
    assert!(!soldier.is_vip());

    let mut enemy_ai = EnemyAi::new(3);
    enemy_ai.is_vip = true;
    match &mut soldier {
        Entity::Soldier(s) => s.npc.ai_brain = AiBrain::Enemy(Box::new(enemy_ai)),
        _ => unreachable!(),
    }
    assert!(soldier.is_vip());

    match &mut soldier {
        Entity::Soldier(s) => s.npc.ai_brain.enemy_mut().unwrap().is_vip = false,
        _ => unreachable!(),
    }
    assert!(!soldier.is_vip());

    // PCs and non-humans are never VIPs by this (EnemyAi-cached) definition.
    let pc = Entity::Pc(ActorPc {
        element: element_data(ElementKind::ActorPc),
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData::default(),
    });
    assert!(!pc.is_vip());
    let fx = Entity::Fx(ElementFx {
        element: element_data(ElementKind::Fx),
        fx: FxData::default(),
    });
    assert!(!fx.is_vip());
}

/// SHA-256 digests of the bitcode bytes, the JSON string and the `StateHash`
/// byte stream (native-endian writes, recorded on a little-endian host).
fn human_golden_digests(human: &HumanData) -> [String; 3] {
    use robin_util::state_hash::StateHash;
    use sha2::Digest;

    struct ByteRecorder(Vec<u8>);
    impl std::hash::Hasher for ByteRecorder {
        fn finish(&self) -> u64 {
            unreachable!("state-hash byte recorder is never finished")
        }
        fn write(&mut self, bytes: &[u8]) {
            self.0.extend_from_slice(bytes);
        }
    }

    let sha = |bytes: &[u8]| hex::encode(sha2::Sha256::digest(bytes));
    let mut recorder = ByteRecorder(Vec::new());
    human.state_hash(&mut recorder);
    [
        sha(&bitcode::encode(human)),
        sha(serde_json::to_string(human).unwrap().as_bytes()),
        sha(&recorder.0),
    ]
}

fn golden_human_fixture() -> HumanData {
    use crate::entity_id::{PcId, SoldierId};

    let mut shield = HumanShieldState {
        top_plane: HumanPlaneState {
            normal: WorldPoint3D::new(0.0, 0.0, 1.0),
            az: 1.5,
            d: -2.0,
            ..HumanPlaneState::default()
        },
        box_3d: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        on_ground: true,
        ..HumanShieldState::default()
    };
    shield.points[1] = HumanShieldPointState {
        obstacle: [0.5, 1.5, 2.5, 3.5],
        polygon: MapPoint::new(8.0, 9.0),
    };
    shield.ground_box.bounds_are_set = true;
    shield.ground_box.bottom_right = MapPoint::new(30.0, 31.0);

    HumanData {
        carrier: Some(EntityId::Pc(PcId(3))),
        sorting_distance: 0.0,
        concussion_of_the_brain: 11,
        concussion_healing_timeout: 12,
        tiredness: 13,
        unconscious: true,
        already_detectable_body: true,
        detectable_list_index: 14,
        sword_strike_boredom: vec![1, 2, 3],
        stuck_under_nets_counter: 15,
        hollow_man: true,
        opponents: SwordfightOpponents::from_pairs([
            (
                EntityId::Soldier(SoldierId(21)),
                Some(JumpLineIndex::new(35).unwrap()),
            ),
            (EntityId::Pc(PcId(22)), None),
            (
                EntityId::Soldier(SoldierId(23)),
                Some(JumpLineIndex::new(2).unwrap()),
            ),
        ]),
        smalltalk_initiative: true,
        received_smalltalk_initiative: true,
        smalltalk_hint: SmalltalkHint::Legs,
        smalltalk_hint_opponent: Some(EntityId::Soldier(SoldierId(24))),
        relative_fighting_ability: 16,
        small_repulsive_radius: true,
        killed_by_accident: true,
        parry_counter: 17,
        invulnerable: true,
        last_motion_was_step_back_in_combat: true,
        running_hulk: 18,
        time_hulk: 19,
        hulk_level: 20,
        hulk_direction: true,
        hulk_speed: 0.75,
        repulsive_point: HumanRepulsivePointState {
            position: MapPoint::new(1.0, 2.0),
            concave: true,
            limit_left: MapPoint::new(3.0, 4.0),
            limit_right: MapPoint::new(5.0, 6.0),
            action_radius: 7.0,
            force_a: 8.0,
            force_b: 9.0,
            radius: 10.0,
            id: 42,
            affects_pcs: true,
            affects_soldiers: false,
            affects_civilians: true,
            affects_animals: false,
        },
        building_sector: SectorHandle::new(7),
        produced_noise_first_word: 3.25,
        shield,
        sword_sweep: HumanSwordSweepState {
            victims: vec![EntityId::Pc(PcId(5)), EntityId::Soldier(SoldierId(6))],
            initial_angle: 0.5,
            current_angle: 1.0,
            final_angle: 1.5,
        },
        pending_shoots: vec![
            crate::sequence::SequenceElementRef::new(crate::sequence::SequenceId(4), 2),
            crate::sequence::SequenceElementRef::new(crate::sequence::SequenceId(9), 0),
        ],
    }
}

/// Check save, native snapshot and state-hash persistence semantics.
/// Sorting scratch is excluded from persisted values, while the state-hash
/// derive writes its fixed skipped-field marker into the structural byte stream.
#[test]
fn human_data_roundtrips_preserve_opponents_and_sorting_scratch_semantics() {
    let mut human = golden_human_fixture();
    let persisted_digests = human_golden_digests(&human);

    human.sorting_distance = 1234.5;
    let runtime_digests = human_golden_digests(&human);
    assert_ne!(runtime_digests[0], persisted_digests[0]);
    assert_eq!(runtime_digests[1..], persisted_digests[1..]);

    let json = serde_json::to_string(&human).unwrap();
    let from_json: HumanData = serde_json::from_str(&json).unwrap();
    assert_eq!(from_json.sorting_distance, 0.0);
    assert_eq!(from_json.opponents, human.opponents);
    assert_eq!(human_golden_digests(&from_json), persisted_digests);

    let from_bitcode: HumanData = bitcode::decode(&bitcode::encode(&human)).unwrap();
    assert_eq!(from_bitcode.sorting_distance, 1234.5);
    assert_eq!(human_golden_digests(&from_bitcode), runtime_digests);
}

/// SHA-256 digests of the entity table's bitcode bytes, the world save JSON
/// (produced by the real `WorldState` save projection) and the entity
/// table's `StateHash` byte stream (native-endian, little-endian host).
fn world_entities_golden_digests(world: &crate::engine::state::WorldState) -> [String; 3] {
    use robin_util::state_hash::StateHash;
    use sha2::Digest;

    struct ByteRecorder(Vec<u8>);
    impl std::hash::Hasher for ByteRecorder {
        fn finish(&self) -> u64 {
            unreachable!("state-hash byte recorder is never finished")
        }
        fn write(&mut self, bytes: &[u8]) {
            self.0.extend_from_slice(bytes);
        }
    }

    let persisted = world.persisted_clone();
    robin_util::persistence_validation::validate(&persisted).expect("valid world save");
    let sha = |bytes: &[u8]| hex::encode(sha2::Sha256::digest(bytes));
    let mut recorder = ByteRecorder(Vec::new());
    world.entities.state_hash(&mut recorder);
    [
        sha(&bitcode::encode(&world.entities)),
        sha(serde_json::to_string(&persisted).unwrap().as_bytes()),
        sha(&recorder.0),
    ]
}

fn golden_element_fixture(kind: ElementKind, seed: u16) -> ElementData {
    let seed_f = f32::from(seed);
    let mut sprite = Sprite::default();
    sprite
        .position_iface
        .set_map_position(MapPoint::new(12.5 + seed_f, -7.25));
    sprite.current_row = 7 + seed;
    sprite.current_frame = 3;
    sprite.last_processed_order_id = 1234;
    sprite.masked = true;
    sprite.display_order_ref = Some(EntityId::Soldier(crate::entity_id::SoldierId(9)));
    sprite.anims_to_be_replaced = vec![OrderType::WaitingUpright, OrderType::WaitingUprightBored];
    sprite.replacing_anims = vec![
        OrderType::TransitionWaitingUprightBoredWaitingUpright,
        OrderType::TransitionWaitingUprightWaitingUprightBored,
    ];
    sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript::default()]);
    sprite.conversion = std::sync::Arc::new(vec![1, 2, 3]);
    sprite.frame_profile_name = format!("profile{seed}");
    sprite.profile_cache_key = "profile#primary".to_owned();
    sprite.alternate_profile_cache_key = "profile#alternate".to_owned();
    ElementData {
        kind,
        blipped: true,
        class_id: 100 + seed,
        active: false,
        hidden_in_building: true,
        sprite_id: 200 + u32::from(seed),
        select_id: 300,
        position_map_delayed: true,
        delayed_map_position: MapPoint::new(4.5, 5.5 + seed_f),
        position_delayed: true,
        delayed_position: WorldPoint3D::new(1.0, 2.0, 3.0 + seed_f),
        in_honolulu: true,
        index_in_elements_list: 17 + seed,
        custom_minimap_dot: 5,
        outline_colors: [1, 2, 3, 4, 5],
        current_outline: OutlineColorName::Striking,
        outline_width: 9,
        unreachable: true,
        posture: Posture::Lying,
        sprite,
        grid_cell: Some((3 + seed, 4)),
    }
}

fn golden_ai_actor_fixture(ai_brain: AiBrain, seed: u16) -> AiActorData {
    let seed_f = f32::from(seed);
    let detectable = |id: u32, detectable_type| Detectable {
        element: Some(EntityId::Pc(crate::entity_id::PcId(id))),
        detectable_type,
        seen_last_frame: true,
        heard_last_frame: false,
        seen_now: true,
        shadow_seen_now: true,
        shadow_seen_last_frame: false,
        last_visibility: 0.25 * id as f32,
    };
    AiActorData {
        register_number: 40 + seed,
        number_of_arrows: 6,
        direction_old: -3,
        initial_view_direction: MapVec::new(0.5, -0.5 + seed_f),
        initial_position_x: 11.0,
        initial_position_y: 12.0,
        initial_position_sector: SectorHandle::new(8),
        initial_position_level: 2,
        inform_my_friends: true,
        money: 77,
        wasp_victim: true,
        old_cover_noise_deafness: 13,
        old_cover_noise_deafness_frame_counter: 14,
        stuck_on_ladder_emergency_counter: 15,
        attached_scroll: Some(EntityId::Scroll(crate::entity_id::ScrollId(6))),
        body_visitors: 16,
        fried_pikachu: true,
        detectable_lists: vec![
            vec![
                detectable(1, DetectableType::Enemy),
                detectable(2, DetectableType::Body),
            ],
            vec![],
            vec![detectable(3, DetectableType::Beggar)],
        ],
        detection_suspects: [1, 2, 3, 4, 5, 6],
        maximal_detection_suspect: 18,
        worst_detected_type: DetectableType::Friend,
        has_given_money_to_beggar: true,
        custom_values: [-1, 2, -3, 4, -5, 6, -7, 8, -9, 10],
        ai_brain,
        alerted: true,
        view_radius: 19,
        eye_status: EyeStatus::Stare,
        half_aperture: 0.1,
        real_half_aperture: 0.2,
        view_angle: 0.3,
        view_angle_step: 0.4,
        view_transition: true,
        view_half_angle_range: 0.5,
        view_angle_iterator: 0.6,
        view_angle_iterator_step: 0.7,
        view_radius_base: 20,
        view_radius_goal: 21,
        view_radius_step: 22,
        view_alpha_start: 23,
        view_longrange_radius_factor: 0.8,
        view_half_aperture_cosine: 0.9,
        view_future_half_aperture: 1.1,
        view_half_aperture_step: 1.2,
        view_half_aperture_changes: true,
        view_crazy_angle_iterator: 1.3,
        view_crazy_angle_iterator_step: 1.4,
        view_crazy_color_iterator: 24,
        view_crazy_half_angle_range: 1.5,
        view_direction: [1.6, 1.7],
        view_left_side: [1.8, 1.9],
        view_right_side: [2.1, 2.2],
        view_lean_out: true,
        drunken_cone_iterators: [2.3, 2.4, 2.5, 2.6],
        view_radius_reduction_permil: 25,
        view_sniper: true,
        stare_point: GroundPoint::new(2.7, 2.8),
        follow_target: Some(EntityId::Pc(crate::entity_id::PcId(4))),
    }
}

fn golden_entities_fixture() -> crate::entities::Entities {
    // Exercise persistent runtime values and immutable behavior identity.
    let enemy = EnemyAi {
        pc_gone_away_in_this_direction: 3,
        thirsty: true,
        previous_state: crate::ai::StoredEnumWord::new(crate::ai::AiState::Default),
        previous_substate: crate::ai::StoredEnumWord::new(crate::ai::Substate::DefaultOnPost),
        forced_next_battle_decision: crate::ai::Decision::None,
        behavior_profile: crate::profiles::SoldierProfileIdx(7),
        sword_range: 40,
        ..EnemyAi::default()
    };
    let friendly = FriendlyAi {
        beggar_dont_talk_counter: 5,
        wants_to_talk: false,
        ..FriendlyAi::default()
    };
    let human = golden_human_fixture();
    crate::entities::Entities::from_legacy_slots(vec![
        Some(Entity::Pc(ActorPc {
            element: golden_element_fixture(ElementKind::ActorPc, 0),
            actor: ActorData::default(),
            human: human.clone(),
            pc: PcData::default(),
        })),
        None,
        Some(Entity::Soldier(ActorSoldier {
            element: golden_element_fixture(ElementKind::ActorSoldier, 1),
            actor: ActorData::default(),
            human: human.clone(),
            npc: NpcData {
                life_points: 42,
                ai: golden_ai_actor_fixture(AiBrain::Enemy(Box::new(enemy)), 1),
            },
            soldier: SoldierData {
                apple_smell: 3,
                rider: true,
                ..SoldierData::default()
            },
        })),
        Some(Entity::Civilian(ActorCivilian {
            element: golden_element_fixture(ElementKind::ActorCivilian, 2),
            actor: ActorData::default(),
            human,
            npc: NpcData {
                life_points: -7,
                ai: golden_ai_actor_fixture(AiBrain::Friendly(Box::new(friendly)), 2),
            },
            civilian: CivilianData {
                current_scroll_set: 4,
                ..CivilianData::default()
            },
        })),
        Some(Entity::Fx(ElementFx {
            element: golden_element_fixture(ElementKind::Fx, 3),
            fx: FxData::default(),
        })),
        Some(Entity::Target(ElementTarget {
            element: golden_element_fixture(ElementKind::Target, 4),
            fx: FxData::default(),
            target: TargetData::default(),
        })),
        Some(Entity::Bonus(ElementBonus {
            element: golden_element_fixture(ElementKind::ObjectOther, 5),
            object: ObjectData::default(),
        })),
        Some(Entity::Scroll(ElementScroll {
            element: golden_element_fixture(ElementKind::ObjectOther, 6),
            object: ObjectData::default(),
            presence: [true, false, true],
            tutorial: true,
            script_class: "ScrollScript".to_owned(),
            script_hourglass_timeout: 21,
        })),
        Some(Entity::Projectile(ElementProjectile {
            element: golden_element_fixture(ElementKind::ObjectOther, 7),
            object: ObjectData::default(),
            projectile: ProjectileData::default(),
        })),
        Some(Entity::Net(ElementNet {
            element: golden_element_fixture(ElementKind::ObjectOther, 8),
            object: ObjectData::default(),
            projectile: ProjectileData::default(),
            net: NetData::default(),
        })),
        None,
    ])
}

/// World save (JSON through `WorldState`), native snapshot (bitcode)
/// and state-hash encodings of the entity table survive serialization intact.
#[test]
fn entity_table_encodings_round_trip() {
    // Debug-build serde of the nested AI owners needs more than the default
    // test-thread stack.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(entity_table_encodings_round_trip_body)
        .expect("spawn entity encoding thread")
        .join()
        .expect("entity encoding thread panicked");
}

fn entity_table_encodings_round_trip_body() {
    let mut world = crate::engine::state::WorldState::new();
    world.entities = golden_entities_fixture();
    let expected = world_entities_golden_digests(&world);

    let json = serde_json::to_string(&world.persisted_clone()).unwrap();
    let decoded: crate::engine::state::WorldState = serde_json::from_str(&json).unwrap();
    let mut restored = crate::engine::state::WorldState::new();
    restored.entities = decoded.entities;
    assert_eq!(world_entities_golden_digests(&restored), expected);
    let soldier = restored
        .entities
        .get(EntityId::Soldier(crate::entity_id::SoldierId(2)))
        .and_then(Entity::npc_data)
        .expect("restored soldier");
    assert!(soldier.ai_brain.enemy().is_some());
    assert!(
        restored
            .entities
            .get(EntityId::Pc(crate::entity_id::PcId(1)))
            .is_none()
    );

    let from_bitcode: crate::entities::Entities =
        bitcode::decode(&bitcode::encode(&world.entities)).unwrap();
    restored.entities = from_bitcode;
    assert_eq!(world_entities_golden_digests(&restored), expected);
}

/// `WorldState::persisted_clone` is also used without serialization (replay
/// save markers, rollback-safe snapshots), so its in-memory entity projection
/// must equal a JSON save/load round trip, including runtime-only state that
/// neither bitcode nor the state hash observe.
#[test]
fn entity_persisted_projection_matches_json_round_trip() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(entity_persisted_projection_matches_json_round_trip_body)
        .expect("spawn projection thread")
        .join()
        .expect("projection thread panicked");
}

fn entity_persisted_projection_matches_json_round_trip_body() {
    use robin_util::state_hash::compute;

    let mut entities = golden_entities_fixture();
    let soldier_id = EntityId::Soldier(crate::entity_id::SoldierId(2));
    let civilian_id = EntityId::Civilian(crate::entity_id::CivilianId(3));
    for id in [soldier_id, civilian_id] {
        let entity = entities.get_mut(id).expect("fixture npc");
        let ai = entity.ai_controller_mut().expect("fixture NPC brain");
        ai.script_locked = true;
        ai.stimulus_queue = vec![
            crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer),
            crate::ai::Stimulus::new(crate::ai::StimulusType::EventDone),
        ];
        let sprite = &mut entity.element_data_mut().sprite;
        sprite.alternate_scripts = Some(std::sync::Arc::new(vec![
            crate::sprite_script::SpriteScript::default(),
        ]));
        sprite.alternate_conversion = Some(std::sync::Arc::new(vec![4, 5]));
        sprite.last_motion_state = Some(crate::sprite::MotionState::InProgress);
    }

    let projected = entities.persisted_projection();
    let json_round_trip: crate::entities::Entities =
        serde_json::from_str(&serde_json::to_string(&entities).unwrap()).unwrap();

    assert_eq!(
        bitcode::encode(&projected),
        bitcode::encode(&json_round_trip)
    );
    assert_eq!(compute(&projected), compute(&json_round_trip));
    assert_eq!(
        serde_json::to_string(&projected).unwrap(),
        serde_json::to_string(&json_round_trip).unwrap()
    );
    assert_eq!(bitcode::encode(&projected), bitcode::encode(&entities));
    assert_eq!(compute(&projected), compute(&entities));

    let runtime_only = |entities: &crate::entities::Entities, id| {
        let entity = entities.get(id).unwrap();
        let sprite = &entity.element_data().sprite;
        (
            sprite.scripts.len(),
            sprite.alternate_scripts.is_some(),
            sprite.conversion.len(),
            sprite.alternate_conversion.is_some(),
            sprite.last_motion_state,
        )
    };
    for id in [soldier_id, civilian_id] {
        assert_eq!(
            runtime_only(&entities, id),
            (
                1,
                true,
                3,
                true,
                Some(crate::sprite::MotionState::InProgress),
            )
        );
        assert_eq!(
            runtime_only(&projected, id),
            runtime_only(&json_round_trip, id)
        );
        assert_eq!(runtime_only(&projected, id), (0, false, 0, false, None));
    }

    // The world save capture is the projection, and restores it unchanged.
    let mut world = crate::engine::state::WorldState::new();
    world.entities = entities;
    let restored = world.persisted_clone().entities;
    for id in [soldier_id, civilian_id] {
        assert_eq!(
            runtime_only(&restored, id),
            runtime_only(&json_round_trip, id)
        );
    }
}

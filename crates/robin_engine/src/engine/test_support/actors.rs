//! Shared synthetic actors. These constructors supply real allegiance and
//! pathfinder identities; callers still register actors through `add_test_entity`
//! and call `complete_test_runtime_fixture` when a scenario needs live assets.
//! Invalid-data tests should construct their invalid inputs explicitly.

use crate::element::Entity;

/// Give one fixture actor an independently authored behavior profile.
pub(crate) fn edit_enemy_profile(
    assets: &mut crate::engine::types::LevelAssets,
    ai: &mut crate::ai_enemy::EnemyAi,
    edit: impl FnOnce(&mut crate::profiles::SoldierProfile),
) {
    let mut profile = ai.profile(&assets.profile_manager).clone();
    edit(&mut profile);
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    let index = crate::profiles::SoldierProfileIdx(
        u32::try_from(profiles.soldiers.len()).expect("fixture profile count"),
    );
    profiles.soldiers.push(profile);
    ai.behavior_profile = index;
}

/// Low-level actor fixture without a pathfinder, profile, or allegiance.
/// Use `make_test_soldier` for scenarios that need a loaded enemy instead.
pub(crate) fn unbound_soldier(posture: crate::element::Posture) -> crate::element::ActorSoldier {
    let mut element = crate::element::ElementData::from_initial_posture(posture);
    element.kind = crate::element::ElementKind::ActorSoldier;
    crate::element::ActorSoldier {
        element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }
}

/// Low-level PC fixture without a pathfinder or campaign profile binding.
pub(crate) fn unbound_pc(posture: crate::element::Posture) -> crate::element::ActorPc {
    let mut element = crate::element::ElementData::from_initial_posture(posture);
    element.kind = crate::element::ElementKind::ActorPc;
    crate::element::ActorPc {
        element,
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    }
}

/// Explicit actor fixture builder over [`unbound_soldier`] / [`unbound_pc`].
///
/// Soldiers start with the enemy allegiance and allow an explicit `.camp(...)`
/// override. Other scenario properties are specified at the call site; this
/// builder supplies no pathfinder or profile binding.
pub(crate) struct TestActor {
    entity: Entity,
}

impl TestActor {
    pub(crate) fn soldier(posture: crate::element::Posture) -> Self {
        Self {
            entity: Entity::Soldier(crate::element::ActorSoldier {
                soldier: crate::element::SoldierData {
                    cached_camp: crate::element::Camp::Lacklandists,
                    ..Default::default()
                },
                ..unbound_soldier(posture)
            }),
        }
    }

    pub(crate) fn pc(posture: crate::element::Posture) -> Self {
        Self {
            entity: Entity::Pc(unbound_pc(posture)),
        }
    }

    /// Overwrite the element kind without changing the entity variant.
    pub(crate) fn element_kind(mut self, kind: crate::element::ElementKind) -> Self {
        self.entity.element_data_mut().kind = kind;
        self
    }

    /// `set_position(pos)` followed by `set_position_map(from_world_xyz(pos))`.
    pub(crate) fn at(mut self, pos: crate::coordinates::WorldPoint3D) -> Self {
        let element = self.entity.element_data_mut();
        element.set_position(pos);
        element.set_position_map(crate::coordinates::MapPoint::from_world_xyz(
            pos.x, pos.y, pos.z,
        ));
        self
    }

    /// `set_position(pos)` only; the map position keeps its default.
    pub(crate) fn position(mut self, pos: crate::coordinates::WorldPoint3D) -> Self {
        self.entity.element_data_mut().set_position(pos);
        self
    }

    /// `set_sector_topology(sector, sector.arena_index())`.
    pub(crate) fn sector_topology(
        mut self,
        sector: Option<crate::position_interface::SectorHandle>,
    ) -> Self {
        self.entity
            .element_data_mut()
            .set_sector_topology(sector, sector.and_then(|sector| sector.arena_index()));
        self
    }

    pub(crate) fn script_class(mut self, script_class: &str) -> Self {
        self.actor_mut().script_class = script_class.into();
        self
    }

    /// Campaign-description slot of a PC; the campaign's character table
    /// must hold a matching entry.
    pub(crate) fn campaign_description(mut self, index: u32) -> Self {
        self.pc_mut().pc.campaign_description_index = Some(index);
        self
    }

    pub(crate) fn robin(mut self, robin: bool) -> Self {
        self.pc_mut().pc.robin = robin;
        self
    }

    /// `set_position_map(pos)` only; the world position keeps its default.
    pub(crate) fn map_position(mut self, pos: crate::coordinates::MapPoint) -> Self {
        self.entity.element_data_mut().set_position_map(pos);
        self
    }

    pub(crate) fn sector(mut self, sector: u16) -> Self {
        self.entity
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(sector));
        self
    }

    pub(crate) fn direction_instantly(mut self, direction: i16) -> Self {
        self.entity
            .element_data_mut()
            .set_direction_instantly(direction);
        self
    }

    pub(crate) fn action_state(mut self, action_state: crate::element::ActionState) -> Self {
        self.actor_mut().action_state = action_state;
        self
    }

    /// NPC or PC life points, depending on the actor variant.
    pub(crate) fn life_points(mut self, life_points: i16) -> Self {
        match &mut self.entity {
            Entity::Soldier(soldier) => soldier.npc.life_points = life_points,
            Entity::Pc(pc) => pc.pc.life_points = life_points,
            other => unreachable!("TestActor holds only soldiers and PCs, got {other:?}"),
        }
        self
    }

    pub(crate) fn camp(mut self, camp: crate::element::Camp) -> Self {
        self.soldier_mut().soldier.cached_camp = camp;
        self
    }

    pub(crate) fn soldier_profile(mut self, index: u32) -> Self {
        self.soldier_mut().soldier.soldier_profile_index =
            crate::profiles::SoldierProfileIdx(index);
        self
    }

    pub(crate) fn rider(mut self, rider: bool) -> Self {
        self.soldier_mut().soldier.rider = rider;
        self
    }

    pub(crate) fn enemy_ai(mut self, ai: crate::ai_enemy::EnemyAi) -> Self {
        self.soldier_mut().npc.ai.ai_brain = crate::element::AiBrain::Enemy(Box::new(ai));
        self
    }

    pub(crate) fn pc_profile(mut self, index: u32) -> Self {
        let Entity::Pc(pc) = &mut self.entity else {
            panic!("pc_profile on a non-PC TestActor");
        };
        pc.pc.profile_index = crate::profiles::CharacterProfileIdx(index);
        self
    }

    pub(crate) fn build(self) -> Entity {
        self.entity
    }

    fn actor_mut(&mut self) -> &mut crate::element::ActorData {
        match &mut self.entity {
            Entity::Soldier(soldier) => &mut soldier.actor,
            Entity::Pc(pc) => &mut pc.actor,
            other => unreachable!("TestActor holds only soldiers and PCs, got {other:?}"),
        }
    }

    fn pc_mut(&mut self) -> &mut crate::element::ActorPc {
        let Entity::Pc(pc) = &mut self.entity else {
            panic!("PC-only setter on a non-PC TestActor");
        };
        pc
    }

    fn soldier_mut(&mut self) -> &mut crate::element::ActorSoldier {
        let Entity::Soldier(soldier) = &mut self.entity else {
            panic!("soldier-only setter on a non-soldier TestActor");
        };
        soldier
    }
}

/// Run a scenario once with the first semantic actor created first and once
/// with it created second, returning `[observe(true), observe(false)]` in
/// that evaluation order.
pub(crate) fn for_both_creation_orders<T>(mut observe: impl FnMut(bool) -> T) -> [T; 2] {
    let first_created_first = observe(true);
    let first_created_second = observe(false);
    [first_created_first, first_created_second]
}

/// Return stable semantic roles while varying their publication order.
pub(crate) fn add_pair_in_creation_order(
    engine: &mut crate::engine::EngineInner,
    first: Entity,
    second: Entity,
    first_is_earlier: bool,
) -> (crate::element::EntityId, crate::element::EntityId) {
    if first_is_earlier {
        let first_id = engine.add_test_entity(first);
        let second_id = engine.add_test_entity(second);
        (first_id, second_id)
    } else {
        let second_id = engine.add_test_entity(second);
        let first_id = engine.add_test_entity(first);
        (first_id, second_id)
    }
}

/// Build a minimal soldier entity for posture / command tests.
pub(crate) fn make_test_soldier(posture: crate::element::Posture) -> Entity {
    // A directly constructed test soldier stands in for a loaded enemy
    // soldier. Production loading always supplies an allegiance; leaving the
    // sentinel `Camp::Error` here makes unrelated full-engine fixtures
    // invalid as soon as they exercise diplomacy-aware combat scans.
    let soldier_data = crate::element::SoldierData {
        cached_camp: crate::element::Camp::Lacklandists,
        ..Default::default()
    };
    let mut entity = Entity::Soldier(crate::element::ActorSoldier {
        soldier: soldier_data,
        ..unbound_soldier(posture)
    });
    entity
        .position_iface_mut()
        .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
    entity
}

/// Build a minimal civilian entity for NPC-translate tests.
pub(crate) fn make_test_civilian(posture: crate::element::Posture) -> Entity {
    // Loaded civilian profiles likewise always provide a real allegiance.
    let civilian_data = crate::element::CivilianData {
        cached_camp: crate::element::Camp::Royalists,
        ..Default::default()
    };
    let mut entity = Entity::Civilian(crate::element::ActorCivilian {
        element: {
            let mut initial_element = crate::element::ElementData::from_initial_posture(posture);
            initial_element.kind = crate::element::ElementKind::ActorCivilian;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        civilian: civilian_data,
    });
    entity
        .position_iface_mut()
        .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
    entity
}

pub(crate) fn make_test_pc(posture: crate::element::Posture) -> Entity {
    let mut entity = Entity::Pc(unbound_pc(posture));
    entity
        .position_iface_mut()
        .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
    entity
}

pub(crate) fn make_test_ai_soldier(camp: crate::element::Camp) -> Entity {
    let mut entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut entity else {
        unreachable!("make_test_soldier returned non-soldier");
    };
    soldier.soldier.cached_camp = camp;
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    entity
}

#[test]
fn soldier_builder_publishes_into_its_selected_camp() {
    use crate::element::{Camp, Posture};

    let mut engine = crate::engine::EngineInner::new();
    let enemy = engine.add_test_entity(TestActor::soldier(Posture::Upright).build());
    let friendly = engine.add_test_entity(
        TestActor::soldier(Posture::Upright)
            .camp(Camp::Royalists)
            .build(),
    );
    assert_eq!(
        engine.world.soldier_registry.camp(Camp::Lacklandists),
        &[enemy.index()]
    );
    assert_eq!(
        engine.world.soldier_registry.camp(Camp::Royalists),
        &[friendly.index()]
    );

    let invalid = TestActor::soldier(Posture::Upright)
        .camp(Camp::Error)
        .build();
    assert_eq!(invalid.camp(), Camp::Error);
    assert!(invalid.position_iface().get_pathfinder_index().is_none());
}

#[test]
fn unbound_fixtures_do_not_supply_runtime_bindings() {
    use crate::element::{Camp, ElementKind, Posture};

    let soldier = unbound_soldier(Posture::Flying);
    assert_eq!(soldier.element.posture(), Posture::Flying);
    assert_eq!(soldier.element.kind, ElementKind::ActorSoldier);
    assert!(soldier.element.active);
    assert_eq!(soldier.soldier.cached_camp, Camp::Error);
    assert_eq!(
        soldier.element.sprite.position_iface.get_pathfinder_index(),
        None
    );

    let pc = unbound_pc(Posture::Undefined);
    assert_eq!(pc.element.posture(), Posture::Undefined);
    assert_eq!(pc.element.kind, ElementKind::ActorPc);
    assert!(pc.element.active);
    assert_eq!(
        pc.element.sprite.position_iface.get_pathfinder_index(),
        None
    );

    let loaded = make_test_soldier(Posture::Flying);
    assert!(loaded.position_iface().get_pathfinder_index().is_some());
    let Entity::Soldier(loaded) = loaded else {
        unreachable!()
    };
    assert_eq!(loaded.soldier.cached_camp, Camp::Lacklandists);
}

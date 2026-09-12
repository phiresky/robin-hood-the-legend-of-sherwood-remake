//! Explicit state mutation seams available only to integration fixtures.
use super::*;

impl Engine {
    // ── Test-only helpers (round-trip save/load tests) ────────────
    //
    // Gated behind the `test-helpers` Cargo feature so production
    // builds of the facade do not expose direct sim-state setters.
    // `robin_rs` enables the feature in its `[dev-dependencies]`
    // block so its round-trip tests compile.

    /// Build populated achievement evidence via the real mission initializer
    /// and tracking operations, without exposing mutable tracker collections.
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_seed_achievement_persistence(&mut self, assets: &LevelAssets) {
        use crate::element::*;
        let mut pc_element = ElementData::default();
        pc_element.kind = ElementKind::ActorPc;
        pc_element.active = true;
        let pc = self.inner.add_test_entity(Entity::Pc(ActorPc {
            element: pc_element,
            actor: Default::default(),
            human: Default::default(),
            pc: PcData {
                life_points: 100,
                mission_role: crate::human_control::MissionRole::PlayerParty,
                kind: Some(crate::character_kind::CharacterKind::MerryManA),
                ..Default::default()
            },
        }));
        let mut soldier_element = ElementData::default();
        soldier_element.kind = ElementKind::ActorSoldier;
        soldier_element.active = true;
        let soldier = self.inner.add_test_entity(Entity::Soldier(ActorSoldier {
            element: soldier_element,
            actor: Default::default(),
            human: Default::default(),
            npc: NpcData {
                life_points: 100,
                ..Default::default()
            },
            soldier: SoldierData {
                cached_camp: Camp::Royalists,
                ..Default::default()
            },
        }));
        let nest = self
            .inner
            .add_test_entity(Entity::Projectile(ElementProjectile {
                element: Default::default(),
                object: Default::default(),
                projectile: Default::default(),
            }));
        self.inner.initialize_achievement_tracking(assets);
        let state = &mut self.inner.mission_domain.achievements;
        state.record_party_health(pc, 90);
        state.record_wasp_nest_throw(nest);
        state.queue_wasp_sting(soldier, nest);
        state.begin_quick_action_execution();
        state.record_quick_action_launch(pc, soldier);
        state.record_qa_success(pc, soldier);
        state.record_quick_action_launch(pc, soldier);
        state.end_quick_action_execution();
    }

    /// Insert a fully-formed entity into a test engine. Input-resolution
    /// tests need live entities to click on; the blank `new_for_test`
    /// level has none and the production spawn path requires proto data.
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_add_entity(&mut self, entity: crate::element::Entity) -> EntityId {
        self.inner.add_test_entity(entity)
    }

    /// Seed real sequence-manager insertion order for persistence regressions.
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_launch_sequence(
        &mut self,
        sequence: crate::sequence::Sequence,
    ) -> crate::sequence::SequenceId {
        self.inner.orders.sequence_manager.launch_sequence(sequence)
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_mission_flags(&mut self, quit_won: bool, quit_lost: bool, mission_won: bool) {
        self.inner
            .test_set_mission_flags(quit_won, quit_lost, mission_won);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_frame_counter(&mut self, frame: u32) {
        self.inner.test_set_frame_counter(frame);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_diplomacy(
        &mut self,
        enabled: bool,
        npc_faction_wars: bool,
        definition: crate::diplomacy::DiplomacyDefinition,
    ) {
        self.inner.mission_domain.diplomacy = crate::diplomacy::DiplomacyState::from_definition(
            enabled,
            npc_faction_wars,
            Some(&definition),
        )
        .expect("test diplomacy definition must be valid");
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_engine_scalars(
        &mut self,
        cheat_used_flags: u32,
        speed: f32,
        speed_int: u16,
        lock_engine: bool,
        freeze_all: bool,
        script_globals: Vec<i32>,
    ) {
        self.inner.test_set_engine_scalars(
            cheat_used_flags,
            speed,
            speed_int,
            lock_engine,
            freeze_all,
            script_globals,
        );
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_mission_stat(&mut self, stat: crate::mission_stat::MissionStat) {
        self.inner.test_set_mission_stat(stat);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_camera_transition_inputs(
        &mut self,
        zoom_init_done: bool,
        mechanized_zoom: bool,
        displacement: crate::coordinates::MapVec,
        displacement_counter: u16,
        pending_zoom_mouse_screen: Option<crate::coordinates::ScreenPoint>,
    ) {
        let camera = &mut self.inner.feedback.cutscene_camera;
        camera.zoom_init_done = zoom_init_done;
        camera.mechanized_zoom = mechanized_zoom;
        camera.displacement = displacement;
        camera.displacement_counter = displacement_counter;
        camera.pending_zoom_mouse_screen = pending_zoom_mouse_screen;
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_camera_transition_inputs(
        &self,
    ) -> (
        bool,
        bool,
        crate::coordinates::MapVec,
        u16,
        Option<crate::coordinates::ScreenPoint>,
    ) {
        let camera = &self.inner.feedback.cutscene_camera;
        (
            camera.zoom_init_done,
            camera.mechanized_zoom,
            camera.displacement,
            camera.displacement_counter,
            camera.pending_zoom_mouse_screen,
        )
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_assert_level_assets_attached(&self, assets: &LevelAssets) {
        assert!(std::sync::Arc::ptr_eq(
            &self.inner.world.fast_grid.level,
            &assets.navigation.level_grid
        ));
        self.inner.scripts.assert_native_attachments_ready();
    }
}

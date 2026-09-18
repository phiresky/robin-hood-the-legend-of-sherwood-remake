//! Explicit read surface used by world rendering, HUDs and their hit testing.
//!
//! This list follows actual presentation consumers; it is not an automatic
//! projection of EngineInner. New simulation queries are not inherited here.
use super::display_state::ViewConeParams;
use super::*;
use crate::achievement::AchievementProgressSnapshot;
use crate::coordinates::MapPoint;
use crate::fog_of_war::FogOfWarState;
use crate::profiles::Action;
use crate::tactical_control::{TacticalPinnedGroup, TacticalUnitOrder};

/// Forwards each listed read-only query to the identically named
/// `EngineInner` method.
macro_rules! delegate_to_inner {
    ($(fn $name:ident(&self $(, $arg:ident: $ty:ty)*) $(-> $ret:ty)?;)*) => {
        $(
            pub fn $name(&self $(, $arg: $ty)*) $(-> $ret)? {
                self.inner.$name($($arg),*)
            }
        )*
    };
}

/// Borrowed rendering capability, valid for either a fixed world or an
/// interpolated presentation copy. There is no dereference, conversion or
/// serialization route back to an engine or its full state.
///
/// ```compile_fail
/// use robin_engine::engine::{EngineInner, PresentationView};
/// fn forbidden<'a>(view: PresentationView<'a>) -> &'a EngineInner { &*view }
/// ```
/// ```compile_fail
/// use robin_engine::engine::PresentationView;
/// fn forbidden(view: PresentationView<'_>) { view.rng_seed(); }
/// ```
/// ```compile_fail
/// use robin_engine::engine::PresentationView;
/// fn forbidden(view: PresentationView<'_>) { view.mission_script(); }
/// ```
/// ```compile_fail
/// use robin_engine::engine::{EngineInner, PresentationView};
/// fn forbidden<'a>(view: &'a PresentationView<'_>) -> &'a EngineInner { view.as_ref() }
/// ```
/// ```compile_fail
/// use robin_engine::engine::{Engine, PresentationView};
/// fn forbidden(view: PresentationView<'_>) -> Engine { view.clone().into() }
/// ```
#[derive(Clone, Copy)]
pub struct PresentationView<'world> {
    inner: &'world EngineInner,
}

impl serde::Serialize for PresentationView<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut diagnostic = serializer.serialize_struct("PresentationView", 2)?;
        diagnostic.serialize_field("frame", &self.frame_counter())?;
        diagnostic.serialize_field("entity_count", &self.inner.entity_count())?;
        diagnostic.end()
    }
}

impl<'de> serde::Deserialize<'de> for PresentationView<'_> {
    fn deserialize<D: serde::Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "presentation views must borrow a live world",
        ))
    }
}

impl<'world> PresentationView<'world> {
    pub(super) fn new(inner: &'world EngineInner) -> Self {
        Self { inner }
    }

    pub fn has_mission_geometry(&self) -> bool {
        self.inner.mission_script().is_some()
    }
    pub fn mission_won(&self) -> bool {
        self.inner.mission().mission_won
    }
    pub fn screen_remarks(&self) -> &[crate::ai::ScreenRemark] {
        &self.inner.ai_global().screen_remarks
    }
    pub fn item_gameplay(&self) -> crate::gameplay_config::ItemGameplayConfig {
        self.inner.sim_config().item_gameplay
    }
    pub fn more_combat_gestures(&self) -> bool {
        self.inner.sim_config().more_combat_gestures
    }
    pub fn timed_missions_enabled(&self) -> bool {
        self.inner.sim_config().enable_timed_missions
    }
    pub fn uses_original_rng_replay(&self) -> bool {
        self.inner.original_rng_replay_cursor().is_some()
    }

    delegate_to_inner! {
        fn achievement_progress(&self) -> AchievementProgressSnapshot;
        fn pc_experience_snapshot(&self, entity: EntityId) -> Result<PcExperienceSnapshot, String>;
        fn is_zoom_possible(&self) -> bool;
        fn is_zoom_up_in_progress(&self) -> bool;
        fn is_zoom_down_in_progress(&self) -> bool;
        fn is_zoom_up_possible(&self) -> bool;
        fn is_zoom_down_possible(&self) -> bool;
        fn is_recording_macro(&self) -> bool;
        fn selected_view_cone_params(&self, selected_view_element: Option<EntityId>) -> Option<ViewConeParams>;
        fn all_npc_view_cone_params(&self) -> Vec<ViewConeParams>;
        fn display_ai_log_for_selected(&self, selected_view_element: Option<EntityId>);
        fn compute_display_order(&self) -> DrawOrder;
        fn minimap_dot_info(&self, id: crate::element::EntityId, assets: &LevelAssets) -> Option<crate::minimap::ElementDotInfo>;
        fn sort_for_minimap(&self) -> Vec<EntityId>;
        fn fog_of_war_enabled(&self) -> bool;
        fn fog_of_war(&self) -> &FogOfWarState;
        fn fog_entity_visible(&self, entity_id: EntityId) -> bool;
        fn fog_entity_is_hostile(&self, entity_id: EntityId) -> bool;
        fn selected_action_for_seat(&self, player_id: crate::player_command::PlayerId) -> crate::profiles::Action;
        fn planned_action_for_seat(&self, player_id: crate::player_command::PlayerId) -> crate::profiles::Action;
        fn automatic_quick_action_count(&self, actor: EntityId) -> usize;
        fn selected_pc_has_contextual_action(&self, assets: &LevelAssets, selected_pc_id: Option<EntityId>, action: crate::profiles::Action) -> bool;
        fn find_patch_for_door(&self, door_idx: u32) -> Option<u32>;
        fn get_nearest_jumpable_jump_line(&self, pc_entity: EntityId, candidate_sector_grid_idx: u32, pt_start: MapPoint, pt_goal: MapPoint, test_posture: bool, preferred_destination_sector: Option<u16>) -> Option<u32>;
        fn resolve_render_variant_for_ambiance(&self, entity: &crate::element::Entity, apply_fog_to_all_sprites: bool, ambiance: Ambiance) -> crate::sprite_variant::SpriteVariant;
        fn mission_countdown(&self) -> Option<MissionCountdownStatus>;
        fn initial_mission_ambiance(&self) -> Ambiance;
        fn initial_mission_night_color(&self) -> u16;
        fn is_player_aligned_camp(&self, camp: crate::element::Camp) -> bool;
        fn is_alt_effective(&self, input: &InputState) -> bool;
        fn is_lock_alt(&self) -> bool;
        fn entity_id_for_index(&self, index: u32) -> Option<EntityId>;
        fn can_have_unconscious_stars(&self, entity_id: EntityId) -> bool;
        fn entities_iter(&self) -> impl Iterator<Item = &Entity> + '_;
        fn active_entity_positions(&self) -> impl Iterator<Item = (EntityId, crate::coordinates::MapPoint)> + '_;
        fn pc_ids(&self) -> &[EntityId];
        fn npc_ids(&self) -> Vec<EntityId>;
        fn selected_hero_ids(&self) -> &[EntityId];
        fn hero_selection(&self, player_id: crate::player_command::PlayerId) -> &[EntityId];
        fn pc_draws_selection_mark(&self, pc_id: EntityId) -> bool;
        fn bg_animation_ids(&self) -> impl Iterator<Item = EntityId> + '_;
        fn titbit_manager(&self) -> &crate::titbit::TitbitManager;
        fn titbit_dotted_start(&self) -> f32;
        fn has_quick_action(&self, pc: EntityId, slot: u8) -> bool;
        fn get_golden_eye_mode(&self) -> bool;
        fn weather(&self) -> &WeatherState;
        fn fast_grid(&self) -> &FastFindGrid;
        fn actor_path_waypoints(&self, actor: EntityId) -> Option<Vec<crate::coordinates::MapPoint>>;
        fn ground_mark(&self) -> &GroundMark;
        fn short_briefings(&self) -> &ShortBriefings;
        fn is_qa_recording_for(&self, pc: EntityId) -> bool;
        fn frame_counter(&self) -> u32;
        fn is_men_to_blazon_conversion_mode(&self) -> bool;
        fn active_blinking_blazons(&self) -> u32;
        fn campaign(&self) -> &crate::campaign::Campaign;
        fn pc_character_kind(&self, pc_id: EntityId) -> Option<crate::character_kind::CharacterKind>;
        fn doors(&self) -> &[crate::gate::Door];
        fn patches(&self) -> &[crate::patch::Patch];
        fn displayed_pc_ids(&self) -> Vec<EntityId>;
        fn retrieve_stature(&self, pc_id: Option<EntityId>) -> Stature;
        fn collect_pcs_with_action(&self, assets: &LevelAssets, action: Action, out: &mut Vec<EntityId>);
        fn tactical_selection(&self, player_id: crate::player_command::PlayerId) -> &[EntityId];
        fn tactical_pinned_groups(&self, player_id: crate::player_command::PlayerId) -> &[TacticalPinnedGroup];
        fn tactical_first_visible_portrait(&self, player_id: crate::player_command::PlayerId) -> usize;
        fn tactical_order(&self, soldier: EntityId) -> Option<&TacticalUnitOrder>;
    }

    pub fn installed_order_type(
        &self,
        order: crate::element::InstalledActorOrder,
    ) -> crate::order::OrderType {
        order
            .resolve(&self.inner.orders.sequence_manager)
            .order_type
    }

    pub fn get_entity<I: Into<EntityId>>(&self, id: I) -> Option<&'world Entity> {
        self.inner.get_entity(id)
    }

    pub fn active_peer_selections(
        &self,
    ) -> impl Iterator<Item = (crate::player_command::PlayerId, &[EntityId], &str)> {
        self.inner
            .active_seats()
            .map(|(id, seat)| (id, seat.selection.as_slice(), seat.nickname.as_str()))
    }

    pub fn portrait_macro(&self, pc: EntityId) -> Option<&crate::macro_store::PcMacroState> {
        self.inner.macro_store().get(pc)
    }

    pub fn draw_navigation_graph(
        &self,
        graph: &crate::pathfinder::PathGraph,
        view: crate::coordinates::MapBBox,
        half_diagonal_idx: u16,
        draw: impl FnMut(MapPoint, MapPoint, u16),
    ) {
        self.inner
            .pathfinder()
            .draw_graph(graph, view, half_diagonal_idx, draw);
    }

    pub fn draw_navigation_nodes(
        &self,
        graph: &crate::pathfinder::PathGraph,
        view: crate::coordinates::MapBBox,
        half_diagonal_idx: u16,
        draw: impl FnMut(MapPoint, MapPoint, u16),
    ) {
        self.inner
            .pathfinder()
            .draw_nodes(graph, view, half_diagonal_idx, draw);
    }

    pub fn sight_obstacles<'a>(
        &'a self,
        assets: &'a LevelAssets,
    ) -> crate::sight_obstacle::ObstacleList<'a> {
        self.inner.sight_obstacles(assets)
    }
}

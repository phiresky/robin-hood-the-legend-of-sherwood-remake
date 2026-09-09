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

    pub fn achievement_progress(&self) -> AchievementProgressSnapshot {
        self.inner.achievement_progress()
    }

    pub fn pc_experience_snapshot(&self, entity: EntityId) -> Result<PcExperienceSnapshot, String> {
        self.inner.pc_experience_snapshot(entity)
    }

    pub fn is_zoom_possible(&self, _display: &HostDisplayState) -> bool {
        self.inner.is_zoom_possible(_display)
    }

    pub fn is_zoom_up_in_progress(&self, _display: &HostDisplayState) -> bool {
        self.inner.is_zoom_up_in_progress(_display)
    }

    pub fn is_zoom_down_in_progress(&self, _display: &HostDisplayState) -> bool {
        self.inner.is_zoom_down_in_progress(_display)
    }

    pub fn is_zoom_up_possible(&self) -> bool {
        self.inner.is_zoom_up_possible()
    }

    pub fn is_zoom_down_possible(&self) -> bool {
        self.inner.is_zoom_down_possible()
    }

    pub fn is_recording_macro(&self) -> bool {
        self.inner.is_recording_macro()
    }

    pub fn selected_view_cone_params(
        &self,
        selected_view_element: Option<EntityId>,
    ) -> Option<ViewConeParams> {
        self.inner.selected_view_cone_params(selected_view_element)
    }

    pub fn all_npc_view_cone_params(&self) -> Vec<ViewConeParams> {
        self.inner.all_npc_view_cone_params()
    }

    pub fn display_ai_log_for_selected(&self, selected_view_element: Option<EntityId>) {
        self.inner
            .display_ai_log_for_selected(selected_view_element)
    }

    pub fn compute_display_order(&self) -> DrawOrder {
        self.inner.compute_display_order()
    }

    pub fn minimap_dot_info(
        &self,
        id: crate::element::EntityId,
        assets: &LevelAssets,
    ) -> Option<crate::minimap::ElementDotInfo> {
        self.inner.minimap_dot_info(id, assets)
    }

    pub fn sort_for_minimap(&self) -> Vec<EntityId> {
        self.inner.sort_for_minimap()
    }

    pub fn fog_of_war_enabled(&self) -> bool {
        self.inner.fog_of_war_enabled()
    }

    pub fn fog_of_war(&self) -> &FogOfWarState {
        self.inner.fog_of_war()
    }

    pub fn fog_entity_visible(&self, entity_id: EntityId) -> bool {
        self.inner.fog_entity_visible(entity_id)
    }

    pub fn fog_entity_is_hostile(&self, entity_id: EntityId) -> bool {
        self.inner.fog_entity_is_hostile(entity_id)
    }

    pub fn selected_action_for_seat(
        &self,
        player_id: crate::player_command::PlayerId,
    ) -> crate::profiles::Action {
        self.inner.selected_action_for_seat(player_id)
    }

    pub fn planned_action_for_seat(
        &self,
        player_id: crate::player_command::PlayerId,
    ) -> crate::profiles::Action {
        self.inner.planned_action_for_seat(player_id)
    }

    pub fn automatic_quick_action_count(&self, actor: EntityId) -> usize {
        self.inner.automatic_quick_action_count(actor)
    }

    pub fn selected_pc_has_contextual_action(
        &self,
        assets: &LevelAssets,
        selected_pc_id: Option<EntityId>,
        action: crate::profiles::Action,
    ) -> bool {
        self.inner
            .selected_pc_has_contextual_action(assets, selected_pc_id, action)
    }

    pub fn find_patch_for_door(&self, door_idx: u32) -> Option<u32> {
        self.inner.find_patch_for_door(door_idx)
    }

    pub fn get_nearest_jumpable_jump_line(
        &self,
        pc_entity: EntityId,
        candidate_sector_grid_idx: u32,
        pt_start: MapPoint,
        pt_goal: MapPoint,
        test_posture: bool,
        preferred_destination_sector: Option<u16>,
    ) -> Option<u32> {
        self.inner.get_nearest_jumpable_jump_line(
            pc_entity,
            candidate_sector_grid_idx,
            pt_start,
            pt_goal,
            test_posture,
            preferred_destination_sector,
        )
    }

    pub fn resolve_render_variant_for_ambiance(
        &self,
        entity: &crate::element::Entity,
        apply_fog_to_all_sprites: bool,
        ambiance: Ambiance,
    ) -> crate::sprite_variant::SpriteVariant {
        self.inner
            .resolve_render_variant_for_ambiance(entity, apply_fog_to_all_sprites, ambiance)
    }

    pub fn mission_countdown(&self) -> Option<MissionCountdownStatus> {
        self.inner.mission_countdown()
    }

    pub fn initial_mission_ambiance(&self) -> Ambiance {
        self.inner.initial_mission_ambiance()
    }

    pub fn initial_mission_night_color(&self) -> u16 {
        self.inner.initial_mission_night_color()
    }

    pub fn is_player_aligned_camp(&self, camp: crate::element::Camp) -> bool {
        self.inner.is_player_aligned_camp(camp)
    }

    pub fn is_alt_effective(&self, input: &InputState) -> bool {
        self.inner.is_alt_effective(input)
    }

    pub fn is_lock_alt(&self) -> bool {
        self.inner.is_lock_alt()
    }

    pub fn get_entity<I: Into<EntityId>>(&self, id: I) -> Option<&'world Entity> {
        self.inner.get_entity(id)
    }

    pub fn entity_id_for_index(&self, index: u32) -> Option<EntityId> {
        self.inner.entity_id_for_index(index)
    }

    pub fn can_have_unconscious_stars(&self, entity_id: EntityId) -> bool {
        self.inner.can_have_unconscious_stars(entity_id)
    }

    pub fn entities_iter(&self) -> impl Iterator<Item = &Entity> + '_ {
        self.inner.entities_iter()
    }

    pub fn active_entity_positions(
        &self,
    ) -> impl Iterator<Item = (EntityId, crate::coordinates::MapPoint)> + '_ {
        self.inner.active_entity_positions()
    }

    pub fn pc_ids(&self) -> &[EntityId] {
        self.inner.pc_ids()
    }

    pub fn npc_ids(&self) -> Vec<EntityId> {
        self.inner.npc_ids()
    }

    pub fn selected_hero_ids(&self) -> &[EntityId] {
        self.inner.selected_hero_ids()
    }

    pub fn hero_selection(&self, player_id: crate::player_command::PlayerId) -> &[EntityId] {
        self.inner.hero_selection(player_id)
    }

    pub fn active_peer_selections(
        &self,
    ) -> impl Iterator<Item = (crate::player_command::PlayerId, &[EntityId], &str)> {
        self.inner
            .active_seats()
            .map(|(id, seat)| (id, seat.selection.as_slice(), seat.nickname.as_str()))
    }

    pub fn pc_draws_selection_mark(&self, pc_id: EntityId) -> bool {
        self.inner.pc_draws_selection_mark(pc_id)
    }

    pub fn bg_animation_ids(&self) -> Vec<EntityId> {
        self.inner.bg_animation_ids()
    }

    pub fn titbit_manager(&self) -> &crate::titbit::TitbitManager {
        self.inner.titbit_manager()
    }

    pub fn titbit_dotted_start(&self) -> f32 {
        self.inner.titbit_dotted_start()
    }

    pub fn portrait_macro(&self, pc: EntityId) -> Option<&crate::macro_store::PcMacroState> {
        self.inner.macro_store().get(pc)
    }

    pub fn has_quick_action(&self, pc: EntityId, slot: u8) -> bool {
        self.inner.has_quick_action(pc, slot)
    }

    pub fn get_golden_eye_mode(&self) -> bool {
        self.inner.get_golden_eye_mode()
    }

    pub fn weather(&self) -> &WeatherState {
        self.inner.weather()
    }

    pub fn fast_grid(&self) -> &FastFindGrid {
        self.inner.fast_grid()
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

    pub fn actor_path_waypoints(
        &self,
        actor: EntityId,
    ) -> Option<Vec<crate::coordinates::MapPoint>> {
        self.inner.actor_path_waypoints(actor)
    }

    pub fn ground_mark(&self) -> &GroundMark {
        self.inner.ground_mark()
    }

    pub fn sight_obstacles<'a>(
        &'a self,
        assets: &'a LevelAssets,
    ) -> crate::sight_obstacle::ObstacleList<'a> {
        self.inner.sight_obstacles(assets)
    }

    pub fn short_briefings(&self) -> &ShortBriefings {
        self.inner.short_briefings()
    }

    pub fn is_qa_recording_for(&self, pc: EntityId) -> bool {
        self.inner.is_qa_recording_for(pc)
    }

    pub fn frame_counter(&self) -> u32 {
        self.inner.frame_counter()
    }

    pub fn is_men_to_blazon_conversion_mode(&self) -> bool {
        self.inner.is_men_to_blazon_conversion_mode()
    }

    pub fn active_blinking_blazons(&self) -> u32 {
        self.inner.active_blinking_blazons()
    }

    pub fn campaign(&self) -> &crate::campaign::Campaign {
        self.inner.campaign()
    }

    pub fn pc_character_kind(
        &self,
        pc_id: EntityId,
    ) -> Option<crate::character_kind::CharacterKind> {
        self.inner.pc_character_kind(pc_id)
    }

    pub fn doors(&self) -> &[crate::gate::Door] {
        self.inner.doors()
    }

    pub fn patches(&self) -> &[crate::patch::Patch] {
        self.inner.patches()
    }

    pub fn displayed_pc_ids(&self) -> Vec<EntityId> {
        self.inner.displayed_pc_ids()
    }

    pub fn retrieve_stature(&self, pc_id: Option<EntityId>) -> Stature {
        self.inner.retrieve_stature(pc_id)
    }

    pub fn collect_pcs_with_action(
        &self,
        assets: &LevelAssets,
        action: Action,
        out: &mut Vec<EntityId>,
    ) {
        self.inner.collect_pcs_with_action(assets, action, out)
    }

    pub fn tactical_selection(&self, player_id: crate::player_command::PlayerId) -> &[EntityId] {
        self.inner.tactical_selection(player_id)
    }

    pub fn tactical_pinned_groups(
        &self,
        player_id: crate::player_command::PlayerId,
    ) -> &[TacticalPinnedGroup] {
        self.inner.tactical_pinned_groups(player_id)
    }

    pub fn tactical_first_visible_portrait(
        &self,
        player_id: crate::player_command::PlayerId,
    ) -> usize {
        self.inner.tactical_first_visible_portrait(player_id)
    }

    pub fn tactical_order(&self, soldier: EntityId) -> Option<&TacticalUnitOrder> {
        self.inner.tactical_order(soldier)
    }
}

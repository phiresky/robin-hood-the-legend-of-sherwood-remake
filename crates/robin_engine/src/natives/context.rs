use std::{
    cell::{RefCell, RefMut},
    ops::{Deref, DerefMut},
};

use super::{
    AttachedScriptBindings, ScriptBindings, ScriptEffects, ScriptHandleCodec, ScriptState,
};
use crate::element::EntityId;

/// Canonical engine owners and read views borrowed for one complete script
/// session. Each [`NativeContext`] takes short-lived mutable borrows for one VM
/// resume; nested dispatch can then borrow the same owners before resuming the
/// outer VM. None of these values move into or through [`ScriptEffects`].
pub struct NativeSessionCapabilities<'a> {
    simulation: &'a crate::sim_rng::SimulationContext,
    entities: RefCell<&'a mut crate::entities::Entities>,
    ai_global: RefCell<&'a mut crate::ai::AiGlobalState>,
    fast_grid: RefCell<&'a mut crate::fast_find_grid::FastFindGrid>,
    campaign: Option<RefCell<&'a mut crate::campaign::Campaign>>,
    mission_stat: Option<RefCell<&'a mut crate::mission_stat::MissionStat>>,
    sequence_manager: Option<RefCell<&'a mut crate::sequence::SequenceManager>>,
    selected_pcs: Option<RefCell<&'a mut Vec<EntityId>>>,
    short_briefings: Option<RefCell<&'a mut crate::short_briefings::ShortBriefings>>,
    standard_view_radius: Option<RefCell<&'a mut u16>>,
    view_radius_cache: Option<RefCell<&'a mut crate::ai_vision::ViewRadiusCache>>,
    sight_obstacles: Option<crate::sight_obstacle::ObstacleList<'a>>,
    sound_sources: Option<RefCell<&'a mut crate::sound_source::SoundSourceManager>>,
    weather: Option<&'a crate::engine::WeatherState>,
    frame_counter: Option<&'a u32>,
}

impl<'a> NativeSessionCapabilities<'a> {
    pub fn new(
        simulation: &'a crate::sim_rng::SimulationContext,
        entities: &'a mut crate::entities::Entities,
        ai_global: &'a mut crate::ai::AiGlobalState,
        fast_grid: &'a mut crate::fast_find_grid::FastFindGrid,
    ) -> Self {
        Self {
            simulation,
            entities: RefCell::new(entities),
            ai_global: RefCell::new(ai_global),
            fast_grid: RefCell::new(fast_grid),
            campaign: None,
            mission_stat: None,
            sequence_manager: None,
            selected_pcs: None,
            short_briefings: None,
            standard_view_radius: None,
            view_radius_cache: None,
            sight_obstacles: None,
            sound_sources: None,
            weather: None,
            frame_counter: None,
        }
    }

    pub fn with_queries(
        mut self,
        sequence_manager: &'a mut crate::sequence::SequenceManager,
        selected_pcs: &'a mut Vec<EntityId>,
        sound_sources: &'a mut crate::sound_source::SoundSourceManager,
        weather: &'a crate::engine::WeatherState,
        frame_counter: &'a u32,
    ) -> Self {
        self.sequence_manager = Some(RefCell::new(sequence_manager));
        self.selected_pcs = Some(RefCell::new(selected_pcs));
        self.sound_sources = Some(RefCell::new(sound_sources));
        self.weather = Some(weather);
        self.frame_counter = Some(frame_counter);
        self
    }

    /// Attach canonical world arrays needed by synchronous native queries.
    /// Mutable sight arrays remain owned by `WorldState`; the dispatcher only
    /// borrows them for the duration of a script session.
    pub fn with_world_views(
        mut self,
        static_sight_obstacles: &'a [crate::sight_obstacle::SightObstacle],
        dynamic_sight_obstacles: &'a [crate::sight_obstacle::SightObstacle],
        static_sight_obstacle_active: &'a [bool],
    ) -> Self {
        self.sight_obstacles = Some(crate::sight_obstacle::ObstacleList {
            static_obstacles: static_sight_obstacles,
            dynamic_obstacles: dynamic_sight_obstacles,
            static_active: static_sight_obstacle_active,
        });
        self
    }

    pub fn with_campaign(
        mut self,
        campaign: &'a mut crate::campaign::Campaign,
        mission_stat: &'a mut crate::mission_stat::MissionStat,
    ) -> Self {
        self.campaign = Some(RefCell::new(campaign));
        self.mission_stat = Some(RefCell::new(mission_stat));
        self
    }

    /// Attach the canonical objective/short-briefing model. Both vanilla
    /// briefing natives and Spellforge objective extensions write this owner
    /// before returning to the VM.
    pub fn with_short_briefings(
        mut self,
        short_briefings: &'a mut crate::short_briefings::ShortBriefings,
    ) -> Self {
        self.short_briefings = Some(RefCell::new(short_briefings));
        self
    }

    /// Attach the AI-domain radius paired with the live entity store. The
    /// `SetViewRadius` native updates both before returning, matching
    /// `RHEngine::SetStandardViewRadius` followed by every NPC's
    /// `InitViewRadius` in the Original.
    pub fn with_standard_view_radius(mut self, radius: &'a mut u16) -> Self {
        self.standard_view_radius = Some(RefCell::new(radius));
        self
    }

    pub(crate) fn with_view_radius_cache(
        mut self,
        cache: &'a mut crate::ai_vision::ViewRadiusCache,
    ) -> Self {
        self.view_radius_cache = Some(RefCell::new(cache));
        self
    }

    fn entities(&self) -> RefMut<'_, crate::entities::Entities> {
        RefMut::map(self.entities.borrow_mut(), |entities| &mut **entities)
    }

    fn ai_global(&self) -> RefMut<'_, crate::ai::AiGlobalState> {
        RefMut::map(self.ai_global.borrow_mut(), |ai_global| &mut **ai_global)
    }

    fn fast_grid(&self) -> RefMut<'_, crate::fast_find_grid::FastFindGrid> {
        RefMut::map(self.fast_grid.borrow_mut(), |fast_grid| &mut **fast_grid)
    }

    fn campaign(&self) -> Option<RefMut<'_, crate::campaign::Campaign>> {
        self.campaign
            .as_ref()
            .map(|campaign| RefMut::map(campaign.borrow_mut(), |campaign| &mut **campaign))
    }

    fn mission_stat(&self) -> Option<RefMut<'_, crate::mission_stat::MissionStat>> {
        self.mission_stat.as_ref().map(|mission_stat| {
            RefMut::map(mission_stat.borrow_mut(), |mission_stat| {
                &mut **mission_stat
            })
        })
    }

    #[cfg(test)]
    pub(crate) fn entities_owner_ptr(&self) -> *const crate::entities::Entities {
        let entities = self.entities.borrow();
        std::ptr::from_ref(&**entities)
    }
}

/// Transient receiver context for one script callback.
///
/// This mirrors the Original's separately bracketed `pScriptThis` and
/// `RHElementScroll::pScrollExecutingScript` values. Frames are copied into a
/// [`NativeContext`] for one VM resume, but are owned and stacked by
/// `MissionScript`; they are never part of a mission snapshot or state hash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScriptCallFrame {
    script_this: i32,
    current_scroll: i32,
}

impl ScriptCallFrame {
    pub fn actor(script_this: i32) -> Self {
        Self {
            script_this,
            current_scroll: 0,
        }
    }

    pub fn scroll(current_scroll: i32) -> Self {
        Self {
            script_this: 0,
            current_scroll,
        }
    }

    pub fn with_script_this(mut self, script_this: i32) -> Self {
        self.script_this = script_this;
        self
    }

    pub fn with_current_scroll(mut self, current_scroll: i32) -> Self {
        self.current_scroll = current_scroll;
        self
    }

    pub fn script_this(self) -> i32 {
        self.script_this
    }

    pub fn current_scroll(self) -> i32 {
        self.current_scroll
    }
}

impl<'a> NativeSessionCapabilities<'a> {
    /// The engine-owned deterministic simulation stream for this script
    /// session. Script runtimes must fail when no session is attached rather
    /// than substituting an ambient or host RNG.
    #[doc(hidden)]
    pub fn simulation_context(&self) -> &crate::sim_rng::SimulationContext {
        self.simulation
    }

    #[doc(hidden)]
    pub fn sequence_manager_option(&self) -> Option<RefMut<'_, crate::sequence::SequenceManager>> {
        self.sequence_manager.as_ref().map(|sequence_manager| {
            RefMut::map(sequence_manager.borrow_mut(), |sequence_manager| {
                &mut **sequence_manager
            })
        })
    }

    #[doc(hidden)]
    pub fn selected_pcs_option(&self) -> Option<RefMut<'_, Vec<EntityId>>> {
        self.selected_pcs.as_ref().map(|selected_pcs| {
            RefMut::map(selected_pcs.borrow_mut(), |selected_pcs| {
                &mut **selected_pcs
            })
        })
    }

    #[doc(hidden)]
    pub fn short_briefings_option(
        &self,
    ) -> Option<RefMut<'_, crate::short_briefings::ShortBriefings>> {
        self.short_briefings.as_ref().map(|short_briefings| {
            RefMut::map(short_briefings.borrow_mut(), |short_briefings| {
                &mut **short_briefings
            })
        })
    }

    #[doc(hidden)]
    pub fn standard_view_radius_option(&self) -> Option<RefMut<'_, u16>> {
        self.standard_view_radius
            .as_ref()
            .map(|radius| RefMut::map(radius.borrow_mut(), |radius| &mut **radius))
    }

    #[doc(hidden)]
    pub fn view_radius_cache_option(
        &self,
    ) -> Option<RefMut<'_, crate::ai_vision::ViewRadiusCache>> {
        self.view_radius_cache
            .as_ref()
            .map(|cache| RefMut::map(cache.borrow_mut(), |cache| &mut **cache))
    }

    #[doc(hidden)]
    pub fn sight_obstacles_option(&self) -> Option<crate::sight_obstacle::ObstacleList<'a>> {
        self.sight_obstacles
    }

    #[doc(hidden)]
    pub fn sound_sources_option(
        &self,
    ) -> Option<RefMut<'_, crate::sound_source::SoundSourceManager>> {
        self.sound_sources.as_ref().map(|sound_sources| {
            RefMut::map(sound_sources.borrow_mut(), |sound_sources| {
                &mut **sound_sources
            })
        })
    }

    #[doc(hidden)]
    pub fn weather_option(&self) -> Option<&'a crate::engine::WeatherState> {
        self.weather
    }

    #[doc(hidden)]
    pub fn frame_counter_option(&self) -> Option<&'a u32> {
        self.frame_counter
    }
}

/// Short-lived native dispatcher assembled for one VM resume.
///
/// Script globals, computed locations, and recorder state are borrowed from
/// their sole owner on `MissionScript`; only typed effects are buffered here.
pub struct NativeContext<'ctx, 'owners: 'ctx> {
    pub(crate) simulation: &'ctx crate::sim_rng::SimulationContext,
    pub(crate) script_effects: &'ctx mut ScriptEffects,
    pub(crate) entities: RefMut<'ctx, crate::entities::Entities>,
    pub(crate) ai_global: RefMut<'ctx, crate::ai::AiGlobalState>,
    pub(crate) fast_grid: RefMut<'ctx, crate::fast_find_grid::FastFindGrid>,
    pub(crate) script_state: &'ctx mut ScriptState,
    pub(crate) script_domains: &'ctx mut crate::engine::ScriptDomains,
    pub(crate) bindings: ScriptBindings<'ctx>,
    pub(crate) campaign: Option<RefMut<'ctx, crate::campaign::Campaign>>,
    pub(crate) mission_stat: Option<RefMut<'ctx, crate::mission_stat::MissionStat>>,
    pub(crate) sequence_manager: Option<RefMut<'ctx, crate::sequence::SequenceManager>>,
    pub(crate) selected_pcs: Option<RefMut<'ctx, Vec<EntityId>>>,
    pub(crate) short_briefings: Option<RefMut<'ctx, crate::short_briefings::ShortBriefings>>,
    pub(crate) standard_view_radius: Option<RefMut<'ctx, u16>>,
    pub(crate) view_radius_cache: Option<RefMut<'ctx, crate::ai_vision::ViewRadiusCache>>,
    pub(crate) sight_obstacles: Option<crate::sight_obstacle::ObstacleList<'owners>>,
    pub(crate) sound_sources: Option<RefMut<'ctx, crate::sound_source::SoundSourceManager>>,
    pub(crate) weather: Option<&'owners crate::engine::WeatherState>,
    pub(crate) frame_counter: Option<&'owners u32>,
    pub(crate) call_frame: ScriptCallFrame,
    pub(crate) script_vm_diagnostic: Option<crate::sim_rng::ScriptVmDiagnosticContext>,
    pub(crate) pending_yield: Option<crate::interp::NativeYield>,
}

impl<'ctx, 'owners: 'ctx> NativeContext<'ctx, 'owners> {
    pub fn new(
        script_effects: &'ctx mut ScriptEffects,
        script_state: &'ctx mut ScriptState,
        script_domains: &'ctx mut crate::engine::ScriptDomains,
        capabilities: &'ctx NativeSessionCapabilities<'owners>,
    ) -> Self {
        Self {
            simulation: capabilities.simulation_context(),
            script_effects,
            entities: capabilities.entities(),
            ai_global: capabilities.ai_global(),
            fast_grid: capabilities.fast_grid(),
            script_state,
            script_domains,
            bindings: ScriptBindings::empty(),
            campaign: capabilities.campaign(),
            mission_stat: capabilities.mission_stat(),
            sequence_manager: capabilities.sequence_manager_option(),
            selected_pcs: capabilities.selected_pcs_option(),
            short_briefings: capabilities.short_briefings_option(),
            standard_view_radius: capabilities.standard_view_radius_option(),
            view_radius_cache: capabilities.view_radius_cache_option(),
            sight_obstacles: capabilities.sight_obstacles_option(),
            sound_sources: capabilities.sound_sources_option(),
            weather: capabilities.weather_option(),
            frame_counter: capabilities.frame_counter_option(),
            call_frame: ScriptCallFrame::default(),
            script_vm_diagnostic: None,
            pending_yield: None,
        }
    }

    pub fn with_bindings(
        script_effects: &'ctx mut ScriptEffects,
        script_state: &'ctx mut ScriptState,
        script_domains: &'ctx mut crate::engine::ScriptDomains,
        bindings: &'ctx AttachedScriptBindings,
        capabilities: &'ctx NativeSessionCapabilities<'owners>,
    ) -> Self {
        Self {
            simulation: capabilities.simulation_context(),
            script_effects,
            entities: capabilities.entities(),
            ai_global: capabilities.ai_global(),
            fast_grid: capabilities.fast_grid(),
            script_state,
            script_domains,
            bindings: bindings.view(),
            campaign: capabilities.campaign(),
            mission_stat: capabilities.mission_stat(),
            sequence_manager: capabilities.sequence_manager_option(),
            selected_pcs: capabilities.selected_pcs_option(),
            short_briefings: capabilities.short_briefings_option(),
            standard_view_radius: capabilities.standard_view_radius_option(),
            view_radius_cache: capabilities.view_radius_cache_option(),
            sight_obstacles: capabilities.sight_obstacles_option(),
            sound_sources: capabilities.sound_sources_option(),
            weather: capabilities.weather_option(),
            frame_counter: capabilities.frame_counter_option(),
            call_frame: ScriptCallFrame::default(),
            script_vm_diagnostic: None,
            pending_yield: None,
        }
    }

    pub fn with_call_frame(
        script_effects: &'ctx mut ScriptEffects,
        script_state: &'ctx mut ScriptState,
        script_domains: &'ctx mut crate::engine::ScriptDomains,
        bindings: &'ctx AttachedScriptBindings,
        capabilities: &'ctx NativeSessionCapabilities<'owners>,
        call_frame: ScriptCallFrame,
    ) -> Self {
        Self {
            simulation: capabilities.simulation_context(),
            script_effects,
            entities: capabilities.entities(),
            ai_global: capabilities.ai_global(),
            fast_grid: capabilities.fast_grid(),
            script_state,
            script_domains,
            bindings: bindings.view(),
            campaign: capabilities.campaign(),
            mission_stat: capabilities.mission_stat(),
            sequence_manager: capabilities.sequence_manager_option(),
            selected_pcs: capabilities.selected_pcs_option(),
            short_briefings: capabilities.short_briefings_option(),
            standard_view_radius: capabilities.standard_view_radius_option(),
            view_radius_cache: capabilities.view_radius_cache_option(),
            sight_obstacles: capabilities.sight_obstacles_option(),
            sound_sources: capabilities.sound_sources_option(),
            weather: capabilities.weather_option(),
            frame_counter: capabilities.frame_counter_option(),
            call_frame,
            script_vm_diagnostic: None,
            pending_yield: None,
        }
    }

    pub fn bindings(&self) -> ScriptBindings<'_> {
        self.bindings
    }

    pub fn script_state(&self) -> &ScriptState {
        self.script_state
    }

    pub fn script_effects(&self) -> &ScriptEffects {
        self.script_effects
    }

    pub fn script_state_mut(&mut self) -> &mut ScriptState {
        self.script_state
    }

    pub(crate) fn ai_global(&self) -> &crate::ai::AiGlobalState {
        &self.ai_global
    }

    pub(crate) fn ai_global_mut(&mut self) -> &mut crate::ai::AiGlobalState {
        &mut self.ai_global
    }

    // Associated functions do not participate in deref method lookup. These
    // forwards let native dispatch use `Self` while the stateless codec remains
    // the single owner of the script-handle representation.
    pub(crate) fn actor_handle<I: Into<EntityId>>(id: I) -> i32 {
        ScriptHandleCodec::actor_handle(id)
    }

    pub(crate) fn actor_handle_from_index(index: usize) -> i32 {
        ScriptHandleCodec::actor_handle_from_index(index)
    }

    pub(crate) fn sound_source_handle_from_index(index: usize) -> i32 {
        ScriptHandleCodec::sound_source_handle_from_index(index)
    }

    pub(crate) fn actor_handle_index(handle: i32) -> Option<usize> {
        ScriptHandleCodec::actor_handle_index(handle)
    }

    pub(crate) fn door_index(handle: i32) -> Option<usize> {
        ScriptHandleCodec::door_index(handle)
    }

    pub(crate) fn patch_index(handle: i32) -> Option<usize> {
        ScriptHandleCodec::patch_index(handle)
    }

    pub(crate) fn location_index(handle: i32) -> Option<usize> {
        ScriptHandleCodec::location_index(handle)
    }

    pub(crate) fn sound_source_index(handle: i32) -> Option<usize> {
        ScriptHandleCodec::sound_source_index(handle)
    }

    pub(crate) fn building_index(handle: i32) -> Option<usize> {
        ScriptHandleCodec::building_index(handle)
    }

    pub(crate) fn way_index(handle: i32) -> Option<usize> {
        ScriptHandleCodec::way_index(handle)
    }
}

impl Deref for NativeContext<'_, '_> {
    type Target = ScriptEffects;

    fn deref(&self) -> &Self::Target {
        self.script_effects
    }
}

impl DerefMut for NativeContext<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.script_effects
    }
}

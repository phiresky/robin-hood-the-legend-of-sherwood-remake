use super::{AttachedScriptBindings, ScriptBindings, ScriptHandleCodec, ScriptState};
use crate::element::EntityId;

/// Canonical engine owners and read views borrowed for one complete script
/// session. Each [`NativeContext`] takes short-lived mutable borrows for one VM
/// resume; nested dispatch can then borrow the same owners before resuming the
/// outer VM. The compiler prevents overlapping contexts; each resume drops its context before callbacks.
pub struct NativeSessionCapabilities<'a> {
    simulation: &'a crate::sim_rng::SimulationContext,
    entities: &'a mut crate::entities::Entities,
    pc_registry: Option<&'a [EntityId]>,
    ai_global: &'a mut crate::ai::AiGlobalState,
    fast_grid: &'a mut crate::fast_find_grid::FastFindGrid,
    script_globals: &'a mut Vec<i32>,
    campaign: Option<&'a mut crate::campaign::Campaign>,
    mission_stat: Option<&'a mut crate::mission_stat::MissionStat>,
    diplomacy: Option<&'a mut crate::diplomacy::DiplomacyState>,
    sequence_manager: Option<&'a mut crate::sequence::SequenceManager>,
    selected_pcs: Option<&'a mut Vec<EntityId>>,
    selected_action: Option<&'a mut crate::profiles::Action>,
    short_briefings: Option<&'a mut crate::short_briefings::ShortBriefings>,
    standard_view_radius: Option<&'a mut u16>,
    view_radius_cache: Option<&'a mut crate::ai_vision::ViewRadiusCache>,
    sight_obstacles: Option<crate::sight_obstacle::ObstacleList<'a>>,
    sound_sources: Option<&'a mut crate::sound_source::SoundSourceManager>,
    weather: Option<&'a crate::engine::WeatherState>,
    frame_counter: Option<&'a u32>,
}

impl<'a> NativeSessionCapabilities<'a> {
    pub fn new(
        simulation: &'a crate::sim_rng::SimulationContext,
        entities: &'a mut crate::entities::Entities,
        ai_global: &'a mut crate::ai::AiGlobalState,
        fast_grid: &'a mut crate::fast_find_grid::FastFindGrid,
        script_globals: &'a mut Vec<i32>,
    ) -> Self {
        Self {
            simulation,
            entities: entities,
            pc_registry: None,
            ai_global: ai_global,
            fast_grid: fast_grid,
            script_globals: script_globals,
            campaign: None,
            mission_stat: None,
            diplomacy: None,
            sequence_manager: None,
            selected_pcs: None,
            selected_action: None,
            short_briefings: None,
            standard_view_radius: None,
            view_radius_cache: None,
            sight_obstacles: None,
            sound_sources: None,
            weather: None,
            frame_counter: None,
        }
    }

    /// Attach the original game's live player-character view. This is
    /// distinct from entity storage: a replaced PC corpse remains an entity
    /// after it is retired from party-oriented script queries.
    pub fn with_pc_registry(mut self, pc_registry: &'a [EntityId]) -> Self {
        self.pc_registry = Some(pc_registry);
        self
    }

    pub fn with_queries(
        mut self,
        sequence_manager: &'a mut crate::sequence::SequenceManager,
        selected_pcs: &'a mut Vec<EntityId>,
        sound_sources: &'a mut crate::sound_source::SoundSourceManager,
        weather: &'a crate::engine::WeatherState,
        frame_counter: &'a u32,
    ) -> Self {
        self.sequence_manager = Some(sequence_manager);
        self.selected_pcs = Some(selected_pcs);
        self.sound_sources = Some(sound_sources);
        self.weather = Some(weather);
        self.frame_counter = Some(frame_counter);
        self
    }

    /// Attach the original game's globally armed player action.
    /// This is intentionally separate from each PC's remembered action.
    pub fn with_selected_action(mut self, action: &'a mut crate::profiles::Action) -> Self {
        self.selected_action = Some(action);
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
        self.campaign = Some(campaign);
        self.mission_stat = Some(mission_stat);
        self
    }

    pub fn with_diplomacy(mut self, diplomacy: &'a mut crate::diplomacy::DiplomacyState) -> Self {
        self.diplomacy = Some(diplomacy);
        self
    }

    /// Attach the canonical objective/short-briefing model. Both vanilla
    /// briefing natives and Spellforge objective extensions write this owner
    /// before returning to the VM.
    pub fn with_short_briefings(
        mut self,
        short_briefings: &'a mut crate::short_briefings::ShortBriefings,
    ) -> Self {
        self.short_briefings = Some(short_briefings);
        self
    }

    /// Attach the AI-domain radius paired with the live entity store. The
    /// `SetViewRadius` native updates both before returning, matching
    /// Original-game standard view-radius update followed by every NPC's
    /// the original game's view-radius initialization.
    pub fn with_standard_view_radius(mut self, radius: &'a mut u16) -> Self {
        self.standard_view_radius = Some(radius);
        self
    }

    pub(crate) fn with_view_radius_cache(
        mut self,
        cache: &'a mut crate::ai_vision::ViewRadiusCache,
    ) -> Self {
        self.view_radius_cache = Some(cache);
        self
    }

    #[cfg(test)]
    pub(crate) fn entities_owner_ptr(&self) -> *const crate::entities::Entities {
        std::ptr::from_ref(self.entities)
    }
}

/// Transient receiver context for one script callback.
///
/// This mirrors the original game's separately bracketed script receiver and
/// original-game scroll execution values. Frames are copied into a
/// [`NativeContext`] for one VM resume, but are owned and stacked by
/// `MissionScript`; they are never part of a mission snapshot or state hash.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
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
}

/// Short-lived native dispatcher assembled for one VM resume.
///
/// Script globals, computed locations, and recorder state are borrowed from
/// their sole owner on `MissionScript`. Yielded native requests leave this
/// context before the engine executes callbacks.
pub struct NativeContext<'ctx, 'owners: 'ctx> {
    pub(crate) simulation: &'ctx crate::sim_rng::SimulationContext,
    pub(crate) entities: &'ctx mut crate::entities::Entities,
    pub(crate) pc_registry: Option<&'owners [EntityId]>,
    pub(crate) ai_global: &'ctx mut crate::ai::AiGlobalState,
    pub(crate) fast_grid: &'ctx mut crate::fast_find_grid::FastFindGrid,
    pub(crate) script_state: &'ctx mut ScriptState,
    pub(crate) script_globals: &'ctx mut Vec<i32>,
    pub(crate) script_domains: &'ctx mut crate::engine::ScriptDomains,
    pub(crate) bindings: ScriptBindings<'ctx>,
    pub(crate) campaign: Option<&'ctx mut crate::campaign::Campaign>,
    pub(crate) mission_stat: Option<&'ctx mut crate::mission_stat::MissionStat>,
    pub(crate) diplomacy: Option<&'ctx mut crate::diplomacy::DiplomacyState>,
    pub(crate) sequence_manager: Option<&'ctx mut crate::sequence::SequenceManager>,
    pub(crate) selected_pcs: Option<&'ctx mut Vec<EntityId>>,
    pub(crate) selected_action: Option<&'ctx mut crate::profiles::Action>,
    pub(crate) short_briefings: Option<&'ctx mut crate::short_briefings::ShortBriefings>,
    pub(crate) standard_view_radius: Option<&'ctx mut u16>,
    pub(crate) view_radius_cache: Option<&'ctx mut crate::ai_vision::ViewRadiusCache>,
    pub(crate) sight_obstacles: Option<crate::sight_obstacle::ObstacleList<'owners>>,
    pub(crate) sound_sources: Option<&'ctx mut crate::sound_source::SoundSourceManager>,
    pub(crate) weather: Option<&'owners crate::engine::WeatherState>,
    pub(crate) frame_counter: Option<&'owners u32>,
    pub(crate) call_frame: ScriptCallFrame,
    pub(crate) script_vm_diagnostic: Option<crate::sim_rng::ScriptVmDiagnosticContext>,
    pub(crate) pending_yield: Option<crate::interp::NativeYield>,
}

impl<'ctx, 'owners: 'ctx> NativeContext<'ctx, 'owners> {
    pub(crate) fn is_player_aligned_camp(&self, camp: crate::element::Camp) -> bool {
        self.diplomacy
            .as_deref()
            .expect("allegiance natives require an attached DiplomacyState")
            .is_player_aligned(camp)
    }

    pub(crate) fn is_hostile_to_player(&self, camp: crate::element::Camp) -> bool {
        self.diplomacy
            .as_deref()
            .expect("hostility natives require an attached DiplomacyState")
            .is_hostile_to_player(camp)
    }

    pub fn new(
        script_state: &'ctx mut ScriptState,
        script_domains: &'ctx mut crate::engine::ScriptDomains,
        capabilities: &'ctx mut NativeSessionCapabilities<'owners>,
    ) -> Self {
        Self {
            simulation: capabilities.simulation,
            entities: &mut *capabilities.entities,
            pc_registry: capabilities.pc_registry,
            ai_global: &mut *capabilities.ai_global,
            fast_grid: &mut *capabilities.fast_grid,
            script_state,
            script_globals: &mut *capabilities.script_globals,
            script_domains,
            bindings: ScriptBindings::empty(),
            campaign: capabilities.campaign.as_deref_mut(),
            mission_stat: capabilities.mission_stat.as_deref_mut(),
            diplomacy: capabilities.diplomacy.as_deref_mut(),
            sequence_manager: capabilities.sequence_manager.as_deref_mut(),
            selected_pcs: capabilities.selected_pcs.as_deref_mut(),
            selected_action: capabilities.selected_action.as_deref_mut(),
            short_briefings: capabilities.short_briefings.as_deref_mut(),
            standard_view_radius: capabilities.standard_view_radius.as_deref_mut(),
            view_radius_cache: capabilities.view_radius_cache.as_deref_mut(),
            sight_obstacles: capabilities.sight_obstacles,
            sound_sources: capabilities.sound_sources.as_deref_mut(),
            weather: capabilities.weather,
            frame_counter: capabilities.frame_counter,
            call_frame: ScriptCallFrame::default(),
            script_vm_diagnostic: None,
            pending_yield: None,
        }
    }

    pub fn with_bindings(
        script_state: &'ctx mut ScriptState,
        script_domains: &'ctx mut crate::engine::ScriptDomains,
        bindings: &'ctx AttachedScriptBindings,
        capabilities: &'ctx mut NativeSessionCapabilities<'owners>,
    ) -> Self {
        Self::with_call_frame(
            script_state,
            script_domains,
            bindings,
            capabilities,
            ScriptCallFrame::default(),
        )
    }

    pub fn with_call_frame(
        script_state: &'ctx mut ScriptState,
        script_domains: &'ctx mut crate::engine::ScriptDomains,
        bindings: &'ctx AttachedScriptBindings,
        capabilities: &'ctx mut NativeSessionCapabilities<'owners>,
        call_frame: ScriptCallFrame,
    ) -> Self {
        let mut context = Self::new(script_state, script_domains, capabilities);
        context.bindings = bindings.view();
        context.call_frame = call_frame;
        context
    }

    pub fn bindings(&self) -> ScriptBindings<'_> {
        self.bindings
    }

    pub fn script_state(&self) -> &ScriptState {
        self.script_state
    }

    pub fn script_globals(&self) -> &[i32] {
        &self.script_globals
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

    // These forwards let native dispatch spell handle conversions as
    // `Self::..` while the stateless codec remains the single owner of the
    // script-handle representation.
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

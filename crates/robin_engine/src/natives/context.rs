use std::{
    cell::{RefCell, RefMut},
    ops::{Deref, DerefMut},
};

use super::{AttachedScriptBindings, GameHost, ScriptBindings, ScriptState};
use crate::element::EntityId;

/// Canonical mutable AI-global owner borrowed for one script session.
///
/// A native resume holds the [`RefMut`] produced by this capability only until
/// that resume yields or returns. Nested script dispatch can then borrow the
/// same `EngineInner` allocation without parking or copying it into
/// [`GameHost`].
pub(crate) struct NativeAiGlobalCapability<'a> {
    ai_global: RefCell<&'a mut crate::ai::AiGlobalState>,
}

pub(crate) trait NativeAiGlobalAccess {
    fn ai_global(&self) -> RefMut<'_, crate::ai::AiGlobalState>;
}

impl<'a> NativeAiGlobalCapability<'a> {
    pub(crate) fn new(ai_global: &'a mut crate::ai::AiGlobalState) -> Self {
        Self {
            ai_global: RefCell::new(ai_global),
        }
    }
}

impl NativeAiGlobalAccess for NativeAiGlobalCapability<'_> {
    fn ai_global(&self) -> RefMut<'_, crate::ai::AiGlobalState> {
        RefMut::map(self.ai_global.borrow_mut(), |ai_global| &mut **ai_global)
    }
}

/// Canonical runtime fast-grid owner borrowed for one script session.
///
/// The `RefCell` sequences individual VM resumes, including synchronous
/// nested dispatch. The grid itself never leaves `EngineInner`, so native
/// reads and mutations always address the engine's exact allocation.
pub struct NativeFastGridCapability<'a> {
    fast_grid: RefCell<&'a mut crate::fast_find_grid::FastFindGrid>,
}

trait NativeFastGridAccess {
    fn fast_grid(&self) -> RefMut<'_, crate::fast_find_grid::FastFindGrid>;
}

impl<'a> NativeFastGridCapability<'a> {
    pub fn new(fast_grid: &'a mut crate::fast_find_grid::FastFindGrid) -> Self {
        Self {
            fast_grid: RefCell::new(fast_grid),
        }
    }
}

impl NativeFastGridAccess for NativeFastGridCapability<'_> {
    fn fast_grid(&self) -> RefMut<'_, crate::fast_find_grid::FastFindGrid> {
        RefMut::map(self.fast_grid.borrow_mut(), |fast_grid| &mut **fast_grid)
    }
}

/// Canonical mutable campaign owners borrowed for one script session.
///
/// The cells only sequence short-lived VM resumes. A [`NativeContext`] holds
/// each mutable borrow until that resume returns; nested script dispatch can
/// then construct a fresh context over these same owners without parking or
/// copying either value into [`GameHost`].
pub(crate) struct NativeCampaignCapabilities<'a> {
    campaign: RefCell<&'a mut crate::campaign::Campaign>,
    mission_stat: RefCell<&'a mut crate::mission_stat::MissionStat>,
}

pub(crate) trait NativeCampaignAccess {
    fn campaign(&self) -> RefMut<'_, crate::campaign::Campaign>;
    fn mission_stat(&self) -> RefMut<'_, crate::mission_stat::MissionStat>;
}

impl<'a> NativeCampaignCapabilities<'a> {
    pub(crate) fn new(
        campaign: &'a mut crate::campaign::Campaign,
        mission_stat: &'a mut crate::mission_stat::MissionStat,
    ) -> Self {
        Self {
            campaign: RefCell::new(campaign),
            mission_stat: RefCell::new(mission_stat),
        }
    }
}

impl NativeCampaignAccess for NativeCampaignCapabilities<'_> {
    fn campaign(&self) -> RefMut<'_, crate::campaign::Campaign> {
        RefMut::map(self.campaign.borrow_mut(), |campaign| &mut **campaign)
    }

    fn mission_stat(&self) -> RefMut<'_, crate::mission_stat::MissionStat> {
        RefMut::map(self.mission_stat.borrow_mut(), |mission_stat| {
            &mut **mission_stat
        })
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

/// Canonical read capabilities borrowed for exactly one VM resume.
///
/// These owners stay in `EngineInner`; unlike the former `GameHost` fields,
/// this view cannot be serialized, refreshed, or observed after the callback.
/// Capability references are intentionally opaque fields on one copyable
/// carrier so later ownership lanes can fold them into a single bundled
/// session view without changing native call sites or Lua's scoped transport.
#[derive(Clone, Copy, Default)]
pub struct NativeQueryViews<'a> {
    sequence_manager: Option<&'a crate::sequence::SequenceManager>,
    selected_pcs: Option<&'a [EntityId]>,
    sound_sources: Option<&'a crate::sound_source::SoundSourceManager>,
    weather: Option<&'a crate::engine::WeatherState>,
    frame_counter: Option<&'a u32>,
    ai_global_capability: Option<&'a dyn NativeAiGlobalAccess>,
    campaign_capabilities: Option<&'a dyn NativeCampaignAccess>,
    fast_grid_capability: Option<&'a dyn NativeFastGridAccess>,
}

impl<'a> NativeQueryViews<'a> {
    pub fn new(
        sequence_manager: &'a crate::sequence::SequenceManager,
        selected_pcs: &'a [EntityId],
        sound_sources: &'a crate::sound_source::SoundSourceManager,
        weather: &'a crate::engine::WeatherState,
        frame_counter: &'a u32,
    ) -> Self {
        Self {
            sequence_manager: Some(sequence_manager),
            selected_pcs: Some(selected_pcs),
            sound_sources: Some(sound_sources),
            weather: Some(weather),
            frame_counter: Some(frame_counter),
            ai_global_capability: None,
            campaign_capabilities: None,
            fast_grid_capability: None,
        }
    }

    pub(crate) fn with_ai_global_capability(
        mut self,
        ai_global: &'a dyn NativeAiGlobalAccess,
    ) -> Self {
        self.ai_global_capability = Some(ai_global);
        self
    }

    #[doc(hidden)]
    pub fn has_ai_global_capability(self) -> bool {
        self.ai_global_capability.is_some()
    }

    pub(crate) fn with_campaign_capabilities(
        mut self,
        campaign: &'a dyn NativeCampaignAccess,
    ) -> Self {
        self.campaign_capabilities = Some(campaign);
        self
    }

    /// Attach the canonical runtime grid for the duration of this script
    /// session. Public for the Lua adapter, which must carry the same scoped
    /// capability through its `'static` callback shims without cloning it.
    pub fn with_fast_grid_capability(
        mut self,
        fast_grid: &'a NativeFastGridCapability<'_>,
    ) -> Self {
        self.fast_grid_capability = Some(fast_grid);
        self
    }

    #[doc(hidden)]
    pub fn sequence_manager_option(self) -> Option<&'a crate::sequence::SequenceManager> {
        self.sequence_manager
    }

    #[doc(hidden)]
    pub fn selected_pcs_option(self) -> Option<&'a [EntityId]> {
        self.selected_pcs
    }

    #[doc(hidden)]
    pub fn sound_sources_option(self) -> Option<&'a crate::sound_source::SoundSourceManager> {
        self.sound_sources
    }

    #[doc(hidden)]
    pub fn weather_option(self) -> Option<&'a crate::engine::WeatherState> {
        self.weather
    }

    #[doc(hidden)]
    pub fn frame_counter_option(self) -> Option<&'a u32> {
        self.frame_counter
    }

    pub(crate) fn sequence_manager(self) -> &'a crate::sequence::SequenceManager {
        self.sequence_manager
            .expect("script native requires a live SequenceManager query view")
    }

    pub(crate) fn selected_pcs(self) -> &'a [EntityId] {
        self.selected_pcs
            .expect("script native requires a live player-selection query view")
    }

    pub(crate) fn sound_sources(self) -> &'a crate::sound_source::SoundSourceManager {
        self.sound_sources
            .expect("script native requires a live SoundSourceManager query view")
    }

    pub(crate) fn weather(self) -> &'a crate::engine::WeatherState {
        self.weather
            .expect("script native requires a live WeatherState query view")
    }

    pub(crate) fn frame_counter(self) -> u32 {
        *self
            .frame_counter
            .expect("script native requires a live simulation-clock query view")
    }
}

/// Short-lived native dispatcher assembled for one VM resume.
///
/// `GameHost` still owns the not-yet-migrated engine adapter state. Script
/// globals, computed locations, and recorder state are borrowed from their
/// sole owner on `MissionScript` and are never copied into that adapter.
pub struct NativeContext<'a> {
    pub(crate) game_host: &'a mut GameHost,
    pub(crate) script_state: &'a mut ScriptState,
    pub(crate) script_domains: &'a mut crate::engine::ScriptDomains,
    pub(crate) bindings: ScriptBindings<'a>,
    pub(crate) queries: NativeQueryViews<'a>,
    pub(crate) ai_global: Option<RefMut<'a, crate::ai::AiGlobalState>>,
    pub(crate) campaign: Option<RefMut<'a, crate::campaign::Campaign>>,
    pub(crate) mission_stat: Option<RefMut<'a, crate::mission_stat::MissionStat>>,
    fast_grid: Option<RefMut<'a, crate::fast_find_grid::FastFindGrid>>,
    pub(crate) call_frame: ScriptCallFrame,
}

impl<'a> NativeContext<'a> {
    pub fn new(
        game_host: &'a mut GameHost,
        script_state: &'a mut ScriptState,
        script_domains: &'a mut crate::engine::ScriptDomains,
    ) -> Self {
        Self {
            game_host,
            script_state,
            script_domains,
            bindings: ScriptBindings::empty(),
            queries: NativeQueryViews::default(),
            ai_global: None,
            campaign: None,
            mission_stat: None,
            fast_grid: None,
            call_frame: ScriptCallFrame::default(),
        }
    }

    pub fn with_bindings(
        game_host: &'a mut GameHost,
        script_state: &'a mut ScriptState,
        script_domains: &'a mut crate::engine::ScriptDomains,
        bindings: &'a AttachedScriptBindings,
        queries: NativeQueryViews<'a>,
    ) -> Self {
        let ai_global = queries
            .ai_global_capability
            .map(NativeAiGlobalAccess::ai_global);
        let campaign = queries
            .campaign_capabilities
            .map(NativeCampaignAccess::campaign);
        let mission_stat = queries
            .campaign_capabilities
            .map(NativeCampaignAccess::mission_stat);
        let fast_grid = queries
            .fast_grid_capability
            .map(NativeFastGridAccess::fast_grid);
        Self {
            game_host,
            script_state,
            script_domains,
            bindings: bindings.view(),
            queries,
            ai_global,
            campaign,
            mission_stat,
            fast_grid,
            call_frame: ScriptCallFrame::default(),
        }
    }

    pub fn with_call_frame(
        game_host: &'a mut GameHost,
        script_state: &'a mut ScriptState,
        script_domains: &'a mut crate::engine::ScriptDomains,
        bindings: &'a AttachedScriptBindings,
        queries: NativeQueryViews<'a>,
        call_frame: ScriptCallFrame,
    ) -> Self {
        let ai_global = queries
            .ai_global_capability
            .map(NativeAiGlobalAccess::ai_global);
        let campaign = queries
            .campaign_capabilities
            .map(NativeCampaignAccess::campaign);
        let mission_stat = queries
            .campaign_capabilities
            .map(NativeCampaignAccess::mission_stat);
        let fast_grid = queries
            .fast_grid_capability
            .map(NativeFastGridAccess::fast_grid);
        Self {
            game_host,
            script_state,
            script_domains,
            bindings: bindings.view(),
            queries,
            ai_global,
            campaign,
            mission_stat,
            fast_grid,
            call_frame,
        }
    }

    pub fn bindings(&self) -> ScriptBindings<'_> {
        self.bindings
    }

    pub fn script_state(&self) -> &ScriptState {
        self.script_state
    }

    pub fn game_host(&self) -> &GameHost {
        self.game_host
    }

    pub fn script_state_mut(&mut self) -> &mut ScriptState {
        self.script_state
    }

    pub(crate) fn ai_global(&self) -> &crate::ai::AiGlobalState {
        self.ai_global
            .as_deref()
            .expect("script native requires the live canonical AiGlobalState capability")
    }

    pub(crate) fn ai_global_mut(&mut self) -> &mut crate::ai::AiGlobalState {
        self.ai_global
            .as_deref_mut()
            .expect("script native requires the live canonical AiGlobalState capability")
    }

    pub(crate) fn fast_grid(&self) -> &crate::fast_find_grid::FastFindGrid {
        self.fast_grid
            .as_deref()
            .expect("script native requires the canonical runtime FastFindGrid capability")
    }

    pub(crate) fn fast_grid_mut(&mut self) -> &mut crate::fast_find_grid::FastFindGrid {
        self.fast_grid
            .as_deref_mut()
            .expect("script native requires the canonical runtime FastFindGrid capability")
    }

    // Associated functions do not participate in deref method lookup. Keep
    // these small forwards while dispatch is split between NativeContext and
    // the legacy GameHost helper implementation.
    pub(crate) fn actor_handle<I: Into<EntityId>>(id: I) -> i32 {
        GameHost::actor_handle(id)
    }

    pub(crate) fn actor_handle_from_index(index: usize) -> i32 {
        GameHost::actor_handle_from_index(index)
    }

    pub(crate) fn sound_source_handle_from_index(index: usize) -> i32 {
        GameHost::sound_source_handle_from_index(index)
    }

    pub(crate) fn actor_handle_index(handle: i32) -> Option<usize> {
        GameHost::actor_handle_index(handle)
    }

    pub(crate) fn door_index(handle: i32) -> Option<usize> {
        GameHost::door_index(handle)
    }

    pub(crate) fn patch_index(handle: i32) -> Option<usize> {
        GameHost::patch_index(handle)
    }

    pub(crate) fn location_index(handle: i32) -> Option<usize> {
        GameHost::location_index(handle)
    }

    pub(crate) fn sound_source_index(handle: i32) -> Option<usize> {
        GameHost::sound_source_index(handle)
    }

    pub(crate) fn building_index(handle: i32) -> Option<usize> {
        GameHost::building_index(handle)
    }

    pub(crate) fn way_index(handle: i32) -> Option<usize> {
        GameHost::way_index(handle)
    }
}

impl Deref for NativeContext<'_> {
    type Target = GameHost;

    fn deref(&self) -> &Self::Target {
        self.game_host
    }
}

impl DerefMut for NativeContext<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.game_host
    }
}

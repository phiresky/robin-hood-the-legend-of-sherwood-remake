use std::ops::{Deref, DerefMut};

use super::{AttachedScriptBindings, GameHost, ScriptBindings, ScriptState};
use crate::element::EntityId;

/// Canonical read capabilities borrowed for exactly one VM resume.
///
/// These owners stay in `EngineInner`; unlike the former `GameHost` fields,
/// this view cannot be serialized, refreshed, or observed after the callback.
#[derive(Clone, Copy, Default)]
pub struct NativeQueryViews<'a> {
    sequence_manager: Option<&'a crate::sequence::SequenceManager>,
    selected_pcs: Option<&'a [EntityId]>,
    sound_sources: Option<&'a crate::sound_source::SoundSourceManager>,
    weather: Option<&'a crate::engine::WeatherState>,
    frame_counter: Option<&'a u32>,
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
        }
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
        }
    }

    pub fn with_bindings(
        game_host: &'a mut GameHost,
        script_state: &'a mut ScriptState,
        script_domains: &'a mut crate::engine::ScriptDomains,
        bindings: &'a AttachedScriptBindings,
        queries: NativeQueryViews<'a>,
    ) -> Self {
        Self {
            game_host,
            script_state,
            script_domains,
            bindings: bindings.view(),
            queries,
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

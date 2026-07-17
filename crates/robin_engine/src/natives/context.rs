use std::ops::{Deref, DerefMut};

use super::{AttachedScriptBindings, GameHost, ScriptBindings, ScriptState};
use crate::element::EntityId;

/// Short-lived native dispatcher assembled for one VM resume.
///
/// `GameHost` still owns the not-yet-migrated engine adapter state. Script
/// globals, computed locations, and recorder state are borrowed from their
/// sole owner on `MissionScript` and are never copied into that adapter.
pub struct NativeContext<'a> {
    pub(crate) game_host: &'a mut GameHost,
    pub(crate) script_state: &'a mut ScriptState,
    pub(crate) bindings: ScriptBindings<'a>,
}

impl<'a> NativeContext<'a> {
    pub fn new(game_host: &'a mut GameHost, script_state: &'a mut ScriptState) -> Self {
        Self {
            game_host,
            script_state,
            bindings: ScriptBindings::empty(),
        }
    }

    pub fn with_bindings(
        game_host: &'a mut GameHost,
        script_state: &'a mut ScriptState,
        bindings: &'a AttachedScriptBindings,
    ) -> Self {
        Self {
            game_host,
            script_state,
            bindings: bindings.view(),
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

//! Browser boundary for custom-mission Lua support.
//!
//! `mlua` supports WebAssembly through Emscripten, while the game uses
//! `wasm32-unknown-unknown` with wasm-bindgen. Keep ordinary browser missions
//! available and reject Spellforge launches explicitly until those targets
//! can be reconciled.

use robin_engine::natives::{ScriptEffects, ScriptState};

use crate::main_entry::CliArgs;
use crate::main_menu::custom_missions::CustomMissionLaunch;

pub struct LuaSession {
    mission_basename: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SpellforgeSessionError {
    #[error(
        "Spellforge mission `{mission}` cannot start in the browser: mlua does not support the wasm-bindgen target"
    )]
    UnsupportedBrowser { mission: String },
}

pub fn validate_launch_mode(
    args: &CliArgs,
    _pending_replay: bool,
) -> Result<(), SpellforgeSessionError> {
    let Some(pending) = args
        .pending_lua_mission
        .as_ref()
        .filter(|pending| pending.requires_spellforge)
    else {
        return Ok(());
    };
    Err(SpellforgeSessionError::UnsupportedBrowser {
        mission: pending.rhm_basename.clone(),
    })
}

impl LuaSession {
    pub fn start_for_launch(
        launch: &CustomMissionLaunch,
        _mods_root: &std::path::Path,
    ) -> Result<Option<Self>, SpellforgeSessionError> {
        if launch.requires_spellforge {
            return Err(SpellforgeSessionError::UnsupportedBrowser {
                mission: launch.rhm_basename.clone(),
            });
        }
        Ok(None)
    }

    pub fn mission_basename(&self) -> &str {
        &self.mission_basename
    }

    pub fn run_required_startup_events(
        &self,
        _native_parts: Option<(
            &mut ScriptEffects,
            &mut ScriptState,
            &mut robin_engine::engine::ScriptDomains,
            &robin_engine::natives::AttachedScriptBindings,
            &robin_engine::natives::NativeSessionCapabilities<'_>,
        )>,
        _initialization_seed: i32,
    ) -> Result<(), SpellforgeSessionError> {
        Err(SpellforgeSessionError::UnsupportedBrowser {
            mission: self.mission_basename.clone(),
        })
    }
}

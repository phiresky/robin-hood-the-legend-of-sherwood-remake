//! Legacy `robin_lua` (mlua) direct-call VM harness.
//!
//! Live missions never load their package into this second VM; all bootstrap
//! and event code runs exactly once in the engine-owned Spellforge runtime.
//! The harness is retained only for focused host/native adapter tests that
//! compare native bindings against a direct Lua call path.

use robin_engine::engine::ScriptDomains;
use robin_engine::natives::{
    AttachedScriptBindings, NativeSessionCapabilities, ScriptEffects, ScriptState,
};
use robin_lua::MissionLuaState;
use tempfile::TempDir;

use crate::lua_session::LuaSession;

/// A [`LuaSession`] paired with a legacy direct-call interpreter that has the
/// same mission script loaded.
pub(super) struct LegacyVmSession {
    /// Holds the extracted `.lua` files. Lives at least as long as `state` so
    /// `require()` lookups stay valid.
    pub(super) _tempdir: TempDir,
    pub(super) state: MissionLuaState,
    pub(super) session: LuaSession,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum LegacyEventError {
    #[error("Lua event `{event}` failed for mission `{mission}`: {source}")]
    Event {
        mission: String,
        event: String,
        #[source]
        source: mlua::Error,
    },
    #[error(
        "Lua event `{event}` for mission `{mission}` returned Lua {actual}; expected an integer, integral number, boolean, or nil"
    )]
    UnexpectedEventReturn {
        mission: String,
        event: String,
        actual: String,
    },
    #[error(
        "Lua event `{event}` for mission `{mission}` returned integer {value}, which is outside the signed 32-bit game ABI range"
    )]
    EventIntegerOutOfRange {
        mission: String,
        event: String,
        value: i64,
    },
}

#[derive(Debug, thiserror::Error)]
pub(super) enum LegacyStartupError {
    #[error(
        "required Spellforge event `{event}` for mission `{mission}` has no mission-script ScriptEffects"
    )]
    MissingScriptEffects {
        mission: String,
        event: &'static str,
    },
    #[error("required Spellforge event `{event}` failed for mission `{mission}`: {source}")]
    RequiredEvent {
        mission: String,
        event: &'static str,
        #[source]
        source: LegacyEventError,
    },
}

impl LegacyVmSession {
    fn mission(&self) -> &str {
        self.session.mission_basename()
    }

    /// Look up a top-level event function on the Lua globals and call it with
    /// the engine's [`ScriptEffects`] attached. A missing function or no
    /// explicit return is a successful no-op; a Lua failure or incompatible
    /// return is preserved as a typed [`LegacyEventError`].
    ///
    /// TODO(parity): The Spellforge DLL's `luaRun` implementation is not in
    /// the available material; verify its accepted event return conversions if that
    /// source becomes available. Runtime errors must remain errors regardless.
    pub(super) fn run_event(
        &self,
        host: &mut ScriptEffects,
        script_state: &mut ScriptState,
        script_domains: &mut ScriptDomains,
        capabilities: &NativeSessionCapabilities<'_>,
        event_name: &str,
        args: &[i32],
    ) -> Result<i32, LegacyEventError> {
        self.run_event_with_bindings(
            host,
            script_state,
            script_domains,
            AttachedScriptBindings::empty_ref(),
            capabilities,
            event_name,
            args,
        )
    }

    fn run_event_with_bindings(
        &self,
        host: &mut ScriptEffects,
        script_state: &mut ScriptState,
        script_domains: &mut ScriptDomains,
        bindings: &AttachedScriptBindings,
        capabilities: &NativeSessionCapabilities<'_>,
        event_name: &str,
        args: &[i32],
    ) -> Result<i32, LegacyEventError> {
        let result = self.state.with_host_state_and_bindings(
            host,
            script_state,
            script_domains,
            bindings,
            capabilities,
            |lua| {
                let globals = lua.globals();
                let v: mlua::Value = globals.get(event_name)?;
                let Some(func) = (match &v {
                    mlua::Value::Function(f) => Some(f.clone()),
                    _ => None,
                }) else {
                    tracing::debug!(
                        "LuaSession[{}]: no global function `{event_name}`",
                        self.mission()
                    );
                    return Ok(None);
                };
                let mut variadic: mlua::Variadic<mlua::Value> = mlua::Variadic::new();
                for a in args {
                    variadic.push(mlua::Value::Integer((*a).into()));
                }
                let ret: mlua::MultiValue = func.call(variadic)?;
                Ok(ret.into_iter().next())
            },
        );

        let returned = result.map_err(|source| LegacyEventError::Event {
            mission: self.mission().to_owned(),
            event: event_name.to_owned(),
            source,
        })?;
        match returned {
            None | Some(mlua::Value::Nil) => Ok(0),
            Some(mlua::Value::Integer(value)) => {
                i32::try_from(value).map_err(|_| LegacyEventError::EventIntegerOutOfRange {
                    mission: self.mission().to_owned(),
                    event: event_name.to_owned(),
                    value,
                })
            }
            Some(mlua::Value::Number(value))
                if value.is_finite()
                    && value.fract() == 0.0
                    && value >= i32::MIN as f64
                    && value <= i32::MAX as f64 =>
            {
                Ok(value as i32)
            }
            Some(mlua::Value::Boolean(value)) => Ok(i32::from(value)),
            Some(value) => Err(LegacyEventError::UnexpectedEventReturn {
                mission: self.mission().to_owned(),
                event: event_name.to_owned(),
                actual: value.type_name().to_owned(),
            }),
        }
    }

    /// Dispatch the required Spellforge startup pair in order. Failure stops
    /// startup immediately and is returned with both mission and event context.
    pub(super) fn run_required_startup_events(
        &self,
        native_parts: Option<(
            &mut ScriptEffects,
            &mut ScriptState,
            &mut ScriptDomains,
            &AttachedScriptBindings,
            &NativeSessionCapabilities<'_>,
        )>,
        initialization_seed: i32,
    ) -> Result<(), LegacyStartupError> {
        let Some((host, script_state, script_domains, bindings, capabilities)) = native_parts
        else {
            return Err(LegacyStartupError::MissingScriptEffects {
                mission: self.mission().to_owned(),
                event: "Initialize",
            });
        };
        for (event, args) in [
            ("Initialize", std::slice::from_ref(&initialization_seed)),
            ("PostInitialize", &[][..]),
        ] {
            self.run_event_with_bindings(
                host,
                script_state,
                script_domains,
                bindings,
                capabilities,
                event,
                args,
            )
            .map_err(|source| LegacyStartupError::RequiredEvent {
                mission: self.mission().to_owned(),
                event,
                source,
            })?;
        }
        Ok(())
    }
}

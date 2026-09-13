//! AI-side adapters over the shared [`ParityGate`].
//!
//! Callers keep their own `OnceLock` so each diagnostic samples the environment
//! once, only when first consulted. Diagnostics never enter simulation state.

use crate::engine::diagnostics::ParityGate;

/// A master switch without filters (`<GATE>` set ⇒ enabled).
pub(crate) fn switch_gate(gate: &str) -> ParityGate<0> {
    ParityGate::from_env(gate, [])
}

/// A gate whose numeric filters are all mandatory once the master switch is
/// set. Enabling it without every filter is an operator error: broad traces
/// make same-frame re-entrant ownership impossible to attribute. Malformed
/// values panic inside [`ParityGate::from_env`]. Match with
/// [`ParityGate::matches_required`] so an absent observed identity never
/// matches.
pub(crate) fn required_parity_gate<const N: usize>(gate: &str, names: [&str; N]) -> ParityGate<N> {
    let parsed = ParityGate::from_env(gate, names);
    if parsed.enabled() {
        for name in names {
            if std::env::var_os(name).is_none() {
                panic!("{name} is required when {gate} is enabled");
            }
        }
    }
    parsed
}

//! Parity stderr payloads, separated from AI decisions.
//!
//! Callers own gates and argument evaluation; each event is a named-field
//! struct whose `Display` is the byte-stable payload line, so call sites can no
//! longer transpose positional arguments. Keep payload text and field order
//! stable for replay tooling. Most lines are `key=value` text or pseudo-JSON
//! embedding Rust `Debug`/`Display` output (`Some(3)`, enum names, `1` for
//! `1.0f32`), which serde_json cannot reproduce byte-for-byte, so these fields
//! stay type-erased formatting borrows. Strict-JSON events use serde_json
//! (see `ai_enemy::parity_trace`).

/// Declare a parity event struct. Fields before `;` fill the positional `{}`
/// placeholders in declaration order; fields after `;` are named captures.
macro_rules! trace_event {
    (
        $name:ident {
            $($argument:ident: $format_trait:ident),*;
            $($capture:ident: $capture_trait:ident),*
        } => $format:literal
    ) => {
        /// Parity payload line: construct with named fields, then `emit()`.
        pub(super) struct $name<'a> {
            $(pub(super) $argument: &'a dyn std::fmt::$format_trait,)*
            $(pub(super) $capture: &'a dyn std::fmt::$capture_trait,)*
        }

        impl std::fmt::Display for $name<'_> {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    formatter,
                    $format,
                    $(self.$argument,)*
                    $($capture = self.$capture,)*
                )
            }
        }

        impl $name<'_> {
            #[cold]
            #[inline(never)]
            pub(super) fn emit(&self) {
                $crate::ai::parity_trace::write_line(self);
            }
        }
    };
}

pub(crate) use trace_event;

/// Single stderr sink for every parity payload line.
pub(crate) fn write_line(line: &dyn std::fmt::Display) {
    eprintln!("{line}");
}

#[cfg(test)]
mod tests;

trace_event! {
    Forecast {
        out_x: Display,
        out_y: Display,
        sector: Display,
        gates: Display;
        input: Debug,
        layer: Display,
        direction: Display,
        entry_gate: Debug
    } => "FORECAST input={input:?} out=({}, {}, sector={}, layer={layer}) dir={direction} gates={} entry={entry_gate:?}"
}

trace_event! {
    BoredBoundaryGetBoredTime {
        frame: Display,
        owner: Display,
        state: Debug,
        substate: Debug,
        rank: Debug,
        pride: Display,
        min: Display,
        delta: Display,
        timer_running: Display,
        timer_deadline: Display,
        self_stimuli: Display;
    } => "BORED_BOUNDARY frame={} owner={} phase=get_bored_time state={:?} substate={:?} rank={:?} pride={} min={} delta={} timer_running={} timer_deadline={} self_stimuli={}"
}

trace_event! {
    Macrolife {
        frame: Display,
        owner_creation_order: Debug,
        me: Display,
        state: Debug,
        substate: Debug,
        in_progress: Display,
        timer_running: Display,
        timer_deadline: Display,
        started_this_frame: Display,
        command_offset: Display,
        remaining_bytes: Display,
        waypoint: Debug,
        self_stimuli_len: Display;
        phase: Display,
        reason: Debug
    } => "MACROLIFE frame={} owner={:?} me={} phase={phase} reason={reason:?} state={:?} substate={:?} in_progress={} timer_running={} timer_deadline={} started_this_frame={} command_offset={} remaining_bytes={} waypoint={:?} self_stimuli_len={}"
}

trace_event! {
    Willstop {
        frame: Display,
        owner: Debug,
        forecast_after: Display,
        value_after: Display;
        caller: Debug,
        path: Debug,
        waypoint: Debug,
        forecasted_before: Display,
        value_before: Display,
        result: Display
    } => "WILLSTOP frame={} owner={:?} caller={caller:?} path={path:?} waypoint={waypoint:?} forecast_before={forecasted_before} value_before={value_before} forecast_after={} value_after={} result={result}"
}

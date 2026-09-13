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
        self_stimuli: Display,
        owner_work: Display,
        orders: Display;
    } => "BORED_BOUNDARY frame={} owner={} phase=get_bored_time state={:?} substate={:?} rank={:?} pride={} min={} delta={} timer_running={} timer_deadline={} self_stimuli={} owner_work={} orders={}"
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
        owner_work_len: Display,
        self_stimuli_len: Display;
        phase: Display,
        reason: Debug
    } => "MACROLIFE frame={} owner={:?} me={} phase={phase} reason={reason:?} state={:?} substate={:?} in_progress={} timer_running={} timer_deadline={} started_this_frame={} command_offset={} remaining_bytes={} waypoint={:?} owner_work_len={} self_stimuli_len={}"
}

trace_event! {
    AidecisionGotoEnter {
        frame: Display,
        owner: Display,
        co: Debug,
        destination_x_bits: LowerHex,
        destination_y_bits: LowerHex,
        destination_sector: Debug,
        destination_level: Display,
        position_x_bits: LowerHex,
        position_y_bits: LowerHex,
        position_sector: Debug,
        position_level: Display,
        couldnt_before: Display,
        already_before: Display,
        owner_work_before: Debug;
        flags: Debug
    } => "AIDECISION frame={} owner={} co={:?} stage=goto_enter destination=({:08x},{:08x},sector={:?},level={}) flags={flags:?} position=({:08x},{:08x},sector={:?},level={}) couldnt_before={} already_before={} owner_work_before={:?}"
}

trace_event! {
    AidecisionGotoResultAlreadyOnPoint {
        frame: Display,
        owner: Display,
        couldnt: Display,
        already: Display,
        owner_work: Debug;
    } => "AIDECISION frame={} owner={} stage=goto_result result=already_on_point couldnt={} already={} owner_work={:?}"
}

trace_event! {
    AidecisionGotoResultAlreadyNear {
        frame: Display,
        owner: Display,
        tolerance_bits: LowerHex,
        couldnt: Display,
        already: Display,
        owner_work: Debug;
    } => "AIDECISION frame={} owner={} stage=goto_result result=already_near tolerance_bits={:08x} couldnt={} already={} owner_work={:?}"
}

trace_event! {
    AidecisionGotoResultRejectNonpositive {
        frame: Display,
        owner: Display;
    } => "AIDECISION frame={} owner={} stage=goto_result result=reject_nonpositive couldnt=true"
}

trace_event! {
    AidecisionGotoResultRejectNullSector {
        frame: Display,
        owner: Display;
    } => "AIDECISION frame={} owner={} stage=goto_result result=reject_null_sector couldnt=true"
}

trace_event! {
    AidecisionGotoResultQueued {
        frame: Display,
        owner: Display,
        couldnt: Display,
        already: Display,
        pending_orders: Display,
        owner_work: Debug;
    } => "AIDECISION frame={} owner={} stage=goto_result result=queued couldnt={} already={} pending_orders={} owner_work={:?}"
}

trace_event! {
    AidecisionGoNearEnter {
        frame: Display,
        owner: Display,
        co: Debug,
        destination_x_bits: LowerHex,
        destination_y_bits: LowerHex,
        destination_sector: Debug,
        destination_level: Display,
        distance: Display,
        recursion_depth: Display,
        couldnt_before: Display,
        already_before: Display;
        flags: Debug
    } => "AIDECISION frame={} owner={} co={:?} stage=go_near_enter destination=({:08x},{:08x},sector={:?},level={}) distance={} flags={flags:?} recursion_depth={} couldnt_before={} already_before={}"
}

trace_event! {
    AidecisionGoNearResultAlreadyOnPoint {
        frame: Display,
        owner: Display,
        effective_distance: Display,
        couldnt: Display,
        already: Display;
    } => "AIDECISION frame={} owner={} stage=go_near_result result=already_on_point effective_distance={} couldnt={} already={}"
}

trace_event! {
    AidecisionGoNearResultAlreadyNear {
        frame: Display,
        owner: Display,
        effective_distance: Display,
        couldnt: Display,
        already: Display;
    } => "AIDECISION frame={} owner={} stage=go_near_result result=already_near effective_distance={} couldnt={} already={}"
}

trace_event! {
    AidecisionGoNearResultQueued {
        frame: Display,
        owner: Display,
        effective_distance: Display,
        couldnt: Display,
        already: Display,
        pending_orders: Display,
        owner_work: Debug;
    } => "AIDECISION frame={} owner={} stage=go_near_result result=queued effective_distance={} couldnt={} already={} pending_orders={} owner_work={:?}"
}

trace_event! {
    PointTo {
        frame: Display;
        owner: Debug,
        pos: Debug,
        target: Debug,
        me: Debug,
        direction: Display
    } => "[POINT_TO frame={} owner={owner:?} pos={pos:?} target={target:?} me={me:?} dir={direction}]"
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

trace_event! {
    Aihide {
        frame: Display,
        co: Debug,
        substate: Debug;
    } => "[AIHIDE f={} co={:?} substate={:?}]"
}

trace_event! {
    Aipanic {
        frame: Display,
        me: Display,
        co: Debug,
        stimulus: Debug,
        runs: Display,
        directed: Display,
        first_try: Display;
    } => "[AIPANIC f={} me={} co={:?} stim={:?} runs={} directed={} first_try={}]"
}

trace_event! {
    AipanicGeometry {
        frame: Display,
        me: Display,
        sector: Display,
        distance_bits: LowerHex,
        origin_x_bits: LowerHex,
        origin_y_bits: LowerHex,
        destination_x_bits: LowerHex,
        destination_y_bits: LowerHex,
        layer: Display,
        move_box: Debug,
        position_authorized: Display,
        reachable_thick: Display,
        straight_authorized: Display;
    } => "[AIPANIC-GEOMETRY f={} me={} sector={} distance_bits={:08x} origin_bits={:08x},{:08x} destination_bits={:08x},{:08x} layer={} move_box={:?} position_authorized={} reachable_thick={} straight_authorized={}]"
}

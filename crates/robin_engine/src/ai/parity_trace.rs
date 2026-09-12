//! Parity stderr payloads, separated from AI decisions.
//!
//! Callers own gates and argument evaluation; these cold emitters only format
//! borrowed values. Keep payload text and field order stable for replay tooling.
//! Formatting traits deliberately avoid coupling diagnostic schemas to AI state.

/// Declare a cold event formatter without repeating its argument list in the
/// stderr call. Captured names follow positional arguments, as in format_args!.
macro_rules! trace_event {
    (
        $name:ident(
            $($argument:ident: $format_trait:ident),*;
            $($capture:ident: $capture_trait:ident),*
        ) => $format:literal
    ) => {
        #[cold]
        #[inline(never)]
        pub(super) fn $name(
            $($argument: &dyn std::fmt::$format_trait,)*
            $($capture: &dyn std::fmt::$capture_trait,)*
        ) {
            eprintln!(
                $format,
                $($argument,)*
                $($capture = $capture,)*
            );
        }
    };
}

pub(crate) use trace_event;

trace_event! {
    forecast(
        out: Display,
        value: Display,
        sector: Display,
        gates: Display;
        input: Debug,
        layer: Display,
        direction: Display,
        entry_gate: Debug
    ) => "FORECAST input={input:?} out=({}, {}, sector={}, layer={layer}) dir={direction} gates={} entry={entry_gate:?}"
}

trace_event! {
    bored_boundary_get_bored_time(
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
    ) => "BORED_BOUNDARY frame={} owner={} phase=get_bored_time state={:?} substate={:?} rank={:?} pride={} min={} delta={} timer_running={} timer_deadline={} self_stimuli={} owner_work={} orders={}"
}

trace_event! {
    macrolife(
        frame: Display,
        owner: Debug,
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
        self_stimuli_len: Display,
        finish_after_stimuli: Display;
        phase: Display,
        reason: Debug
    ) => "MACROLIFE frame={} owner={:?} me={} phase={phase} reason={reason:?} state={:?} substate={:?} in_progress={} timer_running={} timer_deadline={} started_this_frame={} command_offset={} remaining_bytes={} waypoint={:?} owner_work_len={} self_stimuli_len={} finish_after_stimuli={}"
}

trace_event! {
    considerreport_merge_start(
        owner: Display,
        incoming: Debug,
        known_before: Debug;
        frame: Display,
        flags: Display
    ) => "CONSIDERREPORT {{\"stage\":\"merge_start\",\"frame\":{frame},\"owner\":{},\"flags\":{flags},\"incoming\":{:?},\"known_before\":{:?}}}"
}

trace_event! {
    considerreport_body(
        owner: Display,
        resolved_kind: Debug,
        resolved_index: Display;
        frame: Display,
        body: Display
    ) => "CONSIDERREPORT {{\"stage\":\"body\",\"frame\":{frame},\"owner\":{},\"body\":{body},\"known\":false,\"resolved_kind\":{:?},\"resolved_index\":{},\"queued\":true}}"
}

trace_event! {
    considerreport_body_2(
        owner: Display;
        frame: Display,
        body: Display
    ) => "CONSIDERREPORT {{\"stage\":\"body\",\"frame\":{frame},\"owner\":{},\"body\":{body},\"known\":true,\"resolved_kind\":null,\"resolved_index\":null,\"queued\":false}}"
}

trace_event! {
    considerreport_merge_end(
        owner: Display,
        known_after: Debug,
        queued_mutations: Debug;
        frame: Display
    ) => "CONSIDERREPORT {{\"stage\":\"merge_end\",\"frame\":{frame},\"owner\":{},\"known_after\":{:?},\"queued_mutations\":{:?}}}"
}

trace_event! {
    aidecision_goto_enter(
        frame: Display,
        owner: Display,
        co: Debug,
        destination: LowerHex,
        value: LowerHex,
        sector: Debug,
        level: Display,
        position: LowerHex,
        value_2: LowerHex,
        sector_2: Debug,
        level_2: Display,
        couldnt_before: Display,
        already_before: Display,
        owner_work_before: Debug;
        flags: Debug
    ) => "AIDECISION frame={} owner={} co={:?} stage=goto_enter destination=({:08x},{:08x},sector={:?},level={}) flags={flags:?} position=({:08x},{:08x},sector={:?},level={}) couldnt_before={} already_before={} owner_work_before={:?}"
}

trace_event! {
    aidecision_goto_result_already_on_point(
        frame: Display,
        owner: Display,
        couldnt: Display,
        already: Display,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} stage=goto_result result=already_on_point couldnt={} already={} owner_work={:?}"
}

trace_event! {
    aidecision_goto_result_already_near(
        frame: Display,
        owner: Display,
        tolerance_bits: LowerHex,
        couldnt: Display,
        already: Display,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} stage=goto_result result=already_near tolerance_bits={:08x} couldnt={} already={} owner_work={:?}"
}

trace_event! {
    aidecision_goto_result_reject_nonpositive(
        frame: Display,
        owner: Display;
    ) => "AIDECISION frame={} owner={} stage=goto_result result=reject_nonpositive couldnt=true"
}

trace_event! {
    aidecision_goto_result_reject_null_sector(
        frame: Display,
        owner: Display;
    ) => "AIDECISION frame={} owner={} stage=goto_result result=reject_null_sector couldnt=true"
}

trace_event! {
    aidecision_goto_result_queued(
        frame: Display,
        owner: Display,
        couldnt: Display,
        already: Display,
        pending_orders: Display,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} stage=goto_result result=queued couldnt={} already={} pending_orders={} owner_work={:?}"
}

trace_event! {
    aidecision_go_near_enter(
        frame: Display,
        owner: Display,
        co: Debug,
        destination: LowerHex,
        value: LowerHex,
        sector: Debug,
        level: Display,
        distance: Display,
        recursion_depth: Display,
        couldnt_before: Display,
        already_before: Display;
        flags: Debug
    ) => "AIDECISION frame={} owner={} co={:?} stage=go_near_enter destination=({:08x},{:08x},sector={:?},level={}) distance={} flags={flags:?} recursion_depth={} couldnt_before={} already_before={}"
}

trace_event! {
    aidecision_go_near_result_already_on_point(
        frame: Display,
        owner: Display,
        effective_distance: Display,
        couldnt: Display,
        already: Display;
    ) => "AIDECISION frame={} owner={} stage=go_near_result result=already_on_point effective_distance={} couldnt={} already={}"
}

trace_event! {
    aidecision_go_near_result_already_near(
        frame: Display,
        owner: Display,
        effective_distance: Display,
        couldnt: Display,
        already: Display;
    ) => "AIDECISION frame={} owner={} stage=go_near_result result=already_near effective_distance={} couldnt={} already={}"
}

trace_event! {
    aidecision_go_near_result_queued(
        frame: Display,
        owner: Display,
        effective_distance: Display,
        couldnt: Display,
        already: Display,
        pending_orders: Display,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} stage=go_near_result result=queued effective_distance={} couldnt={} already={} pending_orders={} owner_work={:?}"
}

trace_event! {
    point_to(
        frame: Display;
        owner: Debug,
        pos: Debug,
        target: Debug,
        me: Debug,
        direction: Display
    ) => "[POINT_TO frame={} owner={owner:?} pos={pos:?} target={target:?} me={me:?} dir={direction}]"
}

trace_event! {
    willstop(
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
    ) => "WILLSTOP frame={} owner={:?} caller={caller:?} path={path:?} waypoint={waypoint:?} forecast_before={forecasted_before} value_before={value_before} forecast_after={} value_after={} result={result}"
}

trace_event! {
    aihide(
        f: Display,
        co: Debug,
        substate: Debug;
    ) => "[AIHIDE f={} co={:?} substate={:?}]"
}

trace_event! {
    aipanic(
        f: Display,
        me: Display,
        co: Debug,
        stim: Debug,
        runs: Display,
        directed: Display,
        first_try: Display;
    ) => "[AIPANIC f={} me={} co={:?} stim={:?} runs={} directed={} first_try={}]"
}

trace_event! {
    aipanic_geometry(
        f: Display,
        me: Display,
        sector: Display,
        distance_bits: LowerHex,
        origin_bits: LowerHex,
        value: LowerHex,
        destination_bits: LowerHex,
        value_2: LowerHex,
        layer: Display,
        move_box: Debug,
        position_authorized: Display,
        reachable_thick: Display,
        straight_authorized: Display;
    ) => "[AIPANIC-GEOMETRY f={} me={} sector={} distance_bits={:08x} origin_bits={:08x},{:08x} destination_bits={:08x},{:08x} layer={} move_box={:?} position_authorized={} reachable_thick={} straight_authorized={}]"
}

//! Canonical retained-value accounting and admission shared with live execution.
use super::*;

pub(super) fn accounted_string_bytes(value: &str) -> u64 {
    8_u64.saturating_add(value.len() as u64)
}

fn validate_tape_string(label: &str, value: &str) -> Result<(), String> {
    if value.len() > SPELLFORGE_TAPE_STRING_BYTE_LIMIT {
        return Err(format!(
            "Spellforge {label} is {} bytes; limit is {SPELLFORGE_TAPE_STRING_BYTE_LIMIT}",
            value.len()
        ));
    }
    Ok(())
}

/// Shared admission rule for live requests and retained journal records.
pub fn validate_arguments(label: &str, arguments: &[i32]) -> Result<(), String> {
    if arguments.len() > SPELLFORGE_ARGUMENT_WORD_LIMIT {
        return Err(format!(
            "Spellforge {label} contains {} argument words; limit is {SPELLFORGE_ARGUMENT_WORD_LIMIT}",
            arguments.len()
        ));
    }
    Ok(())
}

/// Validate the same invocation limits before execution and during restore.
pub fn validate_invocation(invocation: &SpellforgeInvocation) -> Result<(), String> {
    validate_tape_string("event name", &invocation.event)?;
    validate_arguments("invocation", &invocation.args)?;
    let class = match &invocation.target {
        SpellforgeTarget::Global => None,
        SpellforgeTarget::Actor { class, .. }
        | SpellforgeTarget::Target { class, .. }
        | SpellforgeTarget::Scroll { class, .. }
        | SpellforgeTarget::Zone { class, .. }
        | SpellforgeTarget::Waypoint { class, .. } => Some(class),
    };
    if let Some(class) = class {
        validate_tape_string("target class", class)?;
    }
    Ok(())
}

pub(super) fn measure_event(event: &SpellforgeEventRecord) -> Result<SpellforgeTapeUsage, String> {
    validate_invocation(&event.invocation)?;
    if event.native_calls.len() > SPELLFORGE_EVENT_NATIVE_CALL_LIMIT {
        return Err(format!(
            "Spellforge event contains {} direct native calls; limit is {SPELLFORGE_EVENT_NATIVE_CALL_LIMIT}",
            event.native_calls.len()
        ));
    }
    let mut transcript_entries = 0usize;
    for native in &event.native_calls {
        validate_arguments("native request", &native.arguments)?;
        transcript_entries = transcript_entries.saturating_add(native.nested_transcript.len());
        if transcript_entries > SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT {
            return Err(format!(
                "Spellforge event contains {transcript_entries} nested transcript entries; limit is {SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT}"
            ));
        }
        for entry in &native.nested_transcript {
            match entry {
                SpellforgeNestedTapeEntry::Begin(invocation) => {
                    validate_invocation(invocation)?;
                }
                SpellforgeNestedTapeEntry::NativeRequest { arguments, .. } => {
                    validate_arguments("nested native request", arguments)?;
                }
                SpellforgeNestedTapeEntry::NativeReturn { .. }
                | SpellforgeNestedTapeEntry::Complete { .. } => {}
            }
        }
    }
    let usage = measure_event_unchecked(event);
    usage.validate_total()?;
    Ok(usage)
}

pub(super) fn measure_event_unchecked(event: &SpellforgeEventRecord) -> SpellforgeTapeUsage {
    let mut usage = SpellforgeTapeUsage {
        events: 1,
        retained_bytes: 4,
        ..SpellforgeTapeUsage::default()
    };
    measure_invocation_unchecked(&event.invocation, &mut usage.retained_bytes);
    usage.retained_bytes = usage.retained_bytes.saturating_add(8);
    for native in &event.native_calls {
        usage.native_calls = usage.native_calls.saturating_add(1);
        usage.retained_bytes = usage
            .retained_bytes
            .saturating_add(4 + 8 + (native.arguments.len() as u64).saturating_mul(4) + 4 + 8);
        for entry in &native.nested_transcript {
            usage.transcript_entries = usage.transcript_entries.saturating_add(1);
            usage.retained_bytes = usage.retained_bytes.saturating_add(1);
            match entry {
                SpellforgeNestedTapeEntry::Begin(invocation) => {
                    measure_invocation_unchecked(invocation, &mut usage.retained_bytes);
                }
                SpellforgeNestedTapeEntry::NativeRequest { arguments, .. } => {
                    usage.native_calls = usage.native_calls.saturating_add(1);
                    usage.retained_bytes = usage
                        .retained_bytes
                        .saturating_add(4 + 8 + (arguments.len() as u64).saturating_mul(4));
                }
                SpellforgeNestedTapeEntry::NativeReturn { .. }
                | SpellforgeNestedTapeEntry::Complete { .. } => {
                    usage.retained_bytes = usage.retained_bytes.saturating_add(4);
                }
            }
        }
    }
    usage
}

fn measure_invocation_unchecked(invocation: &SpellforgeInvocation, total: &mut u64) {
    *total = total.saturating_add(1);
    match &invocation.target {
        SpellforgeTarget::Global => {}
        SpellforgeTarget::Actor { class, .. }
        | SpellforgeTarget::Target { class, .. }
        | SpellforgeTarget::Scroll { class, .. } => {
            *total = total
                .saturating_add(accounted_string_bytes(class))
                .saturating_add(4);
        }
        SpellforgeTarget::Zone { class, .. } => {
            *total = total
                .saturating_add(accounted_string_bytes(class))
                .saturating_add(4);
        }
        SpellforgeTarget::Waypoint { class, .. } => {
            *total = total
                .saturating_add(accounted_string_bytes(class))
                .saturating_add(3);
        }
    }
    *total = total
        .saturating_add(accounted_string_bytes(&invocation.event))
        .saturating_add(8)
        .saturating_add((invocation.args.len() as u64).saturating_mul(4))
        .saturating_add(8);
}

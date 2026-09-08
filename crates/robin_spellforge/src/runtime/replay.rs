//! Reconstruct a disposable heap from the authoritative native journal.
use super::*;

pub(super) fn replay_event(
    vm: &mut RuntimeVm,
    record: &SpellforgeEventRecord,
) -> SpellforgeResult<i32> {
    let activation = allocate_replay_activation(vm)?;
    let mut step = begin_event(vm, activation, &record.invocation)?;
    for (call_index, expected) in record.native_calls.iter().enumerate() {
        let SpellforgeStep::Native {
            native_index,
            arguments,
            ..
        } = step
        else {
            return Err(format!(
                "event completed after {call_index} of {} recorded native calls",
                record.native_calls.len()
            )
            .into());
        };
        if native_index != expected.native_index || arguments != expected.arguments {
            return Err(format!(
                "native call {call_index} expected {}({:?}), rebuilt {native_index}({arguments:?})",
                expected.native_index, expected.arguments
            )
            .into());
        }
        replay_nested_transcript(vm, &expected.nested_transcript)
            .map_err(|error| format!("nested transcript of native call {call_index}: {error}"))?;
        step = resume_event(vm, activation, expected.return_word)?;
    }
    match step {
        SpellforgeStep::Complete { result, .. } => Ok(result),
        SpellforgeStep::Native {
            native_index,
            arguments,
            ..
        } => Err(format!(
            "event made unrecorded native call {native_index}({arguments:?}) after transcript end"
        )
        .into()),
    }
}

fn allocate_replay_activation(vm: &mut RuntimeVm) -> SpellforgeResult<u64> {
    let activation = vm.next_replay_activation;
    vm.next_replay_activation = activation
        .checked_add(1)
        .filter(|next| *next <= (1 << 53))
        .ok_or_else(|| "Spellforge replay activation id exhausted exact Lua integers".to_owned())?;
    Ok(activation)
}

struct NestedReplayFrame {
    activation: u64,
    step: SpellforgeStep,
    native_request_open: bool,
}

fn replay_nested_transcript(
    vm: &mut RuntimeVm,
    transcript: &[SpellforgeNestedTapeEntry],
) -> SpellforgeResult<()> {
    let mut stack = Vec::<NestedReplayFrame>::new();
    for (entry_index, entry) in transcript.iter().enumerate() {
        match entry {
            SpellforgeNestedTapeEntry::Begin(invocation) => {
                if stack
                    .last()
                    .is_some_and(|parent| !parent.native_request_open)
                {
                    return Err(format!(
                        "entry {entry_index} begins a callback while its parent is not suspended at a native"
                    )
                    .into());
                }
                let activation = allocate_replay_activation(vm)?;
                let step = begin_event(vm, activation, invocation)?;
                stack.push(NestedReplayFrame {
                    activation,
                    step,
                    native_request_open: false,
                });
            }
            SpellforgeNestedTapeEntry::NativeRequest {
                native_index,
                arguments,
            } => {
                let frame = stack.last_mut().ok_or_else(|| {
                    format!("entry {entry_index} requests a native outside a nested callback")
                })?;
                if frame.native_request_open {
                    return Err(format!(
                        "entry {entry_index} requests a second native before the first returned"
                    )
                    .into());
                }
                let SpellforgeStep::Native {
                    native_index: rebuilt_index,
                    arguments: rebuilt_arguments,
                    ..
                } = &frame.step
                else {
                    return Err(format!(
                        "entry {entry_index} records native {native_index}({arguments:?}), but the callback completed"
                    )
                    .into());
                };
                if rebuilt_index != native_index || rebuilt_arguments != arguments {
                    return Err(format!(
                        "entry {entry_index} expected native {native_index}({arguments:?}), rebuilt {rebuilt_index}({rebuilt_arguments:?})"
                    )
                    .into());
                }
                frame.native_request_open = true;
            }
            SpellforgeNestedTapeEntry::NativeReturn { return_word } => {
                let frame = stack.last_mut().ok_or_else(|| {
                    format!("entry {entry_index} returns from a native outside a nested callback")
                })?;
                if !frame.native_request_open {
                    return Err(format!(
                        "entry {entry_index} returns from a native with no matching request"
                    )
                    .into());
                }
                frame.step = resume_event(vm, frame.activation, *return_word)?;
                frame.native_request_open = false;
            }
            SpellforgeNestedTapeEntry::Complete { result } => {
                let frame = stack.pop().ok_or_else(|| {
                    format!("entry {entry_index} completes a callback that never began")
                })?;
                if frame.native_request_open {
                    return Err(format!(
                        "entry {entry_index} completes while a native request is still open"
                    )
                    .into());
                }
                match frame.step {
                    SpellforgeStep::Complete {
                        result: rebuilt, ..
                    } if rebuilt == *result => {}
                    SpellforgeStep::Complete {
                        result: rebuilt, ..
                    } => {
                        return Err(format!(
                            "entry {entry_index} expected result {result}, rebuilt {rebuilt}"
                        )
                        .into());
                    }
                    SpellforgeStep::Native {
                        native_index,
                        arguments,
                        ..
                    } => {
                        return Err(format!(
                            "entry {entry_index} completes before unrecorded native {native_index}({arguments:?})"
                        )
                        .into());
                    }
                }
            }
        }
    }
    if !stack.is_empty() {
        return Err(format!(
            "nested transcript ended with {} unterminated callback(s)",
            stack.len()
        )
        .into());
    }
    Ok(())
}

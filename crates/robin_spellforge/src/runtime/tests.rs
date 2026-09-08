use super::*;
use robin_engine::spellforge::SpellforgeScriptMode;

fn package(source: &str) -> SpellforgePackage {
    let mut package = SpellforgePackage {
        contract_version: SPELLFORGE_CONTRACT_VERSION,
        vm_abi: spellforge_vm_abi().to_owned(),
        script_mode: SpellforgeScriptMode::Replace,
        entrypoint: "mission.lua".to_owned(),
        files: BTreeMap::from([("mission.lua".to_owned(), source.as_bytes().to_vec())]),
        sha256: [0; 32],
    };
    package.sha256 = compute_package_sha256(&package);
    package
}

fn invoke(
    runtime: &SpellforgeRuntime51,
    tape: &mut SpellforgeTape,
    event: &str,
    args: Vec<i32>,
) -> i32 {
    let invocation = SpellforgeInvocation {
        target: SpellforgeTarget::Global,
        event: event.to_owned(),
        args,
        script_this: 0,
        current_scroll: 0,
    };
    let SpellforgeStep::Complete { activation, result } = runtime.begin(invocation, tape).unwrap()
    else {
        panic!("{event} unexpectedly yielded")
    };
    runtime.commit(activation, result, tape).unwrap();
    result
}

#[test]
fn lifecycle_heap_and_rollback_share_one_contract() {
    let runtime = SpellforgeRuntime51::new(package(
        "value=0; function Initialize(x) value=x end; function Timer(x) value=value+x end; function CheckVictoryCondition() return value end; function Finalize() return value+1 end",
    )).unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    assert_eq!(invoke(&runtime, &mut tape, "Initialize", vec![7]), 0);
    assert_eq!(invoke(&runtime, &mut tape, "Timer", vec![3]), 0);
    assert_eq!(
        invoke(&runtime, &mut tape, "CheckVictoryCondition", vec![]),
        10
    );
    let rebuilt = SpellforgeRuntime51::new(runtime.package().clone()).unwrap();
    assert_eq!(invoke(&rebuilt, &mut tape, "Finalize", vec![0]), 11);
}

#[test]
fn native_journal_and_per_entity_dispatch_rebuild() {
    let runtime = SpellforgeRuntime51::new(package(
        "n=0; Guard={ProcessMessage=function(code) n=n+1; InitGlobal(3,n+code); return n end}",
    ))
    .unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    let invocation = SpellforgeInvocation {
        target: SpellforgeTarget::Actor {
            class: "Guard".into(),
            handle: 4,
        },
        event: "ProcessMessage".into(),
        args: vec![9],
        script_this: 4,
        current_scroll: 0,
    };
    assert!(runtime.has_handler(&invocation, &tape).unwrap());
    let SpellforgeStep::Native {
        activation,
        native_index: 0,
        arguments,
    } = runtime.begin(invocation.clone(), &tape).unwrap()
    else {
        panic!()
    };
    assert_eq!(arguments, [3, 10]);
    let SpellforgeStep::Complete { result: 1, .. } = runtime.resume(activation, 0, &tape).unwrap()
    else {
        panic!()
    };
    runtime.commit(activation, 1, &mut tape).unwrap();
    let rebuilt = SpellforgeRuntime51::new(runtime.package().clone()).unwrap();
    let SpellforgeStep::Native { arguments, .. } = rebuilt.begin(invocation, &tape).unwrap() else {
        panic!()
    };
    assert_eq!(arguments, [3, 11]);
}

#[test]
fn nested_callbacks_are_journaled_at_the_suspending_native() {
    let runtime = SpellforgeRuntime51::new(package(
        "value=0; function Outer() value=value+1; InitGlobal(1,value); value=value+10; return value end; function Inner() value=value+2; InitGlobal(2,value); return value end; function Read() return value end",
    ))
    .unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    let outer = SpellforgeInvocation {
        target: SpellforgeTarget::Global,
        event: "Outer".into(),
        args: vec![],
        script_this: 0,
        current_scroll: 0,
    };
    let SpellforgeStep::Native {
        activation: outer_activation,
        arguments,
        ..
    } = runtime.begin(outer, &tape).unwrap()
    else {
        panic!("outer callback did not suspend at native")
    };
    assert_eq!(arguments, [1, 1]);

    let inner = SpellforgeInvocation {
        target: SpellforgeTarget::Global,
        event: "Inner".into(),
        args: vec![],
        script_this: 0,
        current_scroll: 0,
    };
    let SpellforgeStep::Native {
        activation: inner_activation,
        arguments,
        ..
    } = runtime.begin(inner, &tape).unwrap()
    else {
        panic!("inner callback did not suspend at native")
    };
    assert_eq!(arguments, [2, 3]);
    let SpellforgeStep::Complete { result: 3, .. } =
        runtime.resume(inner_activation, 0, &tape).unwrap()
    else {
        panic!("inner callback did not complete")
    };
    runtime.commit(inner_activation, 3, &mut tape).unwrap();
    assert!(
        tape.events.is_empty(),
        "nested callback must remain inside its parent native journal"
    );
    let SpellforgeStep::Complete { result: 13, .. } =
        runtime.resume(outer_activation, 0, &tape).unwrap()
    else {
        panic!("outer callback did not complete")
    };
    runtime.commit(outer_activation, 13, &mut tape).unwrap();
    assert_eq!(tape.events.len(), 1);
    assert_eq!(
        tape.events[0].native_calls[0].nested_transcript,
        [
            SpellforgeNestedTapeEntry::Begin(SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: "Inner".into(),
                args: vec![],
                script_this: 0,
                current_scroll: 0,
            }),
            SpellforgeNestedTapeEntry::NativeRequest {
                native_index: 0,
                arguments: vec![2, 3],
            },
            SpellforgeNestedTapeEntry::NativeReturn { return_word: 0 },
            SpellforgeNestedTapeEntry::Complete { result: 3 },
        ]
    );

    let rebuilt = SpellforgeRuntime51::new(runtime.package().clone()).unwrap();
    assert_eq!(invoke(&rebuilt, &mut tape, "Read", vec![]), 13);
}

#[test]
fn sequence_callbacks_random_results_and_names_rebuild_exactly() {
    let runtime = SpellforgeRuntime51::new(package(
        "value=0; function Start() SequenceCall(function() value=value+math.random(2,8); return value end) end; function ReadActor() return GetActor('Robin') end; function Read() return value end",
    ))
    .unwrap();
    runtime.set_name_bindings(ScriptNameBindings {
        actors: BTreeMap::from([("Robin".to_owned(), 42)]),
        ..ScriptNameBindings::default()
    });
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    assert_eq!(invoke(&runtime, &mut tape, "ReadActor", vec![]), 42);

    let start = SpellforgeInvocation {
        target: SpellforgeTarget::Global,
        event: "Start".into(),
        args: vec![],
        script_this: 0,
        current_scroll: 0,
    };
    let SpellforgeStep::Native {
        activation: start_activation,
        arguments,
        ..
    } = runtime.begin(start, &tape).unwrap()
    else {
        panic!("SequenceCall did not yield SequenceSendMessage")
    };
    assert_eq!(arguments[0], 0);
    let sequence_message = arguments[1];
    let SpellforgeStep::Complete { result: 0, .. } =
        runtime.resume(start_activation, 0, &tape).unwrap()
    else {
        panic!("Start did not complete")
    };
    runtime.commit(start_activation, 0, &mut tape).unwrap();

    let sequence = SpellforgeInvocation {
        target: SpellforgeTarget::Global,
        event: "ProcessMessage".into(),
        args: vec![sequence_message],
        script_this: 0,
        current_scroll: 0,
    };
    let SpellforgeStep::Native {
        activation: sequence_activation,
        native_index,
        arguments,
    } = runtime.begin(sequence, &tape).unwrap()
    else {
        panic!("sequence callback did not request deterministic random")
    };
    assert_eq!(native_index, PSEUDO_NATIVE_RANDOM);
    assert_eq!(arguments, [2, 8]);
    let SpellforgeStep::Complete { result: 7, .. } =
        runtime.resume(sequence_activation, 7, &tape).unwrap()
    else {
        panic!("sequence callback did not complete")
    };
    runtime.commit(sequence_activation, 7, &mut tape).unwrap();

    let rebuilt = SpellforgeRuntime51::new(runtime.package().clone()).unwrap();
    rebuilt.set_name_bindings(ScriptNameBindings {
        actors: BTreeMap::from([("Robin".to_owned(), 42)]),
        ..ScriptNameBindings::default()
    });
    assert_eq!(invoke(&rebuilt, &mut tape, "Read", vec![]), 7);
    assert_eq!(invoke(&rebuilt, &mut tape, "ReadActor", vec![]), 42);
}

#[test]
fn same_length_tape_divergence_is_fatal() {
    let runtime = SpellforgeRuntime51::new(package(
        "value=0; function Timer(x) value=value+x; return value end",
    ))
    .unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    assert_eq!(invoke(&runtime, &mut tape, "Timer", vec![3]), 3);

    let rebuilt = SpellforgeRuntime51::new(runtime.package().clone()).unwrap();
    let mut divergent = tape.clone();
    let mut records = divergent.events.iter().cloned().collect::<Vec<_>>();
    std::sync::Arc::make_mut(&mut records[0]).result = 4;
    divergent.events = SpellforgeJournal::from_records(records).unwrap();
    let error = rebuilt
        .has_handler(
            &SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: "Timer".into(),
                args: vec![1],
                script_this: 0,
                current_scroll: 0,
            },
            &divergent,
        )
        .expect_err("same-length divergent tape must not be accepted");
    assert_eq!(error.kind, SpellforgeGuestErrorKind::Divergence);
    assert!(
        error.message.contains("expected result 4, rebuilt 3"),
        "{error}"
    );
}

#[test]
fn package_require_random_bounds_and_resource_limits_are_enforced() {
    let mut with_module = package("require('common'); function Read() return module_value end");
    with_module
        .files
        .insert("lib/common.lua".into(), b"module_value=17".to_vec());
    with_module.sha256 = compute_package_sha256(&with_module);
    let runtime = SpellforgeRuntime51::new(with_module).unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    assert_eq!(invoke(&runtime, &mut tape, "Read", vec![]), 17);

    let runaway =
        SpellforgeRuntime51::new(package("function Timer() while true do end end")).unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runaway.package().clone()).unwrap();
    let error = runaway
        .begin(
            SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: "Timer".into(),
                args: vec![],
                script_this: 0,
                current_scroll: 0,
            },
            &tape,
        )
        .unwrap_err();
    assert_eq!(error.kind, SpellforgeGuestErrorKind::Budget);
    assert!(error.message.contains(BUDGET_ERROR), "{error}");

    let mut tampered = package("function Timer() end");
    tampered.files.get_mut("mission.lua").unwrap().push(b' ');
    let Err(error) = SpellforgeRuntime51::new(tampered) else {
        panic!("tampered package unexpectedly loaded")
    };
    assert_eq!(error.kind, SpellforgeGuestErrorKind::Compatibility);
    assert!(error.message.contains("hash mismatch"));
}

#[test]
fn exact_root_module_wins_over_library_leaf_alias() {
    let mut with_modules = package(
        "require('lib/enums'); require('enums'); function Read() return animation_alerted end",
    );
    with_modules
        .files
        .insert("lib/enums.lua".into(), b"animation_alerted=141".to_vec());
    with_modules
        .files
        .insert("enums.lua".into(), b"animation_alerted=140".to_vec());
    with_modules.sha256 = compute_package_sha256(&with_modules);

    let runtime = SpellforgeRuntime51::new(with_modules).unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    assert_eq!(invoke(&runtime, &mut tape, "Read", vec![]), 140);
}

#[test]
fn ambiguous_noncanonical_leaf_aliases_remain_rejected() {
    let mut ambiguous = package("function Read() return 1 end");
    ambiguous
        .files
        .insert("lib/a/value.lua".into(), b"return 1".to_vec());
    ambiguous
        .files
        .insert("lib/b/value.lua".into(), b"return 2".to_vec());
    ambiguous.sha256 = compute_package_sha256(&ambiguous);

    let error = SpellforgeRuntime51::new(ambiguous)
        .err()
        .expect("ambiguous leaf aliases must remain invalid");
    assert_eq!(error.kind, SpellforgeGuestErrorKind::Runtime);
    assert!(
        error.message.contains("alias `value` is ambiguous"),
        "{error}"
    );
}

#[test]
fn published_numeric_boolean_words_accept_only_zero_or_one() {
    let runtime = SpellforgeRuntime51::new(package(
        "function One() SetAlwaysAttentive(7,1) end; function Zero() SetAlwaysAttentive(7,0) end; function Two() SetAlwaysAttentive(7,2) end",
    ))
    .unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();

    for (event, expected) in [("One", 1), ("Zero", 0)] {
        let invocation = SpellforgeInvocation {
            target: SpellforgeTarget::Global,
            event: event.to_owned(),
            args: vec![],
            script_this: 0,
            current_scroll: 0,
        };
        let SpellforgeStep::Native {
            activation,
            native_index,
            arguments,
        } = runtime.begin(invocation, &tape).unwrap()
        else {
            panic!("{event} did not call SetAlwaysAttentive")
        };
        assert_eq!(native_index, NativeFn::SetAlwaysAttentive as u32);
        assert_eq!(arguments, [7, expected]);
        let SpellforgeStep::Complete { result, .. } = runtime.resume(activation, 0, &tape).unwrap()
        else {
            panic!("{event} did not complete")
        };
        runtime.commit(activation, result, &mut tape).unwrap();
    }

    let error = runtime
        .begin(
            SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: "Two".to_owned(),
                args: vec![],
                script_this: 0,
                current_scroll: 0,
            },
            &tape,
        )
        .expect_err("numeric 2 must not coerce to a boolean");
    assert_eq!(error.kind, SpellforgeGuestErrorKind::Runtime);
    assert!(error.message.contains("boolean or numeric 0/1"), "{error}");
}

#[test]
fn privileged_vm_capabilities_are_hidden_before_package_bootstrap() {
    let runtime = SpellforgeRuntime51::new(package(
        r#"
        if coroutine ~= nil or debug ~= nil or io ~= nil or os ~= nil or
           package ~= nil or dofile ~= nil or load ~= nil or loadfile ~= nil or
           loadstring ~= nil or getfenv ~= nil or setfenv ~= nil or
           collectgarbage ~= nil then
            error("privileged capability leaked into package bootstrap")
        end
        function Timer() return 9 end
        "#,
    ))
    .expect("sandboxed package bootstrap");
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    assert_eq!(invoke(&runtime, &mut tape, "Timer", vec![]), 9);
}

#[test]
fn handler_lookup_never_executes_package_metamethods_outside_the_tape() {
    let runtime = SpellforgeRuntime51::new(package(
        "Trap=setmetatable({}, {__index=function() while true do end end})",
    ))
    .unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    let invocation = SpellforgeInvocation {
        target: SpellforgeTarget::Actor {
            class: "Trap".into(),
            handle: 3,
        },
        event: "Timer".into(),
        args: vec![],
        script_this: 3,
        current_scroll: 0,
    };
    assert!(!runtime.has_handler(&invocation, &tape).unwrap());
}
fn failure_fixture() -> (SpellforgeRuntime51, SpellforgeTape) {
    let runtime = SpellforgeRuntime51::new(package(
        "value=3; function Mutate() value=99; return value end; function Suspend() value=99; InitGlobal(1,value); return value end; function Fail() value=99; InitGlobal(1,value); error('resume failed') end; function Read() return value end"
    )).unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    (runtime, tape)
}

fn begin_named(
    runtime: &SpellforgeRuntime51,
    tape: &SpellforgeTape,
    event: &str,
) -> SpellforgeStep {
    runtime
        .begin(
            SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: event.into(),
                args: vec![],
                script_this: 0,
                current_scroll: 0,
            },
            tape,
        )
        .unwrap()
}

fn activation(step: SpellforgeStep) -> u64 {
    match step {
        SpellforgeStep::Complete { activation, .. } | SpellforgeStep::Native { activation, .. } => {
            activation
        }
    }
}

fn assert_accounting(runtime: &SpellforgeRuntime51) {
    let inner = runtime.inner.lock().unwrap();
    let mut total = InFlightUsage::default();
    for active in inner.activations.values() {
        let measured = InFlightUsage {
            native_calls: active.native_calls.len(),
            transcript_entries: active
                .native_calls
                .iter()
                .map(|native| native.nested_transcript.len())
                .sum::<usize>()
                + active
                    .state
                    .pending()
                    .map_or(0, |pending| pending.nested_transcript.len()),
        };
        assert_eq!(active.usage, measured);
        total = total.add(measured).unwrap();
    }
    assert_eq!(inner.usage, total);
}

#[test]
fn unsuccessful_finalization_rebuilds_without_caller_abort() {
    for failure in ["wrong result", "suspended", "append"] {
        let (runtime, mut tape) = failure_fixture();
        let event = if failure == "suspended" {
            "Suspend"
        } else {
            "Mutate"
        };
        let id = activation(begin_named(&runtime, &tape, event));
        let original = tape.events.clone();
        if failure == "append" {
            // Fail snapshot admission after the guest has completed.
            tape.contract_version = 0;
        }
        let result = if failure == "wrong result" { 100 } else { 99 };
        assert!(runtime.commit(id, result, &mut tape).is_err(), "{failure}");
        assert_eq!(tape.events, original, "{failure}");
        tape.contract_version = SPELLFORGE_CONTRACT_VERSION;
        assert_accounting(&runtime);
        assert_eq!(invoke(&runtime, &mut tape, "Read", vec![]), 3, "{failure}");
    }
}

#[test]
fn failed_resume_and_explicit_abort_discard_uncommitted_mutations() {
    for event in ["Fail", "Suspend"] {
        let (runtime, mut tape) = failure_fixture();
        let id = activation(begin_named(&runtime, &tape, event));
        if event == "Fail" {
            assert!(runtime.resume(id, 0, &tape).is_err());
        } else {
            runtime.abort(id);
        }
        assert_accounting(&runtime);
        assert_eq!(invoke(&runtime, &mut tape, "Read", vec![]), 3);
    }
}

#[test]
fn nested_commit_failures_rebuild_the_entire_uncommitted_tree() {
    for missing_parent in [false, true] {
        let (runtime, mut tape) = failure_fixture();
        let parent = activation(begin_named(&runtime, &tape, "Suspend"));
        let child = activation(begin_named(&runtime, &tape, "Mutate"));
        {
            let mut inner = runtime.inner.lock().unwrap();
            if missing_parent {
                // Fault injection: the nested owner disappeared before commit.
                inner.activations.remove(&parent);
            } else {
                let active = inner.activations.get_mut(&parent).unwrap();
                active.state.pending_mut().unwrap().nested_transcript = vec![
                        SpellforgeNestedTapeEntry::NativeReturn { return_word: 0 };
                        SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT
                    ];
                active.usage.transcript_entries = SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT;
                inner.usage.transcript_entries = SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT;
            }
        }
        assert!(runtime.commit(child, 99, &mut tape).is_err());
        assert!(tape.events.is_empty());
        assert_accounting(&runtime);
        assert_eq!(invoke(&runtime, &mut tape, "Read", vec![]), 3);
    }
}

#[test]
fn incremental_usage_matches_nested_native_history_and_cleanup() {
    let runtime = SpellforgeRuntime51::new(package(
        "function Outer() for i=1,128 do InitGlobal(1,i) end end; function Inner() for i=1,4 do InitGlobal(2,i) end end"
    )).unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    let mut outer_step = begin_named(&runtime, &tape, "Outer");
    loop {
        assert_accounting(&runtime);
        match outer_step {
            SpellforgeStep::Native {
                activation: outer, ..
            } => {
                let mut child_step = begin_named(&runtime, &tape, "Inner");
                loop {
                    assert_accounting(&runtime);
                    match child_step {
                        SpellforgeStep::Native { activation, .. } => {
                            child_step = runtime.resume(activation, 0, &tape).unwrap()
                        }
                        SpellforgeStep::Complete { activation, result } => {
                            runtime.commit(activation, result, &mut tape).unwrap();
                            break;
                        }
                    }
                }
                assert_accounting(&runtime);
                outer_step = runtime.resume(outer, 0, &tape).unwrap();
            }
            SpellforgeStep::Complete { activation, result } => {
                runtime.commit(activation, result, &mut tape).unwrap();
                break;
            }
        }
    }
    assert_accounting(&runtime);
    assert_eq!(tape.events.len(), 1);
    // Rebuild exercises the same depth-first transcript after accounting moves.
    let rebuilt = SpellforgeRuntime51::new(runtime.package().clone()).unwrap();
    begin_named(&rebuilt, &tape, "Inner");
}

#[test]
fn in_flight_counter_limits_are_checked_at_the_exact_boundary() {
    let full = InFlightUsage {
        native_calls: SPELLFORGE_TAPE_NATIVE_CALL_LIMIT as usize,
        transcript_entries: SPELLFORGE_TAPE_TRANSCRIPT_ENTRY_LIMIT as usize,
    };
    assert_eq!(full.add(InFlightUsage::default()).unwrap(), full);
    assert!(
        full.add(InFlightUsage {
            native_calls: 1,
            transcript_entries: 0
        })
        .is_err()
    );
    assert!(
        full.add(InFlightUsage {
            native_calls: 0,
            transcript_entries: 1
        })
        .is_err()
    );
    assert!(
        InFlightUsage {
            native_calls: usize::MAX,
            transcript_entries: 0
        }
        .add(InFlightUsage {
            native_calls: 1,
            transcript_entries: 0
        })
        .is_err()
    );
}

#[test]
#[ignore = "requires LLVM unwind support; run with --config profile.test.package.robin_spellforge.codegen-backend=\"llvm\" -- --ignored"]
fn poisoned_runtime_rebuilds_from_committed_history() {
    let (runtime, mut tape) = failure_fixture();
    begin_named(&runtime, &tape, "Mutate");
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = runtime.inner.lock().unwrap();
        panic!("injected failure while holding the runtime");
    }));
    assert!(panic.is_err());
    assert_eq!(invoke(&runtime, &mut tape, "Read", vec![]), 3);
    assert_accounting(&runtime);
}

#[test]
fn every_exposed_native_and_alias_preserves_the_registry_word_abi() {
    let mut probes = Vec::new();
    for definition in NATIVE_REGISTRY
        .iter()
        .filter(|definition| definition.expose_to_lua)
    {
        probes.push((
            definition.signature.name,
            definition.native,
            &definition.signature,
        ));
    }
    for (alias, native) in SPELLFORGE_NATIVE_ALIASES {
        probes.push((
            *alias,
            *native,
            robin_engine::natives::native_signature_by_index(*native as u32).unwrap(),
        ));
    }
    let mut source = String::new();
    for (index, (name, _, signature)) in probes.iter().enumerate() {
        let arguments = signature
            .params
            .iter()
            .map(|parameter| match parameter.abi_type {
                NativeAbiType::Int => "-2147483648",
                NativeAbiType::Handle => "2147483647",
                NativeAbiType::Float => "1.5",
                NativeAbiType::Bool => "true",
                NativeAbiType::Void => panic!("void parameter"),
            })
            .collect::<Vec<_>>()
            .join(",");
        source.push_str(&format!(
            "function Probe{index}() return {name}({arguments}) end\n"
        ));
    }
    let runtime = SpellforgeRuntime51::new(package(&source)).unwrap();
    let mut tape = SpellforgeTape::default();
    tape.initialize(runtime.package().clone()).unwrap();
    for (index, (name, native, signature)) in probes.iter().enumerate() {
        let SpellforgeStep::Native {
            activation,
            native_index,
            arguments,
        } = begin_named(&runtime, &tape, &format!("Probe{index}"))
        else {
            panic!("{name} did not yield");
        };
        assert_eq!(native_index, *native as u32, "{name}");
        let expected = signature
            .params
            .iter()
            .map(|parameter| match parameter.abi_type {
                NativeAbiType::Int => i32::MIN,
                NativeAbiType::Handle => i32::MAX,
                NativeAbiType::Float => 1.5f32.to_bits() as i32,
                NativeAbiType::Bool => 1,
                NativeAbiType::Void => panic!("void parameter"),
            })
            .collect::<Vec<_>>();
        assert_eq!(arguments, expected, "{name}");
        let SpellforgeStep::Complete { result, .. } = runtime.resume(activation, 0, &tape).unwrap()
        else {
            panic!("{name} did not complete");
        };
        assert_eq!(result, 0, "{name}");
        runtime.commit(activation, result, &mut tape).unwrap();
    }
}

#[test]
fn unfinished_nested_events_cannot_be_interleaved_out_of_order() {
    let (runtime, mut tape) = failure_fixture();
    let parent = activation(begin_named(&runtime, &tape, "Suspend"));
    begin_named(&runtime, &tape, "Mutate");
    assert!(runtime.resume(parent, 0, &tape).is_err());
    assert_eq!(invoke(&runtime, &mut tape, "Read", vec![]), 3);

    begin_named(&runtime, &tape, "Mutate");
    let error = runtime
        .begin(
            SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: "Read".into(),
                args: vec![],
                script_this: 0,
                current_scroll: 0,
            },
            &tape,
        )
        .unwrap_err();
    assert_eq!(error.kind, SpellforgeGuestErrorKind::Protocol);
    assert_eq!(invoke(&runtime, &mut tape, "Read", vec![]), 3);
}

#[test]
fn cli_checks_modes_values_and_repeated_entity_filters() {
    use clap::Parser;
    let args = super::CliOptions::try_parse_from([
        "parity",
        "--dump-jsonl",
        "dump.jsonl",
        "--dump-from",
        "3",
        "--dump-entity",
        "pc:1",
        "--dump-entity",
        "soldier:2",
        "trace.jsonl",
    ])
    .unwrap();
    assert_eq!(args.dump_from, 3);
    assert_eq!(args.dump_entities.len(), 2);
    for arguments in [
        vec![
            "parity",
            "--core-datadir",
            "core",
            "--convert",
            "trace.jsonl",
        ],
        vec!["parity", "--convert", "--reblock", "trace.jsonl"],
        vec!["parity", "--reblock-records", "32", "trace.jsonl"],
        vec!["parity", "--http-server", "0", "trace.jsonl"],
        vec!["parity", "--start-paused", "trace.jsonl"],
        vec!["parity", "--dump-entity", "bad:1", "trace.jsonl"],
        vec!["parity", "--dump-entity", "pc:no", "trace.jsonl"],
    ] {
        assert!(super::CliOptions::try_parse_from(arguments).is_err());
    }
}
use super::*;
use base64::Engine as _;
use bitcode_parity as bitcode;

#[test]
fn deliberately_divergent_authoritative_fields_survive_comparison_and_reporting() {
    let recorded = serde_json::json!({
        "health": 100,
        "queue": [1, 2],
        "position": {"bits": 1065353216_u32, "value": 1.0},
        "required_state": true,
    });
    let runtime = serde_json::json!({
        "health": 99,
        "queue": [2, 1],
        "position": {"bits": 1073741824_u32, "value": 2.0},
        "rust_only_diagnostic": "not authoritative",
    });
    let mut differences = Vec::new();
    collect_json_subset_differences("actor", &recorded, &runtime, &mut differences);
    assert_eq!(
        differences,
        vec![
            "actor.health: original=Number(100) rust=Number(99)",
            "actor.position.bits: original=Number(1065353216) rust=Number(1073741824)",
            "actor.queue[0]: original=Number(1) rust=Number(2)",
            "actor.queue[1]: original=Number(2) rust=Number(1)",
            "actor.required_state: original=Bool(true) rust=<missing>",
        ]
    );
    let first_by_field: BTreeMap<_, _> = differences
        .iter()
        .map(|description| {
            (
                description.split(':').next().unwrap().to_owned(),
                (17, description.clone()),
            )
        })
        .collect();
    let report = structured_divergences(&first_by_field);
    assert_eq!(report.len(), 5);
    assert_eq!(report[0].field, "actor.health");
    assert_eq!(report[0].frame, 17);
    assert_eq!(report[0].description, differences[0]);
}

#[test]
fn deliberate_float_divergence_is_not_hidden_by_existing_tolerance() {
    let actor = EntityId::Pc(robin_engine::entity_id::PcId(7));
    let mut differences = Vec::new();
    compare_float(
        &mut differences,
        actor,
        "health",
        TraceFloat {
            bits: 1.0_f32.to_bits(),
        },
        1.5,
    );
    assert_eq!(differences.len(), 1);
    assert!(differences[0].contains("health: original=1 (0x3f800000) rust=1.5 (0x3fc00000)"));
    differences.clear();
    compare_float(
        &mut differences,
        actor,
        "health",
        TraceFloat {
            bits: 1.0_f32.to_bits(),
        },
        1.0 + 1.0e-6,
    );
    assert!(
        differences.is_empty(),
        "the established finite-float tolerance must remain unchanged"
    );
}

#[test]
fn late_movement_retranslation_drops_only_the_dangling_actor_animation() {
    let retransmitted = EntityId::Pc(robin_engine::entity_id::PcId(171));
    let unaffected = EntityId::Pc(robin_engine::entity_id::PcId(173));
    let late = [retransmitted];

    assert!(!original_actor_animation_is_logical(retransmitted, &late));
    assert!(original_actor_animation_is_logical(unaffected, &late));
}

fn lifecycle_event(
    actor_creation_order: u32,
    event: &str,
    phase: &str,
) -> TraceSequenceLifecycleEvent {
    TraceSequenceLifecycleEvent {
        ordinal: 0,
        frame_ordinal: 0,
        event: event.to_owned(),
        phase: phase.to_owned(),
        element_id: 1,
        sequence_id: Some(1),
        owner: None,
        owner_creation_order: None,
        command: 0,
        command_name: None,
        command_level: 1,
        state: None,
        priority: None,
        queue_size_before: None,
        queue_size_after: None,
        actor: None,
        actor_creation_order: Some(actor_creation_order),
        selected_sequence_id: None,
        selected_command: None,
        current_order_id: None,
        current_order_action: None,
        decision: None,
        accepted: Some(true),
    }
}

#[test]
fn completed_during_translation_drops_only_stale_execution_telemetry() {
    let completed = [lifecycle_event(
        167,
        "actor_instruct_result",
        "completed_during_translation",
    )];
    assert!(!original_actor_execution_telemetry_is_logical(
        167, &completed,
    ));
    assert!(original_actor_execution_telemetry_is_logical(
        168, &completed,
    ));
}

#[test]
fn original_motion_state_comparison_rejects_only_undefined_stack_values() {
    for raw in 0..=5 {
        assert!(original_motion_state_is_defined(raw));
    }
    assert!(!original_motion_state_is_defined(6));
    assert!(!original_motion_state_is_defined(1_494_023_856));
}

fn blocked_box_json(
    last_processed_order_id: u32,
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
) -> serde_json::Value {
    serde_json::json!({
        "sprite": {"last_processed_order_id": last_processed_order_id},
        "position": {
            "blocked_box": {
                "min": {"x": {"bits": min_x}, "y": {"bits": min_y}},
                "max": {"x": {"bits": max_x}, "y": {"bits": max_y}}
            }
        }
    })
}

fn deviated_blocked_box_json(
    last_processed_order_id: u32,
    map: (f32, f32),
    old_map: (f32, f32),
    deviated: bool,
    anti_collision_on: bool,
    blocked_count: u32,
) -> serde_json::Value {
    let half = 0.49_f32;
    serde_json::json!({
        "sprite": {"last_processed_order_id": last_processed_order_id},
        "position": {
            "anti_collision_on": anti_collision_on,
            "blocked_count": blocked_count,
            "deviated": deviated,
            "map": {
                "x": {"bits": map.0.to_bits()},
                "y": {"bits": map.1.to_bits()}
            },
            "old_map": {
                "x": {"bits": old_map.0.to_bits()},
                "y": {"bits": old_map.1.to_bits()}
            },
            "blocked_box": {
                "min": {
                    "x": {"bits": (map.0 - half).to_bits()},
                    "y": {"bits": (map.1 - half).to_bits()}
                },
                "max": {
                    "x": {"bits": (map.0 + half).to_bits()},
                    "y": {"bits": (map.1 + half).to_bits()}
                }
            }
        }
    })
}

fn trace_actor_order(
    action: robin_engine::order::OrderType,
    movement_sequence: bool,
    motion_state: robin_engine::sprite::MotionState,
    current_order_id: u32,
) -> TraceActor {
    TraceActor {
        action_state: 0,
        animation: action as u32,
        command: 22,
        command_name: "move_ok".to_owned(),
        motion_state: motion_state as u32,
        wait_time: 0,
        passing_door_directly: false,
        active_pass_door: None,
        sequence_element: Some(TraceSequenceElement {
            id: 1,
            element_type: 4,
            state: 2,
            command_level: 2,
            command: 22,
            command_name: "move_ok".to_owned(),
            order_count: 1,
            priority: 8,
            posture_after_transition: 1,
            action_state_after_transition: 0,
            movement: movement_sequence.then_some(TraceSequenceMovement {
                action: None,
                pass_door: None,
            }),
            following: None,
            postponed: None,
            current_order: Some(
                serde_json::from_value(
                    serde_json::json!({"id": current_order_id, "action": action as u32}),
                )
                .expect("current-order fixture must decode"),
            ),
            movement_payload: None,
        }),
        position_interface: missing_legacy_trace_json_value(),
    }
}

#[test]
fn legacy_blocked_box_reset_requires_perform_motion_execute_arm() {
    let soldier = EntityId::new(122, robin_engine::element::EntityIdKind::Soldier);
    assert!(!original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::TransitionWaitingUprightBoredWaitingUpright,
            true,
            robin_engine::sprite::MotionState::Start,
            10,
        ),
        soldier,
        10,
        false,
        None,
        None,
        None,
    ));
    assert!(!original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::TransitionWalkingUprightWaitingUpright,
            false,
            robin_engine::sprite::MotionState::Start,
            10,
        ),
        soldier,
        10,
        false,
        None,
        None,
        None,
    ));
    assert!(original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::WalkingUpright,
            true,
            robin_engine::sprite::MotionState::Start,
            10,
        ),
        soldier,
        10,
        false,
        None,
        None,
        None,
    ));
    assert!(original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::WalkingUpright,
            true,
            robin_engine::sprite::MotionState::InProgress,
            11,
        ),
        soldier,
        10,
        true,
        None,
        None,
        None,
    ));
    assert!(!original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::WalkingUpright,
            true,
            robin_engine::sprite::MotionState::InProgress,
            10,
        ),
        soldier,
        10,
        true,
        None,
        None,
        None,
    ));
    assert!(original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::WalkingUpright,
            true,
            robin_engine::sprite::MotionState::InProgress,
            10,
        ),
        soldier,
        10,
        true,
        Some(10),
        None,
        None,
    ));
    assert!(!original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::WalkingUpright,
            true,
            robin_engine::sprite::MotionState::InProgress,
            11,
        ),
        soldier,
        10,
        false,
        None,
        None,
        None,
    ));
    let pc = EntityId::new(101, robin_engine::element::EntityIdKind::Pc);
    assert!(original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::RunningWithSword,
            true,
            robin_engine::sprite::MotionState::InProgress,
            30,
        ),
        pc,
        30,
        false,
        Some(30),
        None,
        None,
    ));
    assert!(!original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::RunningWithSword,
            true,
            robin_engine::sprite::MotionState::InProgress,
            30,
        ),
        pc,
        30,
        false,
        Some(29),
        None,
        None,
    ));
    assert!(!original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::RunningWithSword,
            true,
            robin_engine::sprite::MotionState::InProgress,
            31,
        ),
        pc,
        30,
        false,
        Some(30),
        None,
        None,
    ));
    assert!(!original_reset_blocked_box_this_frame(
        &trace_actor_order(
            robin_engine::order::OrderType::RunningWithSword,
            false,
            robin_engine::sprite::MotionState::InProgress,
            30,
        ),
        pc,
        30,
        false,
        Some(30),
        None,
        None,
    ));
}

#[test]
fn legacy_blocked_box_reset_recognizes_hidden_stop_movement_rewrite() {
    let soldier = EntityId::new(126, robin_engine::element::EntityIdKind::Soldier);
    let turn_after_stop = trace_actor_order(
        robin_engine::order::OrderType::TransitionWalkingUprightWaitingUpright,
        false,
        robin_engine::sprite::MotionState::InProgress,
        30,
    );
    let prior_walking = LegacyStoppableMotionOrder {
        id: 20,
        stop_animation: robin_engine::order::OrderType::TransitionWalkingUprightWaitingUpright
            as u32,
    };
    assert!(original_reset_blocked_box_this_frame(
        &turn_after_stop,
        soldier,
        25,
        false,
        None,
        Some(prior_walking),
        Some(20),
    ));
    assert!(!original_reset_blocked_box_this_frame(
        &turn_after_stop,
        soldier,
        25,
        false,
        None,
        Some(prior_walking),
        Some(19),
    ));
    assert!(!original_reset_blocked_box_this_frame(
        &turn_after_stop,
        soldier,
        30,
        false,
        None,
        Some(prior_walking),
        Some(20),
    ));
    assert!(!original_reset_blocked_box_this_frame(
        &turn_after_stop,
        soldier,
        25,
        true,
        None,
        Some(prior_walking),
        Some(20),
    ));

    for (action, stop_animation) in [
        (
            robin_engine::order::OrderType::WalkingUpright,
            robin_engine::order::OrderType::TransitionWalkingUprightWaitingUpright,
        ),
        (
            robin_engine::order::OrderType::RunningUpright,
            robin_engine::order::OrderType::TransitionRunningUprightWaitingUpright,
        ),
        (
            robin_engine::order::OrderType::WalkingCrouched,
            robin_engine::order::OrderType::TransitionWalkingCrouchedWaitingCrouched,
        ),
    ] {
        assert_eq!(
            original_stoppable_current_motion_order(&trace_actor_order(
                action,
                true,
                robin_engine::sprite::MotionState::InProgress,
                20,
            )),
            Some(LegacyStoppableMotionOrder {
                id: 20,
                stop_animation: stop_animation as u32,
            }),
        );
    }
    assert_eq!(
        original_stoppable_current_motion_order(&trace_actor_order(
            robin_engine::order::OrderType::WaitingUpright,
            true,
            robin_engine::sprite::MotionState::InProgress,
            20,
        )),
        None,
    );

    let mut movement_with_following = trace_actor_order(
        robin_engine::order::OrderType::WalkingUpright,
        true,
        robin_engine::sprite::MotionState::InProgress,
        20,
    );
    movement_with_following
        .sequence_element
        .as_mut()
        .unwrap()
        .order_count = 2;
    assert_eq!(
        original_stoppable_current_motion_order(&movement_with_following),
        Some(prior_walking),
        "stopping movement rewrites the current movement even when a following action remains",
    );
}

#[test]
fn legacy_blocked_box_shadow_tracks_resets_and_revalidation() {
    let tuple = LegacyBlockedBoxTuple {
        min_x: 1,
        min_y: 2,
        max_x: 3,
        max_y: 4,
    };
    let mut shadows = BTreeMap::from([(
        10,
        LegacyBlockedBoxShadow {
            tuple,
            validity: LegacyBlockedBoxValidity::Set,
            last_processed_order_id: 20,
            pending_motion_order_id: None,
            stoppable_motion_order: None,
            deviated: None,
            direct_validity_observed: false,
        },
    )]);

    let mut active = blocked_box_json(20, 1, 2, 3, 4);
    assert!(!canonicalize_legacy_blocked_box(
        &mut active,
        10,
        false,
        None,
        None,
        &mut shadows,
    ));
    assert!(!active.pointer("/position/blocked_box").unwrap().is_null());

    let mut continuing_motion = blocked_box_json(20, 1, 2, 3, 4);
    assert!(!canonicalize_legacy_blocked_box(
        &mut continuing_motion,
        10,
        true,
        None,
        None,
        &mut shadows,
    ));
    assert_eq!(
        shadows[&10].validity,
        LegacyBlockedBoxValidity::Set,
        "MotionState::Start alone is insufficient without a new raw order id"
    );

    let mut pending_motion = blocked_box_json(20, 1, 2, 3, 4);
    assert!(!canonicalize_legacy_blocked_box(
        &mut pending_motion,
        10,
        true,
        Some(22),
        None,
        &mut shadows,
    ));
    assert_eq!(shadows[&10].pending_motion_order_id, Some(22));

    let mut began_pending_motion = blocked_box_json(22, 1, 2, 3, 4);
    assert!(canonicalize_legacy_blocked_box(
        &mut began_pending_motion,
        10,
        true,
        Some(22),
        None,
        &mut shadows,
    ));
    assert!(
        began_pending_motion
            .pointer("/position/blocked_box")
            .unwrap()
            .is_null(),
        "a previously observed pending motion order resets when it becomes processed"
    );

    // Re-establish a valid tuple for the remaining independent transition
    // cases below.
    shadows.get_mut(&10).unwrap().validity = LegacyBlockedBoxValidity::Set;

    let mut action_order = blocked_box_json(21, 1, 2, 3, 4);
    assert!(!canonicalize_legacy_blocked_box(
        &mut action_order,
        10,
        false,
        None,
        None,
        &mut shadows,
    ));
    assert_eq!(
        shadows[&10].validity,
        LegacyBlockedBoxValidity::Set,
        "action processing also changes the raw order id but does not reset the box"
    );

    let mut reset = blocked_box_json(22, 1, 2, 3, 4);
    assert!(canonicalize_legacy_blocked_box(
        &mut reset,
        10,
        true,
        None,
        None,
        &mut shadows,
    ));
    assert!(reset.pointer("/position/blocked_box").unwrap().is_null());

    let mut still_unset = blocked_box_json(22, 1, 2, 3, 4);
    assert!(canonicalize_legacy_blocked_box(
        &mut still_unset,
        10,
        false,
        None,
        None,
        &mut shadows,
    ));

    let mut revalidated = blocked_box_json(23, 1, 2, 3, 5);
    assert!(!canonicalize_legacy_blocked_box(
        &mut revalidated,
        10,
        true,
        None,
        None,
        &mut shadows,
    ));
    assert_eq!(
        shadows[&10].validity,
        LegacyBlockedBoxValidity::Set,
        "a post-reset tuple update revalidates the box"
    );

    let mut revisited_old_tuple = blocked_box_json(23, 1, 2, 3, 4);
    assert!(!canonicalize_legacy_blocked_box(
        &mut revisited_old_tuple,
        10,
        false,
        None,
        None,
        &mut shadows,
    ));
    assert!(
        !revisited_old_tuple
            .pointer("/position/blocked_box")
            .unwrap()
            .is_null()
    );
}

#[test]
fn legacy_blocked_box_shadow_recognizes_same_tuple_deviation_revalidation() {
    let map = (639.58826_f32, 1250.3961_f32);
    let mut runtime = deviated_blocked_box_json(20, map, (644.4594, 1249.2684), true, true, 0);
    let tuple = legacy_blocked_box_tuple(&runtime).unwrap();
    assert!(legacy_blocked_box_revalidated_by_deviation(
        &runtime,
        tuple,
        Some(false),
    ));

    let mut shadows = BTreeMap::from([(
        10,
        LegacyBlockedBoxShadow {
            tuple,
            validity: LegacyBlockedBoxValidity::Unset,
            last_processed_order_id: 20,
            pending_motion_order_id: None,
            stoppable_motion_order: None,
            deviated: Some(false),
            direct_validity_observed: false,
        },
    )]);
    assert!(!canonicalize_legacy_blocked_box(
        &mut runtime,
        10,
        false,
        None,
        None,
        &mut shadows,
    ));
    assert_eq!(shadows[&10].validity, LegacyBlockedBoxValidity::Set);

    for mut stale in [
        deviated_blocked_box_json(20, map, (644.4594, 1249.2684), false, true, 0),
        deviated_blocked_box_json(20, map, (644.4594, 1249.2684), true, false, 0),
        deviated_blocked_box_json(20, map, (644.4594, 1249.2684), true, true, 1),
        deviated_blocked_box_json(20, map, map, true, true, 0),
    ] {
        assert!(!legacy_blocked_box_revalidated_by_deviation(
            &stale,
            legacy_blocked_box_tuple(&stale).unwrap(),
            Some(false),
        ));
        stale["position"]["blocked_box"]["max"]["x"]["bits"] = serde_json::json!(0_u32);
        assert!(!legacy_blocked_box_revalidated_by_deviation(
            &stale,
            legacy_blocked_box_tuple(&stale).unwrap(),
            Some(false),
        ));
    }
    let unchanged_deviation =
        deviated_blocked_box_json(20, map, (644.4594, 1249.2684), true, true, 0);
    assert!(!legacy_blocked_box_revalidated_by_deviation(
        &unchanged_deviation,
        legacy_blocked_box_tuple(&unchanged_deviation).unwrap(),
        Some(true),
    ));
    let mut unchanged_deviation = unchanged_deviation;
    let unchanged_tuple = legacy_blocked_box_tuple(&unchanged_deviation).unwrap();
    let mut stale_shadows = BTreeMap::from([(
        10,
        LegacyBlockedBoxShadow {
            tuple: unchanged_tuple,
            validity: LegacyBlockedBoxValidity::Unset,
            last_processed_order_id: 20,
            pending_motion_order_id: None,
            stoppable_motion_order: None,
            deviated: Some(true),
            direct_validity_observed: false,
        },
    )]);
    assert!(canonicalize_legacy_blocked_box(
        &mut unchanged_deviation,
        10,
        false,
        None,
        None,
        &mut stale_shadows,
    ));
    assert!(
        unchanged_deviation
            .pointer("/position/blocked_box")
            .unwrap()
            .is_null()
    );
}

#[test]
fn legacy_blocked_box_shadow_does_not_guess_for_runtime_actor() {
    let mut shadows = BTreeMap::new();
    let mut first = blocked_box_json(20, 1, 2, 3, 4);
    assert!(!canonicalize_legacy_blocked_box(
        &mut first,
        10,
        false,
        None,
        None,
        &mut shadows,
    ));
    assert_eq!(shadows[&10].validity, LegacyBlockedBoxValidity::Unknown);

    let mut reset = blocked_box_json(21, 1, 2, 3, 4);
    assert!(canonicalize_legacy_blocked_box(
        &mut reset,
        10,
        true,
        None,
        None,
        &mut shadows,
    ));
    assert!(reset.pointer("/position/blocked_box").unwrap().is_null());
}

#[test]
fn legacy_blocked_box_shadow_preserves_save_proven_inactive_box() {
    let mut shadows = BTreeMap::from([(
        10,
        LegacyBlockedBoxShadow {
            tuple: LegacyBlockedBoxTuple {
                min_x: 1,
                min_y: 2,
                max_x: 3,
                max_y: 4,
            },
            validity: LegacyBlockedBoxValidity::Unset,
            last_processed_order_id: 20,
            pending_motion_order_id: None,
            stoppable_motion_order: None,
            deviated: None,
            direct_validity_observed: false,
        },
    )]);
    let mut runtime = blocked_box_json(20, 1, 2, 3, 4);
    assert!(canonicalize_legacy_blocked_box(
        &mut runtime,
        10,
        false,
        None,
        None,
        &mut shadows,
    ));
}

#[test]
fn native_suffix_appends_to_the_recording_identity() {
    // The `.jsonl.zst` path is the stable trace identity; the native
    // artifact must derive from it by appending, never by renaming.
    let native = native_binary_trace_path(Path::new("dir/replay-001-session-0001.jsonl.zst"));
    assert_eq!(
        native,
        PathBuf::from(format!(
            "dir/replay-001-session-0001.jsonl.zst{TRACE_NATIVE_SUFFIX}"
        ))
    );
    assert!(!TRACE_NATIVE_SUFFIX.contains("-v"));

    // Direct-from-capture conversions skip the interim zstd recording
    // but keep the identical artifact identity.
    let uncompressed = native_binary_trace_path(Path::new("dir/replay-001-session-0001.jsonl"));
    assert_eq!(uncompressed, native);
}

#[test]
fn npc_boundary_transients_are_typed_bounded_and_legacy_defaulted() {
    let parsed: Vec<TraceInitialNpcTransient> = serde_json::from_value(serde_json::json!([
        {"creation_order": 96, "maximal_visibility": 31},
        {"creation_order": 117, "maximal_visibility": 47}
    ]))
    .expect("parse schema-16 NPC boundary transients");
    assert_eq!(
        parsed,
        [
            TraceInitialNpcTransient {
                creation_order: 96,
                maximal_visibility: 31,
            },
            TraceInitialNpcTransient {
                creation_order: 117,
                maximal_visibility: 47,
            },
        ]
    );
    assert!(
        serde_json::from_value::<TraceInitialNpcTransient>(serde_json::json!({
            "creation_order": 96,
            "maximal_visibility": 65_536
        }))
        .is_err(),
        "The original game's maximum visibility is 16-bit and must not be widened silently"
    );

    let mut header = serde_json::to_value(minimal_test_native_header("test").trace).unwrap();
    header
        .as_object_mut()
        .unwrap()
        .remove("initial_npc_transients");
    let legacy = serde_json::from_value::<TraceHeader>(header)
        .expect("legacy schema-16 headers default the additive NPC transient boundary");
    assert!(legacy.initial_npc_transients.is_none());

    let present_empty = minimal_test_native_header("test").trace;
    assert_eq!(present_empty.initial_npc_transients, Some(Vec::new()));
}

#[test]
fn legacy_segment_visibility_fallback_matches_original_uword_conversion() {
    assert_eq!(
        reconstruct_unrecorded_maximal_visibility(false, [0.0, 1.599_999_9, 0.25]),
        31
    );
    assert_eq!(
        reconstruct_unrecorded_maximal_visibility(false, [2.399_999_9]),
        47
    );
    assert_eq!(
        reconstruct_unrecorded_maximal_visibility(true, [1.599_999_9]),
        319
    );
    assert_eq!(
        reconstruct_unrecorded_maximal_visibility(false, std::iter::empty()),
        0
    );
}

#[test]
fn legacy_visibility_fallback_only_applies_to_in_process_reload_envelopes() {
    assert!(legacy_loaded_save_retains_process_transients(0));
    assert!(!legacy_loaded_save_retains_process_transients(76));
}

fn write_test_native_records(
    records: &[BinaryTraceRecord],
    footer: Option<BinaryTraceFooter>,
) -> tempfile::NamedTempFile {
    write_test_native_records_with_compression(
        records,
        footer,
        TRACE_NATIVE_ZSTD_LEVEL,
        TRACE_NATIVE_LONG_DISTANCE_MATCHING,
    )
}

fn minimal_test_native_header(source_fingerprint: &str) -> BinaryTraceHeaderV68 {
    BinaryTraceHeaderV68 {
        version: TRACE_NATIVE_VERSION,
        source_fingerprint: source_fingerprint.to_owned(),
        trace: TraceHeader {
            record_type: "header".to_owned(),
            mission: "test".to_owned(),
            proto_level: "test".to_owned(),
            rng_seed: 1,
            schema: TRACE_SCHEMA_VERSION,
            session_index: 1,
            start_state: TraceStartState::MissionStart,
            initial_frame: 0,
            simulation_hz: 25,
            synchronous_pathfinding: true,
            rng_stream: "libc_rand_raw_global_draw_order".to_owned(),
            visibility_queries: "opaque_is_reachable".to_owned(),
            random_input_seed: None,
            sim_config: TraceSimConfig {
                difficulty: TraceDifficulty::Medium,
                script_enabled: true,
                highlander: false,
                highlander2: false,
                golden_eye: false,
                ignore_default_loose: false,
                bypass_fog_sprites_crash: false,
                amount_of_speaking: 0,
            },
            campaign: TraceCampaign {
                version: 1,
                values: Vec::new(),
                ares: 0,
                missions: Vec::new(),
                accessible_mission_indices: Vec::new(),
                pending_accessible_mission_indices: Vec::new(),
                last_mission_index: None,
                current_mission_index: None,
                next_mission_index: None,
                blazon_mission_index: None,
                last_played_mission_indices: Vec::new(),
                last_pseudo_mission_status: 0,
                last_pseudo_mission_id: 0,
                characters: Vec::new(),
                gang_indices: Vec::new(),
                reservist_indices: Vec::new(),
                mission_team_indices: Vec::new(),
                peasant_names: Vec::new(),
                reservists_are_back: false,
                collected_relics: Vec::new(),
                production_sectors: Vec::new(),
            },
            motion_grid: TraceMotionGrid { layers: Vec::new() },
            initial_npc_transients: Some(Vec::new()),
            initial_save: None,
        },
        rng_prefix: TraceRngPrefix {
            r#type: "rng_prefix".to_owned(),
            draws: TraceRngBatch {
                first_index: 0,
                values: Vec::new(),
                callsite_offsets: Vec::new(),
                main_thread: Vec::new(),
                domains: Vec::new(),
            },
        },
    }
}

fn write_synthetic_native_trace(path: &Path, source_fingerprint: &str, checksum: bool) {
    let file = File::create(path).unwrap();
    let mut encoder = zstd::stream::write::Encoder::new(BufWriter::new(file), 1).unwrap();
    encoder.window_log(20).unwrap();
    encoder.include_checksum(checksum).unwrap();
    write_binary_record(
        &mut encoder,
        &minimal_test_native_header(source_fingerprint),
        "synthetic native header",
    );
    write_binary_record(
        &mut encoder,
        std::slice::from_ref(&complete_test_end(0, 0)),
        "synthetic native block",
    );
    let mut writer = encoder.finish().unwrap();
    write_binary_trace_footer(
        &mut writer,
        BinaryTraceFooter {
            version: TRACE_NATIVE_VERSION,
            frame_count: 0,
            final_frame: 0,
        },
    )
    .unwrap();
    writer.flush().unwrap();
    writer.get_ref().sync_all().unwrap();
}

fn write_test_native_records_with_compression(
    records: &[BinaryTraceRecord],
    footer: Option<BinaryTraceFooter>,
    level: i32,
    long_distance_matching: bool,
) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    {
        let mut encoder =
            zstd::stream::write::Encoder::new(BufWriter::new(file.as_file_mut()), level).unwrap();
        if long_distance_matching {
            encoder.long_distance_matching(true).unwrap();
        }
        encoder.window_log(20).unwrap();
        for record in records {
            write_binary_record(
                &mut encoder,
                std::slice::from_ref(record),
                "test native trace block",
            );
        }
        let mut writer = encoder.finish().unwrap();
        if let Some(footer) = footer {
            write_binary_trace_footer(&mut writer, footer).unwrap();
        }
        writer.flush().unwrap();
    }
    file.as_file().sync_all().unwrap();
    file
}

fn write_test_native_stream_with_policy(
    header: &BinaryTraceHeaderV68,
    records: &[BinaryTraceRecord],
    footer: BinaryTraceFooter,
    policy: NativeStoragePolicy,
) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    {
        let mut encoder = zstd::stream::write::Encoder::new(
            BufWriter::new(file.as_file_mut()),
            TRACE_NATIVE_ZSTD_LEVEL,
        )
        .unwrap();
        configure_cache_compression(&mut encoder, None, policy.window_log);
        write_binary_record(&mut encoder, header, "test native trace header");
        for block in records.chunks(policy.block_records) {
            write_binary_record(&mut encoder, block, "test native trace block");
        }
        let mut writer = encoder.finish().unwrap();
        write_binary_trace_footer(&mut writer, footer).unwrap();
        writer.flush().unwrap();
    }
    file.as_file().sync_all().unwrap();
    file
}

fn complete_test_end(frame_count: u64, final_frame: u64) -> BinaryTraceRecord {
    BinaryTraceRecord::End {
        rng_suffix: Some(TraceRngBatch {
            first_index: 0,
            values: Vec::new(),
            callsite_offsets: Vec::new(),
            main_thread: Vec::new(),
            domains: Vec::new(),
        }),
        final_frame: Some(final_frame),
        frame_count: Some(frame_count),
    }
}

#[test]
fn reblock_refreshes_stale_semantic_digest_for_exact_bound_source() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("stale-binding.parity.bitcode.zst");
    write_synthetic_native_trace(&native, "stale-binding", false);
    let source = native_reblock_source_path(&native);
    let binding_path = native_reblock_binding_path(&native);
    let mut binding = create_native_reblock_binding(&native);
    binding.source_semantic_sha256 = "pre-projection-change-digest".to_owned();
    write_native_reblock_binding(&binding_path, &binding);
    std::fs::hard_link(&native, &source).unwrap();

    reblock_native_trace(&native, NativeStoragePolicy::default());

    assert_eq!(
        read_binary_trace_footer(&native).unwrap().version,
        TRACE_NATIVE_VERSION
    );
    assert!(!source.exists());
    assert!(!binding_path.exists());
}

#[test]
fn retired_native_versions_are_rejected_before_reading_record_bytes() {
    for version in [0, 66, 67, 69] {
        let footer = BinaryTraceFooter {
            version,
            frame_count: 0,
            final_frame: 0,
        };
        assert!(
            validate_binary_trace_footer(&footer)
                .unwrap_err()
                .contains("migrate")
        );
        assert!(
            read_binary_trace_header_record(&mut std::io::empty(), version)
                .unwrap_err()
                .contains("migrate")
        );
        assert!(
            read_binary_trace_block_record(&mut std::io::empty(), version)
                .unwrap_err()
                .contains("migrate")
        );
    }
}

#[test]
fn native_small_block_policy_round_trips_records_and_footer() {
    assert_eq!(TRACE_NATIVE_VERSION, 68);
    assert_eq!(TRACE_NATIVE_ZSTD_LEVEL, 19);
    const { assert!(!TRACE_NATIVE_LONG_DISTANCE_MATCHING) };
    assert_eq!(TRACE_NATIVE_BLOCK_RECORDS, 32);
    assert_eq!(TRACE_NATIVE_WINDOW_LOG, 26);
    assert_eq!(
        NativeStoragePolicy::new(1, TRACE_NATIVE_MIN_WINDOW_LOG),
        NativeStoragePolicy {
            block_records: 1,
            window_log: 20,
        }
    );
    assert_eq!(
        NativeStoragePolicy::new(
            TRACE_NATIVE_MAX_REBLOCK_RECORDS,
            TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG,
        ),
        NativeStoragePolicy {
            block_records: 1000,
            window_log: 29,
        }
    );

    let footer = BinaryTraceFooter {
        version: TRACE_NATIVE_VERSION,
        frame_count: 0,
        final_frame: 10,
    };
    let native = write_test_native_records(&[complete_test_end(0, 10)], Some(footer));
    let mut reader = BinaryTraceReader::open(native.path());
    assert!(matches!(
        reader.read_record(),
        BinaryTraceRecord::End { .. }
    ));
    reader.validate_terminator(0, 10).unwrap();
    assert_eq!(read_binary_trace_footer(native.path()).unwrap(), footer);
}

#[test]
#[should_panic(expected = "--reblock-records must be between 1 and 1000")]
fn native_storage_policy_rejects_empty_blocks() {
    let _ = NativeStoragePolicy::new(0, TRACE_NATIVE_WINDOW_LOG);
}

#[test]
#[should_panic(expected = "--reblock-window-log must be between 20 and 29")]
fn native_storage_policy_rejects_oversized_windows() {
    let _ = NativeStoragePolicy::new(
        TRACE_NATIVE_BLOCK_RECORDS,
        TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG + 1,
    );
}

#[test]
fn current_reader_preserves_small_block_semantics_digest_and_terminator() {
    let policy = NativeStoragePolicy::default();
    assert_eq!(policy, NativeStoragePolicy::new(32, 26));
    let header = minimal_test_native_header("small-block-v68");
    let mut records = Vec::new();
    for frame_before in 0..65_u64 {
        let mut value = minimal_frame_json();
        value["frame_before"] = frame_before.into();
        value["frame_after"] = (frame_before + 1).into();
        records.push(BinaryTraceRecord::Frame(
            serde_json::from_value(value).unwrap(),
        ));
    }
    records.push(complete_test_end(65, 65));
    let footer = BinaryTraceFooter {
        version: TRACE_NATIVE_VERSION,
        frame_count: 65,
        final_frame: 65,
    };
    let mut expected_digest = Sha256::new();
    update_native_semantic_digest(&mut expected_digest, &header);
    for record in &records {
        update_native_semantic_digest(&mut expected_digest, record);
    }
    let expected_digest = expected_digest.finalize();
    let native = write_test_native_stream_with_policy(&header, &records, footer, policy);

    // This is the ordinary version-68 reader: block cardinality is not
    // represented in the header or footer and has never been fixed at 1,000.
    let mut reader = BinaryTraceReader::open(native.path());
    assert_eq!(reader.read_header().version, TRACE_NATIVE_VERSION);
    assert!(matches!(reader.read_record(), BinaryTraceRecord::Frame(_)));
    assert_eq!(reader.pending.len(), 31);
    for _ in 1..32 {
        assert!(matches!(reader.read_record(), BinaryTraceRecord::Frame(_)));
    }
    assert!(reader.pending.is_empty());
    assert!(matches!(reader.read_record(), BinaryTraceRecord::Frame(_)));
    assert_eq!(reader.pending.len(), 31);

    let (decoded_frames, actual_digest) = digest_and_validate_native_trace(native.path());
    assert_eq!(decoded_frames, 65);
    assert_eq!(actual_digest, expected_digest);
    assert_eq!(read_binary_trace_footer(native.path()).unwrap(), footer);
}

#[test]
fn reblock_recovery_source_is_adjacent_and_version_specific() {
    let native = Path::new("dir/replay-001-session-0001.jsonl.zst.parity.bitcode.zst");
    assert_eq!(
        native_reblock_source_path(native),
        PathBuf::from(
            "dir/replay-001-session-0001.jsonl.zst.parity.bitcode.zst.parity-reblock-source-v67"
        )
    );
    assert_eq!(
        native_reblock_binding_path(native),
        PathBuf::from(
            "dir/replay-001-session-0001.jsonl.zst.parity.bitcode.zst.parity-reblock-binding-v67.json"
        )
    );
}

#[test]
fn reblock_binding_rejects_foreign_content_and_inode() {
    let binding = NativeReblockBinding {
        version: TRACE_NATIVE_VERSION,
        canonical_path: PathBuf::from("trace.parity.bitcode.zst"),
        source_content_sha256: "bound-content".to_owned(),
        source_bytes: 123,
        source_semantic_sha256: "bound-semantics".to_owned(),
        frame_count: 10,
        final_frame: 20,
        #[cfg(unix)]
        source_device: 30,
        #[cfg(unix)]
        source_inode: 40,
    };
    assert!(native_reblock_file_identity_matches(
        &binding,
        123,
        "bound-content",
        30,
        40
    ));
    assert!(!native_reblock_file_identity_matches(
        &binding,
        123,
        "foreign-content",
        30,
        40
    ));
    #[cfg(unix)]
    assert!(!native_reblock_file_identity_matches(
        &binding,
        123,
        "bound-content",
        30,
        41
    ));
}

#[test]
fn foreign_reblock_recovery_without_binding_cannot_replace_canonical() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("trace.parity.bitcode.zst");
    let source = native_reblock_source_path(&native);
    let binding = native_reblock_binding_path(&native);
    write_synthetic_native_trace(&native, "canonical", false);
    write_synthetic_native_trace(&source, "foreign", false);
    let canonical_before = std::fs::read(&native).unwrap();
    let foreign_before = std::fs::read(&source).unwrap();

    assert!(
        classify_native_reblock_recovery_state(true, source.exists(), binding.exists()).is_err()
    );
    assert_eq!(std::fs::read(&native).unwrap(), canonical_before);
    assert_eq!(std::fs::read(&source).unwrap(), foreign_before);
    assert!(!binding.exists());
}

#[test]
fn stale_bound_reblock_source_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("trace.parity.bitcode.zst");
    let source = native_reblock_source_path(&native);
    let binding_path = native_reblock_binding_path(&native);
    write_synthetic_native_trace(&native, "canonical", false);
    let binding = create_native_reblock_binding(&native);
    write_native_reblock_binding(&binding_path, &binding);
    write_synthetic_native_trace(&source, "foreign", false);
    let canonical_before = std::fs::read(&native).unwrap();
    let source_before = std::fs::read(&source).unwrap();
    let binding_before = std::fs::read(&binding_path).unwrap();

    assert!(validate_native_reblock_source_file_identity(&source, &binding).is_err());
    assert_eq!(std::fs::read(&native).unwrap(), canonical_before);
    assert_eq!(std::fs::read(&source).unwrap(), source_before);
    assert_eq!(std::fs::read(&binding_path).unwrap(), binding_before);
}

#[test]
fn authenticated_reblock_source_survives_missing_canonical() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("trace.parity.bitcode.zst");
    let source = native_reblock_source_path(&native);
    let binding_path = native_reblock_binding_path(&native);
    write_synthetic_native_trace(&native, "canonical", false);
    let binding = create_native_reblock_binding(&native);
    write_native_reblock_binding(&binding_path, &binding);
    std::fs::hard_link(&native, &source).unwrap();
    std::fs::remove_file(&native).unwrap();

    assert!(matches!(
        prepare_native_reblock_source(&native, &source, &binding_path),
        NativeReblockPreparation::Ready(_)
    ));
    assert!(!native.exists());
    assert!(source.exists());
    assert!(binding_path.exists());
}

#[test]
fn missing_reblock_canonical_without_authenticated_pair_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("trace.parity.bitcode.zst");
    let source = native_reblock_source_path(&native);
    let binding = native_reblock_binding_path(&native);
    assert!(
        classify_native_reblock_recovery_state(native.exists(), source.exists(), binding.exists())
            .is_err()
    );
}

#[test]
fn binding_only_without_canonical_or_source_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("trace.parity.bitcode.zst");
    let source = native_reblock_source_path(&native);
    let binding_path = native_reblock_binding_path(&native);
    write_synthetic_native_trace(&native, "canonical", false);
    let binding = create_native_reblock_binding(&native);
    write_native_reblock_binding(&binding_path, &binding);
    std::fs::remove_file(&native).unwrap();

    assert!(
        classify_native_reblock_recovery_state(
            native.exists(),
            source.exists(),
            binding_path.exists()
        )
        .is_err()
    );
    assert!(binding_path.exists());
}

#[test]
fn binding_only_same_inode_canonical_recreates_recovery_link() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("trace.parity.bitcode.zst");
    let source = native_reblock_source_path(&native);
    let binding_path = native_reblock_binding_path(&native);
    write_synthetic_native_trace(&native, "canonical", false);
    let binding = create_native_reblock_binding(&native);
    write_native_reblock_binding(&binding_path, &binding);

    assert!(matches!(
        prepare_native_reblock_source(&native, &source, &binding_path),
        NativeReblockPreparation::Ready(_)
    ));
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(&native).unwrap().ino(),
        std::fs::metadata(&source).unwrap().ino()
    );
}

#[test]
fn semantic_equal_postpublish_state_only_cleans_binding() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("trace.parity.bitcode.zst");
    let source = native_reblock_source_path(&native);
    let binding_path = native_reblock_binding_path(&native);
    let replacement = directory.path().join("replacement.parity.bitcode.zst");
    write_synthetic_native_trace(&native, "canonical", false);
    let binding = create_native_reblock_binding(&native);
    write_native_reblock_binding(&binding_path, &binding);
    std::fs::hard_link(&native, &source).unwrap();
    write_synthetic_native_trace(&replacement, "canonical", true);
    assert_ne!(
        trace_content_sha256(&native),
        trace_content_sha256(&replacement)
    );
    std::fs::rename(&replacement, &native).unwrap();
    std::fs::remove_file(&source).unwrap();

    assert!(matches!(
        prepare_native_reblock_source(&native, &source, &binding_path),
        NativeReblockPreparation::AlreadyCommitted
    ));
    assert!(native.exists());
    assert!(!source.exists());
    assert!(!binding_path.exists());
}

#[test]
fn reblock_orphan_cleanup_is_trace_scoped_and_preserves_symlinks() {
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("trace.parity.bitcode.zst");
    let other_native = directory.path().join("other.parity.bitcode.zst");
    let output_orphan = directory.path().join(format!(
        "{}dead",
        native_reblock_temporary_prefix(&native, false)
    ));
    let binding_orphan = directory.path().join(format!(
        "{}dead",
        native_reblock_temporary_prefix(&native, true)
    ));
    let other_orphan = directory.path().join(format!(
        "{}live",
        native_reblock_temporary_prefix(&other_native, false)
    ));
    let unrelated = directory.path().join(".parity-reblock-v67-unrelated");
    std::fs::write(&output_orphan, b"partial output").unwrap();
    std::fs::write(&binding_orphan, b"partial binding").unwrap();
    std::fs::write(&other_orphan, b"another trace").unwrap();
    std::fs::write(&unrelated, b"not ours").unwrap();
    #[cfg(unix)]
    let symlink = {
        let symlink = directory.path().join(format!(
            "{}symlink",
            native_reblock_temporary_prefix(&native, false)
        ));
        std::os::unix::fs::symlink(&unrelated, &symlink).unwrap();
        symlink
    };

    cleanup_native_reblock_orphans(&native);

    assert!(!output_orphan.exists());
    assert!(!binding_orphan.exists());
    assert!(other_orphan.exists());
    assert!(unrelated.exists());
    #[cfg(unix)]
    assert!(
        std::fs::symlink_metadata(symlink)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn reblock_temporary_owner_hash_distinguishes_non_utf8_paths() {
    use std::os::unix::ffi::OsStringExt as _;

    let directory = tempfile::tempdir().unwrap();
    let first = directory
        .path()
        .join(std::ffi::OsString::from_vec(b"trace-\xfe".to_vec()));
    let second = directory
        .path()
        .join(std::ffi::OsString::from_vec(b"trace-\xff".to_vec()));
    assert_ne!(
        native_reblock_temporary_prefix(&first, false),
        native_reblock_temporary_prefix(&second, false)
    );
}

#[test]
fn native_maintenance_commands_accept_logical_and_native_paths() {
    let directory = tempfile::tempdir().unwrap();
    let logical = directory.path().join("replay-001-session-0001.jsonl.zst");
    let native = native_binary_trace_path(&logical);
    assert_eq!(requested_native_trace_path(&logical), native);
    assert_eq!(requested_native_trace_path(&native), native);

    std::fs::write(&logical, b"coexisting source with a different fingerprint").unwrap();
    write_synthetic_native_trace(&native, "native-parity-v67:legacy", true);
    let before = trace_content_sha256(&native);
    assert_eq!(ensure_native_binary_trace(&native), native);
    assert_eq!(trace_content_sha256(&native), before);
}

#[test]
fn native_reader_accepts_current_version_with_level_nineteen_ldm() {
    let footer = BinaryTraceFooter {
        version: TRACE_NATIVE_VERSION,
        frame_count: 0,
        final_frame: 10,
    };
    let native = write_test_native_records_with_compression(
        &[complete_test_end(0, 10)],
        Some(footer),
        19,
        true,
    );
    let mut reader = BinaryTraceReader::open(native.path());
    assert!(matches!(
        reader.read_record(),
        BinaryTraceRecord::End { .. }
    ));
    reader.validate_terminator(0, 10).unwrap();
}

#[test]
fn conversion_cleanup_resolves_relative_and_absolute_paths() {
    let directory = tempfile::tempdir().unwrap();
    let relative = Path::new("relative-trace.jsonl.zst");
    let resolved_relative = absolute_trace_path_from(relative, directory.path());
    assert_eq!(resolved_relative, directory.path().join(relative));

    let absolute = directory.path().join("absolute-trace.jsonl.zst");
    assert_eq!(
        absolute_trace_path_from(&absolute, Path::new("/ignored")),
        absolute
    );

    for trace in [&resolved_relative, &absolute] {
        std::fs::write(trace, b"recording").unwrap();
        let fingerprint = trace_source_fingerprint(trace);
        let quarantine = conversion_quarantine_path(trace);
        let obsolete = PathBuf::from(format!(
            "{}.parity-cache-v63.test",
            trace.as_os_str().to_string_lossy()
        ));
        std::fs::write(&obsolete, b"derived").unwrap();
        let verified = VerifiedNativeReadback {
            decoded_frames: 0,
            source_path: trace.to_path_buf(),
            source_fingerprint: fingerprint,
        };
        move_verified_recording_to_quarantine(trace, &quarantine, &verified).unwrap();
        assert_eq!(
            finish_verified_conversion(
                trace,
                &quarantine,
                &VerifiedNativeReadback {
                    source_path: quarantine.clone(),
                    ..verified
                },
            ),
            1
        );
        assert!(!trace.exists());
        assert!(!obsolete.exists());
    }
}

#[test]
fn replaced_recording_is_not_eligible_for_verified_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let recording = directory.path().join("recording.jsonl.zst");
    std::fs::write(&recording, b"authoritative recording").unwrap();
    let verified = VerifiedNativeReadback {
        decoded_frames: 0,
        source_path: recording.clone(),
        source_fingerprint: trace_source_fingerprint(&recording),
    };
    // Preserve the byte length so this specifically proves the content
    // digest catches a replacement that the old line-count gate missed.
    std::fs::write(&recording, b"replacement recording!!").unwrap();
    let quarantine = conversion_quarantine_path(&recording);

    assert!(
        move_verified_recording_to_quarantine(&recording, &quarantine, &verified)
            .unwrap_err()
            .contains("changed after native readback")
    );
    assert!(recording.exists());
    assert_eq!(
        std::fs::read(&recording).unwrap(),
        b"replacement recording!!"
    );
}

#[test]
fn quarantine_restore_never_overwrites_a_recreated_recording() {
    let parent = tempfile::tempdir().unwrap();
    let quarantined_path = parent.path().join("recording.parity-conversion-source");
    std::fs::write(&quarantined_path, b"quarantined replacement").unwrap();
    let canonical_path = parent.path().join("recording.jsonl.zst");
    std::fs::write(&canonical_path, b"new producer recording").unwrap();

    let error =
        restore_quarantined_recording_no_replace(&quarantined_path, &canonical_path).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read(&canonical_path).unwrap(),
        b"new producer recording"
    );
    assert_eq!(
        std::fs::read(&quarantined_path).unwrap(),
        b"quarantined replacement"
    );
}

#[test]
fn conversion_commit_never_unlinks_a_recreated_recording() {
    let directory = tempfile::tempdir().unwrap();
    let quarantine = directory
        .path()
        .join("recording.jsonl.zst.parity-conversion-source");
    let canonical = directory.path().join("recording.jsonl.zst");
    std::fs::write(&quarantine, b"verified source").unwrap();

    let conflict = commit_verified_conversion_files(&canonical, &quarantine, || {
        // This models a producer winning the pathname after the initial
        // conflict check and immediately before quarantine deletion.
        std::fs::write(&canonical, b"new producer recording").unwrap();
    })
    .unwrap_err();

    assert!(conflict.contains("new recording was preserved"));
    assert_eq!(
        std::fs::read(&canonical).unwrap(),
        b"new producer recording"
    );
    assert!(!quarantine.exists());
}

#[cfg(unix)]
#[test]
fn conversion_rejects_symlinked_logical_inputs() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target.jsonl.zst");
    let link = directory.path().join("recording.jsonl.zst");
    std::fs::write(&target, b"recording").unwrap();
    symlink(&target, &link).unwrap();
    assert!(conversion_path_is_symlink(&link));
    assert!(!conversion_path_is_symlink(&target));
    assert_eq!(std::fs::read(&target).unwrap(), b"recording");
}

#[test]
fn obsolete_derivation_cleanup_requires_a_numeric_version() {
    let prefix = "trace.jsonl.zst.parity-cache-v";
    assert!(is_obsolete_native_derivation(
        "trace.jsonl.zst.parity-cache-v64.native-bincode.zst",
        prefix
    ));
    assert!(is_obsolete_native_derivation(
        "trace.jsonl.zst.parity-cache-v9",
        prefix
    ));
    assert!(!is_obsolete_native_derivation(
        "trace.jsonl.zst.parity-cache-vicious",
        prefix
    ));
    assert!(!is_obsolete_native_derivation(
        "trace.jsonl.zst.parity-cache-v64backup",
        prefix
    ));
}

#[test]
fn trace_timeline_accepts_terminal_snapshot_without_clock_advance() {
    let mut timeline = TraceTimeline::new(9_245);
    timeline.observe(9_245, 9_246).unwrap();
    timeline.observe(9_246, 9_247).unwrap();
    timeline.observe(9_247, 9_247).unwrap();

    timeline.validate_terminator(3, 9_247).unwrap();
    assert!(
        timeline
            .validate_terminator(3, 9_248)
            .unwrap_err()
            .contains("last frame_after=9247")
    );
    assert!(
        timeline
            .validate_terminator(2, 9_247)
            .unwrap_err()
            .contains("3 frame records")
    );
}

#[test]
fn trace_timeline_rejects_gaps_rewinds_and_multi_tick_records() {
    let mut gap = TraceTimeline::new(10);
    assert!(
        gap.observe(11, 12)
            .unwrap_err()
            .contains("continue after frame 10")
    );

    let mut rewind = TraceTimeline::new(10);
    assert!(
        rewind
            .observe(10, 9)
            .unwrap_err()
            .contains("retain or advance")
    );

    let mut jump = TraceTimeline::new(10);
    assert!(
        jump.observe(10, 12)
            .unwrap_err()
            .contains("retain or advance")
    );
}

#[test]
fn retained_terminal_success_selects_the_omitted_quit_repair_only() {
    assert!(is_legacy_retained_terminal_success(
        TRACE_SCHEMA_VERSION,
        9_602,
        9_602,
        false,
        GameCode::LevelSucceeded as i32,
    ));
    assert!(!is_legacy_retained_terminal_success(
        TRACE_SCHEMA_VERSION,
        9_601,
        9_602,
        false,
        GameCode::LevelSucceeded as i32,
    ));
    assert!(!is_legacy_retained_terminal_success(
        TRACE_SCHEMA_VERSION,
        9_602,
        9_602,
        true,
        GameCode::LevelSucceeded as i32,
    ));
    assert!(!is_legacy_retained_terminal_success(
        TRACE_SCHEMA_VERSION,
        9_602,
        9_602,
        false,
        GameCode::LevelFailed as i32,
    ));
}

#[test]
fn retained_terminal_success_repair_is_emitted_exactly_once() {
    let mut commands_before_hourglass = Vec::new();
    let mut commands_after_hourglass = Vec::new();
    let mut applied = false;
    let campaign_run_id = 0x1234_5678_9abc_def0;
    assert!(append_legacy_retained_terminal_success_repair(
        &mut commands_before_hourglass,
        &mut commands_after_hourglass,
        robin_engine::player_profile::DifficultyLevel::Medium,
        campaign_run_id,
        TRACE_SCHEMA_VERSION,
        9_602,
        9_602,
        false,
        GameCode::LevelSucceeded as i32,
        &mut applied,
    ));
    assert!(!append_legacy_retained_terminal_success_repair(
        &mut commands_before_hourglass,
        &mut commands_after_hourglass,
        robin_engine::player_profile::DifficultyLevel::Medium,
        campaign_run_id,
        TRACE_SCHEMA_VERSION,
        9_602,
        9_602,
        false,
        GameCode::LevelSucceeded as i32,
        &mut applied,
    ));
    assert_eq!(commands_before_hourglass.len(), 1);
    assert_eq!(commands_after_hourglass.len(), 1);
    assert!(matches!(
        commands_before_hourglass[0],
        PlayerCommand::QuitMissionRequested
    ));
    assert!(matches!(
        commands_after_hourglass[0],
        PlayerCommand::ApplyQuitMissionUpdates {
            exit_code: GameCode::LevelSucceeded,
            campaign_run_nonce: Some(actual),
            ..
        } if actual == campaign_run_id
    ));
}

#[test]
fn legacy_presentation_sprite_rng_requires_exact_new_terminal_burst() {
    let offsets = [11, 12, 91, 91, 91, 91];
    let values = [1, 2, 3, 4, 5, 6];
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            14,
            true,
            GameCode::LevelInProgress as i32,
            true,
            3,
            false,
            &offsets,
            &values,
        ),
        Some(4),
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            4,
            false,
            &[11, 12, 91, 91, 91, 91, 91, 91],
            &[1, 2, 3, 4, 5, 6, 7, 8],
        ),
        Some(6),
    );

    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            OLDEST_SUPPORTED_TRACE_SCHEMA - 1,
            true,
            GameCode::LevelInProgress as i32,
            true,
            4,
            false,
            &offsets,
            &values,
        ),
        None
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            false,
            GameCode::LevelInProgress as i32,
            true,
            4,
            false,
            &offsets,
            &values,
        ),
        None
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInterrupted as i32,
            true,
            4,
            false,
            &offsets,
            &values,
        ),
        None
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            false,
            4,
            false,
            &offsets,
            &values,
        ),
        None
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            5,
            false,
            &offsets,
            &values,
        ),
        None
    );
}

#[test]
fn legacy_presentation_sprite_rng_rejects_ambiguous_bursts() {
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            4,
            false,
            &[91, 12, 91, 91, 91, 91],
            &[1, 2, 3, 4, 5, 6],
        ),
        None
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            4,
            false,
            &[91, 91, 92, 91, 91, 91],
            &[1, 2, 3, 4, 5, 6],
        ),
        None
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            4,
            false,
            &[91, 91, 91, 91],
            &[1, 2, 3, 4],
        ),
        None
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            0,
            false,
            &[11, 12],
            &[1, 2],
        ),
        None
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            3,
            false,
            &[11, 12, 91, 91, 91, 91],
            &[1, 2, 3, 4, 3, 6],
        ),
        None
    );
}

#[test]
fn legacy_mobile_vibration_rng_accepts_only_new_terminal_xy_pairs() {
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            14,
            false,
            GameCode::LevelInProgress as i32,
            true,
            3,
            false,
            &[11, 12, 71, 72, 71, 72, 71, 72],
            &[1, 2, 3, 4, 5, 6, 7, 8],
        ),
        Some(6),
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            3,
            false,
            &[71, 12, 71, 72, 71, 72, 71, 72],
            &[1, 2, 3, 4, 5, 6, 7, 8],
        ),
        None,
        "either X/Y site appearing in the gameplay prefix is ambiguous",
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            3,
            false,
            &[11, 72, 71, 72, 71, 72, 71, 72],
            &[1, 2, 3, 4, 5, 6, 7, 8],
        ),
        None,
        "either X/Y site appearing in the gameplay prefix is ambiguous",
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            3,
            false,
            &[11, 12, 71, 72],
            &[1, 2, 3, 4],
        ),
        None,
        "one X/Y pair is not a repeated mobile-child burst",
    );
}

#[test]
fn legacy_presentation_sprite_rng_requires_an_exact_unconsumed_suffix() {
    assert_eq!(
        missing_legacy_presentation_sprite_rng_draws(Some(13), 160, 173),
        Some(13)
    );
    assert_eq!(
        missing_legacy_presentation_sprite_rng_draws(Some(13), 173, 173),
        None,
        "a gameplay tick which consumed the homogeneous run must not replay it as presentation RNG"
    );
    assert_eq!(
        missing_legacy_presentation_sprite_rng_draws(Some(13), 161, 173),
        None,
        "a partially unmatched candidate is ambiguous"
    );
    assert_eq!(
        missing_legacy_presentation_sprite_rng_draws(Some(13), 174, 173),
        None,
        "an over-consuming tick remains an ordinary RNG mismatch"
    );
}

#[test]
fn legacy_repeated_arrow_refresh_requires_a_previously_correlated_callsite() {
    let known = BTreeSet::from([71]);
    assert_eq!(
        legacy_additional_arrow_refresh_draws(
            TRACE_SCHEMA_VERSION,
            &[71, 71, 71, 71, 83],
            &known,
            1,
        ),
        Some(3),
    );
    assert_eq!(
        legacy_additional_arrow_refresh_draws(TRACE_SCHEMA_VERSION, &[71, 71, 83], &known, 2,),
        None,
        "two retained falling arrows explain one two-draw refresh",
    );
    assert_eq!(
        legacy_additional_arrow_refresh_draws(TRACE_SCHEMA_VERSION, &[72, 72, 72, 83], &known, 1,),
        None,
        "an uncorrelated homogeneous gameplay prefix is not presentation evidence",
    );
    assert_eq!(
        legacy_additional_arrow_refresh_draws(TRACE_SCHEMA_VERSION, &[71, 71, 83], &known, 0,),
        None,
        "a trace callsite cannot manufacture a missing falling arrow",
    );
}

#[test]
fn legacy_teleport_star_rng_requires_exact_retained_lifecycle_and_ten_draws() {
    let state = |kind, index, creation_order, active, x| LegacyPresentationEntityState {
        entity_id: TraceEntityId { kind, index },
        creation_order,
        kind,
        active,
        position_bits: [x, 0],
    };
    let previous = [
        state(TraceEntityKind::Pc, 130, 130, false, 10),
        state(TraceEntityKind::Target, 223, 223, true, 20),
        state(TraceEntityKind::Pc, 7, 7, true, 30),
        state(TraceEntityKind::Bonus, 234, 234, false, 40),
    ];
    let current = [
        state(TraceEntityKind::Pc, 130, 130, true, 11),
        state(TraceEntityKind::Target, 223, 223, false, 20),
        state(TraceEntityKind::Pc, 7, 7, true, 30),
        state(TraceEntityKind::Bonus, 234, 234, true, 40),
    ];
    assert!(has_legacy_teleport_star_lifecycle(
        Some(&previous),
        &current
    ));

    let offsets = [11, 12, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91];
    let values = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            29,
            true,
            &offsets,
            &values,
        ),
        Some(10)
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            29,
            false,
            &offsets,
            &values,
        ),
        None
    );
    assert_eq!(
        legacy_presentation_sprite_rng_burst(
            TRACE_SCHEMA_VERSION,
            true,
            GameCode::LevelInProgress as i32,
            true,
            29,
            true,
            &offsets[..11],
            &values[..11],
        ),
        None
    );

    let mut pc_did_not_move = current;
    pc_did_not_move[0].position_bits = previous[0].position_bits;
    assert!(!has_legacy_teleport_star_lifecycle(
        Some(&previous),
        &pc_did_not_move
    ));
    let mut unrelated_transition = current;
    unrelated_transition[2].active = false;
    assert!(!has_legacy_teleport_star_lifecycle(
        Some(&previous),
        &unrelated_transition
    ));
    assert!(!has_legacy_teleport_star_lifecycle(None, &current));
    assert!(!has_legacy_teleport_star_lifecycle(
        Some(&previous[..2]),
        &current
    ));
}

#[test]
fn replay_campaign_identity_is_stable_across_recording_family_sessions() {
    let first = replay_campaign_run_id(
        Path::new("interactive-session-002-session-0001.jsonl.zst"),
        1,
    );
    let ninth = replay_campaign_run_id(
        Path::new("interactive-session-002-session-0009.jsonl.zst"),
        9,
    );
    let other = replay_campaign_run_id(
        Path::new("interactive-session-003-session-0001.jsonl.zst"),
        1,
    );

    assert_ne!(first, 0);
    assert_eq!(first, ninth);
    assert_eq!(
        first,
        replay_campaign_run_id(
            Path::new("interactive-session-002-session-0001.jsonl.zst.parity.bitcode.zst"),
            1,
        )
    );
    assert_ne!(first, other);
}

#[test]
fn fixed_native_footer_rejects_early_end_and_trailing_records() {
    let footer = BinaryTraceFooter {
        version: TRACE_NATIVE_VERSION,
        frame_count: 2,
        final_frame: 12,
    };
    let early = write_test_native_records(&[complete_test_end(1, 11)], Some(footer));
    let mut reader = BinaryTraceReader::open(early.path());
    assert!(matches!(
        reader.read_record(),
        BinaryTraceRecord::End { .. }
    ));
    assert!(
        reader
            .validate_terminator(1, 11)
            .unwrap_err()
            .contains("fixed footer says frame_count=2")
    );

    let footer = BinaryTraceFooter {
        version: TRACE_NATIVE_VERSION,
        frame_count: 0,
        final_frame: 10,
    };
    let trailing = write_test_native_records(
        &[complete_test_end(0, 10), complete_test_end(0, 10)],
        Some(footer),
    );
    let mut reader = BinaryTraceReader::open(trailing.path());
    assert!(matches!(
        reader.read_record(),
        BinaryTraceRecord::End { .. }
    ));
    assert!(
        reader
            .validate_terminator(0, 10)
            .unwrap_err()
            .contains("first End")
    );

    // Same trailing record, but inside the End's own block.
    let footer = BinaryTraceFooter {
        version: TRACE_NATIVE_VERSION,
        frame_count: 0,
        final_frame: 10,
    };
    let mut file = tempfile::NamedTempFile::new().unwrap();
    {
        let mut encoder = zstd::stream::write::Encoder::new(
            BufWriter::new(file.as_file_mut()),
            TRACE_NATIVE_ZSTD_LEVEL,
        )
        .unwrap();
        // Coerce to a slice: bitcode encodes fixed-size arrays without a
        // length, which would not decode as the reader's `Vec` blocks.
        write_binary_record(
            &mut encoder,
            [complete_test_end(0, 10), complete_test_end(0, 10)].as_slice(),
            "test native trace block",
        );
        let mut writer = encoder.finish().unwrap();
        write_binary_trace_footer(&mut writer, footer).unwrap();
        writer.flush().unwrap();
    }
    let mut reader = BinaryTraceReader::open(file.path());
    assert!(matches!(
        reader.read_record(),
        BinaryTraceRecord::End { .. }
    ));
    assert!(
        reader
            .validate_terminator(0, 10)
            .unwrap_err()
            .contains("first End")
    );
}

#[test]
fn fixed_native_footer_rejects_missing_and_malformed_data() {
    let missing = write_test_native_records(&[complete_test_end(0, 10)], None);
    let missing_error = read_binary_trace_footer(missing.path()).unwrap_err();
    assert!(
        missing_error.contains("footer magic") || missing_error.contains("shorter than"),
        "unexpected missing-footer error: {missing_error}"
    );

    let mut malformed = tempfile::NamedTempFile::new().unwrap();
    malformed
        .write_all(&vec![0_u8; TRACE_NATIVE_FOOTER_LEN as usize])
        .unwrap();
    assert!(
        read_binary_trace_footer(malformed.path())
            .unwrap_err()
            .contains("footer magic")
    );
}

#[test]
fn movement_and_flight_steps_preserve_exact_operands() {
    let empty: TraceFrame = serde_json::from_value(minimal_frame_json()).unwrap();
    assert!(empty.movement_steps.is_empty());
    assert!(empty.flight_steps.is_empty());

    let mut instrumented = minimal_frame_json();
    instrumented["movement_steps"] = serde_json::json!([{
        "entity": { "kind": "pc", "index": 344 },
        "order_id": 91,
        "order_action": 303,
        "animation": 17,
        "motion_method": 2,
        "pre_position": {
            "x": { "bits": 0x44a1_0001_u32 },
            "y": { "bits": 0x4480_0002_u32 }
        },
        "old_position": {
            "x": { "bits": 0x44a0_0003_u32 },
            "y": { "bits": 0x447f_0004_u32 }
        },
        "goal": {
            "x": { "bits": 0x44b0_0005_u32 },
            "y": { "bits": 0x4490_0006_u32 }
        },
        "cached_increment": {
            "x": { "bits": 0x3f00_0007_u32 },
            "y": { "bits": 0x3f40_0008_u32 }
        },
        "frame_distance_raw": { "bits": 0x4000_0009_u32 },
        "speed_factor": { "bits": 0x3f80_000a_u32 },
        "effective_distance": { "bits": 0x4000_000b_u32 },
        "anti_collision": true,
        "reverse": false,
        "raw_post_position": {
            "x": { "bits": 0x44a1_800c_u32 },
            "y": { "bits": 0x4480_800d_u32 }
        },
        "raw_committed_delta": {
            "x": { "bits": 0x3f00_0010_u32 },
            "y": { "bits": 0x3e80_0011_u32 }
        },
        "post_position": {
            "x": { "bits": 0x44a2_000c_u32 },
            "y": { "bits": 0x4481_000d_u32 }
        },
        "committed_delta": {
            "x": { "bits": 0x3f80_000e_u32 },
            "y": { "bits": 0x3f00_000f_u32 }
        },
        "goal_reached": true,
        "snapped_to_goal": true
    }]);
    instrumented["flight_steps"] = serde_json::json!([{
        "entity": { "kind": "soldier", "index": 102 },
        "order_id": 1056453,
        "order_action": 303,
        "animation": 17,
        "flight_style": 0,
        "entry_position": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 }, "z": { "bits": 0_u32 } },
        "entry_position_map": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 } },
        "old_position": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 }, "z": { "bits": 0_u32 } },
        "old_position_map": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 } },
        "goal": { "x": { "bits": 0x4448_4eb3_u32 }, "y": { "bits": 0x4505_7804_u32 }, "z": { "bits": 0_u32 } },
        "cached_increment": { "x": { "bits": 0_u32 }, "y": { "bits": 0_u32 }, "z": { "bits": 0_u32 } },
        "applied_increment": { "x": { "bits": 0_u32 }, "y": { "bits": 0_u32 }, "z": { "bits": 0_u32 } },
        "raw_post_position": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 }, "z": { "bits": 0_u32 } },
        "raw_post_position_map": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 } },
        "motion_state": 3,
        "post_position": { "x": { "bits": 0x4448_4eb3_u32 }, "y": { "bits": 0x4505_7804_u32 }, "z": { "bits": 0_u32 } },
        "post_position_map": { "x": { "bits": 0x4448_4eb3_u32 }, "y": { "bits": 0x4505_7804_u32 } },
        "snapped_to_goal": true
    }]);
    let parsed: TraceFrame = serde_json::from_value(instrumented).unwrap();
    let step = parsed
        .movement_steps
        .first()
        .expect("instrumented frame retains its movement step");
    assert_eq!(step.entity.index, 344);
    assert_eq!(step.order_id, 91);
    assert_eq!(step.pre_position.x.bits, 0x44a1_0001);
    assert_eq!(step.cached_increment.y.bits, 0x3f40_0008);
    assert_eq!(step.effective_distance.bits, 0x4000_000b);
    assert_eq!(step.raw_post_position.x.bits, 0x44a1_800c);
    assert_eq!(step.committed_delta.y.bits, 0x3f00_000f);
    assert!(step.goal_reached);
    assert!(step.snapped_to_goal);
    let flight = parsed
        .flight_steps
        .first()
        .expect("instrumented frame retains its flight step");
    assert_eq!(flight.entity.index, 102);
    assert_eq!(flight.raw_post_position_map.x.bits, 0x4448_4eb2);
    assert_eq!(flight.post_position_map.x.bits, 0x4448_4eb3);
    assert!(flight.snapped_to_goal);

    let encoded = bitcode::encode(&parsed);
    let roundtrip: TraceFrame =
        bitcode::decode(&encoded).expect("decode instrumented frame with the cache layout");
    assert_eq!(roundtrip.movement_steps.len(), 1);
    assert_eq!(roundtrip.movement_steps[0].entity.index, 344);
    assert_eq!(
        roundtrip.movement_steps[0].raw_committed_delta.x.bits,
        0x3f00_0010
    );
    assert_eq!(roundtrip.flight_steps.len(), 1);
    assert_eq!(roundtrip.flight_steps[0].entity.index, 102);
    assert_eq!(
        roundtrip.flight_steps[0].post_position_map.x.bits,
        0x4448_4eb3
    );
}

#[test]
fn sword_seek_distance_defaults_legacy_records_and_direct_distance_is_ignored() {
    let missing = serde_json::json!({
        "type": "sword_strike",
        "actor": { "kind": "pc", "index": 3 },
        "target": { "kind": "soldier", "index": 7 },
        "original_command": 78,
        "original_command_name": "swordstrike_thrust_a",
        "with_seek": true
    });
    let missing: TraceCommand = serde_json::from_value(missing)
        .expect("legacy sword-strike command may omit seek distance");
    let TraceCommand::SwordStrike { seek_distance, .. } = missing else {
        panic!("decoded wrong trace command")
    };
    assert!(seek_distance.is_nan());

    assert_eq!(trace_sword_seek_distance(false, 0.0), None);
    assert_eq!(trace_sword_seek_distance(true, seek_distance), None);
    assert_eq!(trace_sword_seek_distance(true, 63.0), Some(63.0));
}

#[test]
fn trace_content_hash_distinguishes_equal_length_sources() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.jsonl.zst");
    let second = directory.path().join("second.jsonl.zst");
    std::fs::write(&first, b"equal-length-a").unwrap();
    std::fs::write(&second, b"equal-length-b").unwrap();

    assert_eq!(
        std::fs::metadata(&first).unwrap().len(),
        std::fs::metadata(&second).unwrap().len()
    );
    assert_ne!(trace_content_sha256(&first), trace_content_sha256(&second));
    assert!(trace_source_fingerprint(&first).contains(":sha256="));
}

fn valid_initial_save_with_profile(source_profile: TraceSaveSourceProfile) -> TraceInitialSave {
    let mut bytes = Vec::from(*source_profile.expected_magic());
    bytes.extend_from_slice(&48_u32.to_le_bytes());
    bytes.extend_from_slice(&16_723_u32.to_le_bytes());
    bytes.extend_from_slice(&48_u32.to_le_bytes());
    bytes.extend_from_slice(b"serialized save body");
    TraceInitialSave {
        format: "rhsg".to_owned(),
        source_profile,
        encoding: "base64".to_owned(),
        byte_length: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
        slot: "Restart".to_owned(),
        header_version: 48,
        mission_id: 16_723,
        stream_version: 48,
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

fn valid_initial_save() -> TraceInitialSave {
    valid_initial_save_with_profile(TraceSaveSourceProfile::LinuxI386RhsgV48)
}

#[test]
fn jsonl_trace_reader_accepts_plain_and_zstd_content() {
    let jsonl = b"{\"type\":\"header\"}\n{\"type\":\"rng_prefix\"}\n";
    let directory = tempfile::tempdir().unwrap();
    let plain_path = directory.path().join("trace.jsonl");
    let compressed_path = directory.path().join("trace.jsonl.zst");
    std::fs::write(&plain_path, jsonl).unwrap();
    std::fs::write(
        &compressed_path,
        zstd::stream::encode_all(std::io::Cursor::new(jsonl), 1).unwrap(),
    )
    .unwrap();

    for path in [plain_path, compressed_path] {
        let mut decoded = String::new();
        open_jsonl_trace(&path)
            .read_to_string(&mut decoded)
            .unwrap();
        assert_eq!(decoded.as_bytes(), jsonl);
    }
}

#[test]
fn jsonl_trace_reader_accepts_zstd_frames_with_large_declared_windows() {
    let jsonl = b"{\"type\":\"header\"}\n{\"type\":\"rng_prefix\"}\n";
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    encoder.window_log(28).unwrap();
    encoder.write_all(jsonl).unwrap();
    let compressed = encoder.finish().unwrap();

    // Ensure this fixture exercises a frame rejected by zstd's default
    // 128 MiB window cap rather than merely duplicating the ordinary-zstd
    // coverage above.
    let mut default_decoder =
        zstd::stream::read::Decoder::new(std::io::Cursor::new(&compressed)).unwrap();
    let mut default_output = Vec::new();
    assert!(default_decoder.read_to_end(&mut default_output).is_err());

    let directory = tempfile::tempdir().unwrap();
    let compressed_path = directory.path().join("large-window-trace.jsonl.zst");
    std::fs::write(&compressed_path, compressed).unwrap();

    let mut decoded = String::new();
    open_jsonl_trace(&compressed_path)
        .read_to_string(&mut decoded)
        .unwrap();
    assert_eq!(decoded.as_bytes(), jsonl);
}

#[test]
fn compression_window_is_bounded_for_parallel_replay_lanes() {
    // Unknown and sufficiently large streams use the 64 MiB maximum;
    // known smaller streams retain only the power-of-two window they need.
    assert_eq!(native_stream_window_log(None), TRACE_NATIVE_WINDOW_LOG);
    assert_eq!(native_stream_window_log(Some(32 * 1024 * 1024)), 25);
    assert_eq!(native_stream_window_log(Some(64 * 1024 * 1024)), 26);
    assert_eq!(
        native_stream_window_log(Some(64 * 1024 * 1024 + 1)),
        TRACE_NATIVE_WINDOW_LOG
    );
    // Tiny estimates retain the encoder's accepted minimum.
    assert_eq!(native_stream_window_log(Some(0)), 20);
    assert_eq!(native_stream_window_log(Some(1)), 20);
    assert_eq!(
        native_stream_window_log(Some(u64::MAX)),
        TRACE_NATIVE_WINDOW_LOG
    );
    assert_eq!(
        native_stream_window_log_capped(None, TRACE_NATIVE_MIN_WINDOW_LOG),
        TRACE_NATIVE_MIN_WINDOW_LOG
    );
    assert_eq!(TRACE_NATIVE_BLOCK_RECORDS, 32);
}

#[test]
fn recording_size_comes_from_the_zstd_frame_when_the_recording_is_compressed() {
    let directory = tempfile::tempdir().unwrap();
    let jsonl = vec![b'{'; 4096];

    let plain = directory.path().join("plain.jsonl");
    std::fs::write(&plain, &jsonl).unwrap();
    assert_eq!(recording_uncompressed_bytes(&plain), Some(4096));

    // A recording compressed as a single frame declares its content size,
    // which is what the window has to be sized against -- the file length
    // would size the window against the compressed bytes instead.
    let compressed = directory.path().join("declared.jsonl.zst");
    std::fs::write(&compressed, zstd::bulk::compress(&jsonl, 1).unwrap()).unwrap();
    assert!(std::fs::metadata(&compressed).unwrap().len() < 4096);
    assert_eq!(recording_uncompressed_bytes(&compressed), Some(4096));

    // A streamed frame may not declare one; the caller then uses the
    // bounded replay-memory window rather than guessing from file size.
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    encoder.write_all(&jsonl).unwrap();
    let streamed = directory.path().join("undeclared.jsonl.zst");
    std::fs::write(&streamed, encoder.finish().unwrap()).unwrap();
    assert_eq!(recording_uncompressed_bytes(&streamed), None);
    assert_eq!(
        native_stream_window_log(expected_native_stream_bytes(&streamed)),
        TRACE_NATIVE_WINDOW_LOG
    );
}

#[test]
fn all_existing_trace_schemas_are_accepted() {
    for schema in OLDEST_SUPPORTED_TRACE_SCHEMA..=TRACE_SCHEMA_VERSION {
        validate_trace_schema(schema);
        assert!(trace_schema_is_supported(schema));
    }
    assert!(!trace_schema_is_supported(
        OLDEST_SUPPORTED_TRACE_SCHEMA - 1
    ));
    assert!(!trace_schema_is_supported(TRACE_SCHEMA_VERSION + 1));
}

#[test]
fn current_door_and_route_diagnostics_are_typed() {
    let actor: TraceActor = serde_json::from_value(serde_json::json!({
        "action_state": 1,
        "animation": 12,
        "command": 19,
        "command_name": "pass_door",
        "motion_state": 2,
        "wait_time": 0,
        "passing_door_directly": true,
        "active_pass_door": { "gate_id": 51, "direct": true, "direction": 1 },
        "sequence_element": {
            "id": 160,
            "type": 4,
            "state": 2,
            "command_level": 7,
            "command": 19,
            "command_name": "pass_door",
            "order_count": 3,
            "priority": 8,
            "posture_after_transition": 1,
            "action_state_after_transition": 1,
            "movement": {
                "action": 12,
                "pass_door": { "gate_id": 51, "direct": true, "direction": 1 }
            },
            "following": null,
            "postponed": null,
            "current_order": null,
            "movement_payload": null
        },
        "position_interface": {}
    }))
    .expect("parse current-schema actor diagnostics");
    assert!(actor.passing_door_directly);
    assert_eq!(
        actor
            .active_pass_door
            .as_ref()
            .map(|pass| (pass.gate_id, pass.direction)),
        Some((51, 1))
    );
    let pass = actor.active_pass_door.as_ref().unwrap();
    assert!(active_pass_door_keys_match(Some(pass), Some((51, true))));
    assert!(!active_pass_door_keys_match(Some(pass), Some((51, false))));
    assert!(!active_pass_door_keys_match(Some(pass), None));
    assert_eq!(
        actor
            .sequence_element
            .as_ref()
            .map(|element| (element.id, element.order_count)),
        Some((160, 3))
    );

    let mut frame_json = minimal_frame_json();
    frame_json["route_construction_events"] = serde_json::json!([{
        "kind": "move",
        "actor": { "kind": "soldier", "index": 43 },
        "source": { "x": { "bits": 1154109440_u32 }, "y": { "bits": 1153748992_u32 } },
        "source_sector": 61,
        "source_level": 11,
        "goal": { "x": { "bits": 1155563520_u32 }, "y": { "bits": 1147920384_u32 } },
        "goal_sector": 72,
        "goal_level": 11,
        "gates": [{
            "gate_id": 51,
            "direct": false,
            "sector_out": 60,
            "level_out": 11,
            "sector_in": 61,
            "level_in": 11
        }]
    }]);
    let frame: TraceFrame = serde_json::from_value(frame_json)
        .expect("parse current-schema route-construction diagnostics");
    validate_trace_frame(TRACE_SCHEMA_VERSION, &frame);
    let route = &frame.route_construction_events[0];
    assert_eq!(route.actor.index, 43);
    assert_eq!(route.gates[0].gate_id, 51);
    assert!(!route.gates[0].direct);
}

#[test]
fn current_schema_jump_lines_are_typed_parallel_and_cache_safe() {
    let human: TraceHuman = serde_json::from_value(serde_json::json!({
        "life_points": 40,
        "dead": false,
        "unconscious": false,
        "camp": "lacklandists",
        "original_camp": 1,
        "vip": false,
        "civilian": false,
        "opponents": [{"kind": "pc", "index": 3}],
        "opponent_jump_lines": [{
            "a": {"x": {"bits": 1065353216}, "y": {"bits": 1073741824}},
            "b": {"x": {"bits": 1077936128}, "y": {"bits": 1082130432}}
        }]
    }))
    .expect("parse schema-16 opponent jump-line geometry");
    let entity: TraceEntityId = serde_json::from_value(serde_json::json!({
        "kind": "soldier", "index": 8
    }))
    .unwrap();
    validate_human_jump_line_shape(&entity, &human, false);

    let ai: TraceAi = serde_json::from_value(serde_json::json!({
        "state": 1,
        "substate": 2,
        "script_locked": false,
        "locked": false,
        "locks": 0,
        "was_busy": false,
        "very_busy": false,
        "macro_timer_running": false,
        "macro_timer_ring": 0,
        "macro_cursor": null,
        "macro_remaining": 0,
        "macro_in_progress": false,
        "list_us": [],
        "list_them": [],
        "my_line_jump": null
    }))
    .expect("parse authoritative null soldier jump line");
    assert!(ai.my_line_jump.is_none());

    let encoded = bitcode::encode(&(human, ai));
    let (cached_human, cached_ai): (TraceHuman, TraceAi) =
        bitcode::decode(&encoded).expect("restore typed jump-line snapshots");
    assert_eq!(cached_human.opponent_jump_lines.len(), 1);
    assert!(cached_ai.my_line_jump.is_none());

    let malformed_line = serde_json::json!({
        "a": {"x": {"bits": 0}, "y": {"bits": 0}},
        "b": {"x": {"bits": 0}, "y": {"bits": 0}},
        "pointer": 123
    });
    assert!(serde_json::from_value::<TraceJumpLine>(malformed_line).is_err());
}

#[test]
#[should_panic(expected = "opponent and jump-line arrays differ in length")]
fn current_schema_rejects_misaligned_opponent_jump_lines() {
    let human: TraceHuman = serde_json::from_value(serde_json::json!({
        "life_points": 40,
        "dead": false,
        "unconscious": false,
        "camp": "lacklandists",
        "original_camp": 1,
        "vip": false,
        "civilian": false,
        "opponents": [{"kind": "pc", "index": 3}],
        "opponent_jump_lines": []
    }))
    .unwrap();
    let entity: TraceEntityId = serde_json::from_value(serde_json::json!({
        "kind": "soldier", "index": 8
    }))
    .unwrap();
    validate_human_jump_line_shape(&entity, &human, false);
}

#[test]
fn legacy_schema_accepts_omitted_opponent_jump_lines() {
    let human: TraceHuman = serde_json::from_value(serde_json::json!({
        "life_points": 40,
        "dead": false,
        "unconscious": false,
        "camp": "lacklandists",
        "original_camp": 1,
        "vip": false,
        "civilian": false,
        "opponents": [{"kind": "pc", "index": 3}],
        "opponent_jump_lines": []
    }))
    .unwrap();
    let entity: TraceEntityId = serde_json::from_value(serde_json::json!({
        "kind": "soldier", "index": 8
    }))
    .unwrap();

    validate_human_jump_line_shape(&entity, &human, true);
}

#[test]
fn current_schema_event_window_resets_only_after_record_frame() {
    let mut pending = Vec::<(u64, &'static str)>::new();
    let mut next_ordinal = 0_u64;
    fn emit(pending: &mut Vec<(u64, &'static str)>, next_ordinal: &mut u64, stage: &'static str) {
        pending.push((*next_ordinal, stage));
        *next_ordinal += 1;
    }

    emit(&mut pending, &mut next_ordinal, "input_before_hourglass");
    emit(&mut pending, &mut next_ordinal, "simulation_body");
    let first_frame = std::mem::take(&mut pending);
    next_ordinal = 0; // Frame recording serialized and then reset the window.

    emit(
        &mut pending,
        &mut next_ordinal,
        "refresh_after_record_frame",
    );
    // Starting an engine frame intentionally does not clear or reset event state.
    emit(
        &mut pending,
        &mut next_ordinal,
        "next_input_before_hourglass",
    );
    emit(&mut pending, &mut next_ordinal, "next_simulation_body");
    let second_frame = std::mem::take(&mut pending);

    assert_eq!(
        first_frame,
        vec![(0, "input_before_hourglass"), (1, "simulation_body")]
    );
    assert_eq!(
        second_frame,
        vec![
            (0, "refresh_after_record_frame"),
            (1, "next_input_before_hourglass"),
            (2, "next_simulation_body")
        ]
    );
}

#[test]
fn current_schema_target_unreached_observations_are_explicitly_nullable() {
    let event: TraceTargetLifecycleEvent = serde_json::from_value(serde_json::json!({
        "ordinal": 0,
        "frame_ordinal": 0,
        "phase": "engine_send_message_entry",
        "sequence_id": null,
        "sequence_element_id": 2,
        "command_level": 1,
        "state": 0,
        "command": 15,
        "command_name": "send_message",
        "owner": null,
        "context": null,
        "antagonist": null,
        "antagonist_observed": null,
        "payload": {
            "kind": "send_message",
            "message": null,
            "argument": null,
            "argument_raw": null,
            "extended_argument": null,
            "extended_argument_raw": null
        },
        "payload_observed": false,
        "script_enabled": false,
        "class_instantiated": null
    }))
    .expect("parse unreached schema-16 target evidence");
    assert!(event.sequence_id.is_none());
    assert_eq!(event.payload_observed, Some(false));
    assert!(matches!(
        event.payload,
        TraceTargetLifecyclePayload::SendMessage {
            message: None,
            argument: None,
            ..
        }
    ));
    let encoded = bitcode::encode(&event);
    let cached: TraceTargetLifecycleEvent = bitcode::decode(&encoded).unwrap();
    assert!(cached.sequence_id.is_none());
    assert_eq!(cached.payload_observed, Some(false));
}

#[test]
fn current_schema_movement_action_is_only_present_when_initialized() {
    let sequence = |command: u16,
                    command_name: &str,
                    movement: serde_json::Value,
                    movement_payload: serde_json::Value| {
        serde_json::from_value::<TraceSequenceElement>(serde_json::json!({
            "id": 5,
            "type": 4,
            "state": 2,
            "command_level": 1,
            "command": command,
            "command_name": command_name,
            "order_count": 1,
            "priority": 0,
            "posture_after_transition": 0,
            "action_state_after_transition": 0,
            "movement": movement,
            "following": null,
            "postponed": null,
            "current_order": {"id": 77, "action": 0},
            "movement_payload": movement_payload
        }))
        .expect("parse schema-16 movement constructor-shaped evidence")
    };

    let wait_free_lift = sequence(
        0,
        "wait_free_lift",
        serde_json::json!({}),
        serde_json::json!({
            "speed_factor": {"bits": 1065353216, "value": 1.0},
            "destination": {"x": {"bits": 0}, "y": {"bits": 0}},
            "tolerance": {"bits": 0, "value": 0.0},
            "flags": 0,
            "target": null,
            "sector": null
        }),
    );
    assert_eq!(
        wait_free_lift
            .movement
            .as_ref()
            .expect("movement evidence")
            .action,
        None
    );
    assert!(
        wait_free_lift
            .movement_payload
            .as_ref()
            .expect("movement payload")
            .to_json()
            .get("action")
            .is_none()
    );

    let movement = sequence(
        1,
        "move",
        serde_json::json!({"action": 12}),
        serde_json::json!({
            "speed_factor": {"bits": 1065353216, "value": 1.0},
            "action": 12,
            "destination": {"x": {"bits": 0}, "y": {"bits": 0}},
            "tolerance": {"bits": 0, "value": 0.0},
            "flags": 3,
            "target": null,
            "sector": null
        }),
    );
    assert_eq!(
        movement
            .movement
            .as_ref()
            .expect("movement evidence")
            .action,
        Some(12)
    );
    assert_eq!(
        movement
            .movement_payload
            .as_ref()
            .expect("movement payload")
            .to_json()
            .get("action"),
        Some(&serde_json::json!(12))
    );

    let encoded = bitcode::encode(&[wait_free_lift, movement]);
    let cached: [TraceSequenceElement; 2] = bitcode::decode(&encoded).unwrap();
    assert_eq!(cached[0].movement.as_ref().unwrap().action, None);
    assert_eq!(cached[1].movement.as_ref().unwrap().action, Some(12));
}

#[test]
fn current_actor_sequence_diagnostics_are_typed() {
    validate_trace_schema(TRACE_SCHEMA_VERSION);

    let actor: TraceActor = serde_json::from_value(serde_json::json!({
        "action_state": 1,
        "animation": 12,
        "command": 19,
        "command_name": "pass_door",
        "motion_state": 2,
        "wait_time": 0,
        "passing_door_directly": true,
        "active_pass_door": null,
        "position_interface": {
            "move_box": {
                "top_left": {"x": {"bits": 0}, "y": {"bits": 0}},
                "bottom_right": {"x": {"bits": 0}, "y": {"bits": 0}}
            },
            "anti_collision_on": true,
            "deviated": false,
            "blocked_count": 0,
            "box_blocked": {
                "top_left": {"x": {"bits": 0}, "y": {"bits": 0}},
                "bottom_right": {"x": {"bits": 0}, "y": {"bits": 0}}
            },
            "radius": {"bits": 1065353216, "value": 1.0}
        },
        "sequence_element": {
            "id": 5,
            "type": 4,
            "state": 2,
            "command_level": 7,
            "command": 19,
            "command_name": "pass_door",
            "order_count": 1,
            "priority": 8,
            "posture_after_transition": 1,
            "action_state_after_transition": 1,
            "movement": {"action": 12},
            "following": null,
            "postponed": {"id": 6, "command": 2},
            "current_order": {"id": 77, "action": 12},
            "movement_payload": {"flags": 3, "speed_factor": {"bits": 1065353216, "value": 1.0}}
        }
    }))
    .expect("parse current actor and sequence diagnostics");
    let sequence = actor.sequence_element.as_ref().unwrap();
    assert!(sequence.current_order.is_some());
    assert!(sequence.movement_payload.is_some());
}

fn diagnostic_frame_fixture() -> TraceFrame {
    let mut frame = minimal_frame_json();
    let diagnostics: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test-support/parity/current_diagnostics.json"
        )))
        .expect("parse diagnostic fixture");
    frame.as_object_mut().unwrap().extend(diagnostics);
    serde_json::from_value(frame).expect("parse current-schema diagnostic streams")
}

#[test]
fn current_diagnostic_streams_preserve_typed_fields() {
    let frame = diagnostic_frame_fixture();
    validate_trace_frame(TRACE_SCHEMA_VERSION, &frame);
    assert_eq!(frame.popup_events.len(), 1);
    assert_eq!(frame.popup_events[0].ordinal, Some(0));
    assert_eq!(
        frame.ai_forecast_events[0].phase.as_deref(),
        Some("resolved")
    );
    assert_eq!(frame.alert_formation_events[0].ordinal, Some(0));
    let authorizations = &frame.goto_authorization_events;
    assert_eq!(authorizations[0].source.as_ref().unwrap().layer, 2);
    assert!(authorizations[0].move_box.is_some());
    assert!(authorizations[1].source.is_none());
    assert!(authorizations[1].move_box.is_none());
    let target = &frame.target_lifecycle_events[0];
    assert_eq!(target.sequence_id, Some(701));
    assert_eq!(target.frame_ordinal, Some(6));
    assert_eq!(target.command_level, 2);
    assert!(target.owner.is_none());
    assert!(matches!(
        target.payload,
        TraceTargetLifecyclePayload::SendMessage {
            argument: Some(-1),
            argument_raw: Some(u32::MAX),
            ..
        }
    ));
    let proposals = &frame.strike_proposal_events;
    assert_eq!(proposals[0].actor_creation_order, 373);
    assert_eq!(proposals[0].also_parade, Some(true));
    assert_eq!(proposals[1].only_parade, Some(true));
    assert_eq!(proposals[1].command_name.as_deref(), Some("parry_sword"));
    assert_eq!(
        proposals[2].actor.as_ref().unwrap().kind,
        TraceEntityKind::Soldier
    );
    assert_eq!(proposals[2].threat, None);
    assert_eq!(proposals[3].accepted_as_best, Some(true));
    let lifecycle = &frame.sequence_lifecycle_events;
    assert_eq!(
        lifecycle
            .iter()
            .map(|event| event.ordinal)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert_eq!(lifecycle[0].queue_size_after, Some(1));
    assert_eq!(lifecycle[0].state, None);
    assert_eq!(lifecycle[0].priority, None);
    assert_eq!(lifecycle[1].selected_sequence_id, None);
    assert_eq!(lifecycle[2].queue_size_after, Some(0));
    assert_eq!(lifecycle[3].current_order_action, Some(76));
    assert!(lifecycle[3].accepted.unwrap());
    assert_eq!(lifecycle[4].owner, None);
    assert_eq!(lifecycle[4].sequence_id, Some(701));
    assert_eq!(lifecycle[4].command_level, 2);
    assert!(
        frame.route_construction_events[0]
            .draft_diagnostics
            .contains_key("ordinal")
    );
    assert!(
        frame.route_construction_events[0].gates[0]
            .draft_diagnostics
            .contains_key("score")
    );
}

#[test]
fn current_diagnostic_streams_survive_native_cache() {
    let frame = diagnostic_frame_fixture();
    let encoded = bitcode::encode(&frame);
    let cached: TraceFrame = bitcode::decode(&encoded)
        .expect("decode current-schema frame from native cache representation");
    assert_eq!(cached.alert_formation_events.len(), 1);
    assert_eq!(cached.target_lifecycle_events[0].sequence_id, Some(701));
    let cached_authorizations = &cached.goto_authorization_events;
    assert_eq!(cached_authorizations[0].source.as_ref().unwrap().layer, 2);
    assert!(cached_authorizations[0].move_box.is_some());
    assert!(cached_authorizations[1].source.is_none());
    assert!(cached_authorizations[1].move_box.is_none());
    assert_eq!(cached.strike_proposal_events.len(), 5);
    assert_eq!(cached.sequence_lifecycle_events.len(), 5);
    assert!(
        cached.route_construction_events[0]
            .draft_diagnostics
            .contains_key("result")
    );
}

#[test]
fn current_diagnostics_reject_unrecorded_and_missing_fields() {
    let mut authoritative_state_json = minimal_frame_json();
    authoritative_state_json["campaign"] = serde_json::json!({});
    assert!(serde_json::from_value::<TraceFrame>(authoritative_state_json).is_err());

    let target_with_unknown = serde_json::json!({
        "ordinal": 0,
        "frame_ordinal": 0,
        "phase": "engine_send_message_entry",
        "sequence_id": 1,
        "sequence_element_id": 2,
        "command_level": 2,
        "state": 0,
        "command": 15,
        "command_name": "send_message",
        "owner": null,
        "context": null,
        "antagonist": null,
        "payload": {
            "kind": "send_message", "message": 10,
            "argument": -1, "argument_raw": 4294967295u64,
            "extended_argument": 0, "extended_argument_raw": 0
        },
        "script_enabled": null,
        "class_instantiated": null,
        "unstable_pointer": 123
    });
    assert!(serde_json::from_value::<TraceTargetLifecycleEvent>(target_with_unknown).is_err());

    for required in [
        "route_construction_events",
        "popup_events",
        "ai_forecast_events",
        "alert_formation_events",
        "goto_authorization_events",
        "strike_proposal_events",
        "sequence_lifecycle_events",
        "target_lifecycle_events",
    ] {
        let mut incomplete = minimal_frame_json();
        incomplete.as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<TraceFrame>(incomplete).is_err(),
            "current schema accepted a frame missing {required}"
        );
    }
}

#[test]
fn refused_action_decodes_with_and_without_a_target() {
    let with_target: TraceCommand = serde_json::from_value(serde_json::json!({
        "type": "hero_refused_action",
        "actor": {"kind": "pc", "index": 0},
        "action": "bow",
        "original_action": 1,
        "target": {"kind": "soldier", "index": 3},
        "reason": "anonymous_archer_contest",
    }))
    .expect("refused bow click decodes");
    assert!(matches!(
        with_target,
        TraceCommand::HeroRefusedAction {
            action: TraceAction::Bow,
            target: Some(_),
            ..
        }
    ));

    let without_target: TraceCommand = serde_json::from_value(serde_json::json!({
        "type": "hero_refused_action",
        "actor": {"kind": "pc", "index": 0},
        "action": "no_action",
        "original_action": 0,
        "reason": "locked_patch",
    }))
    .expect("refused patch click decodes");
    assert!(matches!(
        without_target,
        TraceCommand::HeroRefusedAction {
            action: TraceAction::NoAction,
            target: None,
            ..
        }
    ));
}

#[test]
fn initial_save_decodes_and_matches_its_rhsg_envelope() {
    let save = valid_initial_save();
    let decoded = save
        .decode_and_validate(16_723)
        .expect("valid current-schema initial_save");
    assert_eq!(&decoded[..4], b"RHSG");
    assert_eq!(u32::from_le_bytes(decoded[4..8].try_into().unwrap()), 48);
    assert_eq!(
        u32::from_le_bytes(decoded[8..12].try_into().unwrap()),
        16_723
    );
    assert_eq!(u32::from_le_bytes(decoded[12..16].try_into().unwrap()), 48);
}

#[test]
fn interactive_chain_requires_the_exact_adjacent_session_name() {
    assert_eq!(
        preceding_interactive_session_path(Path::new("chain-session-0007.jsonl.zst"), 7),
        Some(PathBuf::from("chain-session-0006.jsonl.zst"))
    );
    assert!(
        preceding_interactive_session_path(Path::new("chain-session-0001.jsonl.zst"), 1).is_none()
    );
    assert_eq!(
        preceding_interactive_session_path(
            Path::new("chain-session-0007.jsonl.zst.parity.bitcode.zst"),
            7,
        ),
        Some(PathBuf::from(
            "chain-session-0006.jsonl.zst.parity.bitcode.zst"
        ))
    );
    assert!(preceding_interactive_session_path(Path::new("unrelated.jsonl.zst"), 7).is_none());
}

#[test]
fn terminal_macro_identity_requires_active_cursor_and_unique_position() {
    use robin_engine::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};
    let waypoint = |x| RawWaypoint {
        x,
        y: 1050,
        sector: 0,
        level: 0,
        command: WaypointCommand::Macro(vec![0; 19]),
    };
    let unique = vec![RawHikingPath {
        waypoints: vec![waypoint(353)],
    }];
    assert_eq!(
        terminal_macro_waypoint_at(
            (353_f32.to_bits(), 1050_f32.to_bits()),
            Some(18),
            true,
            &unique,
        )
        .map(|(path, waypoint, offset)| (path.get(), waypoint, offset)),
        Some((0, 0, 18))
    );
    assert!(
        terminal_macro_waypoint_at(
            (353_f32.to_bits(), 1050_f32.to_bits()),
            Some(18),
            false,
            &unique,
        )
        .is_none()
    );
    let ambiguous = vec![
        RawHikingPath {
            waypoints: vec![waypoint(353)],
        },
        RawHikingPath {
            waypoints: vec![waypoint(353)],
        },
    ];
    assert!(
        terminal_macro_waypoint_at(
            (353_f32.to_bits(), 1050_f32.to_bits()),
            Some(18),
            true,
            &ambiguous,
        )
        .is_none()
    );
}

#[test]
fn windows_i386_save_preserves_and_accepts_gshr_magic() {
    let save = valid_initial_save_with_profile(TraceSaveSourceProfile::WindowsI386GshrV48);
    let decoded = save
        .decode_and_validate(16_723)
        .expect("valid Windows i386 current-schema initial_save");
    assert_eq!(&decoded[..4], b"GSHR");
}

#[test]
fn source_profile_must_match_preserved_container_magic() {
    let mut save = valid_initial_save();
    save.source_profile = TraceSaveSourceProfile::WindowsI386GshrV48;
    assert!(
        save.decode_and_validate(16_723)
            .unwrap_err()
            .contains("requires magic")
    );
}

#[test]
fn initial_save_rejects_decoded_length_mismatch() {
    let mut save = valid_initial_save();
    save.byte_length += 1;
    assert!(
        save.decode_and_validate(16_723)
            .unwrap_err()
            .contains("byte_length")
    );
}

#[test]
fn initial_save_rejects_sha256_mismatch() {
    let mut save = valid_initial_save();
    save.sha256.replace_range(0..1, "0");
    if save.sha256
        == sha256_hex(
            &base64::engine::general_purpose::STANDARD
                .decode(&save.data)
                .unwrap(),
        )
    {
        save.sha256.replace_range(0..1, "1");
    }
    assert!(
        save.decode_and_validate(16_723)
            .unwrap_err()
            .contains("sha256 mismatch")
    );
}

#[test]
fn initial_save_rejects_metadata_that_disagrees_with_rhsg_header() {
    let mut save = valid_initial_save();
    save.mission_id += 1;
    assert!(
        save.decode_and_validate(save.mission_id)
            .unwrap_err()
            .contains("disagrees with metadata")
    );
}

#[test]
fn recorded_sim_config_restores_every_authoritative_field() {
    let config = TraceSimConfig {
        difficulty: TraceDifficulty::Hard,
        script_enabled: false,
        highlander: true,
        highlander2: true,
        golden_eye: true,
        ignore_default_loose: true,
        bypass_fog_sprites_crash: true,
        amount_of_speaking: 2,
    }
    .to_sim_config(true);

    assert_eq!(
        config.difficulty,
        robin_engine::player_profile::DifficultyLevel::Hard
    );
    assert!(!config.script_enabled);
    assert!(config.highlander);
    assert!(config.highlander2);
    assert!(config.golden_eye);
    assert!(config.ignore_default_loose);
    assert!(config.bypass_fog_sprites_crash);
    assert_eq!(config.amount_of_speaking, 2);
    assert!(config.synchronous_pathfinding);
    assert!(!config.diplomacy);
    assert!(config.npc_faction_wars);
    assert!(!config.more_combat_gestures);
    assert!(!config.gesture_quality_damage);
    assert!(!config.fog_of_war);
}

fn minimal_frame_json() -> serde_json::Value {
    serde_json::json!({
        "type": "frame",
        "frame_before": 0,
        "frame_after": 1,
        "game_code": 0,
        "simulation_body_ran": true,
        "commands": [],
        "director_completions": [],
        "selected_pcs": [],
        "elements": [],
        "visibility_queries": [],
        "motion_line_changes": [],
        "path_events": [],
        "route_construction_events": [],
        "popup_events": [],
        "ai_forecast_events": [],
        "alert_formation_events": [],
        "goto_authorization_events": [],
        "strike_proposal_events": [],
        "sequence_lifecycle_events": [],
        "target_lifecycle_events": [],
        "resolved_exclamations": [],
        "movement_steps": [],
        "flight_steps": [],
        "rng_draws": {
            "first_index": 0,
            "values": [],
            "callsite_offsets": [],
            "main_thread": [],
            "domains": []
        }
    })
}

fn minimal_element_json() -> serde_json::Value {
    serde_json::json!({
        "entity_id": {"kind": "pc", "index": 1},
        "creation_order": 1,
        "class_id": 0,
        "kind": "pc",
        "active": true,
        "blipped": false,
        "unreachable": false,
        "surface_id": 0,
        "posture": 0,
        "position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "old_position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "position_goal_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "elevation": {"bits": 0},
        "old_elevation": {"bits": 0},
        "increment_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "movement_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "layer": 0,
        "layer_goal": 0,
        "sector": 0,
        "direction": 0,
        "direction_goal": 0,
        "moving": false,
        "moving_map": false,
        "sprite_row": 0,
        "sprite_frame": 0,
        "sprite_frame_count": 0,
        "runtime": {}
    })
}

#[test]
fn increment_map_valid_preserves_absent_and_explicit_false() {
    let absent: TraceElement = serde_json::from_value(minimal_element_json()).unwrap();
    assert_eq!(absent.increment_map_valid, None);

    let mut present = minimal_element_json();
    present["increment_map_valid"] = serde_json::json!(false);
    let present: TraceElement = serde_json::from_value(present).unwrap();
    assert_eq!(present.increment_map_valid, Some(false));
}

#[test]
fn cache_round_trip_audit_normalizes_floats_and_nulls_and_reports_drops() {
    // A real minimal frame passes the audit end to end.
    let line = minimal_frame_json().to_string();
    let frame: TraceFrame = serde_json::from_str(&line).unwrap();
    verify_trace_line_roundtrip(&frame, &line, 3);

    // The two declared normalizations erase identically on both sides:
    // redundant float renderings and null object entries — while null
    // array elements and bits/value-shaped data inside retained JSON
    // payloads survive untouched.
    let mut recorded = serde_json::json!({
        "elevation": {"bits": 7, "value": 1.5},
        "actor": null,
        "list": [null, {"bits": 7, "value": 1.5, "extra": 0}]
    });
    let mut typed = serde_json::json!({
        "elevation": {"bits": 7},
        "list": [null, {"bits": 7, "value": 1.5, "extra": 0}]
    });
    normalize_trace_json_for_roundtrip(&mut recorded);
    normalize_trace_json_for_roundtrip(&mut typed);
    assert_eq!(first_json_difference("$", &recorded, &typed), None);
    assert_eq!(recorded["list"][1]["value"], serde_json::json!(1.5));

    // Differences are reported with a path into the line.
    let recorded = serde_json::json!({"a": {"b": [{"c": 1, "d": 2}]}});
    let typed = serde_json::json!({"a": {"b": [{"c": 1}]}});
    assert!(
        first_json_difference("$", &recorded, &typed)
            .unwrap()
            .contains("$.a.b[0].d is dropped")
    );

    // A field that a lenient struct silently ignores fails the audit:
    // TraceElement does not deny unknown fields, so parsing accepts the
    // stray key and only the round-trip audit reports the loss.
    let mut stray = minimal_frame_json();
    stray["elements"] = serde_json::json!([{
        "entity_id": {"kind": "pc", "index": 1},
        "creation_order": 1,
        "class_id": 0,
        "kind": "pc",
        "active": true,
        "blipped": false,
        "unreachable": false,
        "surface_id": 0,
        "posture": 0,
        "position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "old_position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "position_goal_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "elevation": {"bits": 0},
        "old_elevation": {"bits": 0},
        "increment_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "increment_map_valid": true,
        "movement_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "layer": 0,
        "layer_goal": 0,
        "sector": 0,
        "direction": 0,
        "direction_goal": 0,
        "moving": false,
        "moving_map": false,
        "sprite_row": 0,
        "sprite_frame": 0,
        "sprite_frame_count": 0,
        "runtime": {},
        "novel_recorder_field": 123
    }]);
    let line = stray.to_string();
    let frame: TraceFrame = serde_json::from_str(&line).unwrap();
    let mut original: serde_json::Value = serde_json::from_str(&line).unwrap();
    let mut reserialized = serde_json::to_value(frame).unwrap();
    normalize_trace_json_for_roundtrip(&mut original);
    normalize_trace_json_for_roundtrip(&mut reserialized);
    let message = first_json_difference("$", &original, &reserialized)
        .expect("the stray recorder field must be reported as dropped");
    assert!(
        message.contains("$.elements[0].novel_recorder_field is dropped"),
        "unexpected audit failure message: {message}"
    );
}

#[test]
fn alert_eligibility_accepts_only_the_current_post_key() {
    #[derive(bitcode::Encode, bitcode::Decode)]
    struct FrozenTraceAlertEligibilityV66 {
        rank: bool,
        able_to_help: Option<bool>,
        allowed_to_leave_post: Option<bool>,
        can_call: Option<bool>,
        max_radius: Option<bool>,
        squared_radius: Option<bool>,
        capacity: Option<bool>,
        think: Option<bool>,
    }

    let current_json = serde_json::json!({
        "rank": true,
        // Current schema-16 recordings emit this spelling. Keep the
        // false case explicit: stay-on-post rejections are exactly where
        // the pre-fix converter used to fail its lossless round-trip
        // audit by serializing this key under the legacy spelling.
        "allowed_to_leave_post": false,
    });
    let current: TraceAlertEligibility = serde_json::from_value(current_json.clone()).unwrap();
    assert_eq!(current.allowed_to_leave_post, Some(false));

    let frozen_v66 = FrozenTraceAlertEligibilityV66 {
        rank: true,
        able_to_help: Some(false),
        allowed_to_leave_post: Some(true),
        can_call: None,
        max_radius: Some(false),
        squared_radius: Some(true),
        capacity: None,
        think: Some(true),
    };
    let frozen_v66_bytes = bitcode::encode(&frozen_v66);
    let decoded: TraceAlertEligibility = bitcode::decode(&frozen_v66_bytes).unwrap();
    assert!(decoded.rank);
    assert_eq!(decoded.able_to_help, Some(false));
    assert_eq!(decoded.allowed_to_leave_post, Some(true));
    assert_eq!(decoded.can_call, None);
    assert_eq!(decoded.max_radius, Some(false));
    assert_eq!(decoded.squared_radius, Some(true));
    assert_eq!(decoded.capacity, None);
    assert_eq!(decoded.think, Some(true));
    assert_eq!(bitcode::encode(&decoded), frozen_v66_bytes);

    let serialized = serde_json::to_value(&current).unwrap();
    assert_eq!(
        serialized["allowed_to_leave_post"],
        serde_json::json!(false)
    );
    assert!(serialized.get("stay_on_post").is_none());

    assert!(
        serde_json::from_value::<TraceAlertEligibility>(serde_json::json!({
            "rank": true,
            "stay_on_post": true,
        }))
        .is_err(),
        "obsolete schema-16 alert key was accepted"
    );
}

#[test]
fn simulation_body_marker_is_mandatory() {
    let mut frame_without_marker = minimal_frame_json();
    frame_without_marker
        .as_object_mut()
        .unwrap()
        .remove("simulation_body_ran");

    let error = serde_json::from_value::<TraceFrame>(frame_without_marker)
        .expect_err("current frames must report whether the simulation body ran");
    assert!(error.to_string().contains("simulation_body_ran"));
}

#[test]
fn current_path_events_are_typed() {
    let event = serde_json::json!({
        "phase": "completed",
        "actor": {"kind": "soldier", "index": 3},
        "antagonist": null,
        "layer": 2,
        "area": 17,
        "source": {"x": {"bits": 1065353216}, "y": {"bits": 1073741824}},
        "goal": {"x": {"bits": 1077936128}, "y": {"bits": 1082130432}},
        "half_diagonal_index": 1,
        "half_diagonal": {
            "x": {"bits": 1056964608},
            "y": {"bits": 1056964608}
        },
        "animation": 42,
        "reverse": false,
        "speed": 3,
        "tolerance": {"bits": 1092616192},
        "use_first_point": true,
        "valid": true,
        "waypoints": [
            {"x": {"bits": 1077936128}, "y": {"bits": 1082130432}}
        ]
    });

    let parsed: TracePathEvent =
        serde_json::from_value(event).expect("parse schema-9 completed path event");
    match parsed {
        TracePathEvent::Completed {
            actor,
            valid,
            waypoints,
            ..
        } => {
            assert_eq!(actor.index, 3);
            assert!(valid);
            assert_eq!(waypoints.len(), 1);
        }
        TracePathEvent::Queued { .. } => panic!("completed event parsed as queued"),
    }
}

#[test]
fn path_events_compare_ordered_request_bits_and_cancelled_validity() {
    let expected: TracePathEvent = serde_json::from_value(serde_json::json!({
        "phase": "completed",
        "actor": {"kind": "soldier", "index": 3},
        "antagonist": null,
        "layer": 2,
        "area": 17,
        "source": {"x": {"bits": 1065353216}, "y": {"bits": 1073741824}},
        "goal": {"x": {"bits": 1077936128}, "y": {"bits": 1082130432}},
        "half_diagonal_index": 1,
        "half_diagonal": {
            "x": {"bits": 1056964608},
            "y": {"bits": 1056964608}
        },
        "animation": 42,
        "reverse": false,
        "speed": 3,
        "tolerance": {"bits": 1092616192},
        "use_first_point": true,
        "valid": false,
        "waypoints": [
            {"x": {"bits": 1077936128}, "y": {"bits": 1082130432}}
        ]
    }))
    .expect("parse completed path event");
    let original_actor = TraceEntityId {
        kind: TraceEntityKind::Soldier,
        index: 3,
    };
    let rust_actor = EntityId::Soldier(robin_engine::entity_id::SoldierId(30));
    let map = EntityMap {
        entities: BTreeMap::from([(original_actor, rust_actor)]),
        entities_by_creation_order: BTreeMap::new(),
        sectors: BTreeMap::new(),
        sector_indices: BTreeMap::new(),
        gates: Vec::new(),
        runtime_creation_order_boundary: u32::MAX,
    };
    let request = robin_engine::pathfinder::ParityPathRequest {
        actor: rust_actor,
        antagonist: None,
        layer: 2,
        area: 17,
        source: MapPoint::new(f32::from_bits(1065353216), f32::from_bits(1073741824)),
        goal: MapPoint::new(f32::from_bits(1077936128), f32::from_bits(1082130432)),
        half_diagonal_index: 1,
        half_diagonal: robin_engine::coordinates::MoveBoxHalfDiagonal::new(0.5, 0.5),
        animation: 42,
        reverse: false,
        speed: 3,
        tolerance: 10.0,
        use_first_point: true,
    };
    let actual = robin_engine::pathfinder::ParityPathEvent::Completed {
        request: request.clone(),
        valid: false,
        waypoints: vec![request.goal],
    };
    assert!(
        compare_path_events(
            std::slice::from_ref(&expected),
            std::slice::from_ref(&actual),
            &map
        )
        .is_empty()
    );

    let mismatched = robin_engine::pathfinder::ParityPathEvent::Completed {
        request,
        valid: true,
        waypoints: match actual {
            robin_engine::pathfinder::ParityPathEvent::Completed { waypoints, .. } => waypoints,
            _ => unreachable!(),
        },
    };
    assert!(
        compare_path_events(&[expected], &[mismatched], &map)
            .iter()
            .any(|difference| difference.contains(".valid:"))
    );
}

#[test]
fn mission_start_requires_frame_zero() {
    validate_trace_start(TraceStartState::MissionStart, 7, 0);
}

#[test]
fn loaded_save_is_admitted_to_strict_reconstruction() {
    validate_trace_start(TraceStartState::LoadedSave, 7, 1234);
}

#[test]
fn stable_rng_domains_do_not_depend_on_callsite_offsets() {
    let batch = TraceRngBatch {
        first_index: 0,
        values: vec![1, 2],
        callsite_offsets: vec![3_305_465, 123],
        main_thread: vec![true, false],
        domains: vec![TraceRngDomain::Simulation, TraceRngDomain::Audio],
    };
    assert_eq!(batch.gameplay_draw_count(), 1);
    assert_eq!(batch.gameplay_callsite_offsets(), vec![3_305_465]);
    assert_eq!(simulation_rng_draws(&batch), vec![1]);
}

#[test]
fn rng_preload_is_limited_to_reconstruction_or_diagnostic_override() {
    assert!(!should_preload_complete_rng_stream(
        TraceStartState::MissionStart,
        0,
        false
    ));
    assert!(!should_preload_complete_rng_stream(
        TraceStartState::LoadedSave,
        1,
        false
    ));
    assert!(should_preload_complete_rng_stream(
        TraceStartState::LoadedSave,
        0,
        false
    ));
    assert!(should_preload_complete_rng_stream(
        TraceStartState::MissionStart,
        1,
        true
    ));
}

#[test]
#[should_panic(expected = "occurred off the main thread")]
fn simulation_rng_draws_from_worker_threads_are_rejected() {
    TraceRngBatch {
        first_index: 41,
        values: vec![7],
        callsite_offsets: vec![123],
        main_thread: vec![false],
        domains: vec![TraceRngDomain::Simulation],
    }
    .validate();
}

#[test]
fn clean_terminator_retains_completion_metadata() {
    let suffix: TraceRngOnly = serde_json::from_value(serde_json::json!({
        "type": "rng_suffix",
        "draws": {
            "first_index": 9,
            "values": [],
            "callsite_offsets": [],
            "main_thread": [],
            "domains": []
        },
        "final_frame": 112,
        "frame_count": 12
    }))
    .expect("parse clean current-schema terminator");
    assert_eq!(suffix.record_type, "rng_suffix");
    assert_eq!(suffix.final_frame, 112);
    assert_eq!(suffix.frame_count, 12);
    suffix.draws.validate();
}

#[test]
fn original_commands_map_by_semantic_name() {
    assert_eq!(Action::from(TraceAction::Bow), Action::Bow);
    assert_eq!(command_from_stable_name("raise_bow"), Command::RaiseBow);
    assert_eq!(command_from_stable_name("jump"), Command::JumpCmd);
    assert_eq!(command_from_stable_name("roll"), Command::Jump);
}

#[test]
fn trace_element_retains_all_recorded_authoritative_state() {
    let element: TraceElement = serde_json::from_value(serde_json::json!({
        "entity_id": {"kind": "soldier", "index": 58},
        "creation_order": 89,
        "class_id": 1,
        "kind": "soldier",
        "active": true,
        "blipped": false,
        "unreachable": false,
        "surface_id": 1226,
        "posture": 1,
        "position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "old_position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "position_goal_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "elevation": {"bits": 0},
        "old_elevation": {"bits": 0},
        "increment_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "increment_map_valid": true,
        "movement_map": {"x": {"bits": 0}, "y": {"bits": 0}},
        "layer": 0,
        "layer_goal": 0,
        "sector": 0,
        "direction": 0,
        "direction_goal": 0,
        "moving": false,
        "moving_map": false,
        "sprite_row": 64,
        "sprite_frame": 3,
        "sprite_frame_count": 65535,
        "actor": {
            "action_state": 0,
            "animation": 254,
            "command": 117,
            "command_name": "whistle",
            "motion_state": 2,
            "wait_time": 25,
            "passing_door_directly": false,
            "active_pass_door": null,
            "sequence_element": null,
            "position_interface": {}
        },
        "human": {
            "life_points": 60,
            "dead": false,
            "unconscious": false,
            "camp": "lacklandists",
            "original_camp": 1,
            "vip": true,
            "civilian": false,
            "opponents": [{"kind": "pc", "index": 2}],
            "opponent_jump_lines": [null]
        },
        "ai": {
            "state": 3,
            "substate": 17,
            "script_locked": false,
            "locked": true,
            "locks": 1,
            "was_busy": true,
            "very_busy": false,
            "macro_timer_running": true,
            "macro_timer_ring": 987,
            "macro_cursor": 4,
            "macro_remaining": 2,
            "macro_in_progress": true,
            "list_us": [{"kind": "soldier", "index": 58}],
            "list_them": [{"kind": "pc", "index": 2}],
            "my_line_jump": null
        },
        "detection": {
            "suspects": [1, 2, 3, 4, 5, 6],
            "maximal_suspect": 6,
            "maximal_visibility": 200,
            "view_status": 1,
            "alert_status": 2,
            "detectables": [{
                "type": 0,
                "target": {"kind": "pc", "index": 2},
                "seen_now": true,
                "seen_last_frame": false,
                "heard_last_frame": true,
                "shadow_seen_now": false,
                "shadow_seen_last_frame": true,
                "last_visibility": {"bits": 1120403456}
            }]
        },
        "runtime": {}
    }))
    .expect("parse authoritative recorded element state");

    // Raw class/surface identifiers remain available to dumps even though
    // logical parity compares the concrete kind instead. In particular,
    // Original surface values are renderer allocation handles.
    assert_eq!(element.class_id, 1);
    assert_eq!(element.surface_id, 1226);
    assert_eq!(element.layer_goal, 0);
    assert_eq!(element.sprite_row, 64);
    assert_eq!(element.sprite_frame, 3);
    assert_eq!(element.sprite_frame_count, u16::MAX);
    let human = element.human.expect("human state");
    assert_eq!(human.camp, "lacklandists");
    assert!(human.vip);
    let ai = element.ai.expect("AI state");
    assert_eq!(ai.macro_cursor, Some(4));
    assert_eq!(
        ai.list_them,
        [TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 2,
        }]
    );
    let detection = element.detection.expect("detection state");
    assert_eq!(detection.suspects, [1, 2, 3, 4, 5, 6]);
    assert_eq!(detection.detectables[0].last_visibility.value(), 100.0);
}

#[test]
fn only_replay_constructed_bonuses_omit_original_undefined_old_position() {
    let boundary = 143;

    assert!(original_runtime_bonus_has_undefined_old_position(
        TraceEntityKind::Bonus,
        326,
        boundary,
    ));
    assert!(!original_runtime_bonus_has_undefined_old_position(
        TraceEntityKind::Bonus,
        142,
        boundary,
    ));
    assert!(!original_runtime_bonus_has_undefined_old_position(
        TraceEntityKind::Projectile,
        326,
        boundary,
    ));
}

#[test]
fn current_ai_core_and_human_snapshots_reject_missing_fields() {
    let complete_ai = serde_json::json!({
        "state": 3,
        "substate": 17,
        "script_locked": false,
        "locked": true,
        "locks": 0,
        "was_busy": false,
        "very_busy": false,
        "macro_timer_running": false,
        "macro_timer_ring": 0,
        "macro_cursor": null,
        "macro_remaining": 0,
        "macro_in_progress": false,
        "list_us": [],
        "list_them": [],
        "my_line_jump": null
    });
    for required in ["state", "substate"] {
        let mut incomplete = complete_ai.clone();
        incomplete.as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<TraceAi>(incomplete).is_err(),
            "current AI snapshot accepted missing {required}"
        );
    }
    let additive_defaults: TraceAi = serde_json::from_value(serde_json::json!({
        "state": 3,
        "substate": 17
    }))
    .expect("parse AI snapshot without later additive diagnostics");
    assert!(!additive_defaults.script_locked);
    assert!(!additive_defaults.locked);
    assert_eq!(additive_defaults.locks, 0);
    assert_eq!(additive_defaults.macro_cursor, None);
    assert!(additive_defaults.list_us.is_empty());
    assert!(additive_defaults.list_them.is_empty());
    assert!(additive_defaults.my_line_jump.is_none());

    let complete_human = serde_json::json!({
        "life_points": 60,
        "dead": false,
        "unconscious": false,
        "camp": "royalists",
        "original_camp": 0,
        "vip": false,
        "civilian": false,
        "opponents": [],
        "opponent_jump_lines": []
    });
    for required in ["opponents", "opponent_jump_lines"] {
        let mut incomplete = complete_human.clone();
        incomplete.as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<TraceHuman>(incomplete).is_err(),
            "current human snapshot accepted missing {required}"
        );
    }
}

#[test]
fn runtime_snapshot_canonicalizes_original_zero_based_order_ids() {
    let mut expected = serde_json::json!({
        "sprite": {
            "last_processed_order_id": 41,
            "unrelated_id": 41
        },
        "nested": [{"last_processed_order_id": 0}],
        "sentinel": {"last_processed_order_id": u32::MAX}
    });

    canonicalize_original_runtime_representation(&mut expected);

    assert_eq!(expected["sprite"]["last_processed_order_id"], 42);
    assert_eq!(expected["sprite"]["unrelated_id"], 41);
    assert_eq!(expected["nested"][0]["last_processed_order_id"], 1);
    assert_eq!(expected["sentinel"]["last_processed_order_id"], u32::MAX);
}

#[test]
fn runtime_projectile_projects_only_indeterminate_constructor_storage() {
    let mut expected = serde_json::json!({
        "position": {
            "computed_position": 7,
            "computed_increment": 2,
            "posture": 1,
            "old_posture": 1,
            "increment": {"x": 1, "y": 2, "z": 3},
            "door_direction": true,
            "goal_world": {"x": 99, "y": 98, "z": 97},
            "radius": {"bits": 97},
            "material": 1_521_537_396_u32,
            "move_box": {
                "min": {"x": 10, "y": 20},
                "max": {"x": 10, "y": 20}
            }
        },
        "sprite": {
            "row": 0,
            "flight_countdown": 9660,
            "behind_display_order_reference": true,
            "display_order_reference": null,
            "last_processed_order_id": 65536
        }
    });

    project_runtime_projectile_constructor_storage(&mut expected);

    let position = expected["position"].as_object().unwrap();
    for omitted in ["door_direction", "goal_world", "radius", "material"] {
        assert!(!position.contains_key(omitted));
    }
    assert!(position["move_box"].is_null());
    assert_eq!(position["computed_position"], 7);
    assert_eq!(position["computed_increment"], 2);
    assert_eq!(
        position["increment"],
        serde_json::json!({"x": 1, "y": 2, "z": 3})
    );
    assert_eq!(position["posture"], 1);
    assert_eq!(position["old_posture"], 1);

    let sprite = expected["sprite"].as_object().unwrap();
    assert!(!sprite.contains_key("flight_countdown"));
    assert!(!sprite.contains_key("behind_display_order_reference"));
    assert_eq!(sprite["row"], 0);
    assert_eq!(sprite["last_processed_order_id"], 65536);
}

#[test]
fn runtime_projectile_material_projects_both_signed_garbage_domains() {
    for material in [serde_json::json!(-143_634_448), serde_json::json!(11)] {
        let mut expected = serde_json::json!({"position": {"material": material}});
        project_runtime_projectile_constructor_storage(&mut expected);
        assert!(expected["position"].get("material").is_none());
    }

    for material in 0..=10 {
        let mut expected = serde_json::json!({"position": {"material": material}});
        project_runtime_projectile_constructor_storage(&mut expected);
        assert_eq!(expected["position"]["material"], material);
    }

    // Malformed trace data is not constructor residue. Preserve it so
    // the strict subset comparator reports the schema/type violation.
    for material in [serde_json::Value::Null, serde_json::json!("invalid")] {
        let mut expected = serde_json::json!({"position": {"material": material.clone()}});
        project_runtime_projectile_constructor_storage(&mut expected);
        assert_eq!(expected["position"]["material"], material);
    }
}

#[test]
fn runtime_snapshot_canonicalizes_only_zero_original_blocked_boxes() {
    let float = |bits| serde_json::json!({"bits": bits, "value": 0.0});
    let zero_box = serde_json::json!({
        "min": {"x": float(0), "y": float(0)},
        "max": {"x": float(0), "y": float(0)}
    });
    let mut expected = serde_json::json!({
        "position": {"blocked_box": zero_box},
        "active_position": {"blocked_box": {
            "min": {"x": float(0), "y": float(0)},
            "max": {"x": float(0x3f80_0000), "y": float(0)}
        }},
        "unrelated_box": zero_box
    });

    canonicalize_original_runtime_representation(&mut expected);

    assert!(expected["position"]["blocked_box"].is_null());
    assert!(expected["active_position"]["blocked_box"].is_object());
    assert!(expected["unrelated_box"].is_object());
}

#[test]
fn runtime_snapshot_compatibility_projection_matches_rust_representation() {
    let mut expected = serde_json::json!({
        "position": {"blocked_box": {
            "min": {
                "x": {"bits": 0, "value": 0.0},
                "y": {"bits": 0, "value": 0.0}
            },
            "max": {
                "x": {"bits": 0, "value": 0.0},
                "y": {"bits": 0, "value": 0.0}
            }
        }},
        "sprite": {"last_processed_order_id": 41}
    });
    let actual = serde_json::json!({
        "position": {"blocked_box": null},
        "sprite": {"last_processed_order_id": 42}
    });

    canonicalize_original_runtime_representation(&mut expected);
    let mut differences = Vec::new();
    collect_json_subset_differences("runtime", &expected, &actual, &mut differences);

    assert!(differences.is_empty(), "{differences:#?}");
}

#[test]
fn missing_draw_view_projects_only_sprite_presentation_cache() {
    let mut expected = serde_json::json!({
        "sprite": {
            "width": 20,
            "height": 53,
            "masked": true,
            "current_row": 9,
            "current_frame": 3,
            "frame_count": 4,
            "use_alternate_profile": true
        },
        "position": {"layer": 2},
        "width": "unrelated gameplay field"
    });
    let mut actual = serde_json::json!({
        "sprite": {
            "width": 24,
            "height": 55,
            "masked": false,
            "current_row": 9,
            "current_frame": 3,
            "frame_count": 4,
            "use_alternate_profile": true
        },
        "position": {"layer": 2},
        "width": "unrelated gameplay field"
    });

    project_missing_draw_view_sprite_cache(&mut expected);
    let mut differences = Vec::new();
    collect_json_subset_differences("runtime", &expected, &actual, &mut differences);
    assert!(differences.is_empty(), "{differences:#?}");

    actual["sprite"]["current_frame"] = serde_json::json!(4);
    actual["position"]["layer"] = serde_json::json!(3);
    actual["width"] = serde_json::json!("changed");
    collect_json_subset_differences("runtime", &expected, &actual, &mut differences);
    assert_eq!(differences.len(), 3, "{differences:#?}");
    assert!(
        differences
            .iter()
            .any(|difference| difference.contains("sprite.current_frame"))
    );
    assert!(
        differences
            .iter()
            .any(|difference| difference.contains("position.layer"))
    );
    assert!(
        differences
            .iter()
            .any(|difference| difference.contains("runtime.width"))
    );
}

#[test]
fn runtime_snapshot_comparison_requires_recorded_subset_and_float_bits() {
    let expected = serde_json::json!({
        "position": {
            "world": {"bits": 1, "value": 1.401298464324817e-45}
        }
    });
    let actual = serde_json::json!({
        "position": {
            "world": {"bits": 1, "value": 1.401_298_464_324_817e-45}
        },
        "rust_only_diagnostic": true
    });
    let mut differences = Vec::new();
    collect_json_subset_differences("runtime", &expected, &actual, &mut differences);
    assert!(differences.is_empty());

    let wrong_bits = serde_json::json!({
        "position": {
            "world": {"bits": 2, "value": 1.401298464324817e-45}
        }
    });
    collect_json_subset_differences("runtime", &expected, &wrong_bits, &mut differences);
    assert_eq!(differences.len(), 1);
    assert!(differences[0].contains("runtime.position.world.bits"));
}

#[test]
fn visibility_query_retains_authoritative_call_and_diagnostics() {
    let query: TraceVisibilityQuery = serde_json::from_value(serde_json::json!({
        "origin": {"x": {"bits": 1}, "y": {"bits": 2}, "z": {"bits": 3}},
        "destination": {"x": {"bits": 4}, "y": {"bits": 5}, "z": {"bits": 6}},
        "result": false,
        "cache_hit": false,
        "cache_key": 123456,
        "cache_offset": 1456,
        "candidate_count": 1,
        "reason": "wall",
        "blocking_obstacle": {
            "id": 8,
            "index": 7,
            "type_mask": 3,
            "types": {
                "solid": true,
                "opaque": true,
                "projection_area": false,
                "mouse": false,
                "shield": false,
                "show_shadow_polygon": false
            },
            "active": true,
            "on_ground": true,
            "layer": 65535,
            "sector": 65535,
            "box_ground": {
                "min": {"x": {"bits": 0}, "y": {"bits": 0}},
                "max": {"x": {"bits": 1065353216}, "y": {"bits": 1065353216}}
            },
            "points": [{
                "x": {"bits": 0},
                "y": {"bits": 0},
                "z_top": {"bits": 1065353216},
                "z_bottom": {"bits": 0}
            }]
        }
    }))
    .expect("parse complete visibility query");

    assert_eq!(query.origin.x.bits, 1);
    assert_eq!(query.reason, "wall");
    let actual = robin_engine::sight_obstacle::ParityVisibilityQuery {
        origin: [f32::from_bits(1), f32::from_bits(2), f32::from_bits(3)],
        destination: [f32::from_bits(4), f32::from_bits(5), f32::from_bits(6)],
        result: false,
        caller_file: file!(),
        caller_line: line!(),
    };
    assert!(compare_visibility_queries(std::slice::from_ref(&query), &[actual]).is_empty());
    let mismatched = robin_engine::sight_obstacle::ParityVisibilityQuery {
        result: true,
        ..actual
    };
    assert!(
        compare_visibility_queries(std::slice::from_ref(&query), &[mismatched])
            .iter()
            .any(|difference| difference.contains(".result:"))
    );
    let obstacle = query.blocking_obstacle.expect("blocking obstacle");
    assert_eq!(obstacle.index, 7);
    assert!(obstacle.types.opaque);
    assert_eq!(obstacle.points.len(), 1);
}

#[test]
fn global_action_cancel_accepts_the_original_no_pc_shape() {
    let command: TraceCommand = serde_json::from_value(serde_json::json!({
        "type": "cancel_action",
        "action": "no_action",
        "original_action": 0
    }))
    .expect("parse Original global action cancellation");
    assert!(matches!(
        command,
        TraceCommand::CancelAction { pc: None, .. }
    ));
}

/// Every resolved-command type the recorder can emit must decode.  A
/// type the runner does not know aborts the whole trace before any
/// simulation comparison happens, so the schema has to stay complete
/// rather than merely covering whatever the current corpus contains.
#[test]
fn every_recorded_command_type_decodes() {
    let pc = serde_json::json!({"kind": "pc", "index": 3});
    let point2 = serde_json::json!({
        "x": {"bits": 1065353216, "value": 1.0},
        "y": {"bits": 1073741824, "value": 2.0}
    });
    let point3 = serde_json::json!({
        "x": {"bits": 1065353216, "value": 1.0},
        "y": {"bits": 1073741824, "value": 2.0},
        "z": {"bits": 1077936128, "value": 3.0}
    });
    let recorded = [
        serde_json::json!({"type": "box_select", "first": point2, "second": point2, "append": false}),
        serde_json::json!({"type": "box_unselect", "first": point2, "second": point2, "append": false}),
        serde_json::json!({"type": "group_move", "actors": [pc], "destination": point2,
            "running": true, "show_marker": true, "goal_sector": 4, "goal_layer": 0}),
        serde_json::json!({"type": "launch_interaction", "actor": pc, "target": pc,
            "original_command": 0, "original_command_name": "hit", "running": false}),
        serde_json::json!({"type": "launch_scroll_read", "actor": pc, "target": pc, "running": false}),
        serde_json::json!({"type": "sword_strike", "actor": pc, "target": pc,
            "original_command": 0, "original_command_name": "hit", "with_seek": true,
            "seek_distance": 63.0}),
        serde_json::json!({"type": "launch_self_ability", "actor": pc,
            "original_command": 0, "original_command_name": "eat"}),
        serde_json::json!({"type": "launch_ground_target", "actor": pc, "target": point3,
            "original_command": 0, "original_command_name": "throw_purse",
            "original_target_field": 30, "titbit_layer": 0}),
        serde_json::json!({"type": "drop_ale_at", "actor": pc, "target": point2, "running": false}),
        serde_json::json!({"type": "shield_select_protected", "actor": pc, "protected_pc": pc}),
        serde_json::json!({"type": "raise_shield_with_danger", "actor": pc, "protected_pc": pc,
            "danger_point": point3, "danger_point_layer": 0}),
        serde_json::json!({"type": "teleport_selected", "destination": point2,
            "goal_sector": -1, "goal_layer": 0}),
        serde_json::json!({"type": "stop_pc", "pc": pc}),
        serde_json::json!({"type": "select_pc", "pc": pc, "append": false}),
        serde_json::json!({"type": "select_all_pcs"}),
        serde_json::json!({"type": "unselect_pc", "pc": pc}),
        serde_json::json!({"type": "unselect_all_pcs"}),
        serde_json::json!({"type": "select_action_index", "index": 1}),
        serde_json::json!({"type": "select_action", "action": "bow", "original_action": 1, "pc": pc}),
        serde_json::json!({"type": "cancel_action", "action": "no_action", "original_action": 0}),
        serde_json::json!({"type": "crouch_down"}),
        serde_json::json!({"type": "stand_up"}),
        serde_json::json!({"type": "start_macro", "slot": 1, "pc": pc}),
        serde_json::json!({"type": "delete_macro", "slot": 1}),
        serde_json::json!({"type": "start_recording_macro", "slot": 2, "pc": pc}),
        serde_json::json!({"type": "change_qa_memory", "slot": 0}),
        serde_json::json!({"type": "set_lock_alt", "on": true}),
        serde_json::json!({"type": "key_control"}),
        serde_json::json!({"type": "key_release_control"}),
        serde_json::json!({"type": "make_pc_fast", "entity": pc}),
        serde_json::json!({"type": "beggar_dont_talk_stamp", "entity": pc}),
        serde_json::json!({"type": "orient_action_at", "action": "bow", "original_action": 1,
            "actor": pc, "mouse_map": point2, "target": point3}),
    ];
    for value in recorded {
        let recorded_type = value["type"].clone();
        serde_json::from_value::<TraceCommand>(value.clone())
            .unwrap_or_else(|err| panic!("decode recorded command {recorded_type}: {err}"));
    }
}

#[test]
fn native_bitcode_trace_handles_heterogeneous_command_variants() {
    let commands = [
        TraceCommand::CrouchDown,
        TraceCommand::LaunchGroundTarget {
            actor: TraceEntityId {
                kind: TraceEntityKind::Pc,
                index: 126,
            },
            target: TracePoint3 {
                x: TraceFloat {
                    bits: 834.0_f32.to_bits(),
                },
                y: TraceFloat {
                    bits: 765.0_f32.to_bits(),
                },
                z: TraceFloat {
                    bits: 0.0_f32.to_bits(),
                },
            },
            original_command: 86,
            original_command_name: "throw_purse".to_owned(),
            original_target_field: 30,
            titbit_layer: 0,
        },
    ];
    let mut encoded = Vec::new();
    for command in &commands {
        write_binary_record(&mut encoded, command, "test command");
    }

    let mut reader = std::io::Cursor::new(encoded);
    assert!(matches!(
        read_binary_record(&mut reader, "test command").unwrap(),
        TraceCommand::CrouchDown
    ));
    assert!(matches!(
        read_binary_record(&mut reader, "test command").unwrap(),
        TraceCommand::LaunchGroundTarget {
            original_target_field: 30,
            ..
        }
    ));
}

#[test]
fn resolved_orientation_is_bit_exact() {
    let command: TraceCommand = serde_json::from_value(serde_json::json!({
        "type": "orient_action_at",
        "action": "bow",
        "original_action": 1,
        "actor": {"kind": "pc", "index": 198},
        "mouse_map": {
            "x": {"bits": 1065353216, "value": 1.0},
            "y": {"bits": 1073741824, "value": 2.0}
        },
        "target": {
            "x": {"bits": 1077936128, "value": 3.0},
            "y": {"bits": 1082130432, "value": 4.0},
            "z": {"bits": 1084227584, "value": 5.0}
        }
    }))
    .unwrap();
    let TraceCommand::OrientActionAt {
        action,
        mouse_map,
        target,
        ..
    } = command
    else {
        panic!("wrong trace command variant");
    };
    assert!(matches!(action, TraceAction::Bow));
    assert_eq!(MapPoint::from(mouse_map), MapPoint::new(1.0, 2.0));
    assert_eq!(WorldPoint3D::from(target), WorldPoint3D::new(3.0, 4.0, 5.0));
}

#[test]
fn matching_action_selection_marks_only_following_orientation_as_late_refresh() {
    let pc = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 282,
    };
    let point = TracePoint {
        x: TraceFloat { bits: 0 },
        y: TraceFloat { bits: 0 },
    };
    let target = TracePoint3 {
        x: TraceFloat { bits: 0 },
        y: TraceFloat { bits: 0 },
        z: TraceFloat { bits: 0 },
    };
    let orientation = || TraceCommand::OrientActionAt {
        action: TraceAction::Purse,
        original_action: 4,
        actor: pc,
        mouse_map: point,
        target,
    };
    let commands = vec![
        orientation(),
        TraceCommand::SelectAction {
            pc,
            action: TraceAction::Purse,
            original_action: 4,
        },
        orientation(),
    ];

    let (before, after) = split_refresh_owned_orientations(commands, false, &[], false);

    assert_eq!(before.len(), 2);
    assert!(matches!(before[0], TraceCommand::OrientActionAt { .. }));
    assert!(matches!(before[1], TraceCommand::SelectAction { .. }));
    assert_eq!(after.len(), 1);
    assert!(matches!(after[0], TraceCommand::OrientActionAt { .. }));
}

#[test]
fn popup_nested_refresh_marks_only_final_purse_orientation_as_late() {
    let pc = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 296,
    };
    let point = TracePoint {
        x: TraceFloat { bits: 0 },
        y: TraceFloat { bits: 0 },
    };
    let target = TracePoint3 {
        x: TraceFloat { bits: 0 },
        y: TraceFloat { bits: 0 },
        z: TraceFloat { bits: 0 },
    };
    let orientation = || TraceCommand::OrientActionAt {
        action: TraceAction::Purse,
        original_action: 4,
        actor: pc,
        mouse_map: point,
        target,
    };

    // The ordinary refresh record remains before Hourglass. The final
    // duplicate was emitted by DisplayPopupText's nested refresh.
    let (before, after) = split_refresh_owned_orientations(
        vec![orientation(), orientation()],
        true,
        &[(pc, TraceAction::Purse)],
        false,
    );

    assert_eq!(before.len(), 1);
    assert!(matches!(before[0], TraceCommand::OrientActionAt { .. }));
    assert_eq!(after.len(), 1);
    assert!(matches!(
        after[0],
        TraceCommand::OrientActionAt {
            action: TraceAction::Purse,
            ..
        }
    ));
}

#[test]
fn popup_frame_keeps_a_single_throw_orientation_before_hourglass() {
    let command = TraceCommand::OrientActionAt {
        action: TraceAction::Purse,
        original_action: 4,
        actor: TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 296,
        },
        mouse_map: TracePoint {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
        },
        target: TracePoint3 {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
            z: TraceFloat { bits: 0 },
        },
    };

    let (before, after) = split_refresh_owned_orientations(
        vec![command],
        true,
        &[(
            TraceEntityId {
                kind: TraceEntityKind::Pc,
                index: 296,
            },
            TraceAction::Purse,
        )],
        false,
    );

    assert_eq!(before.len(), 1);
    assert!(after.is_empty());
}

#[test]
fn legacy_random_popup_keeps_first_post_selection_orientation_early() {
    let pc = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 296,
    };
    let selection = vec![
        TraceCommand::SelectPc { pc, append: false },
        TraceCommand::SelectAction {
            pc,
            action: TraceAction::Purse,
            original_action: 4,
        },
    ];
    let orientation = TraceCommand::OrientActionAt {
        action: TraceAction::Purse,
        original_action: 4,
        actor: pc,
        mouse_map: TracePoint {
            x: TraceFloat { bits: 0x4501_0000 },
            y: TraceFloat { bits: 0x4302_0000 },
        },
        target: TracePoint3 {
            x: TraceFloat { bits: 0x4501_0000 },
            y: TraceFloat { bits: 0x4302_0000 },
            z: TraceFloat { bits: 0 },
        },
    };
    let provenance = LegacyRefreshOrientationProvenance::default().advance(&selection, false);

    // Save055 replay-006 reaches its popup on the first boundary after
    // selecting Purse. Its singleton is the preceding ordinary refresh.
    assert!(
        !provenance
            .proves_single_popup_orientation_is_late(std::slice::from_ref(&orientation), true,)
    );
    let (before, after) = split_refresh_owned_orientations(
        vec![orientation],
        true,
        &[(pc, TraceAction::Purse)],
        false,
    );
    assert_eq!(before.len(), 1);
    assert!(after.is_empty());
}

#[test]
fn legacy_random_popup_marks_exact_repeated_orientation_late() {
    let pc = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 296,
    };
    let selection = vec![
        TraceCommand::SelectPc { pc, append: false },
        TraceCommand::SelectAction {
            pc,
            action: TraceAction::Purse,
            original_action: 4,
        },
    ];
    let orientation = || TraceCommand::OrientActionAt {
        action: TraceAction::Purse,
        original_action: 4,
        actor: pc,
        mouse_map: TracePoint {
            x: TraceFloat { bits: 1158882171 },
            y: TraceFloat { bits: 1126828606 },
        },
        target: TracePoint3 {
            x: TraceFloat { bits: 1158882171 },
            y: TraceFloat { bits: 1126828606 },
            z: TraceFloat { bits: 0 },
        },
    };
    let selected = LegacyRefreshOrientationProvenance::default().advance(&selection, false);
    let first_orientation = orientation();
    let provenance = selected.advance(std::slice::from_ref(&first_orientation), false);
    let popup_orientation = orientation();
    let force_late = provenance
        .proves_single_popup_orientation_is_late(std::slice::from_ref(&popup_orientation), true);

    // Save055 replay-033 first orients in the intervening ordinary
    // refresh. Its identical popup singleton is therefore nested-late.
    assert!(force_late);
    let (before, after) = split_refresh_owned_orientations(
        vec![popup_orientation],
        true,
        &[(pc, TraceAction::Purse)],
        force_late,
    );
    assert!(before.is_empty());
    assert_eq!(after.len(), 1);
}

#[test]
fn legacy_random_popup_duplicate_retains_ordinary_then_nested_order() {
    let pc = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 296,
    };
    let orientation = || TraceCommand::OrientActionAt {
        action: TraceAction::Purse,
        original_action: 4,
        actor: pc,
        mouse_map: TracePoint {
            x: TraceFloat { bits: 1 },
            y: TraceFloat { bits: 2 },
        },
        target: TracePoint3 {
            x: TraceFloat { bits: 1 },
            y: TraceFloat { bits: 2 },
            z: TraceFloat { bits: 0 },
        },
    };
    let previous_orientation = orientation();
    let provenance = LegacyRefreshOrientationProvenance::FirstOrdinaryOrientation(
        RefreshOrientationSignature::from_command(&previous_orientation).unwrap(),
    );
    let popup_commands = vec![orientation(), orientation()];

    // Save055 replay-028 records both the preceding ordinary refresh and
    // the popup's nested refresh. Singleton provenance must not claim it;
    // the established duplicate rule splits only the final orientation.
    assert!(!provenance.proves_single_popup_orientation_is_late(&popup_commands, true));
    let (before, after) =
        split_refresh_owned_orientations(popup_commands, true, &[(pc, TraceAction::Purse)], false);
    assert_eq!(before.len(), 1);
    assert_eq!(after.len(), 1);
    assert!(matches!(before[0], TraceCommand::OrientActionAt { .. }));
    assert!(matches!(after[0], TraceCommand::OrientActionAt { .. }));
}

#[test]
fn legacy_random_popup_proof_rejects_intervening_command_or_changed_target() {
    let pc = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 296,
    };
    let selected = LegacyRefreshOrientationProvenance::ActionSelected {
        actor: pc,
        action: TraceAction::Purse,
    };
    let orientation = |x_bits| TraceCommand::OrientActionAt {
        action: TraceAction::Purse,
        original_action: 4,
        actor: pc,
        mouse_map: TracePoint {
            x: TraceFloat { bits: x_bits },
            y: TraceFloat { bits: 2 },
        },
        target: TracePoint3 {
            x: TraceFloat { bits: x_bits },
            y: TraceFloat { bits: 2 },
            z: TraceFloat { bits: 0 },
        },
    };
    let interrupted = selected.advance(
        &[orientation(1), TraceCommand::MakePcFast { entity: pc }],
        false,
    );
    assert_eq!(interrupted, LegacyRefreshOrientationProvenance::None);

    let first_orientation = orientation(1);
    let provenance = selected.advance(std::slice::from_ref(&first_orientation), false);
    let changed = orientation(3);
    assert!(
        !provenance.proves_single_popup_orientation_is_late(std::slice::from_ref(&changed), true,)
    );
}

#[test]
fn popup_nested_refresh_marks_single_bow_orientation_as_late() {
    let pc = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 342,
    };
    let command = TraceCommand::OrientActionAt {
        action: TraceAction::Bow,
        original_action: 1,
        actor: pc,
        mouse_map: TracePoint {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
        },
        target: TracePoint3 {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
            z: TraceFloat { bits: 0 },
        },
    };

    let (before, after) = split_refresh_owned_orientations(vec![command], true, &[], false);

    assert!(before.is_empty());
    assert_eq!(after.len(), 1);
    assert!(matches!(
        after[0],
        TraceCommand::OrientActionAt {
            action: TraceAction::Bow,
            ..
        }
    ));
}

#[test]
fn trace_index_refresh_uses_stable_creation_order() {
    let old_trace_id = TraceEntityId {
        kind: TraceEntityKind::Projectile,
        index: 127,
    };
    let shifted_trace_id = TraceEntityId {
        kind: TraceEntityKind::Projectile,
        index: 126,
    };
    let rust_id = EntityId::Projectile(robin_engine::entity_id::ProjectileId(158));
    let mut map = EntityMap {
        entities: BTreeMap::from([(old_trace_id, rust_id)]),
        entities_by_creation_order: BTreeMap::from([(158, rust_id)]),
        sectors: BTreeMap::new(),
        sector_indices: BTreeMap::new(),
        gates: Vec::new(),
        runtime_creation_order_boundary: u32::MAX,
    };

    map.refresh_trace_index(shifted_trace_id, 158);

    assert_eq!(map.translate(shifted_trace_id), rust_id);
}

#[test]
fn group_move_translates_retained_sector_identity_without_click_containment() {
    let map = EntityMap {
        entities: BTreeMap::new(),
        entities_by_creation_order: BTreeMap::new(),
        sectors: BTreeMap::from([(55, 23), (56, 23)]),
        sector_indices: BTreeMap::from([
            (
                55,
                robin_engine::fast_find_grid::SectorIndex::new(7).unwrap(),
            ),
            (
                56,
                robin_engine::fast_find_grid::SectorIndex::new(8).unwrap(),
            ),
        ]),
        gates: Vec::new(),
        runtime_creation_order_boundary: 0,
    };

    // Patch moves record the patch's underlying position sector while the
    // recorded waypoint may lie outside that sector's polygon. Translation
    // therefore depends only on retained construction topology.
    assert_eq!(
        map.translate_group_move_goal_sector(55, 0, None),
        GroupMoveGoalTranslation::Runtime(
            (SectorNumber::new(23), 0),
            robin_engine::fast_find_grid::SectorIndex::new(7).unwrap(),
        )
    );
    assert_eq!(
        map.translate_group_move_goal_sector(56, 0, None),
        GroupMoveGoalTranslation::Runtime(
            (SectorNumber::new(23), 0),
            robin_engine::fast_find_grid::SectorIndex::new(8).unwrap(),
        ),
        "two Original sparse slots may share a public identity while retaining distinct arena identities"
    );
    assert_eq!(
        map.translate_group_move_goal_sector(288, 4, None),
        GroupMoveGoalTranslation::RecordedUnmapped((SectorNumber::new(288), 4)),
        "a coincident overlay must not erase the recorded route goal"
    );
}

#[test]
fn recorded_group_move_gate_uses_retained_mixed_gate_order() {
    let map = EntityMap {
        entities: BTreeMap::new(),
        entities_by_creation_order: BTreeMap::new(),
        sectors: BTreeMap::new(),
        sector_indices: BTreeMap::new(),
        // Original constructed jump gate 1 between two stateful doors;
        // Rust installed the stateful doors first, so its runtime peer is
        // door-table index 3.
        gates: vec![
            robin_engine::gate::DoorIndex::from(0),
            robin_engine::gate::DoorIndex::from(3),
            robin_engine::gate::DoorIndex::from(1),
        ],
        runtime_creation_order_boundary: 0,
    };

    assert_eq!(map.translate_gate(1), 3);
}

fn group_move_route_fixture(
    actor: TraceEntityId,
    kind: &str,
    ordinal: u64,
) -> TraceRouteConstructionEvent {
    let point = TracePoint {
        x: TraceFloat { bits: 0 },
        y: TraceFloat { bits: 0 },
    };
    TraceRouteConstructionEvent {
        kind: kind.to_owned(),
        actor,
        source: point,
        source_sector: 114,
        source_level: 7,
        goal: point,
        goal_sector: 117,
        goal_level: 8,
        gates: Vec::new(),
        draft_diagnostics: BTreeMap::from([
            (
                "ordinal".to_owned(),
                TraceJsonValue::from(TraceJsonTree::Unsigned(ordinal)),
            ),
            (
                "result".to_owned(),
                TraceJsonValue::from(TraceJsonTree::String("success".to_owned())),
            ),
        ]),
    }
}

#[test]
fn legacy_route_ordinals_restore_original_append_order() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 344,
    };
    let mut routes = [
        group_move_route_fixture(actor, "move", 0),
        group_move_route_fixture(actor, "move", 1),
    ];
    routes[0].draft_diagnostics.remove("ordinal");
    routes[1].draft_diagnostics.remove("ordinal");
    routes[0].draft_diagnostics.remove("result");
    routes[1].draft_diagnostics.remove("result");

    restore_legacy_route_construction_diagnostics(&mut routes);

    assert_eq!(required_route_construction_ordinal(&routes[0]), 0);
    assert_eq!(required_route_construction_ordinal(&routes[1]), 1);
    for route in &routes {
        assert!(matches!(
            route
                .draft_diagnostics
                .get("result")
                .map(TraceJsonValue::tree),
            Some(TraceJsonTree::String(result)) if result == "success"
        ));
    }
}

#[test]
fn legacy_route_ordinal_restore_preserves_recorded_append_order() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 344,
    };
    let mut routes = [
        group_move_route_fixture(actor, "move", 0),
        group_move_route_fixture(actor, "move", 1),
    ];

    restore_legacy_route_construction_diagnostics(&mut routes);

    assert_eq!(required_route_construction_ordinal(&routes[0]), 0);
    assert_eq!(required_route_construction_ordinal(&routes[1]), 1);
}

#[test]
#[should_panic(expected = "schema-16 route event lacks an unsigned ordinal")]
fn current_route_event_without_ordinal_remains_invalid() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 344,
    };
    let mut route = group_move_route_fixture(actor, "move", 0);
    route.draft_diagnostics.remove("ordinal");

    required_route_construction_ordinal(&route);
}

fn group_move_route_map(max_gate: u32) -> EntityMap {
    EntityMap {
        entities: BTreeMap::new(),
        entities_by_creation_order: BTreeMap::new(),
        sectors: BTreeMap::new(),
        sector_indices: BTreeMap::new(),
        gates: (0..=max_gate)
            .map(robin_engine::gate::DoorIndex::from)
            .collect(),
        runtime_creation_order_boundary: 0,
    }
}

fn group_move_sector_kinds(
    max_sector: u16,
    door: Option<(u16, u32)>,
) -> Vec<LegacyGridSectorAsset> {
    let mut sectors = vec![LegacyGridSectorAsset::NullOrOrdinary; usize::from(max_sector) + 1];
    if let Some((sector, gate_index)) = door {
        sectors[usize::from(sector)] = LegacyGridSectorAsset::Door { gate_index };
    }
    sectors
}

#[test]
fn current_schema_group_move_recovers_ordinary_route_over_door_overlay() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 344,
    };
    let command = TraceCommand::GroupMove {
        actors: vec![actor],
        destination: TracePoint {
            x: TraceFloat {
                bits: 357.031_98_f32.to_bits(),
            },
            y: TraceFloat {
                bits: 714.0_f32.to_bits(),
            },
        },
        running: false,
        show_marker: true,
        goal_sector: 117,
        goal_layer: 8,
    };
    let mut route = group_move_route_fixture(actor, "move", 0);
    route.gates.push(TraceRouteGate {
        gate_id: 53,
        direct: false,
        sector_out: 64,
        level_out: 4,
        sector_in: 290,
        level_in: 13,
        draft_diagnostics: BTreeMap::new(),
    });
    let routes = [route];
    let mut consumed = BTreeSet::new();
    let map = group_move_route_map(53);
    let sectors = group_move_sector_kinds(292, Some((292, 53)));

    assert_eq!(
        resolve_current_group_move_route(&command, &routes, &mut consumed, &map, &sectors,),
        Some(ReplayGroupMoveResolution {
            door_route: false,
            unmapped_goal_search_sector: Some(64),
            recorded_gate_routes: vec![(actor, vec![(53, false)])],
            recorded_failed_gate_routes: Vec::new(),
        })
    );
    assert_eq!(consumed, BTreeSet::from([0]));
}

#[test]
fn current_schema_same_sector_group_move_retains_ordinary_goal_kind() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 320,
    };
    let command = TraceCommand::GroupMove {
        actors: vec![actor],
        destination: TracePoint {
            x: TraceFloat {
                bits: 2533.968_f32.to_bits(),
            },
            y: TraceFloat {
                bits: 580.920_04_f32.to_bits(),
            },
        },
        running: false,
        show_marker: true,
        goal_sector: 150,
        goal_layer: 4,
    };
    let mut consumed = BTreeSet::new();

    assert_eq!(
        resolve_current_group_move_route(
            &command,
            &[],
            &mut consumed,
            &group_move_route_map(0),
            &group_move_sector_kinds(150, None),
        ),
        Some(ReplayGroupMoveResolution {
            door_route: false,
            unmapped_goal_search_sector: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: Vec::new(),
        })
    );
    assert!(consumed.is_empty());
}

#[test]
fn current_schema_group_moves_share_frame_routes_in_command_order() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 345,
    };
    let command = TraceCommand::GroupMove {
        actors: vec![actor],
        destination: TracePoint {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
        },
        running: false,
        show_marker: true,
        goal_sector: 117,
        goal_layer: 8,
    };
    let mut first = group_move_route_fixture(actor, "move", 40);
    first.gates.push(TraceRouteGate {
        gate_id: 53,
        direct: false,
        sector_out: 64,
        level_out: 4,
        sector_in: 290,
        level_in: 13,
        draft_diagnostics: BTreeMap::new(),
    });
    let mut second = group_move_route_fixture(actor, "move", 41);
    second.gates.push(TraceRouteGate {
        gate_id: 54,
        direct: true,
        sector_out: 63,
        level_out: 3,
        sector_in: 65,
        level_in: 4,
        draft_diagnostics: BTreeMap::new(),
    });
    let routes = [second, first];
    let mut consumed = BTreeSet::new();
    let map = group_move_route_map(54);
    let sectors = group_move_sector_kinds(117, None);

    let first_resolution =
        resolve_current_group_move_route(&command, &routes, &mut consumed, &map, &sectors).unwrap();
    assert_eq!(
        first_resolution.recorded_gate_routes,
        vec![(actor, vec![(53, false)])]
    );
    assert_eq!(consumed, BTreeSet::from([40]));

    let second_resolution =
        resolve_current_group_move_route(&command, &routes, &mut consumed, &map, &sectors).unwrap();
    assert_eq!(
        second_resolution.recorded_gate_routes,
        vec![(actor, vec![(54, true)])]
    );
    assert_eq!(consumed, BTreeSet::from([40, 41]));
}

#[test]
fn current_schema_group_move_recovers_internal_door_branch_from_retained_goal_kind() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 344,
    };
    let command = TraceCommand::GroupMove {
        actors: vec![actor],
        destination: TracePoint {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
        },
        running: false,
        show_marker: true,
        goal_sector: 292,
        goal_layer: 4,
    };
    let mut route = group_move_route_fixture(actor, "move", 4);
    route.goal_sector = 292;
    route.gates.push(TraceRouteGate {
        gate_id: 53,
        direct: false,
        sector_out: 64,
        level_out: 4,
        sector_in: 290,
        level_in: 13,
        draft_diagnostics: BTreeMap::new(),
    });
    let routes = [route];
    let mut consumed = BTreeSet::new();
    let map = group_move_route_map(53);
    let sectors = group_move_sector_kinds(292, Some((292, 53)));

    assert_eq!(
        resolve_current_group_move_route(&command, &routes, &mut consumed, &map, &sectors,),
        Some(ReplayGroupMoveResolution {
            door_route: true,
            unmapped_goal_search_sector: Some(64),
            recorded_gate_routes: vec![(actor, vec![(53, false)])],
            recorded_failed_gate_routes: Vec::new(),
        })
    );
}

#[test]
fn current_schema_group_move_retains_door_branch_for_failed_empty_route() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 344,
    };
    let command = TraceCommand::GroupMove {
        actors: vec![actor],
        destination: TracePoint {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
        },
        running: false,
        show_marker: true,
        goal_sector: 292,
        goal_layer: 4,
    };
    let mut route = group_move_route_fixture(actor, "move", 5);
    route.goal_sector = 292;
    route.draft_diagnostics.insert(
        "result".to_owned(),
        TraceJsonValue::from(TraceJsonTree::String("failure".to_owned())),
    );
    let routes = [route];
    let map = group_move_route_map(53);
    let sectors = group_move_sector_kinds(292, Some((292, 53)));
    let mut consumed = BTreeSet::new();

    assert_eq!(
        resolve_current_group_move_route(&command, &routes, &mut consumed, &map, &sectors,),
        Some(ReplayGroupMoveResolution {
            door_route: true,
            unmapped_goal_search_sector: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: vec![actor],
        })
    );
    assert_eq!(consumed, BTreeSet::from([5]));
}

#[test]
fn current_schema_failed_ordinary_group_move_is_authoritative() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 136,
    };
    let command = TraceCommand::GroupMove {
        actors: vec![actor],
        destination: TracePoint {
            x: TraceFloat {
                bits: 642.953_6_f32.to_bits(),
            },
            y: TraceFloat {
                bits: 730.12_f32.to_bits(),
            },
        },
        running: false,
        show_marker: true,
        goal_sector: 421,
        goal_layer: 6,
    };
    let mut route = group_move_route_fixture(actor, "move", 0);
    route.source_sector = 116;
    route.source_level = 8;
    route.goal_sector = 421;
    route.goal_level = 6;
    route.draft_diagnostics.insert(
        "result".to_owned(),
        TraceJsonValue::from(TraceJsonTree::String("failure".to_owned())),
    );
    let mut consumed = BTreeSet::new();

    assert_eq!(
        resolve_current_group_move_route(
            &command,
            &[route],
            &mut consumed,
            &group_move_route_map(0),
            &group_move_sector_kinds(421, None),
        ),
        Some(ReplayGroupMoveResolution {
            door_route: false,
            unmapped_goal_search_sector: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: vec![actor],
        })
    );
    assert_eq!(consumed, BTreeSet::from([0]));
}

#[test]
fn successful_patch_group_move_uses_terminal_gate_as_rust_search_sector() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 297,
    };
    let command = TraceCommand::GroupMove {
        actors: vec![actor],
        destination: TracePoint {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
        },
        running: false,
        show_marker: true,
        goal_sector: 492,
        goal_layer: 0,
    };
    let mut route = group_move_route_fixture(actor, "move", 0);
    route.goal_sector = 492;
    route.gates.push(TraceRouteGate {
        gate_id: 78,
        direct: true,
        sector_out: 0,
        level_out: 0,
        sector_in: 491,
        level_in: 6,
        draft_diagnostics: BTreeMap::new(),
    });
    let mut consumed = BTreeSet::new();
    let map = group_move_route_map(78);
    let sectors = group_move_sector_kinds(492, None);
    let resolution =
        resolve_current_group_move_route(&command, &[route], &mut consumed, &map, &sectors);

    assert_eq!(
        resolution,
        Some(ReplayGroupMoveResolution {
            door_route: false,
            unmapped_goal_search_sector: Some(491),
            recorded_gate_routes: vec![(actor, vec![(78, true)])],
            recorded_failed_gate_routes: Vec::new(),
        })
    );
    let map = EntityMap {
        entities: BTreeMap::new(),
        entities_by_creation_order: BTreeMap::new(),
        sectors: BTreeMap::from([(491, 55)]),
        sector_indices: BTreeMap::from([(
            491,
            robin_engine::fast_find_grid::SectorIndex::new(9).unwrap(),
        )]),
        gates: Vec::new(),
        runtime_creation_order_boundary: 0,
    };
    assert_eq!(
        map.translate_group_move_goal_sector(
            492,
            0,
            resolution.and_then(|resolution| resolution.unmapped_goal_search_sector),
        ),
        GroupMoveGoalTranslation::Runtime(
            (SectorNumber::new(55), 0),
            robin_engine::fast_find_grid::SectorIndex::new(9).unwrap(),
        )
    );
}

fn drop_ale_route_fixture(actor: TraceEntityId, target: TracePoint) -> TraceRouteConstructionEvent {
    TraceRouteConstructionEvent {
        kind: "move".to_owned(),
        actor,
        source: TracePoint {
            x: TraceFloat {
                bits: 2413.0_f32.to_bits(),
            },
            y: TraceFloat {
                bits: 802.0_f32.to_bits(),
            },
        },
        source_sector: 394,
        source_level: 6,
        goal: target,
        goal_sector: 148,
        goal_level: 4,
        gates: vec![TraceRouteGate {
            gate_id: 0,
            direct: false,
            sector_out: 148,
            level_out: 4,
            sector_in: 394,
            level_in: 6,
            draft_diagnostics: BTreeMap::new(),
        }],
        draft_diagnostics: BTreeMap::from([
            (
                "ordinal".to_owned(),
                TraceJsonValue::from(TraceJsonTree::Unsigned(7)),
            ),
            (
                "result".to_owned(),
                TraceJsonValue::from(TraceJsonTree::String("success".to_owned())),
            ),
        ]),
    }
}

fn drop_ale_route_map(actor: TraceEntityId) -> EntityMap {
    let goal_sector_index = robin_engine::fast_find_grid::SectorIndex::new(37).unwrap();
    EntityMap {
        entities: BTreeMap::from([(actor, EntityId::Pc(robin_engine::entity_id::PcId(12)))]),
        entities_by_creation_order: BTreeMap::new(),
        sectors: BTreeMap::from([(148, 55), (394, 56)]),
        sector_indices: BTreeMap::from([
            (148, goal_sector_index),
            (
                394,
                robin_engine::fast_find_grid::SectorIndex::new(38).unwrap(),
            ),
        ]),
        gates: vec![robin_engine::gate::DoorIndex::from(42)],
        runtime_creation_order_boundary: 0,
    }
}

#[test]
#[should_panic(expected = "schema-16 DropAle route has invalid result: None")]
fn current_drop_ale_route_without_result_remains_invalid() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 320,
    };
    let target = TracePoint {
        x: TraceFloat { bits: 0 },
        y: TraceFloat { bits: 0 },
    };
    let mut route = drop_ale_route_fixture(actor, target);
    route.draft_diagnostics.remove("result");

    recorded_gate_path_from_event(&route, &drop_ale_route_map(actor));
}

#[test]
fn current_schema_drop_ale_recovers_save067_route_goal() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 320,
    };
    let target = TracePoint {
        x: TraceFloat {
            bits: 2_607.467_f32.to_bits(),
        },
        y: TraceFloat {
            bits: 881.610_5_f32.to_bits(),
        },
    };
    let command = TraceCommand::DropAleAt {
        actor,
        target,
        running: false,
    };
    let routes = [drop_ale_route_fixture(actor, target)];
    let mut consumed = BTreeSet::new();

    assert_eq!(
        resolve_current_drop_ale(
            &command,
            &routes,
            &mut consumed,
            &drop_ale_route_map(actor),
            None,
            false,
        ),
        Some(ReplayDropAleResolution {
            goal: (SectorNumber::new(55), 4),
            goal_sector_index: robin_engine::fast_find_grid::SectorIndex::new(37),
            recorded_gate_path: Some(robin_engine::gate::RecordedGatePath {
                source_sector: SectorNumber::new(56),
                source_sector_index: robin_engine::fast_find_grid::SectorIndex::new(38),
                source_layer: 6,
                outcome: robin_engine::gate::RecordedGateOutcome::Success(vec![
                    robin_engine::gate::GatePathStep {
                        door_index: robin_engine::gate::DoorIndex::from(42),
                        direct: false,
                    },
                ]),
            }),
        })
    );
    assert_eq!(consumed, BTreeSet::from([7]));
}

#[test]
#[should_panic(expected = "already consumed by the group-move join")]
fn current_schema_route_ordinal_cannot_be_claimed_by_two_joiners() {
    claim_delayed_drop_ale_route_ordinal(7, &mut BTreeSet::new(), &BTreeSet::from([7]));
}

#[test]
#[should_panic(expected = "matched twice")]
fn current_schema_delayed_route_ordinal_cannot_be_claimed_twice() {
    let mut delayed = BTreeSet::from([7]);
    claim_delayed_drop_ale_route_ordinal(7, &mut delayed, &BTreeSet::new());
}

#[test]
fn delayed_drop_ale_ignores_route_without_staged_seek() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 320,
    };
    let target = TracePoint {
        x: TraceFloat {
            bits: 2_607.467_f32.to_bits(),
        },
        y: TraceFloat {
            bits: 881.610_5_f32.to_bits(),
        },
    };

    let mut consumed = BTreeSet::new();
    let routes = collect_current_delayed_drop_ale_routes_matching(
        &[drop_ale_route_fixture(actor, target)],
        &mut consumed,
        &BTreeSet::new(),
        &drop_ale_route_map(actor),
        |_, _| false,
    );

    assert!(routes.is_empty());
    assert!(consumed.is_empty());
}

#[test]
fn current_schema_delayed_drop_ale_retains_recorded_failure_outcome() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 320,
    };
    let target = TracePoint {
        x: TraceFloat {
            bits: 2_607.467_f32.to_bits(),
        },
        y: TraceFloat {
            bits: 881.610_5_f32.to_bits(),
        },
    };
    let mut event = drop_ale_route_fixture(actor, target);
    event.gates.clear();
    event.draft_diagnostics.insert(
        "result".to_owned(),
        TraceJsonValue::from(TraceJsonTree::String("failure".to_owned())),
    );
    let mut consumed = BTreeSet::new();

    let routes = collect_current_delayed_drop_ale_routes_matching(
        &[event],
        &mut consumed,
        &BTreeSet::new(),
        &drop_ale_route_map(actor),
        |runtime_actor, destination| {
            runtime_actor == EntityId::Pc(robin_engine::entity_id::PcId(12))
                && destination.x.to_bits() == target.x.bits
                && destination.y.to_bits() == target.y.bits
        },
    );

    assert_eq!(consumed, BTreeSet::from([7]));
    assert_eq!(routes.len(), 1);
    assert!(matches!(
        &routes[0].recorded_gate_path.outcome,
        robin_engine::gate::RecordedGateOutcome::Failure
    ));
}

#[test]
#[should_panic(expected = "matched 2 exact route events")]
fn current_schema_drop_ale_rejects_duplicate_exact_command_routes() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 0,
    };
    let target = TracePoint {
        x: TraceFloat {
            bits: 2_607.467_f32.to_bits(),
        },
        y: TraceFloat {
            bits: 881.610_5_f32.to_bits(),
        },
    };
    let command = TraceCommand::DropAleAt {
        actor,
        target,
        running: false,
    };
    let first = drop_ale_route_fixture(actor, target);
    let mut second = drop_ale_route_fixture(actor, target);
    second.draft_diagnostics.insert(
        "ordinal".to_owned(),
        TraceJsonValue::from(TraceJsonTree::Unsigned(8)),
    );
    resolve_current_drop_ale(
        &command,
        &[first, second],
        &mut BTreeSet::new(),
        &drop_ale_route_map(actor),
        None,
        false,
    );
}

#[test]
fn drop_ale_route_recovery_rejects_nonmatching_point() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 320,
    };
    let target = TracePoint {
        x: TraceFloat { bits: 0x4522_f77a },
        y: TraceFloat { bits: 0x445c_6712 },
    };
    let command = TraceCommand::DropAleAt {
        actor,
        target,
        running: false,
    };
    let mut wrong_target = target;
    wrong_target.x.bits ^= 1;
    let routes = [drop_ale_route_fixture(actor, wrong_target)];
    let map = drop_ale_route_map(actor);
    let mut consumed = BTreeSet::new();

    assert_eq!(
        resolve_current_drop_ale(&command, &routes, &mut consumed, &map, None, false),
        None
    );
    assert!(consumed.is_empty());
}

#[test]
#[should_panic(expected = "has no retained Rust position-sector mapping")]
fn current_schema_drop_ale_rejects_unmapped_authoritative_goal() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 320,
    };
    let target = TracePoint {
        x: TraceFloat { bits: 0x4522_f77a },
        y: TraceFloat { bits: 0x445c_6712 },
    };
    let command = TraceCommand::DropAleAt {
        actor,
        target,
        running: false,
    };
    let mut map = drop_ale_route_map(actor);
    map.sectors.clear();

    let _ = resolve_current_drop_ale(
        &command,
        &[drop_ale_route_fixture(actor, target)],
        &mut BTreeSet::new(),
        &map,
        None,
        false,
    );
}

#[test]
fn current_schema_drop_ale_recovers_same_sector_actor_goal_without_route() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 320,
    };
    let command = TraceCommand::DropAleAt {
        actor,
        target: TracePoint {
            x: TraceFloat { bits: 0x44d7_a800 },
            y: TraceFloat { bits: 0x4447_8000 },
        },
        running: false,
    };
    let expected = ReplayDropAleResolution {
        goal: (SectorNumber::new(0), 0),
        goal_sector_index: robin_engine::fast_find_grid::SectorIndex::new(0),
        recorded_gate_path: None,
    };
    let mut consumed = BTreeSet::new();

    assert_eq!(
        resolve_current_drop_ale(
            &command,
            &[],
            &mut consumed,
            &drop_ale_route_map(actor),
            Some(expected.clone()),
            false,
        ),
        Some(expected)
    );
    assert!(consumed.is_empty());
}

#[test]
fn legacy_drop_ale_actor_fallback_requires_exact_same_sector() {
    use robin_engine::fast_find_grid::SectorIndex;
    use robin_engine::position_interface::SectorHandle;

    let actor = SectorHandle::new(50)
        .unwrap()
        .with_arena_index(SectorIndex::new(50).unwrap());
    let same = SectorHandle::new(50)
        .unwrap()
        .with_arena_index(SectorIndex::new(50).unwrap());
    let repeated_public_number = SectorHandle::new(50)
        .unwrap()
        .with_arena_index(SectorIndex::new(150).unwrap());
    let cross_sector = SectorHandle::new(0)
        .unwrap()
        .with_arena_index(SectorIndex::new(0).unwrap());
    let number_only = SectorHandle::new(50).unwrap();

    assert!(legacy_drop_ale_target_is_same_exact_sector(actor, same));
    assert!(!legacy_drop_ale_target_is_same_exact_sector(
        actor,
        repeated_public_number
    ));
    assert!(!legacy_drop_ale_target_is_same_exact_sector(
        actor,
        cross_sector
    ));
    assert!(!legacy_drop_ale_target_is_same_exact_sector(
        actor,
        number_only
    ));
}

#[test]
#[should_panic(
    expected = "schema-16 DropAle recorded as a quick action has no authoritative target-sector identity"
)]
fn current_schema_drop_ale_qa_rejects_actor_sector_as_a_fake_target_fallback() {
    let actor = TraceEntityId {
        kind: TraceEntityKind::Pc,
        index: 320,
    };
    let command = TraceCommand::DropAleAt {
        actor,
        target: TracePoint {
            x: TraceFloat { bits: 0x44d7_a800 },
            y: TraceFloat { bits: 0x4447_8000 },
        },
        running: false,
    };
    let fake_actor_goal = ReplayDropAleResolution {
        goal: (SectorNumber::new(0), 0),
        goal_sector_index: robin_engine::fast_find_grid::SectorIndex::new(0),
        recorded_gate_path: None,
    };

    let _ = resolve_current_drop_ale(
        &command,
        &[],
        &mut BTreeSet::new(),
        &drop_ale_route_map(actor),
        Some(fake_actor_goal),
        true,
    );
}

#[test]
fn runtime_identity_ignores_only_numeric_gaps_not_persistent_reordering() {
    let original_projectile = TraceEntityId {
        kind: TraceEntityKind::Projectile,
        index: 131,
    };
    let original_bonus = TraceEntityId {
        kind: TraceEntityKind::Bonus,
        index: 132,
    };
    let rust_projectile = EntityId::Projectile(robin_engine::entity_id::ProjectileId(40));
    let rust_bonus = EntityId::Bonus(robin_engine::entity_id::BonusId(12));
    let originals = vec![
        (original_projectile, 172, EntityIdKind::Projectile),
        (original_bonus, 174, EntityIdKind::Bonus),
    ];

    let shifted = pair_runtime_identities_by_persistent_rank(
        originals.clone(),
        vec![
            (rust_projectile, 170, EntityIdKind::Projectile),
            (rust_bonus, 171, EntityIdKind::Bonus),
        ],
    )
    .unwrap();
    assert_eq!(
        shifted,
        vec![
            (original_projectile, 172, rust_projectile),
            (original_bonus, 174, rust_bonus),
        ]
    );

    let reordered = pair_runtime_identities_by_persistent_rank(
        originals,
        vec![
            (rust_bonus, 170, EntityIdKind::Bonus),
            (rust_projectile, 171, EntityIdKind::Projectile),
        ],
    );
    assert!(
        reordered
            .unwrap_err()
            .contains("persistent creation rank 0")
    );
}

#[test]
fn automatic_dump_window_retains_configured_prior_frames_and_current_frame() {
    let mut frames = VecDeque::new();
    for frame in 0..50 {
        push_rolling_window(&mut frames, frame);
    }
    assert_eq!(
        frames.into_iter().collect::<Vec<_>>(),
        (17..50).collect::<Vec<_>>()
    );
}

use super::*;
use syn::visit::Visit;

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Calls(Vec<String>);

impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            self.0
                .push(path.path.segments.last().unwrap().ident.to_string());
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.0.push(call.method.to_string());
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn assert_phase_order(method: &str, expected: &[&str]) {
    let source = syn::parse_file(include_str!("../../movement.rs")).unwrap();
    let function = source
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item) => Some(&item.items),
            _ => None,
        })
        .flatten()
        .find_map(|item| match item {
            syn::ImplItem::Fn(function) if function.sig.ident == method => Some(function),
            _ => None,
        })
        .unwrap();
    let mut calls = Calls::default();
    calls.visit_block(&function.block);
    let actual: Vec<_> = calls
        .0
        .iter()
        .map(String::as_str)
        .filter(|call| expected.contains(call))
        .collect();
    assert_eq!(actual, expected, "{method}");
}

#[test]
fn owner_snapshots_are_live_and_execution_precedes_outcome_callbacks() {
    assert_phase_order(
        "tick_entity_movement_owner",
        &[
            "prepare_movement_owner_execution",
            "live_mobile_geometry",
            "prepare_movement_owner_prepass",
            "snapshot_all",
            "tick_one_movement_actor",
            "apply_movement_owner_outcome",
        ],
    );
    assert_phase_order(
        "prepare_movement_owner_prepass",
        &[
            "combat_face_target_for_owner",
            "turn_movement_owner_drunken",
            "snapshot_movement_owner_lift",
        ],
    );
}

#[test]
fn synchronous_execute_tails_crossings_and_terminal_handoffs_stay_ordered() {
    assert_phase_order(
        "apply_movement_owner_outcome",
        &[
            "sync_walking_corpse_for_carrier",
            "launch_perform_seek_arrivals",
            "quit_swordfight_with_far_opponents",
            "apply_sword_movement_start_initiative_transfer",
            "launch_sword_movement_termination_provoke",
            "apply_movement_door_transition_effects",
            "refresh_movement_transition_seeks",
            "dispatch_galopp_loop_event",
            "dispatch_movement_owner_crossings",
            "clear_terminal_door_pass_goal",
            "abort_pinched_pc_sword_movement",
            "execute_pass_door",
            "commit_completed_door_pass_position",
            "apply_completed_door_pass_lift_entry_state",
            "insert_door_pass_successor",
            "tick_shouldered_carry_ceiling",
            "drain_script_synchronous_actions",
            "advance_live_order_after_terminal_handoff",
            "pop_selected_movement_order",
            "dispatch_condolations_for_owner_boundary",
            "drain_script_synchronous_actions",
        ],
    );
    assert_phase_order(
        "dispatch_movement_owner_crossings",
        &[
            "resolve_cross_checks",
            "resolve_cross_checks",
            "check_for_line_crossing",
            "update_roll_after_crossing",
            "check_for_non_elevation_line_crossing",
        ],
    );
}

#[test]
fn absent_and_stale_selections_do_not_run_movement_or_completion() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(crate::element::Entity::Pc(
        crate::engine::test_support::actors::unbound_pc(crate::element::Posture::Upright),
    ));
    let order_id = engine.orders.allocate_order_id();
    let mut movement = crate::sequence::SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(owner),
        OrderType::WalkingUpright,
    );
    movement.orders.push_back(crate::order::Order::new(
        OrderType::WalkingUpright,
        100.0,
        100.0,
        order_id,
    ));
    let seq_id = engine.orders.sequence_manager.launch_element(movement);
    let stale = MovementOwnerSelection {
        seq_id,
        elem_idx: 0,
        order_id: std::num::NonZeroU32::new(order_id.get().checked_add(1).unwrap()).unwrap(),
    };
    let before = engine
        .get_entity(owner)
        .unwrap()
        .element_data()
        .position_map();
    for selected in [None, Some(stale)] {
        let result = engine.tick_entity_movement_owner(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            selected,
        );
        assert!(result.initial.is_none());
        assert!(result.post_completion_override.is_none());
        assert!(result.terminal_order_pops.is_empty());
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .position_map(),
            before
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_id,
            order_id
        );
    }
}

use super::{ForecastedDestination, Position, PreparedForecastDestination};
use crate::sim_rng::{RngSite, SimulationContext, with_draw_trace};

#[test]
fn building_exit_rejects_entry_against_the_full_ordered_gate_list() {
    let prepared = PreparedForecastDestination {
        fallback: ForecastedDestination {
            position: Position::default(),
            direction: 0,
        },
        building_gates: vec![
            ForecastedDestination {
                position: Position {
                    x: 10.0,
                    ..Position::default()
                },
                direction: 1,
            },
            ForecastedDestination {
                position: Position {
                    x: 20.0,
                    ..Position::default()
                },
                direction: 2,
            },
            ForecastedDestination {
                position: Position {
                    x: 30.0,
                    ..Position::default()
                },
                direction: 3,
            },
        ],
        entry_gate: Some(1),
        direction_written: true,
    };

    let seed = (0..10_000)
        .find(|seed| {
            let sim = SimulationContext::with_seed(*seed);
            crate::sim_rng::usize(&sim, RngSite::BuildingExitGate, ..3) == 1
                && crate::sim_rng::usize(&sim, RngSite::BuildingExitGate, ..3) != 1
        })
        .expect("find a deterministic entry-then-exit draw sequence");
    let expected_sim = SimulationContext::with_seed(seed);
    assert_eq!(
        crate::sim_rng::usize(&expected_sim, RngSite::BuildingExitGate, ..3),
        1
    );
    let expected_exit = crate::sim_rng::usize(&expected_sim, RngSite::BuildingExitGate, ..3);

    let sim = SimulationContext::with_seed(seed);
    let (resolved, trace) = with_draw_trace(|| prepared.resolve(&sim));
    assert_eq!(
        resolved.position.x,
        prepared.building_gates[expected_exit].position.x
    );
    assert_eq!(
        resolved.direction, prepared.building_gates[expected_exit].direction,
        "rejection must retain the Original all-gates index mapping"
    );
    assert_eq!(
        trace,
        vec![RngSite::BuildingExitGate, RngSite::BuildingExitGate],
        "selecting the entry gate must consume another authoritative draw"
    );
}

#[test]
fn building_exit_without_a_live_entry_gate_accepts_the_first_draw() {
    let prepared = PreparedForecastDestination {
        fallback: ForecastedDestination {
            position: Position::default(),
            direction: 0,
        },
        building_gates: vec![
            ForecastedDestination {
                position: Position {
                    x: 10.0,
                    ..Position::default()
                },
                direction: 1,
            },
            ForecastedDestination {
                position: Position {
                    x: 20.0,
                    ..Position::default()
                },
                direction: 2,
            },
        ],
        entry_gate: None,
        direction_written: true,
    };

    let sim = SimulationContext::with_seed(17);
    let expected_sim = SimulationContext::with_seed(17);
    let expected = crate::sim_rng::usize(&expected_sim, RngSite::BuildingExitGate, ..2);
    let (resolved, trace) = with_draw_trace(|| prepared.resolve(&sim));

    assert_eq!(
        resolved.position.x,
        prepared.building_gates[expected].position.x
    );
    assert_eq!(trace, vec![RngSite::BuildingExitGate]);
}

#[test]
fn one_gate_building_keeps_the_callers_direction_output() {
    // The building branch
    // but assigns direction only when there is more than one gate.
    let prepared = PreparedForecastDestination {
        fallback: ForecastedDestination {
            position: Position {
                x: 985.0,
                y: 2597.0,
                ..Position::default()
            },
            // The target currently faces 10, but this value must not
            // overwrite the caller-owned output on the one-gate branch.
            direction: 10,
        },
        building_gates: Vec::new(),
        entry_gate: None,
        direction_written: false,
    };

    let sim = SimulationContext::with_seed(1);
    let (resolved, trace) = with_draw_trace(|| prepared.resolve_retaining_direction(&sim, 0));

    assert_eq!(resolved.position.x, 985.0);
    assert_eq!(resolved.direction, 0);
    assert!(trace.is_empty());
}

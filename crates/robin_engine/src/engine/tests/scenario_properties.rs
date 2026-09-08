//! Exhaustive bounded histories: failures print the history/checkpoint so the
//! smallest failing prefix can be retained as a focused regression.
use super::*;
use crate::element::Command;
use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

fn advance(engine: &mut EngineInner, assets: &LevelAssets) {
    engine.perform_hourglass(
        &mut HostDisplayState::default(),
        &mut InputState::default(),
        assets,
        &mut DevState::default(),
    );
}

fn operation(engine: &mut EngineInner, assets: &LevelAssets, operation: usize) {
    match operation {
        0 => advance(engine, assets),
        1 | 2 => {
            let mut sequence = Sequence::new();
            for level in 1..=2 {
                let mut timer = SequenceElement::new_generic(level, Command::Timer, None);
                timer.set_property(Field::Timer, FieldValue::Integer(operation as u32));
                sequence.append_element(timer);
            }
            engine.orders.sequence_manager.launch_sequence(sequence);
        }
        _ => unreachable!("bounded operation alphabet"),
    }
}

#[test]
fn bounded_timer_histories_continue_identically_after_every_checkpoint() {
    const LENGTH: usize = 4;
    let assets = LevelAssets::new();
    // Enumerate all 3^4 histories instead of relying on random coverage or a
    // separate generator RNG. Each history has five possible restore points.
    for encoded in 0..3_usize.pow(LENGTH as u32) {
        let mut digits = encoded;
        let history: [usize; LENGTH] = std::array::from_fn(|_| {
            let operation = digits % 3;
            digits /= 3;
            operation
        });
        for checkpoint in 0..=LENGTH {
            let mut baseline = EngineInner::new();
            let mut restored = baseline.clone();
            for step in 0..=LENGTH {
                if step == checkpoint {
                    let bytes = super::super::snapshot::encode_native_engine_inner(&restored);
                    restored = super::super::snapshot::decode_native_engine_inner(&bytes)
                        .expect("valid timer scenario round trips");
                }
                if step < LENGTH {
                    operation(&mut baseline, &assets, history[step]);
                    operation(&mut restored, &assets, history[step]);
                }
                assert_eq!(
                    crate::replay::state_hash(&baseline),
                    crate::replay::state_hash(&restored),
                    "history={history:?} checkpoint={checkpoint} step={step}"
                );
            }
            // Observe future timer completion and Ready-to-successor behavior,
            // rather than asserting only an immediately restored value.
            for tail in 0..8 {
                advance(&mut baseline, &assets);
                advance(&mut restored, &assets);
                assert_eq!(
                    crate::replay::state_hash(&baseline),
                    crate::replay::state_hash(&restored),
                    "history={history:?} checkpoint={checkpoint} tail={tail}"
                );
            }
            assert!(
                baseline.orders.timer_elements.is_empty(),
                "history={history:?} timers did not complete"
            );
            // Completed sequences can remain until Friday-evening cleanup;
            // completion, not immediate registry removal, is the contract.
            assert!(
                baseline
                    .orders
                    .sequence_manager
                    .sequences_iter()
                    .all(Sequence::is_to_be_deleted),
                "history={history:?} timer chains did not complete"
            );
        }
    }
}

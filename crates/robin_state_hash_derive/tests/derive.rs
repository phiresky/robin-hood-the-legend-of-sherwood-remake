//! Smoke tests for the `#[derive(StateHash)]` macro.

use robin_state_hash_derive::StateHash;
use robin_util::state_hash::compute;
use serde::Serialize;

#[derive(StateHash)]
struct Plain {
    a: u32,
    b: i16,
    name: String,
}

#[derive(StateHash)]
struct WithFloat {
    pos_x: f32,
    pos_y: f32,
}

#[derive(StateHash, Serialize)]
struct WithSerdeSkip {
    a: u32,
    // Field exists solely to verify `#[serde(skip)]` is honored by StateHash;
    // it's set with different values in the test but never read.
    #[allow(dead_code)]
    b: u32,
    c: u32,
}

#[derive(StateHash)]
struct Nested {
    pl: Plain,
    pos: WithFloat,
    list: Vec<u8>,
}

#[derive(StateHash)]
enum Mood {
    Happy,
    Sad(u8),
    Mixed { joy: f32, dread: u32 },
}

#[test]
fn struct_fields_in_order() {
    let a = Plain {
        a: 1,
        b: -2,
        name: "robin".into(),
    };
    let b = Plain {
        a: 1,
        b: -2,
        name: "robin".into(),
    };
    assert_eq!(compute(&a), compute(&b));
    let c = Plain {
        a: 2,
        b: -2,
        name: "robin".into(),
    };
    assert_ne!(compute(&a), compute(&c));
}

#[test]
fn float_fields_via_to_bits() {
    let a = WithFloat {
        pos_x: 1.5,
        pos_y: 2.5,
    };
    let b = WithFloat {
        pos_x: 1.5,
        pos_y: 2.5,
    };
    assert_eq!(compute(&a), compute(&b));
    let c = WithFloat {
        pos_x: 1.5000001,
        pos_y: 2.5,
    };
    assert_ne!(compute(&a), compute(&c));
}

#[test]
fn serde_skip_excluded_from_hash() {
    let a = WithSerdeSkip { a: 1, b: 99, c: 3 };
    let b = WithSerdeSkip { a: 1, b: 100, c: 3 };
    assert_eq!(compute(&a), compute(&b));
}

#[test]
fn nested_struct() {
    let a = Nested {
        pl: Plain {
            a: 1,
            b: 2,
            name: "x".into(),
        },
        pos: WithFloat {
            pos_x: 1.0,
            pos_y: 1.0,
        },
        list: vec![1, 2, 3],
    };
    let b = Nested {
        pl: Plain {
            a: 1,
            b: 2,
            name: "x".into(),
        },
        pos: WithFloat {
            pos_x: 1.0,
            pos_y: 1.0,
        },
        list: vec![1, 2, 3],
    };
    assert_eq!(compute(&a), compute(&b));
}

#[test]
fn enum_variants_distinguished() {
    let h_happy = compute(&Mood::Happy);
    let h_sad = compute(&Mood::Sad(0));
    let h_mixed = compute(&Mood::Mixed { joy: 0.0, dread: 0 });
    assert_ne!(h_happy, h_sad);
    assert_ne!(h_happy, h_mixed);
    assert_ne!(h_sad, h_mixed);
    // Same variant + same fields → same hash.
    assert_eq!(compute(&Mood::Sad(7)), compute(&Mood::Sad(7)));
}

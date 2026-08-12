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
    #[serde(skip)]
    b: u32,
    c: u32,
}

#[derive(StateHash, Serialize)]
struct WithExplicitHashSkip {
    a: u32,
    #[state_hash(skip)]
    serialized_but_unhashed: u32,
}

#[derive(StateHash, Serialize)]
enum WithSkippedEnumFields {
    Tuple(u32, #[serde(skip)] u32, u32),
    Named {
        a: u32,
        #[serde(skip_serializing)]
        transient: u32,
        c: u32,
    },
}

#[derive(StateHash)]
struct NoFields;

#[derive(StateHash)]
struct OneSkippedField(#[state_hash(skip)] ());

#[derive(StateHash)]
struct Nested {
    pl: Plain,
    pos: WithFloat,
    list: Vec<u8>,
}

// A struct-variant field named `state` must not shadow the generated
// hasher parameter; the derive renames variant bindings to avoid that.
#[derive(StateHash)]
enum ShadowProne {
    Named { state: u32, other: u32 },
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
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap()
    );
    assert_eq!(compute(&a), compute(&b));
}

#[test]
fn explicit_hash_skip_does_not_change_serialization() {
    let a = WithExplicitHashSkip {
        a: 1,
        serialized_but_unhashed: 10,
    };
    let b = WithExplicitHashSkip {
        a: 1,
        serialized_but_unhashed: 20,
    };

    assert_ne!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap()
    );
    assert_eq!(compute(&a), compute(&b));
}

#[test]
fn enum_tuple_skip_matches_struct_skip_semantics() {
    let a = WithSkippedEnumFields::Tuple(1, 10, 3);
    let b = WithSkippedEnumFields::Tuple(1, 20, 3);

    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap()
    );
    assert_eq!(compute(&a), compute(&b));
}

#[test]
fn enum_named_skip_serializing_matches_struct_skip_semantics() {
    let a = WithSkippedEnumFields::Named {
        a: 1,
        transient: 10,
        c: 3,
    };
    let b = WithSkippedEnumFields::Named {
        a: 1,
        transient: 20,
        c: 3,
    };

    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap()
    );
    assert_eq!(compute(&a), compute(&b));
}

#[test]
fn skipped_field_has_an_explicit_hash_schema_marker() {
    assert_ne!(compute(&NoFields), compute(&OneSkippedField(())));
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
fn struct_variant_field_named_state_compiles_and_hashes() {
    let a = ShadowProne::Named { state: 1, other: 2 };
    let b = ShadowProne::Named { state: 1, other: 2 };
    let c = ShadowProne::Named { state: 9, other: 2 };
    assert_eq!(compute(&a), compute(&b));
    assert_ne!(compute(&a), compute(&c));
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

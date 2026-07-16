//! Integration tests for `robin_engine::vm` decoder that need the
//! `robin_assets::scb` parser. They were `#[cfg(any())]`-gated in-crate
//! while the parser lived here; now they run as integration tests.

mod support;

use robin_assets::scb;
use robin_engine::vm::{DecodeError, decode};
use support::{data_directory, data_file};

/// Every quad in every class of the shipped demo script should
/// decode cleanly. If the demo uses an opcode value we haven't
/// mapped, this fires.
///
/// Original provenance: `original-code/virtualmachine/SCSerialize.cpp:420-450`
/// reads the shipped SCB header and indexes every serialized class.
#[test]
#[ignore = "requires Leicester demo data via ROBINHOOD_DATA_DIR; see README.md"]
fn decodes_every_quad_in_demo_script() {
    let path = data_file("Data/Levels/Dem_Lei_MP.scb");
    let scb = scb::parse_file(&path).unwrap();
    let mut decoded = 0;
    let mut unknown = std::collections::BTreeSet::new();
    for class in &scb.classes {
        for q in &class.quads {
            match decode(*q) {
                Ok(_) => decoded += 1,
                Err(DecodeError::UnknownOpcode(b)) => {
                    unknown.insert(b);
                }
            }
        }
    }
    assert!(decoded > 0, "should have decoded quads");
    // The VM runtime force-rewrites opcodes 58, 107, 208, 229 to
    // Q_EMPTY at runtime as a known workaround — allow those specific
    // bytes through. Anything else is a real problem.
    let allowed: std::collections::BTreeSet<u8> = [58, 107, 208, 229].into_iter().collect();
    let unexpected: Vec<_> = unknown.difference(&allowed).copied().collect();
    assert!(
        unexpected.is_empty(),
        "demo script uses unexpected opcodes: {unexpected:?} \
         (decoded {decoded}, known-workaround bytes {unknown:?})"
    );
}

/// Decode every quad in all 39 full-game .scb files. Verifies our
/// opcode table covers the entire game, not just the demo.
///
/// Original provenance: `original-code/virtualmachine/SCSerialize.cpp:420-450`
/// reads the shipped SCB header and indexes every serialized class.
#[test]
#[ignore = "requires full-game data via ROBINHOOD_DATA_DIR; see README.md"]
fn decodes_every_quad_in_all_fullgame_scripts() {
    let levels_dir = data_directory("Data/Levels");

    let allowed: std::collections::BTreeSet<u8> = [58, 107, 208, 229].into_iter().collect();
    let mut total_decoded = 0usize;
    let mut total_unknown = std::collections::BTreeMap::<u8, usize>::new();
    let mut scripts = 0;

    for entry in std::fs::read_dir(&levels_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("scb") {
            continue;
        }
        let scb = scb::parse_file(&path).unwrap();
        for class in &scb.classes {
            for q in &class.quads {
                match decode(*q) {
                    Ok(_) => total_decoded += 1,
                    Err(DecodeError::UnknownOpcode(b)) => {
                        *total_unknown.entry(b).or_insert(0) += 1;
                    }
                }
            }
        }
        scripts += 1;
    }

    assert!(scripts > 0, "should have found .scb files");
    let unexpected: Vec<_> = total_unknown
        .keys()
        .filter(|k| !allowed.contains(k))
        .copied()
        .collect();
    assert!(
        unexpected.is_empty(),
        "fullgame uses unexpected opcodes: {unexpected:?} \
         (decoded {total_decoded} across {scripts} scripts, \
         known-workaround counts {total_unknown:?})"
    );
}

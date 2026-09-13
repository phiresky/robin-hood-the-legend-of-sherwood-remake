//! Isomorphic mapping between Original and Rust gate-array identities.
//!
//! Original appends proto building/lift doors, proto jump gates, and then
//! mission reinforcement doors to the spatial grid's gate collection. Rust creates
//! the same objects, but installs reinforcement doors before its deferred jump
//! gate attachment pass. Consequently the mixed array index is not a stable
//! cross-engine identity. Construction order *within* the stateful-door and
//! stateless-jump subsequences is stable in both engines.

use crate::{
    engine::LegacyGridGateAsset,
    gate::{Door, DoorIndex, GateType},
};

use super::adopt_common::{AdoptErrorKind, LegacyAdoptError};

/// Map each Original mixed gate-array slot to its Rust runtime gate.
///
/// Matching by kind ordinal is strict: every retained stateful door must have
/// one `GateType::Door` peer and every retained stateless jump gate must have
/// one `GateType::Jump` peer, with no runtime objects left over.
pub fn derive_legacy_gate_order(
    retained: &[LegacyGridGateAsset],
    runtime: &[Door],
) -> Result<Vec<DoorIndex>, LegacyAdoptError> {
    let retained_doors = retained
        .iter()
        .filter(|gate| matches!(gate, LegacyGridGateAsset::Door))
        .count();
    let retained_jumps = retained.len() - retained_doors;
    let runtime_doors = runtime
        .iter()
        .filter(|gate| gate.gate_type == GateType::Door)
        .count();
    let runtime_jumps = runtime
        .iter()
        .filter(|gate| gate.gate_type == GateType::Jump)
        .count();

    if let Some((index, gate)) = runtime
        .iter()
        .enumerate()
        .find(|(_, gate)| !matches!(gate.gate_type, GateType::Door | GateType::Jump))
    {
        return Err(AdoptErrorKind::UnsupportedRuntimeGateKind {
            index,
            kind: gate.gate_type,
        }
        .into());
    }

    let mut door_indices = runtime
        .iter()
        .enumerate()
        .filter_map(|(index, gate)| (gate.gate_type == GateType::Door).then_some(index));
    let mut jump_indices = runtime
        .iter()
        .enumerate()
        .filter_map(|(index, gate)| (gate.gate_type == GateType::Jump).then_some(index));
    let mut mapped = Vec::with_capacity(retained.len());

    for (saved_index, gate) in retained.iter().enumerate() {
        let (kind, runtime_index) = match gate {
            LegacyGridGateAsset::Door => ("door", door_indices.next()),
            LegacyGridGateAsset::Stateless => ("jump gate", jump_indices.next()),
        };
        let runtime_index = runtime_index.ok_or(AdoptErrorKind::GateMissingPeer {
            kind,
            saved_index,
            retained_doors,
            retained_jumps,
            runtime_doors,
            runtime_jumps,
        })?;
        let runtime_index =
            u32::try_from(runtime_index).map_err(|_| AdoptErrorKind::GateRuntimeIndexOverflow {
                index: runtime_index,
            })?;
        mapped.push(DoorIndex::new(runtime_index).expect("valid door index"));
    }

    if door_indices.next().is_some() || jump_indices.next().is_some() {
        return Err(AdoptErrorKind::GateCountMismatch {
            retained_doors,
            retained_jumps,
            runtime_doors,
            runtime_jumps,
        }
        .into());
    }

    Ok(mapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(gate_type: GateType) -> Door {
        Door {
            gate_type,
            ..Default::default()
        }
    }

    #[test]
    fn maps_mixed_original_order_to_grouped_runtime_order_by_kind_ordinal() {
        let retained = [
            LegacyGridGateAsset::Door,
            LegacyGridGateAsset::Stateless,
            LegacyGridGateAsset::Door,
            LegacyGridGateAsset::Stateless,
            LegacyGridGateAsset::Door,
        ];
        let runtime = [
            gate(GateType::Door),
            gate(GateType::Door),
            gate(GateType::Door),
            gate(GateType::Jump),
            gate(GateType::Jump),
        ];

        assert_eq!(
            derive_legacy_gate_order(&retained, &runtime).unwrap(),
            [
                DoorIndex::new(0).expect("valid door index"),
                DoorIndex::new(3).expect("valid door index"),
                DoorIndex::new(1).expect("valid door index"),
                DoorIndex::new(4).expect("valid door index"),
                DoorIndex::new(2).expect("valid door index"),
            ]
        );
    }

    #[test]
    fn rejects_missing_or_extra_kind_peers() {
        let retained = [LegacyGridGateAsset::Door, LegacyGridGateAsset::Stateless];
        assert!(matches!(
            derive_legacy_gate_order(&retained, &[gate(GateType::Door)]),
            Err(LegacyAdoptError {
                kind: AdoptErrorKind::GateMissingPeer {
                    kind: "jump gate",
                    ..
                },
                ..
            })
        ));
        assert!(matches!(
            derive_legacy_gate_order(
                &retained,
                &[
                    gate(GateType::Door),
                    gate(GateType::Door),
                    gate(GateType::Jump)
                ]
            ),
            Err(LegacyAdoptError {
                kind: AdoptErrorKind::GateCountMismatch { .. },
                ..
            })
        ));
    }
}

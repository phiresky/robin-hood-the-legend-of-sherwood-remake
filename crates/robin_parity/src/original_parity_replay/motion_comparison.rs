//! Motion-grid and movement-key comparison policy; no state reconstruction.
use super::{
    BTreeMap, BTreeSet, Engine, LineIndex, TraceJumpLine, TraceMotionGrid, TraceMotionLine,
    TraceMotionLineChange, TracePassDoor,
};

pub(super) fn trace_pass_door_key(pass: &TracePassDoor) -> (u32, bool) {
    assert_eq!(
        pass.direct,
        pass.direction != 0,
        "current-schema active PassDoor direct flag disagrees with its direction"
    );
    (pass.gate_id, pass.direct)
}

pub(super) fn active_pass_door_keys_match(
    expected: Option<&TracePassDoor>,
    actual: Option<(u32, bool)>,
) -> bool {
    expected.map(trace_pass_door_key) == actual
}

pub(super) fn trace_jump_line_bits(line: &TraceJumpLine) -> [u32; 4] {
    [line.a.x.bits, line.a.y.bits, line.b.x.bits, line.b.y.bits]
}

pub(super) fn runtime_jump_line_bits(line: &robin_engine::jump_line::JumpLine) -> [u32; 4] {
    [
        line.point_a.x.to_bits(),
        line.point_a.y.to_bits(),
        line.point_b.x.to_bits(),
        line.point_b.y.to_bits(),
    ]
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct MotionLineSignature {
    pub(super) layer: u16,
    pub(super) ax: u32,
    pub(super) ay: u32,
    pub(super) bx: u32,
    pub(super) by: u32,
    pub(super) repulsive: bool,
}

impl MotionLineSignature {
    pub(super) fn original(layer: u16, line: &TraceMotionLine) -> Self {
        Self {
            layer,
            ax: line.a.x.bits,
            ay: line.a.y.bits,
            bx: line.b.x.bits,
            by: line.b.y.bits,
            repulsive: line.type_mask & 128 != 0,
        }
    }

    pub(super) fn rust(layer: u16, line: &robin_engine::fast_find_grid::GridLine) -> Self {
        Self {
            layer,
            ax: line.a.x.to_bits(),
            ay: line.a.y.to_bits(),
            bx: line.b.x.to_bits(),
            by: line.b.y.to_bits(),
            repulsive: line.is_repulsive,
        }
    }
}

/// Isomorphic mapping from the Original's layer-local line indices to Rust's
/// flat `LineIndex` arena. Geometry and the behavior-relevant repulsive flag
/// form the identity; identical duplicate lines are paired by occurrence.
pub(super) struct MotionLineParity {
    pub(super) original_to_rust: BTreeMap<(u16, u16), LineIndex>,
    pub(super) expected_active: BTreeMap<(u16, u16), bool>,
    pub(super) initial_differences: Vec<String>,
}

impl MotionLineParity {
    pub(super) fn build(engine: &Engine, original: &TraceMotionGrid) -> Self {
        let mut original_groups =
            BTreeMap::<MotionLineSignature, Vec<(u16, &TraceMotionLine)>>::new();
        let mut expected_active = BTreeMap::new();
        let mut initial_differences = Vec::new();
        for layer in &original.layers {
            for line in &layer.lines {
                let address = (layer.layer, line.index);
                if expected_active.insert(address, line.active).is_some() {
                    initial_differences.push(format!(
                        "motion_grid.static_mapping: duplicate Original line address layer={} index={}",
                        layer.layer, line.index
                    ));
                }
                if line.type_mask & 2 == 0 {
                    initial_differences.push(format!(
                        "motion_grid.static_mapping: Original layer={} index={} is not LINE_MOTION (type_mask={} sector={})",
                        layer.layer, line.index, line.type_mask, line.associated_sector
                    ));
                }
                original_groups
                    .entry(MotionLineSignature::original(layer.layer, line))
                    .or_default()
                    .push((layer.layer, line));
            }
        }

        let grid = engine.fast_grid();
        let mut rust_groups = BTreeMap::<MotionLineSignature, Vec<LineIndex>>::new();
        for (layer_index, layer) in grid.level.layers.iter().enumerate() {
            let layer_number =
                u16::try_from(layer_index).expect("Rust motion-grid layer index exceeds u16");
            for &line_index in &layer.line_indices {
                let line = &grid.level.lines[usize::from(line_index)];
                if line.is_motion {
                    rust_groups
                        .entry(MotionLineSignature::rust(layer_number, line))
                        .or_default()
                        .push(line_index);
                }
            }
        }

        let signatures = original_groups
            .keys()
            .chain(rust_groups.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        let mut original_to_rust = BTreeMap::new();
        for signature in signatures {
            let originals = original_groups
                .get(&signature)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let rust = rust_groups
                .get(&signature)
                .map(Vec::as_slice)
                .unwrap_or_default();
            if originals.len() != rust.len() {
                initial_differences.push(format!(
                    "motion_grid.static_mapping: signature={signature:?} original_count={} rust_count={}",
                    originals.len(),
                    rust.len()
                ));
            }
            for ((layer, original_line), &rust_line) in originals.iter().zip(rust) {
                let address = (*layer, original_line.index);
                original_to_rust.insert(address, rust_line);
                let rust_active = grid.is_line_active(rust_line);
                if original_line.active != rust_active {
                    initial_differences.push(format!(
                        "motion_grid.initial_active[layer={} index={} rust_line={}]: original={} rust={} type_mask={} sector={}",
                        layer,
                        original_line.index,
                        rust_line,
                        original_line.active,
                        rust_active,
                        original_line.type_mask,
                        original_line.associated_sector
                    ));
                }
            }
        }

        Self {
            original_to_rust,
            expected_active,
            initial_differences,
        }
    }

    pub(super) fn apply_changes_and_compare(
        &mut self,
        engine: &Engine,
        changes: &[TraceMotionLineChange],
    ) -> Vec<String> {
        let mut differences = std::mem::take(&mut self.initial_differences);
        for change in changes {
            let address = (change.layer, change.index);
            let Some(expected) = self.expected_active.get_mut(&address) else {
                differences.push(format!(
                    "motion_grid.line_active[layer={} index={}]: Original change references an unknown static line",
                    change.layer, change.index
                ));
                continue;
            };
            *expected = change.active;
        }

        let grid = engine.fast_grid();
        for (&(layer, original_index), &expected) in &self.expected_active {
            let Some(&rust_index) = self.original_to_rust.get(&(layer, original_index)) else {
                continue;
            };
            let actual = grid.is_line_active(rust_index);
            if expected != actual {
                differences.push(format!(
                    "motion_grid.line_active[layer={layer} index={original_index} rust_line={rust_index}]: original={expected:?} rust={actual:?}"
                ));
            }
        }
        differences
    }
}

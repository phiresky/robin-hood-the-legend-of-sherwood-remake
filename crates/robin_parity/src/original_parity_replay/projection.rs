//! Extracted projection boundary; wire layouts remain in the parent.

pub(super) fn parity_float_is_positive_zero(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("bits"))
        .and_then(serde_json::Value::as_u64)
        == Some(0)
}

pub(super) fn original_blocked_box_is_unset(value: &serde_json::Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() != 2 {
        return false;
    }
    ["min", "max"].into_iter().all(|corner| {
        let Some(point) = object.get(corner).and_then(serde_json::Value::as_object) else {
            return false;
        };
        point.len() == 2
            && ["x", "y"]
                .into_iter()
                .all(|axis| point.get(axis).is_some_and(parity_float_is_positive_zero))
    })
}

/// Translate Original-only wire representations into their Rust parity
/// projection equivalents.
///
/// Original order IDs start at zero, whereas this implementation
/// deliberately uses `NonZeroU32` order IDs. This is the same +1 translation
/// used while adopting legacy saves, but runtime snapshots must retain their
/// authoritative raw payload and apply it only while comparing.
///
/// Original-game bounding-box reset clears only
/// the bounds-set marker. The parity
/// emitter omits that bit and therefore publishes a newly constructed unset
/// blocked box as an all-positive-zero object. Rust represents the same unset
/// state as `null`.
pub(super) fn canonicalize_original_runtime_representation(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                canonicalize_original_runtime_representation(value);
            }
        }
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if key == "last_processed_order_id"
                    && let Some(original) = child.as_u64()
                    && original < u64::from(u32::MAX)
                {
                    *child = serde_json::Value::from(original + 1);
                } else if key == "blocked_box" && original_blocked_box_is_unset(child) {
                    *child = serde_json::Value::Null;
                } else {
                    canonicalize_original_runtime_representation(child);
                }
            }
        }
        _ => {}
    }
}

/// Remove only sprite state whose producer depends on the unrecorded Original
/// draw viewport.
///
/// Schema 16 records the simulation camera but omits the engine view point.
/// Target-sprite creation updates these three serialized fields only
/// when the current sprite intersects that draw-time view, so their values
/// cannot be reconstructed at viewport edges. All animation, position, and
/// other sprite/gameplay state remains authoritative.
pub(super) fn project_missing_draw_view_sprite_cache(value: &mut serde_json::Value) {
    let Some(sprite) = value
        .as_object_mut()
        .and_then(|runtime| runtime.get_mut("sprite"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    for field in ["width", "height", "masked"] {
        sprite.remove(field);
    }
}

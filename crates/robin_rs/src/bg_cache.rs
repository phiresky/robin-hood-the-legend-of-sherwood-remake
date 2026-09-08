//! Persistent background decals.
//!
//! Pure host state for the patch-effect rendering pipeline.  Lives on
//! `Host` (not the engine) because the renderer owns GPU resources and
//! the engine only emits [`robin_engine::engine::PendingBgBlit`] requests.

use indexmap::IndexMap;
use robin_engine::element::EntityId;
use serde::{Deserialize, Serialize};

/// GPU-rendered persistent background decal baked into the map.
/// Stored in map coordinates and drawn immediately after the base map,
/// before the normal entity/overlay phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundDecal {
    pub bank_id: u32,
    pub dst_x: i32,
    pub dst_y: i32,
    pub width: u32,
    pub height: u32,
    pub shadow_color: u16,
    pub shadow_level: u16,
}

/// Mission-local decals in the order their patch effects became permanent.
///
/// Keep the ordered map private so callers cannot accidentally swap-remove a
/// decal and change how surviving patches overlap. These descriptors refer to
/// the current frame holder, not durable sprite identities; restore them from
/// engine effects after loading a level, never from diagnostic serialization.
#[derive(Debug, Default)]
pub(crate) struct BackgroundDecals {
    entries: IndexMap<EntityId, BackgroundDecal>,
}

impl BackgroundDecals {
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn insert(&mut self, id: EntityId, decal: BackgroundDecal) {
        // IndexMap replacement preserves the original insertion position.
        self.entries.insert(id, decal);
    }

    pub(crate) fn remove(&mut self, id: EntityId) -> Option<BackgroundDecal> {
        // A restore for a patch already absent is an expected no-op.
        self.entries.shift_remove(&id)
    }

    pub(crate) fn in_draw_order(&self) -> impl Iterator<Item = &BackgroundDecal> {
        self.entries.values()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Serialize for BackgroundDecals {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // A sequence retains draw order and supports EntityId keys in JSON.
        serializer.collect_seq(self.entries.iter())
    }
}

impl<'de> Deserialize<'de> for BackgroundDecals {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "background decals must be reconstructed from current-level engine effects",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::element::FxId;

    fn id(value: u32) -> EntityId {
        EntityId::Fx(FxId(value))
    }

    fn decal(bank_id: u32) -> BackgroundDecal {
        BackgroundDecal {
            bank_id,
            dst_x: 0,
            dst_y: 0,
            width: 4,
            height: 4,
            shadow_color: 0,
            shadow_level: 0,
        }
    }

    fn banks(decals: &BackgroundDecals) -> Vec<u32> {
        decals.in_draw_order().map(|decal| decal.bank_id).collect()
    }

    #[test]
    fn replacement_keeps_position_and_reinsertion_appends() {
        let mut decals = BackgroundDecals::default();
        for value in 1..=4 {
            decals.insert(id(value), decal(value));
        }
        decals.insert(id(2), decal(20));
        assert_eq!(banks(&decals), [1, 20, 3, 4]);

        assert_eq!(decals.remove(id(2)).unwrap().bank_id, 20);
        assert_eq!(banks(&decals), [1, 3, 4]);
        assert!(decals.remove(id(2)).is_none());
        assert_eq!(banks(&decals), [1, 3, 4]);

        decals.insert(id(2), decal(22));
        assert_eq!(banks(&decals), [1, 3, 4, 22]);
    }

    #[test]
    fn level_reset_reconstructs_order_without_stale_entries() {
        let mut frontend = crate::host::HostFrontend::default();
        frontend.background_decals.insert(id(1), decal(1));
        frontend.background_decals.insert(id(2), decal(2));

        frontend.clear_background_decals();
        assert!(frontend.background_decals.is_empty());
        assert_eq!(banks(&frontend.background_decals), []);

        frontend.background_decals.insert(id(2), decal(12));
        frontend.background_decals.insert(id(1), decal(11));
        assert_eq!(banks(&frontend.background_decals), [12, 11]);
    }

    #[test]
    fn diagnostics_preserve_order_but_cannot_restore_frame_holder_references() {
        let mut decals = BackgroundDecals::default();
        decals.insert(id(2), decal(20));
        decals.insert(id(1), decal(10));
        let diagnostics = serde_json::to_value(&decals).unwrap();
        assert_eq!(diagnostics[0][1]["bank_id"], 20);
        assert_eq!(diagnostics[1][1]["bank_id"], 10);
        let error = serde_json::from_value::<BackgroundDecals>(diagnostics).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("reconstructed from current-level")
        );
    }
}

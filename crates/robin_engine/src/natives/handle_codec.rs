use crate::element::EntityId;

/// Stateless codec for the opaque handles exchanged with mission scripts.
///
/// Original C++ scripts used typed `void*` values: `NULL` was zero and every
/// valid object handle was non-null. The Rust VM stores script values as
/// `i32`, so the upper nibble identifies the object table and the lower 28
/// bits retain the original zero-based table index. Keeping this outside
/// [`super::GameHost`] makes the representation independent of queued host
/// effects and preserves sparse legacy table slots.
#[derive(Debug, Clone, Copy)]
pub struct ScriptHandleCodec;

const INDEX_MASK: i32 = 0x0fff_ffff;

#[derive(Debug, Clone, Copy)]
pub(super) enum ScriptHandleKind {
    Actor = 0x1000_0000,
    Door = 0x2000_0000,
    Patch = 0x3000_0000,
    Location = 0x4000_0000,
    SoundSource = 0x5000_0000,
    Building = 0x6000_0000,
    Way = 0x7000_0000,
}

impl ScriptHandleCodec {
    pub(super) fn encode(kind: ScriptHandleKind, index: usize) -> i32 {
        let index = i32::try_from(index).expect("script handle index exceeds i32 range");
        assert!(
            (0..=INDEX_MASK).contains(&index),
            "script handle index exceeds 28-bit payload: {index}"
        );
        kind as i32 | index
    }

    pub(super) fn decode(handle: i32, kind: ScriptHandleKind) -> Option<usize> {
        Self::has_kind(handle, kind).then_some((handle & INDEX_MASK) as usize)
    }

    pub(super) fn has_kind(handle: i32, kind: ScriptHandleKind) -> bool {
        handle > 0 && (handle & !INDEX_MASK) == kind as i32
    }

    pub fn actor_handle<I: Into<EntityId>>(id: I) -> i32 {
        Self::actor_handle_from_index(id.into().index() as usize)
    }

    pub fn actor_handle_from_index(index: usize) -> i32 {
        Self::encode(ScriptHandleKind::Actor, index)
    }

    pub fn door_handle_from_index(index: usize) -> i32 {
        Self::encode(ScriptHandleKind::Door, index)
    }

    pub fn patch_handle_from_index(index: usize) -> i32 {
        Self::encode(ScriptHandleKind::Patch, index)
    }

    pub fn location_handle_from_index(index: usize) -> i32 {
        Self::encode(ScriptHandleKind::Location, index)
    }

    pub fn sound_source_handle_from_index(index: usize) -> i32 {
        Self::encode(ScriptHandleKind::SoundSource, index)
    }

    pub fn building_handle_from_index(index: usize) -> i32 {
        Self::encode(ScriptHandleKind::Building, index)
    }

    pub fn actor_handle_index(handle: i32) -> Option<usize> {
        Self::decode(handle, ScriptHandleKind::Actor)
    }

    pub fn door_index(handle: i32) -> Option<usize> {
        Self::decode(handle, ScriptHandleKind::Door)
    }

    pub fn patch_index(handle: i32) -> Option<usize> {
        Self::decode(handle, ScriptHandleKind::Patch)
    }

    pub fn location_index(handle: i32) -> Option<usize> {
        Self::decode(handle, ScriptHandleKind::Location)
    }

    pub fn sound_source_index(handle: i32) -> Option<usize> {
        Self::decode(handle, ScriptHandleKind::SoundSource)
    }

    pub fn building_index(handle: i32) -> Option<usize> {
        Self::decode(handle, ScriptHandleKind::Building)
    }

    pub fn way_index(handle: i32) -> Option<usize> {
        Self::decode(handle, ScriptHandleKind::Way)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_preserve_type_and_sparse_zero_based_index() {
        let actor = ScriptHandleCodec::actor_handle_from_index(70);

        assert_ne!(actor, 0);
        assert_eq!(ScriptHandleCodec::actor_handle_index(actor), Some(70));
        assert_eq!(ScriptHandleCodec::door_index(actor), None);
        assert_eq!(ScriptHandleCodec::actor_handle_index(0), None);
    }
}

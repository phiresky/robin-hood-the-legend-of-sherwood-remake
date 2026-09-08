//! Persistent background decals.
//!
//! Pure host state for the patch-effect rendering pipeline.  Lives on
//! `Host` (not the engine) because the renderer owns GPU resources and
//! the engine only emits [`robin_engine::engine::PendingBgBlit`] requests.

/// GPU-rendered persistent background decal baked into the map.
/// Stored in map coordinates and drawn immediately after the base map,
/// before the normal entity/overlay phase.
#[derive(Debug, Clone)]
pub struct BackgroundDecal {
    pub bank_id: u32,
    pub dst_x: i32,
    pub dst_y: i32,
    pub width: u32,
    pub height: u32,
    pub shadow_color: u16,
    pub shadow_level: u16,
}

//! Deterministic simulation engine for Robin Hood: The Legend of Sherwood.

// Catch the "method-lvalue footgun": `X.position().x = v` evaluates the
// `position()` call (which returns a `Point3D` by value), assigns to the
// temporary's field, and drops it — silently a no-op.  Annotating the
// position/direction-style accessors with `#[must_use]` and denying the
// resulting warning makes the pattern fail at compile time.  See the
// "Tech-debt from PI-into-Sprite refactor" notes in `NEW_FEATURES.md`.
#![deny(unused_must_use)]

pub mod abilities;
pub mod ai;
pub mod ai_detectable_filter;
pub mod ai_enemy;
pub mod ai_entity_view;
pub mod ai_friendly;
pub mod ai_vision;
pub mod alert_colors;
pub mod bow_shot;
pub mod campaign;
pub mod change;
pub mod character_kind;
pub mod combat;
pub mod console;
pub mod coordinates;
pub mod element;
pub mod element_kinds;
pub mod element_priority;
pub mod engine_manager;
pub mod entities;
pub mod entity_id;
pub mod event;
pub mod fast_find_grid;
pub mod game_operation;
pub mod gate;
pub mod geo2d;
pub mod graphic_config;
pub mod interp;
pub mod inventory;
pub mod jump_line;
pub mod legacy_io;
pub mod legacy_save;
pub mod level_data;
pub mod macro_store;
pub mod mask;
pub mod md5;
pub mod messenger;
pub mod mission;
pub mod mission_stat;
pub mod movement;
pub mod multiplayer;
pub mod natives;
pub mod order;
pub mod parameters_ai;
pub mod patch;
pub mod path;
pub mod pathfinder;
pub mod pc_status;
pub mod player_command;
pub mod player_profile;
pub mod position_interface;
pub mod profiles;
pub mod replay;
pub mod repulsive;
pub mod resource_ids;
pub mod rhline;
pub mod sbfile;
pub mod scb;
pub mod script_manager;
pub mod sector;
pub mod sector_production;
pub mod sequence;
pub mod shadow_polygon;
pub mod sherwood_stat;
pub mod sight_obstacle;
pub mod sim_rng;
pub mod sound_cache;
pub mod sound_config;
pub mod sound_kinds;
pub mod sprite;
pub mod sprite_script;
pub mod sprite_variant;
mod state_hash_impls;
pub mod stealth;
pub mod titbit;
pub mod vm;
pub mod weapons;
pub mod widget_state;
/// Stub alias so engine code can refer to sim-side sound classification
/// enums via `crate::sound::*`. The actual host sound dispatch lives in
/// `robin_rs::sound`.
pub mod sound {
    pub use crate::sound_kinds::*;
}
pub mod engine;
pub mod markers;
pub mod material_sectors;
pub mod minimap;
pub mod mobile;
pub mod short_briefings;
pub mod sound_geometry;
pub mod sound_source;
pub mod water_zones;

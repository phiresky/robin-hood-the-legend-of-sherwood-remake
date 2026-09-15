//! Deterministic simulation engine for Robin Hood: The Legend of Sherwood.

// Catch the "method-lvalue footgun": `X.position().x = v` evaluates the
// `position()` call (which returns a `Point3D` by value), assigns to the
// temporary's field, and drops it — silently a no-op.  Annotating the
// position/direction-style accessors with `#[must_use]` and denying the
// resulting warning makes the pattern fail at compile time.  See the
// "Tech-debt from PI-into-Sprite refactor" notes in `NEW_FEATURES.md`.
#![deny(unused_must_use)]

pub mod abilities;
pub mod achievement;
pub mod actor_state;
pub mod ai;
pub mod ai_detectable_filter;
pub mod ai_enemy;
pub mod ai_friendly;
pub mod ai_vision;
pub mod alert_colors;
#[doc(hidden)]
pub mod bitcode_adapters;
pub mod bow_shot;
pub mod campaign;
pub mod campaign_history;
pub mod change;
pub use robin_engine_types::character_kind;
pub mod cloak;
pub mod combat;
pub mod console;
pub mod content_patch;
pub use robin_engine_types::coordinates;
pub mod diplomacy;
pub mod element;
pub mod element_kinds;
pub mod element_priority;
pub mod engine_manager;
pub mod entities;
pub use robin_engine_types::entity_id;
pub mod event;
pub mod fast_find_grid;
pub mod fog_of_war;
pub mod game_operation;
pub use robin_engine_types::gameplay_config;
pub mod gate;
pub use robin_engine_types::geo2d;
pub use robin_engine_types::graphic_config;
pub mod human_control;
pub mod interp;
pub mod inventory;
/// Host/diagnostics JSON projection (engine dumps, save identities, parity
/// reports). Lives here because robin_engine is the lowest crate both
/// robin_rs and the CPU-only robin_parity build depend on.
pub mod json_value;
pub mod jump_line;
pub mod le_bytes;
pub use robin_data_io::legacy_io;
pub mod engine;
pub mod legacy_save;
pub mod level_data;
pub mod macro_store;
pub mod markers;
pub mod mask;
pub mod material_sectors;
pub mod messenger;
pub mod minimap;
pub mod mission;
pub mod mission_assets;
pub mod mission_stat;
pub mod mobile;
pub mod movement;
pub mod movement_diagnostics;
pub mod multiplayer;
pub use robin_engine_types::multiplayer_config;
pub mod natives;
pub mod order;
pub use robin_engine_types::parameters_ai;
pub mod patch;
pub mod path;
pub mod pathfinder;
pub mod pc_status;
pub mod player_command;
pub mod player_profile;
pub mod position_interface;
pub use robin_level_data::profiles;
pub mod ranked_resim;
pub mod replay;
pub mod replay_rankability;
pub mod repulsive;
pub use robin_engine_types::resource_ids;
pub mod ranked_rules;
pub mod rhline;
pub mod sbfile;
pub mod scb;
pub mod script_manager;
pub mod sector;
pub mod sector_production;
pub mod sequence;
pub mod shadow_polygon;
pub mod sherwood_stat;
pub mod short_briefings;
pub mod sight_obstacle;
pub mod sim_rng;
pub mod sim_timeline;
pub mod sound;
pub mod sound_cache;
pub use robin_engine_types::sound_config;
pub mod sound_geometry;
pub mod sound_source;
pub mod spellforge;
pub mod sprite;
pub mod sprite_script;
pub mod sprite_variant;
pub use robin_engine_types::static_arc;
pub mod stealth;
pub mod tactical_control;
pub mod titbit;
pub mod trading;
pub mod vm;
pub mod water_zones;
pub mod weapons;
pub mod widget_state;

pub mod audio_durations;

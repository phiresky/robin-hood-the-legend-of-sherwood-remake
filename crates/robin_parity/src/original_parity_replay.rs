//! Replay a plain or zstd-compressed JSONL parity trace produced by the
//! original game.
//!
//! This intentionally accepts the original game's neutral, resolved-command
//! schema rather than the Rust-native replay schema.
//!
//! Usage:
//!   ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
//!     cargo run -p robin_parity --bin original_parity_replay -- \
//!       parity-traces/original-demo-baseline.jsonl
//!
//! Submodules import shared names from this module by explicit name; each
//! `use` list below is the surface that module offers its siblings. Names only
//! the unit tests need are imported directly by `tests.rs`.

mod native_storage;
mod run_error;
mod runner;
use native_storage::{
    StorageContext, TraceStorageResult, bench_trace_encodings, conversion_quarantine_path,
    difference_field, ensure_native_binary_trace, ensure_native_binary_trace_locked,
    finish_verified_conversion, lock_native_trace_generation,
    move_verified_recording_to_quarantine, read_all_rng_draws, read_binary_trace_footer,
    read_binary_trace_header, reblock_native_trace, reject_conversion_symlink,
    requested_native_trace_path, should_preload_complete_rng_stream, simulation_rng_draws,
    storage_ensure, validate_binary_trace_footer, validate_native_trace,
};
use run_error::{TraceRunError, TraceRunResult};

mod trace_codec;
use trace_codec::{
    VerifiedNativeReadback, absolute_trace_path, canonicalize_trace_identity,
    convert_recording_to_native, join_roundtrip_audit_workers, native_binary_trace_path,
    open_jsonl_trace, spawn_roundtrip_audit_workers, trace_content_sha256,
    trace_source_fingerprint, validate_standalone_native_trace, verify_trace_line_roundtrip,
};
mod comparison;
use comparison::{compare_frame, compare_path_events, compare_visibility_queries};
mod projection;
use projection::{
    canonicalize_original_runtime_representation, project_missing_draw_view_sprite_cache,
};
pub use runner::main;

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{collections::BTreeMap, collections::BTreeSet, collections::VecDeque};

// Version 68 native parity traces are authoritative artifacts. Keep their
// codec pinned independently of the game's intentionally evolving formats.
use bitcode_parity as bitcode;
use robin_engine::coordinates::MapPoint;
use robin_engine::coordinates::WorldPoint3D;
use robin_engine::element::{Command, Entity, EntityId, EntityIdKind};
#[cfg(feature = "client")]
use robin_engine::engine::HostDisplayState;
use robin_engine::engine::{Engine, LegacyGridSectorAsset, LevelAssets};
use robin_engine::fast_find_grid::LineIndex;
use robin_engine::game_operation::GameCode;
#[cfg(feature = "client")]
use robin_engine::graphic_config::TextureScaleMode;
use robin_engine::player_command::{GestureQuality, PlayerCommand};
use robin_engine::sector::SectorNumber;
#[cfg(feature = "client")]
use robin_engine::sprite::BBox;
#[cfg(feature = "client")]
use robin_rs::Host;
#[cfg(feature = "client")]
use robin_rs::gfx_types::BlendMode;
#[cfg(feature = "client")]
use robin_rs::http_server::RpcError;
#[cfg(feature = "client")]
use robin_rs::level_loading_host::draw_background;
#[cfg(feature = "client")]
use robin_rs::renderer::{GpuImage, Renderer, rgb565_to_rgb8};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::sha256_hex;
mod trace_model;
use trace_model::{
    LAST_TRACE_SCHEMA_WITHOUT_DRAW_VIEW, OLDEST_SUPPORTED_TRACE_SCHEMA, TRACE_SCHEMA_VERSION,
    TraceAction, TraceActor, TraceCampaign, TraceCommand, TraceElement, TraceEntityId,
    TraceEntityKind, TraceFlightStep, TraceFloat, TraceFrame, TraceHeader, TraceHuman,
    TraceInitialNpcTransient, TraceJsonTree, TraceJsonValue, TraceJumpLine, TraceMotionGrid,
    TraceMotionLine, TraceMotionLineChange, TraceMovementStep, TracePassDoor, TracePathEvent,
    TracePoint, TraceRecordMarker, TraceRngBatch, TraceRngDomain, TraceRngOnly, TraceRngPrefix,
    TraceRouteConstructionEvent, TraceSequenceLifecycleEvent, TraceStartState,
    TraceVisibilityQuery,
};
mod trace_admission;
use trace_admission::{
    decode_and_validate_initial_save, parse_trace_frame, trace_schema_is_supported,
    validate_trace_frame_with_legacy_additive_omissions, validate_trace_header,
    validate_trace_start,
};
mod reconstruction;
#[cfg(not(feature = "client"))]
use reconstruction::initialize_headless_engine;
use reconstruction::{
    TraceTimeline, apply_initial_npc_transients, apply_legacy_interactive_chain_macro_fallback,
    apply_legacy_segment_visibility_fallback, command_from_stable_name,
    cross_post_initialize_frame, legacy_loaded_save_retains_process_transients,
    record_arrow_publication_before_compare, register_language_data_paths_for_tool,
};
#[cfg(feature = "client")]
use reconstruction::{initialize_engine, restore_campaign};
mod recorder_omissions;
use recorder_omissions::{
    LegacyBlockedBoxShadow, LegacyRefreshOrientationProvenance, advance_trace_qa_recording_state,
    append_legacy_retained_terminal_success_repair, canonicalize_legacy_blocked_box,
    has_legacy_teleport_star_lifecycle, initial_legacy_blocked_box_shadows,
    legacy_additional_arrow_refresh_draws, legacy_presentation_entity_states,
    legacy_presentation_sprite_rng_burst, missing_legacy_presentation_sprite_rng_draws,
    original_motion_executor_order_id, original_reset_blocked_box_this_frame,
    original_stoppable_current_motion_order, replay_campaign_run_id,
    split_refresh_owned_orientations,
};
mod motion_comparison;
use motion_comparison::{
    MotionLineParity, active_pass_door_keys_match, runtime_jump_line_bits, trace_jump_line_bits,
    trace_pass_door_key,
};
mod route_reconstruction;
use route_reconstruction::{
    ReplayDropAleResolution, ReplayGroupMoveResolution, collect_current_delayed_drop_ale_routes,
    current_drop_ale_same_sector_goal, resolve_current_drop_ale, resolve_current_group_move_route,
    restore_legacy_route_construction_diagnostics,
};
mod reporting;
use reporting::{
    DumpOptions, EntityLabel, RollingDumpFrame, print_current_trace_actor_diagnostics,
    print_current_trace_events, print_debug_element, print_startup_actors, push_rolling_window,
    structured_divergences, write_automatic_rolling_dump, write_engine_dump_frame,
    write_jsonl_record,
};
mod native_model;
#[cfg(test)]
use native_model::TRACE_NATIVE_MAX_REBLOCK_RECORDS;
use native_model::{
    BinaryTraceFooter, BinaryTraceHeaderV68, BinaryTraceReader, BinaryTraceRecord,
    NativeReblockBinding, NativeStoragePolicy, TRACE_CONVERSION_QUARANTINE_SUFFIX,
    TRACE_NATIVE_BLOCK_RECORDS, TRACE_NATIVE_FOOTER_LEN, TRACE_NATIVE_FOOTER_MAGIC,
    TRACE_NATIVE_LONG_DISTANCE_MATCHING, TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG,
    TRACE_NATIVE_MIN_WINDOW_LOG, TRACE_NATIVE_SUFFIX, TRACE_NATIVE_VERSION,
    TRACE_NATIVE_WINDOW_LOG, TRACE_NATIVE_ZSTD_LEVEL, TRACE_REBLOCK_BINDING_SUFFIX,
    TRACE_REBLOCK_SOURCE_SUFFIX, TRACE_ZSTD_WINDOW_LOG_MAX,
};
mod cli;
#[cfg(test)]
use cli::CliOptions;
use cli::{Options, parse_options};
#[cfg(feature = "client")]
mod client;
#[cfg(feature = "client")]
use client::{VisualReplay, drain_headless_http, frame_zero_screenshot_path, serve_halted_http};
mod entity_map;
use entity_map::{EntityMap, GroupMoveGoalTranslation};

#[cfg(test)]
mod tests;

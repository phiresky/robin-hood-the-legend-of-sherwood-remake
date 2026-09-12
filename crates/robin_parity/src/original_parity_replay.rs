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

mod native_storage;
mod runner;
use native_storage::*;

mod trace_codec;
use trace_codec::*;
mod comparison;
use comparison::*;
mod projection;
use projection::{
    canonicalize_original_runtime_representation, project_missing_draw_view_sprite_cache,
};
pub use runner::main;

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt as _;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{collections::BTreeMap, collections::BTreeSet, collections::VecDeque};

// Version 68 native parity traces are authoritative artifacts. Keep their
// codec pinned independently of the game's intentionally evolving formats.
use bitcode_parity as bitcode;
use fs2::FileExt as _;
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
use robin_engine::profiles::Action;
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
use trace_model::*;
mod trace_admission;
use trace_admission::*;
mod reconstruction;
use reconstruction::*;
mod recorder_omissions;
use recorder_omissions::*;
mod motion_comparison;
use motion_comparison::*;
mod route_reconstruction;
use route_reconstruction::*;
mod reporting;
use reporting::*;
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
use client::*;
mod entity_map;
use entity_map::*;

#[cfg(test)]
mod tests;

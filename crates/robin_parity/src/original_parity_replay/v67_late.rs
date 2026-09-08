//! Frozen v67-late wire layouts and one-way compatibility conversion.
//!
//! Field and variant order, field types, and shared child layouts are part of
//! authoritative bitcode artifacts. Do not modernize these shapes. Conversion
//! may reconstruct only the omissions documented by this generation. Reverse
//! conversions are test-only: production always writes the current layout.
use super::*;

/// Accidental late-v67 layout written between `1a932c148` and the v68 bump.
/// Unlike the original v67 layout, absence and an explicitly empty transient
/// overlay remain distinguishable here.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceHeaderV67Late {
    pub(super) record_type: String,
    pub(super) mission: String,
    pub(super) proto_level: String,
    pub(super) rng_seed: u64,
    pub(super) schema: u32,
    pub(super) session_index: u32,
    pub(super) start_state: TraceStartState,
    pub(super) initial_frame: u64,
    pub(super) simulation_hz: u32,
    pub(super) synchronous_pathfinding: bool,
    pub(super) rng_stream: String,
    pub(super) visibility_queries: String,
    pub(super) random_input_seed: Option<u32>,
    pub(super) sim_config: TraceSimConfig,
    pub(super) campaign: TraceCampaign,
    pub(super) motion_grid: TraceMotionGrid,
    pub(super) initial_npc_transients: Option<Vec<TraceInitialNpcTransient>>,
    pub(super) initial_save: Option<TraceInitialSave>,
}

impl From<TraceHeaderV67Late> for TraceHeader {
    fn from(header: TraceHeaderV67Late) -> Self {
        Self {
            record_type: header.record_type,
            mission: header.mission,
            proto_level: header.proto_level,
            rng_seed: header.rng_seed,
            schema: header.schema,
            session_index: header.session_index,
            start_state: header.start_state,
            initial_frame: header.initial_frame,
            simulation_hz: header.simulation_hz,
            synchronous_pathfinding: header.synchronous_pathfinding,
            rng_stream: header.rng_stream,
            visibility_queries: header.visibility_queries,
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            initial_npc_transients: header.initial_npc_transients,
            initial_save: header.initial_save,
        }
    }
}

#[cfg(test)]
impl From<TraceHeader> for TraceHeaderV67Late {
    fn from(header: TraceHeader) -> Self {
        Self {
            record_type: header.record_type,
            mission: header.mission,
            proto_level: header.proto_level,
            rng_seed: header.rng_seed,
            schema: header.schema,
            session_index: header.session_index,
            start_state: header.start_state,
            initial_frame: header.initial_frame,
            simulation_hz: header.simulation_hz,
            synchronous_pathfinding: header.synchronous_pathfinding,
            rng_stream: header.rng_stream,
            visibility_queries: header.visibility_queries,
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            initial_npc_transients: header.initial_npc_transients,
            initial_save: header.initial_save,
        }
    }
}

/// Late version-67 header layout written after `1a932c148` changed
/// `initial_npc_transients` back to an `Option<Vec<_>>` without bumping the
/// native format version. A small retained interactive corpus was converted
/// during that window. Keep this separate from both the original v67 layout
/// and the identically-shaped v68 header so the accidental wire generation is
/// explicit and cannot silently drift again.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
pub(super) struct BinaryTraceHeaderV67Late {
    pub(super) version: u32,
    pub(super) source_fingerprint: String,
    pub(super) trace: TraceHeaderV67Late,
    pub(super) rng_prefix: TraceRngPrefix,
}

impl From<BinaryTraceHeaderV67Late> for BinaryTraceHeaderV68 {
    fn from(header: BinaryTraceHeaderV67Late) -> Self {
        Self {
            version: header.version,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

#[cfg(test)]
impl From<BinaryTraceHeaderV68> for BinaryTraceHeaderV67Late {
    fn from(header: BinaryTraceHeaderV68) -> Self {
        Self {
            version: TRACE_NATIVE_LEGACY_VERSION,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

/// Record envelope written during the same accidental late-v67 window as
/// [`BinaryTraceHeaderV67Late`]. Its frame uses the then-current optional
/// `increment_map_valid` representation rather than [`super::v67::TraceFrameV67`]'s bool.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
pub(super) enum BinaryTraceRecordV67Late {
    Frame(TraceFrame),
    End {
        rng_suffix: Option<TraceRngBatch>,
        final_frame: Option<u64>,
        frame_count: Option<u64>,
    },
}

impl BinaryTraceRecordV67Late {
    pub(super) fn into_current(self) -> BinaryTraceRecord {
        match self {
            Self::Frame(frame) => BinaryTraceRecord::Frame(frame),
            Self::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            },
        }
    }
}

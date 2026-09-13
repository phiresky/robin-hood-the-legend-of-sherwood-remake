//! Trace header record: initial save and NPC state, simulation config and
//! the supported schema range.
use super::campaign::TraceCampaign;
use super::motion::TraceMotionGrid;
use crate::sha256_hex;
use base64::Engine as _;
use bitcode_parity as bitcode;
use serde::{Deserialize, Serialize};

/// JSON trace header and the embedded header layout of
/// [`BinaryTraceHeaderV68`](crate::original_parity_replay::native_model::BinaryTraceHeaderV68).
///
/// ON-DISK FORMAT INVARIANT: changing any field, field order, or field type
/// changes bitcode's native trace layout. Such a change must bump
/// `TRACE_NATIVE_VERSION`, freeze the old layout in a version-named struct,
/// and add an explicit decoder branch for it. Never silently edit this type
/// while retaining native trace version 68.
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceHeader {
    #[serde(rename = "type")]
    pub(crate) record_type: String,
    pub(crate) mission: String,
    pub(crate) proto_level: String,
    pub(crate) rng_seed: u64,
    pub(crate) schema: u32,
    pub(crate) session_index: u32,
    pub(crate) start_state: TraceStartState,
    pub(crate) initial_frame: u64,
    pub(crate) simulation_hz: u32,
    pub(crate) synchronous_pathfinding: bool,
    pub(crate) rng_stream: String,
    pub(crate) visibility_queries: String,
    #[serde(default)]
    pub(crate) random_input_seed: Option<u32>,
    pub(crate) sim_config: TraceSimConfig,
    pub(crate) campaign: TraceCampaign,
    pub(crate) motion_grid: TraceMotionGrid,
    /// Current session-boundary state omitted by the original game's save payload.
    /// Early schema-16 interactive recordings predate this additive overlay;
    /// `None` selects the narrowly-scoped legacy reconstruction below while a
    /// present (including empty) list remains authoritative.
    #[serde(default)]
    pub(crate) initial_npc_transients: Option<Vec<TraceInitialNpcTransient>>,
    #[serde(default)]
    pub(crate) initial_save: Option<TraceInitialSave>,
}

#[derive(
    Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, bitcode::Encode, bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceInitialNpcTransient {
    pub(crate) creation_order: u32,
    pub(crate) maximal_visibility: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceInitialSave {
    pub(crate) format: String,
    pub(crate) source_profile: TraceSaveSourceProfile,
    pub(crate) encoding: String,
    pub(crate) byte_length: u64,
    pub(crate) sha256: String,
    pub(crate) slot: String,
    pub(crate) header_version: u32,
    pub(crate) mission_id: u32,
    pub(crate) stream_version: u32,
    pub(crate) data: String,
}

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TraceSaveSourceProfile {
    LinuxI386RhsgV48,
    WindowsI386GshrV48,
}

impl TraceSaveSourceProfile {
    pub(crate) fn expected_magic(self) -> &'static [u8; 4] {
        match self {
            Self::LinuxI386RhsgV48 => b"RHSG",
            Self::WindowsI386GshrV48 => b"GSHR",
        }
    }
}

impl TraceInitialSave {
    pub(crate) fn decode_and_validate(&self, expected_mission_id: u32) -> Result<Vec<u8>, String> {
        if self.format != "rhsg" {
            return Err(format!(
                "unsupported initial_save format {:?}; expected \"rhsg\"",
                self.format
            ));
        }
        if self.encoding != "base64" {
            return Err(format!(
                "unsupported initial_save encoding {:?}; expected \"base64\"",
                self.encoding
            ));
        }
        if self.slot.is_empty() || self.slot.contains(['/', '\\']) {
            return Err(format!(
                "initial_save slot {:?} must be a non-empty basename",
                self.slot
            ));
        }
        if self.header_version != 48 || self.stream_version != 48 {
            return Err(format!(
                "unsupported initial_save RHSG versions header={} stream={}; expected v48/v48",
                self.header_version, self.stream_version
            ));
        }
        if self.mission_id != expected_mission_id {
            return Err(format!(
                "initial_save mission {} does not match campaign mission {}",
                self.mission_id, expected_mission_id
            ));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("initial_save sha256 must be 64 lowercase hexadecimal digits".to_owned());
        }

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.data)
            .map_err(|error| format!("decode initial_save base64: {error}"))?;
        let declared_length = usize::try_from(self.byte_length)
            .map_err(|_| format!("initial_save byte_length {} is too large", self.byte_length))?;
        if bytes.len() != declared_length {
            return Err(format!(
                "initial_save byte_length says {} but decoded {} bytes",
                self.byte_length,
                bytes.len()
            ));
        }

        let actual_sha256 = sha256_hex(&bytes);
        if actual_sha256 != self.sha256 {
            return Err(format!(
                "initial_save sha256 mismatch: header={} decoded={actual_sha256}",
                self.sha256
            ));
        }
        if bytes.len() < 16 {
            return Err(format!(
                "initial_save is only {} bytes; RHSG header needs 16",
                bytes.len()
            ));
        }
        let expected_magic = self.source_profile.expected_magic();
        if &bytes[0..4] != expected_magic {
            return Err(format!(
                "initial_save source profile {:?} requires magic {:?}, found {:?}",
                self.source_profile,
                std::str::from_utf8(expected_magic).unwrap(),
                std::str::from_utf8(&bytes[0..4]).unwrap_or("<non-ASCII>")
            ));
        }

        let payload_header_version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let payload_mission_id = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let payload_stream_version = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        if payload_header_version != self.header_version
            || payload_mission_id != self.mission_id
            || payload_stream_version != self.stream_version
        {
            return Err(format!(
                "initial_save RHSG header ({payload_header_version}, {payload_mission_id}, \
                 {payload_stream_version}) disagrees with metadata ({}, {}, {})",
                self.header_version, self.mission_id, self.stream_version
            ));
        }

        Ok(bytes)
    }
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceSimConfig {
    pub(crate) difficulty: TraceDifficulty,
    pub(crate) script_enabled: bool,
    pub(crate) highlander: bool,
    pub(crate) highlander2: bool,
    pub(crate) golden_eye: bool,
    pub(crate) ignore_default_loose: bool,
    pub(crate) bypass_fog_sprites_crash: bool,
    pub(crate) amount_of_speaking: u16,
}

impl TraceSimConfig {
    pub(crate) fn to_sim_config(
        &self,
        synchronous_pathfinding: bool,
    ) -> robin_engine::engine::SimConfig {
        robin_engine::engine::SimConfig {
            difficulty: self.difficulty.into(),
            // Original-parity traces deliberately retain the shipped bug.
            fix_hard_reaction_times: false,
            // Untying is a post-port extension and stays off in Original traces.
            enable_unbinding: false,
            clean_hands_npc_kills_invalidate: false,
            // Reusable cloaks are an opt-in extension and must never alter
            // original-parity traces.
            reusable_cloaks: false,
            // Item rebalances and their extra cue are post-port extensions.
            item_gameplay: robin_engine::gameplay_config::ItemGameplayConfig::classic(),
            noise_distraction_feedback: false,
            // Original traces use the two-camp distinct-ID rules, including
            // Royalist/Lacklandist NPC combat.
            diplomacy: false,
            npc_faction_wars: true,
            // Advanced gesture recognition and quality-scaled damage are
            // post-port extensions. Original parity traces must retain the
            // shipped combat input and damage rules.
            more_combat_gestures: false,
            gesture_quality_damage: false,
            // Shared-vision fog is also a post-port gameplay rule.
            fog_of_war: false,
            reversible_background_patches: false,
            script_enabled: self.script_enabled,
            highlander: self.highlander,
            highlander2: self.highlander2,
            golden_eye: self.golden_eye,
            ignore_default_loose: self.ignore_default_loose,
            bypass_fog_sprites_crash: self.bypass_fog_sprites_crash,
            amount_of_speaking: self.amount_of_speaking,
            synchronous_pathfinding,
            sherwood_trading: false,
            // Original traces contain no Rust-authored timer or ambience
            // schedule, so leaving the authoring gates enabled is inert.
            enable_timed_missions: true,
            enable_dynamic_ambience: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TraceDifficulty {
    Easy,
    Medium,
    Hard,
}

impl From<TraceDifficulty> for robin_engine::player_profile::DifficultyLevel {
    fn from(value: TraceDifficulty) -> Self {
        match value {
            TraceDifficulty::Easy => Self::Easy,
            TraceDifficulty::Medium => Self::Medium,
            TraceDifficulty::Hard => Self::Hard,
        }
    }
}

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TraceStartState {
    MissionStart,
    LoadedSave,
}

pub(crate) const TRACE_SCHEMA_VERSION: u32 = 16;
pub(crate) const LAST_TRACE_SCHEMA_WITHOUT_DRAW_VIEW: u32 = 16;
pub(crate) const OLDEST_SUPPORTED_TRACE_SCHEMA: u32 = 12;

//! Current trace DTOs and scalar representations; wire field order and types are unchanged.
use super::{
    Action, BTreeMap, Deserialize, EntityId, EntityIdKind, MapPoint, Serialize, WorldPoint3D,
    bitcode, sha256_hex,
};
use base64::Engine as _;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "command", rename_all = "snake_case")]
pub(super) enum TraceDirectorCompletion {
    CameraGoto,
    ZoomLevel,
}

impl From<TraceDirectorCompletion> for robin_engine::engine::DirectorCompletion {
    fn from(value: TraceDirectorCompletion) -> Self {
        match value {
            TraceDirectorCompletion::CameraGoto => Self::CameraGoto,
            TraceDirectorCompletion::ZoomLevel => Self::ZoomLevel,
        }
    }
}

/// JSON trace header and the embedded header layout of
/// [`BinaryTraceHeaderV68`].
///
/// ON-DISK FORMAT INVARIANT: changing any field, field order, or field type
/// changes bitcode's native trace layout. Such a change must bump
/// `TRACE_NATIVE_VERSION`, freeze the old layout in a version-named struct,
/// and add an explicit decoder branch for it. Never silently edit this type
/// while retaining native trace version 68.
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceHeader {
    #[serde(rename = "type")]
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
    #[serde(default)]
    pub(super) random_input_seed: Option<u32>,
    pub(super) sim_config: TraceSimConfig,
    pub(super) campaign: TraceCampaign,
    pub(super) motion_grid: TraceMotionGrid,
    /// Current session-boundary state omitted by the original game's save payload.
    /// Early schema-16 interactive recordings predate this additive overlay;
    /// `None` selects the narrowly-scoped legacy reconstruction below while a
    /// present (including empty) list remains authoritative.
    #[serde(default)]
    pub(super) initial_npc_transients: Option<Vec<TraceInitialNpcTransient>>,
    #[serde(default)]
    pub(super) initial_save: Option<TraceInitialSave>,
}

#[derive(
    Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, bitcode::Encode, bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceInitialNpcTransient {
    pub(super) creation_order: u32,
    pub(super) maximal_visibility: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceInitialSave {
    pub(super) format: String,
    pub(super) source_profile: TraceSaveSourceProfile,
    pub(super) encoding: String,
    pub(super) byte_length: u64,
    pub(super) sha256: String,
    pub(super) slot: String,
    pub(super) header_version: u32,
    pub(super) mission_id: u32,
    pub(super) stream_version: u32,
    pub(super) data: String,
}

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub(super) enum TraceSaveSourceProfile {
    LinuxI386RhsgV48,
    WindowsI386GshrV48,
}

impl TraceSaveSourceProfile {
    pub(super) fn expected_magic(self) -> &'static [u8; 4] {
        match self {
            Self::LinuxI386RhsgV48 => b"RHSG",
            Self::WindowsI386GshrV48 => b"GSHR",
        }
    }
}

impl TraceInitialSave {
    pub(super) fn decode_and_validate(&self, expected_mission_id: u32) -> Result<Vec<u8>, String> {
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
pub(super) struct TraceSimConfig {
    pub(super) difficulty: TraceDifficulty,
    pub(super) script_enabled: bool,
    pub(super) highlander: bool,
    pub(super) highlander2: bool,
    pub(super) golden_eye: bool,
    pub(super) ignore_default_loose: bool,
    pub(super) bypass_fog_sprites_crash: bool,
    pub(super) amount_of_speaking: u16,
}

impl TraceSimConfig {
    pub(super) fn to_sim_config(
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
pub(super) enum TraceDifficulty {
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
pub(super) enum TraceStartState {
    MissionStart,
    LoadedSave,
}

pub(super) const TRACE_SCHEMA_VERSION: u32 = 16;
pub(super) const LAST_TRACE_SCHEMA_WITHOUT_DRAW_VIEW: u32 = 16;
pub(super) const OLDEST_SUPPORTED_TRACE_SCHEMA: u32 = 12;

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceCampaign {
    pub(super) version: u32,
    pub(super) values: Vec<i32>,
    pub(super) ares: i8,
    pub(super) missions: Vec<TraceCampaignMission>,
    pub(super) accessible_mission_indices: Vec<usize>,
    pub(super) pending_accessible_mission_indices: Vec<usize>,
    pub(super) last_mission_index: Option<usize>,
    pub(super) current_mission_index: Option<usize>,
    pub(super) next_mission_index: Option<usize>,
    pub(super) blazon_mission_index: Option<usize>,
    pub(super) last_played_mission_indices: Vec<usize>,
    pub(super) last_pseudo_mission_status: u32,
    pub(super) last_pseudo_mission_id: u32,
    pub(super) characters: Vec<TraceCampaignCharacter>,
    pub(super) gang_indices: Vec<usize>,
    pub(super) reservist_indices: Vec<usize>,
    pub(super) mission_team_indices: Vec<usize>,
    pub(super) peasant_names: Vec<String>,
    pub(super) reservists_are_back: bool,
    pub(super) collected_relics: Vec<u32>,
    pub(super) production_sectors: Vec<TraceProductionSector>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceCampaignMission {
    pub(super) profile_index: u32,
    pub(super) profile_id: u32,
    pub(super) mission: String,
    pub(super) proto_level: String,
    pub(super) age: u16,
    pub(super) blazon_price: u16,
    pub(super) status: u32,
    pub(super) ares_state_succeeded: i8,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceCampaignCharacter {
    pub(super) profile_index: u32,
    pub(super) profile_name: String,
    pub(super) instanced: bool,
    pub(super) status: TracePcStatus,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TracePcStatus {
    pub(super) hand_to_hand: TraceSkill,
    pub(super) bow: TraceSkill,
    pub(super) life_points: i16,
    pub(super) in_coma: bool,
    pub(super) ales: u16,
    pub(super) arrows: u16,
    pub(super) apples: u16,
    pub(super) rations: u16,
    pub(super) stones: u16,
    pub(super) wasp_nests: u16,
    pub(super) nets: u16,
    pub(super) plants: u16,
    pub(super) purses: u16,
    pub(super) name: String,
    pub(super) beam_me_index_in_sherwood: i16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceSkill {
    pub(super) capacity: u32,
    pub(super) experience: u32,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceProductionSector {
    r#type: u32,
    pub(super) speed: u16,
    pub(super) amount: u16,
    pub(super) produced_amount: u16,
    pub(super) max_amount_reached: bool,
    pub(super) occupants: Vec<TraceProductionOccupant>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceProductionOccupant {
    pub(super) character_index: usize,
    pub(super) x: TraceFloat,
    pub(super) y: TraceFloat,
    pub(super) obstacle: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceMotionGrid {
    pub(super) layers: Vec<TraceMotionLayer>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceMotionLayer {
    pub(super) layer: u16,
    pub(super) lines: Vec<TraceMotionLine>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceMotionLine {
    pub(super) index: u16,
    pub(super) a: TracePoint,
    pub(super) b: TracePoint,
    pub(super) type_mask: i32,
    pub(super) associated_sector: i16,
    pub(super) active: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceMotionLineChange {
    pub(super) layer: u16,
    pub(super) index: u16,
    pub(super) active: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub(super) enum TracePathEvent {
    Queued {
        actor: TraceEntityId,
        antagonist: Option<TraceEntityId>,
        layer: u16,
        area: u16,
        source: TracePoint,
        goal: TracePoint,
        half_diagonal_index: u16,
        half_diagonal: TracePoint,
        animation: u32,
        reverse: bool,
        speed: u8,
        tolerance: TraceFloat,
        use_first_point: bool,
    },
    Completed {
        actor: TraceEntityId,
        antagonist: Option<TraceEntityId>,
        layer: u16,
        area: u16,
        source: TracePoint,
        goal: TracePoint,
        half_diagonal_index: u16,
        half_diagonal: TracePoint,
        animation: u32,
        reverse: bool,
        speed: u8,
        tolerance: TraceFloat,
        use_first_point: bool,
        valid: bool,
        waypoints: Vec<TracePoint>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceRngBatch {
    pub(super) first_index: usize,
    pub(super) values: Vec<u32>,
    pub(super) callsite_offsets: Vec<u32>,
    pub(super) main_thread: Vec<bool>,
    pub(super) domains: Vec<TraceRngDomain>,
}

impl TraceRngBatch {
    pub(super) fn validate(&self) {
        let draw_count = self.values.len();
        assert_eq!(
            draw_count,
            self.callsite_offsets.len(),
            "RNG callsite stream has a different length than its values"
        );
        assert_eq!(
            draw_count,
            self.main_thread.len(),
            "RNG thread-origin stream has a different length than its values"
        );
        assert_eq!(
            draw_count,
            self.domains.len(),
            "RNG domain stream has a different length than its values"
        );
        for (index, (domain, main_thread)) in self
            .domains
            .iter()
            .copied()
            .zip(self.main_thread.iter().copied())
            .enumerate()
        {
            assert!(
                domain != TraceRngDomain::Simulation || main_thread,
                "simulation RNG draw {} (global index {}) occurred off the main thread; its global order is not deterministically replayable",
                index,
                self.first_index + index,
            );
        }
    }

    pub(super) fn gameplay_draw_count(&self) -> usize {
        self.validate();
        self.domains
            .iter()
            .filter(|domain| **domain == TraceRngDomain::Simulation)
            .count()
    }

    pub(super) fn gameplay_callsite_offsets(&self) -> Vec<u32> {
        self.validate();
        self.callsite_offsets
            .iter()
            .copied()
            .zip(self.domains.iter().copied())
            .filter_map(|(offset, domain)| (domain == TraceRngDomain::Simulation).then_some(offset))
            .collect()
    }

    pub(super) fn gameplay_values(&self) -> Vec<u32> {
        self.validate();
        self.values
            .iter()
            .copied()
            .zip(self.domains.iter().copied())
            .filter_map(|(value, domain)| (domain == TraceRngDomain::Simulation).then_some(value))
            .collect()
    }
}

#[derive(
    Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub(super) enum TraceRngDomain {
    Simulation,
    Audio,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceRngPrefix {
    #[allow(dead_code)]
    r#type: String,
    pub(super) draws: TraceRngBatch,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceRngOnly {
    #[serde(rename = "type")]
    pub(super) record_type: String,
    pub(super) draws: TraceRngBatch,
    pub(super) final_frame: u64,
    pub(super) frame_count: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct TraceRecordMarker {
    #[serde(rename = "type")]
    pub(super) record_type: Option<String>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    pub(super) bitcode::Encode,
    pub(super) bitcode::Decode,
)]
pub(super) struct TraceEntityId {
    pub(super) kind: TraceEntityKind,
    pub(super) index: u32,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub(super) enum TraceEntityKind {
    Pc,
    Soldier,
    Civilian,
    Fx,
    Target,
    Bonus,
    Scroll,
    Projectile,
    Net,
}

impl From<TraceEntityId> for EntityId {
    fn from(value: TraceEntityId) -> Self {
        let kind = match value.kind {
            TraceEntityKind::Pc => EntityIdKind::Pc,
            TraceEntityKind::Soldier => EntityIdKind::Soldier,
            TraceEntityKind::Civilian => EntityIdKind::Civilian,
            TraceEntityKind::Fx => EntityIdKind::Fx,
            TraceEntityKind::Target => EntityIdKind::Target,
            TraceEntityKind::Bonus => EntityIdKind::Bonus,
            TraceEntityKind::Scroll => EntityIdKind::Scroll,
            TraceEntityKind::Projectile => EntityIdKind::Projectile,
            TraceEntityKind::Net => EntityIdKind::Net,
        };
        EntityId::new(value.index, kind)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceFloat {
    pub(super) bits: u32,
}

impl TraceFloat {
    pub(super) fn value(self) -> f32 {
        f32::from_bits(self.bits)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TracePoint {
    pub(super) x: TraceFloat,
    pub(super) y: TraceFloat,
}

impl From<TracePoint> for MapPoint {
    fn from(value: TracePoint) -> Self {
        Self::new(value.x.value(), value.y.value())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TracePoint3 {
    pub(super) x: TraceFloat,
    pub(super) y: TraceFloat,
    pub(super) z: TraceFloat,
}

impl From<TracePoint3> for WorldPoint3D {
    fn from(value: TracePoint3) -> Self {
        Self::new(value.x.value(), value.y.value(), value.z.value())
    }
}

#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum TraceCommand {
    BoxSelect {
        first: TracePoint,
        second: TracePoint,
        append: bool,
    },
    GroupMove {
        actors: Vec<TraceEntityId>,
        destination: TracePoint,
        running: bool,
        show_marker: bool,
        goal_sector: i16,
        goal_layer: u16,
    },
    LaunchInteraction {
        actor: TraceEntityId,
        target: TraceEntityId,
        /// Original's raw numeric command; `original_command_name` is its
        /// stable name and drives replay. Retained for lossless caching.
        original_command: u32,
        original_command_name: String,
        running: bool,
    },
    LaunchSelfAbility {
        actor: TraceEntityId,
        original_command: u32,
        original_command_name: String,
    },
    LaunchGroundTarget {
        actor: TraceEntityId,
        target: TracePoint3,
        original_command: u32,
        original_command_name: String,
        original_target_field: u32,
        titbit_layer: u16,
    },
    LaunchScrollRead {
        actor: TraceEntityId,
        target: TraceEntityId,
        running: bool,
    },
    SwordStrike {
        actor: TraceEntityId,
        target: TraceEntityId,
        original_command: u32,
        original_command_name: String,
        with_seek: bool,
        #[serde(default = "missing_legacy_seek_distance")]
        seek_distance: f32,
    },
    SelectPc {
        pc: TraceEntityId,
        append: bool,
    },
    UnselectAllPcs,
    StopPc {
        pc: TraceEntityId,
    },
    SelectAction {
        pc: TraceEntityId,
        action: TraceAction,
        /// Original's raw numeric action, retained so the cache round trip is
        /// lossless. `action` is its resolved name; replay uses the name.
        original_action: u32,
    },
    CancelAction {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        /// Always `no_action`: selecting no action is
        /// recorded as cancel_action, but the recorder still emits the pair.
        action: TraceAction,
        original_action: u32,
    },
    OrientActionAt {
        action: TraceAction,
        original_action: u32,
        actor: TraceEntityId,
        mouse_map: TracePoint,
        target: TracePoint3,
    },
    MakePcFast {
        entity: TraceEntityId,
    },
    CrouchDown,
    StandUp,
    // NOTE: with the bitcode-encoded native format, ANY change to this enum
    // (adding, removing, or editing a variant, anywhere) changes the on-disk
    // shape. Bump TRACE_NATIVE_VERSION and migrate existing native traces —
    // converted recordings may no longer have a JSONL source to rebuild from.
    DropAleAt {
        actor: TraceEntityId,
        target: TracePoint,
        running: bool,
    },
    ShieldSelectProtected {
        actor: TraceEntityId,
        protected_pc: TraceEntityId,
    },
    BoxUnselect {
        first: TracePoint,
        second: TracePoint,
        append: bool,
    },
    RaiseShieldWithDanger {
        actor: TraceEntityId,
        protected_pc: TraceEntityId,
        danger_point: TracePoint3,
        danger_point_layer: u16,
    },
    TeleportSelected {
        destination: TracePoint,
        goal_sector: i16,
        goal_layer: u16,
    },
    SelectAllPcs,
    UnselectPc {
        pc: TraceEntityId,
    },
    SelectActionIndex {
        index: u32,
    },
    SetLockAlt {
        on: bool,
    },
    KeyControl,
    KeyReleaseControl,
    StartMacro {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    DeleteMacro {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    StartRecordingMacro {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    ChangeQaMemory {
        slot: u8,
    },
    /// A click the Original refused after it had already barked at the
    /// player. The click itself is a raw mouse message and is never
    /// recorded, so without this the bark's speech resolution arrives with
    /// nothing that caused it.
    HeroRefusedAction {
        actor: TraceEntityId,
        action: TraceAction,
        original_action: u32,
        #[serde(default)]
        target: Option<TraceEntityId>,
        reason: String,
    },
    BeggarDontTalkStamp {
        entity: TraceEntityId,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub(super) enum TraceAction {
    NoAction,
    Bow,
    Hit,
    HitHard,
    Purse,
    Stone,
    Shield,
    BigShield,
    Strangle,
    Lever,
    HelpToClimb,
    Apple,
    Ale,
    Eat,
    Guzzle,
    Listen,
    Heal,
    Net,
    Beggar,
    WaspNest,
    Whistle,
    Climb,
    Jump,
    Search,
    Resuscitate,
    LittleJohnCarry,
    FarmerCarry,
    Tie,
    Lockpick,
    Execute,
    Test,
}

impl From<TraceAction> for Action {
    fn from(value: TraceAction) -> Self {
        match value {
            TraceAction::NoAction => Self::NoAction,
            TraceAction::Bow => Self::Bow,
            TraceAction::Hit => Self::Hit,
            TraceAction::HitHard => Self::HitHard,
            TraceAction::Purse => Self::Purse,
            TraceAction::Stone => Self::Stone,
            TraceAction::Shield => Self::Shield,
            TraceAction::BigShield => Self::BigShield,
            TraceAction::Strangle => Self::Strangle,
            TraceAction::Lever => Self::Lever,
            TraceAction::HelpToClimb => Self::HelpToClimb,
            TraceAction::Apple => Self::Apple,
            TraceAction::Ale => Self::Ale,
            TraceAction::Eat => Self::Eat,
            TraceAction::Guzzle => Self::Guzzle,
            TraceAction::Listen => Self::Listen,
            TraceAction::Heal => Self::Heal,
            TraceAction::Net => Self::Net,
            TraceAction::Beggar => Self::Beggar,
            TraceAction::WaspNest => Self::WaspNest,
            TraceAction::Whistle => Self::Whistle,
            TraceAction::Climb => Self::Climb,
            TraceAction::Jump => Self::Jump,
            TraceAction::Search => Self::Search,
            TraceAction::Resuscitate => Self::Resuscitate,
            TraceAction::LittleJohnCarry => Self::LittleJohnCarry,
            TraceAction::FarmerCarry => Self::FarmerCarry,
            TraceAction::Tie => Self::Tie,
            TraceAction::Lockpick => Self::Lockpick,
            TraceAction::Execute => Self::Execute,
            TraceAction::Test => Self::Test,
        }
    }
}

pub(super) fn missing_legacy_seek_distance() -> f32 {
    // Schema-16 recordings made before the additive seek-distance diagnostic
    // cannot reconstruct it. A quiet NaN is outside the valid distance domain,
    // survives the unchanged native f32 layout, and is mapped back to `None`
    // before command admission.
    f32::NAN
}

/// Element layout embedded in version-68 native frame records.
///
/// ON-DISK FORMAT INVARIANT: do not change fields, their order, or their
/// types without bumping `TRACE_NATIVE_VERSION` and freezing this layout in a
/// version-named compatibility type, as done by [`v67::TraceElementV67`].
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceElement {
    pub(super) entity_id: TraceEntityId,
    pub(super) creation_order: u32,
    pub(super) class_id: u16,
    pub(super) kind: TraceEntityKind,
    pub(super) active: bool,
    pub(super) blipped: bool,
    pub(super) unreachable: bool,
    pub(super) surface_id: u32,
    pub(super) posture: u32,
    pub(super) position_map: TracePoint,
    pub(super) old_position_map: TracePoint,
    pub(super) position_goal_map: TracePoint,
    pub(super) elevation: TraceFloat,
    pub(super) old_elevation: TraceFloat,
    pub(super) increment_map: TracePoint,
    /// Missing in early schema-16 frames. Presence, including an authoritative
    /// `false`, must survive conversion to the native trace.
    #[serde(default)]
    pub(super) increment_map_valid: Option<bool>,
    pub(super) movement_map: TracePoint,
    pub(super) layer: u16,
    pub(super) layer_goal: u16,
    pub(super) sector: u16,
    pub(super) direction: i16,
    pub(super) direction_goal: i16,
    pub(super) moving: bool,
    pub(super) moving_map: bool,
    pub(super) sprite_row: u16,
    pub(super) sprite_frame: u16,
    pub(super) sprite_frame_count: u16,
    #[serde(default)]
    pub(super) actor: Option<TraceActor>,
    #[serde(default)]
    pub(super) human: Option<TraceHuman>,
    #[serde(default)]
    pub(super) pc: Option<TraceElementPc>,
    #[serde(default)]
    pub(super) ai: Option<TraceAi>,
    #[serde(default)]
    pub(super) detection: Option<TraceDetection>,
    /// Whole-entity serialized position/sprite frontier. Early schema-16
    /// recordings omit it; JSON null is the native-layout-compatible marker
    /// for "not recorded" and is excluded from logical comparison.
    #[serde(default = "missing_legacy_trace_json_value")]
    pub(super) runtime: TraceJsonValue,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceActor {
    pub(super) action_state: u32,
    pub(super) animation: u32,
    pub(super) command: u16,
    pub(super) command_name: String,
    pub(super) motion_state: u32,
    pub(super) wait_time: u32,
    #[serde(default)]
    pub(super) passing_door_directly: bool,
    /// Explicitly null when there is no active PassDoor.
    #[serde(default, deserialize_with = "deserialize_nullable_pass_door")]
    pub(super) active_pass_door: Option<TracePassDoor>,
    /// Rust does not yet expose a stable public current-sequence snapshot with
    /// Original's element identities.
    /// TODO(parity-sequence): compare the remaining fields once that capture
    /// can be produced without walking mutable sequence-manager internals.
    #[serde(default, deserialize_with = "deserialize_nullable_sequence_element")]
    pub(super) sequence_element: Option<TraceSequenceElement>,
    /// PositionInterface diagnostics. Kept as a cache-safe JSON
    /// tree because it is observational evidence rather than comparable
    /// engine state yet.
    #[serde(default = "missing_legacy_trace_json_value")]
    pub(super) position_interface: TraceJsonValue,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TracePassDoor {
    pub(super) gate_id: u32,
    pub(super) direct: bool,
    pub(super) direction: i16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceSequenceElement {
    pub(super) id: u32,
    #[serde(rename = "type")]
    pub(super) element_type: u8,
    pub(super) state: u32,
    pub(super) command_level: u16,
    pub(super) command: u16,
    pub(super) command_name: String,
    pub(super) order_count: u16,
    pub(super) priority: u32,
    pub(super) posture_after_transition: u32,
    pub(super) action_state_after_transition: u32,
    #[serde(default, deserialize_with = "deserialize_nullable_sequence_movement")]
    pub(super) movement: Option<TraceSequenceMovement>,
    /// Current sequence topology and active-order diagnostics. These are
    /// nullable or command-shaped in the Original recorder, so retaining the
    /// draft payload verbatim is safer than inventing a false common shape.
    #[serde(default, deserialize_with = "deserialize_nullable_trace_json_value")]
    pub(super) following: Option<TraceJsonValue>,
    #[serde(default, deserialize_with = "deserialize_nullable_trace_json_value")]
    pub(super) postponed: Option<TraceJsonValue>,
    #[serde(default, deserialize_with = "deserialize_nullable_trace_json_value")]
    pub(super) current_order: Option<TraceJsonValue>,
    #[serde(default, deserialize_with = "deserialize_nullable_trace_json_value")]
    pub(super) movement_payload: Option<TraceJsonValue>,
}

pub(super) fn deserialize_nullable_sequence_movement<'de, D>(
    deserializer: D,
) -> Result<Option<TraceSequenceMovement>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TraceSequenceMovement>::deserialize(deserializer)
}

pub(super) fn deserialize_nullable_trace_json_value<'de, D>(
    deserializer: D,
) -> Result<Option<TraceJsonValue>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TraceJsonValue>::deserialize(deserializer)
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceSequenceMovement {
    /// Absent in current schema-16 traces when the movement-element
    /// constructor does not initialize `maction` (for example WAIT_FREE_LIFT).
    #[serde(default)]
    pub(super) action: Option<u32>,
    #[serde(default)]
    pub(super) pass_door: Option<TracePassDoor>,
}

pub(super) fn deserialize_nullable_pass_door<'de, D>(
    deserializer: D,
) -> Result<Option<TracePassDoor>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TracePassDoor>::deserialize(deserializer)
}

pub(super) fn deserialize_nullable_sequence_element<'de, D>(
    deserializer: D,
) -> Result<Option<TraceSequenceElement>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TraceSequenceElement>::deserialize(deserializer)
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceHuman {
    pub(super) life_points: i16,
    pub(super) dead: bool,
    pub(super) unconscious: bool,
    pub(super) camp: String,
    pub(super) original_camp: i32,
    pub(super) vip: bool,
    pub(super) civilian: bool,
    pub(super) opponents: Vec<TraceEntityId>,
    pub(super) opponent_jump_lines: Vec<Option<TraceJumpLine>>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceJumpLine {
    pub(super) a: TracePoint,
    pub(super) b: TracePoint,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceElementPc {
    pub(super) ammo: TraceElementAmmo,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceElementAmmo {
    pub(super) ales: u16,
    pub(super) apples: u16,
    pub(super) arrows: u16,
    pub(super) nets: u16,
    pub(super) plants: u16,
    pub(super) purses: u16,
    pub(super) rations: u16,
    pub(super) stones: u16,
    pub(super) wasp_nests: u16,
}

pub(super) fn deserialize_nullable_u16<'de, D>(deserializer: D) -> Result<Option<u16>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<u16>::deserialize(deserializer)
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceAi {
    pub(super) state: u32,
    pub(super) substate: u32,
    #[serde(default)]
    pub(super) script_locked: bool,
    #[serde(default)]
    pub(super) locked: bool,
    #[serde(default)]
    pub(super) locks: u8,
    #[serde(default)]
    pub(super) was_busy: bool,
    #[serde(default)]
    pub(super) very_busy: bool,
    #[serde(default)]
    pub(super) macro_timer_running: bool,
    #[serde(default)]
    pub(super) macro_timer_ring: u32,
    /// Explicitly null for an inactive macro.
    #[serde(default, deserialize_with = "deserialize_nullable_u16")]
    pub(super) macro_cursor: Option<u16>,
    #[serde(default)]
    pub(super) macro_remaining: u16,
    #[serde(default)]
    pub(super) macro_in_progress: bool,
    #[serde(default)]
    pub(super) list_us: Vec<TraceEntityId>,
    #[serde(default)]
    pub(super) list_them: Vec<TraceEntityId>,
    /// Authoritative jump-line reference, explicitly null when absent.
    #[serde(default, deserialize_with = "deserialize_nullable_jump_line")]
    pub(super) my_line_jump: Option<TraceJumpLine>,
}

pub(super) fn deserialize_nullable_jump_line<'de, D>(
    deserializer: D,
) -> Result<Option<TraceJumpLine>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TraceJumpLine>::deserialize(deserializer)
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceDetection {
    pub(super) suspects: Vec<u16>,
    pub(super) maximal_suspect: u16,
    pub(super) maximal_visibility: u32,
    pub(super) view_status: u8,
    pub(super) alert_status: u32,
    pub(super) detectables: Vec<TraceDetectable>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceDetectable {
    #[serde(rename = "type")]
    pub(super) detectable_type: u32,
    pub(super) target: TraceEntityId,
    pub(super) seen_now: bool,
    pub(super) seen_last_frame: bool,
    pub(super) heard_last_frame: bool,
    pub(super) shadow_seen_now: bool,
    pub(super) shadow_seen_last_frame: bool,
    pub(super) last_visibility: TraceFloat,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceVisibilityQuery {
    pub(super) origin: TracePoint3,
    pub(super) destination: TracePoint3,
    pub(super) result: bool,
    pub(super) cache_hit: bool,
    pub(super) cache_key: u64,
    pub(super) cache_offset: u64,
    pub(super) candidate_count: u16,
    pub(super) reason: String,
    pub(super) blocking_obstacle: Option<TraceSightObstacle>,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceSightObstacle {
    pub(super) id: u32,
    pub(super) index: i64,
    pub(super) type_mask: i32,
    pub(super) types: TraceSightObstacleTypes,
    pub(super) active: bool,
    pub(super) on_ground: bool,
    pub(super) layer: u16,
    pub(super) sector: u16,
    pub(super) box_ground: TraceSightObstacleBox,
    pub(super) points: Vec<TraceSightObstaclePoint>,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceSightObstacleTypes {
    pub(super) solid: bool,
    pub(super) opaque: bool,
    pub(super) projection_area: bool,
    pub(super) mouse: bool,
    pub(super) shield: bool,
    pub(super) show_shadow_polygon: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceSightObstacleBox {
    pub(super) min: TracePoint,
    pub(super) max: TracePoint,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceSightObstaclePoint {
    pub(super) x: TraceFloat,
    pub(super) y: TraceFloat,
    pub(super) z_top: TraceFloat,
    pub(super) z_bottom: TraceFloat,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceResolvedExclamation {
    pub(super) actor: TraceEntityId,
    pub(super) identifier: u32,
    pub(super) exclamation_id: u16,
    pub(super) selected_variant: i32,
    pub(super) selected_entry: Option<u32>,
    pub(super) duration_frames: u32,
}

/// One exact, ordered original-game motion position commit.
/// This additive diagnostic is absent unless the Original recorder was run
/// with `RH_PARITY_MOVEMENT_STEPS` enabled.
#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceMovementStep {
    pub(super) entity: TraceEntityId,
    pub(super) order_id: u32,
    pub(super) order_action: u32,
    pub(super) animation: u32,
    pub(super) motion_method: u32,
    pub(super) pre_position: TracePoint,
    pub(super) old_position: TracePoint,
    pub(super) goal: TracePoint,
    pub(super) cached_increment: TracePoint,
    pub(super) frame_distance_raw: TraceFloat,
    pub(super) speed_factor: TraceFloat,
    pub(super) effective_distance: TraceFloat,
    pub(super) anti_collision: bool,
    pub(super) reverse: bool,
    pub(super) raw_post_position: TracePoint,
    pub(super) raw_committed_delta: TracePoint,
    pub(super) post_position: TracePoint,
    pub(super) committed_delta: TracePoint,
    pub(super) goal_reached: bool,
    pub(super) snapped_to_goal: bool,
}

/// One exact, ordered original-game flight execution.
#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceFlightStep {
    pub(super) entity: TraceEntityId,
    pub(super) order_id: u32,
    pub(super) order_action: u32,
    pub(super) animation: u32,
    pub(super) flight_style: u32,
    pub(super) entry_position: TracePoint3,
    pub(super) entry_position_map: TracePoint,
    pub(super) old_position: TracePoint3,
    pub(super) old_position_map: TracePoint,
    pub(super) goal: TracePoint3,
    pub(super) cached_increment: TracePoint3,
    pub(super) applied_increment: TracePoint3,
    pub(super) raw_post_position: TracePoint3,
    pub(super) raw_post_position_map: TracePoint,
    pub(super) motion_state: u32,
    pub(super) post_position: TracePoint3,
    pub(super) post_position_map: TracePoint,
    pub(super) snapped_to_goal: bool,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceRouteConstructionEvent {
    pub(super) kind: String,
    pub(super) actor: TraceEntityId,
    pub(super) source: TracePoint,
    pub(super) source_sector: u16,
    pub(super) source_level: u16,
    pub(super) goal: TracePoint,
    pub(super) goal_sector: u16,
    pub(super) goal_level: u16,
    pub(super) gates: Vec<TraceRouteGate>,
    /// Schema-16 may extend route events while its diagnostic contract is
    /// being exercised against real recordings. Retain every additive field
    /// in the native cache instead of silently discarding useful evidence.
    #[serde(flatten)]
    pub(super) draft_diagnostics: BTreeMap<String, TraceJsonValue>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceRouteGate {
    pub(super) gate_id: u32,
    pub(super) direct: bool,
    pub(super) sector_out: u16,
    pub(super) level_out: u16,
    pub(super) sector_in: u16,
    pub(super) level_in: u16,
    #[serde(flatten)]
    pub(super) draft_diagnostics: BTreeMap<String, TraceJsonValue>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TracePointBox {
    pub(super) top_left: TracePoint,
    pub(super) bottom_right: TracePoint,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TracePopupEvent {
    #[serde(default)]
    pub(super) ordinal: Option<u64>,
    pub(super) stage: String,
    #[serde(default)]
    pub(super) universal_frame_counter: Option<u64>,
    #[serde(default)]
    pub(super) last_popup_frame: Option<u64>,
    #[serde(default)]
    pub(super) last_popup_frame_initialized: Option<bool>,
    #[serde(default)]
    pub(super) same_frame_suppressed: Option<bool>,
    #[serde(default)]
    pub(super) colorize_background: Option<bool>,
    #[serde(default)]
    pub(super) modal: Option<bool>,
    #[serde(default)]
    pub(super) centered: Option<bool>,
    #[serde(default)]
    pub(super) popup_text_id: Option<u64>,
    #[serde(default)]
    pub(super) source_surface: Option<u64>,
    #[serde(default)]
    pub(super) remove_mouse: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceForecastGate {
    pub(super) gate_id: u32,
    pub(super) kind: String,
    pub(super) active: bool,
    pub(super) point_out: TracePoint,
    pub(super) sector_out: u16,
    pub(super) level_out: u16,
    pub(super) point_in: TracePoint,
    pub(super) sector_in: u16,
    pub(super) level_in: u16,
    pub(super) penalty: TraceFloat,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceAiForecastInput {
    pub(super) position: TracePoint,
    pub(super) sector: u16,
    pub(super) level: u16,
    #[serde(default)]
    pub(super) direction: Option<u16>,
    pub(super) passing_door: bool,
    #[serde(default)]
    pub(super) passing_door_directly: Option<bool>,
    #[serde(default)]
    pub(super) door: Option<TraceForecastGate>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceAiForecastResolved {
    pub(super) position: TracePoint,
    pub(super) sector: u16,
    pub(super) level: u16,
    #[serde(default)]
    pub(super) direction: Option<u16>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceAiForecastEvent {
    pub(super) ordinal: u64,
    #[serde(default)]
    pub(super) phase: Option<String>,
    pub(super) target: TraceEntityId,
    pub(super) input: TraceAiForecastInput,
    pub(super) moving_upwards: bool,
    pub(super) resolution: String,
    pub(super) resolved: TraceAiForecastResolved,
    #[serde(default)]
    pub(super) selected_building_exit: Option<TraceForecastGate>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceAlertEligibility {
    pub(super) rank: bool,
    pub(super) able_to_help: Option<bool>,
    pub(super) allowed_to_leave_post: Option<bool>,
    pub(super) can_call: Option<bool>,
    pub(super) max_radius: Option<bool>,
    pub(super) squared_radius: Option<bool>,
    pub(super) capacity: Option<bool>,
    pub(super) think: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceAlertFormationEvent {
    #[serde(default)]
    pub(super) ordinal: Option<u64>,
    pub(super) stage: String,
    pub(super) invocation: u64,
    #[serde(default)]
    pub(super) officer: Option<TraceEntityId>,
    #[serde(default)]
    pub(super) officer_position: Option<TracePoint>,
    #[serde(default)]
    pub(super) soldier_scan_count: Option<u16>,
    #[serde(default)]
    pub(super) officer_in_building: Option<bool>,
    #[serde(default)]
    pub(super) scan_index: Option<u16>,
    #[serde(default)]
    pub(super) candidate: Option<TraceEntityId>,
    #[serde(default)]
    pub(super) active: Option<bool>,
    #[serde(default)]
    pub(super) script_locked: Option<bool>,
    #[serde(default)]
    pub(super) eligibility: Option<TraceAlertEligibility>,
    #[serde(default)]
    pub(super) rejection_stage: Option<String>,
    #[serde(default)]
    pub(super) insertion_index: Option<u16>,
    #[serde(default)]
    pub(super) squared_distance: Option<TraceFloat>,
    #[serde(default)]
    pub(super) normalized_contribution: Option<TracePoint>,
    #[serde(default)]
    pub(super) running_average: Option<TracePoint>,
    #[serde(default)]
    pub(super) selected_index: Option<u16>,
    #[serde(default)]
    pub(super) outside_step: Option<u16>,
    #[serde(default)]
    pub(super) direction: Option<u16>,
    #[serde(default)]
    pub(super) soldier_count: Option<u16>,
    #[serde(default)]
    pub(super) slot_index: Option<u16>,
    #[serde(default)]
    pub(super) layer: Option<u16>,
    #[serde(default)]
    pub(super) sector: Option<u16>,
    #[serde(default)]
    pub(super) destination: Option<TracePoint>,
    #[serde(default)]
    pub(super) destination_box: Option<TracePointBox>,
    #[serde(default)]
    pub(super) position_authorized: Option<bool>,
    #[serde(default)]
    pub(super) thick_corridor_authorized: Option<bool>,
    #[serde(default)]
    pub(super) blocker_ids_available: Option<bool>,
    #[serde(default)]
    pub(super) blocking_motion_line_ids: Option<Vec<u32>>,
    #[serde(default)]
    pub(super) blocking_mobile_line_ids: Option<Vec<u32>>,
    #[serde(default)]
    pub(super) accepted: Option<bool>,
    #[serde(default)]
    pub(super) result: Option<String>,
    #[serde(default)]
    pub(super) average_direction: Option<u16>,
    #[serde(default)]
    pub(super) selected_direction: Option<u16>,
    #[serde(default)]
    pub(super) final_sector: Option<u16>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceGoToSource {
    pub(super) point: TracePoint,
    pub(super) layer: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceGoToDestination {
    pub(super) point: TracePoint,
    pub(super) sector: Option<u16>,
    pub(super) layer: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceGoToAuthorizationEvent {
    pub(super) ordinal: u64,
    pub(super) actor: Option<TraceEntityId>,
    #[serde(default)]
    pub(super) source: Option<TraceGoToSource>,
    #[serde(default)]
    pub(super) move_box: Option<TracePointBox>,
    pub(super) destination: TraceGoToDestination,
    pub(super) requested_flags: u16,
    pub(super) effective_flags: u16,
    pub(super) phase: String,
    pub(super) outcome: String,
    pub(super) straight_authorized: Option<bool>,
    pub(super) path_authorized: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum TraceTargetLifecyclePayload {
    ActivateSword,
    PlayAnimFreeze {
        animation_id: Option<u32>,
    },
    SendMessage {
        message: Option<u32>,
        argument: Option<i32>,
        argument_raw: Option<u32>,
        extended_argument: Option<i32>,
        extended_argument_raw: Option<u32>,
    },
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceTargetLifecycleEvent {
    pub(super) ordinal: u64,
    #[serde(default)]
    pub(super) frame_ordinal: Option<u64>,
    pub(super) phase: String,
    pub(super) sequence_id: Option<u32>,
    pub(super) sequence_element_id: u32,
    pub(super) command_level: u16,
    pub(super) state: u32,
    pub(super) command: u16,
    pub(super) command_name: String,
    pub(super) owner: Option<TraceEntityId>,
    pub(super) context: Option<TraceEntityId>,
    pub(super) antagonist: Option<TraceEntityId>,
    #[serde(default)]
    pub(super) antagonist_observed: Option<bool>,
    pub(super) payload: TraceTargetLifecyclePayload,
    #[serde(default)]
    pub(super) payload_observed: Option<bool>,
    pub(super) script_enabled: Option<bool>,
    pub(super) class_instantiated: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceStrikeProposalEvent {
    pub(super) invocation: u32,
    pub(super) ordinal: u64,
    pub(super) frame_ordinal: u64,
    pub(super) phase: String,
    pub(super) actor: Option<TraceEntityId>,
    pub(super) actor_creation_order: u32,
    pub(super) threat: Option<TraceEntityId>,
    pub(super) threat_creation_order: Option<u32>,
    pub(super) principal_opponent: Option<TraceEntityId>,
    pub(super) principal_opponent_creation_order: Option<u32>,
    pub(super) command: Option<u16>,
    pub(super) command_name: Option<String>,
    pub(super) also_parade: Option<bool>,
    pub(super) only_parade: Option<bool>,
    pub(super) fighting_ability: Option<u16>,
    pub(super) blood_alcohol: Option<u16>,
    pub(super) special_gate_fighting_ability: Option<u16>,
    pub(super) parade_gate_fighting_ability: Option<u16>,
    pub(super) first_random_raw: Option<u32>,
    pub(super) first_random_modulo: Option<u32>,
    pub(super) second_random_raw: Option<u32>,
    pub(super) second_random_modulo: Option<u32>,
    pub(super) reason: Option<String>,
    pub(super) opponent_animation: Option<u32>,
    pub(super) opponent_strike: Option<i32>,
    pub(super) candidate_strike: Option<i32>,
    pub(super) time_limit: Option<i32>,
    pub(super) minimum_skill: Option<u16>,
    pub(super) maximum_alcohol: Option<u16>,
    pub(super) boredom_before: Option<u16>,
    pub(super) boredom_after_decay: Option<u16>,
    pub(super) skill_eligible: Option<bool>,
    pub(super) alcohol_eligible: Option<bool>,
    pub(super) time_eligible: Option<bool>,
    pub(super) raw_damage: Option<i32>,
    pub(super) victim_count: Option<i32>,
    pub(super) boredom_penalty: Option<i32>,
    pub(super) drunken_bonus: Option<i32>,
    pub(super) adjusted_damage: Option<i32>,
    pub(super) group_strike: Option<bool>,
    pub(super) group_condition: Option<bool>,
    pub(super) accepted_as_best: Option<bool>,
    pub(super) selected_strike: Option<i32>,
    pub(super) parry_transition_frames: Option<i32>,
    pub(super) parry_time_eligible: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceSequenceLifecycleEvent {
    pub(super) ordinal: u64,
    pub(super) frame_ordinal: u64,
    pub(super) event: String,
    pub(super) phase: String,
    pub(super) element_id: u32,
    pub(super) sequence_id: Option<u32>,
    pub(super) owner: Option<TraceEntityId>,
    pub(super) owner_creation_order: Option<u32>,
    pub(super) command: u16,
    pub(super) command_name: Option<String>,
    pub(super) command_level: u16,
    pub(super) state: Option<u32>,
    pub(super) priority: Option<u32>,
    pub(super) queue_size_before: Option<u32>,
    pub(super) queue_size_after: Option<u32>,
    pub(super) actor: Option<TraceEntityId>,
    pub(super) actor_creation_order: Option<u32>,
    pub(super) selected_sequence_id: Option<u32>,
    pub(super) selected_command: Option<u16>,
    pub(super) current_order_id: Option<u32>,
    pub(super) current_order_action: Option<u32>,
    pub(super) decision: Option<i32>,
    pub(super) accepted: Option<bool>,
}

/// Frame layout embedded in version-68 native records.
///
/// ON-DISK FORMAT INVARIANT: any bitcode-shape change requires a native
/// version bump plus a frozen compatibility decoder for this layout.
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(super) struct TraceFrame {
    #[serde(rename = "type")]
    pub(super) record_type: String,
    pub(super) frame_before: u64,
    pub(super) frame_after: u64,
    pub(super) game_code: i32,
    pub(super) simulation_body_ran: bool,
    pub(super) commands: Vec<TraceCommand>,
    pub(super) director_completions: Vec<TraceDirectorCompletion>,
    pub(super) selected_pcs: Vec<TraceEntityId>,
    pub(super) elements: Vec<TraceElement>,
    pub(super) visibility_queries: Vec<TraceVisibilityQuery>,
    pub(super) rng_draws: TraceRngBatch,
    pub(super) motion_line_changes: Vec<TraceMotionLineChange>,
    pub(super) path_events: Vec<TracePathEvent>,
    /// Retained and printed on divergence; Rust has no
    /// side-effect-free route-construction event capture yet.
    /// TODO(parity-schema): compare once the sequence builders publish the
    /// same source/goal and ordered gate list.
    pub(super) route_construction_events: Vec<TraceRouteConstructionEvent>,
    pub(super) popup_events: Vec<TracePopupEvent>,
    pub(super) ai_forecast_events: Vec<TraceAiForecastEvent>,
    pub(super) alert_formation_events: Vec<TraceAlertFormationEvent>,
    pub(super) goto_authorization_events: Vec<TraceGoToAuthorizationEvent>,
    pub(super) strike_proposal_events: Vec<TraceStrikeProposalEvent>,
    pub(super) sequence_lifecycle_events: Vec<TraceSequenceLifecycleEvent>,
    pub(super) target_lifecycle_events: Vec<TraceTargetLifecycleEvent>,
    pub(super) resolved_exclamations: Vec<TraceResolvedExclamation>,
    /// Optional diagnostics in early schema-16 recordings. They are retained
    /// whenever present and default only at the JSON-to-native compatibility
    /// boundary; logical state comparison never invents recorded operands.
    #[serde(default)]
    pub(super) movement_steps: Vec<TraceMovementStep>,
    #[serde(default)]
    pub(super) flight_steps: Vec<TraceFlightStep>,
}

/// Recursive JSON tree used for high-volume trace snapshots.
///
/// `serde_json::Value` deliberately has no native binary-codec derives. This
/// equivalent tree keeps frame parsing strict; it is the serde
/// (JSONL) view of [`TraceJsonValue`], which stores the same data flat.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub(super) enum TraceJsonTree {
    Null(()),
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    String(String),
    Array(Vec<TraceJsonTree>),
    Object(BTreeMap<String, TraceJsonTree>),
}

impl TraceJsonTree {
    pub(super) fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null(()) => serde_json::Value::Null,
            Self::Bool(value) => (*value).into(),
            Self::Unsigned(value) => (*value).into(),
            Self::Signed(value) => (*value).into(),
            Self::Float(value) => (*value).into(),
            Self::String(value) => value.clone().into(),
            Self::Array(values) => values.iter().map(Self::to_json).collect(),
            Self::Object(values) => values
                .iter()
                .map(|(key, value)| (key.clone(), value.to_json()))
                .collect(),
        }
    }

    pub(super) fn flatten_into(&self, tokens: &mut Vec<TraceJsonToken>) {
        match self {
            Self::Null(()) => tokens.push(TraceJsonToken::Null),
            Self::Bool(value) => tokens.push(TraceJsonToken::Bool(*value)),
            Self::Unsigned(value) => tokens.push(TraceJsonToken::Unsigned(*value)),
            Self::Signed(value) => tokens.push(TraceJsonToken::Signed(*value)),
            Self::Float(value) => tokens.push(TraceJsonToken::Float(*value)),
            Self::String(value) => tokens.push(TraceJsonToken::String(value.clone())),
            Self::Array(values) => {
                tokens.push(TraceJsonToken::Array(
                    u32::try_from(values.len()).expect("JSON array length exceeds u32"),
                ));
                for value in values {
                    value.flatten_into(tokens);
                }
            }
            Self::Object(values) => {
                tokens.push(TraceJsonToken::Object(
                    u32::try_from(values.len()).expect("JSON object length exceeds u32"),
                ));
                for (key, value) in values {
                    tokens.push(TraceJsonToken::Key(key.clone()));
                    value.flatten_into(tokens);
                }
            }
        }
    }

    pub(super) fn unflatten(tokens: &mut std::slice::Iter<'_, TraceJsonToken>) -> Self {
        match tokens.next().expect("flat JSON token stream ended early") {
            TraceJsonToken::Null => Self::Null(()),
            TraceJsonToken::Bool(value) => Self::Bool(*value),
            TraceJsonToken::Unsigned(value) => Self::Unsigned(*value),
            TraceJsonToken::Signed(value) => Self::Signed(*value),
            TraceJsonToken::Float(value) => Self::Float(*value),
            TraceJsonToken::String(value) => Self::String(value.clone()),
            TraceJsonToken::Array(len) => {
                Self::Array((0..*len).map(|_| Self::unflatten(tokens)).collect())
            }
            TraceJsonToken::Object(len) => Self::Object(
                (0..*len)
                    .map(|_| {
                        let TraceJsonToken::Key(key) = tokens
                            .next()
                            .expect("flat JSON object ended before its key")
                        else {
                            panic!("flat JSON object entry does not start with a key")
                        };
                        (key.clone(), Self::unflatten(tokens))
                    })
                    .collect(),
            ),
            TraceJsonToken::Key(key) => {
                panic!("unexpected flat JSON key {key:?} in value position")
            }
        }
    }
}

/// One pre-order token of a flattened [`TraceJsonTree`]. `Array`/`Object`
/// carry their child count; object entries are `Key` followed by a value.
#[derive(Clone, Debug, PartialEq, bitcode::Decode, bitcode::Encode)]
pub(super) enum TraceJsonToken {
    Null,
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    String(String),
    Array(u32),
    Object(u32),
    Key(String),
}

/// Cache-safe JSON value: a [`TraceJsonTree`] stored as a flat pre-order
/// token list. bitcode's derives cannot encode recursive types (the derived
/// encoder would be infinitely sized), so the binary codecs see a plain
/// `Vec<TraceJsonToken>` while serde still reads and writes the JSON shape.
#[derive(Clone, Debug, PartialEq, bitcode::Decode, bitcode::Encode)]
pub(super) struct TraceJsonValue {
    pub(super) tokens: Vec<TraceJsonToken>,
}

impl TraceJsonValue {
    pub(super) fn tree(&self) -> TraceJsonTree {
        let mut tokens = self.tokens.iter();
        let tree = TraceJsonTree::unflatten(&mut tokens);
        assert!(
            tokens.next().is_none(),
            "flat JSON token stream has trailing tokens"
        );
        tree
    }

    pub(super) fn to_json(&self) -> serde_json::Value {
        self.tree().to_json()
    }
}

impl From<TraceJsonTree> for TraceJsonValue {
    fn from(tree: TraceJsonTree) -> Self {
        let mut tokens = Vec::new();
        tree.flatten_into(&mut tokens);
        Self { tokens }
    }
}

pub(super) fn missing_legacy_trace_json_value() -> TraceJsonValue {
    TraceJsonTree::Null(()).into()
}

impl Serialize for TraceJsonValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.tree().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TraceJsonValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        TraceJsonTree::deserialize(deserializer).map(Self::from)
    }
}

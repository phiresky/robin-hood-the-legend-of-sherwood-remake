//! Binary level loader for `.rhp` (proto-level) and `.rhm` (mission) files.
//!
//! The game's level data uses a chunk-based binary format:
//! - Each chunk: `[4-byte tag][u32 size (LE)][u32 version (LE)][payload]`
//! - The size field includes the version field (`payload_len = size - 4`)
//! - Chunks can be nested (tracked via a stack)
//!
//! Two file format variants exist (detected at runtime):
//! - **Demo** (`ENABLE_DEMO`): tags like `RHPL`/`RHMI`, lower version numbers
//! - **Fullgame** (`_NEW_LEVELS`): obfuscated tags like `MEUH`/`DUTY`, higher versions

use crate::coordinates::MapPoint;
use crate::sbfile::{SB_FILE_READ, SbFile};
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════
//  Errors
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, thiserror::Error)]
pub enum LevelError {
    #[error("chunk tag mismatch: expected '{expected}', found '{found}'")]
    ChunkTagMismatch { expected: String, found: String },

    #[error("chunk version mismatch in '{tag}': expected {expected}, found {found}")]
    ChunkVersionMismatch {
        tag: String,
        expected: u32,
        found: u32,
    },

    #[error("chunk size mismatch in '{tag}': expected {expected} bytes, read {consumed}")]
    ChunkSizeMismatch {
        tag: String,
        expected: u32,
        consumed: u32,
    },

    #[error("unknown element sub-chunk: '{0}'")]
    UnknownElementChunk(String),

    #[error("file read error (sbfile code {0})")]
    ReadError(i32),

    #[error("file not found: {0}")]
    FileNotFound(String),

    #[error("unknown level format: file header tag '{0}'")]
    UnknownFormat(String),

    #[error("unsupported mobile-element data: {0}")]
    UnsupportedMobileData(String),
}

impl From<i32> for LevelError {
    fn from(code: i32) -> Self {
        LevelError::ReadError(code)
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Level format detection & tag/version tables
// ═══════════════════════════════════════════════════════════════════

/// Level file format variant.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum LevelFormat {
    /// Demo / original release (no `_NEW_LEVELS`).
    Demo,
    /// Fullgame / retail format (`_NEW_LEVELS` defined).
    Fullgame,
}

impl LevelFormat {
    /// Detect format from the first 4 bytes of a proto-level or mission file.
    pub fn detect(tag: &[u8; 4]) -> Result<Self, LevelError> {
        match tag {
            b"RHPL" | b"RHMI" => Ok(Self::Demo),
            b"MEUH" | b"DUTY" => Ok(Self::Fullgame),
            _ => Err(LevelError::UnknownFormat(tag_str(tag))),
        }
    }

    pub fn is_fullgame(self) -> bool {
        matches!(self, Self::Fullgame)
    }

    // ── File-level headers ──────────────────────────────────────
    pub fn proto_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"RHPL",
            Self::Fullgame => b"MEUH",
        }
    }
    pub fn mission_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"RHMI",
            Self::Fullgame => b"DUTY",
        }
    }
    pub fn file_version(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }

    // ── Mission chunk tags & versions ───────────────────────────
    pub fn header_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"HEAD",
            Self::Fullgame => b"FOOT",
        }
    }
    pub fn header_ver(self) -> u32 {
        match self {
            Self::Demo => 3,
            Self::Fullgame => 4,
        }
    }
    pub fn element_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"ELEM",
            Self::Fullgame => b"BOYZ",
        }
    }
    pub fn element_ver(self) -> u32 {
        match self {
            Self::Demo => 2,
            Self::Fullgame => 3,
        }
    }
    pub fn civilian_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"CIVI",
            Self::Fullgame => b"OILE",
        }
    }
    pub fn civilian_ver(self) -> u32 {
        match self {
            Self::Demo => 2,
            Self::Fullgame => 3,
        }
    }
    pub fn soldier_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"EVIL",
            Self::Fullgame => b"BORG",
        }
    }
    pub fn soldier_ver(self) -> u32 {
        match self {
            Self::Demo => 3,
            Self::Fullgame => 4,
        }
    }
    pub fn beamme_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"GOOD",
            Self::Fullgame => b"SCOT",
        }
    }
    pub fn beamme_ver(self) -> u32 {
        match self {
            Self::Demo => 3,
            Self::Fullgame => 4,
        }
    }
    pub fn target_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"TGET",
            Self::Fullgame => b"BOOM",
        }
    }
    pub fn target_ver(self) -> u32 {
        match self {
            Self::Demo => 4,
            Self::Fullgame => 5,
        }
    }
    pub fn pc_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"PRIS",
            Self::Fullgame => b"TOTO",
        }
    }
    pub fn pc_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn bonus_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"BONU",
            Self::Fullgame => b"ZORG",
        }
    }
    pub fn bonus_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn scroll_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"PARC",
            Self::Fullgame => b"SKRO",
        }
    }
    pub fn scroll_ver(self) -> u32 {
        match self {
            Self::Demo => 3,
            Self::Fullgame => 4,
        }
    }
    pub fn animal_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"BETE",
            Self::Fullgame => b"MEOW",
        }
    }

    // ── Proto-level chunk tags & versions ─────────────────────────
    pub fn misc_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"MISC",
            Self::Fullgame => b"SPOK",
        }
    }
    pub fn misc_ver(self) -> u32 {
        match self {
            Self::Demo => 2,
            Self::Fullgame => 3,
        }
    }
    pub fn patch_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"PAT ",
            Self::Fullgame => b"TUPO",
        }
    }
    pub fn patch_ver(self) -> u32 {
        match self {
            Self::Demo => 2,
            Self::Fullgame => 3,
        }
    }
    pub fn patch2_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"PAT ",
            Self::Fullgame => b"POUF",
        }
    }
    pub fn animation_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"ANIM",
            Self::Fullgame => b"FLIM",
        }
    }
    pub fn animation_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn mask_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"MASK",
            Self::Fullgame => b"FACE",
        }
    }
    pub fn mask_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn motion_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"MOVE",
            Self::Fullgame => b"STAT",
        }
    }
    pub fn motion_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn sight_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"SGHT",
            Self::Fullgame => b"WOAW",
        }
    }
    pub fn sight_ver(self) -> u32 {
        match self {
            Self::Demo => 2,
            Self::Fullgame => 3,
        }
    }
    pub fn bond_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"BOND",
            Self::Fullgame => b"007 ",
        }
    }
    pub fn bond_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn sound_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"SND ",
            Self::Fullgame => b"LOUD",
        }
    }
    pub fn sound_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn material_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"MAT ",
            Self::Fullgame => b"TEXT",
        }
    }
    pub fn material_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn lift_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"LIFT",
            Self::Fullgame => b" AZ ",
        }
    }
    pub fn lift_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn building_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"BUIL",
            Self::Fullgame => b"FARM",
        }
    }
    pub fn building_ver(self) -> u32 {
        match self {
            Self::Demo => 3,
            Self::Fullgame => 4,
        }
    }
    pub fn light_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"LZ  ",
            Self::Fullgame => b"DARK",
        }
    }
    pub fn light_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn jump_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"JZ  ",
            Self::Fullgame => b"PPPP",
        }
    }
    pub fn jump_ver(self) -> u32 {
        match self {
            Self::Demo => 3,
            Self::Fullgame => 4,
        }
    }
    pub fn background_tag(self) -> &'static [u8; 4] {
        b"BGND"
    }

    // ── Mission-only chunk tags & versions ──────────────────────────
    pub fn animal_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn tactic_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"AI  ",
            Self::Fullgame => b"HIRN",
        }
    }
    pub fn tactic_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn mobile_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"CART",
            Self::Fullgame => b"TING",
        }
    }
    pub fn mobile_ver(self) -> u32 {
        match self {
            Self::Demo => 2,
            Self::Fullgame => 3,
        }
    }
    pub fn script_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"SCRP",
            Self::Fullgame => b"GULP",
        }
    }
    pub fn script_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn tenant_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"GUYS",
            Self::Fullgame => b"CAVE",
        }
    }
    pub fn tenant_ver(self) -> u32 {
        match self {
            Self::Demo => 2,
            Self::Fullgame => 3,
        }
    }
    pub fn path_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"PWAY",
            Self::Fullgame => b"RAIL",
        }
    }
    pub fn path_ver(self) -> u32 {
        match self {
            Self::Demo => 2,
            Self::Fullgame => 3,
        }
    }

    // ── AI tactic sub-chunk tags & versions ───────────────────────────
    pub fn reinforcement_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"REIN",
            Self::Fullgame => b"POW ",
        }
    }
    pub fn reinforcement_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn ambush_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"AMBU",
            Self::Fullgame => b"BUSH",
        }
    }
    pub fn ambush_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn search_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"SEAR",
            Self::Fullgame => b"HOLE",
        }
    }
    pub fn search_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
    pub fn archery_tag(self) -> &'static [u8; 4] {
        match self {
            Self::Demo => b"ARCH",
            Self::Fullgame => b"NLIP",
        }
    }
    pub fn archery_ver(self) -> u32 {
        match self {
            Self::Demo => 1,
            Self::Fullgame => 2,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Chunk reader — binary navigation over an SbFile
// ═══════════════════════════════════════════════════════════════════

/// Tracks one open chunk's position and expected size.
#[derive(Debug)]
struct ChunkInfo {
    /// File position immediately after reading the 12-byte header (tag+size+version).
    position_after_header: u64,
    /// Size field from the chunk header (includes version + payload).
    expected_size: u32,
    /// Tag (for error messages).
    tag: [u8; 4],
}

/// Chunk-aware binary reader wrapping [`SbFile`].
///
/// Provides chunk-stack navigation: open/close nested chunks while
/// validating tag + size + version of each one.
pub struct ChunkReader {
    file: SbFile,
    chunk_stack: Vec<ChunkInfo>,
}

impl ChunkReader {
    pub fn new(file: SbFile) -> Self {
        Self {
            file,
            chunk_stack: Vec::new(),
        }
    }

    /// Open a chunk: read and validate tag + size + version, push onto stack.
    pub fn chunk_start(
        &mut self,
        expected_tag: &[u8; 4],
        expected_version: u32,
    ) -> Result<(), LevelError> {
        let mut tag = [0u8; 4];
        self.file.serialize_bytes(&mut tag)?;

        let mut size = 0u32;
        self.file.serialize_u32(&mut size)?;

        let mut version = 0u32;
        self.file.serialize_u32(&mut version)?;

        if tag != *expected_tag {
            return Err(LevelError::ChunkTagMismatch {
                expected: tag_str(expected_tag),
                found: tag_str(&tag),
            });
        }

        if version != expected_version {
            return Err(LevelError::ChunkVersionMismatch {
                tag: tag_str(&tag),
                expected: expected_version,
                found: version,
            });
        }

        let pos = self.file.tell();
        self.chunk_stack.push(ChunkInfo {
            position_after_header: pos,
            expected_size: size,
            tag,
        });

        Ok(())
    }

    /// Close the current chunk: validate consumed size, pop from stack.
    pub fn chunk_end(&mut self) -> Result<(), LevelError> {
        let info = self
            .chunk_stack
            .pop()
            .expect("chunk_end called without matching chunk_start");

        let current_pos = self.file.tell();
        // Real size = bytes read since header + sizeof(version u32).
        let consumed = (current_pos - info.position_after_header) as u32 + 4;

        if consumed != info.expected_size {
            return Err(LevelError::ChunkSizeMismatch {
                tag: tag_str(&info.tag),
                expected: info.expected_size,
                consumed,
            });
        }
        Ok(())
    }

    /// Peek at the next chunk's 4-byte tag without consuming it.
    pub fn peek_next_chunk(&mut self) -> Result<[u8; 4], LevelError> {
        let mut tag = [0u8; 4];
        self.file.serialize_bytes(&mut tag)?;
        self.file.skip(-4, 1); // seek back to before the tag
        Ok(tag)
    }

    /// Skip over an entire chunk (tag + size + data).
    ///
    /// File pointer must be at the start of the tag (e.g. after `peek_next_chunk`).
    pub fn skip_chunk(&mut self) -> Result<(), LevelError> {
        self.file.skip(4, 1); // skip past the 4-byte tag
        let mut size = 0u32;
        self.file.serialize_u32(&mut size)?; // read the size field
        self.file.skip(size as i64, 1); // skip version + payload
        Ok(())
    }

    /// Returns true if we've consumed all data in the current (topmost) chunk.
    pub fn at_end_of_chunk(&mut self) -> bool {
        if let Some(info) = self.chunk_stack.last() {
            let current_pos = self.file.tell();
            // End of payload = position_after_header + (expected_size - 4)
            let end_pos = info.position_after_header + (info.expected_size as u64) - 4;
            current_pos >= end_pos
        } else {
            self.file.tell() >= self.file.get_size()
        }
    }

    /// Returns the number of bytes remaining in the current chunk payload.
    pub fn remaining_in_chunk(&mut self) -> usize {
        if let Some(info) = self.chunk_stack.last() {
            let current_pos = self.file.tell();
            let end_pos = info.position_after_header + (info.expected_size as u64) - 4;
            end_pos.saturating_sub(current_pos) as usize
        } else {
            0
        }
    }

    // ── Typed binary readers ───────────────────────────────────

    pub fn read_u8(&mut self) -> Result<u8, LevelError> {
        let mut v = 0u8;
        self.file.serialize_u8(&mut v)?;
        Ok(v)
    }

    pub fn read_i16(&mut self) -> Result<i16, LevelError> {
        let mut v = 0i16;
        self.file.serialize_i16(&mut v)?;
        Ok(v)
    }

    pub fn read_u16(&mut self) -> Result<u16, LevelError> {
        let mut v = 0u16;
        self.file.serialize_u16(&mut v)?;
        Ok(v)
    }

    pub fn read_u32(&mut self) -> Result<u32, LevelError> {
        let mut v = 0u32;
        self.file.serialize_u32(&mut v)?;
        Ok(v)
    }

    pub fn read_bool(&mut self) -> Result<bool, LevelError> {
        let mut v = false;
        self.file.serialize_bool(&mut v)?;
        Ok(v)
    }

    pub fn read_i32(&mut self) -> Result<i32, LevelError> {
        let mut v = 0i32;
        self.file.serialize_i32(&mut v)?;
        Ok(v)
    }

    pub fn read_f32(&mut self) -> Result<f32, LevelError> {
        let mut v = 0f32;
        self.file.serialize_f32(&mut v)?;
        Ok(v)
    }

    pub fn read_string(&mut self) -> Result<String, LevelError> {
        let mut s = String::new();
        self.file.serialize_string(&mut s)?;
        Ok(s)
    }

    pub fn read_bytes(&mut self, count: usize) -> Result<Vec<u8>, LevelError> {
        let mut buf = vec![0u8; count];
        self.file.serialize_bytes(&mut buf)?;
        Ok(buf)
    }

    /// Read a padding byte (fullgame format only). No-op for demo format.
    pub fn read_padding_if_fullgame(&mut self, format: LevelFormat) -> Result<(), LevelError> {
        if format.is_fullgame() {
            let _dummy = self.read_u8()?;
        }
        Ok(())
    }
}

/// Convert a 4-byte tag to a printable string.
fn tag_str(tag: &[u8; 4]) -> String {
    String::from_utf8_lossy(tag).into_owned()
}

// ═══════════════════════════════════════════════════════════════════
//  Parsed data structures
// ═══════════════════════════════════════════════════════════════════

/// Mission header from the HEAD/FOOT chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionHeader {
    pub control_crc: u32,
    pub ambiance: u32,
    pub map_filename: String,
    pub mission_profile_id: u32,
}

/// Beam-me spawn point from the GOOD/SCOT chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct BeamMe {
    pub position: MapPoint,
    pub direction: u32,
    pub action: u32,
    pub projection_area: u16,
    pub sector: u16,
    pub layer: u16,
    pub material: u32,
    pub action_required: BeamMeActions,
    pub index: u16,
    pub script: Option<String>,
    pub required_pc: u8,
}

/// Action requirement flags for beam-me spawn points.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct BeamMeActions {
    pub climb: bool,
    pub jump: bool,
    pub lockpick: bool,
    pub archery: bool,
    pub carry: bool,
    pub tie: bool,
    pub stun: bool,
    pub lever: bool,
    pub eat: bool,
    pub search: bool,
}

/// Raw soldier data from the EVIL/BORG sub-chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawSoldier {
    pub position_x: u16,
    pub position_y: u16,
    pub direction: u32,
    pub action: u32,
    pub obstacle_index: u16,
    pub sector: u16,
    pub layer: u16,
    pub material: u32,
    pub profile_number: u32,
    /// Readable hackable-level identifier; legacy missions use the numeric
    /// profile number above.
    #[serde(default)]
    pub profile_id: Option<String>,
    /// Optional Rust-port extension. Legacy RHM files leave this unset and
    /// derive allegiance from `SoldierProfile::hostile` as before.
    #[serde(default)]
    pub allegiance: Option<u16>,
    /// Custom missions may bypass the normal town-map fog silhouette.
    #[serde(default)]
    pub revealed: bool,
    pub tower_guard: bool,
    pub company_number: u32,
    pub drunk_level: u32,
    pub money: u32,
    pub subordinate_ids: Vec<u16>,
    pub path_id: u16,
    pub alert_path_id: u16,
    /// Script class name (if the soldier is script-bound).
    pub script_class: Option<String>,
}

/// Raw civilian data from the CIVI/OILE sub-chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawCivilian {
    pub position_x: u16,
    pub position_y: u16,
    pub direction: u32,
    pub action: u32,
    pub obstacle_index: u16,
    pub sector: u16,
    pub layer: u16,
    pub material: u32,
    pub profile_number: u32,
    /// Optional Rust-port extension. Legacy RHM files leave this unset and
    /// derive allegiance from the civilian profile attitude as before.
    #[serde(default)]
    pub allegiance: Option<u16>,
    pub path_id: u16,
    pub money: u32,
    /// Scroll sets for beggars (10 sets, each a list of scroll IDs).
    /// `None` for non-beggar civilians.
    pub beggar_scroll_sets: Option<Vec<Vec<u16>>>,
    /// Script class name (if the civilian is script-bound).
    pub script_class: Option<String>,
}

/// Raw target data from the TGET/BOOM sub-chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawTarget {
    pub position_x: i16,
    pub position_y: i16,
    pub position_z: i16,
    pub direction: u32,
    pub action: u32,
    pub obstacle_index: u16,
    pub sector: u16,
    pub layer: u16,
    pub filename: String,
    pub profile_name: String,
    pub action_filter: u32,
    pub action_position_x: i16,
    pub action_position_y: i16,
    pub action_sector: u16,
    pub action_layer: u16,
    pub polyline: Vec<(i16, i16)>,
    pub blit_type: u8,
    pub script_class: Option<String>,
}

/// Raw bonus data from the BONU/ZORG chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawBonus {
    pub bonus_type: u16,
    pub quantity: u16,
    pub position_x: u16,
    pub position_y: u16,
    pub direction: u32,
    pub action: u32,
    pub obstacle_index: u16,
    pub sector: u16,
    pub layer: u16,
}

/// Raw PC-to-rescue data from the PRIS/TOTO sub-chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawPcRescue {
    pub position_x: i16,
    pub position_y: i16,
    pub direction: u32,
    pub action: u32,
    pub obstacle_index: u16,
    pub sector: u16,
    pub layer: u16,
    pub material: u32,
    pub profile_index: u32,
    /// Optional Rust-port extension for custom multi-team missions.
    #[serde(default)]
    pub allegiance: Option<u16>,
    #[serde(default)]
    pub autonomous: bool,
    /// Skip the initial offensive hesitation roll while retaining all normal
    /// strike legality, skill, timing, damage, tiredness, and defence rules.
    #[serde(default)]
    pub aggressive_combat: bool,
    /// Makes a custom-mission PC immediately player-controllable instead of
    /// treating it as a normal rescue target.
    #[serde(default)]
    pub playable: bool,
    pub attributes: u32,
    pub script_class: Option<String>,
}

/// Raw scroll data from the PARC/SKRO chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawScroll {
    pub position_x: u16,
    pub position_y: u16,
    pub direction: u32,
    pub action: u32,
    pub obstacle_index: u16,
    pub sector: u16,
    pub layer: u16,
    /// Presence per difficulty level (Easy/Medium/Hard).
    pub presence: [bool; 3],
    pub tutorial: bool,
    pub force_visible: bool,
    pub script_class: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
//  Tactic (AI) data structures
// ═══════════════════════════════════════════════════════════════════

/// Reinforcement door spawn point from the REIN/POW sub-chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawReinforcementPoint {
    pub x: i16,
    pub y: i16,
    pub direction: u32,
    pub action: u32,
    pub obstacle_index: u16,
    pub sector: u16,
    pub layer: u16,
}

/// Ambush trigger point from the AMBU/BUSH sub-chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawAmbushPoint {
    pub x: i16,
    pub y: i16,
    pub sector: u16,
    pub level: u16,
}

/// Seek/search point from the SEAR/HOLE sub-chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawSeekPoint {
    pub x: i16,
    pub y: i16,
    pub sector: u16,
    pub level: u16,
    pub direction: u16,
}

/// Archery point within an archery sector, from the ARCH/NLIP sub-chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawArcheryPoint {
    pub x: u16,
    pub y: u16,
    pub sector: u16,
    pub is_shooting_point: bool,
    pub direction: u16,
}

/// Archery sector (a path with shooting positions) from the ARCH/NLIP sub-chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawArcherySector {
    pub sector_ref: u16,
    pub polygon: SectorPolygon,
    pub points: Vec<RawArcheryPoint>,
}

/// All tactic data from the TACTIC (AI /HIRN) chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawTacticData {
    pub reinforcement_points: Vec<RawReinforcementPoint>,
    pub ambush_points: Vec<RawAmbushPoint>,
    pub seek_points: Vec<RawSeekPoint>,
    pub archery_sectors: Vec<RawArcherySector>,
}

// ═══════════════════════════════════════════════════════════════════
//  Proto-level data structures
// ═══════════════════════════════════════════════════════════════════

/// A polygon read from a sector — just a list of 2D points (the sector
/// boundary).
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SectorPolygon {
    pub points: Vec<(i16, i16)>,
}

/// A motion obstacle within a motion area.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawMotionObstacle {
    pub state_id: u32,
    pub polygon: SectorPolygon,
}

/// A motion area (walkable polygon + skeleton + obstacles).
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawMotionArea {
    pub is_lift: bool,
    pub state_id: u32,
    pub polygon: SectorPolygon,
    /// Skeleton segments for fast line-of-sight checks.
    pub skeleton_segments: Vec<((i16, i16), (i16, i16))>,
    pub flags: u32,
    pub obstacles: Vec<RawMotionObstacle>,
}

/// Motion data loaded from the MOTION chunk.
/// Contains the raw motion obstacle geometry and the pathfinder graph bytes.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawMotionData {
    /// Motion areas per layer: `layers[layer][area]`.
    pub layers: Vec<Vec<RawMotionArea>>,
    /// Raw bytes of the pathfinder graph (parsed later by `PathGraph::load_from_proto_stream`).
    pub graph_bytes: Vec<u8>,
}

/// Sprite identification for an animation-kind frame entry.
///
/// Stores just the names needed to look up the sprite — the actual frame
/// data is loaded later via the sprite scriptor.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawSpriteRef {
    /// Sprite file name (e.g. "trap01.rhs").
    pub frame_profile_name: String,
    /// Profile name within the file.
    pub profile_name: String,
    /// Position X.
    pub position_x: i16,
    /// Position Y.
    pub position_y: i16,
    /// Elevation (Z height).
    pub elevation: u16,
}

/// Data for a single FX element loaded from the proto-level stream.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawElementFx {
    pub sprite: RawSpriteRef,
    /// Shadow/rendering mode: 0 = blocky, non-zero = needs shadow.
    pub blit_type: u8,
    pub active: bool,
    pub force_display: bool,
    /// Display masking polyline (screen-space).
    pub display_polyline: Vec<(i16, i16)>,
}

/// Reference to a mask in the grid by layer + index.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MaskRef {
    pub layer: u16,
    pub index: u16,
}

/// A single patch (interactive terrain area) loaded from the proto-level.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawPatch {
    pub element_fx: RawElementFx,
    pub active: bool,
    /// Pathfinder changing obstacles value (0 = none).
    /// If non-zero, raw value is `(original - 1) >> 1`.
    pub pathfinder_changing_obstacles: u32,
    pub pathfinder_sector: Option<u16>,
    pub pathfinder_layer: Option<u16>,
    pub start_animation_valid: bool,
    pub transition_animation_valid: bool,
    pub end_animation_valid: bool,
    pub waypoint: (i16, i16),
    pub sector: u16,
    pub layer: u16,
    /// Definitive flag (inverted during load: `!value`).
    pub definitive: bool,
    pub integrate_in_background: bool,
    /// Old state mask references (layer + index pairs).
    pub old_masks: Vec<MaskRef>,
    /// Old state sight obstacle indices.
    pub old_sight_obstacles: Vec<u16>,
    /// Old clickable sector polygon.
    pub old_mouse_sector: SectorPolygon,
    /// Old masking mouse sector polygon.
    pub old_masking_sector: SectorPolygon,
    /// New state mask references.
    pub new_masks: Vec<MaskRef>,
    /// New state sight obstacle indices.
    pub new_sight_obstacles: Vec<u16>,
    /// New clickable sector polygon.
    pub new_mouse_sector: SectorPolygon,
    /// New masking mouse sector polygon.
    pub new_masking_sector: SectorPolygon,
    /// Apply sector polygon (trigger zone).
    pub apply_sector: SectorPolygon,
    /// No-apply sector polygon.
    pub no_apply_sector: SectorPolygon,
    pub door_triggered: bool,
    pub triggers_door: bool,
    /// Door indices (into the gates array).
    pub door_indices: Vec<u16>,
    /// Final layer value (read at end of patch).
    pub final_layer: u16,
}

/// CHUNK_MISC data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ProtoMisc {
    pub control_crc: u32,
    pub forest_level: bool,
    pub default_material: u32,
}

// ── Proto-level chunk structs ────────────────────────────────────

/// Material sector from the MAT/TEXT chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawMaterialSector {
    pub material: u8,
    pub polygon: SectorPolygon,
}

/// Light/shadow sector from the LZ/DARK chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawLightSector {
    pub layer: u16,
    pub polygon: SectorPolygon,
    pub ambience: u32,
}

/// Elevation line from the BOND/007 chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawElevationLine {
    pub point_a: (i16, i16),
    pub point_b: (i16, i16),
    pub right_obstacle_index: u16,
    pub left_obstacle_index: u16,
    pub layer: u16,
}

/// Mask type bitmask constants.
pub const MASK_CHARACTER: u8 = 1;
pub const MASK_PROJECTILE: u8 = 2;
pub const MASK_VIEW: u8 = 4;
pub const MASK_OBSTACLE: u8 = 16;

/// Mask data from the MASK/FACE chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawMask {
    pub layer: u16,
    pub mask_type: u8,
    /// Character masking polyline (present when `mask_type & MASK_CHARACTER`).
    pub character_polyline: Option<Vec<(i16, i16)>>,
    /// Projectile masking polyline (present when `mask_type & MASK_PROJECTILE`).
    pub projectile_polyline: Option<Vec<(i16, i16)>>,
    /// Bounding box top-left corner.
    pub box_top_left: (i16, i16),
    /// Bounding box size (width, height).
    pub box_size: (i16, i16),
    /// Raw mask bitmap data.
    pub mask_data: Vec<u8>,
    /// Obstacle indices (present when `mask_type & MASK_OBSTACLE`).
    pub obstacle_indices: Vec<u16>,
}

/// 3D obstacle point used by sight obstacles.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawObstaclePoint {
    pub x: f32,
    pub y: f32,
    pub z_bottom: f32,
    pub z_top: f32,
}

/// Sight obstacle from the SGHT/WOAW chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawSightObstacle {
    pub points: Vec<RawObstaclePoint>,
    /// Projection area (sector, layer) if this is a projection area.
    pub projection_area: Option<(u16, u16)>,
    pub opaque: bool,
    pub solid: bool,
    pub mouse: bool,
    pub show_shadow_polygon: bool,
    pub default_material: u8,
    pub material_indices: Vec<u16>,
}

/// Sound source kind constant: KIND_SINGLE=0, KIND_LOOPED=1,
/// KIND_DELAYED=2, KIND_VOLATILE=3.
const SOUND_KIND_DELAYED: u8 = 2;

/// Sound source from the SND/LOUD chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawSoundSource {
    pub id: i32,
    pub active: bool,
    pub source_kind: u8,
    /// Delay parameters (min, max, stepping) when `source_kind == SOUND_KIND_DELAYED`.
    pub delayed_params: Option<(u16, u16, u16)>,
    pub global: bool,
    pub inner_distance: Option<u16>,
    pub outer_distance: Option<u16>,
    pub polyline: Option<Vec<(i16, i16)>>,
    pub inner_volume: Option<u16>,
    pub outer_volume: Option<u16>,
    pub noise_covering_distance: Option<u16>,
    pub altitude: u8,
    pub ambience_filter: u32,
}

/// Jump zone from the JZ/PPPP chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawJumpZone {
    pub polygon: SectorPolygon,
    pub sector: u16,
    pub layer: u16,
    pub helper_needed: bool,
}

/// 3D jump line from the JZ/PPPP chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawJumpLine {
    pub point_a: (i16, i16, i16),
    pub point_b: (i16, i16, i16),
    pub jump_zone_index: u16,
}

/// Jump line pair from the JZ/PPPP chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawJumpLinePair {
    pub line1: RawJumpLine,
    pub line2: RawJumpLine,
    pub jump_long: bool,
}

/// Door data (shared between BUILDING and LIFT chunks).
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawDoor {
    pub door_type: u8,
    pub active: bool,
    pub locked_pc: bool,
    pub unlockable: bool,
    pub locked_npc_villain: bool,
    pub locked_npc_civilian: bool,
    pub locked_pc_after_patch: bool,
    pub unlockable_after_patch: bool,
    pub locked_npc_villain_after_patch: bool,
    pub locked_npc_civilian_after_patch: bool,
    pub door_sector: SectorPolygon,
    pub point_out: (i16, i16),
    pub sector_out: u16,
    pub layer_out: u16,
    pub point_mid: (i16, i16),
    pub point_in: (i16, i16),
    pub sector_in: u16,
    pub layer_in: u16,
}

/// Lift data from the LIFT/AZ chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawLift {
    /// Index of the associated motion area.
    pub motion_area_index: u16,
    pub lift_type: u8,
    pub doors: Vec<RawDoor>,
    pub direction: i16,
}

/// Building entry from the BUIL/FARM chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum RawBuildingEntry {
    /// A full building sector with doors.
    Building { doors: Vec<RawDoor> },
    /// Standalone doors (not part of a building).
    StandaloneDoors { doors: Vec<RawDoor> },
}

// ── Mission-only chunk structs ───────────────────────────────────

/// Script point from the SCRP/GULP chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawScriptPoint {
    pub x: i16,
    pub y: i16,
    pub sector: u16,
    pub layer: u16,
}

/// Script sector from the SCRP/GULP chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawScriptSector {
    pub polygon: SectorPolygon,
    pub sector_ref: u16,
    pub layer: u16,
    pub script_class: Option<String>,
}

/// Script line from the SCRP/GULP chunk.
///
/// The script-line type exists in the engine but the only loader path that
/// ever produced one — the legacy old-level script-objects loader — was
/// commented out, so no shipped mission stream emits a line.  The field is
/// kept here so that `RawScriptObjects` preserves the
/// `[points][lines][sectors]` layout literally, and so
/// `script_location_count` cannot off-by-N if a future SCRIPT-chunk version
/// reintroduces lines.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawScriptLine {
    pub x1: i16,
    pub y1: i16,
    pub x2: i16,
    pub y2: i16,
    pub sector: u16,
    pub layer: u16,
}

/// Script objects loaded from the SCRP/GULP chunk.
/// Contains script points, lines, and sectors, indexed by the script system
/// via `GetLocationScript(id)` where id indexes the combined array in the
/// declared field order — `[points][lines][sectors]`.  `lines` is always
/// empty on shipped missions (see `RawScriptLine`).
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawScriptObjects {
    pub points: Vec<RawScriptPoint>,
    pub lines: Vec<RawScriptLine>,
    pub sectors: Vec<RawScriptSector>,
}

/// Building tenant data from the GUYS/CAVE chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawBuildingTenants {
    pub tenant_element_indices: Vec<u16>,
    pub arrow_reserve: bool,
}

// ── Hiking path data (PATH/PWAY/RAIL chunk) ───────────────────

/// Command attached to a waypoint — either a script class name or raw macro data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum WaypointCommand {
    None,
    Script(String),
    Macro(Vec<u8>),
}

/// A single waypoint in a hiking/patrol path.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawWaypoint {
    pub x: i16,
    pub y: i16,
    pub sector: u16,
    pub level: u16,
    pub command: WaypointCommand,
}

/// A hiking/patrol path consisting of ordered waypoints.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawHikingPath {
    pub waypoints: Vec<RawWaypoint>,
}

/// A mission mobile element. Released Robin Hood data uses these records for
/// the five horse-cart/chariot animations; the broader Spellbound mobile
/// subsystem (hookable trailers, barrels, trains, and embedded sight
/// obstacles) is not present in shipped missions.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RawMobileElement {
    pub sprites: Vec<RawElementFx>,
    pub motion_polygon: SectorPolygon,
    /// Legacy editor anchor serialized before the hook/barrel flags. The C++
    /// loader reads these values but does not use them for placement.
    pub editor_anchor: (i16, i16),
    pub hook_point: Option<(i16, i16)>,
    pub barrel_point: Option<(i16, i16)>,
    pub path_index: u16,
    pub start_waypoint: u32,
    /// Serialized by the editor but likewise unused by the C++ loader.
    pub stop_waypoint: i32,
    pub obstacle_index: u16,
}

/// Data loaded from a proto-level file (.rhp).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum ProtoElementChunk {
    Animation,
    Patch,
}

/// Source-file order of proto chunks which construct `RHSector` or
/// `RHGate` instances in Original.  The v48 save stream omits the sizes of
/// those arrays, so preserving this order is required to reconstruct their
/// exact serialization topology.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum ProtoGridChunk {
    Patch,
    Motion,
    Sight,
    /// Elevation lines share Original's per-layer `RHLine` pointer space.
    Bond,
    Material,
    Lift,
    Building,
    Light,
    Jump,
}

/// Source-file order of mission chunks which extend Original's grid arrays.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MissionGridChunk {
    Tactic,
    Script,
    Patch,
}

/// Source order of mission chunks which construct `RHElement` instances.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MissionElementChunk {
    Element,
    Scroll,
    Bonus,
    Mobile,
    Patch,
}

/// Source order of groups nested inside the mission ELEMENT chunk.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MissionElementGroup {
    Civilian,
    Soldier,
    BeamMe,
    Target,
    PcToRescue,
    Animal,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct LoadedProtoLevel {
    pub format: LevelFormat,
    pub misc: Option<ProtoMisc>,
    /// Source-file order of chunks that create script/render entities.
    #[serde(default)]
    pub element_chunk_order: Vec<ProtoElementChunk>,
    /// Exact source order for non-self-describing legacy grid topology.
    #[serde(default)]
    pub grid_chunk_order: Vec<ProtoGridChunk>,
    pub patches: Vec<RawPatch>,
    pub animations: Vec<RawElementFx>,
    pub material_sectors: Vec<RawMaterialSector>,
    pub light_sectors: Vec<RawLightSector>,
    pub elevation_lines: Vec<RawElevationLine>,
    pub masks: Vec<RawMask>,
    pub sight_obstacles: Vec<RawSightObstacle>,
    /// Subset of `material_sectors` indices that the SIGHT chunk flags
    /// as runtime-active. Only
    /// material sectors listed here participate in spatial material
    /// queries (footstep lookup, water/hole impact detection); sectors
    /// present in CHUNK_MATERIAL but missing from this list exist only
    /// as per-obstacle `material_indices` references in obstacle
    /// proto data.
    pub sight_material_indices: Vec<u16>,
    pub sound_sources: Vec<RawSoundSource>,
    pub jump_zones: Vec<RawJumpZone>,
    pub jump_line_pairs: Vec<RawJumpLinePair>,
    pub lifts: Vec<RawLift>,
    pub buildings: Vec<RawBuildingEntry>,
    pub motion_data: Option<RawMotionData>,
}

/// Data loaded from a mission file (.rhm).
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct LoadedMission {
    pub format: LevelFormat,
    /// Rust-authored geometry-only missions need a vacant slot zero because
    /// their AI handles use zero as the null sentinel.
    #[serde(default)]
    pub reserve_null_ai_handle: bool,
    pub header: MissionHeader,
    /// Exact source order for chunks which append to legacy grid arrays.
    #[serde(default)]
    pub grid_chunk_order: Vec<MissionGridChunk>,
    /// Exact source order for chunks which append to `marrayElements`.
    #[serde(default)]
    pub element_chunk_order: Vec<MissionElementChunk>,
    /// Exact group order inside the ELEMENT chunk.
    #[serde(default)]
    pub element_group_order: Vec<MissionElementGroup>,
    pub beam_mes: Vec<BeamMe>,
    pub soldiers: Vec<RawSoldier>,
    pub civilians: Vec<RawCivilian>,
    pub targets: Vec<RawTarget>,
    pub bonuses: Vec<RawBonus>,
    pub pcs_to_rescue: Vec<RawPcRescue>,
    pub scrolls: Vec<RawScroll>,
    /// Mission-level patches (from PATCH_2/POUF chunk).
    pub mission_patches: Vec<RawPatch>,
    /// Building tenants (from GUYS/CAVE chunk).
    pub building_tenants: Vec<RawBuildingTenants>,
    /// Script objects (from SCRP/GULP chunk).
    pub script_objects: Option<RawScriptObjects>,
    /// Hiking/patrol paths (from PWAY/RAIL chunk).
    pub hiking_paths: Vec<RawHikingPath>,
    /// Moving cart/chariot elements (from CART/TING chunk).
    pub mobile_elements: Vec<RawMobileElement>,
    /// AI tactic data (from AI /HIRN chunk).
    pub tactic_data: Option<RawTacticData>,
}

/// Complete loaded level (proto-level + mission).
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct LoadedLevel {
    pub proto: LoadedProtoLevel,
    pub mission: LoadedMission,
}

/// Datadir-relative path of the hackable JSON level descriptor for a mission.
pub fn hackable_level_descriptor_path(mission_filename: &str) -> String {
    format!("Data/Levels/{mission_filename}.level.json")
}

/// Whether a hackable JSON level descriptor exists for this mission in the
/// primary datadir or any registered overlay.
pub fn hackable_level_exists(mission_filename: &str) -> bool {
    crate::sbfile::SbFile::exists(&hackable_level_descriptor_path(mission_filename))
}

/// Editable geometry source for a hackable JSON level.
///
/// This deliberately describes gameplay geometry rather than mirroring the
/// legacy RHP/RHM serialization. It is loaded through the normal datadir
/// overlay and expanded into the same raw structs as an original level.
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub struct HackableLevelDescriptor {
    /// Display name shown in menus; falls back to the mission filename.
    #[serde(default)]
    pub title: Option<String>,
    pub map_filename: String,
    pub spawn: (i16, i16),
    /// Whether to create the ordinary player-controlled beam-me PC.
    #[serde(default = "default_true")]
    pub spawn_player: bool,
    /// Spawn authored NPCs fully revealed rather than as fog silhouettes.
    #[serde(default)]
    pub reveal_all: bool,
    pub walkable_polygon: Vec<(i16, i16)>,
    #[serde(default)]
    pub volumes: Vec<HackableLevelVolume>,
    #[serde(default)]
    pub soldiers: Vec<HackableSoldier>,
    #[serde(default)]
    pub pcs: Vec<HackablePc>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub struct HackableSoldier {
    pub position: (i16, i16),
    pub profile: HackableSoldierProfile,
    pub allegiance: u16,
    #[serde(default)]
    pub direction: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(untagged)]
pub enum HackableSoldierProfile {
    Identifier(String),
    LegacyIndex(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub struct HackablePc {
    pub position: (i16, i16),
    pub profile: u32,
    pub allegiance: u16,
    #[serde(default)]
    pub direction: u32,
    /// Starts this PC in an automatically evaluated swordfight against the
    /// nearest hostile autonomous PC.
    #[serde(default)]
    pub autonomous: bool,
    /// Opt-in high-activity combat behavior for autonomous battle examples.
    #[serde(default)]
    pub aggressive_combat: bool,
    #[serde(default)]
    pub playable: bool,
}

/// One simplified, convex architectural volume in a hackable level descriptor.
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub struct HackableLevelVolume {
    pub name: String,
    pub footprint: Vec<(i16, i16)>,
    pub z_bottom: f32,
    pub z_top: f32,
    #[serde(default)]
    pub motion_blocking: bool,
    #[serde(default)]
    pub opaque: bool,
    /// Deprecated authoring hint retained for descriptor compatibility.
    /// Sprite occlusion comes exclusively from the continuous depth map.
    #[serde(default)]
    pub occludes_sprites: bool,
}

impl LoadedLevel {
    /// Expand a hackable JSON level descriptor into normal level structs.
    ///
    /// Hackable levels have no legacy `.rhp`/`.rhm` authoring source, so
    /// their minimum playable topology is expressed directly in the same
    /// decoded structs those files normally produce. The trailing empty
    /// layer is the lift layer required by the original motion-layout
    /// convention.
    pub fn hackable_from_json(bytes: &[u8]) -> Result<Self, String> {
        let descriptor: HackableLevelDescriptor = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid hackable level descriptor: {error}"))?;
        if descriptor.walkable_polygon.len() < 3 {
            return Err("walkable_polygon must contain at least three points".to_owned());
        }
        for volume in &descriptor.volumes {
            if volume.footprint.len() < 3 {
                return Err(format!(
                    "volume {:?} must contain at least three footprint points",
                    volume.name
                ));
            }
            if volume.z_top <= volume.z_bottom {
                return Err(format!(
                    "volume {:?} has invalid height range {}..{}",
                    volume.name, volume.z_bottom, volume.z_top
                ));
            }
        }

        let mut level = Self::empty_for_test();
        level.mission.reserve_null_ai_handle = true;
        level.proto.grid_chunk_order = vec![ProtoGridChunk::Sight, ProtoGridChunk::Motion];
        let motion_obstacles = descriptor
            .volumes
            .iter()
            .filter(|volume| volume.motion_blocking)
            .map(|volume| RawMotionObstacle {
                // Static authored geometry is active in every state.
                state_id: 0,
                polygon: SectorPolygon {
                    points: volume.footprint.clone(),
                },
            })
            .collect();
        level.proto.motion_data = Some(RawMotionData {
            layers: vec![
                vec![RawMotionArea {
                    is_lift: false,
                    state_id: 0,
                    polygon: SectorPolygon {
                        points: descriptor.walkable_polygon,
                    },
                    skeleton_segments: Vec::new(),
                    flags: 0,
                    obstacles: motion_obstacles,
                }],
                Vec::new(),
            ],
            graph_bytes: Vec::new(),
        });
        for volume in descriptor.volumes {
            if !volume.opaque {
                continue;
            }
            level.proto.sight_obstacles.push(RawSightObstacle {
                points: volume
                    .footprint
                    .iter()
                    .map(|&(x, y)| RawObstaclePoint {
                        x: f32::from(x),
                        y: f32::from(y),
                        z_bottom: volume.z_bottom,
                        z_top: volume.z_top,
                    })
                    .collect(),
                projection_area: None,
                opaque: volume.opaque,
                solid: volume.motion_blocking,
                mouse: volume.motion_blocking,
                show_shadow_polygon: false,
                default_material: 0,
                material_indices: Vec::new(),
            });
        }
        level.mission.header = MissionHeader {
            control_crc: 0,
            ambiance: 1,
            map_filename: descriptor.map_filename,
            mission_profile_id: 0,
        };
        level.mission.element_chunk_order = vec![MissionElementChunk::Element];
        level.mission.element_group_order = Vec::new();
        if descriptor.spawn_player {
            level
                .mission
                .element_group_order
                .push(MissionElementGroup::BeamMe);
            level.mission.beam_mes = vec![BeamMe {
                position: MapPoint::new(
                    f32::from(descriptor.spawn.0),
                    f32::from(descriptor.spawn.1),
                ),
                direction: 0,
                action: 0,
                projection_area: u16::MAX,
                sector: 0,
                layer: 0,
                material: 0,
                action_required: BeamMeActions::default(),
                index: 0,
                script: None,
                required_pc: 0,
            }];
        }
        if !descriptor.soldiers.is_empty() {
            level
                .mission
                .element_group_order
                .push(MissionElementGroup::Soldier);
        }
        let reveal_all = descriptor.reveal_all;
        level.mission.soldiers = descriptor
            .soldiers
            .into_iter()
            .map(|soldier| {
                Ok(RawSoldier {
                    position_x: soldier
                        .position
                        .0
                        .try_into()
                        .map_err(|_| "hackable soldier x must be non-negative")?,
                    position_y: soldier
                        .position
                        .1
                        .try_into()
                        .map_err(|_| "hackable soldier y must be non-negative")?,
                    direction: soldier.direction,
                    action: 0,
                    obstacle_index: u16::MAX,
                    sector: 0,
                    layer: 0,
                    material: 0,
                    profile_number: match &soldier.profile {
                        HackableSoldierProfile::LegacyIndex(index) => *index,
                        HackableSoldierProfile::Identifier(_) => 0,
                    },
                    profile_id: match soldier.profile {
                        HackableSoldierProfile::Identifier(identifier) => Some(identifier),
                        HackableSoldierProfile::LegacyIndex(_) => None,
                    },
                    allegiance: Some(soldier.allegiance),
                    revealed: reveal_all,
                    tower_guard: false,
                    company_number: 0,
                    drunk_level: 0,
                    money: 0,
                    subordinate_ids: Vec::new(),
                    path_id: 0,
                    alert_path_id: 0,
                    script_class: None,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if !descriptor.pcs.is_empty() {
            level
                .mission
                .element_group_order
                .push(MissionElementGroup::PcToRescue);
        }
        level.mission.pcs_to_rescue = descriptor
            .pcs
            .into_iter()
            .map(|pc| RawPcRescue {
                position_x: pc.position.0,
                position_y: pc.position.1,
                direction: pc.direction,
                action: 0,
                obstacle_index: u16::MAX,
                sector: 0,
                layer: 0,
                material: 0,
                profile_index: pc.profile,
                allegiance: Some(pc.allegiance),
                autonomous: pc.autonomous,
                aggressive_combat: pc.aggressive_combat,
                playable: pc.playable,
                attributes: 0,
                script_class: None,
            })
            .collect();
        Ok(level)
    }

    /// Produce an empty `LoadedLevel` suitable for unit tests that
    /// need to drive `Engine::new` without hitting disk.  All
    /// `Vec`/`Option` fields are empty; scalar fields are zero.
    ///
    /// The engine initialisation runs through every level-load phase
    /// (mission-stream spawn, motion data consume, init_ai) with
    /// nothing to process — the resulting engine has no entities,
    /// no pathfinder graph, no motion lines.  That's exactly what
    /// the previous generation of tests wanted from the old blank
    /// `Engine::new(EngineArgs { ..Default::default() })` path, so
    /// it slots in cleanly.
    pub fn empty_for_test() -> Self {
        Self {
            proto: LoadedProtoLevel {
                format: LevelFormat::Fullgame,
                misc: None,
                element_chunk_order: Vec::new(),
                grid_chunk_order: Vec::new(),
                patches: Vec::new(),
                animations: Vec::new(),
                material_sectors: Vec::new(),
                light_sectors: Vec::new(),
                elevation_lines: Vec::new(),
                masks: Vec::new(),
                sight_obstacles: Vec::new(),
                sight_material_indices: Vec::new(),
                sound_sources: Vec::new(),
                jump_zones: Vec::new(),
                jump_line_pairs: Vec::new(),
                lifts: Vec::new(),
                buildings: Vec::new(),
                motion_data: None,
            },
            mission: LoadedMission {
                format: LevelFormat::Fullgame,
                reserve_null_ai_handle: false,
                header: MissionHeader {
                    control_crc: 0,
                    ambiance: 0, // Day
                    map_filename: String::new(),
                    mission_profile_id: 0,
                },
                grid_chunk_order: Vec::new(),
                element_chunk_order: Vec::new(),
                element_group_order: Vec::new(),
                beam_mes: Vec::new(),
                soldiers: Vec::new(),
                civilians: Vec::new(),
                targets: Vec::new(),
                bonuses: Vec::new(),
                pcs_to_rescue: Vec::new(),
                scrolls: Vec::new(),
                mission_patches: Vec::new(),
                building_tenants: Vec::new(),
                script_objects: None,
                hiking_paths: Vec::new(),
                mobile_elements: Vec::new(),
                tactic_data: None,
            },
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Public loading entry point
// ═══════════════════════════════════════════════════════════════════

const PROTO_EXTENSION: &str = ".rhp";
const MISSION_EXTENSION: &str = ".rhm";
const BEGGAR_SCROLL_SET_COUNT: usize = 10;

/// Load a complete level from proto-level (`.rhp`) and mission (`.rhm`) files.
///
/// `is_beggar` determines whether a given civilian profile index is a beggar.
/// Beggars have extra scroll-set data in the binary; without this predicate
/// the file cannot be parsed correctly.
pub fn load_level(
    mission_name: &str,
    proto_level_name: &str,
    level_directory: &str,
    is_beggar: &dyn Fn(u32) -> bool,
    progress: &mut dyn FnMut(f32),
) -> Result<LoadedLevel, LevelError> {
    let proto_path = format!(
        "{}/{}{}",
        level_directory, proto_level_name, PROTO_EXTENSION
    );
    let mission_path = format!("{}/{}{}", level_directory, mission_name, MISSION_EXTENSION);

    // Open proto-level and detect format
    let proto_file = SbFile::open(&proto_path, SB_FILE_READ)
        .map_err(|_| LevelError::FileNotFound(proto_path.clone()))?;
    let mut proto_reader = ChunkReader::new(proto_file);

    let format = {
        let tag = proto_reader.peek_next_chunk()?;
        LevelFormat::detect(&tag)?
    };

    tracing::info!(
        "Loading level: proto='{}', mission='{}', format={:?}",
        proto_level_name,
        mission_name,
        format
    );

    let proto = load_proto_level(&mut proto_reader, format)?;
    progress(1.0);

    // Open and load mission
    let mission_file = SbFile::open(&mission_path, SB_FILE_READ)
        .map_err(|_| LevelError::FileNotFound(mission_path.clone()))?;
    let mut mission_reader = ChunkReader::new(mission_file);

    let mission = load_mission(&mut mission_reader, format, is_beggar)?;
    progress(1.0);

    Ok(LoadedLevel { proto, mission })
}

// ═══════════════════════════════════════════════════════════════════
//  Proto-level loading
// ═══════════════════════════════════════════════════════════════════

pub fn load_proto_level(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<LoadedProtoLevel, LevelError> {
    reader.chunk_start(format.proto_tag(), format.file_version())?;

    let mut misc = None;
    let mut element_chunk_order = Vec::new();
    let mut grid_chunk_order = Vec::new();
    let mut patches = Vec::new();
    let mut animations = Vec::new();
    let mut material_sectors = Vec::new();
    let mut light_sectors = Vec::new();
    let mut elevation_lines = Vec::new();
    let mut masks = Vec::new();
    let mut sight_obstacles = Vec::new();
    let mut sight_material_indices = Vec::new();
    let mut sound_sources = Vec::new();
    let mut jump_zones = Vec::new();
    let mut jump_line_pairs = Vec::new();
    let mut lifts = Vec::new();
    let mut buildings = Vec::new();
    let mut motion_data = None;
    let mut skipped_chunks = Vec::new();

    while !reader.at_end_of_chunk() {
        let tag = reader.peek_next_chunk()?;

        if tag == *format.misc_tag() {
            tracing::debug!("Proto: loading MISC chunk");
            misc = Some(read_proto_misc(reader, format)?);
        } else if tag == *format.patch_tag() {
            tracing::debug!("Proto: loading PATCH chunk");
            element_chunk_order.push(ProtoElementChunk::Patch);
            grid_chunk_order.push(ProtoGridChunk::Patch);
            patches = read_proto_patches(reader, format, true)?;
        } else if tag == *format.animation_tag() {
            tracing::debug!("Proto: loading ANIMATION chunk");
            element_chunk_order.push(ProtoElementChunk::Animation);
            animations = read_proto_animations(reader, format)?;
        } else if tag == *format.material_tag() {
            tracing::debug!("Proto: loading MATERIAL chunk");
            grid_chunk_order.push(ProtoGridChunk::Material);
            material_sectors = read_material_sectors(reader, format)?;
        } else if tag == *format.light_tag() {
            tracing::debug!("Proto: loading LIGHT chunk");
            grid_chunk_order.push(ProtoGridChunk::Light);
            light_sectors = read_light_sectors(reader, format)?;
        } else if tag == *format.bond_tag() {
            tracing::debug!("Proto: loading BOND chunk");
            grid_chunk_order.push(ProtoGridChunk::Bond);
            elevation_lines = read_elevation_lines(reader, format)?;
        } else if tag == *format.mask_tag() {
            tracing::debug!("Proto: loading MASK chunk");
            masks = read_masks(reader, format)?;
        } else if tag == *format.sight_tag() {
            tracing::debug!("Proto: loading SIGHT chunk");
            grid_chunk_order.push(ProtoGridChunk::Sight);
            let sight = read_sight_obstacles(reader, format)?;
            sight_obstacles = sight.obstacles;
            sight_material_indices = sight.material_indices;
        } else if tag == *format.sound_tag() {
            tracing::debug!("Proto: loading SOUND chunk");
            sound_sources = read_sound_sources(reader, format)?;
        } else if tag == *format.jump_tag() {
            tracing::debug!("Proto: loading JUMP chunk");
            grid_chunk_order.push(ProtoGridChunk::Jump);
            (jump_zones, jump_line_pairs) = read_jump_stuff(reader, format)?;
        } else if tag == *format.lift_tag() {
            tracing::debug!("Proto: loading LIFT chunk");
            grid_chunk_order.push(ProtoGridChunk::Lift);
            lifts = read_lifts(reader, format)?;
        } else if tag == *format.building_tag() {
            tracing::debug!("Proto: loading BUILDING chunk");
            grid_chunk_order.push(ProtoGridChunk::Building);
            buildings = read_buildings(reader, format)?;
        } else if tag == *format.motion_tag() {
            tracing::debug!("Proto: loading MOTION chunk");
            grid_chunk_order.push(ProtoGridChunk::Motion);
            motion_data = Some(read_motion_data(reader, format)?);
        } else if tag == *format.background_tag() {
            // "There is no background in proto-levels!" — proto-level streams
            // never carry a background chunk, so this is unexpected.
            tracing::warn!("Proto: unexpected BACKGROUND chunk, skipping");
            reader.skip_chunk()?;
            skipped_chunks.push(tag_str(&tag));
        } else {
            let name = tag_str(&tag);
            tracing::warn!("Proto: skipping unknown chunk '{}'", name);
            reader.skip_chunk()?;
            skipped_chunks.push(name);
        }
    }

    reader.chunk_end()?;

    tracing::info!(
        "Proto loaded: misc={}, {} patches, {} animations, {} materials, \
         {} lights, {} bonds, {} masks, {} sights, {} sounds, \
         {} jump_zones, {} jump_pairs, {} lifts, {} buildings, motion={}, {} skipped",
        misc.is_some(),
        patches.len(),
        animations.len(),
        material_sectors.len(),
        light_sectors.len(),
        elevation_lines.len(),
        masks.len(),
        sight_obstacles.len(),
        sound_sources.len(),
        jump_zones.len(),
        jump_line_pairs.len(),
        lifts.len(),
        buildings.len(),
        motion_data.is_some(),
        skipped_chunks.len(),
    );

    Ok(LoadedProtoLevel {
        format,
        misc,
        element_chunk_order,
        grid_chunk_order,
        patches,
        animations,
        material_sectors,
        light_sectors,
        elevation_lines,
        masks,
        sight_obstacles,
        sight_material_indices,
        sound_sources,
        jump_zones,
        jump_line_pairs,
        lifts,
        buildings,
        motion_data,
    })
}

// ── Shared sub-readers ─────────────────────────────────────────────

/// Read an `RHSector::InitializeFromProtoStream` polygon.
///
/// Binary format:
/// - \[fullgame: u8 padding\]
/// - u16 point count
/// - per point: i16 x, i16 y
/// - \[fullgame: u8 padding\]
fn read_sector_polygon(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<SectorPolygon, LevelError> {
    reader.read_padding_if_fullgame(format)?;

    let num_points = reader.read_u16()?;
    let mut points = Vec::with_capacity(num_points as usize);
    for _ in 0..num_points {
        let x = reader.read_i16()?;
        let y = reader.read_i16()?;
        points.push((x, y));
    }

    reader.read_padding_if_fullgame(format)?;

    Ok(SectorPolygon { points })
}

/// Read the MOTION chunk (motion obstacles + pathfinder graph bytes).
///
/// We read the motion obstacles into raw structs and capture the remaining
/// bytes as the graph data (parsed later by `PathGraph::load_from_proto_stream`).
fn read_motion_data(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<RawMotionData, LevelError> {
    reader.chunk_start(format.motion_tag(), format.motion_ver())?;

    // ── Part 1: Motion obstacles ──
    let num_layers = reader.read_u16()?;
    let mut layers = Vec::with_capacity(num_layers as usize);

    for _layer_idx in 0..num_layers {
        let num_areas = reader.read_u16()?;
        let mut areas = Vec::with_capacity(num_areas as usize);

        for _area_idx in 0..num_areas {
            let is_lift = reader.read_bool()?;
            let state_id = reader.read_u32()?;

            // Sector polygon
            let polygon = read_sector_polygon(reader, format)?;

            // Skeleton segments
            let num_segments = reader.read_u16()?;
            let mut skeleton_segments = Vec::with_capacity(num_segments as usize);
            for _ in 0..num_segments {
                let x1 = reader.read_i16()?;
                let y1 = reader.read_i16()?;
                let x2 = reader.read_i16()?;
                let y2 = reader.read_i16()?;
                skeleton_segments.push(((x1, y1), (x2, y2)));
            }

            // Flags (crouched)
            let flags = reader.read_u32()?;

            // Motion obstacles (sub-sectors within this area)
            let num_obstacles = reader.read_u16()?;
            let mut obstacles = Vec::with_capacity(num_obstacles as usize);
            for _ in 0..num_obstacles {
                let obs_state_id = reader.read_u32()?;
                let obs_polygon = read_sector_polygon(reader, format)?;
                obstacles.push(RawMotionObstacle {
                    state_id: obs_state_id,
                    polygon: obs_polygon,
                });
            }

            areas.push(RawMotionArea {
                is_lift,
                state_id,
                polygon,
                skeleton_segments,
                flags,
                obstacles,
            });
        }

        layers.push(areas);
    }

    // ── Part 2: Pathfinder graph ──
    // The remaining bytes in this chunk are the graph data,
    // parsed later by PathGraph::load_from_proto_stream.
    let graph_remaining = reader.remaining_in_chunk();
    let graph_bytes = reader.read_bytes(graph_remaining)?;

    reader.chunk_end()?;

    tracing::info!(
        "MOTION chunk: {} layers, graph bytes: {}",
        layers.len(),
        graph_bytes.len(),
    );

    Ok(RawMotionData {
        layers,
        graph_bytes,
    })
}

/// Read sprite reference from `RHSprite::LoadSpriteFromFile` for
/// `RHFRAMEKIND_ANIMATION`.
///
/// Binary format:
/// - `LoadFrameInfoFromFile`: string (frame_profile_name), string (profile_name).
///   For ANIMATION kind, no alternate profile is read.
/// - `LoadPositionInfoFromFile`: i16 x, i16 y, u16 elevation.
fn read_sprite_ref_animation(reader: &mut ChunkReader) -> Result<RawSpriteRef, LevelError> {
    let frame_profile_name = reader.read_string()?;
    let profile_name = reader.read_string()?;
    let position_x = reader.read_i16()?;
    let position_y = reader.read_i16()?;
    let elevation = reader.read_u16()?;

    Ok(RawSpriteRef {
        frame_profile_name,
        profile_name,
        position_x,
        position_y,
        elevation,
    })
}

/// Read `RHElementFX::InitializeFromProtoStream`.
///
/// Binary format:
/// 1. Sprite ref (ANIMATION kind)
/// 2. u8 blit_type
/// 3. bool active
/// 4. bool force_display
/// 5. \[fullgame: u8 padding\]
/// 6. u16 point_count + (i16, i16) pairs
/// 7. \[fullgame: u8 padding\]
fn read_element_fx(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<RawElementFx, LevelError> {
    let sprite = read_sprite_ref_animation(reader)?;
    let blit_type = reader.read_u8()?;
    let active = reader.read_bool()?;
    let force_display = reader.read_bool()?;

    reader.read_padding_if_fullgame(format)?;

    let num_points = reader.read_u16()?;
    let mut display_polyline = Vec::with_capacity(num_points as usize);
    for _ in 0..num_points {
        let x = reader.read_i16()?;
        let y = reader.read_i16()?;
        display_polyline.push((x, y));
    }

    reader.read_padding_if_fullgame(format)?;

    Ok(RawElementFx {
        sprite,
        blit_type,
        active,
        force_display,
        display_polyline,
    })
}

// ── CHUNK_MISC reader ──────────────────────────────────────────────

fn read_proto_misc(reader: &mut ChunkReader, format: LevelFormat) -> Result<ProtoMisc, LevelError> {
    reader.chunk_start(format.misc_tag(), format.misc_ver())?;

    let control_crc = reader.read_u32()?;
    let forest_level = reader.read_bool()?;
    let default_material = reader.read_u32()?;

    reader.chunk_end()?;

    tracing::info!(
        "MISC: crc={:#x}, forest={}, material={}",
        control_crc,
        forest_level,
        default_material,
    );

    Ok(ProtoMisc {
        control_crc,
        forest_level,
        default_material,
    })
}

// ── CHUNK_PATCH reader ─────────────────────────────────────────────

/// Read patches from `RHEngine::InitializePatchFromProtoStream`.
///
/// `is_proto_level` controls which chunk tag is expected:
/// - proto-level: CHUNK_PATCH
/// - mission: CHUNK_PATCH_2 (fullgame) or CHUNK_PATCH (demo)
fn read_proto_patches(
    reader: &mut ChunkReader,
    format: LevelFormat,
    is_proto_level: bool,
) -> Result<Vec<RawPatch>, LevelError> {
    let tag = if is_proto_level {
        format.patch_tag()
    } else {
        format.patch2_tag()
    };
    reader.chunk_start(tag, format.patch_ver())?;

    let num_patches = reader.read_u16()?;
    let mut patches = Vec::with_capacity(num_patches as usize);

    for i in 0..num_patches {
        let patch = read_one_patch(reader, format)?;
        tracing::trace!(
            "  patch {}: sprite='{}'",
            i,
            patch.element_fx.sprite.frame_profile_name
        );
        patches.push(patch);
    }

    reader.chunk_end()?;
    Ok(patches)
}

/// Read a single `RHPatch::InitializeFromProtoStream`.
fn read_one_patch(reader: &mut ChunkReader, format: LevelFormat) -> Result<RawPatch, LevelError> {
    // 1. ElementFX (sprite + display polygon)
    let element_fx = read_element_fx(reader, format)?;

    // 2. Activity
    let active = reader.read_bool()?;

    // 3. Pathfinder changing obstacles
    let raw_pf = reader.read_u32()?;
    let (pathfinder_changing_obstacles, pathfinder_sector, pathfinder_layer) = if raw_pf != 0 {
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;
        // The stored value is encoded as `(real - 1) >> 1`; decode the inverse.
        ((raw_pf - 1) >> 1, Some(sector), Some(layer))
    } else {
        (0, None, None)
    };

    // 4. Animation validity flags
    let start_animation_valid = reader.read_bool()?;
    let transition_animation_valid = reader.read_bool()?;
    let end_animation_valid = reader.read_bool()?;

    // 5. Waypoint
    let way_x = reader.read_i16()?;
    let way_y = reader.read_i16()?;

    // 6. Sector & layer
    let sector = reader.read_u16()?;
    let layer = reader.read_u16()?;

    // 7. Definitive (inverted!)
    let definitive = !reader.read_bool()?;

    // 8. Integrate in background
    let integrate_in_background = reader.read_bool()?;

    // 9. Old masks
    let num_old_masks = reader.read_u16()?;
    let mut old_masks = Vec::with_capacity(num_old_masks as usize);
    for _ in 0..num_old_masks {
        let mask_layer = reader.read_u16()?;
        let mask_index = reader.read_u16()?;
        old_masks.push(MaskRef {
            layer: mask_layer,
            index: mask_index,
        });
    }

    // 10. Old sight obstacles
    let num_old_sight = reader.read_u16()?;
    let mut old_sight_obstacles = Vec::with_capacity(num_old_sight as usize);
    for _ in 0..num_old_sight {
        old_sight_obstacles.push(reader.read_u16()?);
    }

    // 11. Old clickable sector
    let old_mouse_sector = read_sector_polygon(reader, format)?;
    // 12. Old masking mouse sector
    let old_masking_sector = read_sector_polygon(reader, format)?;

    // 13. New masks
    let num_new_masks = reader.read_u16()?;
    let mut new_masks = Vec::with_capacity(num_new_masks as usize);
    for _ in 0..num_new_masks {
        let mask_layer = reader.read_u16()?;
        let mask_index = reader.read_u16()?;
        new_masks.push(MaskRef {
            layer: mask_layer,
            index: mask_index,
        });
    }

    // 14. New sight obstacles
    let num_new_sight = reader.read_u16()?;
    let mut new_sight_obstacles = Vec::with_capacity(num_new_sight as usize);
    for _ in 0..num_new_sight {
        new_sight_obstacles.push(reader.read_u16()?);
    }

    // 15. New clickable sector
    let new_mouse_sector = read_sector_polygon(reader, format)?;
    // 16. New masking mouse sector
    let new_masking_sector = read_sector_polygon(reader, format)?;

    // 17. Apply sector
    let apply_sector = read_sector_polygon(reader, format)?;
    // 18. No-apply sector
    let no_apply_sector = read_sector_polygon(reader, format)?;

    // 19. Door data
    let door_triggered = reader.read_bool()?;
    let triggers_door = reader.read_bool()?;

    let num_doors = reader.read_u16()?;
    let mut door_indices = Vec::with_capacity(num_doors as usize);
    for _ in 0..num_doors {
        door_indices.push(reader.read_u16()?);
    }

    // 20. Final layer
    let final_layer = reader.read_u16()?;

    Ok(RawPatch {
        element_fx,
        active,
        pathfinder_changing_obstacles,
        pathfinder_sector,
        pathfinder_layer,
        start_animation_valid,
        transition_animation_valid,
        end_animation_valid,
        waypoint: (way_x, way_y),
        sector,
        layer,
        definitive,
        integrate_in_background,
        old_masks,
        old_sight_obstacles,
        old_mouse_sector,
        old_masking_sector,
        new_masks,
        new_sight_obstacles,
        new_mouse_sector,
        new_masking_sector,
        apply_sector,
        no_apply_sector,
        door_triggered,
        triggers_door,
        door_indices,
        final_layer,
    })
}

// ── CHUNK_ANIMATION reader ─────────────────────────────────────────

fn read_proto_animations(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawElementFx>, LevelError> {
    reader.chunk_start(format.animation_tag(), format.animation_ver())?;

    let num_animations = reader.read_u16()?;
    let mut animations = Vec::with_capacity(num_animations as usize);

    for i in 0..num_animations {
        let fx = read_element_fx(reader, format)?;
        tracing::trace!(
            "  animation {}: sprite='{}'",
            i,
            fx.sprite.frame_profile_name
        );
        animations.push(fx);
    }

    reader.chunk_end()?;
    Ok(animations)
}

// ═══════════════════════════════════════════════════════════════════
//  Mission loading
// ═══════════════════════════════════════════════════════════════════

pub fn load_mission(
    reader: &mut ChunkReader,
    format: LevelFormat,
    is_beggar: &dyn Fn(u32) -> bool,
) -> Result<LoadedMission, LevelError> {
    reader.chunk_start(format.mission_tag(), format.file_version())?;

    // Header must be the first chunk
    let header = read_header(reader, format)?;

    tracing::info!(
        "Mission header: ambiance={}, map='{}', profile_id={}",
        header.ambiance,
        header.map_filename,
        header.mission_profile_id
    );

    let mut beam_mes = Vec::new();
    let mut grid_chunk_order = Vec::new();
    let mut element_chunk_order = Vec::new();
    let mut element_group_order = Vec::new();
    let mut soldiers = Vec::new();
    let mut civilians = Vec::new();
    let mut targets = Vec::new();
    let mut bonuses = Vec::new();
    let mut pcs_to_rescue = Vec::new();
    let mut scrolls = Vec::new();
    let mut mission_patches = Vec::new();
    let mut building_tenants = Vec::new();
    let mut script_objects = None;
    let mut hiking_paths = Vec::new();
    let mut mobile_elements = Vec::new();
    let mut tactic_data = None;
    let mut skipped_chunks = Vec::new();

    while !reader.at_end_of_chunk() {
        let tag = reader.peek_next_chunk()?;
        if tag == *format.element_tag() {
            element_chunk_order.push(MissionElementChunk::Element);
            let elems = read_elements(reader, format, is_beggar)?;
            element_group_order = elems.group_order;
            beam_mes = elems.beam_mes;
            soldiers = elems.soldiers;
            civilians = elems.civilians;
            targets = elems.targets;
            pcs_to_rescue = elems.pcs_to_rescue;
        } else if tag == *format.bonus_tag() {
            element_chunk_order.push(MissionElementChunk::Bonus);
            bonuses = read_bonuses(reader, format)?;
        } else if tag == *format.scroll_tag() {
            element_chunk_order.push(MissionElementChunk::Scroll);
            scrolls = read_scrolls(reader, format)?;
        } else if tag == *format.patch2_tag() || tag == *format.patch_tag() {
            tracing::debug!("Mission: loading PATCH_2 chunk");
            grid_chunk_order.push(MissionGridChunk::Patch);
            element_chunk_order.push(MissionElementChunk::Patch);
            mission_patches = read_proto_patches(reader, format, false)?;
        } else if tag == *format.tenant_tag() {
            tracing::debug!("Mission: loading TENANT chunk");
            building_tenants = read_building_tenants(reader, format)?;
        } else if tag == *format.mobile_tag() {
            tracing::debug!("Mission: loading MOBILE chunk");
            element_chunk_order.push(MissionElementChunk::Mobile);
            mobile_elements = read_mobile_elements(reader, format)?;
        } else if tag == *format.script_tag() {
            tracing::debug!("Mission: loading SCRIPT chunk");
            grid_chunk_order.push(MissionGridChunk::Script);
            script_objects = Some(read_script_objects(reader, format)?);
        } else if tag == *format.path_tag() {
            tracing::debug!("Mission: loading PATH chunk");
            hiking_paths = read_hiking_paths(reader, format)?;
        } else if tag == *format.tactic_tag() {
            tracing::debug!("Mission: loading TACTIC chunk");
            grid_chunk_order.push(MissionGridChunk::Tactic);
            tactic_data = Some(read_tactic_data(reader, format)?);
        } else {
            let name = tag_str(&tag);
            // All mission chunks now handled
            tracing::warn!("Mission: skipping chunk '{}'", name);
            reader.skip_chunk()?;
            skipped_chunks.push(name);
        }
    }

    reader.chunk_end()?;

    tracing::info!(
        "Mission loaded: {} soldiers, {} civilians, {} targets, \
         {} bonuses, {} beam-mes, {} PCs, {} scrolls, {} patches, {} tenants, \
         script_objects={}, {} paths, {} mobiles, tactic={}, {} skipped",
        soldiers.len(),
        civilians.len(),
        targets.len(),
        bonuses.len(),
        beam_mes.len(),
        pcs_to_rescue.len(),
        scrolls.len(),
        mission_patches.len(),
        building_tenants.len(),
        script_objects
            .as_ref()
            .map(|so| so.points.len() + so.sectors.len())
            .unwrap_or(0),
        hiking_paths.len(),
        mobile_elements.len(),
        tactic_data.is_some(),
        skipped_chunks.len()
    );

    Ok(LoadedMission {
        format,
        reserve_null_ai_handle: false,
        header,
        grid_chunk_order,
        element_chunk_order,
        element_group_order,
        beam_mes,
        soldiers,
        civilians,
        targets,
        bonuses,
        pcs_to_rescue,
        scrolls,
        mission_patches,
        building_tenants,
        script_objects,
        hiking_paths,
        mobile_elements,
        tactic_data,
    })
}

// ── Header chunk ────────────────────────────────────────────────

fn read_header(reader: &mut ChunkReader, format: LevelFormat) -> Result<MissionHeader, LevelError> {
    reader.chunk_start(format.header_tag(), format.header_ver())?;

    let control_crc = reader.read_u32()?;
    let ambiance = reader.read_u32()?;
    let map_filename = reader.read_string()?;
    let mission_profile_id = reader.read_u32()?;

    reader.chunk_end()?;

    Ok(MissionHeader {
        control_crc,
        ambiance,
        map_filename,
        mission_profile_id,
    })
}

// ── Element chunk (actor groups) ────────────────────────────────

/// Collected entity data from the ELEMENT chunk.
struct ParsedElements {
    group_order: Vec<MissionElementGroup>,
    beam_mes: Vec<BeamMe>,
    soldiers: Vec<RawSoldier>,
    civilians: Vec<RawCivilian>,
    targets: Vec<RawTarget>,
    pcs_to_rescue: Vec<RawPcRescue>,
}

fn read_elements(
    reader: &mut ChunkReader,
    format: LevelFormat,
    is_beggar: &dyn Fn(u32) -> bool,
) -> Result<ParsedElements, LevelError> {
    let mut beam_mes = Vec::new();
    let mut soldiers = Vec::new();
    let mut civilians = Vec::new();
    let mut targets = Vec::new();
    let mut pcs_to_rescue = Vec::new();
    let mut group_order = Vec::new();
    reader.chunk_start(format.element_tag(), format.element_ver())?;

    let num_groups = reader.read_u16()?;

    for _ in 0..num_groups {
        let tag = reader.peek_next_chunk()?;
        if tag == *format.civilian_tag() {
            group_order.push(MissionElementGroup::Civilian);
            civilians = read_civilians(reader, format, is_beggar)?;
        } else if tag == *format.soldier_tag() {
            group_order.push(MissionElementGroup::Soldier);
            soldiers = read_soldiers(reader, format)?;
        } else if tag == *format.beamme_tag() {
            group_order.push(MissionElementGroup::BeamMe);
            beam_mes = read_beam_mes(reader, format)?;
        } else if tag == *format.target_tag() {
            group_order.push(MissionElementGroup::Target);
            targets = read_targets(reader, format)?;
        } else if tag == *format.pc_tag() {
            group_order.push(MissionElementGroup::PcToRescue);
            pcs_to_rescue = read_pcs_to_rescue(reader, format)?;
        } else if tag == *format.animal_tag() {
            group_order.push(MissionElementGroup::Animal);
            read_animals(reader, format)?;
        } else {
            return Err(LevelError::UnknownElementChunk(tag_str(&tag)));
        }
    }

    reader.chunk_end()?;
    Ok(ParsedElements {
        group_order,
        beam_mes,
        soldiers,
        civilians,
        targets,
        pcs_to_rescue,
    })
}

// ── Civilians ──────────────────────────────────────────────────

fn read_civilians(
    reader: &mut ChunkReader,
    format: LevelFormat,
    is_beggar: &dyn Fn(u32) -> bool,
) -> Result<Vec<RawCivilian>, LevelError> {
    reader.chunk_start(format.civilian_tag(), format.civilian_ver())?;

    let count = reader.read_u16()?;
    let mut civilians = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let position_x = reader.read_u16()?;
        let position_y = reader.read_u16()?;
        let direction = reader.read_u32()?;
        let action = reader.read_u32()?;
        let obstacle_index = reader.read_u16()?;
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;
        let material = reader.read_u32()?;
        let profile_number = reader.read_u32()?;
        let path_id = reader.read_u16()?;
        let money = reader.read_u32()?;

        // Beggars have extra scroll-set data
        let beggar_scroll_sets = if is_beggar(profile_number) {
            let mut sets = Vec::with_capacity(BEGGAR_SCROLL_SET_COUNT);
            for _ in 0..BEGGAR_SCROLL_SET_COUNT {
                let scroll_count = reader.read_u16()?;
                let mut ids = Vec::with_capacity(scroll_count as usize);
                for _ in 0..scroll_count {
                    ids.push(reader.read_u16()?);
                }
                sets.push(ids);
            }
            Some(sets)
        } else {
            None
        };

        // Script binding.
        let script_class = if reader.read_bool()? {
            Some(reader.read_string()?)
        } else {
            None
        };

        civilians.push(RawCivilian {
            position_x,
            position_y,
            direction,
            action,
            obstacle_index,
            sector,
            layer,
            material,
            profile_number,
            allegiance: None,
            path_id,
            money,
            beggar_scroll_sets,
            script_class,
        });
    }

    reader.chunk_end()?;
    Ok(civilians)
}

fn read_animals(reader: &mut ChunkReader, format: LevelFormat) -> Result<(), LevelError> {
    // The BETE/MEOW animal chunk is a Desperados leftover.  In Robin Hood
    // the loader skips it with an "Unimplemented chunk" warning and
    // `RHElementActorAnimal::InitializeFromMissionStream` asserts false — no
    // shipped level carries any animals.  The Rust port has ripped the animal
    // system out entirely, so we only accept an empty header and panic on
    // anything else (which would indicate a modded/corrupt level we can't
    // handle).
    reader.chunk_start(format.animal_tag(), format.animal_ver())?;
    let count = reader.read_u16()?;
    if count != 0 {
        panic!(
            "animal chunk has {count} entries but Robin Hood has no animals; \
             this level is unshippable or corrupt"
        );
    }
    reader.chunk_end()?;
    Ok(())
}

// ── Soldiers ──────────────────────────────────────────────────

fn read_soldiers(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawSoldier>, LevelError> {
    reader.chunk_start(format.soldier_tag(), format.soldier_ver())?;

    let count = reader.read_u16()?;
    let mut soldiers = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let position_x = reader.read_u16()?;
        let position_y = reader.read_u16()?;
        let direction = reader.read_u32()?;
        let action = reader.read_u32()?;
        let obstacle_index = reader.read_u16()?;
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;
        let material = reader.read_u32()?;
        let profile_number = reader.read_u32()?;
        let tower_guard = reader.read_bool()?;
        let company_number = reader.read_u32()?;
        let drunk_level = reader.read_u32()?;
        let money = reader.read_u32()?;

        let num_subordinates = reader.read_u16()?;
        let mut subordinate_ids = Vec::with_capacity(num_subordinates as usize);
        for _ in 0..num_subordinates {
            subordinate_ids.push(reader.read_u16()?);
        }

        let path_id = reader.read_u16()?;
        let alert_path_id = reader.read_u16()?;

        // Script binding.
        let script_class = if reader.read_bool()? {
            Some(reader.read_string()?)
        } else {
            None
        };

        soldiers.push(RawSoldier {
            position_x,
            position_y,
            direction,
            action,
            obstacle_index,
            sector,
            layer,
            material,
            profile_number,
            profile_id: None,
            allegiance: None,
            revealed: false,
            tower_guard,
            company_number,
            drunk_level,
            money,
            subordinate_ids,
            path_id,
            alert_path_id,
            script_class,
        });
    }

    reader.chunk_end()?;
    Ok(soldiers)
}

// ── Beam-me points ─────────────────────────────────────────────

fn read_beam_mes(reader: &mut ChunkReader, format: LevelFormat) -> Result<Vec<BeamMe>, LevelError> {
    reader.chunk_start(format.beamme_tag(), format.beamme_ver())?;

    let count = reader.read_u16()?;
    let mut beam_mes = Vec::with_capacity(count as usize);

    for index in 0..count {
        let pos_x = reader.read_i16()?;
        let pos_y = reader.read_i16()?;
        let direction = reader.read_u32()?;
        let action = reader.read_u32()?;
        let projection_area = reader.read_u16()?;
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;
        let material = reader.read_u32()?;

        // 10 action requirement flags (1 byte each)
        let action_required = BeamMeActions {
            climb: reader.read_bool()?,
            jump: reader.read_bool()?,
            lockpick: reader.read_bool()?,
            archery: reader.read_bool()?,
            carry: reader.read_bool()?,
            tie: reader.read_bool()?,
            stun: reader.read_bool()?,
            lever: reader.read_bool()?,
            eat: reader.read_bool()?,
            search: reader.read_bool()?,
        };

        let scripted = reader.read_bool()?;
        let script = if scripted {
            Some(reader.read_string()?)
        } else {
            None
        };

        let required_pc = reader.read_u8()?;

        beam_mes.push(BeamMe {
            position: MapPoint::new(pos_x as f32, pos_y as f32),
            direction,
            action,
            projection_area,
            sector,
            layer,
            material,
            action_required,
            index,
            script,
            required_pc,
        });
    }

    reader.chunk_end()?;
    Ok(beam_mes)
}

// ── Beam-me summary scan (used by ProfileManager::import_beam_mes) ──

/// Result of a lightweight scan of a mission file for beam-me data only.
///
/// Extracts the per-mission beam-me count and the action-flag bundle from
/// every beam-me spawn point.  The full level loader does the same
/// parse, but this scan runs at profile-load time (before any level is
/// actually played) so the briefing UI and gang-selection math have
/// access to mission-wide required-action info.
#[derive(Debug, Clone, Default)]
pub struct MissionBeamMeScan {
    /// Beam-me count from the BEAMME chunk.  When multiple BEAMME chunks
    /// appear, the last one wins — in practice each ELEMENT chunk has at
    /// most one BEAMME group.
    pub number_of_beam_mes: u16,
    /// Action requirement flags per beam-me, in file order.  Callers can
    /// fan out one action requirement per `true` flag per beam-me; the
    /// per-beam-me grouping is preserved.
    pub action_flags: Vec<BeamMeActions>,
}

/// Scan a mission file for beam-me data only, skipping every other
/// chunk.  Used by `ProfileManager::import_beam_mes` to populate the
/// `number_of_beam_mes` / `required_actions` fields on each
/// `MissionProfile` after the CPF/JSON profile load.
///
/// On version mismatch in the BEAMME or ELEMENT chunk, returns the
/// `ChunkVersionMismatch` error — the caller should fall back to
/// `number_of_beam_mes = 5`.
pub fn scan_mission_for_beam_mes(path: &str) -> Result<MissionBeamMeScan, LevelError> {
    let file =
        SbFile::open(path, SB_FILE_READ).map_err(|_| LevelError::FileNotFound(path.to_string()))?;
    let mut reader = ChunkReader::new(file);

    let format = {
        let tag = reader.peek_next_chunk()?;
        LevelFormat::detect(&tag)?
    };
    reader.chunk_start(format.mission_tag(), format.file_version())?;

    let mut scan = MissionBeamMeScan::default();

    while !reader.at_end_of_chunk() {
        let tag = reader.peek_next_chunk()?;
        if tag == *format.element_tag() {
            reader.chunk_start(format.element_tag(), format.element_ver())?;
            let num_groups = reader.read_u16()?;
            for _ in 0..num_groups {
                let inner = reader.peek_next_chunk()?;
                if inner == *format.beamme_tag() {
                    let beam_mes = read_beam_mes(&mut reader, format)?;
                    scan.number_of_beam_mes = beam_mes.len() as u16;
                    for bm in beam_mes {
                        scan.action_flags.push(bm.action_required);
                    }
                } else {
                    reader.skip_chunk()?;
                }
            }
            reader.chunk_end()?;
        } else {
            reader.skip_chunk()?;
        }
    }

    reader.chunk_end()?;
    Ok(scan)
}

// ── Targets ───────────────────────────────────────────────────

fn read_targets(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawTarget>, LevelError> {
    reader.chunk_start(format.target_tag(), format.target_ver())?;

    let count = reader.read_u16()?;
    let mut targets = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let position_x = reader.read_i16()?;
        let position_y = reader.read_i16()?;
        let position_z = reader.read_i16()?;
        let direction = reader.read_u32()?;
        let action = reader.read_u32()?;
        let obstacle_index = reader.read_u16()?;
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;

        let filename = reader.read_string()?;
        let profile_name = reader.read_string()?;

        let action_filter = reader.read_u32()?;

        let action_position_x = reader.read_i16()?;
        let action_position_y = reader.read_i16()?;
        let action_sector = reader.read_u16()?;
        let action_layer = reader.read_u16()?;

        // Fullgame has an extra padding byte before the polyline
        if format.is_fullgame() {
            let _ = reader.read_u8()?;
        }

        let polyline_count = reader.read_u16()?;
        let mut polyline = Vec::with_capacity(polyline_count as usize);
        for _ in 0..polyline_count {
            let x = reader.read_i16()?;
            let y = reader.read_i16()?;
            polyline.push((x, y));
        }

        // Fullgame has an extra padding byte after the polyline
        if format.is_fullgame() {
            let _ = reader.read_u8()?;
        }

        let blit_type = reader.read_u8()?;

        // Script data (Toolbox::InitializeScriptFromStream)
        let has_script = reader.read_bool()?;
        let script_class = if has_script {
            Some(reader.read_string()?)
        } else {
            None
        };

        targets.push(RawTarget {
            position_x,
            position_y,
            position_z,
            direction,
            action,
            obstacle_index,
            sector,
            layer,
            filename,
            profile_name,
            action_filter,
            action_position_x,
            action_position_y,
            action_sector,
            action_layer,
            polyline,
            blit_type,
            script_class,
        });
    }

    reader.chunk_end()?;
    Ok(targets)
}

// ── PCs to rescue ──────────────────────────────────────────────

fn read_pcs_to_rescue(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawPcRescue>, LevelError> {
    reader.chunk_start(format.pc_tag(), format.pc_ver())?;

    let count = reader.read_u16()?;
    let mut pcs = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let position_x = reader.read_i16()?;
        let position_y = reader.read_i16()?;
        let direction = reader.read_u32()?;
        let action = reader.read_u32()?;
        let obstacle_index = reader.read_u16()?;
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;
        let material = reader.read_u32()?;
        let profile_index = reader.read_u32()?;
        let attributes = reader.read_u32()?;

        // Script data (Toolbox::InitializeScriptFromStream)
        let has_script = reader.read_bool()?;
        let script_class = if has_script {
            Some(reader.read_string()?)
        } else {
            None
        };

        pcs.push(RawPcRescue {
            position_x,
            position_y,
            direction,
            action,
            obstacle_index,
            sector,
            layer,
            material,
            profile_index,
            allegiance: None,
            autonomous: false,
            aggressive_combat: false,
            playable: false,
            attributes,
            script_class,
        });
    }

    reader.chunk_end()?;
    Ok(pcs)
}

// ── Bonuses ───────────────────────────────────────────────────

fn read_bonuses(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawBonus>, LevelError> {
    reader.chunk_start(format.bonus_tag(), format.bonus_ver())?;

    let count = reader.read_u16()?;
    let mut bonuses = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let bonus_type = reader.read_u16()?;
        let quantity = reader.read_u16()?;
        let position_x = reader.read_u16()?;
        let position_y = reader.read_u16()?;
        let direction = reader.read_u32()?;
        let action = reader.read_u32()?;
        let obstacle_index = reader.read_u16()?;
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;

        bonuses.push(RawBonus {
            bonus_type,
            quantity,
            position_x,
            position_y,
            direction,
            action,
            obstacle_index,
            sector,
            layer,
        });
    }

    reader.chunk_end()?;
    Ok(bonuses)
}

// ── Scrolls ───────────────────────────────────────────────────

fn read_scrolls(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawScroll>, LevelError> {
    reader.chunk_start(format.scroll_tag(), format.scroll_ver())?;

    let count = reader.read_u16()?;
    let mut scrolls = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let position_x = reader.read_u16()?;
        let position_y = reader.read_u16()?;
        let direction = reader.read_u32()?;
        let action = reader.read_u32()?;
        let obstacle_index = reader.read_u16()?;
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;

        // Presence per difficulty level (3 bools = 3 bytes)
        let presence = [
            reader.read_bool()?,
            reader.read_bool()?,
            reader.read_bool()?,
        ];
        let tutorial = reader.read_bool()?;
        let force_visible = reader.read_bool()?;

        // Script data (Toolbox::InitializeScriptFromStream)
        let has_script = reader.read_bool()?;
        let script_class = if has_script {
            Some(reader.read_string()?)
        } else {
            None
        };

        scrolls.push(RawScroll {
            position_x,
            position_y,
            direction,
            action,
            obstacle_index,
            sector,
            layer,
            presence,
            tutorial,
            force_visible,
            script_class,
        });
    }

    reader.chunk_end()?;
    Ok(scrolls)
}

// ── Proto-level chunk readers ───────────────────────────────────────

/// Read MATERIAL/MAT/TEXT chunk.
///
/// Format: u16 count, then per sector: u8 material + sector polygon.
fn read_material_sectors(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawMaterialSector>, LevelError> {
    reader.chunk_start(format.material_tag(), format.material_ver())?;

    let count = reader.read_u16()?;
    let mut sectors = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let material = reader.read_u8()?;
        let polygon = read_sector_polygon(reader, format)?;
        sectors.push(RawMaterialSector { material, polygon });
    }

    reader.chunk_end()?;
    Ok(sectors)
}

/// Read LIGHT/LZ/DARK chunk.
///
/// Format: u16 count, then per sector: u16 layer + sector polygon + u32 ambience.
fn read_light_sectors(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawLightSector>, LevelError> {
    reader.chunk_start(format.light_tag(), format.light_ver())?;

    let count = reader.read_u16()?;
    let mut sectors = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let layer = reader.read_u16()?;
        let polygon = read_sector_polygon(reader, format)?;
        let ambience = reader.read_u32()?;
        sectors.push(RawLightSector {
            layer,
            polygon,
            ambience,
        });
    }

    reader.chunk_end()?;
    Ok(sectors)
}

/// Read BOND/007 chunk (elevation lines).
///
/// Format: u16 count, then per line:
/// - 4 x i16 (point A x,y + point B x,y)
/// - 2 x u16 (right/left obstacle index)
/// - u16 layer
fn read_elevation_lines(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawElevationLine>, LevelError> {
    reader.chunk_start(format.bond_tag(), format.bond_ver())?;

    let count = reader.read_u16()?;
    let mut lines = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let ax = reader.read_i16()?;
        let ay = reader.read_i16()?;
        let bx = reader.read_i16()?;
        let by = reader.read_i16()?;
        let right_obstacle_index = reader.read_u16()?;
        let left_obstacle_index = reader.read_u16()?;
        let layer = reader.read_u16()?;

        lines.push(RawElevationLine {
            point_a: (ax, ay),
            point_b: (bx, by),
            right_obstacle_index,
            left_obstacle_index,
            layer,
        });
    }

    reader.chunk_end()?;
    Ok(lines)
}

/// Read MASK/FACE chunk.
///
/// Format: u16 count, then per mask: u16 layer + mask data.
fn read_masks(reader: &mut ChunkReader, format: LevelFormat) -> Result<Vec<RawMask>, LevelError> {
    reader.chunk_start(format.mask_tag(), format.mask_ver())?;

    let count = reader.read_u16()?;
    let mut masks = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let layer = reader.read_u16()?;
        let mask = read_one_mask(reader, format)?;
        masks.push(RawMask { layer, ..mask });
    }

    reader.chunk_end()?;
    Ok(masks)
}

/// Read a single `RHMask::InitializeFromProtoStream`.
fn read_one_mask(reader: &mut ChunkReader, format: LevelFormat) -> Result<RawMask, LevelError> {
    let mut mask_type = reader.read_u8()?;

    let is_character = (mask_type & MASK_CHARACTER) != 0;
    let is_projectile = (mask_type & MASK_PROJECTILE) != 0;
    let is_obstacle = (mask_type & MASK_OBSTACLE) != 0;

    let character_polyline = if is_character {
        reader.read_padding_if_fullgame(format)?;
        let count = reader.read_u16()?;
        let mut pts = Vec::with_capacity(count as usize);
        for _ in 0..count {
            pts.push((reader.read_i16()?, reader.read_i16()?));
        }
        reader.read_padding_if_fullgame(format)?;
        Some(pts)
    } else {
        None
    };

    let projectile_polyline = if is_projectile {
        reader.read_padding_if_fullgame(format)?;
        let count = reader.read_u16()?;
        let mut pts = Vec::with_capacity(count as usize);
        for _ in 0..count {
            pts.push((reader.read_i16()?, reader.read_i16()?));
        }
        reader.read_padding_if_fullgame(format)?;
        Some(pts)
    } else {
        None
    };

    let box_x = reader.read_i16()?;
    let box_y = reader.read_i16()?;
    let box_w = reader.read_i16()?;
    let box_h = reader.read_i16()?;

    let mask_size = reader.read_u16()?;
    let mask_data = reader.read_bytes(mask_size as usize)?;

    let obstacle_indices = if is_obstacle {
        let count = reader.read_u16()?;
        if count == 0 {
            // An obstacle-typed mask with zero obstacles is malformed level
            // data; clear the projectile bit so the mask still loads but is
            // demoted to non-projectile.
            tracing::warn!(
                "VERBOTEN: Obstacle-mask with no obstacle associated; clearing MASK_PROJECTILE"
            );
            mask_type &= !MASK_PROJECTILE;
        }
        let mut indices = Vec::with_capacity(count as usize);
        for _ in 0..count {
            indices.push(reader.read_u16()?);
        }
        indices
    } else {
        Vec::new()
    };

    Ok(RawMask {
        layer: 0, // filled in by caller
        mask_type,
        character_polyline,
        projectile_polyline,
        box_top_left: (box_x, box_y),
        box_size: (box_w, box_h),
        mask_data,
        obstacle_indices,
    })
}

/// Result of parsing the SIGHT chunk: the obstacle list plus the
/// subset of `material_sectors` indices that the chunk flags as
/// runtime-active.
///
/// Only the materials whose index is listed here get registered into
/// the fast-grid's per-block SECTOR_SOUND buckets at layer 0, so only
/// they participate in spatial material-lookup queries (footstep
/// detection, projectile water/hole impact-material lookup, etc.).
/// Material sectors present in CHUNK_MATERIAL but absent from this list
/// are spatially invisible.
#[derive(Debug, Clone, Default)]
pub struct SightChunk {
    pub obstacles: Vec<RawSightObstacle>,
    pub material_indices: Vec<u16>,
}

/// Read SGHT/WOAW chunk (sight obstacles).
///
/// Format: u16 num_material_indices + indices, then u16 num_obstacles + obstacles.
fn read_sight_obstacles(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<SightChunk, LevelError> {
    reader.chunk_start(format.sight_tag(), format.sight_ver())?;

    // Material sector indices (used at runtime to associate materials)
    let num_material_indices = reader.read_u16()?;
    let mut material_indices = Vec::with_capacity(num_material_indices as usize);
    for _ in 0..num_material_indices {
        material_indices.push(reader.read_u16()?);
    }

    let count = reader.read_u16()?;
    let mut obstacles = Vec::with_capacity(count as usize);

    for _ in 0..count {
        obstacles.push(read_one_sight_obstacle(reader)?);
    }

    reader.chunk_end()?;
    Ok(SightChunk {
        obstacles,
        material_indices,
    })
}

/// Read a single `RHSightObstacle::InitializeFromProtoStream`.
fn read_one_sight_obstacle(reader: &mut ChunkReader) -> Result<RawSightObstacle, LevelError> {
    let num_points = reader.read_u16()?;
    let mut points = Vec::with_capacity(num_points as usize);
    for _ in 0..num_points {
        let x = reader.read_f32()?;
        let y = reader.read_f32()?;
        let mut z_bottom = reader.read_f32()?;
        // Level-editor quirk: snap tiny non-zero z_bottom to 0 so the
        // on_ground flag stays true.
        if z_bottom < 0.1 {
            z_bottom = 0.0;
        }
        let z_top = reader.read_f32()?;
        points.push(RawObstaclePoint {
            x,
            y,
            z_bottom,
            z_top,
        });
    }

    // Bounding box (6 floats: point1 x,y,z + point2 x,y,z) — read for
    // stream alignment but discarded.  The C++ engine also reseeds the
    // box from the per-point Add3DPoint calls; the Rust SightObstacle
    // computes its bounding boxes the same way from `points`.
    let _ = reader.read_f32()?;
    let _ = reader.read_f32()?;
    let _ = reader.read_f32()?;
    let _ = reader.read_f32()?;
    let _ = reader.read_f32()?;
    let _ = reader.read_f32()?;

    // Projection area
    let is_projection = reader.read_bool()?;
    let projection_area = if is_projection {
        Some((reader.read_u16()?, reader.read_u16()?))
    } else {
        None
    };

    let opaque = reader.read_bool()?;
    let solid = reader.read_bool()?;
    let mouse = reader.read_bool()?;
    let show_shadow_polygon = reader.read_bool()?;

    let default_material = reader.read_u8()?;

    let num_materials = reader.read_u16()?;
    let mut material_indices = Vec::with_capacity(num_materials as usize);
    for _ in 0..num_materials {
        material_indices.push(reader.read_u16()?);
    }

    Ok(RawSightObstacle {
        points,
        projection_area,
        opaque,
        solid,
        mouse,
        show_shadow_polygon,
        default_material,
        material_indices,
    })
}

/// Read SND/LOUD chunk (sound sources).
fn read_sound_sources(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawSoundSource>, LevelError> {
    reader.chunk_start(format.sound_tag(), format.sound_ver())?;

    let count = reader.read_u16()?;
    let mut sources = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let id = reader.read_i32()?;
        let active = reader.read_bool()?;
        let source_kind = reader.read_u8()?;

        let delayed_params = if source_kind == SOUND_KIND_DELAYED {
            Some((
                reader.read_u16()?, // min_delay
                reader.read_u16()?, // max_delay
                reader.read_u16()?, // delay_stepping
            ))
        } else {
            None
        };

        let global = reader.read_bool()?;

        let (
            inner_distance,
            outer_distance,
            polyline,
            inner_volume,
            outer_volume,
            noise_covering_distance,
        ) = if !global {
            let inner = reader.read_u16()?;
            let outer = reader.read_u16()?;

            reader.read_padding_if_fullgame(format)?;

            let num_pts = reader.read_u16()?;
            let mut poly = Vec::with_capacity(num_pts as usize);
            for _ in 0..num_pts {
                poly.push((reader.read_i16()?, reader.read_i16()?));
            }

            reader.read_padding_if_fullgame(format)?;

            let inner_vol = reader.read_u16()?;
            let outer_vol = reader.read_u16()?;
            let noise_dist = reader.read_u16()?;

            (
                Some(inner),
                Some(outer),
                Some(poly),
                Some(inner_vol),
                Some(outer_vol),
                Some(noise_dist),
            )
        } else {
            (None, None, None, None, None, None)
        };

        let altitude = reader.read_u8()?;
        let ambience_filter = reader.read_u32()?;

        sources.push(RawSoundSource {
            id,
            active,
            source_kind,
            delayed_params,
            global,
            inner_distance,
            outer_distance,
            polyline,
            inner_volume,
            outer_volume,
            noise_covering_distance,
            altitude,
            ambience_filter,
        });
    }

    reader.chunk_end()?;
    Ok(sources)
}

/// Read JZ/PPPP chunk (jump zones + jump line pairs).
fn read_jump_stuff(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<(Vec<RawJumpZone>, Vec<RawJumpLinePair>), LevelError> {
    reader.chunk_start(format.jump_tag(), format.jump_ver())?;

    // Jump zones
    let num_zones = reader.read_u16()?;
    let mut jump_zones = Vec::with_capacity(num_zones as usize);
    for _ in 0..num_zones {
        let polygon = read_sector_polygon(reader, format)?;
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;
        let helper_needed = reader.read_bool()?;
        jump_zones.push(RawJumpZone {
            polygon,
            sector,
            layer,
            helper_needed,
        });
    }

    // Jump line pairs
    let num_pairs = reader.read_u16()?;
    let mut jump_line_pairs = Vec::with_capacity(num_pairs as usize);
    for _ in 0..num_pairs {
        let line1 = read_one_jump_line(reader)?;
        let line2 = read_one_jump_line(reader)?;
        let jump_long = reader.read_bool()?;
        jump_line_pairs.push(RawJumpLinePair {
            line1,
            line2,
            jump_long,
        });
    }

    reader.chunk_end()?;
    Ok((jump_zones, jump_line_pairs))
}

/// Read a single `RHLineJump::InitializeFromProtoStream`.
fn read_one_jump_line(reader: &mut ChunkReader) -> Result<RawJumpLine, LevelError> {
    let ax = reader.read_i16()?;
    let ay = reader.read_i16()?;
    let az = reader.read_i16()?;
    let bx = reader.read_i16()?;
    let by = reader.read_i16()?;
    let bz = reader.read_i16()?;
    let jump_zone_index = reader.read_u16()?;
    Ok(RawJumpLine {
        point_a: (ax, ay, az),
        point_b: (bx, by, bz),
        jump_zone_index,
    })
}

/// Read a single `RHDoor::InitializeFromProtoStream`.
fn read_one_door(reader: &mut ChunkReader, format: LevelFormat) -> Result<RawDoor, LevelError> {
    let door_type = reader.read_u8()?;
    let active = reader.read_bool()?;
    let locked_pc = reader.read_bool()?;
    let unlockable = reader.read_bool()?;
    let locked_npc_villain = reader.read_bool()?;
    let locked_npc_civilian = reader.read_bool()?;
    let locked_pc_after_patch = reader.read_bool()?;
    let unlockable_after_patch = reader.read_bool()?;
    let locked_npc_villain_after_patch = reader.read_bool()?;
    let locked_npc_civilian_after_patch = reader.read_bool()?;

    // Door clickable sector
    let door_sector = read_sector_polygon(reader, format)?;

    // Out point + sector/layer
    let out_x = reader.read_i16()?;
    let out_y = reader.read_i16()?;
    let sector_out = reader.read_u16()?;
    let layer_out = reader.read_u16()?;

    // Mid point
    let mid_x = reader.read_i16()?;
    let mid_y = reader.read_i16()?;

    // In point + sector/layer
    let in_x = reader.read_i16()?;
    let in_y = reader.read_i16()?;
    let sector_in = reader.read_u16()?;
    let layer_in = reader.read_u16()?;

    Ok(RawDoor {
        door_type,
        active,
        locked_pc,
        unlockable,
        locked_npc_villain,
        locked_npc_civilian,
        locked_pc_after_patch,
        unlockable_after_patch,
        locked_npc_villain_after_patch,
        locked_npc_civilian_after_patch,
        door_sector,
        point_out: (out_x, out_y),
        sector_out,
        layer_out,
        point_mid: (mid_x, mid_y),
        point_in: (in_x, in_y),
        sector_in,
        layer_in,
    })
}

/// Read LIFT/AZ chunk.
fn read_lifts(reader: &mut ChunkReader, format: LevelFormat) -> Result<Vec<RawLift>, LevelError> {
    reader.chunk_start(format.lift_tag(), format.lift_ver())?;

    let count = reader.read_u16()?;
    let mut lifts = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let motion_area_index = reader.read_u16()?;
        // Layer field — read for stream alignment but unused; the
        // motion-area drives layer attribution at runtime.
        let _layer = reader.read_u16()?;

        // RHSectorLift::InitializeFromProtoStream
        let lift_type = reader.read_u8()?;

        // Clickable sector (RHSectorAssociated): parsed for stream
        // alignment but discarded — the C++ `RHSectorLift::Initialize`
        // path that would register this polygon as a mouse-pick sector
        // has its `AddSector(pSectorAssociated, ...)` calls commented
        // out in the C++ source, so the polygon carries no runtime
        // meaning.  See `engine/level_loading.rs::initialize_motion_from_level_data`.
        let _click_sector = read_sector_polygon(reader, format)?;

        // Doors
        let num_doors = reader.read_u16()?;
        let mut doors = Vec::with_capacity(num_doors as usize);
        for _ in 0..num_doors {
            doors.push(read_one_door(reader, format)?);
        }

        let direction = reader.read_i16()?;

        lifts.push(RawLift {
            motion_area_index,
            lift_type,
            doors,
            direction,
        });
    }

    reader.chunk_end()?;
    Ok(lifts)
}

/// Read BUIL/FARM chunk (buildings).
fn read_buildings(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawBuildingEntry>, LevelError> {
    reader.chunk_start(format.building_tag(), format.building_ver())?;

    let count = reader.read_u16()?;
    let mut buildings = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let is_building = reader.read_bool()?;

        if is_building {
            // RHSectorBuilding: u16 num_doors + doors
            let num_doors = reader.read_u16()?;
            let mut doors = Vec::with_capacity(num_doors as usize);
            for _ in 0..num_doors {
                doors.push(read_one_door(reader, format)?);
            }
            buildings.push(RawBuildingEntry::Building { doors });
        } else {
            // Standalone doors
            let num_doors = reader.read_u16()?;
            let mut doors = Vec::with_capacity(num_doors as usize);
            for _ in 0..num_doors {
                doors.push(read_one_door(reader, format)?);
            }
            buildings.push(RawBuildingEntry::StandaloneDoors { doors });
        }
    }

    reader.chunk_end()?;
    Ok(buildings)
}

// ── Mission chunk readers ──────────────────────────────────────────

/// Read SCRP/GULP chunk (script objects: points + sectors).
fn read_script_objects(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<RawScriptObjects, LevelError> {
    reader.chunk_start(format.script_tag(), format.script_ver())?;

    // Script points
    let num_points = reader.read_u16()?;
    let mut points = Vec::with_capacity(num_points as usize);
    for _ in 0..num_points {
        let x = reader.read_i16()?;
        let y = reader.read_i16()?;
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;
        points.push(RawScriptPoint {
            x,
            y,
            sector,
            layer,
        });
    }

    // Script sectors
    let num_sectors = reader.read_u16()?;
    let mut sectors = Vec::with_capacity(num_sectors as usize);
    for _ in 0..num_sectors {
        let polygon = read_sector_polygon(reader, format)?;
        let sector_ref = reader.read_u16()?;
        let layer = reader.read_u16()?;
        let script_associated = reader.read_bool()?;
        let script_class = if script_associated {
            let string_length = reader.read_u16()?;
            let bytes = reader.read_bytes(string_length as usize)?;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        } else {
            None
        };
        sectors.push(RawScriptSector {
            polygon,
            sector_ref,
            layer,
            script_class,
        });
    }

    reader.chunk_end()?;

    tracing::info!(
        "SCRIPT chunk: {} points, {} sectors",
        points.len(),
        sectors.len(),
    );

    // The mission stream chunk only carries points + sectors.  Lines come
    // from the dead old-level script-objects loader, which is commented
    // out; leave the slab empty so the in-memory layout still preserves
    // the `[points][lines][sectors]` ordering.
    let lines = Vec::new();
    Ok(RawScriptObjects {
        points,
        lines,
        sectors,
    })
}

/// Read GUYS/CAVE chunk (building tenants).
///
/// Format: u16 num_buildings, then per building:
/// - u16 num_tenants + u16 element_index per tenant
/// - bool arrow_reserve
fn read_building_tenants(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawBuildingTenants>, LevelError> {
    reader.chunk_start(format.tenant_tag(), format.tenant_ver())?;

    let count = reader.read_u16()?;
    let mut tenants = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let num_tenants = reader.read_u16()?;
        let mut tenant_element_indices = Vec::with_capacity(num_tenants as usize);
        for _ in 0..num_tenants {
            tenant_element_indices.push(reader.read_u16()?);
        }
        let arrow_reserve = reader.read_bool()?;
        tenants.push(RawBuildingTenants {
            tenant_element_indices,
            arrow_reserve,
        });
    }

    reader.chunk_end()?;
    Ok(tenants)
}

/// Read the CART/TING chunk as `RHElementMobile::InitializeFromMissionStream`.
///
/// All released missions use the same narrow subset: one masked cart sprite,
/// no local sight obstacles, a motion polygon, and a hiking-path reference.
/// Unsupported Spellbound-engine variants fail explicitly so they cannot be
/// mistaken for parity with shipped Robin Hood data.
fn read_mobile_elements(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawMobileElement>, LevelError> {
    reader.chunk_start(format.mobile_tag(), format.mobile_ver())?;
    let count = reader.read_u16()?;
    let mut elements = Vec::with_capacity(count as usize);

    for mobile_index in 0..count {
        reader.chunk_start(format.animation_tag(), format.animation_ver())?;
        let sprite_count = reader.read_u16()?;
        let mut sprites = Vec::with_capacity(sprite_count as usize);
        for _ in 0..sprite_count {
            sprites.push(read_element_fx(reader, format)?);
        }
        reader.chunk_end()?;

        reader.chunk_start(format.sight_tag(), format.sight_ver())?;
        let sight_count = reader.read_u16()?;
        if sight_count != 0 {
            return Err(LevelError::UnsupportedMobileData(format!(
                "mobile {mobile_index} embeds {sight_count} sight obstacles; released missions embed none"
            )));
        }
        reader.chunk_end()?;

        let motion_polygon = read_sector_polygon(reader, format)?;
        let editor_anchor = (reader.read_i16()?, reader.read_i16()?);
        let hook_point = if reader.read_bool()? {
            Some((reader.read_i16()?, reader.read_i16()?))
        } else {
            None
        };
        let barrel_point = if reader.read_bool()? {
            Some((reader.read_i16()?, reader.read_i16()?))
        } else {
            None
        };
        let path_index = reader.read_u16()?;
        let start_waypoint = reader.read_u32()?;
        let stop_waypoint = reader.read_i32()?;
        let obstacle_index = reader.read_u16()?;

        if sprites.len() != 1 {
            return Err(LevelError::UnsupportedMobileData(format!(
                "mobile {mobile_index} has {} sprites; every released cart has exactly one",
                sprites.len()
            )));
        }
        if hook_point.is_some() || barrel_point.is_some() {
            return Err(LevelError::UnsupportedMobileData(format!(
                "mobile {mobile_index} is hookable or a barrel; released missions only use chariots"
            )));
        }
        if obstacle_index != u16::MAX {
            return Err(LevelError::UnsupportedMobileData(format!(
                "mobile {mobile_index} references projection obstacle {obstacle_index}; released chariots use 0xffff"
            )));
        }

        elements.push(RawMobileElement {
            sprites,
            motion_polygon,
            editor_anchor,
            hook_point,
            barrel_point,
            path_index,
            start_waypoint,
            stop_waypoint,
            obstacle_index,
        });
    }

    reader.chunk_end()?;
    Ok(elements)
}

/// Read PWAY/RAIL chunk (hiking/patrol paths).
///
/// Format:
/// - u16 num_paths
/// - Per path: u16 num_waypoints, then per waypoint:
///   - i16 x, i16 y, u16 sector, u16 level
///   - bool command_is_script, u16 size_of_data
///   - If size_of_data > 0: script string or raw macro bytes
fn read_hiking_paths(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawHikingPath>, LevelError> {
    reader.chunk_start(format.path_tag(), format.path_ver())?;

    let num_paths = reader.read_u16()?;
    let mut paths = Vec::with_capacity(num_paths as usize);

    for _ in 0..num_paths {
        let num_waypoints = reader.read_u16()?;
        let mut waypoints = Vec::with_capacity(num_waypoints as usize);

        for _ in 0..num_waypoints {
            let x = reader.read_i16()?;
            let y = reader.read_i16()?;
            let sector = reader.read_u16()?;
            let level = reader.read_u16()?;
            let command_is_script = reader.read_bool()?;
            let size_of_data = reader.read_u16()?;

            let command = if size_of_data > 0 {
                let data = reader.read_bytes(size_of_data as usize)?;
                if command_is_script {
                    // The on-disk form is null-terminated raw bytes.
                    let s = String::from_utf8_lossy(&data)
                        .trim_end_matches('\0')
                        .to_owned();
                    WaypointCommand::Script(s)
                } else {
                    WaypointCommand::Macro(data)
                }
            } else {
                WaypointCommand::None
            };

            waypoints.push(RawWaypoint {
                x,
                y,
                sector,
                level,
                command,
            });
        }

        paths.push(RawHikingPath { waypoints });
    }

    reader.chunk_end()?;
    Ok(paths)
}

// ── TACTIC chunk reader ─────────────────────────────────────────────

/// Read the TACTIC (AI /HIRN) chunk containing AI tactical data.
fn read_tactic_data(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<RawTacticData, LevelError> {
    reader.chunk_start(format.tactic_tag(), format.tactic_ver())?;

    let num_groups = reader.read_u16()?;

    let mut reinforcement_points = Vec::new();
    let mut ambush_points = Vec::new();
    let mut seek_points = Vec::new();
    let mut archery_sectors = Vec::new();

    for _ in 0..num_groups {
        let tag = reader.peek_next_chunk()?;

        if tag == *format.reinforcement_tag() {
            reinforcement_points = read_reinforcement_points(reader, format)?;
        } else if tag == *format.ambush_tag() {
            ambush_points = read_ambush_points(reader, format)?;
        } else if tag == *format.search_tag() {
            seek_points = read_seek_points(reader, format)?;
        } else if tag == *format.archery_tag() {
            archery_sectors = read_archery_sectors(reader, format)?;
        } else {
            let name = tag_str(&tag);
            tracing::warn!("TACTIC: skipping unknown sub-chunk '{}'", name);
            reader.skip_chunk()?;
        }
    }

    reader.chunk_end()?;

    tracing::info!(
        "TACTIC chunk: {} reinforcements, {} ambushes, {} seeks, {} archery sectors",
        reinforcement_points.len(),
        ambush_points.len(),
        seek_points.len(),
        archery_sectors.len(),
    );

    Ok(RawTacticData {
        reinforcement_points,
        ambush_points,
        seek_points,
        archery_sectors,
    })
}

/// Read reinforcement points from the REIN/POW sub-chunk.
fn read_reinforcement_points(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawReinforcementPoint>, LevelError> {
    reader.chunk_start(format.reinforcement_tag(), format.reinforcement_ver())?;

    let count = reader.read_u16()?;
    let mut points = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let x = reader.read_i16()?;
        let y = reader.read_i16()?;
        let direction = reader.read_u32()?;
        let action = reader.read_u32()?;
        let obstacle_index = reader.read_u16()?;
        let sector = reader.read_u16()?;
        let layer = reader.read_u16()?;

        points.push(RawReinforcementPoint {
            x,
            y,
            direction,
            action,
            obstacle_index,
            sector,
            layer,
        });
    }

    reader.chunk_end()?;
    Ok(points)
}

/// Read ambush points from the AMBU/BUSH sub-chunk.
fn read_ambush_points(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawAmbushPoint>, LevelError> {
    reader.chunk_start(format.ambush_tag(), format.ambush_ver())?;

    let count = reader.read_u16()?;
    let mut points = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let x = reader.read_i16()?;
        let y = reader.read_i16()?;
        let sector = reader.read_u16()?;
        let level = reader.read_u16()?;

        points.push(RawAmbushPoint {
            x,
            y,
            sector,
            level,
        });
    }

    reader.chunk_end()?;
    Ok(points)
}

/// Read seek/search points from the SEAR/HOLE sub-chunk.
fn read_seek_points(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawSeekPoint>, LevelError> {
    reader.chunk_start(format.search_tag(), format.search_ver())?;

    let count = reader.read_u16()?;
    let mut points = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let x = reader.read_i16()?;
        let y = reader.read_i16()?;
        let sector = reader.read_u16()?;
        let level = reader.read_u16()?;
        let direction = reader.read_u16()?;

        points.push(RawSeekPoint {
            x,
            y,
            sector,
            level,
            direction,
        });
    }

    reader.chunk_end()?;
    Ok(points)
}

/// Read archery sectors from the ARCH/NLIP sub-chunk.
fn read_archery_sectors(
    reader: &mut ChunkReader,
    format: LevelFormat,
) -> Result<Vec<RawArcherySector>, LevelError> {
    reader.chunk_start(format.archery_tag(), format.archery_ver())?;

    let count = reader.read_u16()?;
    let mut sectors = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let sector_ref = reader.read_u16()?;
        let _dummy = reader.read_u16()?; // unused dummy field

        let polygon = read_sector_polygon(reader, format)?;

        let num_points = reader.read_u16()?;
        let mut points = Vec::with_capacity(num_points as usize);

        for _ in 0..num_points {
            let x = reader.read_u16()?;
            let y = reader.read_u16()?;
            let sector = reader.read_u16()?;
            let _dummy = reader.read_u16()?; // unused dummy field
            let is_shooting_point = reader.read_bool()?;
            let direction = reader.read_u16()?;

            points.push(RawArcheryPoint {
                x,
                y,
                sector,
                is_shooting_point,
                direction,
            });
        }

        sectors.push(RawArcherySector {
            sector_ref,
            polygon,
            points,
        });
    }

    reader.chunk_end()?;
    Ok(sectors)
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn hackable_descriptor_expands_to_playable_level() {
        let level = LoadedLevel::hackable_from_json(
            br#"{
                "title": "Test Castle",
                "map_filename": "TestMap",
                "spawn": [50, 60],
                "walkable_polygon": [[0, 0], [100, 0], [100, 100], [0, 100]],
                "volumes": [
                    {
                        "name": "keep",
                        "footprint": [[20, 20], [40, 20], [40, 40], [20, 40]],
                        "z_bottom": 0.0,
                        "z_top": 300.0,
                        "motion_blocking": true,
                        "opaque": true
                    }
                ]
            }"#,
        )
        .expect("hackable descriptor");
        let motion = level.proto.motion_data.expect("hackable motion data");
        assert_eq!(motion.layers.len(), 2);
        assert_eq!(motion.layers[0].len(), 1);
        assert!(motion.layers[0][0].polygon.points.len() >= 3);
        assert_eq!(level.mission.header.map_filename, "TestMap");
        assert_eq!(level.mission.beam_mes.len(), 1);
        assert_eq!(level.mission.beam_mes[0].sector, 0);
        assert_eq!(level.mission.beam_mes[0].layer, 0);
        assert!(!motion.layers[0][0].obstacles.is_empty());
        assert!(!level.proto.sight_obstacles.is_empty());
        assert!(level.proto.masks.is_empty());
    }

    #[test]
    fn hackable_descriptor_rejects_degenerate_geometry() {
        let error = LoadedLevel::hackable_from_json(
            br#"{
                "map_filename": "TestMap",
                "spawn": [0, 0],
                "walkable_polygon": [[0, 0], [100, 0]]
            }"#,
        )
        .expect_err("two-point walkable polygon must be rejected");
        assert!(error.contains("walkable_polygon"));
    }

    #[test]
    fn hackable_descriptor_authors_multiple_soldier_and_pc_allegiances() {
        let level = LoadedLevel::hackable_from_json(
            br#"{
                "map_filename": "Arena",
                "spawn": [50, 50],
                "spawn_player": false,
                "walkable_polygon": [[0, 0], [100, 0], [100, 100]],
                "soldiers": [
                    {"position": [20, 20], "profile": 0, "allegiance": 2},
                    {"position": [80, 20], "profile": 1, "allegiance": 9}
                ],
                "pcs": [
                    {"position": [50, 80], "profile": 2, "allegiance": 7, "autonomous": true, "aggressive_combat": true}
                ]
            }"#,
        )
        .expect("multi-team hackable descriptor");
        assert!(level.mission.reserve_null_ai_handle);
        assert!(level.mission.beam_mes.is_empty());
        assert_eq!(level.mission.soldiers.len(), 2);
        assert_eq!(level.mission.soldiers[0].allegiance, Some(2));
        assert_eq!(level.mission.soldiers[1].allegiance, Some(9));
        assert_eq!(level.mission.pcs_to_rescue[0].allegiance, Some(7));
        assert!(level.mission.pcs_to_rescue[0].autonomous);
        assert!(level.mission.pcs_to_rescue[0].aggressive_combat);
    }

    #[test]
    fn bundled_multi_team_missions_parse_and_cover_requested_rosters() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cases = [
            ("multi-team-three-way", "MultiTeamThreeWay", 3, 0),
            ("multi-team-ten-way", "MultiTeamTenWay", 10, 0),
            ("multi-team-four-armies", "MultiTeamFourArmies", 48, 0),
            ("multi-team-four-grades", "MultiTeamFourGrades", 48, 0),
            ("multi-team-all-pcs-circle", "MultiTeamAllPcsCircle", 0, 10),
            (
                "multi-team-arrow-crossfire",
                "MultiTeamArrowCrossfire",
                32,
                0,
            ),
            ("multi-team-twenty-robins", "MultiTeamTwentyRobins", 20, 20),
            (
                "multi-team-champions-retinues",
                "MultiTeamChampionsRetinues",
                16,
                4,
            ),
            (
                "multi-team-all-variants-wheel",
                "MultiTeamAllVariantsWheel",
                68,
                10,
            ),
            (
                "multi-team-robin-vs-little-john",
                "MultiTeamRobinVsLittleJohn",
                0,
                2,
            ),
        ];
        for (slug, mission, soldiers, pcs) in cases {
            let path = repo
                .join("mods")
                .join(slug)
                .join("Data/Levels")
                .join(format!("{mission}.level.json"));
            let bytes = fs::read(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            let level = LoadedLevel::hackable_from_json(&bytes)
                .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()));
            assert_ne!(level.mission.header.map_filename, "Dover", "{mission}");
            assert_eq!(level.mission.soldiers.len(), soldiers, "{mission}");
            assert_eq!(level.mission.pcs_to_rescue.len(), pcs, "{mission}");
            assert!(
                level
                    .mission
                    .soldiers
                    .iter()
                    .all(|soldier| soldier.revealed),
                "{mission} must reveal its complete test roster"
            );
        }
        let duel_path = repo.join(
            "mods/multi-team-robin-vs-little-john/Data/Levels/MultiTeamRobinVsLittleJohn.level.json",
        );
        let duel = LoadedLevel::hackable_from_json(&fs::read(duel_path).unwrap()).unwrap();
        assert!(duel.mission.pcs_to_rescue.iter().all(|pc| pc.autonomous));
        assert!(
            duel.mission
                .pcs_to_rescue
                .iter()
                .all(|pc| pc.aggressive_combat)
        );
        assert_eq!(duel.mission.pcs_to_rescue[0].profile_index, 0);
        assert_eq!(duel.mission.pcs_to_rescue[1].profile_index, 2);

        let pc_circle_path = repo
            .join("mods/multi-team-all-pcs-circle/Data/Levels/MultiTeamAllPcsCircle.level.json");
        let pc_circle =
            LoadedLevel::hackable_from_json(&fs::read(pc_circle_path).unwrap()).unwrap();
        assert!(
            pc_circle
                .mission
                .pcs_to_rescue
                .iter()
                .all(|pc| pc.autonomous && pc.aggressive_combat)
        );
        let pc_profiles: std::collections::BTreeSet<_> = pc_circle
            .mission
            .pcs_to_rescue
            .iter()
            .map(|pc| pc.profile_index)
            .collect();
        let pc_allegiances: std::collections::BTreeSet<_> = pc_circle
            .mission
            .pcs_to_rescue
            .iter()
            .map(|pc| pc.allegiance)
            .collect();
        assert_eq!(pc_profiles, (0..10).collect());
        assert_eq!(pc_allegiances.len(), 10);

        let robins_path =
            repo.join("mods/multi-team-twenty-robins/Data/Levels/MultiTeamTwentyRobins.level.json");
        let robins = LoadedLevel::hackable_from_json(&fs::read(robins_path).unwrap()).unwrap();
        assert!(robins.mission.soldiers.iter().all(|soldier| {
            soldier.allegiance == Some(3) && soldier.profile_id.as_deref() == Some("soldier_b04")
        }));
        assert!(robins.mission.pcs_to_rescue.iter().all(|pc| {
            pc.allegiance == Some(2)
                && pc.profile_index == 0
                && pc.autonomous
                && pc.aggressive_combat
        }));

        let armies_path =
            repo.join("mods/multi-team-four-armies/Data/Levels/MultiTeamFourArmies.level.json");
        let armies = LoadedLevel::hackable_from_json(&fs::read(armies_path).unwrap()).unwrap();
        assert_eq!(armies.mission.header.map_filename, "OpenBattlefield");
        for asset in ["OpenBattlefield.map.png", "OpenBattlefield.min.png"] {
            assert!(
                repo.join("mods/multi-team-four-armies/Data/Levels/Day")
                    .join(asset)
                    .is_file(),
                "missing generated battlefield asset {asset}"
            );
        }
        for allegiance in 2..=5 {
            let faction: Vec<_> = armies
                .mission
                .soldiers
                .iter()
                .filter(|soldier| soldier.allegiance == Some(allegiance))
                .collect();
            assert_eq!(faction.len(), 12, "allegiance {allegiance}");
            for profile_prefix in ["guard_a", "soldier_a", "archer", "officier_b"] {
                assert_eq!(
                    faction
                        .iter()
                        .filter(|soldier| {
                            soldier
                                .profile_id
                                .as_deref()
                                .is_some_and(|profile| profile.starts_with(profile_prefix))
                        })
                        .count(),
                    3,
                    "allegiance {allegiance}, profile family {profile_prefix}"
                );
            }
        }

        let grades_path =
            repo.join("mods/multi-team-four-grades/Data/Levels/MultiTeamFourGrades.level.json");
        let grades = LoadedLevel::hackable_from_json(&fs::read(grades_path).unwrap()).unwrap();
        assert_eq!(grades.mission.header.map_filename, "OpenBattlefield");
        for asset in ["OpenBattlefield.map.png", "OpenBattlefield.min.png"] {
            assert!(
                repo.join("mods/multi-team-four-grades/Data/Levels/Day")
                    .join(asset)
                    .is_file(),
                "missing generated battlefield asset {asset}"
            );
        }
        for (allegiance, grade) in [(2, "01"), (3, "02"), (4, "03"), (5, "04")] {
            let faction: Vec<_> = grades
                .mission
                .soldiers
                .iter()
                .filter(|soldier| soldier.allegiance == Some(allegiance))
                .collect();
            assert_eq!(faction.len(), 12, "allegiance {allegiance}");
            for profile in [
                format!("guard_a{grade}"),
                format!("soldier_a{grade}"),
                format!("archer{grade}"),
                format!("officier_b{grade}"),
            ] {
                assert_eq!(
                    faction
                        .iter()
                        .filter(|soldier| soldier.profile_id.as_deref() == Some(profile.as_str()))
                        .count(),
                    3,
                    "allegiance {allegiance}, profile {profile}"
                );
            }
        }
    }

    /// Build a chunk: tag(4) + size(4) + version(4) + payload.
    /// Size = 4 (version) + payload.len().
    fn build_chunk(tag: &[u8; 4], version: u32, payload: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(tag);
        let size = (payload.len() as u32) + 4; // includes version field
        data.extend_from_slice(&size.to_le_bytes());
        data.extend_from_slice(&version.to_le_bytes());
        data.extend_from_slice(payload);
        data
    }

    /// Build a serialized string: u16 len + bytes.
    fn build_string(s: &str) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&(s.len() as u16).to_le_bytes());
        data.extend_from_slice(s.as_bytes());
        data
    }

    fn write_temp_file(name: &str, data: &[u8]) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        fs::write(&path, data).unwrap();
        let path_str = path.to_str().unwrap().to_string();
        (dir, path_str)
    }

    #[test]
    fn chunk_reader_start_end() {
        // Build a simple chunk: tag="TEST", version=1, payload=4 bytes (u32)
        let payload = 42u32.to_le_bytes();
        let data = build_chunk(b"TEST", 1, &payload);
        let (_dir, path) = write_temp_file("chunk.bin", &data);

        let file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut reader = ChunkReader::new(file);

        reader.chunk_start(b"TEST", 1).unwrap();
        let val = reader.read_u32().unwrap();
        assert_eq!(val, 42);
        reader.chunk_end().unwrap();
    }

    #[test]
    fn chunk_reader_tag_mismatch() {
        let data = build_chunk(b"AAAA", 1, &[]);
        let (_dir, path) = write_temp_file("mismatch.bin", &data);

        let file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut reader = ChunkReader::new(file);

        let err = reader.chunk_start(b"BBBB", 1).unwrap_err();
        assert!(matches!(err, LevelError::ChunkTagMismatch { .. }));
    }

    #[test]
    fn chunk_reader_version_mismatch() {
        let data = build_chunk(b"TEST", 5, &[]);
        let (_dir, path) = write_temp_file("vermis.bin", &data);

        let file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut reader = ChunkReader::new(file);

        let err = reader.chunk_start(b"TEST", 3).unwrap_err();
        assert!(matches!(err, LevelError::ChunkVersionMismatch { .. }));
    }

    #[test]
    fn chunk_reader_peek_and_skip() {
        // Two sequential chunks
        let chunk1 = build_chunk(b"AAA1", 1, &99u32.to_le_bytes());
        let chunk2 = build_chunk(b"BBB2", 2, &77u32.to_le_bytes());
        let mut data = Vec::new();
        data.extend_from_slice(&chunk1);
        data.extend_from_slice(&chunk2);

        // Wrap in outer chunk
        let outer = build_chunk(b"ROOT", 1, &data);
        let (_dir, path) = write_temp_file("peek.bin", &outer);

        let file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut reader = ChunkReader::new(file);

        reader.chunk_start(b"ROOT", 1).unwrap();

        // Peek first chunk
        let tag = reader.peek_next_chunk().unwrap();
        assert_eq!(&tag, b"AAA1");

        // Skip it
        reader.skip_chunk().unwrap();

        // Peek second chunk
        let tag = reader.peek_next_chunk().unwrap();
        assert_eq!(&tag, b"BBB2");

        // Read it properly
        reader.chunk_start(b"BBB2", 2).unwrap();
        assert_eq!(reader.read_u32().unwrap(), 77);
        reader.chunk_end().unwrap();

        assert!(reader.at_end_of_chunk());
        reader.chunk_end().unwrap();
    }

    #[test]
    fn proto_element_chunk_order_is_preserved() {
        let format = LevelFormat::Demo;
        let empty_count = 0u16.to_le_bytes();

        for expected in [
            vec![ProtoElementChunk::Animation, ProtoElementChunk::Patch],
            vec![ProtoElementChunk::Patch, ProtoElementChunk::Animation],
        ] {
            let mut payload = Vec::new();
            for chunk in &expected {
                let (tag, version) = match chunk {
                    ProtoElementChunk::Animation => {
                        (format.animation_tag(), format.animation_ver())
                    }
                    ProtoElementChunk::Patch => (format.patch_tag(), format.patch_ver()),
                };
                payload.extend_from_slice(&build_chunk(tag, version, &empty_count));
            }

            let proto = build_chunk(format.proto_tag(), format.file_version(), &payload);
            let (_dir, path) = write_temp_file("proto-order.rhp", &proto);
            let file = SbFile::open(&path, SB_FILE_READ).unwrap();
            let mut reader = ChunkReader::new(file);

            let loaded = load_proto_level(&mut reader, format).unwrap();
            assert_eq!(loaded.element_chunk_order, expected);
        }
    }

    #[test]
    fn parse_mission_header() {
        let format = LevelFormat::Demo;

        // Build header chunk payload: crc(4) + ambiance(4) + string + profile_id(4)
        let mut payload = Vec::new();
        payload.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes()); // CRC
        payload.extend_from_slice(&2u32.to_le_bytes()); // ambiance = NIGHT
        payload.extend_from_slice(&build_string("testmap")); // map filename
        payload.extend_from_slice(&7u32.to_le_bytes()); // profile ID

        let header_chunk = build_chunk(format.header_tag(), format.header_ver(), &payload);

        // Wrap in outer mission chunk
        let outer = build_chunk(format.mission_tag(), format.file_version(), &header_chunk);
        let (_dir, path) = write_temp_file("mission.bin", &outer);

        let file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut reader = ChunkReader::new(file);

        let mission = load_mission(&mut reader, format, &|_| false).unwrap();

        assert_eq!(mission.header.control_crc, 0xDEAD_BEEF);
        assert_eq!(mission.header.ambiance, 2);
        assert_eq!(mission.header.map_filename, "testmap");
        assert_eq!(mission.header.mission_profile_id, 7);
    }

    #[test]
    fn parse_soldiers() {
        let format = LevelFormat::Demo;

        // Build one soldier's binary data
        let mut soldier_data = Vec::new();
        soldier_data.extend_from_slice(&100u16.to_le_bytes()); // pos_x
        soldier_data.extend_from_slice(&200u16.to_le_bytes()); // pos_y
        soldier_data.extend_from_slice(&4u32.to_le_bytes()); // direction
        soldier_data.extend_from_slice(&1u32.to_le_bytes()); // action
        soldier_data.extend_from_slice(&0xFFFFu16.to_le_bytes()); // obstacle (none)
        soldier_data.extend_from_slice(&5u16.to_le_bytes()); // sector
        soldier_data.extend_from_slice(&0u16.to_le_bytes()); // layer
        soldier_data.extend_from_slice(&0u32.to_le_bytes()); // material
        soldier_data.extend_from_slice(&3u32.to_le_bytes()); // profile
        soldier_data.push(0); // tower_guard = false
        soldier_data.extend_from_slice(&1u32.to_le_bytes()); // company
        soldier_data.extend_from_slice(&0u32.to_le_bytes()); // drunk_level
        soldier_data.extend_from_slice(&50u32.to_le_bytes()); // money
        soldier_data.extend_from_slice(&0u16.to_le_bytes()); // 0 subordinates
        soldier_data.extend_from_slice(&10u16.to_le_bytes()); // path_id
        soldier_data.extend_from_slice(&11u16.to_le_bytes()); // alert_path_id
        soldier_data.push(0); // script_bound = false

        // Wrap: count(2) + soldier data
        let mut elem_payload = Vec::new();
        elem_payload.extend_from_slice(&1u16.to_le_bytes()); // count = 1
        elem_payload.extend_from_slice(&soldier_data);

        let soldier_chunk = build_chunk(format.soldier_tag(), format.soldier_ver(), &elem_payload);

        // ELEM chunk: num_groups(2) + soldier sub-chunk
        let mut group_payload = Vec::new();
        group_payload.extend_from_slice(&1u16.to_le_bytes()); // 1 group
        group_payload.extend_from_slice(&soldier_chunk);

        let elem_chunk = build_chunk(format.element_tag(), format.element_ver(), &group_payload);

        // Header chunk
        let mut header_payload = Vec::new();
        header_payload.extend_from_slice(&0u32.to_le_bytes()); // CRC
        header_payload.extend_from_slice(&0u32.to_le_bytes()); // ambiance
        header_payload.extend_from_slice(&build_string("map"));
        header_payload.extend_from_slice(&0u32.to_le_bytes()); // profile ID

        let header_chunk = build_chunk(format.header_tag(), format.header_ver(), &header_payload);

        // Outer mission chunk
        let mut mission_payload = Vec::new();
        mission_payload.extend_from_slice(&header_chunk);
        mission_payload.extend_from_slice(&elem_chunk);

        let outer = build_chunk(
            format.mission_tag(),
            format.file_version(),
            &mission_payload,
        );

        let (_dir, path) = write_temp_file("soldiers.bin", &outer);
        let file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut reader = ChunkReader::new(file);

        let mission = load_mission(&mut reader, format, &|_| false).unwrap();

        assert_eq!(mission.soldiers.len(), 1);
        let s = &mission.soldiers[0];
        assert_eq!(s.position_x, 100);
        assert_eq!(s.position_y, 200);
        assert_eq!(s.profile_number, 3);
        assert_eq!(s.money, 50);
        assert_eq!(s.path_id, 10);
        assert_eq!(s.alert_path_id, 11);
        assert!(!s.tower_guard);
        assert!(s.subordinate_ids.is_empty());
    }

    #[test]
    fn parse_beam_me() {
        let format = LevelFormat::Demo;

        // Build one beam-me's binary data
        let mut bm_data = Vec::new();
        bm_data.extend_from_slice(&500i16.to_le_bytes()); // pos_x
        bm_data.extend_from_slice(&600i16.to_le_bytes()); // pos_y
        bm_data.extend_from_slice(&8u32.to_le_bytes()); // direction
        bm_data.extend_from_slice(&0u32.to_le_bytes()); // action
        bm_data.extend_from_slice(&0u16.to_le_bytes()); // projection_area
        bm_data.extend_from_slice(&3u16.to_le_bytes()); // sector
        bm_data.extend_from_slice(&0u16.to_le_bytes()); // layer
        bm_data.extend_from_slice(&0u32.to_le_bytes()); // material
        // 10 action flags: climb=true, rest=false
        bm_data.push(1); // climb
        bm_data.extend([0u8; 9]);
        bm_data.push(0); // not scripted
        bm_data.push(2); // required_pc

        let mut elem_payload = Vec::new();
        elem_payload.extend_from_slice(&1u16.to_le_bytes());
        elem_payload.extend_from_slice(&bm_data);

        let bm_chunk = build_chunk(format.beamme_tag(), format.beamme_ver(), &elem_payload);

        let mut group_payload = Vec::new();
        group_payload.extend_from_slice(&1u16.to_le_bytes());
        group_payload.extend_from_slice(&bm_chunk);

        let elem_chunk = build_chunk(format.element_tag(), format.element_ver(), &group_payload);

        let mut header_payload = Vec::new();
        header_payload.extend_from_slice(&0u32.to_le_bytes());
        header_payload.extend_from_slice(&0u32.to_le_bytes());
        header_payload.extend_from_slice(&build_string("m"));
        header_payload.extend_from_slice(&0u32.to_le_bytes());

        let header_chunk = build_chunk(format.header_tag(), format.header_ver(), &header_payload);

        let mut mission_payload = Vec::new();
        mission_payload.extend_from_slice(&header_chunk);
        mission_payload.extend_from_slice(&elem_chunk);

        let outer = build_chunk(
            format.mission_tag(),
            format.file_version(),
            &mission_payload,
        );

        let (_dir, path) = write_temp_file("beamme.bin", &outer);
        let file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut reader = ChunkReader::new(file);

        let mission = load_mission(&mut reader, format, &|_| false).unwrap();

        assert_eq!(mission.beam_mes.len(), 1);
        let bm = &mission.beam_mes[0];
        assert_eq!(bm.position.x, 500.0);
        assert_eq!(bm.position.y, 600.0);
        assert_eq!(bm.direction, 8);
        assert_eq!(bm.sector, 3);
        assert!(bm.action_required.climb);
        assert!(!bm.action_required.jump);
        assert!(bm.script.is_none());
        assert_eq!(bm.required_pc, 2);
        assert_eq!(bm.index, 0);
    }

    #[test]
    fn parse_bonuses() {
        let format = LevelFormat::Demo;

        let mut bonus_data = Vec::new();
        bonus_data.extend_from_slice(&3u16.to_le_bytes()); // type
        bonus_data.extend_from_slice(&5u16.to_le_bytes()); // quantity
        bonus_data.extend_from_slice(&100u16.to_le_bytes()); // pos_x
        bonus_data.extend_from_slice(&200u16.to_le_bytes()); // pos_y
        bonus_data.extend_from_slice(&0u32.to_le_bytes()); // direction
        bonus_data.extend_from_slice(&0u32.to_le_bytes()); // action
        bonus_data.extend_from_slice(&0xFFFFu16.to_le_bytes()); // obstacle
        bonus_data.extend_from_slice(&1u16.to_le_bytes()); // sector
        bonus_data.extend_from_slice(&0u16.to_le_bytes()); // layer

        let mut payload = Vec::new();
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&bonus_data);

        let bonus_chunk = build_chunk(format.bonus_tag(), format.bonus_ver(), &payload);

        let mut header_payload = Vec::new();
        header_payload.extend_from_slice(&0u32.to_le_bytes());
        header_payload.extend_from_slice(&0u32.to_le_bytes());
        header_payload.extend_from_slice(&build_string("m"));
        header_payload.extend_from_slice(&0u32.to_le_bytes());

        let header_chunk = build_chunk(format.header_tag(), format.header_ver(), &header_payload);

        let mut mission_payload = Vec::new();
        mission_payload.extend_from_slice(&header_chunk);
        mission_payload.extend_from_slice(&bonus_chunk);

        let outer = build_chunk(
            format.mission_tag(),
            format.file_version(),
            &mission_payload,
        );

        let (_dir, path) = write_temp_file("bonus.bin", &outer);
        let file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut reader = ChunkReader::new(file);

        let mission = load_mission(&mut reader, format, &|_| false).unwrap();

        assert_eq!(mission.bonuses.len(), 1);
        assert_eq!(mission.bonuses[0].bonus_type, 3);
        assert_eq!(mission.bonuses[0].quantity, 5);
        assert_eq!(mission.bonuses[0].position_x, 100);
    }

    #[test]
    fn level_format_detect() {
        assert_eq!(LevelFormat::detect(b"RHPL").unwrap(), LevelFormat::Demo);
        assert_eq!(LevelFormat::detect(b"MEUH").unwrap(), LevelFormat::Fullgame);
        assert!(LevelFormat::detect(b"XXXX").is_err());
    }
}

//! Sprite animation script loader and cache.
//!
//! Loads `.rhs` (Robin Hood Sprite) binary files, parses named profiles
//! within them, and caches the result keyed by `filename + profile_name`.
//!
//! Each profile contains a set of animation rows ([`SpriteScript`]), where
//! each row is a sequence of frames with per-frame delay, distance, offset,
//! and sound data.  A conversion table maps action IDs ([`OrderType`]
//! discriminants) to row indices.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::coordinates::{SpriteAnchor, SpriteFrameOffset, SpriteLocalPoint, SpriteSize};
use crate::order::OrderType;
use crate::sbfile::SbFile;

// ---------------------------------------------------------------------------
// FrameKind
// ---------------------------------------------------------------------------

/// Describes the kind of sprite for directory resolution when loading.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum FrameKind {
    Character,
    CharacterBlipped,
    Animation,
    Object,
}

// ---------------------------------------------------------------------------
// Ambiance
// ---------------------------------------------------------------------------

/// Ambiance mode, used to pick the correct animation sub-directory.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum Ambiance {
    Day = 1,
    Fog = 2,
    Night = 4,
    Attack = 8,
    Custom1 = 16,
    Custom2 = 32,
    Custom3 = 64,
    Custom4 = 128,
}

impl Ambiance {
    /// Sub-directory path fragment for this ambiance.
    pub fn directory_suffix(self) -> &'static str {
        match self {
            Ambiance::Day => "/Day/",
            Ambiance::Fog => "/Fog/",
            Ambiance::Night => "/Night/",
            Ambiance::Attack => "/Attack/",
            Ambiance::Custom1 => "/Custom1/",
            Ambiance::Custom2 => "/Custom2/",
            Ambiance::Custom3 => "/Custom3/",
            Ambiance::Custom4 => "/Custom4/",
        }
    }
}

// ---------------------------------------------------------------------------
// Conversion table size
// ---------------------------------------------------------------------------

/// Number of physical animation slots (= 283).
pub const NONANIMATION_END: usize = OrderType::NonanimationEnd as usize;

/// Sentinel value meaning "no row mapped for this action".
pub const UNMAPPED: u16 = 0xFFFF;

// ---------------------------------------------------------------------------
// SpriteScript
// ---------------------------------------------------------------------------

/// A single animation row: a sequence of frames with per-frame metadata.
///
/// Each row corresponds to one action/direction combination loaded from an
/// `.rhs` profile.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SpriteScript {
    /// Action ID this row implements (from `RowHeader::action_id`). Kept
    /// per-row so each script can identify its action even when multiple
    /// rows share the same id (only the first gets the conversion slot).
    pub action_id: u16,
    /// Action ID to chain into when this animation completes.
    pub action_done: u16,
    /// Average movement speed across all frames (`sum(distance) / sum(delay)`).
    pub average_speed: f32,
    /// Hotspot / info point for this row.
    pub hotspot: SpriteLocalPoint,
    /// Cumulative distance across all frames.
    pub sum_distance: u16,
    /// Frame bank IDs (one per frame).
    pub frame_ids: Vec<u32>,
    /// Display duration per frame (game ticks).
    pub delays: Vec<u16>,
    /// Movement distance per frame.
    pub distances: Vec<u16>,
    /// X/Y draw offset per frame.
    pub offsets: Vec<SpriteFrameOffset>,
    /// Sound effect ID per frame (0 = none).
    pub sound_ids: Vec<u16>,
}

impl Default for SpriteScript {
    fn default() -> Self {
        Self {
            action_id: UNMAPPED,
            action_done: 0,
            average_speed: 0.0,
            hotspot: SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: Vec::new(),
            delays: Vec::new(),
            distances: Vec::new(),
            offsets: Vec::new(),
            sound_ids: Vec::new(),
        }
    }
}

impl SpriteScript {
    /// Number of frames in this animation row.
    pub fn num_frames(&self) -> usize {
        self.frame_ids.len()
    }
}

// ---------------------------------------------------------------------------
// SpriteInfo
// ---------------------------------------------------------------------------

/// A loaded sprite profile containing all animation rows and their mapping.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SpriteInfo {
    /// All animation rows for this profile.
    /// Arc-shared so entities with the same profile share one copy.
    pub scripts: std::sync::Arc<Vec<SpriteScript>>,
    /// Maps action ID → row index. Length is [`NONANIMATION_END`].
    /// [`UNMAPPED`] (`0xFFFF`) means no row is assigned for that action.
    pub conversion: std::sync::Arc<Vec<u16>>,
    /// Sprite bounding size (width, height).
    pub size: SpriteSize,
    /// Rotation center point.
    pub center: SpriteAnchor,
}

impl SpriteInfo {
    /// Look up which row index corresponds to an action ID.
    /// Returns `None` if the action is unmapped.
    pub fn row_for_action(&self, action_id: u16) -> Option<usize> {
        let idx = action_id as usize;
        if idx >= self.conversion.len() {
            return None;
        }
        let row = self.conversion[idx];
        if row == UNMAPPED {
            None
        } else {
            Some(row as usize)
        }
    }

    /// Get the [`SpriteScript`] for a given action ID, if mapped.
    pub fn script_for_action(&self, action_id: u16) -> Option<&SpriteScript> {
        self.row_for_action(action_id)
            .and_then(|row| self.scripts.get(row))
    }
}

// ---------------------------------------------------------------------------
// Binary format helpers (packed structs from the .rhs file)
// ---------------------------------------------------------------------------

/// Read a profile header from the file (40 bytes packed).
///
/// Layout (pack 2):
/// - `profile_name`: 32 bytes (null-terminated ASCII)
/// - `num_rows`: u16 LE
/// - `width`: u16 LE
/// - `height`: u16 LE
/// - `rotation_x`: i32 LE
/// - `rotation_y`: i32 LE
struct ProfileHeader {
    name: String,
    num_rows: u16,
    width: u16,
    height: u16,
    rotation_x: i32,
    rotation_y: i32,
}

impl ProfileHeader {
    /// Size in bytes of the packed C struct.
    const PACKED_SIZE: usize = 32 + 2 + 2 + 2 + 4 + 4; // = 46

    fn read(file: &mut SbFile) -> Result<Self, String> {
        let mut name_buf = [0u8; 32];
        file.serialize_bytes(&mut name_buf)
            .map_err(|e| format!("ProfileHeader: failed to read name: {e}"))?;
        // Truncate at first null byte — buffer may have garbage after the
        // terminator.
        let name = std::ffi::CStr::from_bytes_until_nul(&name_buf)
            .map(|cs| cs.to_string_lossy().into_owned())
            .unwrap_or_default();

        let mut num_rows = 0u16;
        let mut width = 0u16;
        let mut height = 0u16;
        let mut rotation_x = 0i32;
        let mut rotation_y = 0i32;

        file.serialize_u16(&mut num_rows)
            .map_err(|e| format!("ProfileHeader: num_rows: {e}"))?;
        file.serialize_u16(&mut width)
            .map_err(|e| format!("ProfileHeader: width: {e}"))?;
        file.serialize_u16(&mut height)
            .map_err(|e| format!("ProfileHeader: height: {e}"))?;
        file.serialize_i32(&mut rotation_x)
            .map_err(|e| format!("ProfileHeader: rotation_x: {e}"))?;
        file.serialize_i32(&mut rotation_y)
            .map_err(|e| format!("ProfileHeader: rotation_y: {e}"))?;

        Ok(Self {
            name,
            num_rows,
            width,
            height,
            rotation_x,
            rotation_y,
        })
    }
}

/// Row header from .rhs binary format (12 bytes packed).
///
/// Layout (pack 2):
/// - `num_frames`: u16 LE
/// - `action_done`: u16 LE
/// - `hotspot_x`: i32 LE
/// - `hotspot_y`: i32 LE
/// - `action_id`: u16 LE
struct RowHeader {
    num_frames: u16,
    action_done: u16,
    hotspot_x: i32,
    hotspot_y: i32,
    action_id: u16,
}

impl RowHeader {
    fn read(file: &mut SbFile) -> Result<Self, String> {
        let mut num_frames = 0u16;
        let mut action_done = 0u16;
        let mut hotspot_x = 0i32;
        let mut hotspot_y = 0i32;
        let mut action_id = 0u16;

        file.serialize_u16(&mut num_frames)
            .map_err(|e| format!("RowHeader: num_frames: {e}"))?;
        file.serialize_u16(&mut action_done)
            .map_err(|e| format!("RowHeader: action_done: {e}"))?;
        file.serialize_i32(&mut hotspot_x)
            .map_err(|e| format!("RowHeader: hotspot_x: {e}"))?;
        file.serialize_i32(&mut hotspot_y)
            .map_err(|e| format!("RowHeader: hotspot_y: {e}"))?;
        file.serialize_u16(&mut action_id)
            .map_err(|e| format!("RowHeader: action_id: {e}"))?;

        Ok(Self {
            num_frames,
            action_done,
            hotspot_x,
            hotspot_y,
            action_id,
        })
    }
}

/// Frame header from .rhs binary format (12 bytes packed).
///
/// Layout (pack 2):
/// - `id_in_bank`: u32 LE
/// - `delay`: u16 LE
/// - `distance`: u16 LE
/// - `x_offset`: i16 LE
/// - `y_offset`: i16 LE
/// - `sound_id`: u16 LE
struct FrameHeader {
    id_in_bank: u32,
    delay: u16,
    distance: u16,
    x_offset: i16,
    y_offset: i16,
    sound_id: u16,
}

impl FrameHeader {
    /// Size in bytes of the packed C struct.
    const PACKED_SIZE: usize = 4 + 2 + 2 + 2 + 2 + 2; // = 14

    fn read(file: &mut SbFile) -> Result<Self, String> {
        let mut id_in_bank = 0u32;
        let mut delay = 0u16;
        let mut distance = 0u16;
        let mut x_offset = 0i16;
        let mut y_offset = 0i16;
        let mut sound_id = 0u16;

        file.serialize_u32(&mut id_in_bank)
            .map_err(|e| format!("FrameHeader: id_in_bank: {e}"))?;
        file.serialize_u16(&mut delay)
            .map_err(|e| format!("FrameHeader: delay: {e}"))?;
        file.serialize_u16(&mut distance)
            .map_err(|e| format!("FrameHeader: distance: {e}"))?;
        file.serialize_i16(&mut x_offset)
            .map_err(|e| format!("FrameHeader: x_offset: {e}"))?;
        file.serialize_i16(&mut y_offset)
            .map_err(|e| format!("FrameHeader: y_offset: {e}"))?;
        file.serialize_u16(&mut sound_id)
            .map_err(|e| format!("FrameHeader: sound_id: {e}"))?;

        Ok(Self {
            id_in_bank,
            delay,
            distance,
            x_offset,
            y_offset,
            sound_id,
        })
    }
}

// ---------------------------------------------------------------------------
// SpriteScriptor
// ---------------------------------------------------------------------------

/// Sprite animation loader and cache.
///
/// Loads `.rhs` binary files containing named animation profiles, parses
/// their rows and frames, and caches results keyed by `filename + profile`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SpriteScriptor {
    cache: HashMap<String, SpriteInfo>,
}

impl SpriteScriptor {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Resolve the `.rhs` file path for a given frame kind.
    ///
    /// For `Animation` sprites, tries the current ambiance sub-directory first,
    /// then falls back to Day, then the base directory.
    pub fn resolve_rhs_path(
        frame_kind: FrameKind,
        base_dir: &str,
        filename: &str,
        ambiance: Option<Ambiance>,
    ) -> Result<String, String> {
        match frame_kind {
            FrameKind::Character | FrameKind::Object | FrameKind::CharacterBlipped => {
                Ok(format!("{base_dir}/{filename}.rhs"))
            }
            FrameKind::Animation => {
                let amb = ambiance.unwrap_or(Ambiance::Day);

                // Try current ambiance
                let path = format!("{base_dir}{}{filename}.rhs", amb.directory_suffix());
                if SbFile::exists(&path) {
                    return Ok(path);
                }

                // Fall back to Day
                if amb != Ambiance::Day {
                    let path = format!(
                        "{base_dir}{}{filename}.rhs",
                        Ambiance::Day.directory_suffix()
                    );
                    if SbFile::exists(&path) {
                        return Ok(path);
                    }
                }

                // Fall back to base directory (no ambiance)
                let path = format!("{base_dir}/{filename}.rhs");
                if SbFile::exists(&path) {
                    return Ok(path);
                }

                Err(format!(
                    "Unable to find RHS file {filename}.rhs in {base_dir} (ambiance {:?})",
                    amb
                ))
            }
        }
    }

    /// Load a sprite profile, returning cached data if already loaded.
    ///
    /// `validate_file` is called after opening the file to check the bank
    /// signature. Pass a no-op closure if validation is not needed.
    pub fn load(
        &mut self,
        path: &str,
        profile_name: &str,
        cache_key: &str,
        frame_kind: FrameKind,
        validate_file: impl FnOnce(&mut SbFile) -> Result<(), String>,
    ) -> Result<&SpriteInfo, String> {
        if self.cache.contains_key(cache_key) {
            return Ok(&self.cache[cache_key]);
        }

        let mut file =
            SbFile::open(path, 0).map_err(|e| format!("Unable to open RHS file {path}: {e}"))?;

        validate_file(&mut file)?;

        // Seek to the named profile
        if !Self::find_profile(&mut file, profile_name)? {
            return Err(format!(
                "Unable to find profile '{profile_name}' in RHS file {path}"
            ));
        }

        // Read profile header
        let header = ProfileHeader::read(&mut file)?;

        // Initialize conversion table (all unmapped)
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];

        // Load all rows
        let scripts = Self::load_rows(
            &mut file,
            header.num_rows,
            &mut conversion,
            &format!("{path}:{profile_name}"),
            matches!(
                frame_kind,
                FrameKind::Character | FrameKind::CharacterBlipped
            ),
        )?;

        let info = SpriteInfo {
            scripts: std::sync::Arc::new(scripts),
            conversion: std::sync::Arc::new(conversion),
            size: SpriteSize::new(header.width as f32, header.height as f32),
            center: SpriteAnchor::new(header.rotation_x as f32, header.rotation_y as f32),
        };

        self.cache.insert(cache_key.to_owned(), info);
        Ok(&self.cache[cache_key])
    }

    /// Get a previously loaded sprite info by cache key.
    pub fn get(&self, cache_key: &str) -> Option<&SpriteInfo> {
        self.cache.get(cache_key)
    }

    /// Insert an already-decoded sprite profile into the cache.
    ///
    /// Used by host-side hackable datadir support for `.rhs.d/manifest.json`
    /// folders, where PNG frames are appended to the runtime sprite bank
    /// before the engine asks for the profile.
    pub fn insert(&mut self, cache_key: impl Into<String>, info: SpriteInfo) {
        self.cache.insert(cache_key.into(), info);
    }

    /// Seek through the file to find a named profile.
    ///
    /// On success the file position is rewound to the start of the matching
    /// profile header.  Returns `false` if the profile is not found.
    fn find_profile(file: &mut SbFile, profile_name: &str) -> Result<bool, String> {
        let mut num_profiles = 0u16;
        file.serialize_u16(&mut num_profiles)
            .map_err(|e| format!("find_profile: read num_profiles: {e}"))?;

        let mut header = ProfileHeader::read(file)?;

        let mut current = 0u16;

        while header.name != profile_name && current < num_profiles.saturating_sub(1) {
            // Skip all rows and frames of this non-matching profile
            for _ in 0..header.num_rows {
                let row = RowHeader::read(file)?;
                // Skip all frame headers for this row
                file.skip(
                    (row.num_frames as u64 * FrameHeader::PACKED_SIZE as u64) as i64,
                    1, // SEEK_CUR (relative seek)
                );
            }

            header = ProfileHeader::read(file)?;
            current += 1;
        }

        if header.name == profile_name {
            // Rewind to the start of this profile header
            file.skip(-(ProfileHeader::PACKED_SIZE as i64), 1); // SEEK_CUR (relative seek)
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Dump **every** profile from an `.rhs` file.
    ///
    /// Used by the datadir converter: the runtime only loads one profile at a
    /// time, but for export we want the whole file. Returns the bank
    /// signature from the file header and a list of `(profile_name,
    /// SpriteInfo)` for each profile in file order.
    pub fn load_all_profiles(path: &str) -> Result<(u32, Vec<(String, SpriteInfo)>), String> {
        let mut file = SbFile::open(path, 0).map_err(|e| format!("open rhs {path}: {e}"))?;

        // File starts with a u32 bank signature followed by u16 num_profiles.
        let mut signature = 0u32;
        file.serialize_u32(&mut signature)
            .map_err(|e| format!("read signature: {e}"))?;

        let mut num_profiles = 0u16;
        file.serialize_u16(&mut num_profiles)
            .map_err(|e| format!("read num_profiles: {e}"))?;

        let mut out = Vec::with_capacity(num_profiles as usize);
        for _ in 0..num_profiles {
            let header = ProfileHeader::read(&mut file)?;
            let mut conversion = vec![UNMAPPED; NONANIMATION_END];
            let label = format!("{path}:{}", header.name);
            let scripts =
                Self::load_rows(&mut file, header.num_rows, &mut conversion, &label, false)?;
            let info = SpriteInfo {
                scripts: std::sync::Arc::new(scripts),
                conversion: std::sync::Arc::new(conversion),
                size: SpriteSize::new(header.width as f32, header.height as f32),
                center: SpriteAnchor::new(header.rotation_x as f32, header.rotation_y as f32),
            };
            out.push((header.name, info));
        }
        Ok((signature, out))
    }

    /// Parse all rows and their frames from the current file position.
    fn load_rows(
        file: &mut SbFile,
        num_rows: u16,
        conversion: &mut [u16],
        label: &str,
        expect_directional: bool,
    ) -> Result<Vec<SpriteScript>, String> {
        let mut scripts = Vec::with_capacity(num_rows as usize);

        for _ in 0..num_rows {
            let row_hdr = RowHeader::read(file)?;

            assert!(
                (row_hdr.action_id as usize) < NONANIMATION_END,
                "action_id {} >= NONANIMATION_END ({})",
                row_hdr.action_id,
                NONANIMATION_END
            );

            // Each action has up to 16 consecutive rows, one per facing
            // direction (runtime indexes as `conversion[action] + dir`).
            // Only the first row (dir 0) is recorded in the conversion
            // table; subsequent rows are reached via the +direction offset.
            let row_index = scripts.len() as u16;
            let slot = &mut conversion[row_hdr.action_id as usize];
            if *slot == UNMAPPED {
                *slot = row_index;
            }

            let num_frames = row_hdr.num_frames as usize;
            let mut script = SpriteScript {
                action_id: row_hdr.action_id,
                action_done: row_hdr.action_done,
                hotspot: SpriteLocalPoint::new(row_hdr.hotspot_x as f32, row_hdr.hotspot_y as f32),
                frame_ids: Vec::with_capacity(num_frames),
                delays: Vec::with_capacity(num_frames),
                distances: Vec::with_capacity(num_frames),
                offsets: Vec::with_capacity(num_frames),
                sound_ids: Vec::with_capacity(num_frames),
                ..Default::default()
            };

            let mut total_delay: u32 = 0;
            let mut speed_accum: f32 = 0.0;
            let mut sum_distance: u32 = 0;

            for _ in 0..num_frames {
                let fh = FrameHeader::read(file)?;

                script.frame_ids.push(fh.id_in_bank);
                script.delays.push(fh.delay);
                // Distance is stored as an unsigned but interpreted as signed —
                // take the absolute value.
                script.distances.push((fh.distance as i16).unsigned_abs());
                script.offsets.push(SpriteFrameOffset::new(
                    fh.x_offset as f32,
                    fh.y_offset as f32,
                ));
                script.sound_ids.push(fh.sound_id);

                speed_accum += fh.distance as f32;
                total_delay += fh.delay.max(1) as u32;
                sum_distance += fh.distance as u32;
            }

            script.average_speed = if total_delay > 0 {
                speed_accum / total_delay as f32
            } else {
                0.0
            };
            script.sum_distance = sum_distance as u16;

            scripts.push(script);
        }

        // Sanity check: each action should have exactly 16 consecutive rows
        // (one per facing direction) per the engine convention. Anything
        // else means either the data is unusual or we've misread it.
        let mut counts: std::collections::HashMap<u16, u16> = std::collections::HashMap::new();
        for s in &scripts {
            *counts.entry(s.action_id).or_insert(0) += 1;
        }
        if expect_directional {
            // Expected: 16 rows (one per facing direction).
            // Tolerated: 32 rows — shipped data packs action 180
            // (`TRANSITION_WAITING_UPRIGHT_HELPING_CLIMBING`) together with
            // its reverse transition (action 181,
            // `TRANSITION_HELPING_CLIMBING_WAITING_UPRIGHT`) under the same
            // action id. The conversion table has the same quirk:
            // `conversion[181]` stays UNMAPPED, so action 181 is never
            // played for the blipped variant. Verified visually by dumping
            // all 208 frames — two distinct animations.
            let mut bad: Vec<(u16, u16)> = counts
                .iter()
                .filter(|(_, c)| **c != 16 && **c != 32)
                .map(|(a, c)| (*a, *c))
                .collect();
            if !bad.is_empty() {
                bad.sort();
                tracing::warn!(
                    "[{}] unexpected row counts ({} actions total): {:?}",
                    label,
                    counts.len(),
                    bad,
                );
            }
        }

        Ok(scripts)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sprite_script_default() {
        let s = SpriteScript::default();
        assert_eq!(s.action_done, 0);
        assert_eq!(s.average_speed, 0.0);
        assert_eq!(s.num_frames(), 0);
        assert_eq!(s.sum_distance, 0);
        assert!(s.frame_ids.is_empty());
        assert!(s.delays.is_empty());
        assert!(s.distances.is_empty());
        assert!(s.offsets.is_empty());
        assert!(s.sound_ids.is_empty());
    }

    #[test]
    fn test_sprite_info_row_for_action() {
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[0] = 0; // WaitingUprightBored -> row 0
        conversion[10] = 1; // RunningUpright -> row 1

        let info = SpriteInfo {
            scripts: std::sync::Arc::new(vec![SpriteScript::default(), SpriteScript::default()]),
            conversion: std::sync::Arc::new(conversion),
            size: SpriteSize::new(64.0, 64.0),
            center: SpriteAnchor::new(32.0, 32.0),
        };

        assert_eq!(info.row_for_action(0), Some(0));
        assert_eq!(info.row_for_action(10), Some(1));
        assert_eq!(info.row_for_action(5), None); // unmapped
        assert_eq!(info.row_for_action(999), None); // out of range
    }

    #[test]
    fn test_sprite_info_script_for_action() {
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[0] = 0;

        let script = SpriteScript {
            action_done: 42,
            frame_ids: vec![100, 101, 102],
            delays: vec![3, 3, 3],
            distances: vec![5, 5, 5],
            offsets: vec![
                SpriteFrameOffset::new(0.0, 0.0),
                SpriteFrameOffset::new(1.0, 0.0),
                SpriteFrameOffset::new(2.0, 0.0),
            ],
            sound_ids: vec![0, 0, 1],
            ..Default::default()
        };

        let info = SpriteInfo {
            scripts: std::sync::Arc::new(vec![script]),
            conversion: std::sync::Arc::new(conversion),
            size: SpriteSize::new(128.0, 128.0),
            center: SpriteAnchor::new(64.0, 64.0),
        };

        let s = info.script_for_action(0).unwrap();
        assert_eq!(s.action_done, 42);
        assert_eq!(s.num_frames(), 3);
        assert_eq!(s.frame_ids, vec![100, 101, 102]);

        assert!(info.script_for_action(1).is_none());
    }

    #[test]
    fn test_ambiance_directory_suffix() {
        assert_eq!(Ambiance::Day.directory_suffix(), "/Day/");
        assert_eq!(Ambiance::Fog.directory_suffix(), "/Fog/");
        assert_eq!(Ambiance::Night.directory_suffix(), "/Night/");
        assert_eq!(Ambiance::Attack.directory_suffix(), "/Attack/");
        assert_eq!(Ambiance::Custom1.directory_suffix(), "/Custom1/");
        assert_eq!(Ambiance::Custom2.directory_suffix(), "/Custom2/");
        assert_eq!(Ambiance::Custom3.directory_suffix(), "/Custom3/");
        assert_eq!(Ambiance::Custom4.directory_suffix(), "/Custom4/");
    }

    #[test]
    fn test_nonanimation_end_value() {
        assert_eq!(NONANIMATION_END, 283);
    }

    #[test]
    fn test_scriptor_new_and_get() {
        let scriptor = SpriteScriptor::new();
        assert!(scriptor.get("nonexistent").is_none());
    }

    #[test]
    fn test_frame_kind_serialize_roundtrip() {
        let kinds = [
            FrameKind::Character,
            FrameKind::CharacterBlipped,
            FrameKind::Animation,
            FrameKind::Object,
        ];
        for kind in &kinds {
            let json = serde_json::to_string(kind).unwrap();
            let back: FrameKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*kind, back);
        }
    }

    #[test]
    fn test_sprite_info_serde_roundtrip() {
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[0] = 0;

        let script = SpriteScript {
            frame_ids: vec![1, 2, 3],
            delays: vec![10, 10, 10],
            distances: vec![2, 2, 2],
            offsets: vec![
                SpriteFrameOffset::new(0.0, 0.0),
                SpriteFrameOffset::new(1.0, 1.0),
                SpriteFrameOffset::new(2.0, 2.0),
            ],
            sound_ids: vec![0, 0, 0],
            average_speed: 0.2,
            sum_distance: 6,
            ..Default::default()
        };

        let info = SpriteInfo {
            scripts: std::sync::Arc::new(vec![script]),
            conversion: std::sync::Arc::new(conversion),
            size: SpriteSize::new(48.0, 48.0),
            center: SpriteAnchor::new(24.0, 24.0),
        };

        let json = serde_json::to_string(&info).unwrap();
        let back: SpriteInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(back.scripts.len(), 1);
        assert_eq!(back.scripts[0].frame_ids, vec![1, 2, 3]);
        assert_eq!(back.size.x, 48.0);
        assert_eq!(back.center.y, 24.0);
        assert_eq!(back.conversion[0], 0);
        assert_eq!(back.conversion[1], UNMAPPED);
    }

    #[test]
    fn test_resolve_rhs_path_character() {
        let path =
            SpriteScriptor::resolve_rhs_path(FrameKind::Character, "/data/chars", "robin", None)
                .unwrap();
        assert_eq!(path, "/data/chars/robin.rhs");
    }

    #[test]
    fn test_resolve_rhs_path_object() {
        let path =
            SpriteScriptor::resolve_rhs_path(FrameKind::Object, "/data/objects", "chest", None)
                .unwrap();
        assert_eq!(path, "/data/objects/chest.rhs");
    }

    #[test]
    fn cached_object_master_satisfies_later_animation_lookup() {
        let mut scriptor = SpriteScriptor::new();
        scriptor.insert(
            "ACCESSORIES_Ale/ACCESSOIRES Ale",
            SpriteInfo {
                scripts: std::sync::Arc::new(Vec::new()),
                conversion: std::sync::Arc::new(vec![UNMAPPED; NONANIMATION_END]),
                size: SpriteSize::new(12.0, 8.0),
                center: SpriteAnchor::new(2.0, 3.0),
            },
        );

        let info = scriptor
            .load(
                "Data/Animations/Day/does-not-exist.rhs",
                "ACCESSOIRES Ale",
                "ACCESSORIES_Ale/ACCESSOIRES Ale",
                FrameKind::Animation,
                |_| Err("cached lookup must not open or validate a file".to_owned()),
            )
            .expect("the original cache is shared across frame kinds");

        assert_eq!(info.size, SpriteSize::new(12.0, 8.0));
    }
}

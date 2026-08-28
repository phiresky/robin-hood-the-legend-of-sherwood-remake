//! Level resource descriptors (`.red` files).
//!
//! Each level has a small binary descriptor file that maps dialogue,
//! popup-text, debriefing and short-briefing data to resource IDs in
//! `.res` files (text tables, wave tables, pictures).
//!
//! ## File format
//!
//! The `.red` file is a flat sequence of little-endian u32 values with no
//! header or magic bytes:
//!
//! ```text
//! MissionDescription:
//!   u32 text_table_id
//!   u32 picture_id
//! Dialogues:
//!   u32 dialogue_count
//!   for each dialogue:
//!     u32 text_table_id      — resource ID of the TEXT string table
//!     u32 sound_table_id     — resource ID of the WAVE path table
//!     u32 portrait_count     — number of sentences / portrait entries
//!     portrait_count × u32   — portrait index per sentence (0..15)
//! PopupText:
//!   u32 picture_count
//!   u32 text_table_id
//!   picture_count × u32 picture_ids
//! Debriefing:
//!   u32 win_count
//!   u32 win_text_table_id
//!   u32 lose_count
//!   u32 lose_text_table_id
//! ShortBriefing:
//!   u32 briefing_count
//!   u32 text_table_id
//! ```
//!
//! ## Filename convention
//!
//! Files are named `RHLevel<ID>.red` where `<ID>` is the mission profile
//! ID emitted byte-by-byte (e.g. profile ID `0x4253` → `"SB"` →
//! `RHLevelSB.red`).  They live in the text directory
//! (`Data/Text/` by default).

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::picture::read_u32;
use crate::resource_manager::ResourceId;
use robin_engine::sbfile::SbFile;

// ═══════════════════════════════════════════════════════════════════
//  Public types
// ═══════════════════════════════════════════════════════════════════

/// A single dialogue descriptor: points at a text table and a wave
/// table in the resource manager, plus per-sentence portrait indices.
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct DialogueDescriptor {
    /// Resource ID of the TEXT string table (one string per sentence).
    pub text_table_id: ResourceId,
    /// Resource ID of the WAVE path table (one path per sentence).
    pub sound_table_id: ResourceId,
    /// Portrait index for each sentence (0..15 → `DIALOGUE_PORTRAIT_IDS`).
    pub portrait_ids: Vec<u32>,
}

/// Mission description metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct MissionDescription {
    pub text_table_id: ResourceId,
    pub picture_id: ResourceId,
}

/// Popup-text descriptors for a level.
#[derive(Debug, Clone, Default, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct PopupTextDescriptor {
    pub text_table_id: ResourceId,
    pub picture_ids: Vec<ResourceId>,
}

/// Debriefing descriptors (win / lose text tables).
#[derive(Debug, Clone, Default, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct DebriefingDescriptor {
    pub win_count: u32,
    pub win_text_table_id: ResourceId,
    pub lose_count: u32,
    pub lose_text_table_id: ResourceId,
}

/// Short-briefing descriptor.
#[derive(Debug, Clone, Default, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShortBriefingDescriptor {
    pub briefing_count: u32,
    pub text_table_id: ResourceId,
}

/// All resource descriptors for a single level.
///
/// Loaded from an `RHLevel<ID>.red` file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct LevelDescriptors {
    pub mission_description: MissionDescription,
    pub dialogues: Vec<DialogueDescriptor>,
    pub popup_text: PopupTextDescriptor,
    pub debriefing: DebriefingDescriptor,
    pub short_briefing: ShortBriefingDescriptor,
    /// Mod-local text overrides indexed by the original script text IDs.
    #[serde(default)]
    pub custom_popup_texts: Vec<Option<String>>,
    #[serde(default)]
    pub custom_short_briefings: Vec<Option<String>>,
    #[serde(default)]
    pub custom_dialogue_texts: Vec<Option<Vec<String>>>,
}

// ═══════════════════════════════════════════════════════════════════
//  Loading
// ═══════════════════════════════════════════════════════════════════

/// Build the `.red` filename for a mission profile ID.
///
/// The ID bytes are emitted low-byte-first as ASCII characters,
/// sandwiched between the `"RHLevel"` prefix and `".red"` suffix.
pub fn red_filename(mission_id: u32) -> String {
    let mut name = String::from("RHLevel");
    let mut id = mission_id;
    while id != 0 {
        name.push((id & 0xFF) as u8 as char);
        id >>= 8;
    }
    name.push_str(".red");
    name
}

/// Read one little-endian u32, returning it as an `i32` [`ResourceId`].
fn read_id(file: &mut SbFile) -> Result<ResourceId> {
    Ok(read_u32(file)? as i32)
}

/// Load level descriptors from a `.red` file.
///
/// `path` should be the full path (e.g. `Data/Text/RHLevelSB.red`).
/// Returns an error if the file cannot be opened or is malformed.
pub fn load(path: &str) -> Result<LevelDescriptors> {
    let mut file = SbFile::open(path, 0).map_err(|e| anyhow!("open '{path}': error {e}"))?;

    // ── Mission description (2 × u32) ──
    let mission_description = MissionDescription {
        text_table_id: read_id(&mut file)?,
        picture_id: read_id(&mut file)?,
    };

    // ── Dialogues ──
    let dialogue_count = read_u32(&mut file)? as usize;
    let mut dialogues = Vec::with_capacity(dialogue_count);
    for _ in 0..dialogue_count {
        let text_table_id = read_id(&mut file)?;
        let sound_table_id = read_id(&mut file)?;
        let portrait_count = read_u32(&mut file)? as usize;
        let mut portrait_ids = Vec::with_capacity(portrait_count);
        for _ in 0..portrait_count {
            portrait_ids.push(read_u32(&mut file)?);
        }
        dialogues.push(DialogueDescriptor {
            text_table_id,
            sound_table_id,
            portrait_ids,
        });
    }

    // ── Popup text ──
    let picture_count = read_u32(&mut file)? as usize;
    let popup_text_table_id = read_id(&mut file)?;
    let mut picture_ids = Vec::with_capacity(picture_count);
    for _ in 0..picture_count {
        picture_ids.push(read_id(&mut file)?);
    }
    let popup_text = PopupTextDescriptor {
        text_table_id: popup_text_table_id,
        picture_ids,
    };

    // ── Debriefing (4 × u32, read as a flat struct) ──
    let debriefing = DebriefingDescriptor {
        win_count: read_u32(&mut file)?,
        win_text_table_id: read_id(&mut file)?,
        lose_count: read_u32(&mut file)?,
        lose_text_table_id: read_id(&mut file)?,
    };

    // ── Short briefing (2 × u32) ──
    let short_briefing = ShortBriefingDescriptor {
        briefing_count: read_u32(&mut file)?,
        text_table_id: read_id(&mut file)?,
    };

    Ok(LevelDescriptors {
        mission_description,
        dialogues,
        popup_text,
        debriefing,
        short_briefing,
        custom_popup_texts: Vec::new(),
        custom_short_briefings: Vec::new(),
        custom_dialogue_texts: Vec::new(),
    })
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn red_filename_demo_leicester() {
        // Mission ID for the demo Leicester level is "SB" = 0x4253
        // (little-endian: byte 0 = 'S' = 0x53, byte 1 = 'B' = 0x42)
        assert_eq!(red_filename(0x4253), "RHLevelSB.red");
    }

    #[test]
    fn red_filename_single_byte() {
        assert_eq!(red_filename(0x41), "RHLevelA.red");
    }

    #[test]
    fn red_filename_three_bytes() {
        // 0x434241 → bytes 'A', 'B', 'C'
        assert_eq!(red_filename(0x43_42_41), "RHLevelABC.red");
    }

    #[test]
    fn load_demo_leicester_red() {
        // Requires the demo data directory to be present.
        let path = "Data/Text/RHLevelSB.red";
        let file_check = SbFile::open(path, 0);
        if file_check.is_err() {
            eprintln!("Skipping .red load test — data file not found");
            return;
        }
        drop(file_check);

        let desc = load(path).expect("failed to load .red");

        // The demo has 1 dialogue with 8 sentences.
        assert_eq!(desc.dialogues.len(), 1);
        assert_eq!(desc.dialogues[0].portrait_ids.len(), 8);
        // Portraits alternate between Scarlet (3) and Robin (0).
        assert_eq!(desc.dialogues[0].portrait_ids[0], 3);
        assert_eq!(desc.dialogues[0].portrait_ids[1], 0);
    }
}

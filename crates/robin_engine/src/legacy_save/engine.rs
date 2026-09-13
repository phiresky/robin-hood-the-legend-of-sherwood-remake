//! Decoder for the scalar prefix of original-game v48 engine serialization.
//!
//! This stops at the exact byte where element serialization starts.
//! Element decoding belongs to a later importer milestone; no scan or guessed
//! byte skip is used to find that boundary. Field declaration order is wire
//! order.

use super::read_helpers::DEFAULT_LIST_LIMIT;
use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyRead, LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;

const RH_SHORT_BRIEFINGS_FINGERPRINT: [u8; 16] = [
    0x20, 0xbe, 0xb4, 0x1b, 0x22, 0x2b, 0x27, 0xb2, 0x7c, 0x13, 0x30, 0xfa, 0xeb, 0xd2, 0x4e, 0xa6,
];
const RH_SOUND_FINGERPRINT: [u8; 16] = [
    0x57, 0xd8, 0x5c, 0x0f, 0xe7, 0x80, 0x27, 0x68, 0x22, 0x77, 0xc8, 0x21, 0x0e, 0x31, 0xf9, 0xa4,
];
const RH_SOUND_GEOMETRY_FINGERPRINT: [u8; 16] = [
    0xe1, 0x72, 0x53, 0x47, 0x14, 0xa9, 0x24, 0x00, 0x6d, 0x1d, 0xc5, 0x76, 0x03, 0x8c, 0x2e, 0x5f,
];
const RH_SOUND_SOURCE_MANAGER_FINGERPRINT: [u8; 16] = [
    0xba, 0xff, 0x31, 0xe5, 0x01, 0x84, 0x41, 0x8d, 0x0b, 0xfe, 0x86, 0x85, 0xf4, 0xb1, 0x96, 0x34,
];
const RH_SOUND_SOURCE_FINGERPRINT: [u8; 16] = [
    0xa0, 0x6e, 0x2c, 0x17, 0xfc, 0x06, 0x9a, 0x8d, 0x20, 0x38, 0xec, 0xf8, 0x59, 0x8f, 0x29, 0xbe,
];
const RH_MESSENGER_FINGERPRINT: [u8; 16] = [
    0x5a, 0xde, 0x41, 0xa7, 0xc2, 0x00, 0xab, 0x89, 0x74, 0xfa, 0x81, 0xa6, 0xd5, 0x4e, 0xe6, 0xd2,
];
const RH_GAME_FINGERPRINT: [u8; 16] = [
    0x19, 0x85, 0x28, 0x22, 0x9c, 0x70, 0xa8, 0x38, 0x7a, 0xfc, 0xfe, 0x54, 0x03, 0x6f, 0x1d, 0x56,
];

/// Caller-controlled allocation limits for the lengthless engine prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyEngineLimits {
    pub short_briefings: usize,
    pub sound_sources: usize,
    pub sound_source_shape_points: usize,
}

impl Default for LegacyEngineLimits {
    fn default() -> Self {
        Self {
            short_briefings: DEFAULT_LIST_LIMIT,
            sound_sources: DEFAULT_LIST_LIMIT,
            sound_source_shape_points: 65535,
        }
    }
}

/// Decode context for [`LegacyEnginePreamble`].
#[derive(Clone, Copy)]
pub struct LegacyEngineDecode<'a> {
    pub abi_profile: LegacySaveAbiProfile,
    pub limits: &'a LegacyEngineLimits,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyEngineDecode<'_>)]
pub struct LegacyEnginePreamble {
    #[legacy(offset)]
    pub start_offset: u64,
    pub cheat_used_flags: u32,
    pub shield_protected: bool,
    pub freeze_all: bool,
    pub view: LegacyPoint2,
    pub zoom_factor: f32,
    pub camera_slide: LegacyPoint2,
    pub fixed_camera_speed: u16,
    pub speed: f32,
    pub speed_index: u16,
    pub desired_zoom_factor: f32,
    pub old_zoom_factor: f32,
    #[legacy(read = LegacyBackgroundTransform::read(reader, ctx.abi_profile))]
    pub background_transform: LegacyBackgroundTransform,
    pub universal_frame_counter: u32,
    pub creation_counter: u32,
    pub repulsive_point_counter: u32,
    pub lock_engine: bool,
    pub mission_won: bool,
    pub mission_won_first_time: bool,
    pub camera_wanted: LegacyPoint2,
    pub locker: bool,
    pub skip_data: String,
    #[legacy(read = LegacyShortBriefings::read_field(reader, "short_briefings", ctx.limits))]
    pub short_briefings: LegacyShortBriefings,
    #[legacy(read = LegacySound::read_field(reader, "sound", ctx.limits))]
    pub sound: LegacySound,
    pub messenger: LegacyMessenger,
    pub game: LegacyGameState,
    /// Exact first byte consumed by element serialization.
    #[legacy(offset)]
    pub elements_offset: u64,
}

impl LegacyEnginePreamble {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyEngineLimits,
    ) -> LegacyResult<Self> {
        Self::read_field(
            reader,
            "rhsg.engine",
            &LegacyEngineDecode {
                abi_profile,
                limits,
            },
        )
    }
}

pub use super::payload_base::LegacyPoint2;

/// Raw background transform as emitted by the original game's save format.
///
/// The two padding members are deliberately retained because the raw save writes the
/// complete C struct rather than serializing its logical members.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyBackgroundTransform {
    pub scroll_to_left: bool,
    pub scroll_to_up: bool,
    pub current_x_scrolling_level: u16,
    pub current_y_scrolling_level: u16,
    #[legacy(bytes)]
    pub padding_before_surface_ids: [u8; 2],
    pub surface_id: u32,
    pub final_surface_id: u32,
    pub zoom_to_up: bool,
    pub zoom_to_down: bool,
    pub required_zoom_up: bool,
    pub required_zoom_down: bool,
    pub zoom_count: u16,
    pub number_of_zoom_steps: u16,
    #[legacy(scoped)]
    pub x_scrolling_values: [f32; 32],
    #[legacy(scoped)]
    pub y_scrolling_values: [f32; 32],
    pub current_zoom_level: u16,
    #[legacy(bytes)]
    pub padding_before_zoom_values: [u8; 2],
    #[legacy(scoped)]
    pub zoom_values: [f32; 3],
    pub center_zoom: LegacyPoint2,
    pub clipped_zoom: LegacyPoint2,
    pub scrolling: LegacyPoint2,
}

impl LegacyBackgroundTransform {
    const SERIALIZED_SIZE: u64 = 320;

    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
    ) -> LegacyResult<Self> {
        // Audited MSVC Win32 and GCC i386 layouts agree for this struct.
        // Keep the profile match explicit so adding another producer ABI
        // cannot silently inherit this raw-layout assumption.
        match abi_profile {
            LegacySaveAbiProfile::RetailWindowsX86V48 | LegacySaveAbiProfile::PortLinuxI386V48 => {}
        }
        let start = reader.offset();
        let transform = Self::read_field(reader, "background_transform", &())?;
        debug_assert_eq!(reader.offset() - start, Self::SERIALIZED_SIZE);
        Ok(transform)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyShortBriefing {
    pub id: u32,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyEngineLimits,
    fingerprint = RH_SHORT_BRIEFINGS_FINGERPRINT,
    expected = "short-briefings fingerprint"
)]
pub struct LegacyShortBriefings {
    #[legacy(count_u32 = ctx.short_briefings, items)]
    pub primaries: Vec<LegacyShortBriefing>,
    #[legacy(count_u32 = ctx.short_briefings, items)]
    pub secondaries: Vec<LegacyShortBriefing>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyEngineLimits)]
pub struct LegacySound {
    /// `false` means the Original intentionally emitted no further sound data.
    pub serialized: bool,
    #[legacy(when = serialized, flatten)]
    pub state: Option<LegacySerializedSound>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyEngineLimits,
    fingerprint = RH_SOUND_FINGERPRINT,
    expected = "sound fingerprint"
)]
pub struct LegacySerializedSound {
    pub sound_system_ready: bool,
    pub three_d_sound: bool,
    pub active: bool,
    pub geometry: LegacySoundGeometry,
    pub music_mode: u8,
    pub dummy_channel: i16,
    pub quiet_mode_weight: u32,
    pub alert_mode_weight: u32,
    pub fight_mode_weight: u32,
    /// The member is signed, although the legacy format uses
    /// stored 16-bit width. Both are exactly two bytes in the supported layouts.
    pub loop_index: i16,
    pub stream_position: u32,
    #[legacy(read = LegacySoundSourceManager::read(reader, ctx))]
    pub source_manager: LegacySoundSourceManager,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    fingerprint = RH_SOUND_GEOMETRY_FINGERPRINT,
    expected = "sound-geometry fingerprint"
)]
pub struct LegacySoundGeometry {
    pub listen_point: LegacyPoint2,
    pub zoom_factor: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySoundSourceManager {
    pub slots: Vec<Option<LegacySoundSourceSlot>>,
}

impl LegacySoundSourceManager {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyEngineLimits) -> LegacyResult<Self> {
        reader.scope("source_manager", |reader| {
            reader.read_signature(
                "fingerprint",
                RH_SOUND_SOURCE_MANAGER_FINGERPRINT,
                "sound-source-manager fingerprint",
            )?;
            let count_offset = reader.offset();
            let raw_count = reader.read_u16("slot_count")?;
            let count = raw_count as usize;
            if count > limits.sound_sources {
                return Err(reader.invalid_value(
                    count_offset,
                    "slot_count",
                    raw_count,
                    "sound-source count within caller-supplied limit",
                ));
            }
            let slots = reader.read_list("slots", count, |reader, item| {
                reader.scope(item, |reader| {
                    let slot_index = reader.read_i16("slot_index")?;
                    if slot_index == -1 {
                        Ok(None)
                    } else {
                        Ok(Some(LegacySoundSourceSlot {
                            slot_index,
                            source: LegacySoundSource::read_field(reader, "source", limits)?,
                            registration_id: reader.read_u32("registration_id")?,
                        }))
                    }
                })
            })?;
            Ok(Self { slots })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySoundSourceSlot {
    pub slot_index: i16,
    pub source: LegacySoundSource,
    pub registration_id: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyEngineLimits,
    fingerprint = RH_SOUND_SOURCE_FINGERPRINT,
    expected = "sound-source fingerprint"
)]
pub struct LegacySoundSource {
    pub kind: u8,
    pub altitude: u8,
    pub id: u32,
    pub global: bool,
    pub inner_distance: u16,
    pub outer_distance: u16,
    pub noise_covering_distance: u16,
    pub inner_volume: u16,
    pub outer_volume: u16,
    pub min_delay: u16,
    pub max_delay: u16,
    pub delay_stepping: u16,
    pub timer: u16,
    pub active_first: bool,
    pub active_second: bool,
    pub former_need_update: bool,
    #[legacy(read = read_sound_source_shape(reader, ctx))]
    pub shape: Vec<LegacyPoint2>,
}

fn read_sound_source_shape(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyEngineLimits,
) -> LegacyResult<Vec<LegacyPoint2>> {
    let count_offset = reader.offset();
    let raw_count = reader.read_u16("shape.count")?;
    let count = raw_count as usize;
    if count > limits.sound_source_shape_points {
        return Err(reader.invalid_value(
            count_offset,
            "shape.count",
            raw_count,
            "sound-source shape count within caller-supplied limit",
        ));
    }
    reader.read_list("shape", count, |reader, item| {
        LegacyPoint2::read_field(reader, item, &())
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(fingerprint = RH_MESSENGER_FINGERPRINT, expected = "messenger fingerprint")]
pub struct LegacyMessenger {
    pub lock_view: bool,
    pub setting_watch: bool,
    pub watch_timer: u16,
    pub action: u16,
    pub draw_hidden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(fingerprint = RH_GAME_FINGERPRINT, expected = "game fingerprint")]
pub struct LegacyGameState {
    pub men_to_blazon_conversion: bool,
    pub campaign_map: bool,
    pub campaign_map_displayed: bool,
    pub post_initialized: bool,
    pub start_mission_disabled_temp: bool,
    pub quit_mission_disabled_temp: bool,
    pub start_mission_enabled: bool,
    pub quit_mission_enabled: bool,
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::legacy_save::LegacySaveHeader;
    use crate::legacy_save::campaign::{LegacyCampaignLimits, LegacySaveCampaigns};
    use crate::sbfile::SbFile;

    #[allow(dead_code)]
    use robin_test_support::original_data;

    fn repository_fixture(relative: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join(relative);
        assert!(
            path.is_file(),
            "required tracked legacy-save fixture missing: {}",
            path.display()
        );
        path
    }

    fn read_fixture(path: &Path) -> (LegacySaveHeader, LegacyEnginePreamble) {
        let path = path.to_string_lossy();
        let mut file = SbFile::open(&path).unwrap();
        let mut reader = LegacyReader::new(&mut file);
        let header = LegacySaveHeader::read(&mut reader).unwrap();
        let campaigns =
            LegacySaveCampaigns::read(&mut reader, &LegacyCampaignLimits::default()).unwrap();
        assert_eq!(reader.offset(), campaigns.engine_offset);
        let engine =
            LegacyEnginePreamble::read(&mut reader, header.abi_profile, &Default::default())
                .unwrap();
        assert_eq!(reader.offset(), engine.elements_offset);
        (header, engine)
    }

    #[test]
    #[ignore = "requires Linux i386 v48 profile saves via ROBINHOOD_DATA_DIR; select fixtures-legacy-linux"]
    fn golden_lincoln_restart_engine_boundary() {
        let path = original_data::data_file("Data/Savegame/Profile_000/Restart");
        let (header, engine) = read_fixture(&path);
        assert_eq!(header.abi_profile, LegacySaveAbiProfile::PortLinuxI386V48);
        assert_eq!(engine.start_offset, 5442);
        assert_eq!(engine.elements_offset, 6480);
        assert_eq!(engine.universal_frame_counter, 0);
        assert_eq!(engine.creation_counter, 158);
        assert!(engine.short_briefings.primaries.is_empty());
        assert!(engine.short_briefings.secondaries.is_empty());
        assert_eq!(
            engine
                .sound
                .state
                .as_ref()
                .unwrap()
                .source_manager
                .slots
                .len(),
            14
        );
        assert!(engine.elements_offset < std::fs::metadata(path).unwrap().len());
    }

    #[test]
    #[ignore = "requires Linux i386 v48 profile saves via ROBINHOOD_DATA_DIR; select fixtures-legacy-linux"]
    fn parses_current_linux_continue_engine_boundary() {
        let path = original_data::data_file("Data/Savegame/Profile_001/Continue");
        let (header, engine) = read_fixture(&path);
        assert_eq!(header.abi_profile, LegacySaveAbiProfile::PortLinuxI386V48);
        // `Continue` is mutable profile state. Immutable Restart/archive
        // fixtures above and below retain exact golden offsets and values.
        assert!(engine.start_offset >= crate::legacy_save::RHSG_HEADER_LEN);
        assert!(engine.elements_offset > engine.start_offset);
        assert!(engine.creation_counter > 0);
        assert!(engine.elements_offset < std::fs::metadata(path).unwrap().len());
    }

    #[test]
    fn golden_retail_windows_engine_boundary() {
        let path =
            repository_fixture("reference-saves/Savegame_SuN1Sh1nE/Profile_004/Savegame_005");
        let (header, engine) = read_fixture(&path);
        assert_eq!(
            header.abi_profile,
            LegacySaveAbiProfile::RetailWindowsX86V48
        );
        assert_eq!(engine.start_offset, 6678);
        assert_eq!(engine.elements_offset, 7614);
        assert_eq!(engine.universal_frame_counter, 173);
        assert_eq!(engine.creation_counter, 87);
        assert_eq!(engine.short_briefings.primaries.len(), 1);
        assert_eq!(engine.short_briefings.secondaries.len(), 1);
        assert_eq!(
            engine
                .sound
                .state
                .as_ref()
                .unwrap()
                .source_manager
                .slots
                .len(),
            8
        );
        assert!(engine.elements_offset < std::fs::metadata(path).unwrap().len());
    }

    #[test]
    #[ignore = "requires Linux i386 v48 profile saves via ROBINHOOD_DATA_DIR; select fixtures-legacy-linux"]
    fn rejects_sound_source_count_before_allocation() {
        let path = original_data::data_file("Data/Savegame/Profile_000/Restart");
        let path = path.to_string_lossy();
        let mut file = SbFile::open(&path).unwrap();
        let mut reader = LegacyReader::new(&mut file);
        let header = LegacySaveHeader::read(&mut reader).unwrap();
        LegacySaveCampaigns::read(&mut reader, &LegacyCampaignLimits::default()).unwrap();
        let limits = LegacyEngineLimits {
            sound_sources: 0,
            ..Default::default()
        };
        let error =
            LegacyEnginePreamble::read(&mut reader, header.abi_profile, &limits).unwrap_err();
        assert_eq!(error.field, "rhsg.engine.sound.source_manager.slot_count");
        assert!(error.to_string().contains("caller-supplied limit"));
    }
}

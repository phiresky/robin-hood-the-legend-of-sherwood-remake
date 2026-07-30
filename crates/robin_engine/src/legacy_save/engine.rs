//! Decoder for the scalar prefix of Original v48 `RHEngine::Serialize`.
//!
//! This stops at the exact byte where `RHEngine::SerializeElements` starts.
//! Element decoding belongs to a later importer milestone; no scan or guessed
//! byte skip is used to find that boundary.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

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
            short_briefings: 4096,
            sound_sources: 4096,
            sound_source_shape_points: 65535,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyEnginePreamble {
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
    pub short_briefings: LegacyShortBriefings,
    pub sound: LegacySound,
    pub messenger: LegacyMessenger,
    pub game: LegacyGameState,
    /// Exact first byte consumed by `RHEngine::SerializeElements`.
    pub elements_offset: u64,
}

impl LegacyEnginePreamble {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyEngineLimits,
    ) -> LegacyResult<Self> {
        reader.scope("rhsg.engine", |reader| {
            let start_offset = reader.offset();
            let cheat_used_flags = reader.read_u32("cheat_used_flags")?;
            let shield_protected = reader.read_bool("shield_protected")?;
            let freeze_all = reader.read_bool("freeze_all")?;
            let view = LegacyPoint2::read(reader, "view")?;
            let zoom_factor = reader.read_f32("zoom_factor")?;
            let camera_slide = LegacyPoint2::read(reader, "camera_slide")?;
            let fixed_camera_speed = reader.read_u16("fixed_camera_speed")?;
            let speed = reader.read_f32("speed")?;
            let speed_index = reader.read_u16("speed_index")?;
            let desired_zoom_factor = reader.read_f32("desired_zoom_factor")?;
            let old_zoom_factor = reader.read_f32("old_zoom_factor")?;
            let background_transform = LegacyBackgroundTransform::read(reader, abi_profile)?;
            let universal_frame_counter = reader.read_u32("universal_frame_counter")?;
            let creation_counter = reader.read_u32("creation_counter")?;
            let repulsive_point_counter = reader.read_u32("repulsive_point_counter")?;
            let lock_engine = reader.read_bool("lock_engine")?;
            let mission_won = reader.read_bool("mission_won")?;
            let mission_won_first_time = reader.read_bool("mission_won_first_time")?;
            let camera_wanted = LegacyPoint2::read(reader, "camera_wanted")?;
            let locker = reader.read_bool("locker")?;
            let skip_data = reader.read_string("skip_data")?;
            let short_briefings = LegacyShortBriefings::read(reader, limits)?;
            let sound = LegacySound::read(reader, limits)?;
            let messenger = LegacyMessenger::read(reader)?;
            let game = LegacyGameState::read(reader)?;
            let elements_offset = reader.offset();

            Ok(Self {
                start_offset,
                cheat_used_flags,
                shield_protected,
                freeze_all,
                view,
                zoom_factor,
                camera_slide,
                fixed_camera_speed,
                speed,
                speed_index,
                desired_zoom_factor,
                old_zoom_factor,
                background_transform,
                universal_frame_counter,
                creation_counter,
                repulsive_point_counter,
                lock_engine,
                mission_won,
                mission_won_first_time,
                camera_wanted,
                locker,
                skip_data,
                short_briefings,
                sound,
                messenger,
                game,
                elements_offset,
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPoint2 {
    pub x: f32,
    pub y: f32,
}

impl LegacyPoint2 {
    fn read(reader: &mut LegacyReader<'_>, field: impl Into<String>) -> LegacyResult<Self> {
        reader.scope(field, |reader| {
            Ok(Self {
                x: reader.read_f32("x")?,
                y: reader.read_f32("y")?,
            })
        })
    }
}

/// Raw `RHbackgroundTransform` as emitted by the Original producer ABI.
///
/// The two padding members are deliberately retained: `CHECKVAR` writes the
/// complete C struct rather than serializing its logical members.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyBackgroundTransform {
    pub scroll_to_left: bool,
    pub scroll_to_up: bool,
    pub current_x_scrolling_level: u16,
    pub current_y_scrolling_level: u16,
    pub padding_before_surface_ids: [u8; 2],
    pub surface_id: u32,
    pub final_surface_id: u32,
    pub zoom_to_up: bool,
    pub zoom_to_down: bool,
    pub required_zoom_up: bool,
    pub required_zoom_down: bool,
    pub zoom_count: u16,
    pub number_of_zoom_steps: u16,
    pub x_scrolling_values: [f32; 32],
    pub y_scrolling_values: [f32; 32],
    pub current_zoom_level: u16,
    pub padding_before_zoom_values: [u8; 2],
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
        reader.scope("background_transform", |reader| {
            let start = reader.offset();
            // Audited MSVC Win32 and GCC i386 layouts agree for this struct.
            // Keep the profile match explicit so adding another producer ABI
            // cannot silently inherit this raw-layout assumption.
            match abi_profile {
                LegacySaveAbiProfile::RetailWindowsX86V48
                | LegacySaveAbiProfile::PortLinuxI386V48 => {}
            }

            let scroll_to_left = reader.read_bool("scroll_to_left")?;
            let scroll_to_up = reader.read_bool("scroll_to_up")?;
            let current_x_scrolling_level = reader.read_u16("current_x_scrolling_level")?;
            let current_y_scrolling_level = reader.read_u16("current_y_scrolling_level")?;
            let padding_before_surface_ids = read_array::<2>(reader, "padding_before_surface_ids")?;
            let surface_id = reader.read_u32("surface_id")?;
            let final_surface_id = reader.read_u32("final_surface_id")?;
            let zoom_to_up = reader.read_bool("zoom_to_up")?;
            let zoom_to_down = reader.read_bool("zoom_to_down")?;
            let required_zoom_up = reader.read_bool("required_zoom_up")?;
            let required_zoom_down = reader.read_bool("required_zoom_down")?;
            let zoom_count = reader.read_u16("zoom_count")?;
            let number_of_zoom_steps = reader.read_u16("number_of_zoom_steps")?;
            let x_scrolling_values = read_f32_array(reader, "x_scrolling_values")?;
            let y_scrolling_values = read_f32_array(reader, "y_scrolling_values")?;
            let current_zoom_level = reader.read_u16("current_zoom_level")?;
            let padding_before_zoom_values = read_array::<2>(reader, "padding_before_zoom_values")?;
            let zoom_values = read_f32_array(reader, "zoom_values")?;
            let center_zoom = LegacyPoint2::read(reader, "center_zoom")?;
            let clipped_zoom = LegacyPoint2::read(reader, "clipped_zoom")?;
            let scrolling = LegacyPoint2::read(reader, "scrolling")?;

            debug_assert_eq!(reader.offset() - start, Self::SERIALIZED_SIZE);
            Ok(Self {
                scroll_to_left,
                scroll_to_up,
                current_x_scrolling_level,
                current_y_scrolling_level,
                padding_before_surface_ids,
                surface_id,
                final_surface_id,
                zoom_to_up,
                zoom_to_down,
                required_zoom_up,
                required_zoom_down,
                zoom_count,
                number_of_zoom_steps,
                x_scrolling_values,
                y_scrolling_values,
                current_zoom_level,
                padding_before_zoom_values,
                zoom_values,
                center_zoom,
                clipped_zoom,
                scrolling,
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyShortBriefing {
    pub id: u32,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyShortBriefings {
    pub primaries: Vec<LegacyShortBriefing>,
    pub secondaries: Vec<LegacyShortBriefing>,
}

impl LegacyShortBriefings {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyEngineLimits) -> LegacyResult<Self> {
        reader.scope("short_briefings", |reader| {
            reader.read_signature(
                "fingerprint",
                RH_SHORT_BRIEFINGS_FINGERPRINT,
                "MD5(\"RHShortBriefings\")",
            )?;
            let primaries = read_short_briefing_vec(reader, "primaries", limits.short_briefings)?;
            let secondaries =
                read_short_briefing_vec(reader, "secondaries", limits.short_briefings)?;
            Ok(Self {
                primaries,
                secondaries,
            })
        })
    }
}

fn read_short_briefing_vec(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    limit: usize,
) -> LegacyResult<Vec<LegacyShortBriefing>> {
    reader.scope(field, |reader| {
        let count = reader.read_count_u32("count", limit)?;
        let mut values = try_vec(reader, "items", count)?;
        for index in 0..count {
            values.push(reader.scope(format!("items[{index}]"), |reader| {
                Ok(LegacyShortBriefing {
                    id: reader.read_u32("id")?,
                    done: reader.read_bool("done")?,
                })
            })?);
        }
        Ok(values)
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySound {
    /// `false` means the Original intentionally emitted no further sound data.
    pub serialized: bool,
    pub state: Option<LegacySerializedSound>,
}

impl LegacySound {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyEngineLimits) -> LegacyResult<Self> {
        reader.scope("sound", |reader| {
            let serialized = reader.read_bool("serialized")?;
            let state = if serialized {
                Some(LegacySerializedSound::read(reader, limits)?)
            } else {
                None
            };
            Ok(Self { serialized, state })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub loop_index: i16,
    pub stream_position: u32,
    pub source_manager: LegacySoundSourceManager,
}

impl LegacySerializedSound {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyEngineLimits) -> LegacyResult<Self> {
        reader.read_signature("fingerprint", RH_SOUND_FINGERPRINT, "MD5(\"RHSound\")")?;
        Ok(Self {
            sound_system_ready: reader.read_bool("sound_system_ready")?,
            three_d_sound: reader.read_bool("three_d_sound")?,
            active: reader.read_bool("active")?,
            geometry: LegacySoundGeometry::read(reader)?,
            music_mode: reader.read_u8("music_mode")?,
            dummy_channel: reader.read_i16("dummy_channel")?,
            quiet_mode_weight: reader.read_u32("quiet_mode_weight")?,
            alert_mode_weight: reader.read_u32("alert_mode_weight")?,
            fight_mode_weight: reader.read_u32("fight_mode_weight")?,
            // The member is signed, although the C++ call spells its width as
            // sizeof(UWORD). Both are exactly two bytes in the supported ABIs.
            loop_index: reader.read_i16("loop_index")?,
            stream_position: reader.read_u32("stream_position")?,
            source_manager: LegacySoundSourceManager::read(reader, limits)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySoundGeometry {
    pub listen_point: LegacyPoint2,
    pub zoom_factor: f32,
}

impl LegacySoundGeometry {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.scope("geometry", |reader| {
            reader.read_signature(
                "fingerprint",
                RH_SOUND_GEOMETRY_FINGERPRINT,
                "MD5(\"RHSoundGeometry\")",
            )?;
            Ok(Self {
                listen_point: LegacyPoint2::read(reader, "listen_point")?,
                zoom_factor: reader.read_f32("zoom_factor")?,
            })
        })
    }
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
                "MD5(\"RHSoundSourceManager\")",
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
            let mut slots = try_vec(reader, "slots", count)?;
            for index in 0..count {
                slots.push(reader.scope(format!("slots[{index}]"), |reader| {
                    let slot_index = reader.read_i16("slot_index")?;
                    if slot_index == -1 {
                        Ok(None)
                    } else {
                        Ok(Some(LegacySoundSourceSlot {
                            slot_index,
                            source: LegacySoundSource::read(reader, limits)?,
                            registration_id: reader.read_u32("registration_id")?,
                        }))
                    }
                })?);
            }
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub shape: Vec<LegacyPoint2>,
}

impl LegacySoundSource {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyEngineLimits) -> LegacyResult<Self> {
        reader.scope("source", |reader| {
            reader.read_signature(
                "fingerprint",
                RH_SOUND_SOURCE_FINGERPRINT,
                "MD5(\"RHSoundSource\")",
            )?;
            let kind = reader.read_u8("kind")?;
            let altitude = reader.read_u8("altitude")?;
            let id = reader.read_u32("id")?;
            let global = reader.read_bool("global")?;
            let inner_distance = reader.read_u16("inner_distance")?;
            let outer_distance = reader.read_u16("outer_distance")?;
            let noise_covering_distance = reader.read_u16("noise_covering_distance")?;
            let inner_volume = reader.read_u16("inner_volume")?;
            let outer_volume = reader.read_u16("outer_volume")?;
            let min_delay = reader.read_u16("min_delay")?;
            let max_delay = reader.read_u16("max_delay")?;
            let delay_stepping = reader.read_u16("delay_stepping")?;
            let timer = reader.read_u16("timer")?;
            let active_first = reader.read_bool("active_first")?;
            let active_second = reader.read_bool("active_second")?;
            let former_need_update = reader.read_bool("former_need_update")?;
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
            let mut shape = try_vec(reader, "shape", count)?;
            for index in 0..count {
                shape.push(LegacyPoint2::read(reader, format!("shape[{index}]"))?);
            }
            Ok(Self {
                kind,
                altitude,
                id,
                global,
                inner_distance,
                outer_distance,
                noise_covering_distance,
                inner_volume,
                outer_volume,
                min_delay,
                max_delay,
                delay_stepping,
                timer,
                active_first,
                active_second,
                former_need_update,
                shape,
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyMessenger {
    pub lock_view: bool,
    pub setting_watch: bool,
    pub watch_timer: u16,
    pub action: u16,
    pub draw_hidden: bool,
}

impl LegacyMessenger {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.scope("messenger", |reader| {
            reader.read_signature(
                "fingerprint",
                RH_MESSENGER_FINGERPRINT,
                "MD5(\"RHMessenger\")",
            )?;
            Ok(Self {
                lock_view: reader.read_bool("lock_view")?,
                setting_watch: reader.read_bool("setting_watch")?,
                watch_timer: reader.read_u16("watch_timer")?,
                action: reader.read_u16("action")?,
                draw_hidden: reader.read_bool("draw_hidden")?,
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

impl LegacyGameState {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.scope("game", |reader| {
            reader.read_signature("fingerprint", RH_GAME_FINGERPRINT, "MD5(\"RHGame\")")?;
            Ok(Self {
                men_to_blazon_conversion: reader.read_bool("men_to_blazon_conversion")?,
                campaign_map: reader.read_bool("campaign_map")?,
                campaign_map_displayed: reader.read_bool("campaign_map_displayed")?,
                post_initialized: reader.read_bool("post_initialized")?,
                start_mission_disabled_temp: reader.read_bool("start_mission_disabled_temp")?,
                quit_mission_disabled_temp: reader.read_bool("quit_mission_disabled_temp")?,
                start_mission_enabled: reader.read_bool("start_mission_enabled")?,
                quit_mission_enabled: reader.read_bool("quit_mission_enabled")?,
            })
        })
    }
}

fn read_array<const N: usize>(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
) -> LegacyResult<[u8; N]> {
    let mut value = [0; N];
    reader.read_bytes(field, &mut value)?;
    Ok(value)
}

fn read_f32_array<const N: usize>(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
) -> LegacyResult<[f32; N]> {
    reader.scope(field, |reader| {
        let mut values = [0.0; N];
        for (index, value) in values.iter_mut().enumerate() {
            *value = reader.read_f32(format_args!("[{index}]"))?;
        }
        Ok(values)
    })
}

fn try_vec<T>(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    count: usize,
) -> LegacyResult<Vec<T>> {
    let offset = reader.offset();
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| reader.allocation_error(offset, field, count))?;
    Ok(values)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::legacy_save::LegacySaveHeader;
    use crate::legacy_save::campaign::{LegacyCampaignLimits, LegacySaveCampaigns};
    use crate::sbfile::{SB_FILE_READ, SbFile};

    fn repository_fixture(relative: &str) -> Option<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join(relative);
        path.is_file().then_some(path)
    }

    fn read_fixture(path: &Path) -> (LegacySaveHeader, LegacyEnginePreamble) {
        let path = path.to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
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
    fn golden_lincoln_restart_engine_boundary() {
        let Some(path) =
            repository_fixture("datadirs/fullgame_linux/Data/Savegame/Profile_000/Restart")
        else {
            return;
        };
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
    fn golden_nottingham_continue_engine_boundary() {
        let Some(path) =
            repository_fixture("datadirs/fullgame_linux/Data/Savegame/Profile_001/Continue")
        else {
            return;
        };
        let (header, engine) = read_fixture(&path);
        assert_eq!(header.abi_profile, LegacySaveAbiProfile::PortLinuxI386V48);
        assert_eq!(engine.start_offset, 7178);
        assert_eq!(engine.elements_offset, 8861);
        assert_eq!(engine.universal_frame_counter, 3852);
        assert_eq!(engine.creation_counter, 300);
        assert_eq!(engine.short_briefings.primaries.len(), 1);
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
            24
        );
        assert!(engine.elements_offset < std::fs::metadata(path).unwrap().len());
    }

    #[test]
    fn golden_retail_windows_engine_boundary() {
        let Some(path) =
            repository_fixture("reference-saves/Savegame_SuN1Sh1nE/Profile_004/Savegame_005")
        else {
            return;
        };
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
    fn rejects_sound_source_count_before_allocation() {
        let Some(path) =
            repository_fixture("datadirs/fullgame_linux/Data/Savegame/Profile_000/Restart")
        else {
            return;
        };
        let path = path.to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
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

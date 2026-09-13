//! Original-game v48 campaign decoder.
//!
//! Game serialization writes two consecutive campaign streams before the
//! engine: the restart backup and the live campaign. Neither stream has an
//! outer byte length, so every value below mirrors the original game's save order in
//! order. The returned `engine_offset` is therefore an independently checked
//! boundary, not a scan for the next checkpoint.

use super::read_helpers::DEFAULT_LIST_LIMIT;
use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyRead, LegacyReader, LegacyResult};

const CAMPAIGN_VALUE_COUNT: usize = 27;
const SKILL_COUNT: usize = 2;
const NULL_MISSION_INDEX: u16 = u16::MAX;
const NULL_PROFILE_INDEX: u32 = u32::MAX;
const RH_CAMPAIGN_FINGERPRINT: [u8; 16] = [
    0x42, 0x8d, 0x11, 0x6e, 0xaa, 0x94, 0x5a, 0xba, 0x47, 0x57, 0xbd, 0x37, 0x20, 0xc1, 0x40, 0x95,
];
const RH_MISSION_FINGERPRINT: [u8; 16] = [
    0x34, 0x51, 0x37, 0xa2, 0x8c, 0x77, 0xb2, 0xad, 0xf8, 0xfd, 0x26, 0xdc, 0xe9, 0xca, 0xa4, 0x15,
];
const RH_HUMAN_STATUS_FINGERPRINT: [u8; 16] = [
    0x7d, 0xb9, 0x62, 0xa4, 0x53, 0x63, 0x7c, 0x9e, 0x9b, 0xd6, 0xe9, 0xf4, 0x18, 0x38, 0xf5, 0xc1,
];
const RH_PC_STATUS_FINGERPRINT: [u8; 16] = [
    0x79, 0x16, 0xc6, 0x08, 0xb9, 0xa8, 0xe3, 0x9c, 0x38, 0x18, 0xf2, 0xb4, 0x89, 0x2f, 0xee, 0xde,
];
const RH_SECTOR_PRODUCTION_FINGERPRINT: [u8; 16] = [
    0xb9, 0x79, 0xa2, 0xcf, 0x62, 0xd1, 0x15, 0x4c, 0x7e, 0x77, 0x60, 0xf0, 0x3a, 0x2c, 0x17, 0x71,
];

/// Allocation and string bounds for a single v48 campaign stream.
///
/// These are deliberately supplied by the caller instead of inferred from
/// untrusted save bytes. Defaults are generous relative to shipped campaigns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyCampaignLimits {
    pub missions: usize,
    pub mission_links: usize,
    pub characters: usize,
    pub character_links: usize,
    pub production_sectors: usize,
    pub production_occupants: usize,
    pub collected_relics: usize,
    pub peasant_names: usize,
    pub last_played_missions: usize,
    pub wide_string_code_units: usize,
}

impl Default for LegacyCampaignLimits {
    fn default() -> Self {
        Self {
            missions: DEFAULT_LIST_LIMIT,
            mission_links: DEFAULT_LIST_LIMIT,
            characters: DEFAULT_LIST_LIMIT,
            character_links: DEFAULT_LIST_LIMIT,
            production_sectors: 64,
            production_occupants: DEFAULT_LIST_LIMIT,
            collected_relics: DEFAULT_LIST_LIMIT,
            peasant_names: 65535,
            last_played_missions: 3,
            wide_string_code_units: DEFAULT_LIST_LIMIT,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySaveCampaigns {
    pub backup: LegacyCampaignStream,
    pub live: LegacyCampaignStream,
    /// First byte of engine serialization.
    pub engine_offset: u64,
}

impl LegacySaveCampaigns {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacyCampaignLimits,
    ) -> LegacyResult<Self> {
        let backup = reader.scope("rhsg.backup_campaign", |reader| {
            LegacyCampaignStream::read(reader, limits)
        })?;
        let live = reader.scope("rhsg.live_campaign", |reader| {
            LegacyCampaignStream::read(reader, limits)
        })?;
        let engine_offset = reader.offset();
        Ok(Self {
            backup,
            live,
            engine_offset,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyCampaignStream {
    pub start_offset: u64,
    pub end_offset: u64,
    pub campaign: LegacyCampaign,
}

impl LegacyCampaignStream {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyCampaignLimits) -> LegacyResult<Self> {
        let start_offset = reader.offset();
        let campaign = LegacyCampaign::read(reader, limits)?;
        let end_offset = reader.offset();
        Ok(Self {
            start_offset,
            end_offset,
            campaign,
        })
    }
}

/// Field declaration order is wire order. Lists use [`read_vec`], whose
/// allocation errors report the count offset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyCampaignLimits,
    fingerprint = RH_CAMPAIGN_FINGERPRINT,
    expected = "campaign fingerprint"
)]
pub struct LegacyCampaign {
    pub reservists_are_back: bool,
    pub values: [i32; CAMPAIGN_VALUE_COUNT],
    pub ares: i8,
    #[legacy(with = read_vec, args(ctx.missions, |reader, _| LegacyMission::read(reader, &())))]
    pub missions: Vec<LegacyMission>,
    #[legacy(with = read_vec, args(ctx.mission_links, |reader, _| {
        read_mission_link(reader, "mission")
    }))]
    pub accessible_missions: Vec<Option<u16>>,
    #[legacy(with = read_vec, args(ctx.mission_links, |reader, _| {
        read_mission_link(reader, "mission")
    }))]
    pub pending_accessible_missions: Vec<Option<u16>>,
    #[legacy(with = read_vec, args(ctx.characters, |reader, _| {
        LegacyPcDescription::read(reader, &ctx.wide_string_code_units)
    }))]
    pub characters: Vec<LegacyPcDescription>,
    #[legacy(with = read_vec, args(ctx.character_links, |reader, _| reader.read_u32("character")))]
    pub gang: Vec<u32>,
    #[legacy(with = read_vec, args(ctx.character_links, |reader, _| reader.read_u32("character")))]
    pub reservists: Vec<u32>,
    #[legacy(with = read_vec, args(ctx.character_links, |reader, _| reader.read_u32("character")))]
    pub mission_team: Vec<u32>,
    #[legacy(with = read_vec, args(ctx.production_sectors, |reader, _| {
        LegacyProductionSector::read(reader, &ctx.production_occupants)
    }))]
    pub production_sectors: Vec<LegacyProductionSector>,
    #[legacy(with = read_vec, args(ctx.collected_relics, |reader, _| {
        reader.read_u32("object_type")
    }))]
    pub collected_relics: Vec<u32>,
    #[legacy(with = read_vec, args(ctx.peasant_names, |reader, _| {
        reader.read_wide_string("name", ctx.wide_string_code_units)
    }))]
    pub peasant_names: Vec<String>,
    #[legacy(with = read_mission_link)]
    pub last_mission: Option<u16>,
    #[legacy(with = read_mission_link)]
    pub current_mission: Option<u16>,
    #[legacy(with = read_mission_link)]
    pub next_mission: Option<u16>,
    #[legacy(with = read_mission_link)]
    pub blazon_mission: Option<u16>,
    #[legacy(with = read_vec, args(ctx.last_played_missions, |reader, _| {
        read_mission_link(reader, "mission")
    }))]
    pub last_played_missions: Vec<Option<u16>>,
    pub last_pseudo_mission_status: u32,
    pub last_pseudo_mission_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(fingerprint = RH_MISSION_FINGERPRINT, expected = "mission fingerprint")]
pub struct LegacyMission {
    pub age: u16,
    pub blazon_price: u16,
    pub status: u32,
    /// Four bytes skipped by mission serialization, retained explicitly.
    pub legacy_padding_words: [u16; 2],
    #[legacy(with = read_profile_link)]
    pub profile_index: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
pub struct LegacySkill {
    pub capacity: u32,
    pub experience: u32,
}

/// Context: the maximum name length in UTF-16 code units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = usize)]
pub struct LegacyPcStatus {
    #[legacy(read = read_campaign_skills(reader))]
    pub skills: [LegacySkill; SKILL_COUNT],
    #[legacy(fingerprint = RH_PC_STATUS_FINGERPRINT, expected = "MD5(\"RHPCStatus\")")]
    pub life_points: i16,
    pub in_coma: bool,
    pub ales: u16,
    pub apples: u16,
    pub arrows: u16,
    pub nets: u16,
    pub plants: u16,
    pub purses: u16,
    pub rations: u16,
    pub stones: u16,
    pub wasp_nests: u16,
    pub beam_me_index_in_sherwood: i16,
    #[legacy(read = reader.read_wide_string("name", *ctx))]
    pub name: String,
}

/// The embedded human status is reported as `human_status.*`.
fn read_campaign_skills(reader: &mut LegacyReader<'_>) -> LegacyResult<[LegacySkill; SKILL_COUNT]> {
    reader.scope("human_status", |reader| {
        reader.read_signature(
            "fingerprint",
            RH_HUMAN_STATUS_FINGERPRINT,
            "human-status fingerprint",
        )?;
        <[LegacySkill; SKILL_COUNT]>::read_field(reader, "skills", &())
    })
}

/// Context: the maximum name length in UTF-16 code units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = usize)]
pub struct LegacyPcDescription {
    pub status: LegacyPcStatus,
    #[legacy(with = read_profile_link)]
    pub character_profile_index: Option<u32>,
    pub instanced: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyProductionOccupant {
    pub character_index: u32,
    #[legacy(name = "position.x")]
    pub x: f32,
    #[legacy(name = "position.y")]
    pub y: f32,
    pub obstacle: u16,
}

/// Context: the maximum occupant count.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = usize,
    fingerprint = RH_SECTOR_PRODUCTION_FINGERPRINT,
    expected = "production-sector fingerprint"
)]
pub struct LegacyProductionSector {
    pub production_type: u32,
    pub speed: u16,
    pub amount: u16,
    pub produced_amount: u16,
    pub max_amount_reached: bool,
    #[legacy(with = read_vec, args(*ctx, |reader, _| LegacyProductionOccupant::read(reader, &())))]
    pub occupants: Vec<LegacyProductionOccupant>,
}

fn read_vec<T>(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    maximum: usize,
    mut read_item: impl FnMut(&mut LegacyReader<'_>, usize) -> LegacyResult<T>,
) -> LegacyResult<Vec<T>> {
    let count_offset = reader.offset();
    let count = reader.read_count_u32(format_args!("{field}.count"), maximum)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| reader.allocation_error(count_offset, field, count))?;
    for index in 0..count {
        values.push(reader.scope_indexed(field, index, |reader| read_item(reader, index))?);
    }
    Ok(values)
}

fn read_mission_link(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<Option<u16>> {
    let index = reader.read_u16(field)?;
    Ok((index != NULL_MISSION_INDEX).then_some(index))
}

fn read_profile_link(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<Option<u32>> {
    let index = reader.read_u32(field)?;
    Ok((index != NULL_PROFILE_INDEX).then_some(index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;
    use crate::test_support::with_reader;

    fn minimal_campaign_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&RH_CAMPAIGN_FINGERPRINT);
        bytes.push(0);
        bytes.extend_from_slice(&[0; CAMPAIGN_VALUE_COUNT * 4]);
        bytes.push(0xff);
        for _ in 0..10 {
            bytes.extend_from_slice(&0_u32.to_le_bytes());
        }
        for _ in 0..4 {
            bytes.extend_from_slice(&NULL_MISSION_INDEX.to_le_bytes());
        }
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes
    }

    #[test]
    fn parses_two_consecutive_campaigns_and_reports_engine_offset() {
        let mut bytes = minimal_campaign_bytes();
        bytes.extend_from_slice(&minimal_campaign_bytes());
        bytes.push(0xaa);

        with_reader(&bytes, |reader| {
            let campaigns =
                LegacySaveCampaigns::read(reader, &LegacyCampaignLimits::default()).unwrap();
            assert_eq!(campaigns.backup.start_offset, 0);
            assert_eq!(campaigns.backup.end_offset, campaigns.live.start_offset);
            assert_eq!(campaigns.engine_offset, bytes.len() as u64 - 1);
            assert_eq!(reader.read_u8("engine.first_byte").unwrap(), 0xaa);
        });
    }

    #[test]
    fn rejects_malicious_campaign_count_with_context() {
        let mut bytes = minimal_campaign_bytes();
        let mission_count_offset = 16 + 1 + CAMPAIGN_VALUE_COUNT * 4 + 1;
        bytes[mission_count_offset..mission_count_offset + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());

        let error = with_reader(&bytes, |reader| {
            LegacyCampaignStream::read(reader, &LegacyCampaignLimits::default()).unwrap_err()
        });
        assert_eq!(error.offset, mission_count_offset as u64);
        assert_eq!(error.field, "missions.count");
        assert!(matches!(
            error.kind,
            LegacyIoErrorKind::CountLimit {
                count: u32::MAX,
                ..
            }
        ));
    }

    #[test]
    fn reports_truncation_at_exact_campaign_field() {
        let bytes = minimal_campaign_bytes();
        let error = with_reader(&bytes[..bytes.len() - 1], |reader| {
            LegacyCampaignStream::read(reader, &LegacyCampaignLimits::default()).unwrap_err()
        });
        assert_eq!(error.field, "last_pseudo_mission_id");
        assert!(matches!(error.kind, LegacyIoErrorKind::SbFile(_)));
    }
}

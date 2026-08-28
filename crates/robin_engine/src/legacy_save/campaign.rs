//! Original v48 `RHCampaign::Serialize` decoder.
//!
//! `RHGame::Serialize` writes two consecutive campaign streams before the
//! engine: the restart backup and the live campaign. Neither stream has an
//! outer byte length, so every field below mirrors the Original serializer in
//! order. The returned `engine_offset` is therefore an independently checked
//! boundary, not a scan for the next checkpoint.

use enum_map::enum_map;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::campaign::{Campaign, CampaignValue, PcDescription};
use crate::legacy_io::{LegacyReader, LegacyResult};
use crate::mission::{Mission, MissionStatus};
use crate::pc_status::{HumanStatus, PcStatus, Skill};
use crate::profiles::{CharacterProfileIdx, ProfileManager};
use crate::sector_production::{Occupant, SectorProduction, Type};

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
            missions: 4096,
            mission_links: 4096,
            characters: 4096,
            character_links: 4096,
            production_sectors: 64,
            production_occupants: 4096,
            collected_relics: 4096,
            peasant_names: 65535,
            last_played_missions: 4096,
            wide_string_code_units: 4096,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySaveCampaigns {
    pub backup: LegacyCampaignStream,
    pub live: LegacyCampaignStream,
    /// First byte of `RHEngine::Serialize`.
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyCampaign {
    pub reservists_are_back: bool,
    pub values: [i32; CAMPAIGN_VALUE_COUNT],
    pub ares: i8,
    pub missions: Vec<LegacyMission>,
    pub accessible_missions: Vec<Option<u16>>,
    pub pending_accessible_missions: Vec<Option<u16>>,
    pub characters: Vec<LegacyPcDescription>,
    pub gang: Vec<u32>,
    pub reservists: Vec<u32>,
    pub mission_team: Vec<u32>,
    pub production_sectors: Vec<LegacyProductionSector>,
    pub collected_relics: Vec<u32>,
    pub peasant_names: Vec<String>,
    pub last_mission: Option<u16>,
    pub current_mission: Option<u16>,
    pub next_mission: Option<u16>,
    pub blazon_mission: Option<u16>,
    pub last_played_missions: Vec<Option<u16>>,
    pub last_pseudo_mission_status: u32,
    pub last_pseudo_mission_id: u32,
}

impl LegacyCampaign {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyCampaignLimits) -> LegacyResult<Self> {
        reader.read_signature(
            "fingerprint",
            RH_CAMPAIGN_FINGERPRINT,
            "MD5(\"RHCampaign\")",
        )?;
        let reservists_are_back = reader.read_bool("reservists_are_back")?;

        let mut values = [0; CAMPAIGN_VALUE_COUNT];
        for (index, value) in values.iter_mut().enumerate() {
            *value = reader.read_i32(format_args!("values[{index}]"))?;
        }
        let ares = reader.read_i8("ares")?;

        let missions = read_vec(reader, "missions", limits.missions, |reader, _| {
            LegacyMission::read(reader)
        })?;
        let accessible_missions = read_vec(
            reader,
            "accessible_missions",
            limits.mission_links,
            |reader, _| read_mission_link(reader, "mission"),
        )?;
        let pending_accessible_missions = read_vec(
            reader,
            "pending_accessible_missions",
            limits.mission_links,
            |reader, _| read_mission_link(reader, "mission"),
        )?;
        let characters = read_vec(reader, "characters", limits.characters, |reader, _| {
            LegacyPcDescription::read(reader, limits.wide_string_code_units)
        })?;
        let gang = read_vec(reader, "gang", limits.character_links, |reader, _| {
            reader.read_u32("character")
        })?;
        let reservists = read_vec(reader, "reservists", limits.character_links, |reader, _| {
            reader.read_u32("character")
        })?;
        let mission_team = read_vec(
            reader,
            "mission_team",
            limits.character_links,
            |reader, _| reader.read_u32("character"),
        )?;
        let production_sectors = read_vec(
            reader,
            "production_sectors",
            limits.production_sectors,
            |reader, _| LegacyProductionSector::read(reader, limits.production_occupants),
        )?;
        let collected_relics = read_vec(
            reader,
            "collected_relics",
            limits.collected_relics,
            |reader, _| reader.read_u32("object_type"),
        )?;
        let peasant_names = read_vec(
            reader,
            "peasant_names",
            limits.peasant_names,
            |reader, _| reader.read_wide_string("name", limits.wide_string_code_units),
        )?;

        let last_mission = read_mission_link(reader, "last_mission")?;
        let current_mission = read_mission_link(reader, "current_mission")?;
        let next_mission = read_mission_link(reader, "next_mission")?;
        let blazon_mission = read_mission_link(reader, "blazon_mission")?;
        let last_played_missions = read_vec(
            reader,
            "last_played_missions",
            limits.last_played_missions,
            |reader, _| read_mission_link(reader, "mission"),
        )?;
        let last_pseudo_mission_status = reader.read_u32("last_pseudo_mission_status")?;
        let last_pseudo_mission_id = reader.read_u32("last_pseudo_mission_id")?;

        Ok(Self {
            reservists_are_back,
            values,
            ares,
            missions,
            accessible_missions,
            pending_accessible_missions,
            characters,
            gang,
            reservists,
            mission_team,
            production_sectors,
            collected_relics,
            peasant_names,
            last_mission,
            current_mission,
            next_mission,
            blazon_mission,
            last_played_missions,
            last_pseudo_mission_status,
            last_pseudo_mission_id,
        })
    }

    /// Validate every serialized reference and construct the matching Rust
    /// campaign. The save header mission id is resolved through static
    /// profiles and reported with its proto/mission filenames.
    pub fn bootstrap(
        &self,
        profiles: &ProfileManager,
        header_mission_id: u32,
    ) -> Result<LegacyCampaignBootstrap, LegacyCampaignMappingError> {
        let header_profile_index = profiles
            .missions
            .iter()
            .position(|profile| profile.id == header_mission_id)
            .ok_or(LegacyCampaignMappingError::MissingHeaderMissionProfile {
                mission_id: header_mission_id,
            })?;
        let campaign_mission_index = self
            .missions
            .iter()
            .position(|mission| mission.profile_index == Some(header_profile_index as u32))
            .ok_or(
                LegacyCampaignMappingError::HeaderMissionMissingFromCampaign {
                    mission_id: header_mission_id,
                    profile_index: header_profile_index,
                },
            )?;

        let missions = self
            .missions
            .iter()
            .enumerate()
            .map(|(index, mission)| mission.to_rust(profiles, index))
            .collect::<Result<Vec<_>, _>>()?;
        let characters = self
            .characters
            .iter()
            .enumerate()
            .map(|(index, character)| character.to_rust(profiles, index))
            .collect::<Result<Vec<_>, _>>()?;
        let mission_count = missions.len();
        let character_count = characters.len();

        let accessible_mission_indices =
            map_required_mission_links(&self.accessible_missions, mission_count, "accessible")?;
        let pending_accessible_mission_indices = map_required_mission_links(
            &self.pending_accessible_missions,
            mission_count,
            "pending_accessible",
        )?;
        let gang_indices = map_character_links(&self.gang, character_count, "gang")?;
        let reservist_indices =
            map_character_links(&self.reservists, character_count, "reservists")?;
        let mission_team_indices =
            map_character_links(&self.mission_team, character_count, "mission_team")?;
        let last_played_mission_indices =
            map_required_mission_links(&self.last_played_missions, mission_count, "last_played")?;
        let production_sectors = self
            .production_sectors
            .iter()
            .enumerate()
            .map(|(index, sector)| sector.to_rust(character_count, index))
            .collect::<Result<Vec<_>, _>>()?;

        let mut campaign = Campaign {
            values: enum_map! {
                CampaignValue::Amulets => self.values[0],
                CampaignValue::Ransom => self.values[1],
                CampaignValue::Score => self.values[2],
                CampaignValue::Blazon => self.values[3],
                CampaignValue::LivingSoldiers => self.values[4],
                CampaignValue::DeadSoldiers => self.values[5],
                CampaignValue::MissionLength => self.values[6],
                CampaignValue::Custom1 => self.values[7],
                CampaignValue::Custom2 => self.values[8],
                CampaignValue::Custom3 => self.values[9],
                CampaignValue::Custom4 => self.values[10],
                CampaignValue::Custom5 => self.values[11],
                CampaignValue::Custom6 => self.values[12],
                CampaignValue::Custom7 => self.values[13],
                CampaignValue::Custom8 => self.values[14],
                CampaignValue::Custom9 => self.values[15],
                CampaignValue::Custom10 => self.values[16],
                CampaignValue::Custom11 => self.values[17],
                CampaignValue::Custom12 => self.values[18],
                CampaignValue::Custom13 => self.values[19],
                CampaignValue::Custom14 => self.values[20],
                CampaignValue::Custom15 => self.values[21],
                CampaignValue::Custom16 => self.values[22],
                CampaignValue::Custom17 => self.values[23],
                CampaignValue::Custom18 => self.values[24],
                CampaignValue::Custom19 => self.values[25],
                CampaignValue::Custom20 => self.values[26],
            }
            .into(),
            ares: self.ares,
            missions,
            accessible_mission_indices,
            pending_accessible_mission_indices,
            last_mission_idx: map_optional_mission_link(
                self.last_mission,
                mission_count,
                "last_mission",
            )?,
            current_mission_idx: map_optional_mission_link(
                self.current_mission,
                mission_count,
                "current_mission",
            )?,
            next_mission_idx: map_optional_mission_link(
                self.next_mission,
                mission_count,
                "next_mission",
            )?,
            blazon_mission_idx: map_optional_mission_link(
                self.blazon_mission,
                mission_count,
                "blazon_mission",
            )?,
            last_played_mission_indices,
            last_pseudo_mission_status: map_mission_status(
                self.last_pseudo_mission_status,
                "last_pseudo_mission_status",
            )?,
            last_pseudo_mission_id: self.last_pseudo_mission_id,
            earned_achievements: crate::achievement::AchievementSet::empty(),
            mission_attempt_sequence: 0,
            campaign_history_run_id: None,
            history_replay_mission_idx: None,
            characters,
            gang_indices,
            reservist_indices,
            mission_team_indices,
            peasant_names: self.peasant_names.clone(),
            reservists_are_back: self.reservists_are_back,
            collected_relics: self.collected_relics.clone(),
            production_sectors,
            pre_mission_snapshot: None,
            pre_mission_rng_seed: None,
            pre_mission_sim_config: None,
            pre_mission_was_preselected: false,
        };
        campaign.migrate_legacy_aggregate_history();

        let profile = &profiles.missions[header_profile_index];
        Ok(LegacyCampaignBootstrap {
            campaign,
            identity: LegacyMissionIdentity {
                mission_id: header_mission_id,
                campaign_mission_index,
                profile_index: header_profile_index,
                proto_level_filename: profile.proto_level_filename.clone(),
                mission_filename: profile.mission_filename.clone(),
                mission_name: profile.mission_name.clone(),
            },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyMission {
    pub age: u16,
    pub blazon_price: u16,
    pub status: u32,
    /// Four bytes skipped by `RHMission::Serialize`, retained explicitly.
    pub legacy_padding_words: [u16; 2],
    pub profile_index: Option<u32>,
}

impl LegacyMission {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.read_signature("fingerprint", RH_MISSION_FINGERPRINT, "MD5(\"RHMission\")")?;
        Ok(Self {
            age: reader.read_u16("age")?,
            blazon_price: reader.read_u16("blazon_price")?,
            status: reader.read_u32("status")?,
            legacy_padding_words: [
                reader.read_u16("legacy_padding_words[0]")?,
                reader.read_u16("legacy_padding_words[1]")?,
            ],
            profile_index: read_profile_link(reader, "profile_index")?,
        })
    }

    fn to_rust(
        &self,
        profiles: &ProfileManager,
        mission_index: usize,
    ) -> Result<Mission, LegacyCampaignMappingError> {
        let profile_idx = required_profile_link(
            self.profile_index,
            profiles.missions.len(),
            format!("missions[{mission_index}].profile_index"),
        )?;
        Ok(Mission {
            age: self.age,
            blazon_price: self.blazon_price,
            status: map_mission_status(self.status, format!("missions[{mission_index}].status"))?,
            profile_idx: Some(profile_idx as u32),
            ares_state_override: None,
            achievement_history: crate::achievement::MissionAchievementHistory::default(),
            attempt_history: crate::campaign_history::MissionAttemptHistory::default(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySkill {
    pub capacity: u32,
    pub experience: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPcStatus {
    pub skills: [LegacySkill; SKILL_COUNT],
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
    pub name: String,
}

impl LegacyPcStatus {
    fn read(reader: &mut LegacyReader<'_>, maximum_name_units: usize) -> LegacyResult<Self> {
        reader.read_signature(
            "human_status.fingerprint",
            RH_HUMAN_STATUS_FINGERPRINT,
            "MD5(\"RHHumanStatus\")",
        )?;
        let skills = [
            LegacySkill {
                capacity: reader.read_u32("human_status.skills[0].capacity")?,
                experience: reader.read_u32("human_status.skills[0].experience")?,
            },
            LegacySkill {
                capacity: reader.read_u32("human_status.skills[1].capacity")?,
                experience: reader.read_u32("human_status.skills[1].experience")?,
            },
        ];
        reader.read_signature(
            "fingerprint",
            RH_PC_STATUS_FINGERPRINT,
            "MD5(\"RHPCStatus\")",
        )?;
        Ok(Self {
            skills,
            life_points: reader.read_i16("life_points")?,
            in_coma: reader.read_bool("in_coma")?,
            ales: reader.read_u16("ales")?,
            apples: reader.read_u16("apples")?,
            arrows: reader.read_u16("arrows")?,
            nets: reader.read_u16("nets")?,
            plants: reader.read_u16("plants")?,
            purses: reader.read_u16("purses")?,
            rations: reader.read_u16("rations")?,
            stones: reader.read_u16("stones")?,
            wasp_nests: reader.read_u16("wasp_nests")?,
            beam_me_index_in_sherwood: reader.read_i16("beam_me_index_in_sherwood")?,
            name: reader.read_wide_string("name", maximum_name_units)?,
        })
    }

    fn to_rust(&self) -> PcStatus {
        PcStatus {
            human_status: HumanStatus {
                hand_to_hand: Skill {
                    capacity: self.skills[0].capacity,
                    experience: self.skills[0].experience,
                },
                bow: Skill {
                    capacity: self.skills[1].capacity,
                    experience: self.skills[1].experience,
                },
            },
            life_points: self.life_points,
            in_coma: self.in_coma,
            num_ales: self.ales,
            num_arrows: self.arrows,
            num_apples: self.apples,
            num_rations: self.rations,
            num_stones: self.stones,
            num_wasp_nests: self.wasp_nests,
            num_nets: self.nets,
            num_plants: self.plants,
            num_purses: self.purses,
            name: self.name.clone(),
            name_override: None,
            beam_me_index_in_sherwood: self.beam_me_index_in_sherwood,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPcDescription {
    pub status: LegacyPcStatus,
    pub character_profile_index: Option<u32>,
    pub instanced: bool,
}

impl LegacyPcDescription {
    fn read(reader: &mut LegacyReader<'_>, maximum_name_units: usize) -> LegacyResult<Self> {
        Ok(Self {
            status: reader.scope("status", |reader| {
                LegacyPcStatus::read(reader, maximum_name_units)
            })?,
            character_profile_index: read_profile_link(reader, "character_profile_index")?,
            instanced: reader.read_bool("instanced")?,
        })
    }

    fn to_rust(
        &self,
        profiles: &ProfileManager,
        character_index: usize,
    ) -> Result<PcDescription, LegacyCampaignMappingError> {
        let profile_index = required_profile_link(
            self.character_profile_index,
            profiles.characters.len(),
            format!("characters[{character_index}].character_profile_index"),
        )?;
        Ok(PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(profile_index as u32)),
            instanced: self.instanced,
            status: self.status.to_rust(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyProductionOccupant {
    pub character_index: u32,
    pub x: f32,
    pub y: f32,
    pub obstacle: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyProductionSector {
    pub production_type: u32,
    pub speed: u16,
    pub amount: u16,
    pub produced_amount: u16,
    pub max_amount_reached: bool,
    pub occupants: Vec<LegacyProductionOccupant>,
}

impl LegacyProductionSector {
    fn read(reader: &mut LegacyReader<'_>, maximum_occupants: usize) -> LegacyResult<Self> {
        reader.read_signature(
            "fingerprint",
            RH_SECTOR_PRODUCTION_FINGERPRINT,
            "MD5(\"RHSectorProduction\")",
        )?;
        let production_type = reader.read_u32("production_type")?;
        let speed = reader.read_u16("speed")?;
        let amount = reader.read_u16("amount")?;
        let produced_amount = reader.read_u16("produced_amount")?;
        let max_amount_reached = reader.read_bool("max_amount_reached")?;
        let occupants = read_vec(reader, "occupants", maximum_occupants, |reader, _| {
            Ok(LegacyProductionOccupant {
                character_index: reader.read_u32("character_index")?,
                x: reader.read_f32("position.x")?,
                y: reader.read_f32("position.y")?,
                obstacle: reader.read_u16("obstacle")?,
            })
        })?;
        Ok(Self {
            production_type,
            speed,
            amount,
            produced_amount,
            max_amount_reached,
            occupants,
        })
    }

    fn to_rust(
        &self,
        character_count: usize,
        sector_index: usize,
    ) -> Result<SectorProduction, LegacyCampaignMappingError> {
        let prod_type = Type::from_script_i32(self.production_type as i32).ok_or_else(|| {
            LegacyCampaignMappingError::InvalidEnum {
                field: format!("production_sectors[{sector_index}].production_type"),
                value: self.production_type,
                expected: "RHSectorProduction::Type 0..12",
            }
        })?;
        let occupants = self
            .occupants
            .iter()
            .enumerate()
            .map(|(occupant_index, occupant)| {
                let pc_description_idx = checked_reference(
                    occupant.character_index as usize,
                    character_count,
                    format!(
                        "production_sectors[{sector_index}].occupants[{occupant_index}].character_index"
                    ),
                )?;
                Ok(Occupant {
                    pc_description_idx,
                    x: occupant.x,
                    y: occupant.y,
                    obstacle: occupant.obstacle,
                })
            })
            .collect::<Result<Vec<_>, LegacyCampaignMappingError>>()?;
        Ok(SectorProduction {
            prod_type,
            script_zone: None,
            speed: self.speed,
            production_points: Vec::new(),
            occupants,
            amount: self.amount,
            produced_amount: self.produced_amount,
            max_amount_reached: self.max_amount_reached,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyMissionIdentity {
    pub mission_id: u32,
    pub campaign_mission_index: usize,
    pub profile_index: usize,
    pub proto_level_filename: String,
    pub mission_filename: String,
    pub mission_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyCampaignBootstrap {
    pub campaign: Campaign,
    pub identity: LegacyMissionIdentity,
}

#[derive(Debug, Error)]
pub enum LegacyCampaignMappingError {
    #[error("save header mission id {mission_id} has no matching static mission profile")]
    MissingHeaderMissionProfile { mission_id: u32 },
    #[error(
        "save header mission id {mission_id} (profile index {profile_index}) is absent from the campaign"
    )]
    HeaderMissionMissingFromCampaign {
        mission_id: u32,
        profile_index: usize,
    },
    #[error("{field} reference {index} is outside collection length {length}")]
    InvalidReference {
        field: String,
        index: usize,
        length: usize,
    },
    #[error("{field} is unexpectedly null")]
    NullReference { field: String },
    #[error("{field} has invalid enum value {value}; expected {expected}")]
    InvalidEnum {
        field: String,
        value: u32,
        expected: &'static str,
    },
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
        values.push(reader.scope(format!("{field}[{index}]"), |reader| {
            read_item(reader, index)
        })?);
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

fn checked_reference(
    index: usize,
    length: usize,
    field: impl Into<String>,
) -> Result<usize, LegacyCampaignMappingError> {
    if index < length {
        Ok(index)
    } else {
        Err(LegacyCampaignMappingError::InvalidReference {
            field: field.into(),
            index,
            length,
        })
    }
}

fn required_profile_link(
    index: Option<u32>,
    length: usize,
    field: impl Into<String>,
) -> Result<usize, LegacyCampaignMappingError> {
    let field = field.into();
    let index = index.ok_or_else(|| LegacyCampaignMappingError::NullReference {
        field: field.clone(),
    })?;
    checked_reference(index as usize, length, field)
}

fn map_optional_mission_link(
    index: Option<u16>,
    length: usize,
    field: impl Into<String>,
) -> Result<Option<usize>, LegacyCampaignMappingError> {
    index
        .map(|index| checked_reference(index as usize, length, field))
        .transpose()
}

fn map_required_mission_links(
    links: &[Option<u16>],
    length: usize,
    field: &str,
) -> Result<Vec<usize>, LegacyCampaignMappingError> {
    links
        .iter()
        .enumerate()
        .map(|(position, link)| {
            let item_field = format!("{field}[{position}]");
            let index = link.ok_or_else(|| LegacyCampaignMappingError::NullReference {
                field: item_field.clone(),
            })?;
            checked_reference(index as usize, length, item_field)
        })
        .collect()
}

fn map_character_links(
    links: &[u32],
    length: usize,
    field: &str,
) -> Result<Vec<usize>, LegacyCampaignMappingError> {
    links
        .iter()
        .enumerate()
        .map(|(position, &index)| {
            checked_reference(index as usize, length, format!("{field}[{position}]"))
        })
        .collect()
}

fn map_mission_status(
    value: u32,
    field: impl Into<String>,
) -> Result<MissionStatus, LegacyCampaignMappingError> {
    match value {
        0 => Ok(MissionStatus::Available),
        1 => Ok(MissionStatus::Won),
        2 => Ok(MissionStatus::Lost),
        _ => Err(LegacyCampaignMappingError::InvalidEnum {
            field: field.into(),
            value,
            expected: "RHMission::Status 0..2",
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use tempfile::NamedTempFile;

    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;
    use crate::legacy_save::{
        LegacySaveAbiProfile, LegacySaveHeader, PORT_LINUX_I386_MAGIC, RETAIL_WINDOWS_X86_MAGIC,
    };
    use crate::sbfile::{SB_FILE_READ, SbFile};

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut fixture = NamedTempFile::new().unwrap();
        fixture.write_all(bytes).unwrap();
        fixture.flush().unwrap();
        let path = fixture.path().to_string_lossy().into_owned();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

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
        assert!(matches!(error.kind, LegacyIoErrorKind::SbFile { .. }));
    }

    fn repository_fixture(relative: &str) -> Option<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join(relative);
        path.is_file().then_some(path)
    }

    fn read_fixture(path: &Path) -> (LegacySaveHeader, LegacySaveCampaigns) {
        let path = path.to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut reader = LegacyReader::new(&mut file);
        let header = LegacySaveHeader::read(&mut reader).unwrap();
        let campaigns =
            LegacySaveCampaigns::read(&mut reader, &LegacyCampaignLimits::default()).unwrap();
        (header, campaigns)
    }

    fn read_fixture_profiles() -> Option<ProfileManager> {
        let path = repository_fixture("datadirs/fullgame_linux/Data/Configuration/profile.cpf")?;
        let path = path.to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut profiles = ProfileManager::new();
        profiles.load_all_legacy_cpf(&mut file).unwrap();
        Some(profiles)
    }

    #[test]
    fn parses_current_linux_continue_campaign_boundaries() {
        let Some(path) =
            repository_fixture("datadirs/fullgame_linux/Data/Savegame/Profile_001/Continue")
        else {
            return;
        };
        let (header, campaigns) = read_fixture(&path);
        assert_eq!(header.magic, PORT_LINUX_I386_MAGIC);
        assert_eq!(header.abi_profile, LegacySaveAbiProfile::PortLinuxI386V48);
        // `Continue` is live profile state and changes whenever that data
        // directory is played. Keep exact golden offsets on the immutable
        // Restart/archive fixtures; here verify structural boundaries.
        assert_eq!(campaigns.backup.start_offset, 16);
        assert_eq!(campaigns.live.start_offset, campaigns.backup.end_offset);
        assert_eq!(campaigns.engine_offset, campaigns.live.end_offset);
        assert_eq!(campaigns.backup.campaign.missions.len(), 63);
        assert_eq!(campaigns.live.campaign.missions.len(), 63);
        assert!(!campaigns.backup.campaign.characters.is_empty());
        assert!(!campaigns.live.campaign.characters.is_empty());
        if let Some(profiles) = read_fixture_profiles() {
            let bootstrap = campaigns
                .live
                .campaign
                .bootstrap(&profiles, header.mission_id)
                .unwrap();
            assert_eq!(bootstrap.identity.mission_id, header.mission_id);
            assert_eq!(
                Some(bootstrap.identity.campaign_mission_index),
                campaigns.live.campaign.current_mission.map(usize::from)
            );
            assert!(!bootstrap.identity.proto_level_filename.is_empty());
            assert!(!bootstrap.identity.mission_filename.is_empty());
            assert!(!bootstrap.campaign.characters.is_empty());
        }
        assert!(campaigns.engine_offset < std::fs::metadata(path).unwrap().len());
    }

    #[test]
    fn golden_lincoln_restart_campaign_boundaries() {
        let Some(path) =
            repository_fixture("datadirs/fullgame_linux/Data/Savegame/Profile_000/Restart")
        else {
            return;
        };
        let (header, campaigns) = read_fixture(&path);
        assert_eq!(header.magic, PORT_LINUX_I386_MAGIC);
        assert_eq!(header.abi_profile, LegacySaveAbiProfile::PortLinuxI386V48);
        assert_eq!(header.mission_id, 16712);
        assert_eq!(campaigns.backup.start_offset, 16);
        assert_eq!(campaigns.backup.end_offset, 2729);
        assert_eq!(campaigns.live.start_offset, 2729);
        assert_eq!(campaigns.live.end_offset, 5442);
        assert_eq!(campaigns.engine_offset, 5442);
        assert_eq!(campaigns.backup.campaign.missions.len(), 63);
        assert_eq!(campaigns.live.campaign.missions.len(), 63);
        assert_eq!(campaigns.backup.campaign.characters.len(), 1);
        assert_eq!(campaigns.live.campaign.characters.len(), 1);
        assert_eq!(campaigns.live.campaign.current_mission, Some(21));
        if let Some(profiles) = read_fixture_profiles() {
            let bootstrap = campaigns
                .live
                .campaign
                .bootstrap(&profiles, header.mission_id)
                .unwrap();
            assert_eq!(bootstrap.identity.mission_id, 16712);
            assert_eq!(bootstrap.identity.campaign_mission_index, 21);
            assert!(!bootstrap.identity.proto_level_filename.is_empty());
            assert!(!bootstrap.identity.mission_filename.is_empty());
            assert_eq!(bootstrap.campaign.characters.len(), 1);
        }
        assert!(campaigns.engine_offset < std::fs::metadata(path).unwrap().len());
    }

    #[test]
    fn golden_retail_windows_campaign_boundaries() {
        let Some(path) =
            repository_fixture("reference-saves/Savegame_SuN1Sh1nE/Profile_004/Savegame_005")
        else {
            return;
        };
        let (header, campaigns) = read_fixture(&path);
        assert_eq!(header.magic, RETAIL_WINDOWS_X86_MAGIC);
        assert_eq!(
            header.abi_profile,
            LegacySaveAbiProfile::RetailWindowsX86V48
        );
        assert_eq!(header.header_version, 48);
        assert_eq!(header.mission_id, 20808);
        assert_eq!(header.stream_version, 48);
        assert_eq!(campaigns.backup.start_offset, 16);
        assert_eq!(campaigns.backup.end_offset, 3347);
        assert_eq!(campaigns.live.start_offset, 3347);
        assert_eq!(campaigns.live.end_offset, 6678);
        assert_eq!(campaigns.engine_offset, 6678);
        assert_eq!(campaigns.backup.campaign.missions.len(), 63);
        assert_eq!(campaigns.live.campaign.missions.len(), 63);
        assert_eq!(campaigns.backup.campaign.characters.len(), 6);
        assert_eq!(campaigns.live.campaign.characters.len(), 6);
        assert_eq!(campaigns.live.campaign.current_mission, Some(0));
        assert!(campaigns.engine_offset < std::fs::metadata(path).unwrap().len());
    }
}

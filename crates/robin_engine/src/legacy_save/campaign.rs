//! Conversion of decoded campaign records into simulation state.

use super::adopt_common::{AdoptErrorKind, AdoptSite, LegacyAdoptError};
use crate::campaign::{Campaign, CampaignValue, PcDescription};
use crate::mission::{Mission, MissionStatus};
use crate::pc_status::{HumanStatus, PcStatus, Skill};
use crate::profiles::{CharacterProfileIdx, ProfileManager};
use crate::sector_production::{Occupant, SectorProduction, Type};
use enum_map::enum_map;
pub use robin_legacy_save::campaign::*;
use serde::{Deserialize, Serialize};

pub(super) trait LegacyCampaignAdoption {
    fn bootstrap(
        &self,
        profiles: &ProfileManager,
        header_mission_id: u32,
    ) -> Result<LegacyCampaignBootstrap, LegacyAdoptError>;
}

impl LegacyCampaignAdoption for LegacyCampaign {
    /// Validate every serialized reference and construct the matching Rust
    /// campaign. The save header mission id is resolved through static
    /// profiles and reported with its proto/mission filenames.
    fn bootstrap(
        &self,
        profiles: &ProfileManager,
        header_mission_id: u32,
    ) -> Result<LegacyCampaignBootstrap, LegacyAdoptError> {
        let header_profile_index = profiles
            .missions
            .iter()
            .position(|profile| profile.id == header_mission_id)
            .ok_or(AdoptErrorKind::MissingHeaderMissionProfile {
                mission_id: header_mission_id,
            })?;
        let campaign_mission_index = self
            .missions
            .iter()
            .position(|mission| mission.profile_index == Some(header_profile_index as u32))
            .ok_or(AdoptErrorKind::HeaderMissionMissingFromCampaign {
                mission_id: header_mission_id,
                profile_index: header_profile_index,
            })?;

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

        if self.last_played_missions.len() > 3 {
            return Err(AdoptErrorKind::TooManyRecentMissions {
                count: self.last_played_missions.len(),
            }
            .into());
        }

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
            deeds: crate::achievement::CampaignDeeds {
                complete_evidence: false,
                ..Default::default()
            },
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
            last_pseudo_mission_status: map_mission_status(
                self.last_pseudo_mission_status,
                "last_pseudo_mission_status",
            )?,
            last_pseudo_mission_id: self.last_pseudo_mission_id,
            mission_attempt_sequence: 0,
            campaign_history_run_id: None,
            history_replay_mission_idx: None,
            practice_return_snapshot: None,
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
        campaign.reconstruct_original_save_history(&last_played_mission_indices);

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

trait LegacyMissionAdoption {
    fn to_rust(
        &self,
        profiles: &ProfileManager,
        mission_index: usize,
    ) -> Result<Mission, LegacyAdoptError>;
}

impl LegacyMissionAdoption for LegacyMission {
    fn to_rust(
        &self,
        profiles: &ProfileManager,
        mission_index: usize,
    ) -> Result<Mission, LegacyAdoptError> {
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
            attempt_history: crate::campaign_history::MissionAttemptHistory::default(),
        })
    }
}

trait LegacyPcStatusAdoption {
    fn to_rust(&self) -> PcStatus;
}

impl LegacyPcStatusAdoption for LegacyPcStatus {
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

trait LegacyPcDescriptionAdoption {
    fn to_rust(
        &self,
        profiles: &ProfileManager,
        character_index: usize,
    ) -> Result<PcDescription, LegacyAdoptError>;
}

impl LegacyPcDescriptionAdoption for LegacyPcDescription {
    fn to_rust(
        &self,
        profiles: &ProfileManager,
        character_index: usize,
    ) -> Result<PcDescription, LegacyAdoptError> {
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

trait LegacyProductionSectorAdoption {
    fn to_rust(
        &self,
        character_count: usize,
        sector_index: usize,
    ) -> Result<SectorProduction, LegacyAdoptError>;
}

impl LegacyProductionSectorAdoption for LegacyProductionSector {
    fn to_rust(
        &self,
        character_count: usize,
        sector_index: usize,
    ) -> Result<SectorProduction, LegacyAdoptError> {
        let prod_type = Type::from_script_i32(self.production_type as i32).ok_or_else(|| {
            CAMPAIGN.invalid(
                format!("production_sectors[{sector_index}].production_type"),
                self.production_type,
                "production sector type 0..12",
            )
        })?;
        let occupants = self
            .occupants
            .iter()
            .enumerate()
            .map(|(occupant_index, occupant)| {
                let pc_description_idx = checked_collection_index(
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
                    obstacle: crate::position_interface::ObstacleHandle::from_serialized_pointer(
                        occupant.obstacle,
                    ),
                })
            })
            .collect::<Result<Vec<_>, LegacyAdoptError>>()?;
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

/// Error context for campaign-stream reference and enum validation.
const CAMPAIGN: AdoptSite = AdoptSite::new("saved campaign");

fn checked_collection_index(
    index: usize,
    length: usize,
    field: impl Into<String>,
) -> Result<usize, LegacyAdoptError> {
    if index < length {
        Ok(index)
    } else {
        Err(CAMPAIGN.out_of_range(field.into(), "index", index, length))
    }
}

fn required_profile_link(
    index: Option<u32>,
    length: usize,
    field: impl Into<String>,
) -> Result<usize, LegacyAdoptError> {
    let field = field.into();
    let index =
        index.ok_or_else(|| CAMPAIGN.field_error(field.clone(), AdoptErrorKind::NullReference))?;
    checked_collection_index(index as usize, length, field)
}

fn map_optional_mission_link(
    index: Option<u16>,
    length: usize,
    field: impl Into<String>,
) -> Result<Option<usize>, LegacyAdoptError> {
    index
        .map(|index| checked_collection_index(index as usize, length, field))
        .transpose()
}

fn map_required_mission_links(
    links: &[Option<u16>],
    length: usize,
    field: &str,
) -> Result<Vec<usize>, LegacyAdoptError> {
    links
        .iter()
        .enumerate()
        .map(|(position, link)| {
            let item_field = format!("{field}[{position}]");
            let index = link.ok_or_else(|| {
                CAMPAIGN.field_error(item_field.clone(), AdoptErrorKind::NullReference)
            })?;
            checked_collection_index(index as usize, length, item_field)
        })
        .collect()
}

fn map_character_links(
    links: &[u32],
    length: usize,
    field: &str,
) -> Result<Vec<usize>, LegacyAdoptError> {
    links
        .iter()
        .enumerate()
        .map(|(position, &index)| {
            checked_collection_index(index as usize, length, format!("{field}[{position}]"))
        })
        .collect()
}

fn map_mission_status(
    value: u32,
    field: impl Into<String>,
) -> Result<MissionStatus, LegacyAdoptError> {
    match value {
        0 => Ok(MissionStatus::Available),
        1 => Ok(MissionStatus::Won),
        2 => Ok(MissionStatus::Lost),
        _ => Err(CAMPAIGN.invalid(field.into(), value, "mission status 0..2")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_io::LegacyReader;
    use crate::legacy_save::{
        LegacySaveAbiProfile, LegacySaveHeader, PORT_LINUX_I386_MAGIC, RETAIL_WINDOWS_X86_MAGIC,
    };
    use crate::sbfile::SbFile;
    use std::path::{Path, PathBuf};

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

    fn read_fixture(path: &Path) -> (LegacySaveHeader, LegacySaveCampaigns) {
        let path = path.to_string_lossy();
        let mut file = SbFile::open(&path).unwrap();
        let mut reader = LegacyReader::new(&mut file);
        let header = LegacySaveHeader::read(&mut reader).unwrap();
        let campaigns =
            LegacySaveCampaigns::read(&mut reader, &LegacyCampaignLimits::default()).unwrap();
        (header, campaigns)
    }

    fn read_fixture_profiles() -> ProfileManager {
        let path = original_data::data_file("Data/Configuration/profile.cpf");
        let path = path.to_string_lossy();
        let mut file = SbFile::open(&path).unwrap();
        let mut profiles = ProfileManager::new();
        profiles.load_all_legacy_cpf(&mut file).unwrap();
        profiles
    }

    fn assert_original_import_history(campaign: &Campaign, original: &LegacyCampaign) {
        campaign.validate_history_schema().unwrap();
        let mut imported_recent = campaign
            .missions
            .iter()
            .enumerate()
            .flat_map(|(mission_index, mission)| {
                mission
                    .attempt_history()
                    .attempts()
                    .iter()
                    .filter(|attempt| {
                        attempt.outcome() == crate::campaign_history::MissionAttemptOutcome::Unknown
                    })
                    .map(move |attempt| (attempt.sequence(), mission_index))
            })
            .collect::<Vec<_>>();
        imported_recent.sort_unstable();
        assert_eq!(
            imported_recent
                .iter()
                .map(|(_, mission_index)| *mission_index)
                .collect::<Vec<_>>(),
            original
                .last_played_missions
                .iter()
                .map(|mission| usize::from(mission.expect("Original recent mission is required")))
                .collect::<Vec<_>>()
        );
        for attempt in campaign
            .missions
            .iter()
            .flat_map(|mission| mission.attempt_history().attempts())
        {
            assert_eq!(
                attempt.source(),
                crate::campaign_history::MissionAttemptSource::OriginalSaveImport
            );
            assert_eq!(attempt.completed_at_unix_seconds(), None);
            assert_eq!(attempt.duration_seconds(), None);
            assert_eq!(attempt.rules(), None);
            assert!(attempt.stats().is_empty());
            assert_eq!(attempt.achievements(), None);
            assert_eq!(attempt.achievement_attestation(), None);
        }
    }

    #[test]
    #[ignore = "requires Linux i386 v48 profile saves via ROBINHOOD_DATA_DIR; select fixtures-legacy-linux"]
    fn parses_current_linux_continue_campaign_boundaries() {
        let path = original_data::data_file("Data/Savegame/Profile_001/Continue");
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
        let profiles = read_fixture_profiles();
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
        assert_original_import_history(&bootstrap.campaign, &campaigns.live.campaign);
        assert!(campaigns.engine_offset < std::fs::metadata(path).unwrap().len());
    }

    #[test]
    #[ignore = "requires Linux i386 v48 profile saves via ROBINHOOD_DATA_DIR; select fixtures-legacy-linux"]
    fn golden_lincoln_restart_campaign_boundaries() {
        let path = original_data::data_file("Data/Savegame/Profile_000/Restart");
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
        let profiles = read_fixture_profiles();
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
        assert_original_import_history(&bootstrap.campaign, &campaigns.live.campaign);
        assert!(campaigns.engine_offset < std::fs::metadata(path).unwrap().len());
    }

    #[test]
    fn golden_retail_windows_campaign_boundaries() {
        let path =
            repository_fixture("reference-saves/Savegame_SuN1Sh1nE/Profile_004/Savegame_005");
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

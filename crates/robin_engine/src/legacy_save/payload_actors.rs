//! V48 leaf payloads for the current human actor classes.
//!
//! The PC wire grammar is unusual: its serializer writes a PC-specific
//! prefix, invokes `RHElementActorHuman::Serialize` in the middle, then writes
//! its portrait and remaining PC state. [`read_pc_payload`] mirrors that call
//! order by accepting callbacks for the shared Human payload and embedded
//! quick-action sequences.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::payload_base::{LegacyElementRef, LegacyPoint2, LegacyPoint3, read_element_ref};

const PC_ACTION_COUNT: usize = 3;
const QUICK_ACTION_MEMORY_COUNT: usize = 3;
const HUMAN_SKILL_COUNT: usize = 2;

const RH_ELEMENT_ACTOR_PC_FINGERPRINT: [u8; 16] = [
    0x88, 0x04, 0xb9, 0xc1, 0xf6, 0x61, 0xc4, 0xa2, 0x05, 0x88, 0x4e, 0xa0, 0x9f, 0x80, 0xdb, 0xfd,
];
const RH_ELEMENT_ACTOR_SOLDIER_FINGERPRINT: [u8; 16] = [
    0x63, 0x86, 0xf9, 0xe6, 0x6f, 0x1b, 0xd1, 0x8d, 0xf3, 0xe0, 0x62, 0xeb, 0x8d, 0xd4, 0x11, 0x58,
];
const RH_ELEMENT_ACTOR_CIVILIAN_FINGERPRINT: [u8; 16] = [
    0x97, 0x2f, 0x73, 0x8b, 0x96, 0xa9, 0xac, 0xbf, 0x9c, 0x1f, 0x89, 0x2e, 0x27, 0x1e, 0x61, 0xd3,
];
const RH_WIDGET_PORTRAIT_FINGERPRINT: [u8; 16] = [
    0x79, 0x23, 0xc0, 0xee, 0x5e, 0xed, 0x75, 0x84, 0x9f, 0x85, 0xf2, 0x9a, 0xc0, 0xde, 0x2d, 0x44,
];
const RH_HUMAN_STATUS_FINGERPRINT: [u8; 16] = [
    0x7d, 0xb9, 0x62, 0xa4, 0x53, 0x63, 0x7c, 0x9e, 0x9b, 0xd6, 0xe9, 0xf4, 0x18, 0x38, 0xf5, 0xc1,
];
const RH_PC_STATUS_FINGERPRINT: [u8; 16] = [
    0x79, 0x16, 0xc6, 0x08, 0xb9, 0xa8, 0xe3, 0x9c, 0x38, 0x18, 0xf2, 0xb4, 0x89, 0x2f, 0xee, 0xde,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyActorLeafLimits {
    pub campaign_characters: usize,
    pub pc_name_code_units: usize,
}

impl Default for LegacyActorLeafLimits {
    fn default() -> Self {
        Self {
            campaign_characters: 4096,
            pc_name_code_units: 4096,
        }
    }
}

/// Index into the already-decoded live campaign PC-description table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPcDescriptionRef(pub u32);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPcPayload<HumanPayload, SequencePayload> {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub pre_human: LegacyPcPreHuman<SequencePayload>,
    pub human: HumanPayload,
    pub portrait: LegacyPortraitState,
    pub post_human: LegacyPcPostHuman,
    pub end_offset: u64,
}

/// Read the complete `RHElementActorPC::Serialize` grammar.
///
/// `read_human` must consume exactly `RHElementActorHuman::Serialize` at the
/// callback point. `read_sequence` must consume one `RHSequence::Serialize`
/// with `bUsePreSerialization == false`; it is called only when the preceding
/// wire boolean says that the optional sequence exists.
pub fn read_pc_payload<HumanPayload, SequencePayload>(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    limits: &LegacyActorLeafLimits,
    read_human: impl FnOnce(&mut LegacyReader<'_>, LegacySaveAbiProfile) -> LegacyResult<HumanPayload>,
    mut read_sequence: impl FnMut(
        &mut LegacyReader<'_>,
        LegacySaveAbiProfile,
    ) -> LegacyResult<SequencePayload>,
) -> LegacyResult<LegacyPcPayload<HumanPayload, SequencePayload>> {
    reader.scope("actor_pc", |reader| {
        audit_abi(abi_profile);
        let start_offset = reader.offset();
        let pre_human = LegacyPcPreHuman::read(reader, abi_profile, limits, &mut read_sequence)?;
        let human = reader.scope("human", |reader| read_human(reader, abi_profile))?;
        let portrait = LegacyPortraitState::read(reader)?;
        let post_human = LegacyPcPostHuman::read(reader, limits)?;
        let end_offset = reader.offset();
        Ok(LegacyPcPayload {
            abi_profile,
            start_offset,
            pre_human,
            human,
            portrait,
            post_human,
            end_offset,
        })
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPcPreHuman<SequencePayload> {
    pub work_icon: u32,
    pub description: LegacyPcDescriptionRef,
    pub playable_member: bool,
    pub beam_me_index: u16,
    pub already_selected: bool,
    pub belt_seen: bool,
    pub feet_seen: bool,
    pub head_seen: bool,
    pub immortal: bool,
    pub fried_psykokwack: bool,
    pub list_index: u8,
    pub teleport_counter: u16,
    pub current_action: u32,
    pub saved_action: u32,
    pub disabled_actions: [bool; PC_ACTION_COUNT],
    pub disabled_actions_temp: [bool; PC_ACTION_COUNT],
    pub interface_displayed: bool,
    pub position_before_teleport: LegacyPoint2,
    pub quick_actions: [LegacyPcQuickAction<SequencePayload>; QUICK_ACTION_MEMORY_COUNT],
    /// A second, independently serialized playability value.
    pub playable_interface: bool,
}

impl<SequencePayload> LegacyPcPreHuman<SequencePayload> {
    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyActorLeafLimits,
        read_sequence: &mut impl FnMut(
            &mut LegacyReader<'_>,
            LegacySaveAbiProfile,
        ) -> LegacyResult<SequencePayload>,
    ) -> LegacyResult<Self> {
        reader.read_signature(
            "fingerprint",
            RH_ELEMENT_ACTOR_PC_FINGERPRINT,
            "MD5(\"RHElementActorPC\")",
        )?;
        let work_icon = reader.read_u32("work_icon")?;
        let description_offset = reader.offset();
        let description_index = reader.read_u32("description_index")?;
        if description_index as usize >= limits.campaign_characters {
            return Err(reader.invalid_value(
                description_offset,
                "description_index",
                description_index,
                "index into the decoded live campaign character table",
            ));
        }
        let description = LegacyPcDescriptionRef(description_index);
        let playable_member = reader.read_bool("playable_member")?;
        let beam_me_index = reader.read_u16("beam_me_index")?;
        let already_selected = reader.read_bool("already_selected")?;
        let belt_seen = reader.read_bool("belt_seen")?;
        let feet_seen = reader.read_bool("feet_seen")?;
        let head_seen = reader.read_bool("head_seen")?;
        let immortal = reader.read_bool("immortal")?;
        let fried_psykokwack = reader.read_bool("fried_psykokwack")?;
        let list_index = reader.read_u8("list_index")?;
        let teleport_counter = reader.read_u16("teleport_counter")?;
        let current_action = reader.read_u32("current_action")?;
        let saved_action = reader.read_u32("saved_action")?;
        let disabled_actions = read_bool3(reader, "disabled_actions")?;
        let disabled_actions_temp = read_bool3(reader, "disabled_actions_temp")?;
        let interface_displayed = reader.read_bool("interface_displayed")?;
        let position_before_teleport = read_point2(reader, "position_before_teleport")?;

        let metadata = [
            LegacyPcQuickActionMetadata::read(reader, 0)?,
            LegacyPcQuickActionMetadata::read(reader, 1)?,
            LegacyPcQuickActionMetadata::read(reader, 2)?,
        ];
        let sequences = [
            LegacyPcQuickActionSequences::read(reader, 0, abi_profile, read_sequence)?,
            LegacyPcQuickActionSequences::read(reader, 1, abi_profile, read_sequence)?,
            LegacyPcQuickActionSequences::read(reader, 2, abi_profile, read_sequence)?,
        ];
        let [metadata0, metadata1, metadata2] = metadata;
        let [sequences0, sequences1, sequences2] = sequences;
        let quick_actions = [
            LegacyPcQuickAction {
                metadata: metadata0,
                sequences: sequences0,
            },
            LegacyPcQuickAction {
                metadata: metadata1,
                sequences: sequences1,
            },
            LegacyPcQuickAction {
                metadata: metadata2,
                sequences: sequences2,
            },
        ];
        let playable_interface = reader.read_bool("playable_interface")?;

        Ok(Self {
            work_icon,
            description,
            playable_member,
            beam_me_index,
            already_selected,
            belt_seen,
            feet_seen,
            head_seen,
            immortal,
            fried_psykokwack,
            list_index,
            teleport_counter,
            current_action,
            saved_action,
            disabled_actions,
            disabled_actions_temp,
            interface_displayed,
            position_before_teleport,
            quick_actions,
            playable_interface,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPcQuickAction<SequencePayload> {
    pub metadata: LegacyPcQuickActionMetadata,
    pub sequences: LegacyPcQuickActionSequences<SequencePayload>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPcQuickActionMetadata {
    pub number_of_special_quick_actions: u16,
    pub quickito: u32,
    pub titbit: u32,
    pub button: u16,
    pub interactor: LegacyElementRef,
}

impl LegacyPcQuickActionMetadata {
    fn read(reader: &mut LegacyReader<'_>, index: usize) -> LegacyResult<Self> {
        reader.scope(format!("quick_actions[{index}].metadata"), |reader| {
            Ok(Self {
                number_of_special_quick_actions: reader
                    .read_u16("number_of_special_quick_actions")?,
                quickito: reader.read_u32("quickito")?,
                titbit: reader.read_u32("titbit")?,
                button: reader.read_u16("button")?,
                interactor: read_element_ref(reader, "interactor")?,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPcQuickActionSequences<SequencePayload> {
    pub action: Option<SequencePayload>,
    pub seek: Option<SequencePayload>,
}

impl<SequencePayload> LegacyPcQuickActionSequences<SequencePayload> {
    fn read(
        reader: &mut LegacyReader<'_>,
        index: usize,
        abi_profile: LegacySaveAbiProfile,
        read_sequence: &mut impl FnMut(
            &mut LegacyReader<'_>,
            LegacySaveAbiProfile,
        ) -> LegacyResult<SequencePayload>,
    ) -> LegacyResult<Self> {
        reader.scope(format!("quick_actions[{index}].sequences"), |reader| {
            let has_action = reader.read_bool("has_action")?;
            let action = if has_action {
                Some(reader.scope("action", |reader| read_sequence(reader, abi_profile))?)
            } else {
                None
            };
            let has_seek = reader.read_bool("has_seek")?;
            let seek = if has_seek {
                Some(reader.scope("seek", |reader| read_sequence(reader, abi_profile))?)
            } else {
                None
            };
            Ok(Self { action, seek })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPortraitState {
    pub quantities: [u16; 3],
    pub two_buttons_mode: bool,
    pub displayed: bool,
    pub burned: bool,
    pub open: bool,
    pub life_level: f32,
    pub trumpet_enabled: bool,
    pub quick_icons: [LegacyPortraitQuickIcon; QUICK_ACTION_MEMORY_COUNT],
}

impl LegacyPortraitState {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.scope("portrait", |reader| {
            reader.read_signature(
                "fingerprint",
                RH_WIDGET_PORTRAIT_FINGERPRINT,
                "MD5(\"RHWidgetPortrait\")",
            )?;
            Ok(Self {
                quantities: [
                    reader.read_u16("quantities[0]")?,
                    reader.read_u16("quantities[1]")?,
                    reader.read_u16("quantities[2]")?,
                ],
                two_buttons_mode: reader.read_bool("two_buttons_mode")?,
                displayed: reader.read_bool("displayed")?,
                burned: reader.read_bool("burned")?,
                open: reader.read_bool("open")?,
                life_level: reader.read_f32("life_level")?,
                trumpet_enabled: reader.read_bool("trumpet_enabled")?,
                quick_icons: [
                    LegacyPortraitQuickIcon::read(reader, 0)?,
                    LegacyPortraitQuickIcon::read(reader, 1)?,
                    LegacyPortraitQuickIcon::read(reader, 2)?,
                ],
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPortraitQuickIcon {
    pub titbit_id: u32,
    pub running: bool,
}

impl LegacyPortraitQuickIcon {
    fn read(reader: &mut LegacyReader<'_>, index: usize) -> LegacyResult<Self> {
        reader.scope(format!("quick_icons[{index}]"), |reader| {
            Ok(Self {
                titbit_id: reader.read_u32("titbit_id")?,
                running: reader.read_bool("running")?,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPcPostHuman {
    pub carried: LegacyElementRef,
    pub carried_posture: u32,
    pub shield_danger_point: LegacyPoint3,
    pub shield_protected: LegacyElementRef,
    pub shield_protector: LegacyElementRef,
    pub status: LegacyPcStatus,
    pub guard: LegacyElementRef,
    pub time_until_reinforcement: u32,
    pub last_ammo_dropping_position: LegacyPoint2,
    pub last_dropped_ammo: LegacyElementRef,
    pub update_last_dropped_ammo: bool,
    pub last_dropping_direction: u8,
}

impl LegacyPcPostHuman {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyActorLeafLimits) -> LegacyResult<Self> {
        reader.scope("post_human", |reader| {
            Ok(Self {
                carried: read_element_ref(reader, "carried")?,
                carried_posture: reader.read_u32("carried_posture")?,
                shield_danger_point: read_point3(reader, "shield_danger_point")?,
                shield_protected: read_element_ref(reader, "shield_protected")?,
                shield_protector: read_element_ref(reader, "shield_protector")?,
                status: LegacyPcStatus::read(reader, limits)?,
                guard: read_element_ref(reader, "guard")?,
                time_until_reinforcement: reader.read_u32("time_until_reinforcement")?,
                last_ammo_dropping_position: read_point2(reader, "last_ammo_dropping_position")?,
                last_dropped_ammo: read_element_ref(reader, "last_dropped_ammo")?,
                update_last_dropped_ammo: reader.read_bool("update_last_dropped_ammo")?,
                last_dropping_direction: reader.read_u8("last_dropping_direction")?,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyHumanStatus {
    pub skills: [LegacyHumanSkill; HUMAN_SKILL_COUNT],
}

impl LegacyHumanStatus {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.scope("human_status", |reader| {
            reader.read_signature(
                "fingerprint",
                RH_HUMAN_STATUS_FINGERPRINT,
                "MD5(\"RHHumanStatus\")",
            )?;
            Ok(Self {
                skills: [
                    LegacyHumanSkill::read(reader, 0)?,
                    LegacyHumanSkill::read(reader, 1)?,
                ],
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyHumanSkill {
    /// Serialized before experience despite the reverse declaration order.
    pub capacity: u32,
    pub experience: u32,
}

impl LegacyHumanSkill {
    fn read(reader: &mut LegacyReader<'_>, index: usize) -> LegacyResult<Self> {
        reader.scope(format!("skills[{index}]"), |reader| {
            Ok(Self {
                capacity: reader.read_u32("capacity")?,
                experience: reader.read_u32("experience")?,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPcStatus {
    pub human: LegacyHumanStatus,
    pub life_points: i16,
    pub in_coma: bool,
    pub number_of_ales: u16,
    pub number_of_apples: u16,
    pub number_of_arrows: u16,
    pub number_of_nets: u16,
    pub number_of_plants: u16,
    pub number_of_purses: u16,
    pub number_of_stoeckel_rations: u16,
    pub number_of_stones: u16,
    pub number_of_wasp_nests: u16,
    pub beam_me_index_in_sherwood: i16,
    pub name: String,
}

impl LegacyPcStatus {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyActorLeafLimits) -> LegacyResult<Self> {
        reader.scope("status", |reader| {
            let human = LegacyHumanStatus::read(reader)?;
            reader.read_signature(
                "fingerprint",
                RH_PC_STATUS_FINGERPRINT,
                "MD5(\"RHPCStatus\")",
            )?;
            Ok(Self {
                human,
                life_points: reader.read_i16("life_points")?,
                in_coma: reader.read_bool("in_coma")?,
                number_of_ales: reader.read_u16("number_of_ales")?,
                number_of_apples: reader.read_u16("number_of_apples")?,
                number_of_arrows: reader.read_u16("number_of_arrows")?,
                number_of_nets: reader.read_u16("number_of_nets")?,
                number_of_plants: reader.read_u16("number_of_plants")?,
                number_of_purses: reader.read_u16("number_of_purses")?,
                number_of_stoeckel_rations: reader.read_u16("number_of_stoeckel_rations")?,
                number_of_stones: reader.read_u16("number_of_stones")?,
                number_of_wasp_nests: reader.read_u16("number_of_wasp_nests")?,
                beam_me_index_in_sherwood: reader.read_i16("beam_me_index_in_sherwood")?,
                name: reader.read_wide_string("name", limits.pc_name_code_units)?,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySoldierPayload<NpcPayload> {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub npc: NpcPayload,
    pub leaf: LegacySoldierLeaf,
    pub end_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySoldierLeaf {
    pub start_offset: u64,
    pub apple_smell: u32,
    pub end_offset: u64,
}

/// Read only the bytes after `RHElementActorNPC::Serialize` in the soldier
/// serializer.
pub fn read_soldier_leaf(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
) -> LegacyResult<LegacySoldierLeaf> {
    audit_abi(abi_profile);
    let start_offset = reader.offset();
    reader.read_signature(
        "fingerprint",
        RH_ELEMENT_ACTOR_SOLDIER_FINGERPRINT,
        "MD5(\"RHElementActorSoldier\")",
    )?;
    let apple_smell = reader.read_u32("apple_smell")?;
    let end_offset = reader.offset();
    Ok(LegacySoldierLeaf {
        start_offset,
        apple_smell,
        end_offset,
    })
}

pub fn read_soldier_payload<NpcPayload>(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    read_npc: impl FnOnce(&mut LegacyReader<'_>, LegacySaveAbiProfile) -> LegacyResult<NpcPayload>,
) -> LegacyResult<LegacySoldierPayload<NpcPayload>> {
    reader.scope("actor_soldier", |reader| {
        audit_abi(abi_profile);
        let start_offset = reader.offset();
        let npc = reader.scope("npc", |reader| read_npc(reader, abi_profile))?;
        let leaf = reader.scope("leaf", |reader| read_soldier_leaf(reader, abi_profile))?;
        let end_offset = reader.offset();
        Ok(LegacySoldierPayload {
            abi_profile,
            start_offset,
            npc,
            leaf,
            end_offset,
        })
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyCivilianPayload<NpcPayload> {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub npc: NpcPayload,
    pub leaf: LegacyCivilianLeaf,
    pub end_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyCivilianLeaf {
    pub start_offset: u64,
    pub current_scroll_set: u32,
    pub end_offset: u64,
}

/// Read only the bytes after `RHElementActorNPC::Serialize` in Nottingham's
/// concrete civilian serializer.
pub fn read_civilian_leaf(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
) -> LegacyResult<LegacyCivilianLeaf> {
    audit_abi(abi_profile);
    let start_offset = reader.offset();
    reader.read_signature(
        "fingerprint",
        RH_ELEMENT_ACTOR_CIVILIAN_FINGERPRINT,
        "MD5(\"RHElementActorCivilian\")",
    )?;
    let current_scroll_set = reader.read_u32("current_scroll_set")?;
    let end_offset = reader.offset();
    Ok(LegacyCivilianLeaf {
        start_offset,
        current_scroll_set,
        end_offset,
    })
}

pub fn read_civilian_payload<NpcPayload>(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    read_npc: impl FnOnce(&mut LegacyReader<'_>, LegacySaveAbiProfile) -> LegacyResult<NpcPayload>,
) -> LegacyResult<LegacyCivilianPayload<NpcPayload>> {
    reader.scope("actor_civilian", |reader| {
        audit_abi(abi_profile);
        let start_offset = reader.offset();
        let npc = reader.scope("npc", |reader| read_npc(reader, abi_profile))?;
        let leaf = reader.scope("leaf", |reader| read_civilian_leaf(reader, abi_profile))?;
        let end_offset = reader.offset();
        Ok(LegacyCivilianPayload {
            abi_profile,
            start_offset,
            npc,
            leaf,
            end_offset,
        })
    })
}

fn read_point2(
    reader: &mut LegacyReader<'_>,
    field: impl Into<String>,
) -> LegacyResult<LegacyPoint2> {
    reader.scope(field, |reader| {
        Ok(LegacyPoint2 {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
        })
    })
}

fn read_point3(
    reader: &mut LegacyReader<'_>,
    field: impl Into<String>,
) -> LegacyResult<LegacyPoint3> {
    reader.scope(field, |reader| {
        Ok(LegacyPoint3 {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
            z: reader.read_f32("z")?,
        })
    })
}

fn read_bool3(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
) -> LegacyResult<[bool; PC_ACTION_COUNT]> {
    reader.scope(field, |reader| {
        Ok([
            reader.read_bool("[0]")?,
            reader.read_bool("[1]")?,
            reader.read_bool("[2]")?,
        ])
    })
}

fn audit_abi(abi_profile: LegacySaveAbiProfile) {
    match abi_profile {
        LegacySaveAbiProfile::RetailWindowsX86V48 | LegacySaveAbiProfile::PortLinuxI386V48 => {}
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_i16(bytes: &mut Vec<u8>, value: i16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut fixture = NamedTempFile::new().unwrap();
        fixture.write_all(bytes).unwrap();
        fixture.flush().unwrap();
        let path = fixture.path().to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    fn synthetic_pc_payload() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&RH_ELEMENT_ACTOR_PC_FINGERPRINT);
        push_u32(&mut bytes, 4); // work icon
        push_u32(&mut bytes, 2); // campaign description
        bytes.push(1); // playable member
        push_u16(&mut bytes, 7);
        bytes.extend_from_slice(&[1, 0, 1, 0, 1, 0]);
        bytes.push(5); // list index
        push_u16(&mut bytes, 9);
        push_u32(&mut bytes, 11);
        push_u32(&mut bytes, 12);
        bytes.extend_from_slice(&[1, 0, 1]);
        bytes.extend_from_slice(&[0, 1, 0]);
        bytes.push(1); // interface displayed
        push_f32(&mut bytes, 13.0);
        push_f32(&mut bytes, 14.0);

        for index in 0..QUICK_ACTION_MEMORY_COUNT as u32 {
            push_u16(&mut bytes, index as u16);
            push_u32(&mut bytes, index + 1);
            push_u32(&mut bytes, index + 20);
            push_u16(&mut bytes, index as u16 + 30);
            push_u32(&mut bytes, if index == 1 { u32::MAX } else { index + 40 });
        }
        // QA 0 has one embedded action sequence represented by a callback
        // marker. The other five option flags are false.
        bytes.push(1);
        push_u32(&mut bytes, 0x5151_0000);
        bytes.push(0);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.push(0); // independently serialized playable interface value

        push_u32(&mut bytes, 0x4848_0000); // Human callback marker

        bytes.extend_from_slice(&RH_WIDGET_PORTRAIT_FINGERPRINT);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 2);
        push_u16(&mut bytes, 3);
        bytes.extend_from_slice(&[1, 0, 1, 0]);
        push_f32(&mut bytes, 0.75);
        bytes.push(1);
        for index in 0..QUICK_ACTION_MEMORY_COUNT as u32 {
            push_u32(&mut bytes, index + 100);
            bytes.push((index == 2) as u8);
        }

        push_u32(&mut bytes, 50); // carried
        push_u32(&mut bytes, 3); // carried posture
        push_f32(&mut bytes, 1.0);
        push_f32(&mut bytes, 2.0);
        push_f32(&mut bytes, 3.0);
        push_u32(&mut bytes, u32::MAX);
        push_u32(&mut bytes, 51);

        bytes.extend_from_slice(&RH_HUMAN_STATUS_FINGERPRINT);
        for index in 0..HUMAN_SKILL_COUNT as u32 {
            push_u32(&mut bytes, 60 + index); // capacity
            push_u32(&mut bytes, 70 + index); // experience
        }
        bytes.extend_from_slice(&RH_PC_STATUS_FINGERPRINT);
        push_i16(&mut bytes, 80);
        bytes.push(0);
        for value in 81..=89 {
            push_u16(&mut bytes, value);
        }
        push_i16(&mut bytes, 90);
        push_u16(&mut bytes, 2);
        push_u16(&mut bytes, b'P' as u16);
        push_u16(&mut bytes, b'C' as u16);

        push_u32(&mut bytes, 52); // guard
        push_u32(&mut bytes, 91);
        push_f32(&mut bytes, 92.0);
        push_f32(&mut bytes, 93.0);
        push_u32(&mut bytes, 53);
        bytes.push(1);
        bytes.push(15);
        bytes
    }

    #[test]
    fn pc_parser_invokes_callbacks_at_the_original_midstream_positions() {
        let bytes = synthetic_pc_payload();
        with_reader(&bytes, |reader| {
            let payload = read_pc_payload(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacyActorLeafLimits {
                    campaign_characters: 3,
                    ..Default::default()
                },
                |reader, abi| {
                    assert_eq!(abi, LegacySaveAbiProfile::PortLinuxI386V48);
                    reader.read_u32("marker")
                },
                |reader, abi| {
                    assert_eq!(abi, LegacySaveAbiProfile::PortLinuxI386V48);
                    reader.read_u32("marker")
                },
            )
            .unwrap();

            assert_eq!(payload.start_offset, 0);
            assert_eq!(payload.end_offset, bytes.len() as u64);
            assert_eq!(payload.human, 0x4848_0000);
            assert_eq!(
                payload.pre_human.quick_actions[0].sequences.action,
                Some(0x5151_0000)
            );
            assert!(payload.pre_human.quick_actions[0].sequences.seek.is_none());
            assert_eq!(
                payload.pre_human.quick_actions[1].metadata.interactor,
                LegacyElementRef(None)
            );
            assert_eq!(payload.portrait.life_level, 0.75);
            assert_eq!(payload.post_human.status.name, "PC");
            assert_eq!(
                payload.post_human.last_dropped_ammo,
                LegacyElementRef(Some(53))
            );
        });
    }

    #[test]
    fn soldier_and_civilian_call_npc_before_their_leaf_signature() {
        for (signature, value, soldier) in [
            (RH_ELEMENT_ACTOR_SOLDIER_FINGERPRINT, 0x1234, true),
            (RH_ELEMENT_ACTOR_CIVILIAN_FINGERPRINT, 0x5678, false),
        ] {
            let mut bytes = Vec::new();
            push_u32(&mut bytes, 0x4e50_4300);
            bytes.extend_from_slice(&signature);
            push_u32(&mut bytes, value);
            with_reader(&bytes, |reader| {
                if soldier {
                    let payload = read_soldier_payload(
                        reader,
                        LegacySaveAbiProfile::RetailWindowsX86V48,
                        |reader, _| reader.read_u32("marker"),
                    )
                    .unwrap();
                    assert_eq!(payload.npc, 0x4e50_4300);
                    assert_eq!(payload.leaf.apple_smell, value);
                    assert_eq!(payload.leaf.start_offset, 4);
                    assert_eq!(payload.end_offset, bytes.len() as u64);
                } else {
                    let payload = read_civilian_payload(
                        reader,
                        LegacySaveAbiProfile::RetailWindowsX86V48,
                        |reader, _| reader.read_u32("marker"),
                    )
                    .unwrap();
                    assert_eq!(payload.npc, 0x4e50_4300);
                    assert_eq!(payload.leaf.current_scroll_set, value);
                    assert_eq!(payload.leaf.start_offset, 4);
                    assert_eq!(payload.end_offset, bytes.len() as u64);
                }
            });
        }
    }

    #[test]
    fn rejects_pc_description_outside_live_campaign() {
        let bytes = synthetic_pc_payload();
        with_reader(&bytes, |reader| {
            let error = read_pc_payload(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacyActorLeafLimits {
                    campaign_characters: 2,
                    ..Default::default()
                },
                |_, _| Ok(()),
                |_, _| Ok(()),
            )
            .unwrap_err();
            assert_eq!(error.field, "actor_pc.description_index");
            assert_eq!(error.offset, 20);
        });
    }

    #[test]
    fn reports_leaf_signature_failure_without_consuming_a_false_boundary() {
        let mut bytes = Vec::new();
        push_u32(&mut bytes, 0x4e50_4300);
        bytes.extend_from_slice(&[0; 16]);
        push_u32(&mut bytes, 1);
        with_reader(&bytes, |reader| {
            let error = read_soldier_payload(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                |reader, _| reader.read_u32("marker"),
            )
            .unwrap_err();
            assert_eq!(error.field, "actor_soldier.leaf.fingerprint");
            assert_eq!(error.offset, 4);
        });
    }
}

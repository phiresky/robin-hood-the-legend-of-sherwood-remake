//! Atomic adoption of Original v48 Human and PC leaf state.
//!
//! `LegacyStaticElementAdoption` owns the Element/Actor/NPC inheritance
//! prefixes. This sibling plan accounts for every field serialized by
//! `RHElementActorHuman::Serialize` and `RHElementActorPC::Serialize`,
//! including owner-local quick-action sequences and the PC status alias into
//! the live campaign character table.

use thiserror::Error;

use crate::{
    character_kind::CharacterKind,
    coordinates::{MapPoint, WorldPoint3D},
    element::{
        Entity, EntityId, EntityIdKind, HumanBoundingBox2State, HumanData, HumanPlaneState,
        HumanRepulsivePointState, HumanShieldPointState, HumanShieldState, HumanSwordSweepState,
        PcAmmoData, PcData, PcPortraitQuickIconState, PcPortraitState, Posture, QuickAction,
        SmalltalkHint, WorkIcon,
    },
    engine::{EngineInner, LevelAssets},
    pc_status::{HumanStatus, PcStatus, Skill},
    position_interface::SectorHandle,
    profiles::{Action, CharacterProfileIdx, ProfileManager},
    sequence::{Sequence, SequenceElementData, SequenceElementRef},
};

use super::{
    adopt::{
        LegacyEntityFixups, LegacyLineTopology, LegacyLineTopologyError, LegacyPositionTopology,
        LegacySaveAdoptError,
    },
    adopt_sequences::{
        LegacySequenceAdoptError, LegacySequenceAdoptionPlan, LegacySequenceTopology,
        convert_owner_local_sequence,
    },
    campaign::LegacyCampaign,
    payload_actors::{LegacyPcPayload, LegacyPcStatus},
    payload_base::{
        LegacyBoundingBox2, LegacyHumanPayload, LegacyPlane3, LegacyRepulsivePoint,
        LegacyShieldPayload,
    },
    payload_dispatch::{LegacyElementPayload, LegacyElementPayloadStream},
    payload_sequences::LegacyInlineSequence,
};

#[derive(Debug, Error)]
pub enum LegacyPcHumanAdoptError {
    #[error(transparent)]
    Identity(#[from] LegacySaveAdoptError),
    #[error(transparent)]
    Line(#[from] LegacyLineTopologyError),
    #[error(transparent)]
    Sequence(#[from] LegacySequenceAdoptError),
    #[error(
        "saved Human creation order {creation_order} resolved to non-Human runtime entity {entity_id}"
    )]
    ExpectedHuman {
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error(
        "saved PC creation order {creation_order} resolved to non-PC runtime entity {entity_id}"
    )]
    ExpectedPc {
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error(
        "saved Human creation order {creation_order} field {field} references {entity_id}, expected {expected}"
    )]
    ReferenceKind {
        creation_order: u32,
        field: &'static str,
        entity_id: EntityId,
        expected: &'static str,
    },
    #[error(
        "saved Human creation order {creation_order} field {field} has value {value}; expected {expected}"
    )]
    InvalidField {
        creation_order: u32,
        field: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error(
        "saved PC creation order {creation_order} references campaign character {character_index}, but the live campaign contains {character_count} characters"
    )]
    MissingCampaignCharacter {
        creation_order: u32,
        character_index: usize,
        character_count: usize,
    },
    #[error(
        "saved PC creation order {creation_order} campaign character {character_index} has no character profile"
    )]
    MissingCampaignProfile {
        creation_order: u32,
        character_index: usize,
    },
    #[error(
        "saved PC creation order {creation_order} campaign character {character_index} references missing character profile {profile_index}"
    )]
    UnknownCampaignProfile {
        creation_order: u32,
        character_index: usize,
        profile_index: u32,
    },
    #[error(
        "saved PC creation order {creation_order} contains two different playability values: member={member}, interface={interface}"
    )]
    PlayabilityMismatch {
        creation_order: u32,
        member: bool,
        interface: bool,
    },
    #[error(
        "saved Human creation order {creation_order} shoot-list entry {index} does not reference an Interaction element"
    )]
    ShootNotInteraction { creation_order: u32, index: usize },
}

#[derive(Debug)]
pub struct LegacyPcHumanAdoptionPlan {
    records: Vec<ConvertedRecord>,
}

#[derive(Debug)]
struct ConvertedRecord {
    entity_id: EntityId,
    human: ConvertedHuman,
    pc: Option<ConvertedPc>,
}

#[derive(Debug)]
struct ConvertedHuman {
    carrier: Option<EntityId>,
    concussion: u16,
    concussion_healing_timeout: u16,
    tiredness: u16,
    unconscious: bool,
    already_detectable_body: bool,
    detectable_list_index: u16,
    sword_strike_boredom: Vec<u16>,
    stuck_under_nets_counter: u16,
    hollow_man: bool,
    opponents: Vec<EntityId>,
    opponent_jump_lines: Vec<Option<crate::jump_line::JumpLineIndex>>,
    smalltalk_initiative: bool,
    received_smalltalk_initiative: bool,
    smalltalk_hint: SmalltalkHint,
    smalltalk_hint_opponent: Option<EntityId>,
    relative_fighting_ability: u16,
    small_repulsive_radius: bool,
    killed_by_accident: bool,
    parry_counter: u16,
    invulnerable: bool,
    last_motion_was_step_back: bool,
    running_hulk: u32,
    time_hulk: u32,
    hulk_level: u16,
    hulk_direction: bool,
    hulk_speed: f32,
    repulsive_point: HumanRepulsivePointState,
    building_sector: Option<SectorHandle>,
    produced_noise_first_word: f32,
    shield: HumanShieldState,
    sword_sweep: HumanSwordSweepState,
    pending_shoots: Vec<SequenceElementRef>,
}

#[derive(Debug)]
struct ConvertedPc {
    character_index: usize,
    profile_index: CharacterProfileIdx,
    kind: Option<CharacterKind>,
    has_lockpick: bool,
    has_climb: bool,
    has_jump: bool,
    status: PcStatus,
    work_icon: WorkIcon,
    playable: bool,
    beam_me_index: i16,
    already_selected: bool,
    belt_seen: bool,
    feet_seen: bool,
    head_seen: bool,
    immortal: bool,
    fried_psykokwack: bool,
    list_index: u8,
    teleport_counter: u16,
    current_action: Action,
    saved_action: Action,
    disabled_actions: Vec<bool>,
    disabled_actions_temp: Vec<bool>,
    interface_hidden: bool,
    position_before_teleport: MapPoint,
    quick_action_types: Vec<QuickAction>,
    quick_action_sequences: Vec<Option<Sequence>>,
    quick_seek_sequences: Vec<Option<Sequence>>,
    quick_action_special_counts: Vec<u16>,
    quick_action_buttons: Vec<u16>,
    quick_action_interactors: Vec<Option<EntityId>>,
    titbits: Vec<u32>,
    portrait: PcPortraitState,
    carried: Option<EntityId>,
    carried_posture: u32,
    shield_danger_point: WorldPoint3D,
    shield_protected: Option<EntityId>,
    shield_protector: Option<EntityId>,
    guard: Option<EntityId>,
    time_till_reinforcement: u32,
    last_ammo_dropping_position: MapPoint,
    last_dropped_ammo: Option<EntityId>,
    update_last_dropped_ammo: bool,
    last_dropping_direction: u8,
}

impl LegacyPcHumanAdoptionPlan {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preflight(
        engine: &EngineInner,
        payloads: &LegacyElementPayloadStream,
        entities: &LegacyEntityFixups,
        position_topology: &LegacyPositionTopology,
        sequence_topology: &LegacySequenceTopology,
        sequences: &LegacySequenceAdoptionPlan,
        live_campaign: &LegacyCampaign,
        assets: &LevelAssets,
    ) -> Result<Self, LegacyPcHumanAdoptError> {
        let line_topology = LegacyLineTopology::derive(engine)?;
        let mut records = Vec::new();
        for record in &payloads.records {
            let creation_order = record.header.creation_order;
            let (saved_human, saved_pc) = match &record.payload {
                LegacyElementPayload::ActorPc(pc) => (&pc.human, Some(pc)),
                LegacyElementPayload::ActorNpcSoldier(soldier) => (&soldier.npc.human, None),
                LegacyElementPayload::ActorNpcCivilian(civilian) => (&civilian.npc.human, None),
                _ => continue,
            };
            let entity_id = entities
                .by_creation_order
                .get(&creation_order)
                .copied()
                .ok_or(LegacySaveAdoptError::MissingCreationOrderReference { creation_order })?;
            let runtime = engine.world.entities.get(entity_id).ok_or(
                LegacyPcHumanAdoptError::ExpectedHuman {
                    creation_order,
                    entity_id,
                },
            )?;
            if runtime.human_data().is_none() {
                return Err(LegacyPcHumanAdoptError::ExpectedHuman {
                    creation_order,
                    entity_id,
                });
            }
            let human = convert_human(
                saved_human,
                creation_order,
                entities,
                position_topology,
                &line_topology,
                sequences,
            )?;
            let pc = saved_pc
                .map(|saved| {
                    let Entity::Pc(_) = runtime else {
                        return Err(LegacyPcHumanAdoptError::ExpectedPc {
                            creation_order,
                            entity_id,
                        });
                    };
                    convert_pc(
                        saved,
                        creation_order,
                        entities,
                        sequence_topology,
                        live_campaign,
                        &assets.profile_manager,
                    )
                })
                .transpose()?;
            records.push(ConvertedRecord {
                entity_id,
                human,
                pc,
            });
        }
        Ok(Self { records })
    }

    /// Apply only preflighted, owned values. No lookup or conversion occurs
    /// after mutation starts.
    pub(crate) fn apply(self, engine: &mut EngineInner) {
        for record in self.records {
            let entity = engine
                .world
                .entities
                .get_mut(record.entity_id)
                .expect("preflighted Human disappeared from adoption candidate");
            apply_human(
                entity
                    .human_data_mut()
                    .expect("preflighted Human changed concrete kind"),
                record.human,
            );
            if let Some(saved) = record.pc {
                let Entity::Pc(pc) = entity else {
                    unreachable!("preflighted PC changed concrete kind");
                };
                apply_pc(&mut pc.pc, &saved);
                let campaign_character = engine
                    .mission_domain
                    .campaign
                    .characters
                    .get_mut(saved.character_index)
                    .expect("preflighted campaign character disappeared");
                // RHElementActorPC::Serialize restores `mpStatus` as an alias
                // into this campaign description, then deserializes through
                // that pointer. The leaf copy is therefore authoritative over
                // the campaign stream read immediately beforehand.
                campaign_character.status = saved.status;
            }
        }
    }
}

fn convert_human(
    saved: &LegacyHumanPayload,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    position_topology: &LegacyPositionTopology,
    line_topology: &LegacyLineTopology,
    sequences: &LegacySequenceAdoptionPlan,
) -> Result<ConvertedHuman, LegacyPcHumanAdoptError> {
    let carrier = checked_ref(
        entities.resolve_element(saved.carrier)?,
        creation_order,
        "carrier",
        "PC",
        |kind| kind == EntityIdKind::Pc,
    )?;
    let mut opponents = Vec::with_capacity(saved.opponents.len());
    let mut opponent_jump_lines = Vec::with_capacity(saved.opponents.len());
    for opponent in &saved.opponents {
        opponents.push(
            checked_ref(
                entities.resolve_element(opponent.opponent)?,
                creation_order,
                "opponents.opponent",
                "Human",
                is_human_kind,
            )?
            .ok_or_else(|| {
                invalid(
                    creation_order,
                    "opponents.opponent",
                    "null",
                    "non-null Human",
                )
            })?,
        );
        opponent_jump_lines
            .push(line_topology.resolve("human.opponents.jump_line", opponent.jump_line)?);
    }
    let sword_victims = saved
        .sword_strike_victims
        .iter()
        .map(|reference| {
            checked_ref(
                entities.resolve_element(*reference)?,
                creation_order,
                "sword_strike_victims",
                "Human",
                is_human_kind,
            )?
            .ok_or_else(|| {
                invalid(
                    creation_order,
                    "sword_strike_victims",
                    "null",
                    "non-null Human",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut pending_shoots = Vec::with_capacity(saved.shoots.len());
    for (index, reference) in saved.shoots.iter().enumerate() {
        let (element_ref, element) = sequences
            .resolve_element("human.shoots", *reference)?
            .ok_or_else(|| invalid(creation_order, "shoots", "null", "non-null Interaction"))?;
        if !matches!(element.data, SequenceElementData::Interaction { .. }) {
            return Err(LegacyPcHumanAdoptError::ShootNotInteraction {
                creation_order,
                index,
            });
        }
        pending_shoots.push(element_ref);
    }
    Ok(ConvertedHuman {
        carrier,
        concussion: saved.concussion,
        concussion_healing_timeout: saved.concussion_healing_timeout,
        tiredness: saved.tiredness,
        unconscious: saved.unconscious,
        already_detectable_body: saved.already_detectable_body,
        detectable_list_index: saved.detectable_list_index,
        sword_strike_boredom: saved.sword_strike_boredom.to_vec(),
        stuck_under_nets_counter: saved.stuck_under_nets_counter,
        hollow_man: saved.hollow_man,
        opponents,
        opponent_jump_lines,
        smalltalk_initiative: saved.smalltalk_initiative,
        received_smalltalk_initiative: saved.received_smalltalk_initiative,
        smalltalk_hint: smalltalk_hint(saved.smalltalk_hint, creation_order)?,
        smalltalk_hint_opponent: checked_ref(
            entities.resolve_element(saved.hint_opponent)?,
            creation_order,
            "hint_opponent",
            "Human",
            is_human_kind,
        )?,
        relative_fighting_ability: saved.relative_fighting_ability,
        small_repulsive_radius: saved.small_repulsive_radius,
        killed_by_accident: saved.killed_by_accident,
        parry_counter: saved.parry_counter,
        invulnerable: saved.invulnerable,
        last_motion_was_step_back: saved.last_motion_was_step_back,
        running_hulk: saved.running_hulk,
        time_hulk: saved.time_hulk,
        hulk_level: saved.hulk_level,
        hulk_direction: saved.hulk_direction,
        hulk_speed: saved.hulk_speed,
        repulsive_point: repulsive_point(&saved.repulsive_point),
        building_sector: checked_sector(
            creation_order,
            "building",
            saved.building.0,
            &position_topology.sectors,
        )?,
        // Original's CHECKENUM bug writes exactly this word. Human noise
        // production refreshes every other member; retaining only the word is
        // the exact load semantic, not a fabricated partial Noise.
        produced_noise_first_word: saved.currently_produced_noise_first_word,
        shield: shield(&saved.shield),
        sword_sweep: HumanSwordSweepState {
            victims: sword_victims,
            initial_angle: saved.initial_strike_angle,
            current_angle: saved.current_strike_angle,
            final_angle: saved.final_strike_angle,
        },
        pending_shoots,
    })
}

#[allow(clippy::too_many_arguments)]
fn convert_pc(
    saved: &LegacyPcPayload<LegacyHumanPayload, LegacyInlineSequence>,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    sequence_topology: &LegacySequenceTopology,
    live_campaign: &LegacyCampaign,
    profiles: &ProfileManager,
) -> Result<ConvertedPc, LegacyPcHumanAdoptError> {
    let character_index = saved.pre_human.description.0 as usize;
    let description = live_campaign.characters.get(character_index).ok_or(
        LegacyPcHumanAdoptError::MissingCampaignCharacter {
            creation_order,
            character_index,
            character_count: live_campaign.characters.len(),
        },
    )?;
    let profile_index = description.character_profile_index.ok_or(
        LegacyPcHumanAdoptError::MissingCampaignProfile {
            creation_order,
            character_index,
        },
    )?;
    let profile = profiles.get_character(profile_index).ok_or(
        LegacyPcHumanAdoptError::UnknownCampaignProfile {
            creation_order,
            character_index,
            profile_index,
        },
    )?;
    let kind = CharacterKind::from_profile(&profile.filename, &profile.profile_name);
    let (has_lockpick, has_climb, has_jump) = PcData::movement_auth_from_profile(profile);
    if saved.pre_human.playable_member != saved.pre_human.playable_interface {
        return Err(LegacyPcHumanAdoptError::PlayabilityMismatch {
            creation_order,
            member: saved.pre_human.playable_member,
            interface: saved.pre_human.playable_interface,
        });
    }
    let mut quick_action_types = Vec::with_capacity(3);
    let mut quick_action_sequences = Vec::with_capacity(3);
    let mut quick_seek_sequences = Vec::with_capacity(3);
    let mut quick_action_special_counts = Vec::with_capacity(3);
    let mut quick_action_buttons = Vec::with_capacity(3);
    let mut quick_action_interactors = Vec::with_capacity(3);
    let mut titbits = Vec::with_capacity(3);
    for action in &saved.pre_human.quick_actions {
        quick_action_types.push(quick_action(action.metadata.quickito, creation_order)?);
        quick_action_sequences.push(
            action
                .sequences
                .action
                .as_ref()
                .map(|sequence| convert_owner_local_sequence(sequence, entities, sequence_topology))
                .transpose()?,
        );
        quick_seek_sequences.push(
            action
                .sequences
                .seek
                .as_ref()
                .map(|sequence| convert_owner_local_sequence(sequence, entities, sequence_topology))
                .transpose()?,
        );
        quick_action_special_counts.push(action.metadata.number_of_special_quick_actions);
        quick_action_buttons.push(action.metadata.button);
        quick_action_interactors.push(entities.resolve_element(action.metadata.interactor)?);
        titbits.push(action.metadata.titbit);
    }
    let status = pc_status(&saved.post_human.status);
    let carried = checked_ref(
        entities.resolve_element(saved.post_human.carried)?,
        creation_order,
        "carried",
        "Human",
        is_human_kind,
    )?;
    let carried_posture =
        preserve_dormant_carried_posture(saved.post_human.carried_posture, carried.is_some())
            .map_err(|value| {
                invalid(
                    creation_order,
                    "carried_posture",
                    value,
                    "RHposture 0..24 while a carried Human is present",
                )
            })?;
    Ok(ConvertedPc {
        character_index,
        profile_index: CharacterProfileIdx(profile_index),
        kind,
        has_lockpick,
        has_climb,
        has_jump,
        status,
        work_icon: work_icon(saved.pre_human.work_icon, creation_order)?,
        playable: saved.pre_human.playable_member,
        beam_me_index: if saved.pre_human.beam_me_index == u16::MAX {
            -1
        } else {
            i16::try_from(saved.pre_human.beam_me_index).map_err(|_| {
                invalid(
                    creation_order,
                    "beam_me_index",
                    saved.pre_human.beam_me_index,
                    "0..=32767 or 0xffff",
                )
            })?
        },
        already_selected: saved.pre_human.already_selected,
        belt_seen: saved.pre_human.belt_seen,
        feet_seen: saved.pre_human.feet_seen,
        head_seen: saved.pre_human.head_seen,
        immortal: saved.pre_human.immortal,
        fried_psykokwack: saved.pre_human.fried_psykokwack,
        list_index: saved.pre_human.list_index,
        teleport_counter: saved.pre_human.teleport_counter,
        current_action: action(
            saved.pre_human.current_action,
            creation_order,
            "current_action",
        )?,
        saved_action: action(saved.pre_human.saved_action, creation_order, "saved_action")?,
        disabled_actions: saved.pre_human.disabled_actions.to_vec(),
        disabled_actions_temp: saved.pre_human.disabled_actions_temp.to_vec(),
        interface_hidden: !saved.pre_human.interface_displayed,
        position_before_teleport: point2(saved.pre_human.position_before_teleport),
        quick_action_types,
        quick_action_sequences,
        quick_seek_sequences,
        quick_action_special_counts,
        quick_action_buttons,
        quick_action_interactors,
        titbits,
        portrait: PcPortraitState {
            quantities: saved.portrait.quantities,
            two_buttons_mode: saved.portrait.two_buttons_mode,
            displayed: saved.portrait.displayed,
            burned: saved.portrait.burned,
            open: saved.portrait.open,
            life_level: saved.portrait.life_level,
            trumpet_enabled: saved.portrait.trumpet_enabled,
            quick_icons: saved
                .portrait
                .quick_icons
                .map(|icon| PcPortraitQuickIconState {
                    titbit_id: icon.titbit_id,
                    running: icon.running,
                }),
        },
        carried,
        carried_posture,
        shield_danger_point: point3(saved.post_human.shield_danger_point),
        shield_protected: checked_ref(
            entities.resolve_element(saved.post_human.shield_protected)?,
            creation_order,
            "shield_protected",
            "PC",
            |kind| kind == EntityIdKind::Pc,
        )?,
        shield_protector: checked_ref(
            entities.resolve_element(saved.post_human.shield_protector)?,
            creation_order,
            "shield_protector",
            "PC",
            |kind| kind == EntityIdKind::Pc,
        )?,
        guard: checked_ref(
            entities.resolve_element(saved.post_human.guard)?,
            creation_order,
            "guard",
            "Soldier",
            |kind| kind == EntityIdKind::Soldier,
        )?,
        time_till_reinforcement: saved.post_human.time_until_reinforcement,
        last_ammo_dropping_position: point2(saved.post_human.last_ammo_dropping_position),
        last_dropped_ammo: checked_ref(
            entities.resolve_element(saved.post_human.last_dropped_ammo)?,
            creation_order,
            "last_dropped_ammo",
            "Bonus",
            |kind| kind == EntityIdKind::Bonus,
        )?,
        update_last_dropped_ammo: saved.post_human.update_last_dropped_ammo,
        last_dropping_direction: saved.post_human.last_dropping_direction,
    })
}

fn apply_human(human: &mut HumanData, saved: ConvertedHuman) {
    human.carrier = saved.carrier;
    human.concussion_of_the_brain = saved.concussion;
    human.concussion_healing_timeout = saved.concussion_healing_timeout;
    human.tiredness = saved.tiredness;
    human.unconscious = saved.unconscious;
    human.already_detectable_body = saved.already_detectable_body;
    human.detectable_list_index = saved.detectable_list_index;
    human.sword_strike_boredom = saved.sword_strike_boredom;
    human.stuck_under_nets_counter = saved.stuck_under_nets_counter;
    human.hollow_man = saved.hollow_man;
    human.opponents = saved.opponents;
    human.opponent_jump_lines = saved.opponent_jump_lines;
    human.smalltalk_initiative = saved.smalltalk_initiative;
    human.received_smalltalk_initiative = saved.received_smalltalk_initiative;
    human.smalltalk_hint = saved.smalltalk_hint;
    human.smalltalk_hint_opponent = saved.smalltalk_hint_opponent;
    human.relative_fighting_ability = saved.relative_fighting_ability;
    human.small_repulsive_radius = saved.small_repulsive_radius;
    // The corpse-intersection observer is a Rust-only derived cache. None
    // makes its first tick seed from the authoritative saved flag without
    // generating an update.
    human.last_is_lying_for_corpse_intersection = None;
    human.killed_by_accident = saved.killed_by_accident;
    human.parry_counter = saved.parry_counter;
    human.invulnerable = saved.invulnerable;
    human.last_motion_was_step_back_in_combat = saved.last_motion_was_step_back;
    human.running_hulk = saved.running_hulk;
    human.time_hulk = saved.time_hulk;
    human.hulk_level = saved.hulk_level;
    human.hulk_direction = saved.hulk_direction;
    human.hulk_speed = saved.hulk_speed;
    human.repulsive_point = saved.repulsive_point;
    human.building_sector = saved.building_sector;
    human.produced_noise_first_word = saved.produced_noise_first_word;
    human.shield = saved.shield;
    human.sword_sweep = saved.sword_sweep;
    human.pending_shoots = saved.pending_shoots;
}

fn apply_pc(pc: &mut PcData, saved: &ConvertedPc) {
    pc.life_points = saved.status.life_points;
    pc.campaign_description_index = Some(
        u32::try_from(saved.character_index)
            .expect("saved campaign description index originated as an Original u32"),
    );
    // RHElementActorPC::Serialize restores mpDescription and then replaces
    // mpProfile from that description. Profile-backed behavior follows the
    // serialized description even when the mission constructor used another
    // profile (notably PCs waiting to be rescued). Constructor-only `robin`
    // and sprite geometry are deliberately retained.
    pc.profile_index = saved.profile_index;
    pc.kind = saved.kind;
    pc.has_lockpick = saved.has_lockpick;
    pc.has_climb = saved.has_climb;
    pc.has_jump = saved.has_jump;
    pc.ammo = PcAmmoData {
        ales: saved.status.num_ales,
        arrows: saved.status.num_arrows,
        apples: saved.status.num_apples,
        rations: saved.status.num_rations,
        stones: saved.status.num_stones,
        wasp_nests: saved.status.num_wasp_nests,
        nets: saved.status.num_nets,
        plants: saved.status.num_plants,
        purses: saved.status.num_purses,
    };
    pc.work_icon = saved.work_icon;
    pc.playable = saved.playable;
    pc.beam_me_index = saved.beam_me_index;
    pc.already_selected = saved.already_selected;
    pc.belt_seen = saved.belt_seen;
    pc.feet_seen = saved.feet_seen;
    pc.head_seen = saved.head_seen;
    pc.immortal = saved.immortal;
    pc.fried_psykokwack = saved.fried_psykokwack;
    pc.list_index = saved.list_index;
    pc.teleport_counter = saved.teleport_counter;
    // Rust's max counter is a derived render denominator absent from the
    // Original save. The remaining count is the only authoritative bound at
    // load and avoids inventing an earlier teleport duration.
    pc.max_teleport_counter = saved.teleport_counter;
    pc.current_action = saved.current_action;
    pc.saved_action = saved.saved_action;
    pc.disabled_actions.clone_from(&saved.disabled_actions);
    pc.disabled_actions_temp
        .clone_from(&saved.disabled_actions_temp);
    pc.interface_hidden = saved.interface_hidden;
    pc.position_before_teleport = saved.position_before_teleport;
    pc.quick_action_types.clone_from(&saved.quick_action_types);
    pc.quick_action_sequences
        .clone_from(&saved.quick_action_sequences);
    pc.quick_seek_sequences
        .clone_from(&saved.quick_seek_sequences);
    pc.quick_action_special_counts
        .clone_from(&saved.quick_action_special_counts);
    pc.quick_action_buttons
        .clone_from(&saved.quick_action_buttons);
    pc.quick_action_interactors
        .clone_from(&saved.quick_action_interactors);
    pc.titbits.clone_from(&saved.titbits);
    pc.portrait = saved.portrait.clone();
    pc.trumpet_enabled = saved.portrait.trumpet_enabled;
    pc.carried = saved.carried;
    pc.carried_posture = saved.carried_posture;
    pc.shield_danger_point = saved.shield_danger_point;
    pc.shield_protected = saved.shield_protected;
    pc.shield_protector = saved.shield_protector;
    pc.guard = saved.guard;
    pc.time_till_reinforcement = saved.time_till_reinforcement;
    pc.last_ammo_dropping_position = saved.last_ammo_dropping_position;
    pc.last_dropped_ammo = saved.last_dropped_ammo;
    pc.update_last_dropped_ammo = saved.update_last_dropped_ammo;
    pc.last_dropping_direction = saved.last_dropping_direction;
}

fn pc_status(saved: &LegacyPcStatus) -> PcStatus {
    PcStatus {
        human_status: HumanStatus {
            hand_to_hand: Skill {
                capacity: saved.human.skills[0].capacity,
                experience: saved.human.skills[0].experience,
            },
            bow: Skill {
                capacity: saved.human.skills[1].capacity,
                experience: saved.human.skills[1].experience,
            },
        },
        life_points: saved.life_points,
        in_coma: saved.in_coma,
        num_ales: saved.number_of_ales,
        num_arrows: saved.number_of_arrows,
        num_apples: saved.number_of_apples,
        num_rations: saved.number_of_stoeckel_rations,
        num_stones: saved.number_of_stones,
        num_wasp_nests: saved.number_of_wasp_nests,
        num_nets: saved.number_of_nets,
        num_plants: saved.number_of_plants,
        num_purses: saved.number_of_purses,
        name: saved.name.clone(),
        name_override: None,
        beam_me_index_in_sherwood: saved.beam_me_index_in_sherwood,
    }
}

fn repulsive_point(saved: &LegacyRepulsivePoint) -> HumanRepulsivePointState {
    HumanRepulsivePointState {
        position: point2(saved.position),
        concave: saved.concave,
        limit_left: point2(saved.limit_left),
        limit_right: point2(saved.limit_right),
        action_radius: saved.action_radius,
        force_a: saved.force_a,
        force_b: saved.force_b,
        radius: saved.radius,
        id: saved.id,
        affects_pcs: saved.affects_pcs,
        affects_soldiers: saved.affects_soldiers,
        affects_civilians: saved.affects_civilians,
        affects_animals: saved.affects_animals,
    }
}

fn shield(saved: &LegacyShieldPayload) -> HumanShieldState {
    HumanShieldState {
        points: saved.points.map(|point| HumanShieldPointState {
            obstacle: point.obstacle,
            polygon: point2(point.polygon),
        }),
        top_plane: plane(&saved.top_plane),
        bottom_plane: plane(&saved.bottom_plane),
        box_3d: [
            saved.box_3d.x_min,
            saved.box_3d.x_max,
            saved.box_3d.y_min,
            saved.box_3d.y_max,
            saved.box_3d.z_min,
            saved.box_3d.z_max,
        ],
        ground_box: bbox2(saved.ground_box),
        screen_box: bbox2(saved.screen_box),
        on_ground: saved.on_ground,
    }
}

fn plane(saved: &LegacyPlane3) -> HumanPlaneState {
    HumanPlaneState {
        a: point3(saved.a),
        b: point3(saved.b),
        normal: point3(saved.normal),
        origin: point3(saved.origin),
        u: point3(saved.u),
        v: point3(saved.v),
        az: saved.az,
        bz: saved.bz,
        dz: saved.dz,
        d: saved.d,
    }
}

fn bbox2(saved: LegacyBoundingBox2) -> HumanBoundingBox2State {
    HumanBoundingBox2State {
        top_left: point2(saved.top_left),
        bottom_right: point2(saved.bottom_right),
        bounds_are_set: saved.bounds_are_set,
    }
}

fn point2(saved: super::payload_base::LegacyPoint2) -> MapPoint {
    MapPoint::new(saved.x, saved.y)
}

fn point3(saved: super::payload_base::LegacyPoint3) -> WorldPoint3D {
    WorldPoint3D::new(saved.x, saved.y, saved.z)
}

fn checked_sector(
    creation_order: u32,
    field: &'static str,
    raw: Option<u16>,
    sectors: &[Option<SectorHandle>],
) -> Result<Option<SectorHandle>, LegacyPcHumanAdoptError> {
    let Some(index) = raw else {
        return Ok(None);
    };
    let Some(sector) = sectors.get(usize::from(index)) else {
        return Err(invalid(
            creation_order,
            field,
            index,
            "an initialized Original sector index",
        ));
    };
    (*sector).map(Some).ok_or_else(|| {
        invalid(
            creation_order,
            field,
            index,
            "an Original sector slot with a Rust position-sector counterpart",
        )
    })
}

fn checked_ref(
    entity_id: Option<EntityId>,
    creation_order: u32,
    field: &'static str,
    expected: &'static str,
    accepts: impl FnOnce(EntityIdKind) -> bool,
) -> Result<Option<EntityId>, LegacyPcHumanAdoptError> {
    if let Some(entity_id) = entity_id
        && !accepts(entity_id.kind())
    {
        return Err(LegacyPcHumanAdoptError::ReferenceKind {
            creation_order,
            field,
            entity_id,
            expected,
        });
    }
    Ok(entity_id)
}

fn is_human_kind(kind: EntityIdKind) -> bool {
    matches!(
        kind,
        EntityIdKind::Pc | EntityIdKind::Soldier | EntityIdKind::Civilian
    )
}

fn preserve_dormant_carried_posture(raw: u32, carried_is_present: bool) -> Result<u32, u32> {
    if carried_is_present && Posture::try_from(raw).is_err() {
        return Err(raw);
    }
    Ok(raw)
}

fn action(
    raw: u32,
    creation_order: u32,
    field: &'static str,
) -> Result<Action, LegacyPcHumanAdoptError> {
    Action::try_from(raw)
        .map_err(|_| invalid(creation_order, field, raw, "a known RHaction discriminant"))
}

fn quick_action(raw: u32, creation_order: u32) -> Result<QuickAction, LegacyPcHumanAdoptError> {
    match raw {
        0 => Ok(QuickAction::None),
        1 => Ok(QuickAction::GoDown),
        2 => Ok(QuickAction::GoUp),
        3 => Ok(QuickAction::Interact),
        _ => Err(invalid(
            creation_order,
            "quick_actions.quickito",
            raw,
            "RHquickitos 0..3",
        )),
    }
}

fn work_icon(raw: u32, creation_order: u32) -> Result<WorkIcon, LegacyPcHumanAdoptError> {
    match raw {
        0 => Ok(WorkIcon::Arrows),
        1 => Ok(WorkIcon::Purses),
        2 => Ok(WorkIcon::Stones),
        3 => Ok(WorkIcon::Apples),
        4 => Ok(WorkIcon::Beer),
        5 => Ok(WorkIcon::Legs),
        6 => Ok(WorkIcon::Plants),
        7 => Ok(WorkIcon::Nets),
        8 => Ok(WorkIcon::Wasps),
        9 => Ok(WorkIcon::BowTraining),
        10 => Ok(WorkIcon::SwordTraining),
        11 => Ok(WorkIcon::Regeneration),
        12 => Ok(WorkIcon::None),
        _ => Err(invalid(
            creation_order,
            "work_icon",
            raw,
            "RHworkicon 0..12",
        )),
    }
}

fn smalltalk_hint(raw: u32, creation_order: u32) -> Result<SmalltalkHint, LegacyPcHumanAdoptError> {
    // RHSwordStrike: NONE=11, SMALLTALK_LEFT=12,
    // SMALLTALK_RIGHT=13, LEGS=14. No other strike is valid in this member.
    match raw {
        11 => Ok(SmalltalkHint::None),
        12 => Ok(SmalltalkHint::Left),
        13 => Ok(SmalltalkHint::Right),
        14 => Ok(SmalltalkHint::Legs),
        _ => Err(invalid(
            creation_order,
            "smalltalk_hint",
            raw,
            "SWORDSTRIKE_NONE/SMALLTALK_LEFT/SMALLTALK_RIGHT/LEGS (11..14)",
        )),
    }
}

fn invalid(
    creation_order: u32,
    field: &'static str,
    value: impl ToString,
    expected: &'static str,
) -> LegacyPcHumanAdoptError {
    LegacyPcHumanAdoptError::InvalidField {
        creation_order,
        field,
        value: value.to_string(),
        expected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_leaf_enums_are_mapped_strictly() {
        assert!(matches!(work_icon(12, 31), Ok(WorkIcon::None)));
        assert!(work_icon(13, 31).is_err());
        assert!(matches!(quick_action(3, 31), Ok(QuickAction::Interact)));
        assert!(quick_action(4, 31).is_err());
        assert!(matches!(smalltalk_hint(11, 31), Ok(SmalltalkHint::None)));
        assert!(matches!(smalltalk_hint(14, 31), Ok(SmalltalkHint::Legs)));
        assert!(smalltalk_hint(0, 31).is_err());
    }

    #[test]
    fn null_and_kind_checked_references_remain_distinct() {
        assert_eq!(
            checked_ref(None, 31, "carrier", "PC", |kind| kind == EntityIdKind::Pc).unwrap(),
            None
        );
        let soldier = EntityId::new(7, EntityIdKind::Soldier);
        assert!(
            checked_ref(Some(soldier), 31, "carrier", "PC", |kind| kind
                == EntityIdKind::Pc)
            .is_err()
        );
    }

    #[test]
    fn carried_posture_is_raw_only_while_no_body_is_carried() {
        let indeterminate = 161_437_968;
        assert_eq!(
            preserve_dormant_carried_posture(indeterminate, false),
            Ok(indeterminate)
        );
        assert_eq!(
            preserve_dormant_carried_posture(indeterminate, true),
            Err(indeterminate)
        );
        assert_eq!(
            preserve_dormant_carried_posture(Posture::Lying as u32, true),
            Ok(Posture::Lying as u32)
        );
    }
}

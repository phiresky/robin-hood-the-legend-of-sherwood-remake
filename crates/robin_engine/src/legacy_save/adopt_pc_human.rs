//! Atomic adoption of Original v48 Human and PC leaf state.
//!
//! `LegacyStaticElementAdoption` owns the Element/Actor/NPC inheritance
//! prefixes. This sibling plan accounts for every field serialized by
//! human-actor and player-character serialization,
//! including owner-local quick-action sequences and the PC status alias into
//! the live campaign character table.

use crate::{
    character_kind::CharacterKind,
    element::{
        Entity, EntityId, EntityIdKind, HumanBoundingBox2State, HumanData, HumanPlaneState,
        HumanRepulsivePointState, HumanShieldPointState, HumanShieldState, HumanSwordSweepState,
        PcAmmoData, PcData, PcPortraitQuickIconState, PcPortraitState, Posture, QuickAction,
        SmalltalkHint, WorkIcon,
    },
    engine::EngineInner,
    pc_status::{HumanStatus, PcStatus, Skill},
    position_interface::SectorHandle,
    profiles::{Action, CharacterProfileIdx},
    sequence::SequenceElementData,
};

use super::{
    LegacySaveAbiProfile,
    adopt::{LegacyLineTopology, LegacyPositionTopology, missing_creation_order},
    adopt_common::{AdoptCtx, AdoptErrorKind, AdoptSite, LegacyAdoptError, point2, point3},
    adopt_sequences::{LegacySequenceAdoptionPlan, convert_owner_local_sequence},
    campaign::LegacyCampaign,
    payload_actors::{LegacyPcPayload, LegacyPcStatus},
    payload_base::{
        LegacyBoundingBox2, LegacyHumanPayload, LegacyPlane3, LegacyRepulsivePoint,
        LegacyShieldPayload,
    },
    payload_dispatch::{LegacyElementPayload, LegacyElementPayloadStream},
    payload_sequences::LegacyInlineSequence,
};

fn human_site(creation_order: u32) -> AdoptSite {
    AdoptSite::element("saved Human", creation_order)
}

fn pc_site(creation_order: u32) -> AdoptSite {
    AdoptSite::element("saved PC", creation_order)
}

#[derive(Debug)]
pub struct LegacyPcHumanAdoptionPlan {
    records: Vec<ConvertedRecord>,
}

#[derive(Debug)]
struct ConvertedRecord {
    entity_id: EntityId,
    /// Finished Human component: the preflight-time runtime clone with every
    /// serialized Human member replaced. See the apply-order invariant in
    /// [`super::adopt_engine`].
    human: HumanData,
    pc: Option<ConvertedPc>,
}

/// Preflight authorities shared by every Human conversion in one plan.
#[derive(Clone, Copy)]
struct HumanSources<'a> {
    abi_profile: LegacySaveAbiProfile,
    line_topology: &'a LegacyLineTopology,
    sequences: &'a LegacySequenceAdoptionPlan,
}

#[derive(Debug)]
struct ConvertedPc {
    character_index: usize,
    status: PcStatus,
    pc: PcData,
    quick_slots: Vec<(crate::macro_store::QuickActionSlot, u16)>,
}

impl LegacyPcHumanAdoptionPlan {
    pub(crate) fn preflight(
        ctx: &AdoptCtx<'_>,
        payloads: &LegacyElementPayloadStream,
        abi_profile: LegacySaveAbiProfile,
        sequences: &LegacySequenceAdoptionPlan,
        live_campaign: &LegacyCampaign,
    ) -> Result<Self, LegacyAdoptError> {
        let AdoptCtx {
            engine,
            assets,
            entities,
            ..
        } = *ctx;
        let line_topology = LegacyLineTopology::derive(engine, assets)?;
        let sources = HumanSources {
            abi_profile,
            line_topology: &line_topology,
            sequences,
        };
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
                .ok_or_else(|| missing_creation_order(creation_order))?;
            let expected_human = || {
                human_site(creation_order).error(AdoptErrorKind::WrongEntityKind {
                    entity_id,
                    expected: "Human",
                })
            };
            let runtime = engine
                .world
                .entities
                .get(entity_id)
                .ok_or_else(expected_human)?;
            let Some(runtime_human) = runtime.human_data() else {
                return Err(expected_human());
            };
            let human = convert_human(ctx, saved_human, runtime_human, creation_order, sources)?;
            let pc = saved_pc
                .map(|saved| {
                    let Entity::Pc(runtime_pc) = runtime else {
                        return Err(pc_site(creation_order).error(
                            AdoptErrorKind::WrongEntityKind {
                                entity_id,
                                expected: "PC",
                            },
                        ));
                    };
                    convert_pc(ctx, saved, &runtime_pc.pc, creation_order, live_campaign)
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
            *entity
                .human_data_mut()
                .expect("preflighted Human changed concrete kind") = record.human;
            restore_saved_shield_obstacle(entity);
            if let Some(saved) = record.pc {
                let Entity::Pc(pc) = entity else {
                    unreachable!("preflighted PC changed concrete kind");
                };
                pc.pc = saved.pc;
                let campaign_character = engine
                    .mission_domain
                    .campaign
                    .characters
                    .get_mut(saved.character_index)
                    .expect("preflighted campaign character disappeared");
                // Player-character serialization restores status as an alias
                // into this campaign description, then deserializes through
                // that pointer. The leaf copy is therefore authoritative over
                // the campaign stream read immediately beforehand.
                campaign_character.status = saved.status;
                let macros = engine.players.macro_store.get_or_insert(record.entity_id);
                for (slot, (value, special_count)) in saved.quick_slots.into_iter().enumerate() {
                    macros.adopt_slot(slot, value, special_count);
                }
            }
        }
    }
}

fn convert_human(
    ctx: &AdoptCtx<'_>,
    saved: &LegacyHumanPayload,
    runtime: &HumanData,
    creation_order: u32,
    sources: HumanSources<'_>,
) -> Result<HumanData, LegacyAdoptError> {
    let AdoptCtx {
        engine,
        entities,
        position_topology,
        ..
    } = *ctx;
    let HumanSources {
        abi_profile,
        line_topology,
        sequences,
    } = sources;
    let carrier = checked_ref(
        entities.resolve_element(saved.carrier)?,
        creation_order,
        "carrier",
        "PC",
        |kind| kind == EntityIdKind::Pc,
    )?;
    let mut opponents = Vec::with_capacity(saved.opponents.len());
    for opponent in &saved.opponents {
        opponents.push(crate::element::SwordfightOpponent::new(
            checked_ref(
                entities.resolve_element(opponent.opponent)?,
                creation_order,
                "opponents.opponent",
                "Human",
                is_human_kind,
            )?
            .ok_or_else(|| {
                human_site(creation_order).invalid("opponents.opponent", "null", "non-null Human")
            })?,
            line_topology.resolve("human.opponents.jump_line", opponent.jump_line)?,
        ));
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
                human_site(creation_order).invalid("sword_strike_victims", "null", "non-null Human")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut pending_shoots = Vec::with_capacity(saved.shoots.len());
    for (index, reference) in saved.shoots.iter().enumerate() {
        let (element_ref, element) = sequences
            .resolve_element("human.shoots", *reference)?
            .ok_or_else(|| {
                human_site(creation_order).invalid("shoots", "null", "non-null Interaction")
            })?;
        if !matches!(element.data, SequenceElementData::Interaction { .. }) {
            return Err(human_site(creation_order).invalid(
                format!("shoots[{index}]"),
                format!("{:?} element", element.command),
                "an Interaction sequence element",
            ));
        }
        pending_shoots.push(element_ref);
    }
    Ok(HumanData {
        carrier,
        sorting_distance: 0.0,
        concussion_of_the_brain: saved.concussion,
        concussion_healing_timeout: saved.concussion_healing_timeout,
        tiredness: saved.tiredness,
        unconscious: saved.unconscious,
        already_detectable_body: saved.already_detectable_body,
        detectable_list_index: saved.detectable_list_index,
        sword_strike_boredom: saved.sword_strike_boredom.to_vec(),
        stuck_under_nets_counter: saved.stuck_under_nets_counter,
        hollow_man: saved.hollow_man,
        opponents: crate::element::SwordfightOpponents::from_entries(opponents),
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
        // The corpse-intersection observer is a Rust-only derived cache. None
        // makes its first tick seed from the authoritative saved flag without
        // generating an update.
        last_is_lying_for_corpse_intersection: None,
        killed_by_accident: saved.killed_by_accident,
        parry_counter: saved.parry_counter,
        invulnerable: saved.invulnerable,
        last_motion_was_step_back_in_combat: saved.last_motion_was_step_back,
        running_hulk: saved.running_hulk,
        time_hulk: saved.time_hulk,
        hulk_level: saved.hulk_level,
        hulk_direction: saved.hulk_direction,
        hulk_speed: saved.hulk_speed,
        repulsive_point: repulsive_point(&saved.repulsive_point),
        building_sector: convert_building_sector(
            engine,
            saved,
            abi_profile,
            creation_order,
            position_topology,
        )?,
        // The original game's enum-serialization bug writes exactly this word. Human noise
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
        // Preserve mission-initialized Human state outside this save
        // section's ownership.
        ..runtime.clone()
    })
}

fn convert_building_sector(
    engine: &EngineInner,
    saved: &LegacyHumanPayload,
    abi_profile: LegacySaveAbiProfile,
    creation_order: u32,
    position_topology: &LegacyPositionTopology,
) -> Result<Option<SectorHandle>, LegacyAdoptError> {
    if abi_profile == LegacySaveAbiProfile::PortLinuxI386V48 {
        return human_site(creation_order).checked_sector(
            "building",
            saved.building.0,
            &position_topology.sectors,
        );
    }

    // The retail stream uses both zero and 0xffff for "not in a building".
    // For any other stale/non-building index, the compatibility loader in
    // The original game resolves the authoritative
    // building from the actor's already-deserialized position sector.
    let Some(raw_building) = saved.building.0 else {
        return Ok(None);
    };
    if raw_building == 0 {
        return Ok(None);
    }
    let direct = position_topology
        .sectors
        .get(usize::from(raw_building))
        .copied()
        .flatten();
    if engine.entity_building_sector(direct).is_some() {
        return Ok(direct);
    }

    let position_sector = human_site(creation_order).checked_sector(
        "actor.position.sector",
        saved.actor.element.sprite.position.sector.0,
        &position_topology.sectors,
    )?;
    Ok(engine.entity_building_sector(position_sector))
}

fn convert_pc(
    ctx: &AdoptCtx<'_>,
    saved: &LegacyPcPayload<LegacyHumanPayload, LegacyInlineSequence>,
    runtime: &PcData,
    creation_order: u32,
    live_campaign: &LegacyCampaign,
) -> Result<ConvertedPc, LegacyAdoptError> {
    let AdoptCtx {
        assets,
        entities,
        sequence_topology,
        ..
    } = *ctx;
    let profiles = &assets.profile_manager;
    let character_index = saved.pre_human.description.0 as usize;
    let pc = pc_site(creation_order);
    let description = live_campaign
        .characters
        .get(character_index)
        .ok_or_else(|| {
            pc.out_of_range(
                "description",
                "campaign character",
                character_index,
                live_campaign.characters.len(),
            )
        })?;
    let campaign_character = || format!("campaign characters[{character_index}]");
    let profile_index = description.character_profile_index.ok_or_else(|| {
        pc.field_error(
            campaign_character(),
            AdoptErrorKind::Missing {
                what: "character profile",
            },
        )
    })?;
    let profile = profiles.get_character(profile_index).ok_or_else(|| {
        pc.field_error(
            campaign_character(),
            AdoptErrorKind::MissingCharacterProfile { profile_index },
        )
    })?;
    let kind = CharacterKind::from_profile(&profile.filename, &profile.profile_name);
    let (has_lockpick, has_climb, has_jump) = PcData::movement_auth_from_profile(profile);
    if saved.pre_human.playable_member != saved.pre_human.playable_interface {
        return Err(pc.error(AdoptErrorKind::PlayabilityMismatch {
            member: saved.pre_human.playable_member,
            interface: saved.pre_human.playable_interface,
        }));
    }
    if saved.pre_human.interface_displayed != saved.portrait.displayed {
        return Err(pc.error(AdoptErrorKind::PortraitDisplayMismatch {
            interface: saved.pre_human.interface_displayed,
            portrait: saved.portrait.displayed,
        }));
    }
    let status = pc_status(&saved.post_human.status);
    let expected_quantities = profile.actions.map(|action| status.get_ammo(action));
    if saved.portrait.quantities != expected_quantities {
        return Err(pc.error(AdoptErrorKind::PortraitQuantityMismatch {
            actual: saved.portrait.quantities,
            expected: expected_quantities,
        }));
    }
    let expected_two_buttons = profile.actions[2] == crate::profiles::Action::NoAction;
    if saved.portrait.two_buttons_mode != expected_two_buttons {
        return Err(pc.error(AdoptErrorKind::PortraitButtonModeMismatch {
            actual: saved.portrait.two_buttons_mode,
            expected: expected_two_buttons,
        }));
    }
    let expected_life = f32::from(status.life_points).to_bits();
    if saved.portrait.life_level.to_bits() != expected_life {
        return Err(pc.error(AdoptErrorKind::PortraitLifeMismatch {
            actual: saved.portrait.life_level.to_bits(),
            expected: expected_life,
        }));
    }
    let mut quick_slots = Vec::with_capacity(crate::macro_store::NUMBER_OF_QA_MEMORY);
    for (slot, action) in saved.pre_human.quick_actions.iter().enumerate() {
        let quickito = quick_action(action.metadata.quickito, creation_order)?;
        let interactor = entities.resolve_element(action.metadata.interactor)?;
        if quickito != QuickAction::None
            && (action.sequences.action.is_some() || action.sequences.seek.is_some())
        {
            return Err(pc.error(AdoptErrorKind::QuickitoSequenceConflict { slot, quickito }));
        }
        let valid_metadata = match quickito {
            QuickAction::None => true,
            QuickAction::GoDown | QuickAction::GoUp => {
                interactor.is_none() && action.metadata.button == 0
            }
            QuickAction::Interact => {
                interactor.is_some() && matches!(action.metadata.button, 0x0001 | 0x0008)
            }
        };
        if !valid_metadata {
            return Err(pc.error(AdoptErrorKind::InvalidQuickitoMetadata {
                slot,
                quickito,
                interactor,
                button: action.metadata.button,
            }));
        }
        let sequence = action
            .sequences
            .action
            .as_ref()
            .map(|sequence| convert_owner_local_sequence(sequence, entities, sequence_topology))
            .transpose()?;
        let seek = action
            .sequences
            .seek
            .as_ref()
            .map(|sequence| convert_owner_local_sequence(sequence, entities, sequence_topology))
            .transpose()?;
        quick_slots.push((
            crate::macro_store::QuickActionSlot::retained(
                sequence,
                seek,
                crate::macro_store::Quickito {
                    kind: quickito,
                    interactor,
                    button: action.metadata.button,
                },
                crate::titbit::TitbitId::new(action.metadata.titbit),
            ),
            action.metadata.number_of_special_quick_actions,
        ));
    }
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
                human_site(creation_order).invalid(
                    "carried_posture",
                    value,
                    "posture 0..24 while a carried human is present",
                )
            })?;
    let current_action = action(
        saved.pre_human.current_action,
        creation_order,
        "current_action",
    )?;
    let saved_action = action(saved.pre_human.saved_action, creation_order, "saved_action")?;
    let (current_action, saved_action) = refresh_actions_after_load(
        current_action,
        saved_action,
        &profile.actions,
        &saved.pre_human.disabled_actions,
        &saved.pre_human.disabled_actions_temp,
    );
    let mut pc = PcData {
        profile_index: CharacterProfileIdx(profile_index),
        kind,
        has_lockpick,
        has_climb,
        has_jump,
        work_icon: work_icon(saved.pre_human.work_icon, creation_order)?,
        beam_me_index: if saved.pre_human.beam_me_index == u16::MAX {
            -1
        } else {
            i16::try_from(saved.pre_human.beam_me_index).map_err(|_| {
                human_site(creation_order).invalid(
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
        max_teleport_counter: saved.pre_human.teleport_counter,
        current_action,
        saved_action,
        disabled_actions: saved.pre_human.disabled_actions.to_vec(),
        disabled_actions_temp: saved.pre_human.disabled_actions_temp.to_vec(),
        interface_hidden: !saved.pre_human.interface_displayed,
        position_before_teleport: point2(saved.pre_human.position_before_teleport),
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
                    titbit_id: crate::titbit::TitbitId::new(icon.titbit_id),
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
        life_points: status.life_points,
        campaign_description_index: Some(saved.pre_human.description.0),
        ammo: PcAmmoData {
            ales: status.num_ales,
            arrows: status.num_arrows,
            apples: status.num_apples,
            rations: status.num_rations,
            stones: status.num_stones,
            wasp_nests: status.num_wasp_nests,
            nets: status.num_nets,
            plants: status.num_plants,
            purses: status.num_purses,
        },
        trumpet_enabled: saved.portrait.trumpet_enabled,
        // Preserve constructor-only identity/geometry and fields owned by
        // other adoption sections, rather than fabricating their defaults.
        ..runtime.clone()
    };
    restore_loaded_playable(&mut pc, saved.pre_human.playable_member);
    Ok(ConvertedPc {
        character_index,
        status,
        pc,
        quick_slots,
    })
}

/// Apply player-character serialization's read-side action refresh.
///
/// After deserializing both action fields and the portrait, Original visits
/// every authored action slot. Disabled slots call `DisableAction`, temporary
/// slots call `DisableActionTemp`, and enabled slots call `EnableActionTemp`.
/// The last call restores `msavedAction` through a targeted
/// `MSG_SELECT_ACTION`, even while this PC is not selected.
fn refresh_actions_after_load(
    mut current: Action,
    mut saved: Action,
    actions: &[Action; crate::profiles::NUMBER_OF_PC_ACTIONS],
    disabled: &[bool; crate::profiles::NUMBER_OF_PC_ACTIONS],
    disabled_temp: &[bool; crate::profiles::NUMBER_OF_PC_ACTIONS],
) -> (Action, Action) {
    for ((&action, &disabled), &disabled_temp) in actions.iter().zip(disabled).zip(disabled_temp) {
        if disabled {
            if current == action {
                current = Action::NoAction;
            }
            if saved == action {
                saved = Action::NoAction;
            }
        } else if disabled_temp {
            if current == action {
                saved = current;
                current = Action::NoAction;
            }
        } else if saved == action {
            current = saved;
        }
    }
    (current, saved)
}

fn restore_saved_shield_obstacle(entity: &mut Entity) {
    let holding_shield = entity
        .actor_data()
        .expect("preflighted Human has no Actor data")
        .action_state
        .is_shield();
    if !holding_shield {
        return;
    }
    let serialized = entity
        .human_data()
        .expect("preflighted Human changed concrete kind")
        .shield
        .clone();
    entity
        .actor_data_mut()
        .expect("preflighted Human has no mutable Actor data")
        .shield_obstacle = Some(Box::new(
        crate::bow_shot::shield_obstacle_from_serialized_state(&serialized),
    ));
}

/// Restore the serialized playable flag through the same semantic
/// boundary as the original game's playable-state change.
///
/// `mission_role` is initialization state rather than a serialized original-game
/// field.  A save made after a PRIS/rescue PC joined the party therefore
/// retains a RescueTarget constructor role while serializing `playable=true`.
/// Direct assignment leaves that live party member outside the ordinary
/// party-defeat test and can terminate the replay before Original's final
/// simulation tick. `PcData::set_playable` performs the rescue
/// transition and leaves every other authored role unchanged.
fn restore_loaded_playable(pc: &mut PcData, playable: bool) {
    pc.set_playable(playable);
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

fn checked_ref(
    entity_id: Option<EntityId>,
    creation_order: u32,
    field: &'static str,
    expected: &'static str,
    accepts: impl FnOnce(EntityIdKind) -> bool,
) -> Result<Option<EntityId>, LegacyAdoptError> {
    if let Some(entity_id) = entity_id
        && !accepts(entity_id.kind())
    {
        return Err(human_site(creation_order).field_error(
            field,
            AdoptErrorKind::WrongEntityKind {
                entity_id,
                expected,
            },
        ));
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

fn action(raw: u32, creation_order: u32, field: &'static str) -> Result<Action, LegacyAdoptError> {
    Action::try_from(raw)
        .map_err(|_| human_site(creation_order).invalid(field, raw, "a known action discriminant"))
}

fn quick_action(raw: u32, creation_order: u32) -> Result<QuickAction, LegacyAdoptError> {
    match raw {
        0 => Ok(QuickAction::None),
        1 => Ok(QuickAction::GoDown),
        2 => Ok(QuickAction::GoUp),
        3 => Ok(QuickAction::Interact),
        _ => Err(human_site(creation_order).invalid(
            "quick_actions.quickito",
            raw,
            "quick-action slot 0..3",
        )),
    }
}

fn work_icon(raw: u32, creation_order: u32) -> Result<WorkIcon, LegacyAdoptError> {
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
        _ => Err(human_site(creation_order).invalid("work_icon", raw, "work icon 0..12")),
    }
}

fn smalltalk_hint(raw: u32, creation_order: u32) -> Result<SmalltalkHint, LegacyAdoptError> {
    // Sword-strike values: NONE=11, SMALLTALK_LEFT=12,
    // SMALLTALK_RIGHT=13, LEGS=14. No other strike is valid in this member.
    match raw {
        11 => Ok(SmalltalkHint::None),
        12 => Ok(SmalltalkHint::Left),
        13 => Ok(SmalltalkHint::Right),
        14 => Ok(SmalltalkHint::Legs),
        _ => Err(human_site(creation_order).invalid(
            "smalltalk_hint",
            raw,
            "SWORDSTRIKE_NONE/SMALLTALK_LEFT/SMALLTALK_RIGHT/LEGS (11..14)",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::human_control::{CombatStance, CommandInterface, MissionRole};

    #[test]
    fn load_refresh_restores_saved_action_from_enabled_slot() {
        let actions = [Action::Bow, Action::Hit, Action::Purse];
        assert_eq!(
            refresh_actions_after_load(
                Action::NoAction,
                Action::Bow,
                &actions,
                &[false; 3],
                &[false; 3],
            ),
            (Action::Bow, Action::Bow),
        );
    }

    #[test]
    fn load_refresh_preserves_disable_ordering() {
        let actions = [Action::Bow, Action::Hit, Action::Purse];
        assert_eq!(
            refresh_actions_after_load(
                Action::Hit,
                Action::Bow,
                &actions,
                &[true, false, false],
                &[false, true, false],
            ),
            (Action::NoAction, Action::Hit),
        );
    }

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

    #[test]
    fn loaded_playable_rescue_pc_rejoins_the_player_party() {
        let mut pc = PcData {
            playable: false,
            mission_role: MissionRole::RescueTarget,
            command_interface: CommandInterface::None,
            combat_stance: CombatStance::Hold,
            ..PcData::default()
        };

        restore_loaded_playable(&mut pc, true);

        assert!(pc.playable);
        assert_eq!(pc.mission_role, MissionRole::PlayerParty);
        assert_eq!(pc.command_interface, CommandInterface::HeroActions);
        assert_eq!(pc.combat_stance, CombatStance::Aggressive);
    }

    #[test]
    fn loaded_nonplayable_rescue_pc_retains_its_authored_role() {
        let mut pc = PcData {
            playable: true,
            mission_role: MissionRole::RescueTarget,
            ..PcData::default()
        };

        restore_loaded_playable(&mut pc, false);

        assert!(!pc.playable);
        assert_eq!(pc.mission_role, MissionRole::RescueTarget);
    }
}

//! Preflighted Original v48 sequence-manager adoption.
//!
//! The decoder has already checked ID uniqueness and pointer membership. This
//! module performs the second boundary: map Original identities into the
//! initialized Rust mission, convert every represented field, retain exact
//! Original-only values in compatibility sidecars, and only then replace the
//! live manager atomically.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    num::NonZeroU32,
};

use thiserror::Error;

use crate::{
    element::{ActionState, Command, EntityId, Posture},
    engine::{EngineInner, LegacyGridGateAsset, LevelAssets},
    gate::DoorIndex,
    jump_line::JumpLineIndex,
    order::{Order, OrderType},
    position_interface::SectorHandle,
    sequence::{
        Field, FieldValue, LegacyV48OrderState, LegacyV48SequenceElementState, MoveFlags, Sequence,
        SequenceElement, SequenceElementData, SequenceElementRef, SequenceId,
        SequenceManagerV48State, SequencePriority, SequenceState,
    },
};

use super::{
    adopt::LegacyEntityFixups,
    payload_base::{LegacyLineRef, LegacySectorRef},
    payload_sequences::{
        LegacyGateRef, LegacyGenericField, LegacyGenericFieldKind, LegacyGenericFieldValue,
        LegacyInlineOrder, LegacyInlineSequence, LegacyInlineSequenceElement,
    },
    post_sequence_manager::LegacySequenceManagerState,
};

/// Initialized mission pointer spaces used by sequence payloads.
#[derive(Clone, Debug, Default)]
pub struct LegacySequenceTopology {
    /// Complete Original `marrayGates` order mapped to Rust door/gate IDs.
    pub gates: Vec<Option<DoorIndex>>,
    /// Exact Original `(layer, index-in-layer)` line identity.
    pub lines: BTreeMap<(u16, i16), JumpLineIndex>,
    /// Sparse Original `marraySectors` bound.
    pub sector_count: usize,
}

impl LegacySequenceTopology {
    /// Reconstruct the mission-created pointer spaces used by sequences.
    ///
    /// Original gate order includes stateless jump gates. Rust's door table
    /// omits them, so the retained gate array must keep explicit empty slots
    /// or every later saved door pointer shifts identity.
    pub fn derive(
        engine: &EngineInner,
        assets: &LevelAssets,
    ) -> Result<Self, LegacySequenceAdoptError> {
        let retained = assets.legacy_grid_topology.as_ref().ok_or_else(|| {
            LegacySequenceAdoptError::MissingTopology {
                field: "sequence.topology",
                identity: "retained Original grid topology".to_owned(),
            }
        })?;
        let mut door_ordinal = 0u32;
        let gates = retained
            .gates
            .iter()
            .map(|gate| match gate {
                LegacyGridGateAsset::Door => {
                    let door = DoorIndex(door_ordinal);
                    door_ordinal += 1;
                    Some(door)
                }
                LegacyGridGateAsset::Stateless => None,
            })
            .collect::<Vec<_>>();
        if door_ordinal as usize != engine.script_domains.interactables.doors.len() {
            return Err(LegacySequenceAdoptError::MissingTopology {
                field: "sequence.topology.gates",
                identity: format!(
                    "{} retained doors versus {} initialized doors",
                    door_ordinal,
                    engine.script_domains.interactables.doors.len()
                ),
            });
        }

        let mut next_in_layer = BTreeMap::<u16, i16>::new();
        let mut lines = BTreeMap::new();
        for (runtime_index, line) in engine.world.fast_grid.level.jump_lines.iter().enumerate() {
            let index_in_layer = next_in_layer.entry(line.layer).or_default();
            let runtime_index = u32::try_from(runtime_index).map_err(|_| {
                LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: "runtime jump-line index exceeds u32".to_owned(),
                }
            })?;
            let handle = JumpLineIndex::new(runtime_index).ok_or_else(|| {
                LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: "runtime jump-line index equals null sentinel".to_owned(),
                }
            })?;
            lines.insert((line.layer, *index_in_layer), handle);
            *index_in_layer = index_in_layer.checked_add(1).ok_or_else(|| {
                LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: format!("layer {} contains more than i16::MAX lines", line.layer),
                }
            })?;
        }

        Ok(Self {
            gates,
            lines,
            sector_count: retained.sectors.len(),
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LegacySequenceAdoptError {
    #[error("saved sequence field {field} has value {value}; expected {expected}")]
    InvalidField {
        field: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error("saved sequence reference {field} names absent ID {id}")]
    MissingIdentity { field: &'static str, id: u32 },
    #[error(
        "saved sequence topology reference {field} names {identity}, absent from the initialized mission"
    )]
    MissingTopology {
        field: &'static str,
        identity: String,
    },
    #[error(
        "manager sequence list is not representable in Rust's identity-ordered manager: ID {current} follows {previous}"
    )]
    NonMonotonicManagerOrder { previous: u32, current: u32 },
    #[error("saved sequence {sequence_id} has duplicate generic field {field:?}")]
    DuplicateGenericField { sequence_id: u32, field: Field },
}

/// Fully converted manager state. Construction is read-only; applying it is
/// infallible and replaces the manager plus its order-ID counter together.
#[derive(Debug)]
pub(crate) struct LegacySequenceAdoptionPlan {
    manager: SequenceManagerV48State,
    next_order_id: u32,
}

impl LegacySequenceAdoptionPlan {
    pub(crate) fn apply(self, engine: &mut crate::engine::EngineInner) {
        engine
            .orders
            .sequence_manager
            .restore_v48_state(self.manager);
        engine.orders.next_order_id = self.next_order_id;
    }

    /// Resolve a tail-owned Original sequence-element pointer against the
    /// exact converted manager that will be installed. Tail preflight runs
    /// before this plan is consumed, so no live-manager fallback is needed.
    pub(crate) fn resolve_element(
        &self,
        field: &'static str,
        reference: super::payload_base::LegacySequenceElementRef,
    ) -> Result<Option<(SequenceElementRef, &SequenceElement)>, LegacySequenceAdoptError> {
        let Some(id) = reference.0 else {
            return Ok(None);
        };
        let (element_ref, element) = self
            .manager
            .sequences
            .iter()
            .find_map(|sequence| {
                sequence
                    .elements
                    .iter()
                    .enumerate()
                    .find(|(_, element)| element.id == id)
                    .map(|(element_index, element)| {
                        (SequenceElementRef::new(sequence.id, element_index), element)
                    })
            })
            .ok_or(LegacySequenceAdoptError::MissingIdentity { field, id })?;
        Ok(Some((element_ref, element)))
    }
}

/// Convert every manager-owned sequence and deferred element without mutating
/// the initialized engine.
pub(crate) fn preflight_v48_sequence_manager(
    saved: &LegacySequenceManagerState,
    entities: &LegacyEntityFixups,
    topology: &LegacySequenceTopology,
) -> Result<LegacySequenceAdoptionPlan, LegacySequenceAdoptError> {
    validate_manager_order(saved)?;

    let sequence_ids = saved
        .sequences
        .iter()
        .map(|saved| (saved.body.unique_id.0, SequenceId(saved.body.unique_id.0)))
        .collect::<BTreeMap<_, _>>();
    let mut element_refs = BTreeMap::new();
    for saved_sequence in &saved.sequences {
        let sequence_id = SequenceId(saved_sequence.body.unique_id.0);
        for (element_index, element) in saved_sequence.body.elements.iter().enumerate() {
            element_refs.insert(
                element.base().unique_id.0,
                SequenceElementRef::new(sequence_id, element_index),
            );
        }
    }

    let mut sequences = Vec::with_capacity(saved.sequences.len());
    for saved_sequence in &saved.sequences {
        sequences.push(convert_sequence(
            &saved_sequence.body,
            true,
            entities,
            topology,
            &sequence_ids,
            &element_refs,
        )?);
    }

    let mut elements_to_go = VecDeque::with_capacity(saved.deferred_elements.len());
    for deferred in &saved.deferred_elements {
        let id = deferred
            .element
            .0
            .expect("decoder rejects null manager deferred elements");
        let reference = resolve_element_ref("deferred_elements", Some(id), &element_refs)?
            .expect("non-null reference resolves to Some");
        elements_to_go.push_back((reference.sequence_id, reference.element_index));
    }

    Ok(LegacySequenceAdoptionPlan {
        manager: SequenceManagerV48State {
            sequences,
            elements_to_go,
            next_sequence_id: saved.static_ids.sequence_next_id,
            next_element_id: saved.static_ids.sequence_element_next_id,
        },
        // Original order ID zero is valid, while Rust uses NonZeroU32.
        // Shift the complete identity domain by one; this is bijective and
        // lets later pointer-adoption slices map `legacy_id -> legacy_id + 1`.
        next_order_id: saved
            .static_ids
            .order_next_id
            .checked_add(1)
            .ok_or_else(|| invalid("order_static.next_id", u32::MAX, "at most 0xfffffffe"))?,
    })
}

fn validate_manager_order(
    saved: &LegacySequenceManagerState,
) -> Result<(), LegacySequenceAdoptError> {
    for pair in saved.sequences.windows(2) {
        let previous = pair[0].body.unique_id.0;
        let current = pair[1].body.unique_id.0;
        if current <= previous {
            return Err(LegacySequenceAdoptError::NonMonotonicManagerOrder { previous, current });
        }
    }
    Ok(())
}

fn convert_sequence(
    saved: &LegacyInlineSequence,
    manager_owned: bool,
    entities: &LegacyEntityFixups,
    topology: &LegacySequenceTopology,
    sequence_ids: &BTreeMap<u32, SequenceId>,
    element_refs: &BTreeMap<u32, SequenceElementRef>,
) -> Result<Sequence, LegacySequenceAdoptError> {
    let cursor = usize::from(saved.sequence_element_cursor);
    if cursor > saved.elements.len() {
        return Err(invalid(
            "sequence_element_cursor",
            cursor,
            "an index at or before elements.len()",
        ));
    }

    let mut elements = Vec::with_capacity(saved.elements.len());
    for element in &saved.elements {
        elements.push(convert_element(
            saved.unique_id.0,
            element,
            manager_owned,
            entities,
            topology,
            sequence_ids,
            element_refs,
        )?);
    }
    for pair in elements.windows(2) {
        let previous = pair[0].command_level;
        let current = pair[1].command_level;
        if current != previous && previous.checked_add(1) != Some(current) {
            return Err(invalid(
                "command_level",
                current,
                "the previous command level or exactly its successor",
            ));
        }
    }

    Ok(Sequence::restore_v48_state(
        SequenceId(saved.unique_id.0),
        elements,
        cursor,
        saved.current_command_level,
        saved.running_elements,
        saved.elements_in_progress,
        saved.started,
    ))
}

#[allow(clippy::too_many_arguments)]
fn convert_element(
    sequence_id: u32,
    saved: &LegacyInlineSequenceElement,
    manager_owned: bool,
    entities: &LegacyEntityFixups,
    topology: &LegacySequenceTopology,
    sequence_ids: &BTreeMap<u32, SequenceId>,
    element_refs: &BTreeMap<u32, SequenceElementRef>,
) -> Result<SequenceElement, LegacySequenceAdoptError> {
    let base = saved.base();
    let command = Command::try_from(base.command)
        .map_err(|_| invalid("command", base.command, "a known RHcommand discriminant"))?;
    let owner = entities.resolve_element(base.owner).map_err(|error| {
        LegacySequenceAdoptError::MissingIdentity {
            field: "owner",
            id: match error {
                super::adopt::LegacySaveAdoptError::MissingCreationOrderReference {
                    creation_order,
                } => creation_order,
                _ => base.owner.0.unwrap_or(u32::MAX),
            },
        }
    })?;
    let state = sequence_state(base.state)?;
    let priority = sequence_priority(base.priority)?;
    let posture_after_transition =
        Posture::try_from(u32::try_from(base.posture_after_transition).map_err(|_| {
            invalid(
                "posture_after_transition",
                base.posture_after_transition,
                "RHposture 0..24",
            )
        })?)
        .map_err(|_| {
            invalid(
                "posture_after_transition",
                base.posture_after_transition,
                "RHposture 0..24",
            )
        })?;
    let action_state_after_transition = ActionState::try_from(
        u32::try_from(base.action_state_after_transition).map_err(|_| {
            invalid(
                "action_state_after_transition",
                base.action_state_after_transition,
                "RHactionState 0..17",
            )
        })?,
    )
    .map_err(|_| {
        invalid(
            "action_state_after_transition",
            base.action_state_after_transition,
            "RHactionState 0..17",
        )
    })?;

    let (orders, order_state) = convert_orders(&base.orders, entities)?;
    let mut generic_raw_unions = Vec::new();
    let (data, linked_seek, damage_arrow, raw_sword_strike) = match saved {
        LegacyInlineSequenceElement::Simple(_) => (SequenceElementData::Simple, None, None, None),
        LegacyInlineSequenceElement::Interaction(interaction) => (
            SequenceElementData::Interaction {
                antagonist: resolve_entity("interaction.element", interaction.element, entities)?,
            },
            None,
            None,
            None,
        ),
        LegacyInlineSequenceElement::Damage(damage) => {
            let raw_strike = damage.sword_strike;
            let sword_strike = match raw_strike {
                0 => Some(crate::weapons::SwordStrike::A),
                1 => Some(crate::weapons::SwordStrike::B),
                2 => Some(crate::weapons::SwordStrike::C),
                3 => Some(crate::weapons::SwordStrike::D),
                4 => Some(crate::weapons::SwordStrike::E),
                5 => Some(crate::weapons::SwordStrike::F),
                6 => Some(crate::weapons::SwordStrike::G),
                7 => Some(crate::weapons::SwordStrike::H),
                8 => Some(crate::weapons::SwordStrike::I),
                9 => Some(crate::weapons::SwordStrike::Charge),
                10..=14 => None,
                value => {
                    return Err(invalid("damage.sword_strike", value, "RHSwordStrike 0..14"));
                }
            };
            let sword_profile_idx = damage.sword.map(|profile| profile.0);
            (
                SequenceElementData::Damage {
                    origin: resolve_entity("damage.origin", damage.origin, entities)?,
                    damage: damage.damage,
                    concussion: damage.concussion,
                    sword_strike,
                    sword_profile_idx,
                    is_harder_hit: damage.harder_hit,
                },
                None,
                resolve_entity("damage.arrow", damage.arrow, entities)?,
                Some(raw_strike),
            )
        }
        LegacyInlineSequenceElement::Generic(generic) => {
            let mut properties = HashMap::with_capacity(generic.fields.len());
            for field in &generic.fields {
                let (kind, value, raw) =
                    convert_generic_field(field, entities, topology, sequence_id)?;
                if properties.insert(kind, value).is_some() {
                    return Err(LegacySequenceAdoptError::DuplicateGenericField {
                        sequence_id,
                        field: kind,
                    });
                }
                if let Some(raw) = raw {
                    generic_raw_unions.push((kind, raw));
                }
            }
            (
                SequenceElementData::Generic { properties },
                None,
                None,
                None,
            )
        }
        LegacyInlineSequenceElement::Movement(movement) => {
            let flags = MoveFlags::from_bits(movement.flags).ok_or_else(|| {
                invalid(
                    "movement.flags",
                    movement.flags,
                    "known RHmovementFlags bits",
                )
            })?;
            let action = OrderType::try_from(u32::try_from(movement.action).map_err(|_| {
                invalid(
                    "movement.action",
                    movement.action,
                    "a non-negative RHanimation",
                )
            })?)
            .map_err(|_| invalid("movement.action", movement.action, "a known RHanimation"))?;
            let post_seek_sequence = movement
                .post_seek_sequence
                .as_deref()
                .map(|sequence| {
                    convert_sequence(
                        sequence,
                        false,
                        entities,
                        topology,
                        sequence_ids,
                        element_refs,
                    )
                    .map(Box::new)
                })
                .transpose()?;
            let linked_seek = if manager_owned {
                Some(resolve_element_ref(
                    "movement.linked_seek",
                    movement
                        .manager_linked_seek_fixup
                        .as_ref()
                        .expect("decoder requires manager movement fixup")
                        .0,
                    element_refs,
                )?)
            } else {
                None
            };
            (
                SequenceElementData::Movement {
                    destination: crate::coordinates::MapPoint::new(
                        movement.destination.x,
                        movement.destination.y,
                    ),
                    layer: movement.layer,
                    sector: resolve_sector(movement.sector, topology)?,
                    gate_id: resolve_gate("movement.gate", movement.gate, topology)?,
                    line_id: resolve_line("movement.line", movement.line, topology)?,
                    element: resolve_entity("movement.element", movement.element, entities)?,
                    flags,
                    tolerance: movement.tolerance,
                    direction: movement.direction,
                    action,
                    speed_factor: movement.speed_factor,
                    post_seek_sequence,
                },
                linked_seek,
                None,
                None,
            )
        }
    };

    let (next, postponed, mummy) = if manager_owned {
        let fixups = base
            .manager_fixups
            .as_ref()
            .expect("decoder requires manager fixups");
        (
            resolve_element_ref("element.next", fixups.next.0, element_refs)?,
            resolve_element_ref("element.postponed", fixups.postponed.0, element_refs)?,
            resolve_sequence_ref("element.mummy", fixups.mummy.0, sequence_ids)?,
        )
    } else {
        (None, None, None)
    };

    let mut element = SequenceElement::new(base.command_level, command, owner);
    element.id = base.unique_id.0;
    element.state = state;
    element.priority = priority;
    element.posture_after_transition = posture_after_transition;
    element.action_state_after_transition = action_state_after_transition;
    element.orders = orders.into();
    element.data = data;
    if let Some(postponed) = postponed {
        if postponed.sequence_id == SequenceId(sequence_id) {
            element.postponed_element_index = Some(postponed.element_index);
        } else {
            element.cross_postponed = Some((postponed.sequence_id, postponed.element_index));
        }
    }
    element.legacy_v48 = Some(LegacyV48SequenceElementState {
        deleted: base.deleted,
        script_driven: base.script_driven,
        next,
        postponed,
        mummy,
        linked_seek,
        damage_arrow,
        raw_sword_strike,
        order_state,
        generic_raw_unions,
    });
    Ok(element)
}

fn convert_orders(
    saved: &[LegacyInlineOrder],
    entities: &LegacyEntityFixups,
) -> Result<(Vec<Order>, Vec<LegacyV48OrderState>), LegacySequenceAdoptError> {
    let mut orders = Vec::with_capacity(saved.len());
    let mut retained = Vec::with_capacity(saved.len());
    for order in saved {
        let action =
            OrderType::try_from(u32::try_from(order.action).map_err(|_| {
                invalid("order.action", order.action, "a non-negative RHanimation")
            })?)
            .map_err(|_| invalid("order.action", order.action, "a known RHanimation"))?;
        let mapped_id = order
            .unique_id
            .0
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .ok_or_else(|| invalid("order.unique_id", order.unique_id.0, "at most 0xfffffffe"))?;
        let antagonist = resolve_entity("order.antagonist", order.antagonist, entities)?;
        let mut converted = Order::new(
            action,
            order.destination_2d.x,
            order.destination_2d.y,
            mapped_id,
        );
        converted.compute_direction = order.compute_direction;
        converted.tolerance = order.tolerance;
        converted.lock_ai = order.lock_ai;
        converted.reverse = order.reverse;
        converted.target_actor = antagonist.map(EntityId::index);
        converted.antagonist = antagonist;
        orders.push(converted);
        retained.push(LegacyV48OrderState {
            legacy_id: order.unique_id.0,
            apply_transition_at_this_point: order.apply_transition_at_this_point,
            can_fly: order.can_fly,
            transition: order.transition,
            destination_3d: [
                order.destination_3d.x,
                order.destination_3d.y,
                order.destination_3d.z,
            ],
            flight_vector: [order.flight_vector.x, order.flight_vector.y],
        });
    }
    Ok((orders, retained))
}

fn convert_generic_field(
    saved: &LegacyGenericField,
    entities: &LegacyEntityFixups,
    topology: &LegacySequenceTopology,
    _sequence_id: u32,
) -> Result<(Field, FieldValue, Option<[u8; 12]>), LegacySequenceAdoptError> {
    use LegacyGenericFieldKind as K;
    let field = match saved.kind {
        K::Direction => Field::Direction,
        K::Event => Field::Event,
        K::Timer => Field::Timer,
        K::Message => Field::Message,
        K::MessageArgument => Field::MessageArgument,
        K::MessageExtendedArgument => Field::MessageExtendedArgument,
        K::BowTargetGuy => Field::BowTargetGuy,
        K::BowTargetPoint => Field::BowTargetPoint,
        K::CameraPoint => Field::CameraPoint,
        K::CameraZoomLevel => Field::CameraZoomLevel,
        K::CameraSpeed => Field::CameraSpeed,
        K::ActionId => Field::ActionId,
        K::ActionAvailable => Field::ActionAvailable,
        K::CharacterAvailable => Field::CharacterAvailable,
        K::ConcussionLevel => Field::ConcussionLevel,
        K::SpeakId => Field::SpeakId,
        K::SpeakFlags => Field::SpeakFlags,
        K::SpeakVariant => Field::SpeakVariant,
        K::DialogId => Field::DialogId,
        K::DialogSource => Field::DialogSource,
        K::PopupTextId => Field::PopupTextId,
        K::AnimationId => Field::AnimationId,
        K::MapDisplay => Field::MapDisplay,
        K::JumpLineSource => Field::JumplineSource,
        K::JumpLineDestination => Field::JumplineDestination,
        K::Amount => Field::Amount,
        K::ShieldDangerPoint => Field::ShieldDangerPoint,
        K::ShieldDangerPointLayer => Field::ShieldDangerPointLayer,
        K::ShieldProtected => Field::ShieldProtected,
        K::RollPoint => Field::RollPoint,
        K::PurseTarget => Field::PurseTarget,
        K::NetTarget => Field::NetTarget,
        K::WaspNestTarget => Field::WaspNestTarget,
        K::Opponent => Field::Opponent,
        K::SwordfightPrepared => Field::SwordfightPrepared,
        K::Gate => Field::Gate,
        K::Door => Field::Door,
        K::OldAnimation => Field::OldAnimation,
        K::NewAnimation => Field::NewAnimation,
        K::Freeze => Field::Freeze,
        K::Scroll => Field::Scroll,
        K::ScrollReader => Field::ScrollReader,
        K::ScrollOwner => Field::ScrollOwner,
    };
    let (value, raw) = match &saved.value {
        LegacyGenericFieldValue::Element(reference) => {
            let entity = resolve_entity("generic.element", *reference, entities)?;
            (
                entity
                    .map(FieldValue::Element)
                    .unwrap_or(FieldValue::OptionalElement(None)),
                None,
            )
        }
        LegacyGenericFieldValue::Line(reference) => {
            let line = resolve_line("generic.line", *reference, topology)?;
            (
                line.map(FieldValue::LineId)
                    .unwrap_or(FieldValue::OptionalLineId(None)),
                None,
            )
        }
        LegacyGenericFieldValue::Gate(reference) => {
            let gate = resolve_gate("generic.gate", *reference, topology)?;
            (
                gate.map(FieldValue::DoorId)
                    .unwrap_or(FieldValue::OptionalDoorId(None)),
                None,
            )
        }
        LegacyGenericFieldValue::RawUnion12(bytes) => {
            (FieldValue::Bool(bytes[0] != 0), Some(*bytes))
        }
        LegacyGenericFieldValue::Geo3(values) => {
            let raw = geo3_bytes(*values);
            let value = match saved.kind {
                K::CameraZoomLevel => FieldValue::Float(values[0]),
                K::CameraPoint => FieldValue::GeoPoint2D {
                    x: values[0],
                    y: values[1],
                },
                K::BowTargetPoint
                | K::ShieldDangerPoint
                | K::RollPoint
                | K::PurseTarget
                | K::NetTarget
                | K::WaspNestTarget => FieldValue::Point3D {
                    x: values[0],
                    y: values[1],
                    z: values[2],
                },
                K::AnimationId | K::OldAnimation | K::NewAnimation => {
                    let raw_animation = values[0].to_bits();
                    FieldValue::Animation(OrderType::try_from(raw_animation).map_err(|_| {
                        invalid(
                            "generic.animation",
                            raw_animation,
                            "a known RHanimation discriminant",
                        )
                    })?)
                }
                _ => FieldValue::Integer(values[0].to_bits()),
            };
            (value, Some(raw))
        }
    };
    Ok((field, value, raw))
}

fn geo3_bytes(values: [f32; 3]) -> [u8; 12] {
    let mut bytes = [0; 12];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_bits().to_le_bytes());
    }
    bytes
}

fn resolve_entity(
    field: &'static str,
    reference: super::payload_base::LegacyElementRef,
    entities: &LegacyEntityFixups,
) -> Result<Option<EntityId>, LegacySequenceAdoptError> {
    let Some(id) = reference.0 else {
        return Ok(None);
    };
    entities
        .by_creation_order
        .get(&id)
        .copied()
        .map(Some)
        .ok_or(LegacySequenceAdoptError::MissingIdentity { field, id })
}

fn resolve_element_ref(
    field: &'static str,
    id: Option<u32>,
    refs: &BTreeMap<u32, SequenceElementRef>,
) -> Result<Option<SequenceElementRef>, LegacySequenceAdoptError> {
    id.map(|id| {
        refs.get(&id)
            .copied()
            .ok_or(LegacySequenceAdoptError::MissingIdentity { field, id })
    })
    .transpose()
}

fn resolve_sequence_ref(
    field: &'static str,
    id: Option<u32>,
    refs: &BTreeMap<u32, SequenceId>,
) -> Result<Option<SequenceId>, LegacySequenceAdoptError> {
    id.map(|id| {
        refs.get(&id)
            .copied()
            .ok_or(LegacySequenceAdoptError::MissingIdentity { field, id })
    })
    .transpose()
}

fn resolve_gate(
    field: &'static str,
    reference: LegacyGateRef,
    topology: &LegacySequenceTopology,
) -> Result<Option<DoorIndex>, LegacySequenceAdoptError> {
    let Some(raw) = reference.0 else {
        return Ok(None);
    };
    let index =
        usize::try_from(raw).map_err(|_| invalid(field, raw, "a non-negative gate-array index"))?;
    topology
        .gates
        .get(index)
        .copied()
        .flatten()
        .map(Some)
        .ok_or_else(|| LegacySequenceAdoptError::MissingTopology {
            field,
            identity: format!("door gate at Original gate index {index}"),
        })
}

fn resolve_line(
    field: &'static str,
    reference: LegacyLineRef,
    topology: &LegacySequenceTopology,
) -> Result<Option<JumpLineIndex>, LegacySequenceAdoptError> {
    match (reference.layer, reference.index) {
        (None, None) => Ok(None),
        (Some(layer), Some(index)) if index >= 0 => topology
            .lines
            .get(&(layer, index))
            .copied()
            .map(Some)
            .ok_or_else(|| LegacySequenceAdoptError::MissingTopology {
                field,
                identity: format!("line layer {layer}, index {index}"),
            }),
        _ => Err(invalid(
            field,
            format!("{:?}", (reference.layer, reference.index)),
            "both null sentinels or a layer plus non-negative line index",
        )),
    }
}

fn resolve_sector(
    reference: LegacySectorRef,
    topology: &LegacySequenceTopology,
) -> Result<Option<SectorHandle>, LegacySequenceAdoptError> {
    let Some(index) = reference.0 else {
        return Ok(None);
    };
    if usize::from(index) >= topology.sector_count {
        return Err(LegacySequenceAdoptError::MissingTopology {
            field: "movement.sector",
            identity: format!("sector index {index}"),
        });
    }
    Ok(Some(
        SectorHandle::new(index).expect("legacy null sector sentinel is decoded as None"),
    ))
}

fn sequence_state(raw: i32) -> Result<SequenceState, LegacySequenceAdoptError> {
    Ok(match raw {
        0 => SequenceState::Terminated,
        1 => SequenceState::Done,
        2 => SequenceState::InProgress,
        3 => SequenceState::Todo,
        4 => SequenceState::Postponed,
        5 => SequenceState::Impossible,
        6 => SequenceState::Interrupted,
        _ => return Err(invalid("state", raw, "RHsequenceState 0..6")),
    })
}

fn sequence_priority(raw: i32) -> Result<SequencePriority, LegacySequenceAdoptError> {
    Ok(match raw {
        0 => SequencePriority::NonInterruptable,
        1 => SequencePriority::PostponeEverythingButInjuries,
        2 => SequencePriority::Lethal,
        3 => SequencePriority::Ko,
        4 => SequencePriority::Ko2,
        5 => SequencePriority::Injury,
        6 => SequencePriority::Script,
        7 => SequencePriority::Preference,
        8 => SequencePriority::Normal,
        9 => SequencePriority::Wait,
        10 => SequencePriority::None,
        11 => SequencePriority::NotYetSet,
        _ => return Err(invalid("priority", raw, "RHpriority 0..11")),
    })
}

fn invalid(
    field: &'static str,
    value: impl std::fmt::Display,
    expected: &'static str,
) -> LegacySequenceAdoptError {
    LegacySequenceAdoptError::InvalidField {
        field,
        value: value.to_string(),
        expected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        element::EntityIdKind,
        legacy_save::{
            payload_base::{
                LegacyElementRef, LegacyPoint2, LegacyPoint3, LegacySequenceElementRef,
                LegacySequenceRef,
            },
            payload_sequences::{
                LegacyInlineOrderId, LegacyInlineSequenceElementId, LegacyInlineSequenceId,
                LegacySequenceElementBase, LegacySequenceElementFixups,
                LegacySequenceElementGeneric, LegacySequenceElementMovement,
            },
            post_sequence_manager::{
                LegacyDeferredSequenceElement, LegacyManagedSequence, LegacySequenceStaticIds,
            },
        },
    };

    fn entities() -> (LegacyEntityFixups, EntityId, EntityId) {
        let owner = EntityId::new(4, EntityIdKind::Pc);
        let target = EntityId::new(9, EntityIdKind::Soldier);
        let fixups = LegacyEntityFixups {
            by_creation_order: [(40, owner), (90, target)].into(),
            by_saved_slot: vec![owner, target],
            creation_order_by_entity: [(owner, 40), (target, 90)].into(),
        };
        (fixups, owner, target)
    }

    fn fixups(
        next: Option<u32>,
        postponed: Option<u32>,
        mummy: u32,
    ) -> LegacySequenceElementFixups {
        LegacySequenceElementFixups {
            next_offset: 0,
            next: LegacySequenceElementRef(next),
            postponed_offset: 0,
            postponed: LegacySequenceElementRef(postponed),
            mummy_offset: 0,
            mummy: LegacySequenceRef(Some(mummy)),
        }
    }

    fn order(id: u32) -> LegacyInlineOrder {
        LegacyInlineOrder {
            action: OrderType::WaitingUpright as i32,
            apply_transition_at_this_point: true,
            compute_direction: false,
            can_fly: true,
            lock_ai: true,
            reverse: true,
            transition: true,
            tolerance: 3.5,
            unique_id: LegacyInlineOrderId(id),
            destination_2d: LegacyPoint2 { x: 5.0, y: 6.0 },
            destination_3d: LegacyPoint3 {
                x: 7.0,
                y: 8.0,
                z: 9.0,
            },
            flight_vector: LegacyPoint2 { x: 1.5, y: 2.5 },
            antagonist: LegacyElementRef(Some(90)),
        }
    }

    fn base(
        id: u32,
        level: u16,
        state: i32,
        fixups: LegacySequenceElementFixups,
    ) -> LegacySequenceElementBase {
        LegacySequenceElementBase {
            command: Command::Move as i32,
            state,
            command_level: level,
            priority: SequencePriority::Normal as i32,
            unique_id: LegacyInlineSequenceElementId(id),
            posture_after_transition: Posture::Upright as i32,
            action_state_after_transition: ActionState::Waiting as i32,
            deleted: false,
            script_driven: true,
            owner: LegacyElementRef(Some(40)),
            orders: vec![order(0)],
            manager_fixups: Some(fixups),
        }
    }

    fn sequence(
        id: u32,
        elements: Vec<LegacyInlineSequenceElement>,
        cursor: u16,
        in_progress: u16,
    ) -> LegacyManagedSequence {
        LegacyManagedSequence {
            start_offset: 0,
            body: LegacyInlineSequence {
                started: true,
                current_command_level: 1,
                running_elements: 1,
                sequence_element_cursor: cursor,
                unique_id: LegacyInlineSequenceId(id),
                elements,
                elements_in_progress: in_progress,
            },
            end_offset: 0,
        }
    }

    fn fixture() -> LegacySequenceManagerState {
        let movement = LegacyInlineSequenceElement::Movement(LegacySequenceElementMovement {
            base: base(100, 1, 3, fixups(Some(101), Some(200), 10)),
            action: OrderType::WalkingUpright as i32,
            tolerance: 12.0,
            direction: 7,
            flags: (MoveFlags::SEEK | MoveFlags::USE_POINT).bits(),
            layer: 2,
            speed_factor: 1.25,
            destination: LegacyPoint2 { x: 30.0, y: 40.0 },
            element: LegacyElementRef(Some(90)),
            gate: LegacyGateRef(Some(0)),
            line: LegacyLineRef {
                layer: Some(2),
                index: Some(3),
            },
            sector: LegacySectorRef(Some(4)),
            post_seek_sequence: None,
            manager_linked_seek_fixup_offset: Some(0),
            manager_linked_seek_fixup: Some(LegacySequenceElementRef(Some(101))),
        });
        let generic = LegacyInlineSequenceElement::Generic(LegacySequenceElementGeneric {
            base: base(101, 2, 2, fixups(None, None, 10)),
            fields: vec![
                LegacyGenericField {
                    kind: LegacyGenericFieldKind::Opponent,
                    value: LegacyGenericFieldValue::Element(LegacyElementRef(Some(90))),
                },
                LegacyGenericField {
                    kind: LegacyGenericFieldKind::Freeze,
                    value: LegacyGenericFieldValue::RawUnion12([
                        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
                    ]),
                },
            ],
        });
        let other = LegacyInlineSequenceElement::Simple(base(200, 1, 3, fixups(None, None, 20)));
        LegacySequenceManagerState {
            start_offset: 0,
            static_ids: LegacySequenceStaticIds {
                order_next_id_offset: 0,
                order_next_id: 1,
                sequence_next_id_offset: 0,
                sequence_next_id: 21,
                sequence_element_next_id_offset: 0,
                sequence_element_next_id: 201,
            },
            sequences: vec![
                sequence(10, vec![movement, generic], 1, 1),
                sequence(20, vec![other], 1, 0),
            ],
            deferred_elements: vec![
                LegacyDeferredSequenceElement {
                    offset: 0,
                    element: LegacySequenceElementRef(Some(100)),
                },
                LegacyDeferredSequenceElement {
                    offset: 0,
                    element: LegacySequenceElementRef(Some(200)),
                },
            ],
            end_offset: 0,
        }
    }

    fn topology() -> LegacySequenceTopology {
        LegacySequenceTopology {
            gates: vec![Some(DoorIndex(7))],
            lines: [((2, 3), JumpLineIndex::new(5).unwrap())].into(),
            sector_count: 6,
        }
    }

    #[test]
    fn stateless_gate_slot_does_not_shift_following_door_identity() {
        let topology = LegacySequenceTopology {
            gates: vec![None, Some(DoorIndex(0)), Some(DoorIndex(1))],
            ..Default::default()
        };
        assert!(resolve_gate("movement.gate", LegacyGateRef(Some(0)), &topology).is_err());
        assert_eq!(
            resolve_gate("movement.gate", LegacyGateRef(Some(2)), &topology).unwrap(),
            Some(DoorIndex(1))
        );
    }

    #[test]
    fn preflights_and_atomically_restores_manager_identity_and_fifo_order() {
        let (entities, owner, target) = entities();
        let plan = preflight_v48_sequence_manager(&fixture(), &entities, &topology()).unwrap();
        let (resolved_ref, resolved) = plan
            .resolve_element("tail.timer", LegacySequenceElementRef(Some(101)))
            .unwrap()
            .unwrap();
        assert_eq!(resolved_ref, SequenceElementRef::new(SequenceId(10), 1));
        assert_eq!(resolved.id, 101);
        let mut engine = crate::engine::EngineInner::new();
        plan.apply(&mut engine);

        let manager = &engine.orders.sequence_manager;
        assert_eq!(manager.sequence_count(), 2);
        assert_eq!(
            manager.v48_elements_to_go(),
            vec![(SequenceId(10), 0), (SequenceId(20), 0)]
        );
        let movement = manager.get_element(SequenceId(10), 0).unwrap();
        assert_eq!(movement.owner, Some(owner));
        assert_eq!(movement.orders[0].order_id.get(), 1);
        assert_eq!(movement.orders[0].antagonist, Some(target));
        let retained = movement.legacy_v48.as_ref().unwrap();
        assert_eq!(
            retained.next,
            Some(SequenceElementRef::new(SequenceId(10), 1))
        );
        assert_eq!(
            retained.postponed,
            Some(SequenceElementRef::new(SequenceId(20), 0))
        );
        assert_eq!(
            retained.linked_seek,
            Some(Some(SequenceElementRef::new(SequenceId(10), 1)))
        );
        assert!(retained.script_driven);
        assert_eq!(retained.order_state[0].destination_3d, [7.0, 8.0, 9.0]);
        assert_eq!(
            manager
                .get_element(SequenceId(10), 1)
                .unwrap()
                .legacy_v48
                .as_ref()
                .unwrap()
                .generic_raw_unions,
            vec![(Field::Freeze, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])]
        );

        assert_eq!(engine.orders.allocate_order_id().get(), 2);
        let mut fresh = Sequence::new();
        fresh.append_element(SequenceElement::new(1, Command::Wait, Some(owner)));
        let fresh_id = engine.orders.sequence_manager.launch_sequence(fresh);
        assert_eq!(fresh_id, SequenceId(21));
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(fresh_id, 0)
                .unwrap()
                .id,
            201
        );
    }

    #[test]
    fn rejects_unrepresentable_manager_order_before_mutating_engine() {
        let (entities, owner, _) = entities();
        let mut saved = fixture();
        saved.sequences.swap(0, 1);
        let mut engine = crate::engine::EngineInner::new();
        let mut existing = Sequence::new();
        existing.append_element(SequenceElement::new(1, Command::Wait, Some(owner)));
        let existing_id = engine.orders.sequence_manager.launch_sequence(existing);

        let error = preflight_v48_sequence_manager(&saved, &entities, &topology()).unwrap_err();

        assert_eq!(
            error,
            LegacySequenceAdoptError::NonMonotonicManagerOrder {
                previous: 20,
                current: 10,
            }
        );
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
        assert!(
            engine
                .orders
                .sequence_manager
                .get_sequence(existing_id)
                .is_some()
        );
    }

    #[test]
    fn rejects_missing_initialized_line_identity_strictly() {
        let (entities, _, _) = entities();
        let error = preflight_v48_sequence_manager(
            &fixture(),
            &entities,
            &LegacySequenceTopology::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            LegacySequenceAdoptError::MissingTopology {
                field: "movement.gate",
                ..
            }
        ));
    }
}

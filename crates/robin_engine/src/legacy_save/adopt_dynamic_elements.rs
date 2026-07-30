//! Strict construction plan for elements created after mission initialization.
//!
//! Original `RHEngine::SerializeElements` first removes live dynamic elements,
//! then walks the saved phase-one envelope in creation order. A saved static
//! identity is reused when present; otherwise the class-specific load switch
//! constructs a new element, consuming one draw from
//! `RHElement::gulCreationCounter`, before replacing that provisional identity
//! with the identity stored in the save. Constructing an `RHElementBonus`
//! additionally calls `ForceRandomSpriteFrame`, consuming one global RNG draw
//! before phase two overwrites the sprite state from the save.
//!
//! This module separates validation/construction from mutation. Preflight
//! resolves every immutable sprite/profile dependency and builds owned
//! entities. Applying a successful plan only appends those entities and
//! installs the exact Original identity map.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    character_kind::CharacterKind,
    coordinates::{MapVec, MoveBox},
    element::{
        ActorData, ActorPc, ElementBonus, ElementData, ElementKind, ElementNet, ElementProjectile,
        ElementScroll, Entity, EntityId, HULK_LENGTH, HumanData, NetData, ObjectData, ObjectType,
        ObjectTypeExt, PcData, ProjectileData,
    },
    engine::{EngineInner, LevelAssets},
    fast_find_grid::FastFindGrid,
    profiles::CharacterProfileIdx,
};

use super::{
    adopt::LegacyEntityFixups,
    campaign::LegacyPcDescription,
    elements::{
        LegacyDynamicElementFactory, LegacyElementClass, LegacyElementEnvelope,
        LegacyElementRecord, LegacyElementResolution,
    },
    topology_adapter::LegacyStaticElementTopology,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LegacyDynamicElementAdoptionError {
    #[error(
        "initialized engine contains non-static entity {entity_id}; dynamic save adoption requires a clean mission-start candidate"
    )]
    PreexistingDynamicEntity { entity_id: EntityId },
    #[error(
        "saved element slot {slot} creation order {creation_order} uses static resolution at or beyond boundary {boundary}"
    )]
    InvalidStaticResolution {
        slot: usize,
        creation_order: u32,
        boundary: u32,
    },
    #[error(
        "saved element slot {slot} creation order {creation_order} uses dynamic resolution below boundary {boundary}"
    )]
    InvalidDynamicResolution {
        slot: usize,
        creation_order: u32,
        boundary: u32,
    },
    #[error(
        "saved element slot {slot} creation order {creation_order} class {saved:?} does not match initialized class {initialized:?}"
    )]
    StaticClassMismatch {
        slot: usize,
        creation_order: u32,
        saved: LegacyElementClass,
        initialized: LegacyElementClass,
    },
    #[error(
        "saved element slot {slot} creation order {creation_order} resolves to an Original mobile master, which has no Rust entity identity"
    )]
    UnsupportedMobileMaster { slot: usize, creation_order: u32 },
    #[error(
        "saved element slot {slot} creation order {creation_order} class {class:?} has no initialized entity and no Original load factory"
    )]
    MissingStaticEntity {
        slot: usize,
        creation_order: u32,
        class: LegacyElementClass,
    },
    #[error(
        "saved element slot {slot} class {class:?} names factory {factory:?}, but that class maps to {expected:?}"
    )]
    FactoryClassMismatch {
        slot: usize,
        class: LegacyElementClass,
        factory: LegacyDynamicElementFactory,
        expected: Option<LegacyDynamicElementFactory>,
    },
    #[error("dynamic factory {factory:?} requires missing object sprite master {object_type:?}")]
    MissingObjectSpriteMaster {
        factory: LegacyDynamicElementFactory,
        object_type: ObjectType,
    },
    #[error("dynamic PC slot {slot} omits its campaign description index")]
    MissingPcDescriptionIndex { slot: usize },
    #[error(
        "dynamic PC slot {slot} references campaign description {description_index}, but only {description_count} descriptions were decoded"
    )]
    PcDescriptionOutOfRange {
        slot: usize,
        description_index: u32,
        description_count: usize,
    },
    #[error("dynamic PC description {description_index} has no character profile")]
    MissingPcProfileLink { description_index: u32 },
    #[error(
        "dynamic PC description {description_index} references missing character profile {profile_index}"
    )]
    MissingPcProfile {
        description_index: u32,
        profile_index: u32,
    },
    #[error(
        "dynamic PC description {description_index} requires missing character sprite master for profile {profile_index}"
    )]
    MissingPcSpriteMaster {
        description_index: u32,
        profile_index: u32,
    },
    #[error(
        "dynamic PC description {description_index} profile {profile_index} requires pathfinder move-box index {pathfinder_index}, but the loaded grid has only {move_box_count} entries"
    )]
    MissingPcMoveBox {
        description_index: u32,
        profile_index: u32,
        pathfinder_index: u8,
        move_box_count: usize,
    },
    #[error("dynamic PC description index {description_index} does not fit PcData::list_index")]
    PcDescriptionIndexOverflow { description_index: u32 },
    #[error(
        "saved Original creation counter {saved_creation_counter} overflows after {construction_count} dynamic constructions"
    )]
    CreationCounterOverflow {
        saved_creation_counter: u32,
        construction_count: usize,
    },
}

enum PlannedElement {
    Existing(EntityId),
    Construct(Entity),
}

/// Fully validated, mutation-free plan for Original phase-one element setup.
pub struct LegacyDynamicElementAdoptionPlan {
    elements: Vec<(usize, u32, PlannedElement)>,
    static_creation_orders: BTreeMap<EntityId, u32>,
    saved_creation_counter: u32,
    post_load_creation_counter: u32,
}

impl LegacyDynamicElementAdoptionPlan {
    pub fn preflight(
        engine: &EngineInner,
        assets: &LevelAssets,
        envelope: &LegacyElementEnvelope,
        topology: &LegacyStaticElementTopology,
        campaign_characters: &[LegacyPcDescription],
        saved_creation_counter: u32,
    ) -> Result<Self, LegacyDynamicElementAdoptionError> {
        let static_ids: BTreeSet<_> = topology.creation_order_by_entity.keys().copied().collect();
        for (entity_id, _) in engine.world.entities.occupied() {
            if !static_ids.contains(&entity_id) {
                return Err(
                    LegacyDynamicElementAdoptionError::PreexistingDynamicEntity { entity_id },
                );
            }
        }

        let initialized_by_creation_order: BTreeMap<_, _> = topology
            .creation_order_by_entity
            .iter()
            .map(|(&entity_id, &creation_order)| (creation_order, entity_id))
            .collect();
        let mut elements = Vec::with_capacity(envelope.records.len());
        let mut construction_count = 0usize;

        for record in &envelope.records {
            let planned = match record.resolution {
                LegacyElementResolution::ResolveStatic { fallback_factory } => {
                    if record.creation_order >= topology.static_creation_order_boundary {
                        return Err(LegacyDynamicElementAdoptionError::InvalidStaticResolution {
                            slot: record.slot,
                            creation_order: record.creation_order,
                            boundary: topology.static_creation_order_boundary,
                        });
                    }
                    if let Some(entity_id) = initialized_by_creation_order
                        .get(&record.creation_order)
                        .copied()
                    {
                        let initialized = topology
                            .payload_metadata
                            .by_creation_order
                            .get(&record.creation_order)
                            .expect("entity topology and payload metadata are built together")
                            .class;
                        if initialized != record.class {
                            return Err(LegacyDynamicElementAdoptionError::StaticClassMismatch {
                                slot: record.slot,
                                creation_order: record.creation_order,
                                saved: record.class,
                                initialized,
                            });
                        }
                        PlannedElement::Existing(entity_id)
                    } else if topology
                        .payload_metadata
                        .by_creation_order
                        .get(&record.creation_order)
                        .is_some_and(|metadata| metadata.class == LegacyElementClass::Mobile)
                    {
                        return Err(LegacyDynamicElementAdoptionError::UnsupportedMobileMaster {
                            slot: record.slot,
                            creation_order: record.creation_order,
                        });
                    } else if let Some(factory) = fallback_factory {
                        validate_factory(record, factory)?;
                        construction_count += 1;
                        PlannedElement::Construct(construct_entity(
                            record,
                            factory,
                            assets,
                            &engine.world.fast_grid,
                            campaign_characters,
                        )?)
                    } else {
                        return Err(LegacyDynamicElementAdoptionError::MissingStaticEntity {
                            slot: record.slot,
                            creation_order: record.creation_order,
                            class: record.class,
                        });
                    }
                }
                LegacyElementResolution::ConstructDynamic { factory } => {
                    if record.creation_order < topology.static_creation_order_boundary {
                        return Err(
                            LegacyDynamicElementAdoptionError::InvalidDynamicResolution {
                                slot: record.slot,
                                creation_order: record.creation_order,
                                boundary: topology.static_creation_order_boundary,
                            },
                        );
                    }
                    validate_factory(record, factory)?;
                    construction_count += 1;
                    PlannedElement::Construct(construct_entity(
                        record,
                        factory,
                        assets,
                        &engine.world.fast_grid,
                        campaign_characters,
                    )?)
                }
            };
            elements.push((record.slot, record.creation_order, planned));
        }

        let construction_count_u32 = u32::try_from(construction_count).map_err(|_| {
            LegacyDynamicElementAdoptionError::CreationCounterOverflow {
                saved_creation_counter,
                construction_count,
            }
        })?;
        let post_load_creation_counter = saved_creation_counter
            .checked_add(construction_count_u32)
            .ok_or(LegacyDynamicElementAdoptionError::CreationCounterOverflow {
                saved_creation_counter,
                construction_count,
            })?;

        Ok(Self {
            elements,
            static_creation_orders: topology.creation_order_by_entity.clone(),
            saved_creation_counter,
            post_load_creation_counter,
        })
    }

    pub fn post_load_creation_counter(&self) -> u32 {
        self.post_load_creation_counter
    }

    /// Apply the preflighted plan to a detached candidate engine.
    ///
    /// This is infallible: every allocation-independent dependency was
    /// resolved by [`Self::preflight`].
    pub fn apply(self, engine: &mut EngineInner) -> LegacyEntityFixups {
        engine.world.next_original_creation_order = self.saved_creation_counter;

        let mut by_creation_order = BTreeMap::new();
        let mut by_saved_slot = vec![None; self.elements.len()];
        let mut creation_order_by_entity = BTreeMap::new();
        let mut installed_creation_orders = self.static_creation_orders;

        for (slot, creation_order, planned) in self.elements {
            let entity_id = match planned {
                PlannedElement::Existing(entity_id) => entity_id,
                PlannedElement::Construct(entity) => {
                    if matches!(entity, Entity::Bonus(_)) {
                        // RHElementBonus::Initialize always invokes
                        // ForceRandomSpriteFrame during the phase-one load
                        // factory. Phase two immediately restores the saved
                        // sprite frame, so only the global draw ordering is
                        // authoritative here.
                        engine.with_simulation_context(|_, sim| {
                            let _ = crate::sim_rng::u32(
                                sim,
                                crate::sim_rng::RngSite::LevelBonusInitialFrame,
                                ..,
                            );
                        });
                    }
                    engine.add_entity(entity)
                }
            };
            by_creation_order.insert(creation_order, entity_id);
            by_saved_slot[slot] = Some(entity_id);
            if let Some(first_creation_order) =
                creation_order_by_entity.insert(entity_id, creation_order)
            {
                panic!(
                    "preflight admitted duplicate entity {entity_id}: creation orders {first_creation_order} and {creation_order}"
                );
            }
            installed_creation_orders.insert(entity_id, creation_order);
        }

        engine.world.install_original_creation_orders(
            installed_creation_orders,
            self.post_load_creation_counter,
        );

        LegacyEntityFixups {
            by_creation_order,
            by_saved_slot: by_saved_slot
                .into_iter()
                .map(|entity_id| entity_id.expect("phase-one slots are dense and preflighted"))
                .collect(),
            creation_order_by_entity,
        }
    }
}

fn validate_factory(
    record: &LegacyElementRecord,
    factory: LegacyDynamicElementFactory,
) -> Result<(), LegacyDynamicElementAdoptionError> {
    let expected = record.class.dynamic_factory();
    if expected != Some(factory) {
        return Err(LegacyDynamicElementAdoptionError::FactoryClassMismatch {
            slot: record.slot,
            class: record.class,
            factory,
            expected,
        });
    }
    Ok(())
}

fn construct_entity(
    record: &LegacyElementRecord,
    factory: LegacyDynamicElementFactory,
    assets: &LevelAssets,
    fast_grid: &FastFindGrid,
    campaign_characters: &[LegacyPcDescription],
) -> Result<Entity, LegacyDynamicElementAdoptionError> {
    if factory == LegacyDynamicElementFactory::ActorPc {
        return construct_pc(record, assets, fast_grid, campaign_characters);
    }

    let (object_type, kind) = factory_object(factory);
    let sprite = assets
        .accessory_sprite_prototypes
        .get(&object_type)
        .cloned()
        .ok_or(
            LegacyDynamicElementAdoptionError::MissingObjectSpriteMaster {
                factory,
                object_type,
            },
        )?;
    let element = ElementData {
        kind,
        sprite,
        ..Default::default()
    };
    let object = ObjectData {
        associated_action: object_type.to_action(),
        object_type,
        ..Default::default()
    };

    Ok(match kind {
        ElementKind::ObjectProjectile => Entity::Projectile(ElementProjectile {
            element,
            object,
            projectile: ProjectileData::default(),
        }),
        ElementKind::ObjectNet => Entity::Net(ElementNet {
            element,
            object,
            projectile: ProjectileData::default(),
            net: NetData::default(),
        }),
        ElementKind::ObjectOther | ElementKind::ObjectBonus => {
            Entity::Bonus(ElementBonus { element, object })
        }
        ElementKind::ObjectScroll => Entity::Scroll(ElementScroll {
            element,
            object,
            ..Default::default()
        }),
        _ => unreachable!("factory_object only returns concrete object kinds"),
    })
}

fn construct_pc(
    record: &LegacyElementRecord,
    assets: &LevelAssets,
    fast_grid: &FastFindGrid,
    campaign_characters: &[LegacyPcDescription],
) -> Result<Entity, LegacyDynamicElementAdoptionError> {
    let description_index = record.pc_description_index.ok_or(
        LegacyDynamicElementAdoptionError::MissingPcDescriptionIndex { slot: record.slot },
    )?;
    let description = campaign_characters.get(description_index as usize).ok_or(
        LegacyDynamicElementAdoptionError::PcDescriptionOutOfRange {
            slot: record.slot,
            description_index,
            description_count: campaign_characters.len(),
        },
    )?;
    let raw_profile_index = description
        .character_profile_index
        .ok_or(LegacyDynamicElementAdoptionError::MissingPcProfileLink { description_index })?;
    let profile_index = CharacterProfileIdx(raw_profile_index);
    let profile = assets.profile_manager.get_character(profile_index).ok_or(
        LegacyDynamicElementAdoptionError::MissingPcProfile {
            description_index,
            profile_index: raw_profile_index,
        },
    )?;
    let mut sprite = assets
        .character_sprite_prototypes
        .get(&profile_index)
        .cloned()
        .ok_or(LegacyDynamicElementAdoptionError::MissingPcSpriteMaster {
            description_index,
            profile_index: raw_profile_index,
        })?;
    let pathfinder_index = profile.pathfinder_index;
    let half_diagonal = fast_grid
        .try_move_box_half_diagonal(pathfinder_index as usize)
        .ok_or(LegacyDynamicElementAdoptionError::MissingPcMoveBox {
            description_index,
            profile_index: raw_profile_index,
            pathfinder_index,
            move_box_count: fast_grid.level.move_box_half_diagonals.len(),
        })?;
    // RHElementActorPC's constructor rebuilds these fields from the
    // character profile and the loaded fast-find grid. RHPositionInterface's
    // v48 serializer omits both fields, so phase-two save adoption must
    // preserve this constructor state.
    sprite
        .position_iface
        .set_pathfinder_index(pathfinder_index as u16);
    sprite.position_iface.set_move_box(MoveBox::from_corners(
        MapVec::new(-half_diagonal.x, -half_diagonal.y),
        MapVec::new(half_diagonal.x, half_diagonal.y),
    ));
    let list_index = u8::try_from(description_index).map_err(|_| {
        LegacyDynamicElementAdoptionError::PcDescriptionIndexOverflow { description_index }
    })?;
    let kind = CharacterKind::from_profile(&profile.filename, &profile.profile_name);
    let (has_lockpick, has_climb, has_jump) = PcData::movement_auth_from_profile(profile);

    Ok(Entity::Pc(ActorPc {
        element: ElementData {
            kind: ElementKind::ActorPc,
            sprite,
            ..Default::default()
        },
        actor: ActorData::default(),
        human: HumanData {
            time_hulk: HULK_LENGTH,
            ..Default::default()
        },
        pc: PcData {
            life_points: description.status.life_points,
            robin: kind.is_some_and(|kind| kind.is_robin()),
            list_index,
            campaign_description_index: Some(description_index),
            profile_index,
            kind,
            has_lockpick,
            has_climb,
            has_jump,
            ..Default::default()
        },
    }))
}

fn factory_object(factory: LegacyDynamicElementFactory) -> (ObjectType, ElementKind) {
    use LegacyDynamicElementFactory as F;
    use ObjectType as O;

    match factory {
        F::Apple => (O::Apple, ElementKind::ObjectProjectile),
        F::Arrow => (O::Arrow, ElementKind::ObjectProjectile),
        F::Coin => (O::Coin, ElementKind::ObjectProjectile),
        F::Net => (O::Net, ElementKind::ObjectNet),
        F::Purse => (O::Purse, ElementKind::ObjectProjectile),
        F::WaspNest => (O::WaspNest, ElementKind::ObjectProjectile),
        F::Wasp => (O::Wasp, ElementKind::ObjectProjectile),
        F::Stone => (O::Stone, ElementKind::ObjectProjectile),
        F::Ale => (O::Ale, ElementKind::ObjectOther),
        F::SpyCape => (O::Cape, ElementKind::ObjectOther),
        F::BonusAle => (O::BonusAle, ElementKind::ObjectBonus),
        F::BonusArrow => (O::BonusArrow, ElementKind::ObjectBonus),
        F::BonusApple => (O::BonusApple, ElementKind::ObjectBonus),
        F::BonusLambLeg => (O::BonusLambLeg, ElementKind::ObjectBonus),
        F::BonusNet => (O::BonusNet, ElementKind::ObjectBonus),
        F::BonusPlants => (O::BonusPlants, ElementKind::ObjectBonus),
        F::BonusPurse => (O::BonusPurse, ElementKind::ObjectBonus),
        F::BonusStone => (O::BonusStone, ElementKind::ObjectBonus),
        F::BonusWaspNest => (O::BonusWaspNest, ElementKind::ObjectBonus),
        F::Scroll => (O::Scroll, ElementKind::ObjectScroll),
        F::BonusAmulet => (O::BonusAmulet, ElementKind::ObjectBonus),
        F::BonusRansom => (O::BonusRansom, ElementKind::ObjectBonus),
        F::BonusAmpulla => (O::BonusAmpulla, ElementKind::ObjectBonus),
        F::BonusCoronationSpoon => (O::BonusCoronationSpoon, ElementKind::ObjectBonus),
        F::BonusRichardsCrown => (O::BonusRichardsCrown, ElementKind::ObjectBonus),
        F::BonusRoyalSeal => (O::BonusRoyalSeal, ElementKind::ObjectBonus),
        F::BonusRoyalSceptre => (O::BonusRoyalSceptre, ElementKind::ObjectBonus),
        F::BonusDomesdayBook => (O::BonusDomesdayBook, ElementKind::ObjectBonus),
        F::BonusSwordOfTheState => (O::BonusSwordOfTheState, ElementKind::ObjectBonus),
        F::ActorPc => unreachable!("PC construction is handled separately"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coordinates::MoveBoxHalfDiagonal,
        legacy_save::{
            campaign::{LegacyPcStatus, LegacySkill},
            elements::{LegacyElementFixupTable, LegacyElementRecord},
            payload_context::LegacyMissionPayloadMetadata,
        },
        profiles::CharacterProfile,
        sprite::Sprite,
    };

    fn dynamic_record(
        slot: usize,
        creation_order: u32,
        class: LegacyElementClass,
        factory: LegacyDynamicElementFactory,
    ) -> LegacyElementRecord {
        LegacyElementRecord {
            slot,
            class,
            creation_order,
            pc_description_index: None,
            resolution: LegacyElementResolution::ConstructDynamic { factory },
            creation_order_offset: 0,
        }
    }

    fn envelope(records: Vec<LegacyElementRecord>) -> LegacyElementEnvelope {
        LegacyElementEnvelope {
            start_offset: 0,
            phase2_offset: 0,
            fixups: LegacyElementFixupTable {
                by_creation_order: records
                    .iter()
                    .map(|record| (record.creation_order, record.slot))
                    .collect(),
            },
            records,
        }
    }

    fn empty_topology(boundary: u32) -> LegacyStaticElementTopology {
        LegacyStaticElementTopology {
            payload_metadata: LegacyMissionPayloadMetadata {
                static_creation_order_boundary: boundary,
                ..Default::default()
            },
            creation_order_by_entity: BTreeMap::new(),
            static_creation_order_boundary: boundary,
        }
    }

    fn pc_description(profile_index: u32) -> LegacyPcDescription {
        LegacyPcDescription {
            status: LegacyPcStatus {
                skills: [
                    LegacySkill {
                        capacity: 0,
                        experience: 0,
                    },
                    LegacySkill {
                        capacity: 0,
                        experience: 0,
                    },
                ],
                life_points: 100,
                in_coma: false,
                ales: 0,
                apples: 0,
                arrows: 0,
                nets: 0,
                plants: 0,
                purses: 0,
                rations: 0,
                stones: 0,
                wasp_nests: 0,
                beam_me_index_in_sherwood: -1,
                name: String::new(),
            },
            character_profile_index: Some(profile_index),
            instanced: true,
        }
    }

    #[test]
    fn constructs_in_saved_order_and_installs_exact_identities() {
        let mut engine = EngineInner::new();
        engine.control.rng = crate::engine::SimulationRng::with_original_replay(vec![17, 23]);
        let mut assets = LevelAssets::new();
        assets
            .accessory_sprite_prototypes
            .insert(ObjectType::Arrow, Sprite::default());
        assets
            .accessory_sprite_prototypes
            .insert(ObjectType::BonusPlants, Sprite::default());
        let envelope = envelope(vec![
            dynamic_record(
                0,
                45,
                LegacyElementClass::Arrow,
                LegacyDynamicElementFactory::Arrow,
            ),
            dynamic_record(
                1,
                91,
                LegacyElementClass::BonusPlants,
                LegacyDynamicElementFactory::BonusPlants,
            ),
        ]);

        let plan = LegacyDynamicElementAdoptionPlan::preflight(
            &engine,
            &assets,
            &envelope,
            &empty_topology(40),
            &[],
            120,
        )
        .unwrap();
        assert_eq!(plan.post_load_creation_counter(), 122);
        let fixups = plan.apply(&mut engine);

        assert_eq!(fixups.by_saved_slot.len(), 2);
        assert!(matches!(
            engine.get_entity(fixups.by_saved_slot[0]),
            Some(Entity::Projectile(projectile))
                if projectile.object.object_type == ObjectType::Arrow
        ));
        assert!(matches!(
            engine.get_entity(fixups.by_saved_slot[1]),
            Some(Entity::Bonus(bonus))
                if bonus.object.object_type == ObjectType::BonusPlants
        ));
        assert_eq!(
            engine
                .world
                .original_creation_order(fixups.by_saved_slot[0]),
            45
        );
        assert_eq!(
            engine
                .world
                .original_creation_order(fixups.by_saved_slot[1]),
            91
        );
        assert_eq!(engine.world.next_original_creation_order, 122);
        assert_eq!(engine.control.rng.original_replay_cursor(), Some(1));
        assert_eq!(
            engine
                .control
                .rng
                .original_replay_sites(0..1)
                .expect("Original RNG replay"),
            vec![crate::sim_rng::RngSite::LevelBonusInitialFrame]
        );
    }

    #[test]
    fn rejects_missing_master_without_mutating_engine() {
        let engine = EngineInner::new();
        let assets = LevelAssets::new();
        let envelope = envelope(vec![dynamic_record(
            0,
            45,
            LegacyElementClass::Net,
            LegacyDynamicElementFactory::Net,
        )]);

        let error = LegacyDynamicElementAdoptionPlan::preflight(
            &engine,
            &assets,
            &envelope,
            &empty_topology(40),
            &[],
            100,
        )
        .err()
        .expect("missing master must fail preflight");

        assert_eq!(
            error,
            LegacyDynamicElementAdoptionError::MissingObjectSpriteMaster {
                factory: LegacyDynamicElementFactory::Net,
                object_type: ObjectType::Net,
            }
        );
        assert_eq!(engine.world.entities.occupied().count(), 0);
    }

    #[test]
    fn rejects_factory_and_class_disagreement() {
        let engine = EngineInner::new();
        let assets = LevelAssets::new();
        let envelope = envelope(vec![dynamic_record(
            0,
            45,
            LegacyElementClass::Arrow,
            LegacyDynamicElementFactory::Stone,
        )]);

        let error = LegacyDynamicElementAdoptionPlan::preflight(
            &engine,
            &assets,
            &envelope,
            &empty_topology(40),
            &[],
            100,
        )
        .err()
        .expect("mismatched factory must fail preflight");

        assert_eq!(
            error,
            LegacyDynamicElementAdoptionError::FactoryClassMismatch {
                slot: 0,
                class: LegacyElementClass::Arrow,
                factory: LegacyDynamicElementFactory::Stone,
                expected: Some(LegacyDynamicElementFactory::Arrow),
            }
        );
    }

    #[test]
    fn dynamic_pc_rebuilds_constructor_owned_pathfinder_geometry() {
        let mut engine = EngineInner::new();
        engine
            .world
            .fast_grid
            .add_move_box_half_diagonal(MoveBoxHalfDiagonal::new(2.0, 3.0));
        engine
            .world
            .fast_grid
            .add_move_box_half_diagonal(MoveBoxHalfDiagonal::new(7.0, 11.0));
        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .push(CharacterProfile {
                pathfinder_index: 1,
                ..Default::default()
            });
        assets
            .character_sprite_prototypes
            .insert(CharacterProfileIdx(0), Sprite::default());
        let mut record = dynamic_record(
            0,
            45,
            LegacyElementClass::ActorPc,
            LegacyDynamicElementFactory::ActorPc,
        );
        record.pc_description_index = Some(0);
        let envelope = envelope(vec![record]);

        let plan = LegacyDynamicElementAdoptionPlan::preflight(
            &engine,
            &assets,
            &envelope,
            &empty_topology(40),
            &[pc_description(0)],
            100,
        )
        .unwrap();
        let fixups = plan.apply(&mut engine);
        let Entity::Pc(pc) = engine
            .get_entity(fixups.by_saved_slot[0])
            .expect("constructed PC")
        else {
            panic!("dynamic PC factory constructed a non-PC entity");
        };
        let position = &pc.element.sprite.position_iface;

        assert_eq!(position.get_pathfinder_index(), 1);
        assert_eq!(
            position.get_half_diagonal(),
            MoveBoxHalfDiagonal::new(7.0, 11.0)
        );
    }

    #[test]
    fn dynamic_pc_rejects_missing_constructor_move_box() {
        let engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .push(CharacterProfile {
                pathfinder_index: 3,
                ..Default::default()
            });
        assets
            .character_sprite_prototypes
            .insert(CharacterProfileIdx(0), Sprite::default());
        let mut record = dynamic_record(
            0,
            45,
            LegacyElementClass::ActorPc,
            LegacyDynamicElementFactory::ActorPc,
        );
        record.pc_description_index = Some(0);
        let envelope = envelope(vec![record]);

        let error = LegacyDynamicElementAdoptionPlan::preflight(
            &engine,
            &assets,
            &envelope,
            &empty_topology(40),
            &[pc_description(0)],
            100,
        )
        .err()
        .expect("missing constructor move box must fail preflight");

        assert_eq!(
            error,
            LegacyDynamicElementAdoptionError::MissingPcMoveBox {
                description_index: 0,
                profile_index: 0,
                pathfinder_index: 3,
                move_box_count: 0,
            }
        );
        assert_eq!(engine.world.entities.occupied().count(), 0);
    }
}

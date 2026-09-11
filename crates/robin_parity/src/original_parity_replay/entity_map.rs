//! Original-to-runtime identity translation; semantic ordinals remain unchanged.
use super::{
    BTreeMap, BTreeSet, Engine, EntityId, EntityIdKind, LevelAssets, MapPoint, SectorNumber,
    TraceEntityId, TraceEntityKind, TraceFrame,
};

pub(super) struct EntityMap {
    /// Per-frame Original array index to Rust entity. Original array indices
    /// can shift after a physical removal, so this is a view rather than the
    /// durable identity registry.
    pub(super) entities: BTreeMap<TraceEntityId, EntityId>,
    /// Original's immutable per-engine construction serial is the durable
    /// identity anchor for refreshing the per-frame raw-index view.
    pub(super) entities_by_creation_order: BTreeMap<u32, EntityId>,
    /// Original sparse fast-grid sector slot to this implementation's compact
    /// canonical position-sector number. The raw numbers are allocation
    /// details rather than gameplay identity.
    pub(super) sectors: BTreeMap<u16, u16>,
    /// Original sparse sector slot to the exact Rust FastFindGrid arena slot.
    /// Public sector numbers are insufficient when retained overlays share an
    /// identity.
    pub(super) sector_indices: BTreeMap<u16, robin_engine::fast_find_grid::SectorIndex>,
    /// Original mixed gate-array slot to Rust's runtime door-table index.
    pub(super) gates: Vec<robin_engine::gate::DoorIndex>,
    /// One past the highest creation order the mission start established.
    /// Below it the Original's serials come from the mission file or the save
    /// and are exact; at or above it they also count the throwaway elements
    /// described on [`Self::extend_runtime_entities`], so only their relative
    /// order is comparable.
    pub(super) runtime_creation_order_boundary: u32,
}

impl EntityMap {
    /// Build the exact one-to-one correspondence from Original's immutable
    /// per-engine construction serial.
    ///
    /// The trace frame is post-tick while this runs against Rust's pre-tick
    /// state, so position, active state and posture are all invalid identity
    /// labels here. In particular, several inactive beam PCs can be colocated
    /// and then start moving on the first recorded frame. Both mission loading
    /// and legacy-save adoption install the authoritative Original creation
    /// order on every Rust entity, including gaps consumed by mobile masters.
    pub(super) fn build(engine: &Engine, assets: &LevelAssets, frame: &TraceFrame) -> Self {
        let mut rust_by_creation_order = BTreeMap::new();
        for (id, entity) in engine.entities_with_ids_iter() {
            let creation_order = engine.original_creation_order(id);
            if let Some((previous, _)) =
                rust_by_creation_order.insert(creation_order, (id, entity.entity_id_kind()))
            {
                panic!(
                    "Rust entities {previous:?} and {id:?} share Original creation order \
                     {creation_order}"
                );
            }
        }
        assert_eq!(
            frame.elements.len(),
            rust_by_creation_order.len(),
            "entity tables have different cardinality"
        );

        let mut result = BTreeMap::new();
        let mut entities_by_creation_order = BTreeMap::new();
        for original in &frame.elements {
            let expected_kind = EntityIdKind::from(original.entity_id.kind);
            let &(rust_id, actual_kind) = rust_by_creation_order
                .get(&original.creation_order)
                .unwrap_or_else(|| {
                    panic!(
                        "Original {:?} has creation order {}, absent from the Rust identity table",
                        original.entity_id, original.creation_order
                    )
                });
            assert_eq!(
                actual_kind, expected_kind,
                "Original {:?} creation order {} has kind {:?}, but Rust {rust_id:?} has kind \
                 {actual_kind:?}",
                original.entity_id, original.creation_order, expected_kind,
            );
            assert!(
                result.insert(original.entity_id, rust_id).is_none(),
                "Original trace entity {:?} occurs twice",
                original.entity_id
            );
            assert!(
                entities_by_creation_order
                    .insert(original.creation_order, rust_id)
                    .is_none(),
                "Original creation order {} occurs twice in the trace",
                original.creation_order
            );
        }
        let retained = assets
            .navigation
            .legacy_grid_topology
            .as_ref()
            .expect("parity replay requires retained Original fast-grid topology");
        let mut sectors = BTreeMap::new();
        let mut sector_indices = BTreeMap::new();
        for (original, (runtime, runtime_index)) in retained
            .position_sector_numbers
            .iter()
            .zip(&retained.position_sector_indices)
            .enumerate()
        {
            let Some(runtime) = runtime else {
                assert!(
                    runtime_index.is_none(),
                    "Original sparse sector slot {original} has an arena index but no public number"
                );
                continue;
            };
            let runtime_index = runtime_index.unwrap_or_else(|| {
                panic!("Original sparse sector slot {original} has a public number but no exact Rust arena index")
            });
            let original = u16::try_from(original)
                .expect("Original sparse sector slot exceeds its u16 identity domain");
            let runtime =
                u16::try_from(*runtime).expect("Rust canonical sector number is negative");
            assert!(
                sectors.insert(original, runtime).is_none(),
                "Original sparse sector slot {original} was mapped twice"
            );
            assert!(
                sector_indices.insert(original, runtime_index).is_none(),
                "Original sparse sector slot {original} had two exact arena mappings"
            );
        }
        let runtime_creation_order_boundary = entities_by_creation_order
            .keys()
            .next_back()
            .map_or(0, |highest| highest + 1);
        Self {
            entities: result,
            entities_by_creation_order,
            sectors,
            sector_indices,
            gates: engine.legacy_gate_order(assets),
            runtime_creation_order_boundary,
        }
    }

    /// Whether the Original's raw serial for this element is an exact
    /// cross-engine value rather than one carrying presentation-only gaps.
    pub(super) fn creation_order_is_exact(&self, creation_order: u32) -> bool {
        creation_order < self.runtime_creation_order_boundary
    }

    pub(super) fn refresh_trace_indices(&mut self, frame: &TraceFrame) {
        for original in &frame.elements {
            self.refresh_trace_index(original.entity_id, original.creation_order);
        }
    }

    /// Verify hidden-building occupants against the construction-topology
    /// isomorphism. The mapping itself must not be learned from mutable frame
    /// state: an initially active tenant can cross the first-frame comparison
    /// boundary before any inactive occupant exposes that building.
    pub(super) fn validate_building_sector_mapping(&self, engine: &Engine, frame: &TraceFrame) {
        for original in frame
            .elements
            .iter()
            .filter(|element| element.actor.is_some() && !element.active)
        {
            let Some(&rust_id) = self.entities.get(&original.entity_id) else {
                continue;
            };
            let Some(actual) = engine.get_entity(rust_id) else {
                continue;
            };
            let actual_element = actual.element_data();
            if actual_element.active || !actual_element.hidden_in_building {
                continue;
            }
            let Some(actual_sector) = actual_element.sector().map(|sector| sector.get()) else {
                continue;
            };
            let expected_position: MapPoint = original.position_map.into();
            let actual_position = actual_element.position_map();
            if expected_position.x.to_bits() != actual_position.x.to_bits()
                || expected_position.y.to_bits() != actual_position.y.to_bits()
                || original.sector == actual_sector
            {
                continue;
            }
            assert_eq!(
                self.sectors.get(&original.sector),
                Some(&actual_sector),
                "hidden building occupant exposed a sector identity absent from retained topology"
            );
        }
    }

    pub(super) fn refresh_trace_index(&mut self, trace_id: TraceEntityId, creation_order: u32) {
        if let Some(rust_id) = self
            .entities_by_creation_order
            .get(&creation_order)
            .copied()
        {
            self.entities.insert(trace_id, rust_id);
        }
    }

    /// Extend the mission-start bijection for persistent entities created while
    /// replaying the mission.
    ///
    /// Runtime creation orders are not exact cross-engine identities. Original
    /// cursor previews construct temporary element projectiles (for
    /// example the temporary arrow in trajectory validation), consuming
    /// global construction orders without ever adding those objects to the
    /// world. Rust computes the same presentation preview without constructing
    /// an entity. Match newly persistent entities isomorphically by global
    /// persistent construction rank and require the concrete kind at every
    /// rank to agree; raw order numbers may differ by presentation-only gaps.
    pub(super) fn extend_runtime_entities(&mut self, engine: &Engine, frame: &TraceFrame) {
        self.refresh_trace_indices(frame);
        let originals: Vec<_> = frame
            .elements
            .iter()
            .filter(|element| {
                !self
                    .entities_by_creation_order
                    .contains_key(&element.creation_order)
            })
            .collect();
        let used: BTreeSet<_> = self.entities_by_creation_order.values().copied().collect();
        let mut rust_by_creation_order = BTreeMap::new();
        for (id, entity) in engine
            .entities_with_ids_iter()
            .filter(|(id, _)| !used.contains(id))
        {
            let creation_order = engine.original_creation_order(id);
            if let Some((previous, _)) =
                rust_by_creation_order.insert(creation_order, (id, entity.entity_id_kind()))
            {
                panic!(
                    "unmapped Rust entities {previous:?} and {id:?} share Original creation \
                     order {creation_order}"
                );
            }
        }
        let original_identities: Vec<_> = originals
            .iter()
            .map(|element| {
                (
                    element.entity_id,
                    element.creation_order,
                    EntityIdKind::from(element.entity_id.kind),
                )
            })
            .collect();
        let rust_identities: Vec<_> = rust_by_creation_order
            .iter()
            .map(|(&creation_order, &(id, kind))| (id, creation_order, kind))
            .collect();
        let pairs = pair_runtime_identities_by_persistent_rank(
            original_identities.clone(),
            rust_identities.clone(),
        )
        .unwrap_or_else(|detail| {
            panic!(
                "runtime persistent entity identity mismatch: {detail}; \
                 Original={original_identities:?}; Rust={rust_identities:?}"
            )
        });

        let originals_by_id: BTreeMap<_, _> = originals
            .into_iter()
            .map(|original| (original.entity_id, original))
            .collect();
        for (original_id, original_creation_order, rust_id) in pairs {
            let original = originals_by_id
                .get(&original_id)
                .expect("runtime identity pairing returned an unknown Original entity");
            debug_assert_eq!(original.creation_order, original_creation_order);
            self.entities.insert(original.entity_id, rust_id);
            assert!(
                self.entities_by_creation_order
                    .insert(original.creation_order, rust_id)
                    .is_none(),
                "Original creation order {} was mapped twice",
                original.creation_order
            );
        }
    }

    pub(super) fn translate(&self, original: TraceEntityId) -> EntityId {
        *self
            .entities
            .get(&original)
            .unwrap_or_else(|| panic!("original entity {original:?} has no Rust correspondence"))
    }

    pub(super) fn translate_gate(&self, original: u32) -> u32 {
        let original_index = usize::try_from(original)
            .unwrap_or_else(|_| panic!("Original gate index {original} exceeds usize"));
        self.gates
            .get(original_index)
            .copied()
            .unwrap_or_else(|| {
                panic!("Original gate index {original} is absent from the retained gate topology")
            })
            .into()
    }

    /// Preserve the patch-aware goal sector recorded by group movement.
    /// A retained canonical position sector is translated normally. A true
    /// unmapped initial value remains an authoritative route goal in the original game's
    /// sparse identity domain: keeping
    /// its number forces the same movement-sequence gate search instead of
    /// silently substituting Rust's coincident selected-sector overlay.
    pub(super) fn translate_group_move_goal_sector(
        &self,
        original: i16,
        layer: u16,
        unmapped_goal_search_sector: Option<u16>,
    ) -> GroupMoveGoalTranslation {
        let original = u16::try_from(original)
            .unwrap_or_else(|_| panic!("Original group-move sector is negative: {original}"));
        if let Some(&runtime) = self.sectors.get(&original) {
            let runtime = i16::try_from(runtime).unwrap_or_else(|_| {
                panic!("Rust position sector {runtime} exceeds its signed identity domain")
            });
            let index = *self.sector_indices.get(&original).unwrap_or_else(|| {
                panic!("mapped Original group-move sector {original} lost its exact Rust arena identity")
            });
            GroupMoveGoalTranslation::Runtime((SectorNumber::new(runtime), layer), index)
        } else if let Some(search_sector) = unmapped_goal_search_sector {
            let runtime = self.sectors.get(&search_sector).copied().unwrap_or_else(|| {
                panic!(
                    "successful group-move route terminal Original sector {search_sector} has no retained Rust position-sector mapping"
                )
            });
            let runtime = i16::try_from(runtime).unwrap_or_else(|_| {
                panic!("Rust position sector {runtime} exceeds its signed identity domain")
            });
            let index = *self.sector_indices.get(&search_sector).unwrap_or_else(|| {
                panic!("mapped group-move terminal sector {search_sector} lost its exact Rust arena identity")
            });
            GroupMoveGoalTranslation::Runtime((SectorNumber::new(runtime), layer), index)
        } else {
            let recorded = i16::try_from(original).unwrap_or_else(|_| {
                panic!("Original group-move sector {original} exceeds its signed identity domain")
            });
            GroupMoveGoalTranslation::RecordedUnmapped((SectorNumber::new(recorded), layer))
        }
    }

    pub(super) fn translate_required_drop_ale_goal_sector(
        &self,
        original: u16,
    ) -> (SectorNumber, robin_engine::fast_find_grid::SectorIndex) {
        let runtime = self.sectors.get(&original).copied().unwrap_or_else(|| {
            panic!(
                "schema-16 DropAle route goal Original sector {original} has no retained Rust position-sector mapping"
            )
        });
        let runtime = i16::try_from(runtime).unwrap_or_else(|_| {
            panic!("Rust DropAle goal sector {runtime} exceeds its signed identity domain")
        });
        let index = *self.sector_indices.get(&original).unwrap_or_else(|| {
            panic!(
                "mapped schema-16 DropAle route goal Original sector {original} lost its exact Rust arena identity"
            )
        });
        (SectorNumber::new(runtime), index)
    }

    pub(super) fn sectors_equivalent(&self, original: u16, rust: u16) -> bool {
        original == rust || self.sectors.get(&original) == Some(&rust)
    }

    pub(super) fn translate_sector(&self, original: u16) -> u16 {
        self.sectors.get(&original).copied().unwrap_or(original)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GroupMoveGoalTranslation {
    Runtime(
        (SectorNumber, u16),
        robin_engine::fast_find_grid::SectorIndex,
    ),
    RecordedUnmapped((SectorNumber, u16)),
}

pub(super) fn pair_runtime_identities_by_persistent_rank(
    mut originals: Vec<(TraceEntityId, u32, EntityIdKind)>,
    mut rust: Vec<(EntityId, u32, EntityIdKind)>,
) -> Result<Vec<(TraceEntityId, u32, EntityId)>, String> {
    originals.sort_by_key(|(_, creation_order, _)| *creation_order);
    rust.sort_by_key(|(_, creation_order, _)| *creation_order);
    if originals.len() != rust.len() {
        return Err(format!(
            "different persistent cardinality (Original {}, Rust {})",
            originals.len(),
            rust.len()
        ));
    }

    originals
        .into_iter()
        .zip(rust)
        .enumerate()
        .map(
            |(
                rank,
                ((original_id, original_order, original_kind), (rust_id, rust_order, rust_kind)),
            )| {
                if original_kind != rust_kind {
                    return Err(format!(
                        "persistent creation rank {rank} has Original {original_kind:?} order \
                         {original_order}, but Rust {rust_kind:?} order {rust_order}"
                    ));
                }
                Ok((original_id, original_order, rust_id))
            },
        )
        .collect()
}

impl From<TraceEntityKind> for EntityIdKind {
    fn from(value: TraceEntityKind) -> Self {
        match value {
            TraceEntityKind::Pc => Self::Pc,
            TraceEntityKind::Soldier => Self::Soldier,
            TraceEntityKind::Civilian => Self::Civilian,
            TraceEntityKind::Fx => Self::Fx,
            TraceEntityKind::Target => Self::Target,
            TraceEntityKind::Bonus => Self::Bonus,
            TraceEntityKind::Scroll => Self::Scroll,
            TraceEntityKind::Projectile => Self::Projectile,
            TraceEntityKind::Net => Self::Net,
        }
    }
}

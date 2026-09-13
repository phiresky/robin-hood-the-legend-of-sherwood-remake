//! Shared adoption error and the small validation helpers every adopter uses.
//!
//! Every `adopt_*` stage reports failures as one [`LegacyAdoptError`]: a
//! subject phrase naming the saved object ("saved NPC", "serialized VM owner
//! Element(3)"), the optional Original creation order and field, and a typed
//! [`AdoptErrorKind`] predicate. The rendered message is
//! `"{subject} creation order {n} field {field} {kind}"` with absent parts
//! omitted, so generic kinds such as [`AdoptErrorKind::NonFinite`] read the
//! same regardless of which stage raised them.

use std::borrow::Cow;
use std::fmt;

use crate::{
    coordinates::{MapPoint, MapVec, WorldPoint3D, WorldVec3D},
    element::{Command, EntityId, ObjectType, QuickAction},
    gate::GateType,
    sequence::{Field, SequenceElementRef, SequenceState},
};

use super::{
    elements::{LegacyDynamicElementFactory, LegacyElementClass},
    payload_base::{LegacyPoint2, LegacyPoint3},
    payload_vm::LegacyVmMemberKind,
    topology_adapter::LegacyMissingTopologyFact,
};

/// One failed validation while planning Original v48 save adoption.
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyAdoptError {
    /// Noun phrase for the saved or initialized object; may be empty when the
    /// kind's message is self-contained.
    pub subject: Cow<'static, str>,
    pub creation_order: Option<u32>,
    pub field: Option<Cow<'static, str>>,
    pub kind: AdoptErrorKind,
}

impl LegacyAdoptError {
    pub fn new(subject: impl Into<Cow<'static, str>>, kind: AdoptErrorKind) -> Self {
        Self {
            subject: subject.into(),
            creation_order: None,
            field: None,
            kind,
        }
    }

    pub fn with_creation_order(mut self, creation_order: u32) -> Self {
        self.creation_order = Some(creation_order);
        self
    }

    pub fn with_field(mut self, field: impl Into<Cow<'static, str>>) -> Self {
        self.field = Some(field.into());
        self
    }

    /// Wrap this error in an outer explanation, rendered `"{context}: {self}"`.
    pub fn context(self, context: impl Into<Cow<'static, str>>) -> Self {
        AdoptErrorKind::Context {
            context: context.into(),
            source: Box::new(self),
        }
        .into()
    }
}

impl From<AdoptErrorKind> for LegacyAdoptError {
    fn from(kind: AdoptErrorKind) -> Self {
        Self::new("", kind)
    }
}

impl fmt::Display for LegacyAdoptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        if !self.subject.is_empty() {
            formatter.write_str(&self.subject)?;
            separator = " ";
        }
        if let Some(creation_order) = self.creation_order {
            write!(formatter, "{separator}creation order {creation_order}")?;
            separator = " ";
        }
        if let Some(field) = &self.field {
            write!(formatter, "{separator}field {field}")?;
            separator = " ";
        }
        write!(formatter, "{separator}{}", self.kind)
    }
}

impl std::error::Error for LegacyAdoptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            AdoptErrorKind::Context { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// What went wrong. Generic kinds render as a predicate on the error's
/// subject/creation order/field; stage-unique kinds carry their full message.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum AdoptErrorKind {
    // --- Generic predicates -------------------------------------------------
    #[error("{context}: {source}")]
    Context {
        context: Cow<'static, str>,
        source: Box<LegacyAdoptError>,
    },
    #[error("contains non-finite value {value}")]
    NonFinite { value: f32 },
    #[error("has value {value}; expected {expected}")]
    InvalidValue {
        value: String,
        expected: &'static str,
    },
    #[error("has unknown enum value {value}")]
    UnknownEnum { value: i64 },
    #[error("is null")]
    NullReference,
    #[error("has no {what}")]
    Missing { what: &'static str },
    #[error("references {what} {index}, but only {count} exist")]
    OutOfRange {
        what: &'static str,
        index: usize,
        count: usize,
    },
    #[error("resolves to missing entity {entity_id}")]
    MissingEntity { entity_id: EntityId },
    #[error("resolves to {entity_id}; expected {expected}")]
    WrongEntityKind {
        entity_id: EntityId,
        expected: &'static str,
    },
    #[error("has {saved} entries, but the initialized mission has {runtime}")]
    CountMismatch { saved: usize, runtime: usize },

    // --- Topology derivation ------------------------------------------------
    #[error("cannot derive {fact:?}: Original owns it in {original_owner}; {detail}")]
    MissingRetainedFact {
        fact: LegacyMissingTopologyFact,
        original_owner: &'static str,
        detail: &'static str,
    },
    #[error(
        "cannot derive {fact}: initialized engine value {engine_value} does not match attached level asset value {asset_value}"
    )]
    MissionAttachmentMismatch {
        fact: &'static str,
        engine_value: String,
        asset_value: String,
    },
    #[error("cannot derive Original {what} topology: {detail}")]
    TopologyMismatch { what: &'static str, detail: String },
    #[error(
        "references Original sector slot {index}, which has no Rust position-sector counterpart"
    )]
    UnmappedSector { index: usize },

    // --- Element identity ---------------------------------------------------
    #[error("slot {slot}, class {class:?} has no initialized Rust entity")]
    MissingStaticEntity {
        slot: usize,
        class: LegacyElementClass,
    },
    #[error("slot {slot}, class {class:?} requires dynamic factory adoption")]
    UnsupportedDynamicElement {
        slot: usize,
        class: LegacyElementClass,
    },
    #[error(
        "initialized entity {entity_id} occurs at both Original creation orders {first_creation_order} and {second_creation_order}"
    )]
    DuplicateInitializedEntity {
        entity_id: EntityId,
        first_creation_order: u32,
        second_creation_order: u32,
    },
    #[error("save references absent Original element creation order {creation_order}")]
    MissingCreationOrderReference { creation_order: u32 },
    #[error(
        "save references AI element slot {slot}, but the serialized element array contains only {element_count} records"
    )]
    MissingAiElementSlot { slot: u16, element_count: usize },
    #[error(
        "save references Original mobile master at AI element slot {slot}; mobile masters are not actors"
    )]
    MobileMasterAiReference { slot: u16 },

    // --- Jump-line identity -------------------------------------------------
    #[error("initialized jump-line runtime index {index} exceeds u32")]
    JumpLineRuntimeIndexOverflow { index: usize },
    #[error("initialized jump-line runtime index equals the null sentinel")]
    JumpLineRuntimeIndexNullSentinel,
    #[error("initialized jump-line layer {layer} contains more than i16::MAX entries")]
    JumpLineLayerIndexOverflow { layer: u16 },
    #[error("initialized mission retained {retained} jump-line identities for {runtime} lines")]
    JumpLineRetainedCountMismatch { retained: usize, runtime: usize },
    #[error(
        "initialized mission retained duplicate jump-line identity layer {layer}, index {index}"
    )]
    DuplicateJumpLineIdentity { layer: u16, index: i16 },
    #[error("has inconsistent null identity layer={layer:?}, index={index:?}")]
    InconsistentLineNull {
        layer: Option<u16>,
        index: Option<i16>,
    },
    #[error("references missing Original line layer {layer}, index {index}")]
    MissingLine { layer: u16, index: i16 },
    #[error(
        "references shifted Original line layer {layer}, index {index}, but owner {owner} and primary target {target} do not identify a reciprocal table-swordfight line"
    )]
    MissingLineGeometry {
        layer: u16,
        index: i16,
        owner: u32,
        target: u32,
    },
    #[error(
        "references shifted Original line layer {layer}, index {index}, but owner {owner} and primary target {target} ambiguously identify runtime lines {candidates:?}"
    )]
    AmbiguousLineGeometry {
        layer: u16,
        index: i16,
        owner: u32,
        target: u32,
        candidates: Vec<u32>,
    },

    // --- Gate order ---------------------------------------------------------
    #[error(
        "Original {kind} at gate index {saved_index} has no initialized Rust peer \
         (retained doors={retained_doors}, jumps={retained_jumps}; \
         initialized doors={runtime_doors}, jumps={runtime_jumps})"
    )]
    GateMissingPeer {
        kind: &'static str,
        saved_index: usize,
        retained_doors: usize,
        retained_jumps: usize,
        runtime_doors: usize,
        runtime_jumps: usize,
    },
    #[error(
        "gate-kind counts differ (retained doors={retained_doors}, jumps={retained_jumps}; \
         initialized doors={runtime_doors}, jumps={runtime_jumps})"
    )]
    GateCountMismatch {
        retained_doors: usize,
        retained_jumps: usize,
        runtime_doors: usize,
        runtime_jumps: usize,
    },
    #[error("initialized gate index {index} has unsupported kind {kind:?}")]
    UnsupportedRuntimeGateKind { index: usize, kind: GateType },
    #[error("runtime gate index {index} exceeds u32")]
    GateRuntimeIndexOverflow { index: usize },

    // --- Campaign -----------------------------------------------------------
    #[error("save header mission id {mission_id} has no matching static mission profile")]
    MissingHeaderMissionProfile { mission_id: u32 },
    #[error(
        "save header mission id {mission_id} (profile index {profile_index}) is absent from the campaign"
    )]
    HeaderMissionMissingFromCampaign {
        mission_id: u32,
        profile_index: usize,
    },
    #[error("Original campaign has {count} recent missions; format permits at most 3")]
    TooManyRecentMissions { count: usize },
    #[error(
        "saved campaign streams disagree on mission identity: live mission/profile {live_mission}/{live_profile}, backup {backup_mission}/{backup_profile}"
    )]
    CampaignIdentityMismatch {
        live_mission: u32,
        live_profile: usize,
        backup_mission: u32,
        backup_profile: usize,
    },
    #[error(
        "Original save mission profile id {save_mission_id} does not match initialized mission profile id {initialized_mission_id}"
    )]
    MissionProfileMismatch {
        save_mission_id: u32,
        initialized_mission_id: u32,
    },

    // --- Sequences ----------------------------------------------------------
    #[error("names absent ID {id}")]
    MissingIdentity { id: u32 },
    #[error("names {identity}, absent from the initialized mission")]
    MissingTopology { identity: String },
    #[error("saved sequence {sequence_id} has duplicate generic field {field:?}")]
    DuplicateGenericField { sequence_id: u32, field: Field },

    // --- Script VM heaps ----------------------------------------------------
    #[error("occurs more than once")]
    DuplicateVmOwner,
    #[error("was not planned by the shared VM arena")]
    UnplannedVmOwner,
    #[error("has {actual} serialized Location members, but the shared arena reserved {expected}")]
    VmOwnerLocationCountMismatch { expected: usize, actual: usize },
    #[error("script VM presence is {saved}, but initialized presence is {runtime}")]
    VmPresenceMismatch { saved: bool, runtime: bool },
    #[error("VM class is {saved:?}, but initialized class is {runtime:?}")]
    VmClassMismatch { saved: String, runtime: String },
    #[error("VM member count is {saved}, but initialized class {class_name:?} has {runtime}")]
    VmMemberCountMismatch {
        class_name: String,
        saved: usize,
        runtime: usize,
    },
    #[error("VM member {index} schema mismatch: {detail}")]
    VmSchemaMismatch { index: usize, detail: String },
    #[error("requires VM heap bytes {address}..{end}, outside initialized heap length {heap_len}")]
    VmHeapRange {
        heap_len: usize,
        address: usize,
        end: usize,
    },
    #[error("requires unrepresentable script handle index {index}")]
    VmHandleOverflow { index: usize },
    #[error("declares {kind:?}, but its decoded value is {value_kind}")]
    VmMemberValueMismatch {
        kind: LegacyVmMemberKind,
        value_kind: &'static str,
    },

    // --- Engine preamble and services ---------------------------------------
    #[error("legacy short briefing id {id} occurs more than once")]
    DuplicateShortBriefing { id: u32 },
    #[error("saved sound slot {slot} stores slot_index {value}; expected its exact array ordinal")]
    WrongSoundSlotIndex { slot: usize, value: i16 },
    #[error(
        "saved sound slot {slot} registration id {registration_id} differs from source sample id {source_id}"
    )]
    WrongSoundRegistration {
        slot: usize,
        registration_id: u32,
        source_id: u32,
    },
    #[error(
        "saved sound slot {slot} serializes the same active member twice with different values ({first}, {second})"
    )]
    InconsistentSoundActive {
        slot: usize,
        first: bool,
        second: bool,
    },

    // --- Actor sequence ownership -------------------------------------------
    #[error("resolves to sequence element {reference:?} owned by {actual:?}, expected {expected}")]
    WrongSequenceOwner {
        reference: SequenceElementRef,
        actual: Option<EntityId>,
        expected: EntityId,
    },
    #[error("resolves to command {command:?}, expected Wait or Freeze")]
    WrongWaitCommand { command: Command },
    #[error(
        "selected sequence element is {saved:?}, but the converted manager reconstructs {runtime:?}"
    )]
    SelectedElementMismatch {
        saved: Option<SequenceElementRef>,
        runtime: Option<SequenceElementRef>,
    },
    #[error(
        "order pointer resolves to {order_element:?} order index {order_index}, but selected element is {selected:?}"
    )]
    OrderElementMismatch {
        order_element: SequenceElementRef,
        order_index: usize,
        selected: Option<SequenceElementRef>,
    },
    #[error(
        "order pointer resolves to index {order_index}, but selected element's current order is index zero"
    )]
    OrderCursorMismatch { order_index: usize },
    #[error(
        "has selected element {selected:?} but a null order pointer while that element has a current order"
    )]
    MissingOrder { selected: SequenceElementRef },
    #[error("has an order pointer but no selected sequence element")]
    OrderWithoutElement,

    // --- Object leaves ------------------------------------------------------
    #[error("is a mobile master; mobile state belongs to the mobile adoption stage")]
    MobileMasterLeaf,

    // --- Engine tail --------------------------------------------------------
    #[error("resolves to command {command:?} in state {state:?}; expected an active Timer")]
    InvalidTimerElement {
        command: Command,
        state: SequenceState,
    },
    #[error(
        "resolves to command {command:?} in state {state:?}; expected a nonterminal CameraGoto or ZoomLevel"
    )]
    InvalidCameraElement {
        command: Command,
        state: SequenceState,
    },
    #[error("hiking path index {path} is not representable as a runtime PathId")]
    InvalidPathId { path: usize },
    #[error("hiking waypoint index {waypoint} on path {path} exceeds the u8 runtime identity")]
    InvalidWaypointId { path: usize, waypoint: usize },

    // --- Dynamic element construction ---------------------------------------
    #[error(
        "initialized engine contains non-static entity {entity_id}; dynamic save adoption requires a clean mission-start candidate"
    )]
    PreexistingDynamicEntity { entity_id: EntityId },
    #[error("uses static resolution at or beyond boundary {boundary}")]
    InvalidStaticResolution { boundary: u32 },
    #[error("uses dynamic resolution below boundary {boundary}")]
    InvalidDynamicResolution { boundary: u32 },
    #[error("class {saved:?} does not match initialized class {initialized:?}")]
    StaticClassMismatch {
        saved: LegacyElementClass,
        initialized: LegacyElementClass,
    },
    #[error("class {class:?} names factory {factory:?}, but that class maps to {expected:?}")]
    FactoryClassMismatch {
        class: LegacyElementClass,
        factory: LegacyDynamicElementFactory,
        expected: Option<LegacyDynamicElementFactory>,
    },
    #[error("dynamic factory {factory:?} requires missing object sprite master {object_type:?}")]
    MissingObjectSpriteMaster {
        factory: LegacyDynamicElementFactory,
        object_type: ObjectType,
    },
    #[error("references missing character profile {profile_index}")]
    MissingCharacterProfile { profile_index: u32 },
    #[error("requires missing character sprite master for profile {profile_index}")]
    MissingPcSpriteMaster { profile_index: u32 },
    #[error(
        "profile {profile_index} requires pathfinder move-box index {pathfinder_index}, but the loaded grid has only {move_box_count} entries"
    )]
    MissingPcMoveBox {
        profile_index: u32,
        pathfinder_index: u8,
        move_box_count: usize,
    },
    #[error(
        "saved Original creation counter {saved_creation_counter} overflows after {construction_count} dynamic constructions"
    )]
    CreationCounterOverflow {
        saved_creation_counter: u32,
        construction_count: usize,
    },

    // --- Paths --------------------------------------------------------------
    #[error("actor {actor:?} does not match sequence-element owner {owner:?}")]
    PathOwnerMismatch {
        actor: EntityId,
        owner: Option<EntityId>,
    },
    #[error("saved pending path FIFO contains actor {actor:?} more than once")]
    DuplicatePendingActor { actor: EntityId },
    #[error(
        "saved pathfinder state shape at layer {layer:?}: saved {saved}, initialized graph {graph}, runtime {runtime}"
    )]
    PathStateShape {
        layer: Option<usize>,
        saved: usize,
        graph: usize,
        runtime: usize,
    },
    #[error(
        "saved pathfinder has do_not_ignore_next_path=true; the v48 writer always emits false after excluding an ignored head"
    )]
    IgnoredHeadNotRepresentable,

    // --- Fast-find grid -----------------------------------------------------
    #[error("at index {index} does not match initialized topology")]
    TopologyKindMismatch { index: usize },
    #[error("maps to missing initialized {what} {index}")]
    MissingInitialized { what: &'static str, index: usize },
    #[error("is shorter than required index {index}")]
    MissingRuntimeIndex { index: usize },
    #[error("FX points at patch {saved_patch:?}, expected {patch_index}")]
    PatchFxOwnerMismatch {
        patch_index: usize,
        saved_patch: Option<i16>,
    },
    #[error("has invalid changing-obstacle topology: {detail}")]
    InvalidChangingObstacle { detail: String },

    // --- PC / Human ---------------------------------------------------------
    #[error("contains two different playability values: member={member}, interface={interface}")]
    PlayabilityMismatch { member: bool, interface: bool },
    #[error(
        "contains two different portrait-display values: interface={interface}, portrait={portrait}"
    )]
    PortraitDisplayMismatch { interface: bool, portrait: bool },
    #[error("quick-action slot {slot} combines Quickito {quickito:?} with an inline sequence")]
    QuickitoSequenceConflict { slot: usize, quickito: QuickAction },
    #[error(
        "Quickito slot {slot} has invalid interactor/button metadata for {quickito:?}: interactor={interactor:?}, button={button}"
    )]
    InvalidQuickitoMetadata {
        slot: usize,
        quickito: QuickAction,
        interactor: Option<EntityId>,
        button: u16,
    },
    #[error(
        "portrait quantities {actual:?} disagree with status/profile-derived quantities {expected:?}"
    )]
    PortraitQuantityMismatch {
        actual: [u16; 3],
        expected: [u16; 3],
    },
    #[error("portrait two-buttons flag {actual} disagrees with profile-derived value {expected}")]
    PortraitButtonModeMismatch { actual: bool, expected: bool },
    #[error("portrait life bits 0x{actual:08x} disagree with PC-status life bits 0x{expected:08x}")]
    PortraitLifeMismatch { actual: u32, expected: u32 },

    // --- Static elements and local AI ---------------------------------------
    #[error("has invalid bit flags 0x{value:x}")]
    InvalidFlags { value: u16 },
    #[error("has negative enum value {value}")]
    NegativeEnum { value: i32 },
    #[error("writes inconsistent duplicate distance-to-boundary values {first} and {second}")]
    DistanceToBoundaryMismatch { first: f32, second: f32 },
    #[error("has {saved_kind} local AI but its initialized Rust entity has {runtime_kind}")]
    AiKindMismatch {
        saved_kind: &'static str,
        runtime_kind: &'static str,
    },
    #[error("local-AI owner resolves to {actual:?}; expected itself ({expected})")]
    AiOwnerMismatch {
        expected: EntityId,
        actual: Option<EntityId>,
    },
    #[error("stimulus declares info type {declared}, but decoded payload is {actual}")]
    StimulusInfoMismatch { declared: i32, actual: &'static str },
    #[error(
        "hiking path {path} has {count} waypoints, which does not fit Original's u8 path state"
    )]
    TooManyWaypoints { path: u16, count: usize },
    #[error("path {path} references waypoint {waypoint}, but the path has {count} waypoints")]
    MissingWaypoint {
        path: u16,
        waypoint: u8,
        count: usize,
    },
    #[error(
        "macro cursor {offset} with {remaining} remaining bytes exceeds current waypoint macro length {length}"
    )]
    InvalidMacroCursor {
        offset: usize,
        remaining: u16,
        length: usize,
    },
    #[error("has active macro state on a current waypoint whose command is {command}")]
    MacroCommandKind { command: &'static str },
    #[error("has macro progress without a patrol-path-relative cursor")]
    MacroWithoutPatrolPath,
}

/// Fixed error context for one validation scope; builds errors without each
/// stage re-declaring its own `finite`/`invalid` constructors.
#[derive(Clone, Debug)]
pub(crate) struct AdoptSite {
    pub subject: Cow<'static, str>,
    pub creation_order: Option<u32>,
}

impl AdoptSite {
    pub const fn new(subject: &'static str) -> Self {
        Self {
            subject: Cow::Borrowed(subject),
            creation_order: None,
        }
    }

    pub const fn element(subject: &'static str, creation_order: u32) -> Self {
        Self {
            subject: Cow::Borrowed(subject),
            creation_order: Some(creation_order),
        }
    }

    /// A site whose subject is computed, e.g. `"serialized VM owner Global"`.
    pub fn owned(subject: String) -> Self {
        Self {
            subject: Cow::Owned(subject),
            creation_order: None,
        }
    }

    pub fn error(&self, kind: AdoptErrorKind) -> LegacyAdoptError {
        LegacyAdoptError {
            subject: self.subject.clone(),
            creation_order: self.creation_order,
            field: None,
            kind,
        }
    }

    pub fn field_error(
        &self,
        field: impl Into<Cow<'static, str>>,
        kind: AdoptErrorKind,
    ) -> LegacyAdoptError {
        self.error(kind).with_field(field)
    }

    pub fn invalid(
        &self,
        field: impl Into<Cow<'static, str>>,
        value: impl fmt::Display,
        expected: &'static str,
    ) -> LegacyAdoptError {
        self.field_error(
            field,
            AdoptErrorKind::InvalidValue {
                value: value.to_string(),
                expected,
            },
        )
    }

    pub fn finite(
        &self,
        field: impl Into<Cow<'static, str>>,
        value: f32,
    ) -> Result<(), LegacyAdoptError> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(self.field_error(field, AdoptErrorKind::NonFinite { value }))
        }
    }

    pub fn finite_point(
        &self,
        field: impl Into<Cow<'static, str>> + Clone,
        point: LegacyPoint2,
    ) -> Result<(), LegacyAdoptError> {
        self.finite(field.clone(), point.x)?;
        self.finite(field, point.y)
    }

    /// Decode a saved enum word through its `TryFrom<u32>` table.
    pub fn enum_value<T: TryFrom<u32>>(
        &self,
        field: &'static str,
        value: u32,
    ) -> Result<T, LegacyAdoptError> {
        T::try_from(value).map_err(|_| {
            self.field_error(
                field,
                AdoptErrorKind::UnknownEnum {
                    value: i64::from(value),
                },
            )
        })
    }

    pub fn unknown_enum(
        &self,
        field: impl Into<Cow<'static, str>>,
        value: impl Into<i64>,
    ) -> LegacyAdoptError {
        self.field_error(
            field,
            AdoptErrorKind::UnknownEnum {
                value: value.into(),
            },
        )
    }

    pub fn out_of_range(
        &self,
        field: impl Into<Cow<'static, str>>,
        what: &'static str,
        index: usize,
        count: usize,
    ) -> LegacyAdoptError {
        self.field_error(field, AdoptErrorKind::OutOfRange { what, index, count })
    }

    /// Resolve a sparse Original sector slot to its retained Rust identity.
    pub fn checked_sector<T: Copy>(
        &self,
        field: &'static str,
        raw: Option<u16>,
        sectors: &[Option<T>],
    ) -> Result<Option<T>, LegacyAdoptError> {
        let Some(index) = raw else {
            return Ok(None);
        };
        let index = usize::from(index);
        let Some(sector) = sectors.get(index) else {
            return Err(self.out_of_range(field, "sector", index, sectors.len()));
        };
        sector
            .map(Some)
            .ok_or_else(|| self.field_error(field, AdoptErrorKind::UnmappedSector { index }))
    }
}

pub(crate) fn point2(value: LegacyPoint2) -> MapPoint {
    MapPoint::new(value.x, value.y)
}

pub(crate) fn vector2(value: LegacyPoint2) -> MapVec {
    MapVec::new(value.x, value.y)
}

pub(crate) fn point3(value: LegacyPoint3) -> WorldPoint3D {
    WorldPoint3D::new(value.x, value.y, value.z)
}

pub(crate) fn vector3(value: LegacyPoint3) -> WorldVec3D {
    WorldVec3D::new(value.x, value.y, value.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_subject_creation_order_field_and_predicate_in_order() {
        let error = AdoptSite::element("saved NPC", 7)
            .finite("speed", f32::NAN)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "saved NPC creation order 7 field speed contains non-finite value NaN"
        );
        let bare = LegacyAdoptError::from(AdoptErrorKind::TooManyRecentMissions { count: 4 });
        assert_eq!(
            bare.to_string(),
            "Original campaign has 4 recent missions; format permits at most 3"
        );
        assert_eq!(
            bare.context("cannot map saved live campaign").to_string(),
            "cannot map saved live campaign: Original campaign has 4 recent missions; format permits at most 3"
        );
    }
}

//! Resolved player/director commands and recorded actions.
use super::scalar::{TraceEntityId, TracePoint, TracePoint3};
use bitcode_parity as bitcode;
use robin_engine::profiles::Action;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum TraceCommand {
    BoxSelect {
        first: TracePoint,
        second: TracePoint,
        append: bool,
    },
    GroupMove {
        actors: Vec<TraceEntityId>,
        destination: TracePoint,
        running: bool,
        show_marker: bool,
        goal_sector: i16,
        goal_layer: u16,
    },
    LaunchInteraction {
        actor: TraceEntityId,
        target: TraceEntityId,
        /// Original's raw numeric command; `original_command_name` is its
        /// stable name and drives replay. Retained for lossless caching.
        original_command: u32,
        original_command_name: String,
        running: bool,
    },
    LaunchSelfAbility {
        actor: TraceEntityId,
        original_command: u32,
        original_command_name: String,
    },
    LaunchGroundTarget {
        actor: TraceEntityId,
        target: TracePoint3,
        original_command: u32,
        original_command_name: String,
        original_target_field: u32,
        titbit_layer: u16,
    },
    LaunchScrollRead {
        actor: TraceEntityId,
        target: TraceEntityId,
        running: bool,
    },
    SwordStrike {
        actor: TraceEntityId,
        target: TraceEntityId,
        original_command: u32,
        original_command_name: String,
        with_seek: bool,
        #[serde(default = "missing_legacy_seek_distance")]
        seek_distance: f32,
    },
    SelectPc {
        pc: TraceEntityId,
        append: bool,
    },
    UnselectAllPcs,
    StopPc {
        pc: TraceEntityId,
    },
    SelectAction {
        pc: TraceEntityId,
        action: TraceAction,
        /// Original's raw numeric action, retained so the cache round trip is
        /// lossless. `action` is its resolved name; replay uses the name.
        original_action: u32,
    },
    CancelAction {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        /// Always `no_action`: selecting no action is
        /// recorded as cancel_action, but the recorder still emits the pair.
        action: TraceAction,
        original_action: u32,
    },
    OrientActionAt {
        action: TraceAction,
        original_action: u32,
        actor: TraceEntityId,
        mouse_map: TracePoint,
        target: TracePoint3,
    },
    MakePcFast {
        entity: TraceEntityId,
    },
    CrouchDown,
    StandUp,
    // NOTE: with the bitcode-encoded native format, ANY change to this enum
    // (adding, removing, or editing a variant, anywhere) changes the on-disk
    // shape. Bump TRACE_NATIVE_VERSION and migrate existing native traces —
    // converted recordings may no longer have a JSONL source to rebuild from.
    DropAleAt {
        actor: TraceEntityId,
        target: TracePoint,
        running: bool,
    },
    ShieldSelectProtected {
        actor: TraceEntityId,
        protected_pc: TraceEntityId,
    },
    BoxUnselect {
        first: TracePoint,
        second: TracePoint,
        append: bool,
    },
    RaiseShieldWithDanger {
        actor: TraceEntityId,
        protected_pc: TraceEntityId,
        danger_point: TracePoint3,
        danger_point_layer: u16,
    },
    TeleportSelected {
        destination: TracePoint,
        goal_sector: i16,
        goal_layer: u16,
    },
    SelectAllPcs,
    UnselectPc {
        pc: TraceEntityId,
    },
    SelectActionIndex {
        index: u32,
    },
    SetLockAlt {
        on: bool,
    },
    KeyControl,
    KeyReleaseControl,
    StartMacro {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    DeleteMacro {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    StartRecordingMacro {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    ChangeQaMemory {
        slot: u8,
    },
    /// A click the Original refused after it had already barked at the
    /// player. The click itself is a raw mouse message and is never
    /// recorded, so without this the bark's speech resolution arrives with
    /// nothing that caused it.
    HeroRefusedAction {
        actor: TraceEntityId,
        action: TraceAction,
        original_action: u32,
        #[serde(default)]
        target: Option<TraceEntityId>,
        reason: String,
    },
    BeggarDontTalkStamp {
        entity: TraceEntityId,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TraceAction {
    NoAction,
    Bow,
    Hit,
    HitHard,
    Purse,
    Stone,
    Shield,
    BigShield,
    Strangle,
    Lever,
    HelpToClimb,
    Apple,
    Ale,
    Eat,
    Guzzle,
    Listen,
    Heal,
    Net,
    Beggar,
    WaspNest,
    Whistle,
    Climb,
    Jump,
    Search,
    Resuscitate,
    LittleJohnCarry,
    FarmerCarry,
    Tie,
    Lockpick,
    Execute,
    Test,
}

impl From<TraceAction> for Action {
    fn from(value: TraceAction) -> Self {
        match value {
            TraceAction::NoAction => Self::NoAction,
            TraceAction::Bow => Self::Bow,
            TraceAction::Hit => Self::Hit,
            TraceAction::HitHard => Self::HitHard,
            TraceAction::Purse => Self::Purse,
            TraceAction::Stone => Self::Stone,
            TraceAction::Shield => Self::Shield,
            TraceAction::BigShield => Self::BigShield,
            TraceAction::Strangle => Self::Strangle,
            TraceAction::Lever => Self::Lever,
            TraceAction::HelpToClimb => Self::HelpToClimb,
            TraceAction::Apple => Self::Apple,
            TraceAction::Ale => Self::Ale,
            TraceAction::Eat => Self::Eat,
            TraceAction::Guzzle => Self::Guzzle,
            TraceAction::Listen => Self::Listen,
            TraceAction::Heal => Self::Heal,
            TraceAction::Net => Self::Net,
            TraceAction::Beggar => Self::Beggar,
            TraceAction::WaspNest => Self::WaspNest,
            TraceAction::Whistle => Self::Whistle,
            TraceAction::Climb => Self::Climb,
            TraceAction::Jump => Self::Jump,
            TraceAction::Search => Self::Search,
            TraceAction::Resuscitate => Self::Resuscitate,
            TraceAction::LittleJohnCarry => Self::LittleJohnCarry,
            TraceAction::FarmerCarry => Self::FarmerCarry,
            TraceAction::Tie => Self::Tie,
            TraceAction::Lockpick => Self::Lockpick,
            TraceAction::Execute => Self::Execute,
            TraceAction::Test => Self::Test,
        }
    }
}

pub(crate) fn missing_legacy_seek_distance() -> f32 {
    // Schema-16 recordings made before the additive seek-distance diagnostic
    // cannot reconstruct it. A quiet NaN is outside the valid distance domain,
    // survives the unchanged native f32 layout, and is mapped back to `None`
    // before command admission.
    f32::NAN
}

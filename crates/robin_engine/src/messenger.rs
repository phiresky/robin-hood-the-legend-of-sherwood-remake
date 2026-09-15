//! In-game message/event system.
//!
//! Typed messages delivered synchronously by the engine.

use serde::{Deserialize, Serialize};

use crate::entity_id::EntityId;

// ---------------------------------------------------------------------------
// Sub-type enums
// ---------------------------------------------------------------------------

/// Simple (non-entity) messages — keyboard, UI, display, mission flow.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u16)]
pub enum SimpleMessage {
    None = 0,

    // Display / scrolling
    ScrollDown,
    ScrollUp,
    ScrollRight,
    ScrollLeft,

    ZoomUp,
    ZoomDown,

    ZoomUpStart,
    ZoomDownStart,
    ZoomUpEnd,
    ZoomDownEnd,

    // Load / Save
    QuickSave,
    QuickLoad,

    // Modifier keys
    KeyShift,
    KeyAlt,
    KeyControl,

    KeyReleaseShift,
    KeyReleaseAlt,
    KeyReleaseControl,

    DisplayInfo,
    DisplayIaInfo,
    SlowMotion,

    Pause,
    RecordMovie,

    UiHasFocus,
    ReloadWeapon,

    LockAlt,
    UnlockAlt,

    LockUser,
    UnlockUser,

    Stature,
    StatureChangeEnd,

    PrintScreen,
    DisplayConsole,
    HideConsole,
    DisplayMenu,

    SwitchMaskedDisplay,

    SwitchTask,

    StartMission,
    QuitMission,
    InterruptMission,
    DisplayCampaignMap,

    MarkAction,
    ResetInput,
}

/// Mouse-related messages.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u16)]
pub enum MouseMessage {
    None = 0,
    Moved,
    Button,
}

/// Player-character / action messages.
///
/// The discriminants are sequential and match the on-disk `MSG_PC_*`
/// ordering so scripts can pass them through as raw integers and we
/// recover the variant with `TryFrom<u16>`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    num_enum::IntoPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u16)]
pub enum PcMessage {
    None = 0,

    // Macros
    StartMacro,
    FizzleMacro,
    DeleteMacro,
    DeleteAllMacroFor,

    StartRecordingMacro,
    StopRecordingMacro,
    UpdateRecordingMacro,
    DoTetrisOnMacro,
    ChangeQaMemory,
    QaFocus,

    // Character selection
    EnableCharacter,
    DisableCharacter,

    SelectCharacter,
    SelectCharacterWithEcho,
    SelectAddCharacter,
    SelectAddCharacterWithEcho,
    ReselectCharacter,
    UnselectCharacter,

    CenterOn,
    CharacterKilled,

    // Actions
    SelectActionIndex,
    SelectAction,
    UnselectAction,
    FocusAction,
    SelectActionSimple,

    DisableAction,
    EnableAction,
    DisableActionIndex,
    EnableActionIndex,
    DisableAllActions,
    EnableAllActions,
    DisableAllActionsTemp,
    EnableAllActionsTemp,
    DisableAllButOneActions,

    // Ammo
    DropSingleAmmo,
    DropSeveralAmmo,

    // Movement
    StandUp,
    Teleport,

    // Reinforcement
    SendReinforcement,
    ReinforcementArrived,

    // Popup
    ShowPcInformation,
    HidePcInformation,
}

// ---------------------------------------------------------------------------
// Top-level message type
// ---------------------------------------------------------------------------

/// Discriminated message type combining the top-level tag with its
/// sub-type enum.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MessageType {
    Simple(SimpleMessage),
    Mouse(MouseMessage),
    /// PC-targeted message.  `Some(id)` targets one specific PC; `None`
    /// is the fan-out / "no specific PC" signal that handlers branch on.
    Pc(PcMessage, Option<EntityId>),
    LoadSave,
}

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

/// A single synchronous message.
///
/// `value` is the generic parameter; `arg1`/`arg2` carry additional
/// context supplied by the sender.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Message {
    pub msg_type: MessageType,
    /// Generic parameter.
    pub value: u32,
    pub arg1: u32,
    pub arg2: u32,
}

impl Message {
    pub fn new(msg_type: MessageType) -> Self {
        Self {
            msg_type,
            value: 0,
            arg1: 0,
            arg2: 0,
        }
    }

    pub fn with_value(msg_type: MessageType, value: u32) -> Self {
        Self {
            msg_type,
            value,
            arg1: 0,
            arg2: 0,
        }
    }

    /// Build a `MSG_PC` message.  `pc = None` is the no-target signal
    /// that handlers fan out / no-op on.
    pub fn pc(sub: PcMessage, pc: Option<EntityId>) -> Self {
        Self {
            msg_type: MessageType::Pc(sub, pc),
            value: 0,
            arg1: 0,
            arg2: 0,
        }
    }

    /// Build a `MSG_PC` message with a generic value.
    pub fn pc_with_value(sub: PcMessage, pc: Option<EntityId>, value: u32) -> Self {
        Self {
            msg_type: MessageType::Pc(sub, pc),
            value,
            arg1: 0,
            arg2: 0,
        }
    }
}

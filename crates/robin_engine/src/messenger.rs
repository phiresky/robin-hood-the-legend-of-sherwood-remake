//! In-game message/event system.
//!
//! This module provides the data model (message types, message struct) and a
//! simple queue-based [`Messenger`] that replaces the original pub/sub
//! singleton.

use std::collections::VecDeque;

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

/// A single message in the queue.
///
/// `value` is the generic parameter; `arg1`/`arg2` carry additional
/// context passed via `send_with_args`.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
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

// ---------------------------------------------------------------------------
// Messenger
// ---------------------------------------------------------------------------

/// Queue-based message dispatcher.
///
/// A simple FIFO queue that consumers poll, replacing the original
/// pub/sub singleton's forwarding model.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct Messenger {
    queue: VecDeque<Message>,
}

impl Default for Messenger {
    fn default() -> Self {
        Self::new()
    }
}

impl Messenger {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    /// Push a message onto the back of the queue.
    pub fn send(&mut self, msg: Message) {
        self.queue.push_back(msg);
    }

    /// Build and enqueue a message with extra arguments.
    pub fn send_with_args(&mut self, msg_type: MessageType, arg1: u32, arg2: u32) {
        self.queue.push_back(Message {
            msg_type,
            value: 0,
            arg1,
            arg2,
        });
    }

    /// Pop the next message from the front of the queue.
    pub fn poll(&mut self) -> Option<Message> {
        self.queue.pop_front()
    }

    /// Drain all pending messages, returning them as a Vec.
    pub fn drain(&mut self) -> Vec<Message> {
        self.queue.drain(..).collect()
    }

    /// Discard all pending messages.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Number of messages currently queued.
    pub fn count(&self) -> usize {
        self.queue.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_poll_fifo_order() {
        let mut m = Messenger::new();

        let msg1 = Message::new(MessageType::Simple(SimpleMessage::Pause));
        let msg2 = Message::pc(
            PcMessage::SelectCharacter,
            Some(EntityId::Pc(crate::entity_id::PcId(42))),
        );

        m.send(msg1.clone());
        m.send(msg2.clone());

        assert_eq!(m.count(), 2);
        assert_eq!(m.poll(), Some(msg1));
        assert_eq!(m.poll(), Some(msg2));
        assert_eq!(m.poll(), None);
        assert_eq!(m.count(), 0);
    }

    #[test]
    fn send_with_args() {
        let mut m = Messenger::new();
        m.send_with_args(MessageType::Mouse(MouseMessage::Button), 10, 20);

        let msg = m.poll().unwrap();
        assert_eq!(msg.msg_type, MessageType::Mouse(MouseMessage::Button));
        assert_eq!(msg.arg1, 10);
        assert_eq!(msg.arg2, 20);
    }

    #[test]
    fn clear_removes_all() {
        let mut m = Messenger::new();
        m.send(Message::new(MessageType::Simple(SimpleMessage::ScrollUp)));
        m.send(Message::new(MessageType::Simple(SimpleMessage::ScrollDown)));
        assert_eq!(m.count(), 2);

        m.clear();
        assert_eq!(m.count(), 0);
        assert_eq!(m.poll(), None);
    }

    #[test]
    fn serde_roundtrip() {
        let msg = Message {
            msg_type: MessageType::Pc(
                PcMessage::StartMacro,
                Some(EntityId::Pc(crate::entity_id::PcId(7))),
            ),
            value: 99,
            arg1: 1,
            arg2: 2,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }
}

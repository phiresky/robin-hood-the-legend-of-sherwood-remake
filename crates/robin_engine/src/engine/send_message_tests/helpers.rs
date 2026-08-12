//! Focused parity tests for script `SendMessage`.
//!
//! Original provenance:
//! - `RHScript.cpp::SendMessage[WithArguments]` builds an
//!   `RHCOMMAND_SEND_MESSAGE` element and calls `LaunchSequenceElement`.
//! - `RHSequenceElement::ExecutedImmediately` calls the owner's
//!   `ExecuteImmediately` directly (or the engine for a null owner), so the
//!   element does not enter normal actor `Instruct` priority contention.
//! - `RHelementactor.cpp` / `RHengine.cpp` invoke `ProcessMessage` and only
//!   then set the element to `RHSEQ_TERMINATED` in the same frame.

use crate::element::{
    ActorData, ActorSoldier, AiBrain, ElementData, ElementKind, Entity, HumanData, NpcData,
    Posture, SoldierData,
};
use crate::engine::EngineInner;
use crate::engine::types::{LevelAssets, MissionScript};
use crate::natives::{NativeFn, ScriptHandleCodec};
use crate::scb::{ClassEntry, Function, ScbFile};
use crate::sequence::{Field, FieldValue, SequenceElement};
use crate::vm::{Opcode, Quad};

pub(super) const TMP0: u16 = 0xC000;
pub(super) const TMP1: u16 = 0xC004;
pub(super) const TMP2: u16 = 0xC008;
pub(super) const TMP3: u16 = 0xC00C;
pub(super) const HEAP0: u16 = 0x4000;

pub(super) fn quad(operation: Opcode) -> Quad {
    Quad {
        operation: operation as u8,
        operands: [0; 8],
    }
}

pub(super) fn begin_function(temp_count: u16) -> Quad {
    let mut quad = quad(Opcode::BeginFunction);
    quad.operands[2..4].copy_from_slice(&temp_count.to_le_bytes());
    quad
}

pub(super) fn get_param(dst: u16, offset: i32) -> Quad {
    let mut quad = quad(Opcode::Aff1GetParam);
    quad.operands[0..2].copy_from_slice(&dst.to_le_bytes());
    quad.operands[4..8].copy_from_slice(&offset.to_le_bytes());
    quad
}

pub(super) fn integer_constant(dst: u16, value: i32) -> Quad {
    let mut quad = quad(Opcode::Aff0IConstant);
    quad.operands[0..2].copy_from_slice(&dst.to_le_bytes());
    quad.operands[4..8].copy_from_slice(&value.to_le_bytes());
    quad
}

pub(super) fn native_param(sym: u16) -> Quad {
    let mut quad = quad(Opcode::NativeParam);
    quad.operands[0..2].copy_from_slice(&sym.to_le_bytes());
    quad
}

pub(super) fn native_call(native: NativeFn) -> Quad {
    let mut quad = quad(Opcode::NativeCall);
    quad.operands[0..4].copy_from_slice(&(native as u32).to_le_bytes());
    quad
}

pub(super) fn native_return(dst: u16) -> Quad {
    let mut quad = quad(Opcode::Aff1NativeGetReturn);
    quad.operands[0..2].copy_from_slice(&dst.to_le_bytes());
    quad
}

pub(super) fn return_value(sym: u16) -> Quad {
    let mut quad = quad(Opcode::ReturnVal);
    quad.operands[0..2].copy_from_slice(&sym.to_le_bytes());
    quad
}

fn freeze_toggling_scroll_class(class_name: &str, frozen: bool) -> ClassEntry {
    ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: class_name.into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "Hourglass".into(),
            address: 0,
            num_parameters: 1,
            size_of_return_value: 0,
            size_of_parameters: 4,
            size_of_volatile: 0,
            size_of_temporary: 4,
        }],
        quads: vec![
            begin_function(1),
            integer_constant(TMP0, i32::from(frozen)),
            native_param(TMP0),
            native_call(NativeFn::FreezeAll),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    }
}

/// `ProcessMessage(message, _, _)` stores the received message in global 900.
/// Sending 41 then 72 must therefore leave 72, pinning callback launch order.
pub(super) fn message_script() -> MissionScript {
    let process_message = Function {
        name: "ProcessMessage".into(),
        address: 0,
        num_parameters: 3,
        size_of_return_value: 0,
        size_of_parameters: 12,
        size_of_volatile: 0,
        size_of_temporary: 8,
    };
    let trigger_self = Function {
        name: "TriggerSelf".into(),
        address: 8,
        num_parameters: 0,
        size_of_return_value: 0,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 12,
    };
    let trigger_honolulu = Function {
        name: "TriggerHonolulu".into(),
        address: 27,
        num_parameters: 0,
        size_of_return_value: 0,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 8,
    };
    let trigger_life = Function {
        name: "TriggerLife".into(),
        address: 38,
        num_parameters: 0,
        size_of_return_value: 4,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 12,
    };
    let trigger_concussion = Function {
        name: "TriggerConcussion".into(),
        address: 53,
        num_parameters: 0,
        size_of_return_value: 4,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 12,
    };
    let trigger_relay = Function {
        name: "TriggerRelay".into(),
        address: 68,
        num_parameters: 1,
        size_of_return_value: 0,
        size_of_parameters: 4,
        size_of_volatile: 0,
        size_of_temporary: 16,
    };
    let trigger_posture = Function {
        name: "TriggerPosture".into(),
        address: 91,
        num_parameters: 0,
        size_of_return_value: 4,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 8,
    };
    let trigger_timer = Function {
        name: "TriggerTimer".into(),
        address: 103,
        num_parameters: 0,
        size_of_return_value: 0,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 4,
    };
    let trigger_action_state = Function {
        name: "TriggerActionState".into(),
        address: 111,
        num_parameters: 0,
        size_of_return_value: 4,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 8,
    };
    let receiver = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "MessageReceiver".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![
            process_message,
            trigger_self,
            trigger_honolulu,
            trigger_life,
            trigger_concussion,
            trigger_relay,
            trigger_posture,
            trigger_timer,
            trigger_action_state,
        ],
        quads: vec![
            begin_function(2),
            integer_constant(TMP0, 900),
            get_param(TMP1, 0),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            // TriggerSelf: write 901=1, send to the same actor, then write
            // 901=2. The nested A→A activation must not overwrite this
            // function's instruction pointer or temporary stack.
            begin_function(3),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            integer_constant(TMP1, 901),
            integer_constant(TMP2, 1),
            native_param(TMP1),
            native_param(TMP2),
            native_call(NativeFn::SetGlobal),
            integer_constant(TMP1, 314),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            integer_constant(TMP2, 2),
            // Reload key after TMP1 became the message code.
            integer_constant(TMP1, 901),
            native_param(TMP1),
            native_param(TMP2),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            // TriggerHonolulu: SetActorLocation(NULL) yields through the
            // engine's full Honolulu pipeline, then UnlockAI must observe the
            // lock before this same callback returns.
            begin_function(2),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            integer_constant(TMP1, 0),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SetActorLocation),
            native_param(TMP0),
            native_call(NativeFn::UnlockAI),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            // TriggerLife: SetPersistentProperty(2) must be applied by the
            // engine yield before GetPersistentProperty executes.
            begin_function(3),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            integer_constant(TMP1, 2),
            integer_constant(TMP2, 37),
            native_param(TMP0),
            native_param(TMP1),
            native_param(TMP2),
            native_call(NativeFn::SetPersistentProperty),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::GetPersistentProperty),
            native_return(TMP2),
            return_value(TMP2),
            quad(Opcode::EndFunction),
            // TriggerConcussion is the same barrier for property 3.
            begin_function(3),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            integer_constant(TMP1, 3),
            integer_constant(TMP2, 123),
            native_param(TMP0),
            native_param(TMP1),
            native_param(TMP2),
            native_call(NativeFn::SetPersistentProperty),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::GetPersistentProperty),
            native_return(TMP2),
            return_value(TMP2),
            quad(Opcode::EndFunction),
            // TriggerRelay(B): A writes before, synchronously enters B, B
            // sends back to A, then the original A activation writes after.
            begin_function(4),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            get_param(TMP1, 0),
            integer_constant(TMP2, 903),
            integer_constant(TMP3, 1),
            native_param(TMP2),
            native_param(TMP3),
            native_call(NativeFn::SetGlobal),
            integer_constant(TMP2, 10),
            integer_constant(TMP3, 0),
            native_param(TMP1),
            native_param(TMP2),
            native_param(TMP0),
            native_param(TMP3),
            native_call(NativeFn::SendMessageWithArguments),
            integer_constant(TMP2, 903),
            integer_constant(TMP3, 3),
            native_param(TMP2),
            native_param(TMP3),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            // Posture must route its generated WAIT through the canonical
            // instruction/arbitration path before GetActorPosture resumes.
            begin_function(2),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            integer_constant(TMP1, 2),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SetActorPosture),
            native_param(TMP0),
            native_call(NativeFn::GetActorPosture),
            native_return(TMP1),
            return_value(TMP1),
            quad(Opcode::EndFunction),
            // Timer is in the ExecutedImmediately group and must register
            // with the engine timer owner before Thanx returns.
            begin_function(1),
            native_call(NativeFn::Start),
            integer_constant(TMP0, 12),
            native_param(TMP0),
            native_call(NativeFn::RecordTimer),
            native_call(NativeFn::Thanx),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            begin_function(2),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            integer_constant(TMP1, 1),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SetActorActionState),
            native_param(TMP0),
            native_call(NativeFn::GetActorActionState),
            native_return(TMP1),
            return_value(TMP1),
            quad(Opcode::EndFunction),
        ],
    };
    let relay_process_message = Function {
        name: "ProcessMessage".into(),
        address: 0,
        num_parameters: 3,
        size_of_return_value: 0,
        size_of_parameters: 12,
        size_of_volatile: 0,
        size_of_temporary: 8,
    };
    let relay = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "RelayReceiver".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![relay_process_message],
        quads: vec![
            begin_function(2),
            get_param(TMP0, 4),
            integer_constant(TMP1, 20),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let ordering_process_message = Function {
        name: "ProcessMessage".into(),
        address: 0,
        num_parameters: 3,
        size_of_return_value: 0,
        size_of_parameters: 12,
        size_of_volatile: 0,
        size_of_temporary: 12,
    };
    let ordering_trigger = Function {
        name: "TriggerParentOrder".into(),
        address: 23,
        num_parameters: 0,
        size_of_return_value: 0,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 8,
    };
    let ordering_next_level = Function {
        name: "TriggerNextLevel".into(),
        address: 36,
        num_parameters: 0,
        size_of_return_value: 0,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 12,
    };
    let ordering = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "OrderingReceiver".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![
            ordering_process_message,
            ordering_trigger,
            ordering_next_level,
        ],
        quads: vec![
            // The nested callback observes whether the parent's later Unblip
            // has already run, then launches its own immediate LockAI.
            begin_function(3),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            native_param(TMP0),
            native_call(NativeFn::IsUnblipped),
            native_return(TMP1),
            integer_constant(TMP2, 904),
            native_param(TMP2),
            native_param(TMP1),
            native_call(NativeFn::SetGlobal),
            native_call(NativeFn::Start),
            native_param(TMP0),
            native_call(NativeFn::RecordLockAI),
            native_call(NativeFn::Thanx),
            // The child Thanx must finish LockAI and resume here while the
            // parent's Unblip tail is still detached. This observation
            // distinguishes true A→child C→parent B depth-first execution
            // from the old shared FIFO's A→B→C order.
            native_param(TMP0),
            native_call(NativeFn::IsUnblipped),
            native_return(TMP1),
            integer_constant(TMP2, 907),
            native_param(TMP2),
            native_param(TMP1),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            // Parent recording: SendMessage followed by Unblip.
            begin_function(2),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            native_call(NativeFn::Start),
            integer_constant(TMP1, 77),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::RecordSendMessage),
            native_param(TMP0),
            native_call(NativeFn::RecordUnBlip),
            native_call(NativeFn::Thanx),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            // Parent recording: actor SendMessage at level 1, then Unblip at
            // level 2. Thanx must not resume until SendMessage's SetState has
            // closed its owner card and Ready() has run the successor.
            begin_function(3),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            native_call(NativeFn::Start),
            integer_constant(TMP1, 78),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::RecordSendMessage),
            native_call(NativeFn::Then),
            native_param(TMP0),
            native_call(NativeFn::RecordUnBlip),
            native_call(NativeFn::Thanx),
            native_param(TMP0),
            native_call(NativeFn::IsUnblipped),
            native_return(TMP1),
            integer_constant(TMP2, 908),
            native_param(TMP2),
            native_param(TMP1),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let target_ordering = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "TargetOrdering".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "ActivatedByArrow".into(),
            address: 0,
            num_parameters: 1,
            size_of_return_value: 0,
            size_of_parameters: 4,
            size_of_volatile: 0,
            size_of_temporary: 4,
        }],
        quads: vec![
            begin_function(1),
            integer_constant(TMP0, 1),
            native_param(TMP0),
            native_call(NativeFn::FreezeAll),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let move_ordering = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "MoveOrdering".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "ProcessMessage".into(),
            address: 0,
            num_parameters: 3,
            size_of_return_value: 0,
            size_of_parameters: 12,
            size_of_volatile: 0,
            size_of_temporary: 12,
        }],
        quads: vec![
            begin_function(3),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            native_param(TMP0),
            native_call(NativeFn::GetCurrentAction),
            native_return(TMP1),
            integer_constant(TMP2, 909),
            native_param(TMP2),
            native_param(TMP1),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let scroll_observer = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "ScrollObserver".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "ProcessMessage".into(),
            address: 0,
            num_parameters: 3,
            size_of_return_value: 0,
            size_of_parameters: 12,
            size_of_volatile: 0,
            size_of_temporary: 8,
        }],
        quads: vec![
            begin_function(2),
            native_call(NativeFn::ThisScroll),
            native_return(TMP0),
            integer_constant(TMP1, 905),
            native_param(TMP1),
            native_param(TMP0),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let self_deactivating_scroll = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "SelfDeactivatingScroll".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "Hourglass".into(),
            address: 0,
            num_parameters: 1,
            size_of_return_value: 0,
            size_of_parameters: 4,
            size_of_volatile: 0,
            size_of_temporary: 4,
        }],
        quads: vec![
            begin_function(1),
            native_call(NativeFn::ThisScroll),
            native_return(TMP0),
            native_param(TMP0),
            native_call(NativeFn::Deactivate),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let freeze_on_scroll = freeze_toggling_scroll_class("FreezeOnScroll", true);
    let freeze_off_scroll = freeze_toggling_scroll_class("FreezeOffScroll", false);
    let scroll_relay = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "ScrollRelay".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![
            Function {
                name: "TriggerScroll".into(),
                address: 0,
                num_parameters: 1,
                size_of_return_value: 0,
                size_of_parameters: 4,
                size_of_volatile: 0,
                size_of_temporary: 12,
            },
            Function {
                name: "TriggerOwnerless".into(),
                address: 14,
                num_parameters: 0,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 12,
            },
        ],
        quads: vec![
            begin_function(3),
            get_param(TMP0, 0),
            integer_constant(TMP1, 55),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            native_call(NativeFn::ThisScroll),
            native_return(TMP0),
            integer_constant(TMP2, 906),
            native_param(TMP2),
            native_param(TMP0),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            begin_function(3),
            integer_constant(TMP0, 0),
            integer_constant(TMP1, 66),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            native_call(NativeFn::ThisScroll),
            native_return(TMP0),
            integer_constant(TMP2, 909),
            native_param(TMP2),
            native_param(TMP0),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let global_process_message = Function {
        name: "ProcessMessage".into(),
        address: 0,
        num_parameters: 3,
        size_of_return_value: 0,
        size_of_parameters: 12,
        size_of_volatile: 0,
        size_of_temporary: 8,
    };
    let global_hourglass = Function {
        name: "Hourglass".into(),
        address: 14,
        num_parameters: 0,
        size_of_return_value: 0,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 8,
    };
    let startup = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![global_process_message, global_hourglass],
        quads: vec![
            begin_function(2),
            integer_constant(TMP0, 902),
            get_param(TMP1, 0),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SetGlobal),
            native_call(NativeFn::ThisScroll),
            native_return(TMP1),
            integer_constant(TMP0, 908),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            begin_function(2),
            integer_constant(TMP0, 0),
            integer_constant(TMP1, 4240),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let recursive = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "RecursiveReceiver".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "ProcessMessage".into(),
            address: 0,
            num_parameters: 3,
            size_of_return_value: 0,
            size_of_parameters: 12,
            size_of_volatile: 0,
            size_of_temporary: 8,
        }],
        quads: vec![
            begin_function(2),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            integer_constant(TMP1, 1),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let heap_a = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "HeapA".into(),
        size_of_member_variables: 4,
        member_variables: Vec::new(),
        functions: vec![
            Function {
                name: "ProcessMessage".into(),
                address: 0,
                num_parameters: 3,
                size_of_return_value: 0,
                size_of_parameters: 12,
                size_of_volatile: 0,
                size_of_temporary: 0,
            },
            Function {
                name: "TriggerSelf".into(),
                address: 4,
                num_parameters: 0,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 12,
            },
            Function {
                name: "TriggerRelay".into(),
                address: 19,
                num_parameters: 1,
                size_of_return_value: 0,
                size_of_parameters: 4,
                size_of_volatile: 0,
                size_of_temporary: 16,
            },
        ],
        quads: vec![
            begin_function(0),
            integer_constant(HEAP0, 20),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            // A→A: member 1 before, nested ProcessMessage writes 20, outer
            // reads that member into global 905, then writes member 3.
            begin_function(3),
            integer_constant(HEAP0, 1),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            integer_constant(TMP1, 1),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            integer_constant(TMP2, 905),
            native_param(TMP2),
            native_param(HEAP0),
            native_call(NativeFn::SetGlobal),
            integer_constant(HEAP0, 3),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            // A→B→A: B gets A as arg1 and sends back. A's nested callback
            // writes member 20; the suspended outer activation observes it.
            begin_function(4),
            integer_constant(HEAP0, 10),
            get_param(TMP1, 0),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            integer_constant(TMP2, 10),
            integer_constant(TMP3, 0),
            native_param(TMP1),
            native_param(TMP2),
            native_param(TMP0),
            native_param(TMP3),
            native_call(NativeFn::SendMessageWithArguments),
            integer_constant(TMP2, 906),
            native_param(TMP2),
            native_param(HEAP0),
            native_call(NativeFn::SetGlobal),
            integer_constant(HEAP0, 30),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let heap_b = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "HeapB".into(),
        size_of_member_variables: 4,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "ProcessMessage".into(),
            address: 0,
            num_parameters: 3,
            size_of_return_value: 0,
            size_of_parameters: 12,
            size_of_volatile: 0,
            size_of_temporary: 8,
        }],
        quads: vec![
            begin_function(2),
            integer_constant(HEAP0, 11),
            get_param(TMP0, 4),
            integer_constant(TMP1, 99),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            integer_constant(HEAP0, 12),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let failure_receiver = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "FailureReceiver".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![
            Function {
                name: "ProcessMessage".into(),
                address: 0,
                num_parameters: 3,
                size_of_return_value: 0,
                size_of_parameters: 12,
                size_of_volatile: 0,
                size_of_temporary: 8,
            },
            Function {
                name: "TriggerFailure".into(),
                address: 8,
                num_parameters: 1,
                size_of_return_value: 0,
                size_of_parameters: 4,
                size_of_volatile: 0,
                size_of_temporary: 16,
            },
        ],
        quads: vec![
            // Parent ProcessMessage launches the actual failing child: a
            // valid actor handle with no required bound script VM.
            begin_function(2),
            get_param(TMP0, 4),
            integer_constant(TMP1, 66),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            // Parent recording owns SendMessage(self, missing, 0), followed
            // by Unblip. Its tail must remain detached through child failure.
            begin_function(4),
            native_call(NativeFn::ThisActor),
            native_return(TMP0),
            get_param(TMP1, 0),
            native_call(NativeFn::Start),
            integer_constant(TMP2, 77),
            integer_constant(TMP3, 0),
            native_param(TMP0),
            native_param(TMP2),
            native_param(TMP1),
            native_param(TMP3),
            native_call(NativeFn::RecordSendMessageWithArguments),
            native_param(TMP0),
            native_call(NativeFn::RecordUnBlip),
            native_call(NativeFn::Thanx),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let open_scroll_failure = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "OpenScrollFailure".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "IsTaken".into(),
            address: 0,
            num_parameters: 1,
            size_of_return_value: 4,
            size_of_parameters: 4,
            size_of_volatile: 0,
            size_of_temporary: 8,
        }],
        quads: vec![
            // The ScrollReader parameter is a valid actor with no bound VM.
            // Its nested SendMessage is therefore the actual failing child.
            begin_function(2),
            get_param(TMP0, 0),
            integer_constant(TMP1, 66),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            integer_constant(TMP0, 1),
            return_value(TMP0),
            quad(Opcode::EndFunction),
        ],
    };
    let open_scroll_local_failure = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "OpenScrollLocalFailure".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "IsTaken".into(),
            address: 0,
            num_parameters: 1,
            size_of_return_value: 4,
            size_of_parameters: 4,
            size_of_volatile: 0,
            size_of_temporary: 0,
        }],
        // The declared function has no instruction at address zero, making
        // this a direct local RanOff failure rather than a descendant action.
        quads: Vec::new(),
    };
    let yielding_flavor = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "YieldingFlavor".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![
            ("Initialize", 0),
            ("EnterZone", 1),
            ("ActivatedByArrow", 1),
            ("IsTaken", 1),
            ("ReachPoint", 1),
            ("EmitEffect", 0),
        ]
        .into_iter()
        .map(|(name, num_parameters)| Function {
            name: name.into(),
            address: if name == "EmitEffect" { 8 } else { 0 },
            num_parameters,
            size_of_return_value: 0,
            size_of_parameters: i32::from(num_parameters) * 4,
            size_of_volatile: 0,
            size_of_temporary: 8,
        })
        .collect(),
        quads: vec![
            begin_function(2),
            integer_constant(TMP0, 0),
            integer_constant(TMP1, 4241),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SendMessage),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
            begin_function(1),
            integer_constant(TMP0, 1),
            native_param(TMP0),
            native_call(NativeFn::DisplayMap),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    MissionScript::from_scb(ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![
            startup,
            receiver,
            relay,
            ordering,
            target_ordering,
            move_ordering,
            scroll_observer,
            self_deactivating_scroll,
            freeze_on_scroll,
            freeze_off_scroll,
            scroll_relay,
            recursive,
            heap_a,
            heap_b,
            failure_receiver,
            open_scroll_failure,
            open_scroll_local_failure,
            yielding_flavor,
        ],
    })
    .expect("message test script should decode")
}

pub(super) fn scripted_receiver() -> Entity {
    scripted_soldier("MessageReceiver")
}

pub(super) fn scripted_soldier(script_class: &str) -> Entity {
    Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        },
        actor: ActorData {
            script_class: script_class.into(),
            ..ActorData::default()
        },
        human: HumanData::default(),
        npc: NpcData {
            life_points: 50,
            ai_brain: AiBrain::Enemy(Box::default()),
            ..NpcData::default()
        },
        soldier: SoldierData::default(),
    })
}

pub(super) fn bind_script_actor(
    engine: &mut EngineInner,
    actor_id: crate::element::EntityId,
    class_name: &str,
) -> i32 {
    let handle = ScriptHandleCodec::actor_handle(actor_id);
    let simulation = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &simulation,
        &mut engine.world.entities,
        &mut engine.ai.global,
        std::sync::Arc::make_mut(&mut engine.world.fast_grid),
    );
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("script installed")
            .bind_actor(
                handle,
                class_name,
                &mut engine.script_domains,
                &capabilities,
            )
    );
    handle
}

pub(super) fn script_instance_heap_word(script: &MissionScript, handle: i32) -> i32 {
    let bytes: [u8; 4] = script
        .actor_instances
        .get(&handle)
        .expect("bound actor instance")
        .vm
        .heap[0..4]
        .try_into()
        .expect("four-byte member heap");
    i32::from_le_bytes(bytes)
}

pub(super) fn engine_with_receiver() -> (EngineInner, crate::element::EntityId, i32) {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission = Some(message_script());
    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .state
        .globals
        .extend([
            (900, 0),
            (901, 0),
            (902, 0),
            (903, 0),
            (904, -1),
            (905, 0),
            (906, 0),
            (907, -1),
            (908, 0),
            (909, 0),
        ]);
    engine.attach_script_bindings(&LevelAssets::new());
    let receiver = engine.add_entity(scripted_receiver());
    let handle = ScriptHandleCodec::actor_handle(receiver);
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut engine.world.entities,
        &mut engine.ai.global,
        std::sync::Arc::make_mut(&mut engine.world.fast_grid),
    );
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("script installed")
            .bind_actor(
                handle,
                "MessageReceiver",
                &mut engine.script_domains,
                &capabilities,
            )
    );
    (engine, receiver, handle)
}

pub(super) fn integer_property(element: &SequenceElement, field: Field) -> u32 {
    match element.get_property(field) {
        Some(FieldValue::Integer(value)) => *value,
        other => panic!("expected integer {field:?} property, got {other:?}"),
    }
}

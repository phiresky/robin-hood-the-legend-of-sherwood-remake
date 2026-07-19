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
    ActorData, ActorSoldier, AiBrain, Command, ElementData, ElementKind, Entity, HumanData,
    NpcData, Posture, SoldierData,
};
use crate::engine::EngineInner;
use crate::engine::types::{LevelAssets, MissionScript};
use crate::natives::{NativeFn, ScriptHandleCodec};
use crate::order::OrderType;
use crate::scb::{ClassEntry, Function, ScbFile};
use crate::sequence::{Field, FieldValue, SequenceAction, SequenceElement, SequenceState};
use crate::vm::{Opcode, Quad};

const TMP0: u16 = 0xC000;
const TMP1: u16 = 0xC004;
const TMP2: u16 = 0xC008;
const TMP3: u16 = 0xC00C;
const HEAP0: u16 = 0x4000;

fn quad(operation: Opcode) -> Quad {
    Quad {
        operation: operation as u8,
        operands: [0; 8],
    }
}

fn begin_function(temp_count: u16) -> Quad {
    let mut quad = quad(Opcode::BeginFunction);
    quad.operands[2..4].copy_from_slice(&temp_count.to_le_bytes());
    quad
}

fn get_param(dst: u16, offset: i32) -> Quad {
    let mut quad = quad(Opcode::Aff1GetParam);
    quad.operands[0..2].copy_from_slice(&dst.to_le_bytes());
    quad.operands[4..8].copy_from_slice(&offset.to_le_bytes());
    quad
}

fn integer_constant(dst: u16, value: i32) -> Quad {
    let mut quad = quad(Opcode::Aff0IConstant);
    quad.operands[0..2].copy_from_slice(&dst.to_le_bytes());
    quad.operands[4..8].copy_from_slice(&value.to_le_bytes());
    quad
}

fn native_param(sym: u16) -> Quad {
    let mut quad = quad(Opcode::NativeParam);
    quad.operands[0..2].copy_from_slice(&sym.to_le_bytes());
    quad
}

fn native_call(native: NativeFn) -> Quad {
    let mut quad = quad(Opcode::NativeCall);
    quad.operands[0..4].copy_from_slice(&(native as u32).to_le_bytes());
    quad
}

fn native_return(dst: u16) -> Quad {
    let mut quad = quad(Opcode::Aff1NativeGetReturn);
    quad.operands[0..2].copy_from_slice(&dst.to_le_bytes());
    quad
}

fn return_value(sym: u16) -> Quad {
    let mut quad = quad(Opcode::ReturnVal);
    quad.operands[0..2].copy_from_slice(&sym.to_le_bytes());
    quad
}

/// `ProcessMessage(message, _, _)` stores the received message in global 900.
/// Sending 41 then 72 must therefore leave 72, pinning callback launch order.
fn message_script() -> MissionScript {
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
    let ordering = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "OrderingReceiver".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![ordering_process_message, ordering_trigger],
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
            scroll_observer,
            scroll_relay,
            recursive,
            heap_a,
            heap_b,
            failure_receiver,
            open_scroll_failure,
            yielding_flavor,
        ],
    })
    .expect("message test script should decode")
}

#[test]
fn ownerless_send_message_routes_to_global_process_message() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    engine
        .call_external_native(&assets, "SendMessage", &[0, 2718])
        .expect("ownerless SendMessage should use RHEngine::ProcessMessage");

    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .expect("script installed")
            .state
            .globals
            .get(&902),
        Some(&2718)
    );
}

#[test]
fn every_script_vm_flavor_drives_yields_through_the_shared_engine_boundary() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let actor_id = engine.add_entity(scripted_soldier("YieldingFlavor"));
    let actor_handle = bind_script_actor(&mut engine, actor_id, "YieldingFlavor");
    let zone = 7;
    let target_handle = ScriptHandleCodec::actor_handle_from_index(7001);
    let scroll_handle = ScriptHandleCodec::actor_handle_from_index(7002);
    let path = crate::ai::PathId::new(9).unwrap();
    {
        let script = engine.scripts.mission.as_mut().unwrap();
        let zone_instance = script.manager.create_instance("YieldingFlavor").unwrap();
        let target_instance = script.manager.create_instance("YieldingFlavor").unwrap();
        let scroll_instance = script.manager.create_instance("YieldingFlavor").unwrap();
        let waypoint_instance = script.manager.create_instance("YieldingFlavor").unwrap();
        script.zone_instances.insert(zone, zone_instance);
        script
            .target_instances
            .insert(target_handle, target_instance);
        script
            .scroll_instances
            .insert(scroll_handle, scroll_instance);
        script
            .waypoint_instances
            .insert((path, 3), waypoint_instance);
    }

    let calls = [
        (
            super::ScriptVmKey::Global,
            "Hourglass",
            Vec::new(),
            crate::natives::ScriptCallFrame::default(),
            4240,
        ),
        (
            super::ScriptVmKey::Actor(actor_handle),
            "Initialize",
            Vec::new(),
            crate::natives::ScriptCallFrame::actor(actor_handle),
            4241,
        ),
        (
            super::ScriptVmKey::Zone(zone),
            "EnterZone",
            vec![actor_handle],
            crate::natives::ScriptCallFrame::default(),
            4241,
        ),
        (
            super::ScriptVmKey::Target(target_handle),
            "ActivatedByArrow",
            vec![actor_handle],
            crate::natives::ScriptCallFrame::actor(target_handle),
            4241,
        ),
        (
            super::ScriptVmKey::Scroll(scroll_handle),
            "IsTaken",
            vec![actor_handle],
            crate::natives::ScriptCallFrame::scroll(scroll_handle),
            4241,
        ),
        (
            super::ScriptVmKey::Waypoint(path, 3),
            "ReachPoint",
            vec![actor_handle],
            crate::natives::ScriptCallFrame::default(),
            4241,
        ),
    ];
    for (key, function, params, frame, expected_message) in calls {
        engine
            .scripts
            .mission
            .as_mut()
            .unwrap()
            .state
            .globals
            .insert(902, 0);
        engine
            .call_script_vm(&assets, key, function, &params, frame)
            .unwrap_or_else(|error| panic!("{key:?}.{function} failed: {error}"));
        assert_eq!(
            engine
                .scripts
                .mission
                .as_ref()
                .unwrap()
                .state
                .globals
                .get(&902),
            Some(&expected_message),
            "{key:?}.{function} did not synchronously resume after ownerless ProcessMessage"
        );
    }
}

#[test]
fn shared_driver_preserves_same_actor_outer_activation() {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerSelf",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("A→A callback should resume its outer activation");

    let globals = &engine
        .scripts
        .mission
        .as_ref()
        .expect("script installed")
        .state
        .globals;
    assert_eq!(globals.get(&900), Some(&314), "nested callback completed");
    assert_eq!(
        globals.get(&901),
        Some(&2),
        "outer callback resumed after child"
    );
}

#[test]
fn self_reentrant_driver_preserves_and_shares_the_instance_member_heap() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let actor_id = engine.add_entity(scripted_soldier("HeapA"));
    let handle = bind_script_actor(&mut engine, actor_id, "HeapA");

    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerSelf",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("A→A member-heap callback");

    let script = engine.scripts.mission.as_ref().unwrap();
    assert_eq!(
        script.state.globals.get(&905),
        Some(&20),
        "heap={}, globals={:?}",
        script_instance_heap_word(script, handle),
        script.state.globals
    );
    assert_eq!(script_instance_heap_word(script, handle), 3);
}

#[test]
fn completed_reentrant_continuation_round_trips_as_an_idle_snapshot() {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerSelf",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("first self-reentrant callback");
    let program = engine
        .scripts
        .mission
        .as_ref()
        .expect("mission")
        .manager
        .program
        .clone();
    let json = serde_json::to_string(&engine).expect("idle engine snapshot");
    let mut restored: EngineInner = serde_json::from_str(&json).expect("restore idle snapshot");
    restored
        .scripts
        .mission
        .as_mut()
        .expect("restored mission")
        .attach_program(program);
    restored.attach_script_bindings(&assets);
    restored
        .scripts
        .mission
        .as_mut()
        .unwrap()
        .state
        .globals
        .insert(901, 0);

    restored
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerSelf",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("restored instance accepts another self-reentrant callback");
    assert_eq!(
        restored
            .scripts
            .mission
            .as_ref()
            .unwrap()
            .state
            .globals
            .get(&901),
        Some(&2)
    );
}

#[test]
fn real_active_driver_rejects_snapshot_and_idle_driver_serializes() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let actor_id = engine.add_entity(scripted_soldier("YieldingFlavor"));
    let handle = bind_script_actor(&mut engine, actor_id, "YieldingFlavor");

    super::script::arm_active_driver_snapshot_probe();
    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "EmitEffect",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("effect-producing callback");
    let error = super::script::take_active_driver_snapshot_error()
        .expect("effect drain executed the active-driver snapshot probe");
    assert!(error.contains("active script callback"), "{error}");
    let script = engine.scripts.mission.as_ref().unwrap();
    assert_eq!(script.active_call_frame_count(), 0);
    serde_json::to_string(&engine).expect("idle full-engine state is snapshot-safe");
}

#[test]
fn shared_driver_preserves_a_b_a_activation_stack() {
    let (mut engine, _receiver, a_handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let relay_id = engine.add_entity(scripted_soldier("RelayReceiver"));
    let b_handle = ScriptHandleCodec::actor_handle(relay_id);
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &mut engine.world.entities,
        &mut engine.ai.global,
        &mut engine.world.fast_grid,
    );
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("script installed")
            .bind_actor(
                b_handle,
                "RelayReceiver",
                &mut engine.script_domains,
                &capabilities,
            )
    );

    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(a_handle),
            "TriggerRelay",
            &[b_handle],
            crate::natives::ScriptCallFrame::actor(a_handle),
        )
        .expect("A→B→A callback stack should fully unwind");

    let globals = &engine
        .scripts
        .mission
        .as_ref()
        .expect("script installed")
        .state
        .globals;
    assert_eq!(globals.get(&900), Some(&20), "B re-entered A");
    assert_eq!(globals.get(&903), Some(&3), "outer A resumed last");
}

#[test]
fn a_b_a_driver_preserves_each_instances_member_heap() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let a_id = engine.add_entity(scripted_soldier("HeapA"));
    let b_id = engine.add_entity(scripted_soldier("HeapB"));
    let a_handle = bind_script_actor(&mut engine, a_id, "HeapA");
    let b_handle = bind_script_actor(&mut engine, b_id, "HeapB");

    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(a_handle),
            "TriggerRelay",
            &[b_handle],
            crate::natives::ScriptCallFrame::actor(a_handle),
        )
        .expect("A→B→A member-heap callback");

    let script = engine.scripts.mission.as_ref().unwrap();
    assert_eq!(
        script.state.globals.get(&906),
        Some(&20),
        "A heap={}, B heap={}, globals={:?}",
        script_instance_heap_word(script, a_handle),
        script_instance_heap_word(script, b_handle),
        script.state.globals
    );
    assert_eq!(script_instance_heap_word(script, a_handle), 30);
    assert_eq!(script_instance_heap_word(script, b_handle), 12);
}

#[test]
fn shared_driver_reports_missing_vm_and_depth_overflow_as_errors() {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    assert_eq!(
        engine
            .call_script_vm(
                &assets,
                super::ScriptVmKey::Actor(handle),
                "OptionalMissingMethod",
                &[],
                crate::natives::ScriptCallFrame::actor(handle),
            )
            .expect("a missing optional method is the base no-op"),
        0
    );
    let missing = ScriptHandleCodec::actor_handle_from_index(9999);
    assert!(
        engine
            .call_script_vm(
                &assets,
                super::ScriptVmKey::Actor(missing),
                "ProcessMessage",
                &[],
                crate::natives::ScriptCallFrame::actor(missing),
            )
            .expect_err("missing required VM must not fabricate a return")
            .contains("required VM is not bound")
    );

    let recursive_id = engine.add_entity(scripted_soldier("RecursiveReceiver"));
    let recursive_handle = bind_script_actor(&mut engine, recursive_id, "RecursiveReceiver");
    let error = engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(recursive_handle),
            "ProcessMessage",
            &[1, 0, 0],
            crate::natives::ScriptCallFrame::actor(recursive_handle),
        )
        .expect_err("recursive ProcessMessage must stop at the driver boundary");
    assert!(error.contains("depth limit"), "unexpected error: {error}");
    let direct_states: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| &sequence.elements)
        .filter(|element| element.command == Command::SendMessage)
        .map(|element| element.state)
        .collect();
    assert_eq!(
        direct_states
            .iter()
            .filter(|state| **state == SequenceState::Terminated)
            .count(),
        3,
        "four real VM activations are accepted; the first is direct"
    );
    assert_eq!(
        direct_states
            .iter()
            .filter(|state| **state == SequenceState::Impossible)
            .count(),
        1,
        "the fifth real VM activation is rejected"
    );

    let (mut external, _receiver, _handle) = engine_with_receiver();
    let recursive_id = external.add_entity(scripted_soldier("RecursiveReceiver"));
    let recursive_handle = bind_script_actor(&mut external, recursive_id, "RecursiveReceiver");
    let error = external
        .call_external_native(&assets, "SendMessage", &[recursive_handle, 1])
        .expect_err("external receiver context must not consume a VM depth slot");
    assert!(error.contains("depth limit"), "unexpected error: {error}");
    let external_states: Vec<_> = external
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| &sequence.elements)
        .filter(|element| element.command == Command::SendMessage)
        .map(|element| element.state)
        .collect();
    assert_eq!(
        external_states
            .iter()
            .filter(|state| **state == SequenceState::Terminated)
            .count(),
        4,
        "the synthetic external receiver is not a real VM activation"
    );
    assert_eq!(
        external_states
            .iter()
            .filter(|state| **state == SequenceState::Impossible)
            .count(),
        1
    );
}

#[test]
fn nested_sequence_actions_finish_before_parent_tail() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let mut ordering = scripted_soldier("OrderingReceiver");
    ordering.element_data_mut().blipped = true;
    let ordering_id = engine.add_entity(ordering);
    let ordering_handle = ScriptHandleCodec::actor_handle(ordering_id);
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &mut engine.world.entities,
        &mut engine.ai.global,
        &mut engine.world.fast_grid,
    );
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("script installed")
            .bind_actor(
                ordering_handle,
                "OrderingReceiver",
                &mut engine.script_domains,
                &capabilities,
            )
    );

    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(ordering_handle),
            "TriggerParentOrder",
            &[],
            crate::natives::ScriptCallFrame::actor(ordering_handle),
        )
        .expect("nested sequence stack should drain depth-first");

    let script = engine.scripts.mission.as_ref().expect("script installed");
    assert_eq!(
        script.state.globals.get(&904),
        Some(&0),
        "nested ProcessMessage ran before the parent's later Unblip"
    );
    assert_eq!(
        script.state.globals.get(&907),
        Some(&0),
        "nested LockAI sequence completed and resumed before parent Unblip"
    );
    let actor = engine.get_entity(ordering_id).expect("ordering actor");
    assert!(!actor.element_data().blipped, "parent tail eventually ran");
    assert!(
        actor
            .ai_controller()
            .expect("ordering actor AI")
            .script_locked,
        "nested LockAI completed before control returned to the parent"
    );
}

#[test]
fn detached_parent_tail_is_restored_when_child_dispatch_fails() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let failure_id = engine.add_entity(scripted_soldier("FailureReceiver"));
    engine
        .get_entity_mut(failure_id)
        .unwrap()
        .element_data_mut()
        .blipped = true;
    let failure_handle = bind_script_actor(&mut engine, failure_id, "FailureReceiver");
    let missing_id = engine.add_entity(scripted_soldier(""));
    let missing_handle = ScriptHandleCodec::actor_handle(missing_id);
    let error = engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(failure_handle),
            "TriggerFailure",
            &[missing_handle],
            crate::natives::ScriptCallFrame::actor(failure_handle),
        )
        .expect_err("nested child with a missing required VM must fail");
    assert!(
        error.contains("required VM is not bound"),
        "unexpected error: {error}"
    );

    let mut parent_send = None;
    let mut child_send = None;
    let mut parent_unblip = None;
    for sequence in engine.orders.sequence_manager.sequences_iter() {
        for (element_index, element) in sequence.elements.iter().enumerate() {
            match (element.command, element.state) {
                (Command::SendMessage, SequenceState::Terminated) => {
                    parent_send = Some((sequence.id, element_index));
                }
                (Command::SendMessage, SequenceState::Impossible) => {
                    child_send = Some((sequence.id, element_index));
                }
                (Command::Unblip, SequenceState::Todo) => {
                    parent_unblip = Some((sequence.id, element_index));
                }
                _ => {}
            }
        }
    }
    assert!(parent_send.is_some(), "successful ancestor is Terminated");
    assert!(child_send.is_some(), "only the actual child is Impossible");
    let parent_unblip = parent_unblip.expect("detached parent Unblip tail");
    assert_eq!(
        engine
            .get_entity(failure_id)
            .unwrap()
            .element_data()
            .blipped,
        true,
        "parent tail was restored but not overtaken after the child error"
    );
    assert!(
        matches!(
            engine
                .orders
                .sequence_manager
                .pop_pending_immediate_action(),
            Some(SequenceAction::ExecuteImmediateOwner {
                sequence_id,
                element_index,
                ..
            }) if (sequence_id, element_index) == parent_unblip
        ),
        "the real native path restores the detached parent tail on error"
    );
}

#[test]
fn open_scroll_terminates_before_nested_child_failure_and_restores_tail() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    let mut scroll = crate::element::ElementScroll::default();
    scroll.element.kind = ElementKind::ObjectScroll;
    scroll.element.active = true;
    let scroll_id = engine.add_entity(Entity::Scroll(scroll));
    let scroll_handle = ScriptHandleCodec::actor_handle(scroll_id);
    let reader_id = engine.add_entity(scripted_soldier(""));
    engine
        .get_entity_mut(reader_id)
        .expect("reader")
        .element_data_mut()
        .blipped = true;

    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &mut engine.world.entities,
        &mut engine.ai.global,
        &mut engine.world.fast_grid,
    );
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("script installed")
            .bind_scroll(
                scroll_handle,
                "OpenScrollFailure",
                &mut engine.script_domains,
                &capabilities,
            )
    );

    let mut open_scroll = SequenceElement::new_generic(1, Command::OpenScroll, None);
    open_scroll.set_property(Field::Scroll, FieldValue::Element(scroll_id));
    open_scroll.set_property(Field::ScrollReader, FieldValue::Element(reader_id));
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(open_scroll);
    sequence.append_element(SequenceElement::new(1, Command::Unblip, Some(reader_id)));
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);

    let error = engine
        .drain_script_synchronous_actions(&assets, &mut Vec::new())
        .expect_err("nested IsTaken SendMessage must fail on the missing reader VM");
    assert!(
        error.detail.contains("required VM is not bound"),
        "unexpected error: {}",
        error.detail
    );

    let sequence = engine
        .orders
        .sequence_manager
        .get_sequence(sequence_id)
        .expect("OpenScroll sequence");
    assert_eq!(sequence.elements[0].state, SequenceState::Terminated);
    assert_eq!(sequence.elements[1].state, SequenceState::Todo);
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .any(|sequence| {
                sequence.elements.iter().any(|element| {
                    element.command == Command::SendMessage
                        && element.state == SequenceState::Impossible
                })
            }),
        "only the nested SendMessage child is Impossible"
    );
    assert!(
        matches!(
            engine
                .orders
                .sequence_manager
                .pop_pending_immediate_action(),
            Some(SequenceAction::ExecuteImmediateOwner {
                sequence_id: pending_sequence,
                element_index: 1,
                ..
            }) if pending_sequence == sequence_id
        ),
        "the parent Unblip tail is restored after the child error"
    );
    assert!(
        engine
            .get_entity(reader_id)
            .expect("reader")
            .element_data()
            .blipped,
        "the restored parent tail was not executed after the error"
    );
}

#[test]
fn scroll_send_message_preserves_this_scroll_through_child_and_resume() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let observer_id = engine.add_entity(scripted_soldier("ScrollObserver"));
    let observer_handle = ScriptHandleCodec::actor_handle(observer_id);
    let scroll_handle = 0x1A2B_3C4D;
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &mut engine.world.entities,
        &mut engine.ai.global,
        &mut engine.world.fast_grid,
    );
    let script = engine.scripts.mission.as_mut().expect("script installed");
    assert!(script.bind_actor(
        observer_handle,
        "ScrollObserver",
        &mut engine.script_domains,
        &capabilities,
    ));
    assert!(script.bind_scroll(
        scroll_handle,
        "ScrollRelay",
        &mut engine.script_domains,
        &capabilities,
    ));

    let frame = crate::natives::ScriptCallFrame::default()
        .with_script_this(scroll_handle)
        .with_current_scroll(scroll_handle);
    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Scroll(scroll_handle),
            "TriggerScroll",
            &[observer_handle],
            frame,
        )
        .expect("scroll→actor message should preserve the caller frame");

    let globals = &engine
        .scripts
        .mission
        .as_ref()
        .expect("script installed")
        .state
        .globals;
    assert_eq!(globals.get(&905), Some(&scroll_handle));
    assert_eq!(globals.get(&906), Some(&scroll_handle));
}

#[test]
fn scroll_ownerless_send_message_preserves_this_scroll_in_global_and_parent() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let scroll_handle = 0x1020_3040;
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &mut engine.world.entities,
        &mut engine.ai.global,
        &mut engine.world.fast_grid,
    );
    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .bind_scroll(
            scroll_handle,
            "ScrollRelay",
            &mut engine.script_domains,
            &capabilities,
        );
    let frame = crate::natives::ScriptCallFrame::default()
        .with_script_this(scroll_handle)
        .with_current_scroll(scroll_handle);
    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Scroll(scroll_handle),
            "TriggerOwnerless",
            &[],
            frame,
        )
        .expect("scroll→global message should preserve caller frame");
    let globals = &engine.scripts.mission.as_ref().unwrap().state.globals;
    assert_eq!(globals.get(&902), Some(&66));
    assert_eq!(globals.get(&908), Some(&scroll_handle));
    assert_eq!(globals.get(&909), Some(&scroll_handle));
}

#[test]
fn set_actor_location_honolulu_finishes_before_same_callback_unlock() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerHonolulu",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("Honolulu action and continuation should finish synchronously");

    let entity = engine
        .get_entity(receiver)
        .expect("scripted receiver remains present");
    assert!(!entity.element_data().active);
    assert!(entity.element_data().in_honolulu);
    assert!(
        !entity
            .ai_controller()
            .expect("receiver NPC AI")
            .script_locked,
        "UnlockAI after SetActorLocation(NULL) observed and cleared the inline lock"
    );
}

#[test]
fn set_actor_location_preserves_original_partial_order_and_bool_contract() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let mut assets = LevelAssets::new();
    assets.scripts.location_count = 2;
    assets.scripts.point_count = 1;
    assets.scripts.location_positions = std::sync::Arc::new(vec![(12.0, 34.0), (90.0, 91.0)]);
    assets.scripts.location_layers = std::sync::Arc::new(vec![1, 1]);
    assets.scripts.location_sectors = std::sync::Arc::new(vec![7, 7]);
    engine.attach_script_bindings(&assets);

    {
        let element = engine
            .get_entity_mut(receiver)
            .expect("receiver")
            .element_data_mut();
        element.active = false;
        element.in_honolulu = true;
    }
    let sector_location = ScriptHandleCodec::location_handle_from_index(1);
    assert_eq!(
        engine
            .call_external_native(&assets, "SetActorLocation", &[handle, sector_location])
            .expect("wrong location type is a false result, not a driver error"),
        0
    );
    let element = engine.get_entity(receiver).unwrap().element_data();
    assert!(element.active && !element.in_honolulu);
    assert_eq!(
        element.position_map(),
        crate::coordinates::MapPoint::default()
    );

    let point_location = ScriptHandleCodec::location_handle_from_index(0);
    assert_eq!(
        engine
            .call_external_native(&assets, "SetActorLocation", &[handle, point_location])
            .expect("non-motion sector is a false result"),
        0
    );
    let element = engine.get_entity(receiver).unwrap().element_data();
    assert_eq!(
        element.position_map(),
        crate::coordinates::MapPoint::new(12.0, 34.0)
    );
    assert_eq!(element.layer(), 1);
    assert_eq!(element.sector().map(u16::from), Some(7));

    let mut level = crate::fast_find_grid::LevelGrid::default();
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        layer: 1,
        sector_number: crate::sector::SectorNumber::new(7),
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    });
    engine.world.fast_grid.level = std::sync::Arc::new(level);
    assert_eq!(
        engine
            .call_external_native(&assets, "SetActorLocation", &[handle, point_location])
            .expect("valid point succeeds"),
        1
    );
}

#[test]
fn persistent_life_and_concussion_are_visible_after_engine_yield_in_same_callback() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    let life = engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerLife",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("life setter resumes");
    assert_eq!(life, 37);
    assert_eq!(
        engine
            .get_entity(receiver)
            .expect("receiver")
            .npc_data()
            .expect("NPC")
            .life_points,
        37
    );

    let concussion = engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerConcussion",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("concussion setter resumes");
    assert_eq!(concussion, 123);
    assert_eq!(
        engine
            .get_entity(receiver)
            .expect("receiver")
            .human_data()
            .expect("human")
            .concussion_of_the_brain,
        123
    );
}

#[test]
fn persistent_setters_use_original_narrowing_and_virtual_kill_path() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    for amount in [-1, 65_535, 40_000] {
        assert_eq!(
            engine
                .call_external_native(&assets, "SetPersistentProperty", &[handle, 3, amount],)
                .expect("concussion setter"),
            1
        );
        let human = engine
            .get_entity(receiver)
            .expect("receiver")
            .human_data()
            .expect("human");
        assert_eq!(
            human.concussion_of_the_brain, 0,
            "UWORD {amount:#x} is negative when re-read as SWORD"
        );
        assert!(!human.unconscious);
    }

    {
        let entity = engine.get_entity_mut(receiver).unwrap();
        entity.npc_data_mut().unwrap().alerted = true;
        entity.enemy_ai_mut().unwrap().forced_attentive = true;
    }

    let sequence_count = engine.orders.sequence_manager.sequence_count();
    assert_eq!(
        engine
            .call_external_native(&assets, "SetPersistentProperty", &[handle, 2, 65_535])
            .expect("life setter"),
        1
    );
    let entity = engine.get_entity(receiver).expect("receiver");
    assert_eq!(entity.npc_data().expect("NPC").life_points, 0);
    assert_eq!(
        entity.ai_controller().expect("NPC AI").current_substate,
        crate::ai::Substate::SleepingForever
    );
    assert!(!entity.npc_data().expect("NPC").alerted);
    let enemy = entity.enemy_ai().expect("soldier enemy AI");
    assert_eq!(
        enemy.base.current_music_alert_status,
        crate::ai::AlertLevel::Green
    );
    assert_eq!(
        enemy.base.view_alert_status,
        crate::ai::AlertLevel::Yellow,
        "virtual Kill preserves the forced-attentive Green→Yellow view override"
    );
    assert_eq!(
        engine.orders.sequence_manager.sequence_count(),
        sequence_count,
        "script SetLifePoints must not synthesize a ReceiveDamage sequence"
    );
}

#[test]
fn scripted_invulnerable_life_setter_forces_literal_one_hundred() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    {
        let entity = engine.get_entity_mut(receiver).expect("receiver");
        entity.human_data_mut().unwrap().invulnerable = true;
        entity.npc_data_mut().unwrap().life_points = 80;
    }

    assert_eq!(
        engine
            .call_external_native(&assets, "SetPersistentProperty", &[handle, 2, 0])
            .expect("scripted invulnerable life setter"),
        1
    );
    assert_eq!(
        engine
            .get_entity(receiver)
            .unwrap()
            .npc_data()
            .unwrap()
            .life_points,
        100,
        "RHElementActorHuman::SetLifePoints stores literal 100 for invulnerable humans"
    );
}

#[test]
fn scripted_pc_concussion_and_ko_unselect_immediately() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let make_pc = || {
        let mut entity = Entity::Pc(crate::element::ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: crate::element::PcData {
                playable: true,
                ..crate::element::PcData::default()
            },
        });
        let row = crate::sprite_script::SpriteScript {
            action_id: 0,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![0],
            delays: vec![0],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![row]),
            std::sync::Arc::new(vec![
                crate::sprite_script::UNMAPPED;
                crate::sprite_script::NONANIMATION_END
            ]),
        );
        entity
    };
    let persistent_pc = engine.add_entity(make_pc());
    let posture_pc = engine.add_entity(make_pc());
    let persistent_handle = ScriptHandleCodec::actor_handle(persistent_pc);
    let posture_handle = ScriptHandleCodec::actor_handle(posture_pc);
    engine.players.seats[0].selection = vec![persistent_pc, posture_pc];

    engine
        .call_external_native(
            &assets,
            "SetPersistentProperty",
            &[persistent_handle, 3, crate::combat::CONCUSSION_MAX as i32],
        )
        .expect("persistent concussion");
    assert_eq!(engine.players.seats[0].selection, vec![posture_pc]);

    engine
        .call_external_native(&assets, "SetActorPosture", &[posture_handle, 17])
        .expect("posture KO");
    assert!(engine.players.seats[0].selection.is_empty());
}

#[test]
fn posture_wait_uses_real_instruction_path_before_callback_resumes() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    let posture = engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerPosture",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("posture WAIT should dispatch and resume");
    assert_eq!(posture, 2);
    let (sequence_id, element_index) = engine
        .orders
        .sequence_manager
        .current_element_for_actor(receiver)
        .expect("WAIT is the actor's live canonical instruction");
    let wait = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, element_index)
        .expect("WAIT element");
    assert_eq!(wait.command, Command::Wait);
    assert_eq!(wait.state, SequenceState::InProgress);
}

#[test]
fn anonymous_archer_accepts_a_non_pc_human_and_adds_hidden_titbit_inline() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    assert_eq!(
        engine
            .call_external_native(&assets, "SetActorPosture", &[handle, 100])
            .expect("AnonymousArcher should accept a human soldier"),
        0
    );
    let entity = engine.get_entity(receiver).expect("receiver");
    assert_eq!(entity.element_data().posture, Posture::AnonymousArcher);
    assert_eq!(
        entity.actor_data().expect("actor").action_state,
        crate::element::ActionState::Waiting
    );
    assert!(engine.feedback.titbit_manager.titbit_exists(
        crate::titbit::TitbitKind::Hidden,
        crate::titbit::ElementHandle(receiver.index()),
    ));
}

#[test]
#[should_panic(expected = "SetActorPosture(UPRIGHT) from CarryingCorpse requires a PC")]
fn upright_from_carrying_corpse_rejects_a_non_pc_human() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    engine
        .get_entity_mut(receiver)
        .expect("receiver")
        .set_posture(Posture::CarryingCorpse);

    let _ = engine.call_external_native(&LevelAssets::new(), "SetActorPosture", &[handle, 0]);
}

#[test]
fn action_state_set_get_resumes_after_real_wait_instruction() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    let state = engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerActionState",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("action-state setter should dispatch WAIT and resume");
    assert_eq!(state, crate::element::ActionState::Bored as i32);
    assert_eq!(
        engine
            .get_entity(receiver)
            .expect("receiver")
            .actor_data()
            .expect("actor")
            .action_state,
        crate::element::ActionState::Bored
    );
    let current = engine
        .orders
        .sequence_manager
        .current_element_for_actor(receiver)
        .expect("WAIT is live before callback return");
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(current.0, current.1)
            .expect("WAIT element")
            .command,
        Command::Wait
    );
}

#[test]
fn recorded_timer_is_registered_before_thanx_returns() {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    engine
        .call_script_vm(
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerTimer",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("recorded timer should execute immediately");

    assert_eq!(engine.orders.timer_elements.len(), 1);
    assert_eq!(engine.orders.timer_elements[0].remaining, 12);
    let timer_ref = engine.orders.timer_elements[0].element_ref;
    let timer = engine
        .orders
        .sequence_manager
        .get_element(timer_ref.sequence_id, timer_ref.element_index)
        .expect("timer sequence element");
    assert_eq!(timer.command, Command::Timer);
    assert_eq!(
        timer.state,
        SequenceState::Todo,
        "Original ExecutedImmediately registers Timer before Go, so it stays TODO until expiry terminates it"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_pending_immediate_actions()
    );
}

#[test]
fn recorded_lock_user_clears_and_restores_selection_in_original_order() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let pc_id = engine.add_entity(Entity::Pc(crate::element::ActorPc {
        element: ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: crate::element::PcData {
            playable: true,
            current_action: crate::profiles::Action::Bow,
            ..crate::element::PcData::default()
        },
    }));
    engine.players.seats[0].selection.push(pc_id);

    // MSG_UNLOCK_USER only adds the saved selection. With no preceding lock,
    // it must leave the live selection untouched.
    engine.call_external_native(&assets, "Start", &[]).unwrap();
    engine
        .call_external_native(&assets, "RecordUnLockUser", &[])
        .unwrap();
    engine.call_external_native(&assets, "Thanx", &[]).unwrap();
    assert_eq!(engine.players.seats[0].selection, vec![pc_id]);

    engine.call_external_native(&assets, "Start", &[]).unwrap();
    engine
        .call_external_native(&assets, "RecordLockUser", &[])
        .unwrap();
    engine.call_external_native(&assets, "Thanx", &[]).unwrap();
    assert!(engine.players.user_locked);
    assert!(engine.players.seats[0].selection.is_empty());
    assert_eq!(
        engine
            .get_entity(pc_id)
            .unwrap()
            .pc_data()
            .unwrap()
            .current_action,
        crate::profiles::Action::NoAction
    );

    engine.call_external_native(&assets, "Start", &[]).unwrap();
    engine
        .call_external_native(&assets, "RecordUnLockUser", &[])
        .unwrap();
    engine.call_external_native(&assets, "Thanx", &[]).unwrap();
    assert!(!engine.players.user_locked);
    assert_eq!(engine.players.seats[0].selection, vec![pc_id]);
    assert_eq!(engine.players.selection_before_user_lock, vec![pc_id]);

    // The original saved list is not consumed; repeated Unlock remains an
    // additive idempotent selection restore.
    engine.call_external_native(&assets, "Start", &[]).unwrap();
    engine
        .call_external_native(&assets, "RecordUnLockUser", &[])
        .unwrap();
    engine.call_external_native(&assets, "Thanx", &[]).unwrap();
    assert_eq!(engine.players.seats[0].selection, vec![pc_id]);
    assert_eq!(engine.players.selection_before_user_lock, vec![pc_id]);
    assert!(engine.feedback.pending_side_effects.pending_reset_input);
}

fn scripted_receiver() -> Entity {
    scripted_soldier("MessageReceiver")
}

fn scripted_soldier(script_class: &str) -> Entity {
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

fn bind_script_actor(
    engine: &mut EngineInner,
    actor_id: crate::element::EntityId,
    class_name: &str,
) -> i32 {
    let handle = ScriptHandleCodec::actor_handle(actor_id);
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &mut engine.world.entities,
        &mut engine.ai.global,
        &mut engine.world.fast_grid,
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

fn script_instance_heap_word(script: &MissionScript, handle: i32) -> i32 {
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

fn engine_with_receiver() -> (EngineInner, crate::element::EntityId, i32) {
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
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &mut engine.world.entities,
        &mut engine.ai.global,
        &mut engine.world.fast_grid,
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

fn integer_property(element: &SequenceElement, field: Field) -> u32 {
    match element.get_property(field) {
        Some(FieldValue::Integer(value)) => *value,
        other => panic!("expected integer {field:?} property, got {other:?}"),
    }
}

#[test]
fn script_send_message_sequence_does_not_preempt_current_actor_element() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    let active_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_movement(
            1,
            Command::Move,
            Some(receiver),
            OrderType::RunningUpright,
        ));
    engine
        .orders
        .sequence_manager
        .element_in_progress(active_id, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(receiver),
        Some((active_id, 0))
    );

    let frame_before = engine.control.frame_counter;
    engine
        .call_external_native(&assets, "SendMessageWithArguments", &[handle, 1234, 55, -7])
        .expect("SendMessageWithArguments should complete synchronously");

    assert_eq!(
        engine.control.frame_counter, frame_before,
        "SendMessage is zero-frame"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(receiver),
        Some((active_id, 0)),
        "ExecutedImmediately bypasses Instruct contention and preserves the current element"
    );

    let send = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .find(|element| element.command == Command::SendMessage)
        .expect("native request should launch a SendMessage sequence element");
    assert_eq!(send.owner, Some(receiver));
    assert_eq!(send.state, SequenceState::Terminated);
    assert_eq!(integer_property(send, Field::Message), 1234);
    assert_eq!(integer_property(send, Field::MessageArgument), 55);
    assert_eq!(
        integer_property(send, Field::MessageExtendedArgument),
        (-7_i32) as u32
    );
}

#[test]
fn script_send_message_callback_completes_before_sequence_launch_returns() {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    // Original: RHScript::SendMessage calls LaunchSequenceElement, whose
    // RHCOMMAND_SEND_MESSAGE ExecutedImmediately path invokes ProcessMessage
    // inline (RHScript.cpp:6846-6865; RHsequenceelement.cpp:736-777).
    engine
        .call_external_native(&assets, "SendMessage", &[handle, 314])
        .expect("SendMessage should complete synchronously");

    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .expect("script installed")
            .state
            .globals
            .get(&900),
        Some(&314),
        "the nested ProcessMessage mutation must be visible when sequence launch returns"
    );
    let send = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .find(|element| element.command == Command::SendMessage)
        .expect("SendMessage launch should retain its sequence element");
    assert_eq!(
        send.state,
        SequenceState::Terminated,
        "ProcessMessage and termination both happen inside the launch call"
    );
}

#[test]
fn script_send_message_callbacks_run_in_launch_order_in_same_frame() {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let frame_before = engine.control.frame_counter;

    engine
        .call_external_native(&assets, "SendMessage", &[handle, 41])
        .expect("first SendMessage");
    engine
        .call_external_native(&assets, "SendMessage", &[handle, 72])
        .expect("second SendMessage");

    assert_eq!(engine.control.frame_counter, frame_before);
    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .expect("script installed")
            .state
            .globals
            .get(&900),
        Some(&72),
        "ProcessMessage callbacks must run in SendMessage launch order"
    );
    let states: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.command == Command::SendMessage)
        .map(|element| element.state)
        .collect();
    assert_eq!(
        states,
        vec![SequenceState::Terminated, SequenceState::Terminated],
        "both callbacks and terminations complete without advancing a frame"
    );
}

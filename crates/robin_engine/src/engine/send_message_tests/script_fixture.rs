//! Synthetic script classes; each fixture owns its bytecode and function addresses.
use super::helpers::{
    HEAP0, TMP0, TMP1, TMP2, TMP3, begin_function, get_param, integer_constant, native_call,
    native_param, native_return, quad, return_value,
};
use crate::engine::types::MissionScript;
use crate::natives::NativeFn;
use crate::scb::{ClassEntry, Function, ScbFile};
use crate::vm::Opcode;

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

fn receiver_class() -> ClassEntry {
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
    ClassEntry {
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
            // The test's missing actor location yields through the
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
            // Timer is in the immediate-execution group and must register
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
    }
}

fn relay_class() -> ClassEntry {
    let relay_process_message = Function {
        name: "ProcessMessage".into(),
        address: 0,
        num_parameters: 3,
        size_of_return_value: 0,
        size_of_parameters: 12,
        size_of_volatile: 0,
        size_of_temporary: 8,
    };
    ClassEntry {
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
    }
}

fn ordering_class() -> ClassEntry {
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
    ClassEntry {
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
            // level 2. Thanx must not resume until SendMessage's state change has
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
    }
}

fn target_ordering_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn move_ordering_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn scroll_observer_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn self_deactivating_scroll_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn scroll_relay_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn startup_class() -> ClassEntry {
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
    ClassEntry {
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
    }
}

fn recursive_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn heap_a_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn heap_b_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn failure_receiver_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn open_scroll_failure_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn open_scroll_local_failure_class() -> ClassEntry {
    ClassEntry {
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
    }
}

fn yielding_flavor_class() -> ClassEntry {
    ClassEntry {
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
            size_of_parameters: num_parameters * 4,
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
    }
}

pub(super) fn message_script() -> MissionScript {
    MissionScript::from_scb(ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![
            startup_class(),
            receiver_class(),
            relay_class(),
            ordering_class(),
            target_ordering_class(),
            move_ordering_class(),
            scroll_observer_class(),
            self_deactivating_scroll_class(),
            freeze_toggling_scroll_class("FreezeOnScroll", true),
            freeze_toggling_scroll_class("FreezeOffScroll", false),
            scroll_relay_class(),
            recursive_class(),
            heap_a_class(),
            heap_b_class(),
            failure_receiver_class(),
            open_scroll_failure_class(),
            open_scroll_local_failure_class(),
            yielding_flavor_class(),
        ],
    })
    .expect("message test script should decode")
}

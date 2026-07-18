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
use crate::natives::{DeferredCommand, GameHost, NativeFn};
use crate::order::OrderType;
use crate::scb::{ClassEntry, Function, ScbFile};
use crate::sequence::{Field, FieldValue, SequenceElement, SequenceState};
use crate::vm::{Opcode, Quad};

const TMP0: u16 = 0xC000;
const TMP1: u16 = 0xC004;

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
    quad.operands[4..8].copy_from_slice(&(native as u32).to_le_bytes());
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
    let receiver = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "MessageReceiver".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![process_message],
        quads: vec![
            begin_function(2),
            integer_constant(TMP0, 900),
            get_param(TMP1, 0),
            native_param(TMP0),
            native_param(TMP1),
            native_call(NativeFn::SetGlobal),
            quad(Opcode::Return),
            quad(Opcode::EndFunction),
        ],
    };
    let startup = ClassEntry {
        source_file: "send_message_test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: Vec::new(),
        quads: Vec::new(),
    };
    MissionScript::from_scb(ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![startup, receiver],
    })
    .expect("message test script should decode")
}

fn scripted_receiver() -> Entity {
    Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        },
        actor: ActorData {
            script_class: "MessageReceiver".into(),
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

fn engine_with_receiver() -> (EngineInner, crate::element::EntityId, i32) {
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(message_script());
    engine.attach_script_bindings(&LevelAssets::new());
    let receiver = engine.add_entity(scripted_receiver());
    let handle = GameHost::actor_handle(receiver);
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("script installed")
            .bind_actor(
                handle,
                "MessageReceiver",
                crate::natives::NativeQueryViews::default(),
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

    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .game_host
        .deferred_commands
        .push(DeferredCommand::SendMessage {
            actor: handle,
            message: 1234,
            arg1: 55,
            arg2: -7,
        });
    let frame_before = engine.control.frame_counter;
    engine.sync_game_host_post_script(&assets);

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
    engine.launch_script_send_message(&assets, handle, 314, 0, 0);

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

    let host = &mut engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .game_host;
    host.deferred_commands.push(DeferredCommand::SendMessage {
        actor: handle,
        message: 41,
        arg1: 0,
        arg2: 0,
    });
    host.deferred_commands.push(DeferredCommand::SendMessage {
        actor: handle,
        message: 72,
        arg1: 0,
        arg2: 0,
    });

    engine.sync_game_host_post_script(&assets);

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

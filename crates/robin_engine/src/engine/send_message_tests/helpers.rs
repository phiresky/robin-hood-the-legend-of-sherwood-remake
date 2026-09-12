//! Focused parity tests for script `SendMessage`.
//!
//! Required behavior:
//! - Sending a scripted message builds an
//!   send-message element and launches the sequence element.
//! - Immediate sequence execution calls the owner's
//!   immediate execution directly (or the engine for a null owner), so the
//!   element does not enter normal actor instruction priority contention.
//! - Actor and engine dispatch invoke message processing and only
//!   then set the element to `RHSEQ_TERMINATED` in the same frame.

use crate::element::{Entity, SoldierData};
use crate::engine::EngineInner;
use crate::engine::test_support::asm;
pub(super) use crate::engine::test_support::asm::{
    q_aff0_iconstant as integer_constant, q_aff1_get_param as get_param,
    q_aff1_native_get_return as native_return, q_native_param as native_param,
    q_return_val as return_value,
};
use crate::engine::types::{LevelAssets, MissionScript};
use crate::natives::{NativeFn, ScriptHandleCodec};
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
    asm::q_begin_function(0, temp_count)
}

pub(super) fn native_call(native: NativeFn) -> Quad {
    asm::q_native_call(native as u32)
}

/// `ProcessMessage(message, _, _)` stores the received message in global 900.
/// Sending 41 then 72 must therefore leave 72, pinning callback launch order.
pub(super) use super::script_fixture::message_script;

pub(super) fn scripted_receiver() -> Entity {
    scripted_soldier("MessageReceiver")
}

pub(super) fn scripted_soldier(script_class: &str) -> Entity {
    let mut entity = crate::engine::test_support::actors::make_test_ai_soldier(
        SoldierData::default().cached_camp,
    );
    entity.position_iface_mut().clear_pathfinder_index();
    entity.element_data_mut().active = true;
    entity
        .actor_data_mut()
        .expect("script actor fixture")
        .script_class = script_class.into();
    entity
        .npc_data_mut()
        .expect("script soldier fixture")
        .life_points = 50;
    entity
}

pub(super) fn bind_script_actor(
    engine: &mut EngineInner,
    actor_id: crate::element::EntityId,
    class_name: &str,
) -> i32 {
    let handle = ScriptHandleCodec::actor_handle(actor_id);
    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .bind_actor(handle, class_name);
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
    engine.scripts.globals.resize(916, 0);
    engine.scripts.globals[904] = -1;
    engine.scripts.globals[907] = -1;
    engine.attach_script_bindings(&LevelAssets::new());
    let receiver = engine.add_test_entity(scripted_receiver());
    let handle = ScriptHandleCodec::actor_handle(receiver);
    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .bind_actor(handle, "MessageReceiver");
    (engine, receiver, handle)
}

pub(super) fn integer_property(element: &SequenceElement, field: Field) -> u32 {
    match element.get_property(field) {
        Some(FieldValue::Integer(value)) => *value,
        other => panic!("expected integer {field:?} property, got {other:?}"),
    }
}

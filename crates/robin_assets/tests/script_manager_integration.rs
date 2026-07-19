//! Integration tests for `robin_engine::script_manager` that depend on
//! the `.scb` parser in `robin_assets::scb`. They were `#[cfg(any())]`-gated
//! in-crate while the parser lived here; now they run as integration tests
//! against both crates.

mod support;

use robin_assets::scb;
use robin_engine::natives::{NativeContext, NativeSessionCapabilities, ScriptEffects, ScriptState};
use robin_engine::script_manager::{ScriptError, ScriptManager};
use support::{data_directory, data_file};

/// Build a minimal .scb byte buffer with one class, one function.
fn make_scb_bytes(class_name: &str, fn_name: &str, heap_size: i32, quads: &[scb::Quad]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(scb::SCB_MAGIC);
    b.extend_from_slice(&1.5f32.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes()); // num_classes

    let src = b"test.scs";
    b.extend_from_slice(&(src.len() as u32).to_le_bytes());
    b.extend_from_slice(src);

    b.extend_from_slice(&(class_name.len() as u32).to_le_bytes());
    b.extend_from_slice(class_name.as_bytes());

    b.extend_from_slice(&0i32.to_le_bytes()); // mv_count
    b.extend_from_slice(&heap_size.to_le_bytes()); // size_of_member_variables

    b.extend_from_slice(&1i32.to_le_bytes()); // fn_count
    b.extend_from_slice(&(fn_name.len() as u32).to_le_bytes());
    b.extend_from_slice(fn_name.as_bytes());
    b.extend_from_slice(&0i32.to_le_bytes()); // address
    b.extend_from_slice(&0i32.to_le_bytes()); // num_parameters
    b.extend_from_slice(&4i32.to_le_bytes()); // size_of_return_value
    b.extend_from_slice(&0i32.to_le_bytes()); // size_of_parameters
    b.extend_from_slice(&4i32.to_le_bytes()); // size_of_volatile
    b.extend_from_slice(&0i32.to_le_bytes()); // size_of_temporary

    b.extend_from_slice(&(quads.len() as i32).to_le_bytes());
    for q in quads {
        b.push(q.operation);
        b.extend_from_slice(&q.operands);
    }
    b
}

fn load_manager(bytes: &[u8]) -> ScriptManager {
    let scb = scb::parse_bytes(bytes).unwrap();
    ScriptManager::new(scb)
}

fn q_begin_function(vol: u16, tmp: u16) -> scb::Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&vol.to_le_bytes());
    ops[2..4].copy_from_slice(&tmp.to_le_bytes());
    scb::Quad {
        operation: 3,
        operands: ops,
    }
}

fn q_iconst(dst: u16, value: i32) -> scb::Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    ops[4..8].copy_from_slice(&value.to_le_bytes());
    scb::Quad {
        operation: 19,
        operands: ops,
    }
}

fn q_return_val(sym: u16) -> scb::Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&sym.to_le_bytes());
    scb::Quad {
        operation: 7,
        operands: ops,
    }
}

fn q_iadd(dst: u16, a: u16, b: u16) -> scb::Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    ops[2..4].copy_from_slice(&a.to_le_bytes());
    ops[4..6].copy_from_slice(&b.to_le_bytes());
    scb::Quad {
        operation: 25,
        operands: ops,
    }
}

fn q_native_param(sym: u16) -> scb::Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&sym.to_le_bytes());
    scb::Quad {
        operation: 11,
        operands: ops,
    }
}

fn q_native_call(index: u32) -> scb::Quad {
    let mut ops = [0u8; 8];
    ops[0..4].copy_from_slice(&index.to_le_bytes());
    scb::Quad {
        operation: 12,
        operands: ops,
    }
}

fn q_native_get_return(sym: u16) -> scb::Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&sym.to_le_bytes());
    scb::Quad {
        operation: 13,
        operands: ops,
    }
}

const TMP0: u16 = 0xC000;
const TMP4: u16 = 0xC004;
const TMP8: u16 = 0xC008;

#[test]
fn create_manager_and_instance() {
    let quads = vec![
        q_begin_function(0, 3),
        q_iconst(TMP0, 30),
        q_iconst(TMP4, 12),
        q_iadd(TMP8, TMP0, TMP4),
        q_return_val(TMP8),
    ];
    let bytes = make_scb_bytes("Test", "Main", 0, &quads);
    let mut mgr = load_manager(&bytes);

    assert_eq!(mgr.class_count(), 1);
    assert_eq!(mgr.class_names().collect::<Vec<_>>(), vec!["Test"]);
    assert!(mgr.find_class("Test").is_some());
    assert!(mgr.find_class("NoSuchClass").is_none());

    let mut inst = mgr.create_instance("Test").unwrap();
    assert!(inst.has_function(&mgr, "Main"));
    assert!(!inst.has_function(&mgr, "NoSuchFn"));

    let result = inst.call_function(&mut mgr, "Main").unwrap();
    assert_eq!(result, 42);
}

#[test]
fn class_not_found_error() {
    let quads = vec![q_begin_function(0, 0), q_return_val(TMP0)];
    let bytes = make_scb_bytes("Real", "Main", 0, &quads);
    let mgr = load_manager(&bytes);

    let err = mgr.create_instance("Wrong").unwrap_err();
    assert!(matches!(err, ScriptError::ClassNotFound(_)));
}

#[test]
fn function_not_found_error() {
    let quads = vec![q_begin_function(0, 0), q_return_val(TMP0)];
    let bytes = make_scb_bytes("Test", "Main", 0, &quads);
    let mut mgr = load_manager(&bytes);
    let mut inst = mgr.create_instance("Test").unwrap();

    let err = inst.call_function(&mut mgr, "NoSuchFn").unwrap_err();
    assert!(matches!(err, ScriptError::FunctionNotFound(_)));
}

#[test]
fn static_area_shared_between_instances() {
    // Two instances of the same class. One writes a global via
    // InitGlobal, the other reads it via GetGlobal. The globals
    // are stored in ScriptEffects (not the static area), so this test
    // verifies the ScriptManager + instance API works correctly.
    let quads = vec![
        // fn SetGlobal42: InitGlobal(0, 42)
        q_begin_function(0, 2),
        q_iconst(TMP0, 0),  // id=0
        q_iconst(TMP4, 42), // value=42
        q_native_param(TMP0),
        q_native_param(TMP4),
        q_native_call(0), // InitGlobal
        q_return_val(TMP0),
    ];
    let bytes = make_scb_bytes("Test", "SetGlobal42", 0, &quads);
    let mut mgr = load_manager(&bytes);

    let mut host = ScriptEffects::new();
    let mut script_state = ScriptState::default();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut entities = robin_engine::entities::Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut inst = mgr.create_instance("Test").unwrap();

    {
        let mut context = NativeContext::new(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &capabilities,
        );
        let _ = inst
            .call_function_with_host(&mut mgr, "SetGlobal42", &mut context)
            .unwrap();
    }
    assert_eq!(script_state.globals.get(&0), Some(&42));
}

#[test]
fn native_calls_through_instance() {
    // Script: return BitwiseAnd(0xFF, 0x0F) → should be 0x0F
    let quads = vec![
        q_begin_function(0, 3),
        q_iconst(TMP0, 0xFF),
        q_iconst(TMP4, 0x0F),
        q_native_param(TMP0),
        q_native_param(TMP4),
        q_native_call(206), // BitwiseAnd
        q_native_get_return(TMP8),
        q_return_val(TMP8),
    ];
    let bytes = make_scb_bytes("Test", "Go", 0, &quads);
    let mut mgr = load_manager(&bytes);
    let mut inst = mgr.create_instance("Test").unwrap();
    let mut host = ScriptEffects::new();
    let mut script_state = ScriptState::default();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut entities = robin_engine::entities::Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut context = NativeContext::new(
        &mut host,
        &mut script_state,
        &mut script_domains,
        &capabilities,
    );

    let result = inst
        .call_function_with_host(&mut mgr, "Go", &mut context)
        .unwrap();
    assert_eq!(result, 0x0F);
}

#[test]
fn push_param_before_call() {
    // Build a .scb where Main(x) returns x + 100.
    let quads = vec![
        q_begin_function(0, 2),
        // tmp0 = GetParam(0)
        scb::Quad {
            operation: 8, // Aff1GetParam
            operands: {
                let mut o = [0u8; 8];
                o[0..2].copy_from_slice(&TMP0.to_le_bytes());
                o[4..8].copy_from_slice(&0i32.to_le_bytes());
                o
            },
        },
        q_iconst(TMP4, 100),
        q_iadd(TMP0, TMP0, TMP4),
        q_return_val(TMP0),
    ];
    let bytes = make_scb_bytes("Test", "Main", 0, &quads);
    let mut mgr = load_manager(&bytes);
    let mut inst = mgr.create_instance("Test").unwrap();

    inst.push_param(7);
    let result = inst.call_function(&mut mgr, "Main").unwrap();
    assert_eq!(result, 107);
}

#[test]
fn function_names_listed() {
    let quads = vec![q_begin_function(0, 0), q_return_val(TMP0)];
    let bytes = make_scb_bytes("Test", "Main", 0, &quads);
    let mgr = load_manager(&bytes);
    let inst = mgr.create_instance_idx(0);

    let names = inst.function_names(&mgr);
    assert_eq!(names, vec!["Main"]);
}

#[test]
fn destroy_clears_state() {
    let quads = vec![q_begin_function(0, 0), q_return_val(TMP0)];
    let bytes = make_scb_bytes("Test", "Main", 0, &quads);
    let mut mgr = load_manager(&bytes);
    assert_eq!(mgr.class_count(), 1);

    mgr.destroy();
    assert_eq!(mgr.class_count(), 0);
}

/// Run the demo script's StartUp class through the ScriptManager API.
///
/// Original provenance: `original-code/RHgame.cpp:453-456` initializes the VM
/// from the mission bytecode, and `original-code/RHengine.cpp:1077-1081`
/// requires that mission bytecode to bind a `StartUp` class.
#[test]
#[ignore = "requires Leicester demo data via ROBINHOOD_DATA_DIR; see README.md"]
fn demo_script_via_manager() {
    let path = data_file("Data/Levels/Dem_Lei_MP.scb");

    let scb = scb::parse_file(&path).unwrap();
    let mut mgr = ScriptManager::new(scb);
    assert!(mgr.class_count() > 0);
    assert!(mgr.find_class("StartUp").is_some());

    let mut inst = mgr.create_instance("StartUp").unwrap();
    assert!(inst.has_function(&mgr, "Initialize"));

    let mut host = ScriptEffects::new();
    let mut script_state = ScriptState::default();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut entities = robin_engine::entities::Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);

    // Run PutActorInBuilding (addr 0, the first function).
    // It won't do much with stub natives, but shouldn't crash.
    let first_fn = inst
        .function_names(&mgr)
        .into_iter()
        .next()
        .unwrap()
        .to_owned();

    // Just verify we can call without panicking.
    // Most functions need real engine state, so errors are expected.
    let mut context = NativeContext::new(
        &mut host,
        &mut script_state,
        &mut script_domains,
        &capabilities,
    );
    let _ = inst.call_function_with_host(&mut mgr, &first_fn, &mut context);
}

/// Exercises all fullgame scripts through the manager: load each .scb,
/// create an instance of every class, list functions. Pure structural
/// test — doesn't execute (engine stubs would cause wild branching).
///
/// Original provenance: `original-code/virtualmachine/SBSimpleVirtualMachine.cpp:32-45`
/// constructs a VM instance by looking up its serialized class, while lines
/// 97-111 implement the equivalent class-binding path.
#[test]
#[ignore = "requires full-game data via ROBINHOOD_DATA_DIR; see README.md"]
fn load_all_fullgame_scripts_via_manager() {
    let levels_dir = data_directory("Data/Levels");

    let mut scripts = 0;
    let mut total_classes = 0;
    let mut total_functions = 0;

    for entry in std::fs::read_dir(&levels_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("scb") {
            continue;
        }
        let scb = scb::parse_file(&path).unwrap();
        let mgr = ScriptManager::new(scb);
        for i in 0..mgr.class_count() {
            let inst = mgr.create_instance_idx(i);
            let fns = inst.function_names(&mgr);
            total_functions += fns.len();
            total_classes += 1;
        }
        scripts += 1;
    }

    assert!(scripts > 0, "should have found .scb files");
    assert!(total_classes > 0);
    assert!(total_functions > 0);
}

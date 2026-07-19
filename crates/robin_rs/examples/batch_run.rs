//! Run Initialize on every .scb in a directory and report results.
//!
//!   cargo run --bin batch_run -- datadirs/fullgame/Data/Levels
#![deny(clippy::print_stdout, clippy::print_stderr)]

use robin_assets::scb;
use robin_engine::interp::Vm;
use robin_engine::natives::{NativeContext, ScriptEffects, ScriptState};
use robin_engine::vm::{self, Instruction};

fn main() {
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).map(|s| s.as_str()).unwrap_or(".");

    let mut results: Vec<(String, &str, usize, usize)> = Vec::new();

    let entries: Vec<_> = std::fs::read_dir(dir)
        .expect("can't read dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("scb"))
        .collect();

    for entry in &entries {
        let path = entry.path();
        let name = path.file_stem().unwrap().to_string_lossy().to_string();

        let scb_file = match scb::parse_file(&path) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("{name:<30} PARSE ERROR: {e}");
                continue;
            }
        };

        // Find a class with an Initialize function
        let mut found = false;
        for class in &scb_file.classes {
            if let Some(func) = class.functions.iter().find(|f| f.name == "Initialize") {
                let instructions: Vec<Instruction> = class
                    .quads
                    .iter()
                    .map(|q| vm::decode(*q).unwrap_or(Instruction::Empty))
                    .collect();

                let mut script_effects = ScriptEffects::new();
                let mut entities = robin_engine::entities::Entities::new();
                let mut ai_global = robin_engine::ai::AiGlobalState::default();
                let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
                let capabilities = robin_engine::natives::NativeSessionCapabilities::new(
                    &mut entities,
                    &mut ai_global,
                    &mut fast_grid,
                );
                let mut script_state = ScriptState::default();
                let mut script_domains = robin_engine::engine::ScriptDomains::default();
                let context = NativeContext::new(
                    &mut script_effects,
                    &mut script_state,
                    &mut script_domains,
                    &capabilities,
                );
                let mut vm_state = Vm::new().with_host(context);
                vm_state
                    .vm
                    .heap
                    .resize(class.size_of_member_variables.max(0) as usize, 0);

                // Set up caller frame
                vm_state.vm.frames[0].temporary.resize(64, 0);
                for _ in 0..func.num_parameters {
                    vm_state
                        .vm
                        .outgoing_params
                        .extend_from_slice(&0i32.to_le_bytes());
                }
                let caller_frame = robin_engine::interp::Frame {
                    parameters: std::mem::take(&mut vm_state.vm.outgoing_params),
                    return_address: instructions.len() as u32,
                    ..Default::default()
                };
                vm_state.vm.frames.push(caller_frame);
                vm_state.vm.ip = func.address as u32;

                let stop = vm_state.run_up_to(&instructions, 500_000);

                let stop_str = match stop {
                    robin_engine::interp::StopReason::Returned => "OK-ret",
                    robin_engine::interp::StopReason::ReturnedValue(_) => "OK-retval",
                    robin_engine::interp::StopReason::RanOff => "OK-ranoff",
                    robin_engine::interp::StopReason::StepLimit => "LIMIT",
                    robin_engine::interp::StopReason::HitEmpty => "EMPTY",
                    robin_engine::interp::StopReason::Unimplemented(_) => "UNIMPL",
                    robin_engine::interp::StopReason::Yield(_) => "YIELD",
                };

                let final_ip = vm_state.vm.ip as usize;
                // Count deferred engine commands as a stand-in for "natives invoked"
                // (the previous host-trace facility is gone).
                let host = vm_state.take_host();

                results.push((
                    format!("{}::{}", class.class_name, func.name),
                    stop_str,
                    host.script_effects().engine_commands().len(),
                    final_ip,
                ));
                found = true;
                break;
            }
        }
        if !found {
            results.push((name, "NO-INIT", 0, 0));
        }
    }

    tracing::info!(
        "{:<40} {:>10} {:>8} {:>8}",
        "SCRIPT",
        "RESULT",
        "NATIVES",
        "IP"
    );
    tracing::info!("{}", "-".repeat(70));
    for (name, result, natives, ip) in &results {
        tracing::info!("{name:<40} {result:>10} {natives:>8} {ip:>8}");
    }
    tracing::info!("{} scripts processed", results.len());
}

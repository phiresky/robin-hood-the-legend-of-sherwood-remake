//! Run a named function from a .scb script with verbose native call tracing.
//!
//!   cargo run --bin run_script -- <path.scb> <class> <function>
//!
//! Example:
//!   cargo run --bin run_script -- datadirs/demo/.../Dem_Lei_MP.scb StartUp Initialize
#![deny(clippy::print_stdout, clippy::print_stderr)]

use robin_rs::interp::Vm;
use robin_rs::natives::GameHost;
use robin_rs::scb;
use robin_rs::vm::{self, Instruction};

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        tracing::error!("usage: {} <path.scb> <class> <function>", args[0]);
        return std::process::ExitCode::from(2);
    }
    let (path, class_name, fn_name) = (&args[1], &args[2], &args[3]);

    let scb_file = match scb::parse_file(path) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("error loading {path}: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let class = scb_file
        .classes
        .iter()
        .find(|c| c.class_name == *class_name)
        .unwrap_or_else(|| {
            tracing::error!("class {class_name:?} not found. available:");
            for c in &scb_file.classes {
                tracing::error!("  {}", c.class_name);
            }
            std::process::exit(1);
        });

    let func = class
        .functions
        .iter()
        .find(|f| f.name == *fn_name)
        .unwrap_or_else(|| {
            tracing::error!("function {fn_name:?} not found in {class_name}. available:");
            for f in &class.functions {
                tracing::error!("  {} (addr={})", f.name, f.address);
            }
            std::process::exit(1);
        });

    let instructions: Vec<Instruction> = class
        .quads
        .iter()
        .map(|q| vm::decode(*q).unwrap_or(Instruction::Empty))
        .collect();

    tracing::info!(
        "Running {class_name}::{fn_name} (addr={}, {} quads total)",
        func.address,
        instructions.len()
    );

    let host = GameHost::new().verbose();
    let mut vm_state = Vm::new().with_host(host);
    vm_state
        .vm
        .heap
        .resize(class.size_of_member_variables.max(0) as usize, 0);

    // Set up a caller frame and jump to the function
    vm_state.vm.frames[0].temporary.resize(64, 0);
    // Push dummy params (self + possible args)
    for _ in 0..func.num_parameters {
        vm_state
            .vm
            .outgoing_params
            .extend_from_slice(&0i32.to_le_bytes());
    }
    let caller_frame = robin_rs::interp::Frame {
        parameters: std::mem::take(&mut vm_state.vm.outgoing_params),
        return_address: instructions.len() as u32,
        ..Default::default()
    };
    vm_state.vm.frames.push(caller_frame);
    vm_state.vm.ip = func.address as u32;

    let stop = vm_state.run_up_to(&instructions, 500_000);

    tracing::info!("--- Result ---");
    tracing::info!("stop reason: {stop:?}");
    tracing::info!("ip: {}", vm_state.vm.ip);
    tracing::info!("frames depth: {}", vm_state.vm.frames.len());

    // Recover host and print summary
    let host = vm_state.take_host();

    tracing::info!("--- {} deferred engine commands ---", host.commands.len());

    if !host.globals.is_empty() {
        tracing::info!("--- Globals ---");
        let mut globals: Vec<_> = host.globals.iter().collect();
        globals.sort_by_key(|(k, _)| *k);
        for (id, val) in globals {
            tracing::info!("  [{id}] = {val}");
        }
    }

    std::process::ExitCode::SUCCESS
}

//! Run a named function from a .scb script with tracing enabled.
//!
//! cargo run --example run_script -- <path.scb> <class> <function>
#![deny(clippy::print_stdout, clippy::print_stderr)]

use robin_assets::scb;
use robin_engine::interp::StopReason;
use robin_engine::natives::{NativeContext, ScriptEffects, ScriptState};
use robin_engine::script_manager::{ScriptError, ScriptManager, ScriptProgram};
use std::sync::Arc;

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    path: String,
    class: String,
    function: String,
}

fn run_script(
    file: scb::ScbFile,
    class: &str,
    function: &str,
) -> Result<(StopReason, u32, usize), String> {
    // Validate the entire file, including classes other than the requested one.
    let program = ScriptProgram::from_scb(file).map_err(|error| error.to_string())?;
    let mut manager = ScriptManager::from_program(Arc::new(program));
    let mut instance = manager
        .create_instance(class)
        .map_err(|error| error.to_string())?;
    let metadata = manager.scb().classes[instance.class_idx()]
        .functions
        .iter()
        .find(|entry| entry.name == function)
        .ok_or_else(|| ScriptError::FunctionNotFound(function.to_owned()).to_string())?;
    let count = usize::try_from(metadata.num_parameters)
        .map_err(|_| format!("{class}::{function}: negative parameter count"))?;
    let mut activation = instance
        .begin_activation(&manager, function, &vec![0; count])
        .map_err(|error| error.to_string())?;
    let mut script_effects = ScriptEffects::new();
    let mut entities = robin_engine::entities::Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let simulation = robin_engine::sim_rng::SimulationContext::with_seed(0);
    let mut native_globals = Vec::new();
    let capabilities = robin_engine::natives::NativeSessionCapabilities::new(
        &simulation,
        &mut entities,
        &mut ai_global,
        &mut fast_grid,
        &mut native_globals,
    );
    let mut script_state = ScriptState::default();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut context = NativeContext::new(
        &mut script_effects,
        &mut script_state,
        &mut script_domains,
        &capabilities,
    );

    let stop = instance.poll_activation_with_host(
        &mut manager,
        &mut activation,
        500_000,
        function,
        &mut context,
    );
    tracing::info!("stop reason: {stop:?}");
    tracing::info!("ip: {}", activation.ip);
    tracing::info!("frames depth: {}", activation.frames.len());
    tracing::info!(
        "--- {} deferred engine commands ---",
        context.engine_commands().len()
    );
    for (id, val) in context.script_globals().iter().enumerate() {
        tracing::info!("  [{id}] = {val}");
    }
    Ok((stop, activation.ip, activation.frames.len()))
}

fn run_file(path: &std::path::Path, class: &str, function: &str) -> Result<(), String> {
    let result = scb::parse_file(path)
        .map_err(|error| error.to_string())
        .and_then(|file| run_script(file, class, function).map(|_| ()));
    result.map_err(|error| format!("{}: {class}::{function}: {error}", path.display()))
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt::init();
    let args = <Args as clap::Parser>::parse();
    match run_file(
        std::path::Path::new(&args.path),
        &args.class,
        &args.function,
    ) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(opcode: u8) -> scb::ScbFile {
        scb::ScbFile {
            version: scb::SCB_VERSION,
            classes: vec![scb::ClassEntry {
                source_file: "fixture.scs".into(),
                class_name: "Probe".into(),
                size_of_member_variables: 4,
                member_variables: vec![],
                functions: vec![scb::Function {
                    name: "Initialize".into(),
                    address: 0,
                    num_parameters: 2,
                    size_of_return_value: 0,
                    size_of_parameters: 8,
                    size_of_volatile: 0,
                    size_of_temporary: 0,
                }],
                quads: vec![scb::Quad {
                    operation: opcode,
                    operands: [0; 8],
                }],
            }],
        }
    }

    #[test]
    fn checked_ingestion_reports_malformed_opcode_with_class_and_address() {
        let error = run_script(fixture(255), "Probe", "Initialize").unwrap_err();
        assert!(error.contains("0xff"), "{error}");
        assert!(error.contains("Probe"), "{error}");
        assert!(error.contains("instruction 0"), "{error}");
        let mut whole_file = fixture(58);
        let mut corrupt = fixture(255).classes.remove(0);
        corrupt.class_name = "Other".into();
        whole_file.classes.push(corrupt);
        assert!(
            run_script(whole_file, "Probe", "Initialize")
                .unwrap_err()
                .contains("Other")
        );
    }

    #[test]
    fn known_empty_workaround_remains_an_explicit_stop() {
        for opcode in [0, 58, 107, 208, 229] {
            let (stop, ip, frames) = run_script(fixture(opcode), "Probe", "Initialize").unwrap();
            assert!(matches!(stop, StopReason::HitEmpty));
            assert!(ip <= 1);
            assert_eq!(
                frames, 1,
                "canonical activation has no fabricated caller frame"
            );
        }
    }

    #[test]
    fn missing_class_function_and_file_are_errors() {
        assert!(
            run_script(fixture(58), "Missing", "Initialize")
                .unwrap_err()
                .contains("class not found")
        );
        assert!(
            run_script(fixture(58), "Probe", "Missing")
                .unwrap_err()
                .contains("function not found")
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.scb");
        let error = run_file(&path, "Probe", "Initialize").unwrap_err();
        assert!(error.contains(path.to_str().unwrap()), "{error}");
    }
}

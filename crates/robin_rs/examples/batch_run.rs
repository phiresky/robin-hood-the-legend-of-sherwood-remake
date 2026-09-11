//! Run Initialize on every .scb in a directory and report results.
//!
//! cargo run --example batch_run -- datadirs/fullgame/Data/Levels
#![deny(clippy::print_stdout, clippy::print_stderr)]

use robin_assets::scb;
use robin_engine::natives::{NativeContext, ScriptEffects, ScriptState};
use robin_engine::script_manager::{ScriptManager, ScriptProgram};
use std::path::Path;
use std::sync::Arc;

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    #[arg(default_value = ".")]
    dir: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ScriptResult {
    script: String,
    status: String,
    natives: usize,
    ip: u32,
    frames: usize,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct BatchReport {
    results: Vec<ScriptResult>,
    errors: Vec<String>,
}

impl BatchReport {
    fn exit_code(&self) -> std::process::ExitCode {
        if self.errors.is_empty() {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_initialize(file: scb::ScbFile) -> Result<Option<ScriptResult>, String> {
    // Validate every class even when none provides Initialize.
    let program = ScriptProgram::from_scb(file).map_err(|error| error.to_string())?;
    let mut manager = ScriptManager::from_program(Arc::new(program));
    let Some(class) = manager.scb().classes.iter().find(|class| {
        class
            .functions
            .iter()
            .any(|function| function.name == "Initialize")
    }) else {
        return Ok(None);
    };
    let class_name = class.class_name.clone();
    let count = class
        .functions
        .iter()
        .find(|function| function.name == "Initialize")
        .expect("selected class has Initialize")
        .num_parameters;
    let count = usize::try_from(count)
        .map_err(|_| format!("{class_name}::Initialize: negative parameter count"))?;
    let mut instance = manager
        .create_instance(&class_name)
        .map_err(|error| error.to_string())?;
    let mut activation = instance
        .begin_activation(&manager, "Initialize", &vec![0; count])
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
        "Initialize",
        &mut context,
    );
    Ok(Some(ScriptResult {
        script: format!("{class_name}::Initialize"),
        // Report the actual stop. Yield, instruction budget, authored Empty,
        // and a nested return are not claims of completed initialization.
        status: format!("{stop:?}"),
        natives: context.engine_commands().len(),
        ip: activation.ip,
        frames: activation.frames.len(),
    }))
}

fn run_directory(directory: &Path) -> Result<BatchReport, String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("{}: read directory: {error}", directory.display()))?;
    let mut report = BatchReport::default();
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("scb") {
                    paths.push(path);
                }
            }
            Err(error) => report.errors.push(format!(
                "{}: enumerate directory: {error}",
                directory.display()
            )),
        }
    }
    paths.sort();
    for path in paths {
        let outcome = scb::parse_file(&path)
            .map_err(|error| error.to_string())
            .and_then(run_initialize);
        match outcome {
            Ok(Some(mut result)) => {
                result.script = format!("{}: {}", path.display(), result.script);
                report.results.push(result);
            }
            Ok(None) => report.results.push(ScriptResult {
                script: path.display().to_string(),
                status: "NO-INIT".into(),
                natives: 0,
                ip: 0,
                frames: 0,
            }),
            Err(error) => {
                let error = format!("{}: {error}", path.display());
                report.results.push(ScriptResult {
                    script: path.display().to_string(),
                    status: format!("ERROR: {error}"),
                    natives: 0,
                    ip: 0,
                    frames: 0,
                });
                report.errors.push(error);
            }
        }
    }
    Ok(report)
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt::init();
    let args = <Args as clap::Parser>::parse();
    let report = match run_directory(Path::new(&args.dir)) {
        Ok(report) => report,
        Err(error) => {
            tracing::error!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    for result in &report.results {
        tracing::info!(
            "{}: {} (deferred commands: {}, ip: {}, frames: {})",
            result.script,
            result.status,
            result.natives,
            result.ip,
            result.frames
        );
    }
    for error in &report.errors {
        tracing::error!("{error}");
    }
    tracing::info!(
        "{} scripts processed, {} errors",
        report.results.len(),
        report.errors.len()
    );
    report.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal real SCB encoding exercises the same directory/parser/program
    // path as the command-line tool; this is not a replacement interpreter.
    fn script_bytes(opcode: u8, initialize: bool) -> Vec<u8> {
        let mut bytes = scb::SCB_MAGIC.to_vec();
        bytes.extend_from_slice(&scb::SCB_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for text in ["fixture.scs", "Probe"] {
            bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
        bytes.extend_from_slice(&0i32.to_le_bytes()); // members
        bytes.extend_from_slice(&0i32.to_le_bytes()); // heap size
        bytes.extend_from_slice(&i32::from(initialize).to_le_bytes());
        if initialize {
            bytes.extend_from_slice(&10u32.to_le_bytes());
            bytes.extend_from_slice(b"Initialize");
            for _ in 0..6 {
                bytes.extend_from_slice(&0i32.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.push(opcode);
        bytes.extend_from_slice(&[0; 8]);
        bytes
    }

    #[test]
    fn missing_directory_is_an_explicit_error() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing");
        let error = run_directory(&missing).unwrap_err();
        assert!(error.contains("read directory"), "{error}");
        assert!(error.contains(missing.to_str().unwrap()), "{error}");
    }

    #[test]
    fn mixed_valid_and_corrupt_scripts_are_all_reported_and_fail_the_batch() {
        let directory = tempfile::tempdir().unwrap();
        for (name, bytes) in [
            ("a-corrupt.scb", script_bytes(255, false)),
            ("b-valid.scb", script_bytes(58, true)),
            ("c-parse.scb", b"not a script".to_vec()),
        ] {
            std::fs::write(directory.path().join(name), bytes).unwrap();
        }
        let report = run_directory(directory.path()).unwrap();
        assert_eq!(report.results.len(), 3);
        assert_eq!(report.errors.len(), 2);
        assert_eq!(report.exit_code(), std::process::ExitCode::FAILURE);
        assert!(report.results[0].status.contains("0xff"));
        assert!(report.results[0].status.contains("Probe"));
        assert!(report.results[0].status.contains("instruction 0"));
        assert_eq!(report.results[1].status, "HitEmpty");
        assert!(report.results[2].status.starts_with("ERROR:"));
    }

    #[test]
    fn only_a_valid_program_without_initialize_is_no_init() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("no-init.scb"),
            script_bytes(58, false),
        )
        .unwrap();
        let report = run_directory(directory.path()).unwrap();
        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].status, "NO-INIT");
        assert_eq!(report.exit_code(), std::process::ExitCode::SUCCESS);
    }
}

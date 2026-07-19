//! Dump a `.res` resource file to JSON.
//!
//!   cargo run --bin dump_res -- path/to/file.res
#![deny(clippy::print_stdout, clippy::print_stderr)]
fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt::init();
    let mut args = std::env::args();
    let prog = args.next().unwrap_or_else(|| "dump_res".into());
    let Some(path) = args.next() else {
        tracing::error!("usage: {prog} <path-to-file.res>");
        return std::process::ExitCode::from(2);
    };

    let mut mgr = robin_assets::resource_manager::ResourceManager::new();
    match mgr.attach_resource_file(&path) {
        Ok(()) => {
            let json = mgr.dump_json();
            let output = serde_json::to_string_pretty(&json).expect("json serialize");
            std::io::Write::write_all(&mut std::io::stdout(), output.as_bytes())
                .expect("write to stdout");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

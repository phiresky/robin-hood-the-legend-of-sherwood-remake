//! Dump a `.res` resource file to JSON.
//!
//!   cargo run -p robin_modding_tools --bin dump_res -- path/to/file.res
#![deny(clippy::print_stdout, clippy::print_stderr)]
#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
#[command(about = "Dump a Robin Hood .res resource archive to JSON")]
struct Args {
    input: String,
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let args = <Args as clap::Parser>::parse();
    let path = &args.input;

    let mut mgr = robin_assets::resource_manager::ResourceManager::legacy_tool();
    match mgr.attach_resource_file(path) {
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

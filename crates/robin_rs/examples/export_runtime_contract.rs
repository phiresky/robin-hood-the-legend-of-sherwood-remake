//! Generate or check packaging metadata using compiled engine constants.
use robin_rs::runtime_contract::RuntimeContract;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(&RuntimeContract::current())?
    );
    match args.as_slice() {
        [] => print!("{serialized}"),
        [mode, path] if mode == "--write" => std::fs::write(path, serialized)?,
        [mode, path] if mode == "--check" => {
            let actual: RuntimeContract = serde_json::from_slice(&std::fs::read(Path::new(path))?)?;
            anyhow::ensure!(
                actual == RuntimeContract::current(),
                "{path} is stale; run export_runtime_contract --write {path}"
            );
        }
        _ => anyhow::bail!("usage: export_runtime_contract [--write PATH | --check PATH]"),
    }
    Ok(())
}

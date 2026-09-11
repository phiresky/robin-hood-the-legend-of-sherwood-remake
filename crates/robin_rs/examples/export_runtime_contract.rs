//! Generate or check packaging metadata using compiled engine constants.
use robin_rs::runtime_contract::RuntimeContract;
use std::path::PathBuf;

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    #[arg(long, conflicts_with = "check")]
    write: Option<PathBuf>,
    #[arg(long)]
    check: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = <Args as clap::Parser>::parse();
    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(&RuntimeContract::current())?
    );
    if let Some(path) = args.write {
        std::fs::write(path, serialized)?;
    } else if let Some(path) = args.check {
        let actual: RuntimeContract = serde_json::from_slice(&std::fs::read(&path)?)?;
        anyhow::ensure!(
            actual == RuntimeContract::current(),
            "{} is stale; run export_runtime_contract --write {}",
            path.display(),
            path.display()
        );
    } else {
        print!("{serialized}");
    }
    Ok(())
}

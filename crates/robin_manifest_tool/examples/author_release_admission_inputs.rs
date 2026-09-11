use anyhow::Result;
use std::path::PathBuf;

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
#[command(about = "Author or validate release-admission inputs")]
struct Args {
    #[arg(value_enum)]
    operation: Operation,
    plan: PathBuf,
    output: PathBuf,
}

#[derive(Clone, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
enum Operation {
    Author,
    Validate,
}

fn main() -> Result<()> {
    let Args {
        operation,
        plan,
        output,
    } = <Args as clap::Parser>::parse();
    let authored = match operation {
        Operation::Author => {
            robin_manifest_tool::release_admission_v1::author_release_admission_v1(&plan, &output)?
        }
        Operation::Validate => {
            robin_manifest_tool::release_admission_v1::validate_release_admission_v1(
                &plan, &output,
            )?
        }
    };
    println!("index_sha256={}", authored.index_sha256);
    println!("rulesets={}", authored.index.rulesets.len());
    println!("competitions={}", authored.index.competitions.len());
    Ok(())
}

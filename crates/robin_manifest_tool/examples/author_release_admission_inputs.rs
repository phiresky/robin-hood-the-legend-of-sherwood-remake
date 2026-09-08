use std::path::PathBuf;

use anyhow::{Context as _, Result, ensure};

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let operation = arguments
        .next()
        .context("usage: author_release_admission_inputs <author|validate> PLAN OUTPUT")?;
    let plan = PathBuf::from(
        arguments
            .next()
            .context("release-admission plan path is absent")?,
    );
    let output = PathBuf::from(
        arguments
            .next()
            .context("release-admission output path is absent")?,
    );
    ensure!(arguments.next().is_none(), "unexpected trailing argument");

    let authored = match operation.to_str() {
        Some("author") => {
            robin_manifest_tool::release_admission_v1::author_release_admission_v1(&plan, &output)?
        }
        Some("validate") => {
            robin_manifest_tool::release_admission_v1::validate_release_admission_v1(
                &plan, &output,
            )?
        }
        _ => anyhow::bail!(
            "unknown operation; usage: author_release_admission_inputs <author|validate> PLAN OUTPUT"
        ),
    };
    println!("index_sha256={}", authored.index_sha256);
    println!("rulesets={}", authored.index.rulesets.len());
    println!("competitions={}", authored.index.competitions.len());
    Ok(())
}

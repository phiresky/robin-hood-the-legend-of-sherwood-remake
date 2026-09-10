//! Print the canonical native Data/locale closure SHA-256 used by browser
//! multiplayer tickets and static Demo/Full catalogs.

#![allow(clippy::print_stdout)]

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    data_dir: std::path::PathBuf,
}

fn main() -> anyhow::Result<()> {
    let data_dir = <Args as clap::Parser>::parse().data_dir;
    let identity = robin_rs::multiplayer::content_identity::source_content_identity(
        std::path::Path::new(&data_dir),
    )
    .map_err(anyhow::Error::msg)?;
    println!("{identity}");
    Ok(())
}

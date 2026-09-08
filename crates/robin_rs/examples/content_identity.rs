//! Print the canonical native Data/locale closure SHA-256 used by browser
//! multiplayer tickets and static Demo/Full catalogs.

#![allow(clippy::print_stdout)]

fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args_os();
    let executable = arguments.next().unwrap_or_default();
    let data_dir = arguments.next().ok_or_else(|| {
        anyhow::anyhow!(
            "usage: {} <installation-Data-directory>",
            std::path::Path::new(&executable).display()
        )
    })?;
    if arguments.next().is_some() {
        anyhow::bail!("content_identity accepts exactly one Data directory");
    }
    let identity = robin_rs::multiplayer::content_identity::source_content_identity(
        std::path::Path::new(&data_dir),
    )
    .map_err(anyhow::Error::msg)?;
    println!("{identity}");
    Ok(())
}

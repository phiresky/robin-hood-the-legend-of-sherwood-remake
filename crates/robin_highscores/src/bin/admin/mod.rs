//! Operator commands: migrations, snapshots, key bootstrap and moderation.

mod cli;

pub(super) async fn run() -> anyhow::Result<()> {
    cli::run().await
}

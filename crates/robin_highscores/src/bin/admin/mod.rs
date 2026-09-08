//! Administrative authority boundaries. The binary only invokes typed dispatch.

mod capacity;
mod cleanup;
mod cli;
mod execution;
mod filesystem;
mod key_activation;
mod policy;
mod sources;
mod verification;

#[cfg(test)]
mod fixtures;

pub(super) async fn run() -> anyhow::Result<()> {
    cli::run().await
}

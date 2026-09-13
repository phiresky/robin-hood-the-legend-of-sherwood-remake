#![forbid(unsafe_code)]

#[cfg(not(target_os = "linux"))]
compile_error!("robin-highscores-admin is Linux-only");

#[path = "admin/mod.rs"]
mod admin;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    admin::run().await
}

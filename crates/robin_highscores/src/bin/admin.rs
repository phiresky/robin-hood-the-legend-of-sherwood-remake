#[path = "admin/mod.rs"]
mod admin;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    admin::run().await
}

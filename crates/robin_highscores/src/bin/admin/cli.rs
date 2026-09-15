//! Typed command parsing and dispatch. Authority stays with the called owner.

use clap::Parser;
use clap::Subcommand;
use robin_highscores::Database;
use robin_highscores::ServerConfig;
use std::io::Read as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(about = "Explicit high-score database and moderation administration")]
struct Arguments {
    #[arg(
        long,
        env = "ROBIN_HIGHSCORES_CONFIG",
        default_value = "highscores-server.toml"
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Apply all reviewed SQL migrations. Serving processes never do this.
    Migrate,
    /// Write a consistent copy of the database (`VACUUM INTO`) to a new path.
    SnapshotDb {
        /// Destination file; must not already exist.
        path: PathBuf,
    },
    /// Print the highest migration version applied to the configured database.
    DatabaseSchemaVersion,
    /// Print the database schema version this binary requires. Config-free.
    SupportedSchemaVersion,
    /// Create the durable cursor key without printing or otherwise exposing it.
    InitializeCursorKey,
    Reports {
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    Moderate {
        report_id: String,
        #[arg(long)]
        state: String,
        #[arg(long)]
        detail: String,
    },
    Audit {
        #[arg(long)]
        report_id: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
}

pub(super) async fn run() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    match arguments.command {
        Command::SupportedSchemaVersion => {
            println!("{}", robin_highscores::db::CURRENT_SCHEMA_VERSION);
        }
        Command::InitializeCursorKey => {
            let config = load_secret_bootstrap_config(&arguments.config, "cursor_secret_path")?;
            config.load_or_create_cursor_key()?;
            println!("cursor key is initialized");
        }
        Command::SnapshotDb { path } => {
            let config = ServerConfig::load(&arguments.config)?;
            robin_highscores::db::snapshot_database(
                &config.database_path,
                &path,
                config.database_busy_timeout_ms,
            )
            .await?;
            println!("database snapshot written to {}", path.display());
        }
        Command::DatabaseSchemaVersion => {
            let config = ServerConfig::load(&arguments.config)?;
            let version = robin_highscores::db::applied_schema_version(
                &config.database_path,
                config.database_busy_timeout_ms,
            )
            .await?;
            println!("{version}");
        }
        Command::Migrate => {
            let config = ServerConfig::load(&arguments.config)?;
            let database = Database::migrate(&config).await?;
            database.close().await;
            println!("database migrations applied successfully");
        }
        Command::Reports { state, limit } => {
            let config = ServerConfig::load(&arguments.config)?;
            let db = Database::connect(&config).await?;
            let reports = db.moderation_reports(state.as_deref(), limit).await;
            db.close().await;
            println!("{}", serde_json::to_string_pretty(&reports?)?);
        }
        Command::Moderate {
            report_id,
            state,
            detail,
        } => {
            let config = ServerConfig::load(&arguments.config)?;
            let db = Database::connect(&config).await?;
            let result: anyhow::Result<()> = async {
                let lease = db
                    .acquire_maintenance_write_lease(
                        robin_highscores::db::MaintenanceWriteClass::Admin,
                        "robin-highscores-admin-moderation",
                        Duration::from_secs(5 * 60),
                    )
                    .await?;
                let mutation = db
                    .moderate_report(&report_id, &state, &detail, &config.moderation_operator_id)
                    .await;
                let release = db.release_maintenance_write_lease(&lease).await;
                match (mutation, release) {
                    (Ok(()), Ok(true)) => Ok(()),
                    (Ok(()), Ok(false)) => {
                        anyhow::bail!("moderation write lease disappeared before release")
                    }
                    (Ok(()), Err(error)) => Err(error.into()),
                    (Err(operation), Ok(true)) => Err(operation.into()),
                    (Err(operation), Ok(false)) => Err(anyhow::Error::from(operation)
                        .context("moderation failed and its write lease disappeared")),
                    (Err(operation), Err(release)) => {
                        Err(anyhow::Error::from(operation).context(format!(
                            "moderation failed and releasing its write lease also failed: {release}"
                        )))
                    }
                }
            }
            .await;
            db.close().await;
            result?;
            println!("moderation action recorded");
        }
        Command::Audit { report_id, limit } => {
            let config = ServerConfig::load(&arguments.config)?;
            let db = Database::connect(&config).await?;
            let audit = db.moderation_audit(report_id.as_deref(), limit).await;
            db.close().await;
            println!("{}", serde_json::to_string_pretty(&audit?)?);
        }
    }
    Ok(())
}

/// Read only one secret path from a possibly incomplete bootstrap config, so a
/// fresh host can create its secrets before the final configuration exists.
fn load_secret_bootstrap_config(path: &Path, field: &str) -> anyhow::Result<ServerConfig> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= 4 * 1024 * 1024,
        "bootstrap config must be a bounded regular file"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "bootstrap config is too large"
    );
    let value: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
    let secret_path = PathBuf::from(
        value
            .get(field)
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("bootstrap config omits {field}"))?,
    );
    anyhow::ensure!(
        secret_path.is_absolute(),
        "bootstrap secret path is not absolute"
    );
    let mut config = ServerConfig::default();
    match field {
        "cursor_secret_path" => config.cursor_secret_path = secret_path,
        _ => anyhow::bail!("unsupported bootstrap secret field"),
    }
    Ok(config)
}

#[cfg(test)]
mod tests;

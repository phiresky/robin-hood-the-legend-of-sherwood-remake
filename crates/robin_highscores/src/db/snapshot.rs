//! Operator snapshot and schema probes used by `ops/deploy.sh`, `ops/rollback.sh`
//! and `ops/backup.sh`. Both open the database read-only and deliberately skip
//! the "schema is current" check: a deploy must be able to snapshot and inspect
//! a database whose schema is older or newer than this binary supports.

use super::DbError;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row as _, SqlitePool};
use std::path::Path;
use std::time::Duration;

async fn open_read_only(database_path: &Path, busy_timeout_ms: u64) -> Result<SqlitePool, DbError> {
    let options = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(false)
        .read_only(true)
        .busy_timeout(Duration::from_millis(busy_timeout_ms));
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?)
}

/// Highest successfully applied migration version, or 0 for an unmigrated file.
pub async fn applied_schema_version(
    database_path: &Path,
    busy_timeout_ms: u64,
) -> Result<i64, DbError> {
    let pool = open_read_only(database_path, busy_timeout_ms).await?;
    let result = sqlx::query(
        "SELECT COALESCE(MAX(version), 0) AS version FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&pool)
    .await;
    pool.close().await;
    match result {
        Ok(row) => Ok(row.try_get::<i64, _>("version")?),
        Err(sqlx::Error::Database(database)) if database.message().contains("no such table") => {
            Ok(0)
        }
        Err(error) => Err(error.into()),
    }
}

/// Write a transactionally consistent copy of the database with `VACUUM INTO`.
/// Live writers may continue; the copy reflects one committed read snapshot.
/// Refuses to overwrite anything already present at `destination`.
pub async fn snapshot_database(
    database_path: &Path,
    destination: &Path,
    busy_timeout_ms: u64,
) -> Result<(), DbError> {
    match tokio::fs::symlink_metadata(destination).await {
        Ok(_) => {
            return Err(DbError::Corrupt(format!(
                "snapshot destination already exists: {}",
                destination.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(sqlx::Error::Io(error).into()),
    }
    let destination_text = destination.to_str().ok_or_else(|| {
        DbError::Corrupt("snapshot destination path is not valid UTF-8".to_owned())
    })?;
    let pool = open_read_only(database_path, busy_timeout_ms).await?;
    let result = sqlx::query("VACUUM INTO ?")
        .bind(destination_text)
        .execute(&pool)
        .await;
    pool.close().await;
    result?;
    {
        use std::os::unix::fs::PermissionsExt as _;
        tokio::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(sqlx::Error::Io)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Database, ServerConfig};

    #[tokio::test]
    async fn snapshot_copies_a_migrated_database_and_refuses_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            database_path: directory.path().join("db/highscores.sqlite3"),
            ..Default::default()
        };
        let database = Database::migrate(&config).await.unwrap();
        database.close().await;

        assert_eq!(
            applied_schema_version(&config.database_path, 1_000)
                .await
                .unwrap(),
            super::super::CURRENT_SCHEMA_VERSION
        );

        let destination = directory.path().join("snapshot.sqlite3");
        snapshot_database(&config.database_path, &destination, 1_000)
            .await
            .unwrap();
        assert_eq!(
            applied_schema_version(&destination, 1_000).await.unwrap(),
            super::super::CURRENT_SCHEMA_VERSION
        );
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let before = std::fs::read(&destination).unwrap();
        let error = snapshot_database(&config.database_path, &destination, 1_000)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error}");
        assert_eq!(std::fs::read(&destination).unwrap(), before);

        let dangling = directory.path().join("dangling.sqlite3");
        std::os::unix::fs::symlink(directory.path().join("missing"), &dangling).unwrap();
        assert!(
            snapshot_database(&config.database_path, &dangling, 1_000)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn unmigrated_database_reports_schema_zero() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty.sqlite3");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE unrelated (id INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        assert_eq!(applied_schema_version(&path, 1_000).await.unwrap(), 0);
    }
}

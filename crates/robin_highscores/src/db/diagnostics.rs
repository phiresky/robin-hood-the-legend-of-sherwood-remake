use super::*;
use robin_run_protocol::diagnostics::{DiagnosticReceiptV1, MAX_DIAGNOSTIC_BODY_BYTES};
use sha2::{Digest as _, Sha256};

#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticSummary {
    pub report_id: String,
    pub received_at_unix_ms: i64,
    pub kind: String,
    pub engine_commit: String,
}

/// An opaque attachment. Neither ingestion nor download decodes its contents.
#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticPayload {
    pub bytes: Vec<u8>,
    pub encoding: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::diagnostics::DiagnosticReportV1;
    use sqlx::Connection as _;

    #[tokio::test]
    async fn compressed_diagnostics_migration_preserves_legacy_reports() {
        let mut connection = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/0003_diagnostics.sql"))
            .execute(&mut connection)
            .await
            .unwrap();
        let json = r#"{"kind":"bug","engine_commit":"legacy","description":"stuck"}"#;
        sqlx::query("INSERT INTO diagnostic_reports VALUES (?, ?, ?, ?, ?)")
            .bind("legacy-id")
            .bind(1_i64)
            .bind([1_u8; 32].as_slice())
            .bind(json)
            .bind(json.len() as i64)
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../migrations/0004_compressed_diagnostics.sql"
        ))
        .execute(&mut connection)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/0005_larger_diagnostics.sql"))
            .execute(&mut connection)
            .await
            .unwrap();
        let row = sqlx::query("SELECT payload, payload_bytes, encoding, kind, engine_commit FROM diagnostic_reports WHERE id = 'legacy-id'").fetch_one(&mut connection).await.unwrap();
        assert_eq!(row.get::<Vec<u8>, _>("payload"), json.as_bytes());
        assert_eq!(row.get::<i64, _>("payload_bytes"), json.len() as i64);
        assert_eq!(row.get::<String, _>("encoding"), "identity");
        assert_eq!(row.get::<String, _>("kind"), "bug");
        assert_eq!(row.get::<String, _>("engine_commit"), "legacy");
    }

    #[tokio::test]
    async fn storage_accounts_for_compressed_bytes() {
        let root = tempfile::tempdir().unwrap();
        let database = Database::migrate(&ServerConfig {
            database_path: root.path().join("reports.sqlite3"),
            ..Default::default()
        })
        .await
        .unwrap();
        let report = DiagnosticReportV1 {
            schema_version: 1,
            kind: robin_run_protocol::diagnostics::DiagnosticKindV1::Bug,
            description: "stuck".into(),
            engine_commit: "test".into(),
            platform: "test".into(),
            occurred_at_unix_ms: 0,
            backtrace: None,
            recent_log: "x".repeat(24 * 1024 * 1024),
            attachments: vec![],
            warnings: vec![],
        };
        let compressed =
            robin_run_protocol::diagnostics::compress_report(&serde_json::to_vec(&report).unwrap())
                .unwrap();
        let expected_size = compressed.len();
        let receipt = database
            .insert_diagnostic(compressed.clone(), "zstd", "bug", "test", [1; 32])
            .await
            .unwrap();
        let row = sqlx::query("SELECT payload, payload_bytes FROM diagnostic_reports WHERE id = ?")
            .bind(&receipt.report_id)
            .fetch_one(&database.pool)
            .await
            .unwrap();
        assert_eq!(row.get::<Vec<u8>, _>("payload"), compressed);
        assert_eq!(row.get::<i64, _>("payload_bytes"), expected_size as i64);
        assert_eq!(
            database
                .diagnostic_report(&receipt.report_id)
                .await
                .unwrap()
                .bytes,
            compressed
        );
        // Opaque captures are preserved without attempting decompression,
        // including artifacts exceeding the former 20 MiB upload limit.
        let opaque = vec![42; 21 * 1024 * 1024];
        let receipt = database
            .insert_diagnostic(opaque.clone(), "zstd", "panic", "test", [1; 32])
            .await
            .unwrap();
        assert_eq!(
            database
                .diagnostic_report(&receipt.report_id)
                .await
                .unwrap()
                .bytes,
            opaque
        );
        assert!(matches!(
            database
                .insert_diagnostic(vec![1], "zstd", "", "test", [1; 32])
                .await,
            Err(DbError::ResultInvariant(_))
        ));
    }
}
impl Database {
    pub async fn insert_diagnostic(
        &self,
        payload: Vec<u8>,
        encoding: &'static str,
        kind: &str,
        engine_commit: &str,
        ip_hash: [u8; 32],
    ) -> Result<DiagnosticReceiptV1, DbError> {
        if payload.is_empty() || payload.len() > MAX_DIAGNOSTIC_BODY_BYTES {
            return Err(DbError::ResultInvariant(
                "invalid compressed report size".into(),
            ));
        }
        if !matches!(encoding, "zstd" | "gzip") || !matches!(kind, "bug" | "panic" | "fatal_error")
        {
            return Err(DbError::ResultInvariant(
                "invalid diagnostic metadata".into(),
            ));
        }
        if engine_commit.is_empty()
            || engine_commit.len() > 128
            || !engine_commit
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(DbError::ResultInvariant(
                "invalid diagnostic build identifier".into(),
            ));
        }
        // Preserve the exact uploaded artifact, including incomplete/crashed
        // captures. Index only the small metadata explicitly supplied by clients.
        let (id, payload) =
            tokio::task::spawn_blocking(move || (hex::encode(Sha256::digest(&payload)), payload))
                .await
                .map_err(|e| DbError::Corrupt(e.to_string()))?;
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM diagnostic_reports WHERE received_at_ms < ?")
            .bind(now.saturating_sub(30 * 24 * 60 * 60 * 1000))
            .execute(&mut *tx)
            .await?;
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM diagnostic_reports WHERE id = ?")
                .bind(&id)
                .fetch_one(&mut *tx)
                .await?;
        if exists == 0 {
            let hour = now.saturating_sub(60 * 60 * 1000);
            let global: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM diagnostic_reports WHERE received_at_ms >= ?",
            )
            .bind(hour)
            .fetch_one(&mut *tx)
            .await?;
            let per_ip: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM diagnostic_reports WHERE received_at_ms >= ? AND reporter_ip_hash = ?").bind(hour).bind(ip_hash.as_slice()).fetch_one(&mut *tx).await?;
            let bytes: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(payload_bytes), 0) FROM diagnostic_reports",
            )
            .fetch_one(&mut *tx)
            .await?;
            if global >= 100 || per_ip >= 10 || bytes + payload.len() as i64 > 512 * 1024 * 1024 {
                return Err(DbError::QueueFull);
            }
            sqlx::query("INSERT INTO diagnostic_reports (id, received_at_ms, reporter_ip_hash, payload, payload_bytes, encoding, kind, engine_commit) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(&id).bind(now).bind(ip_hash.as_slice()).bind(&payload).bind(payload.len() as i64).bind(encoding).bind(kind).bind(&engine_commit).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(DiagnosticReceiptV1 {
            schema_version: 1,
            report_id: id,
        })
    }
    pub async fn diagnostic_reports(&self) -> Result<Vec<DiagnosticSummary>, DbError> {
        let rows = sqlx::query("SELECT id, received_at_ms, kind, engine_commit FROM diagnostic_reports ORDER BY received_at_ms DESC, id DESC LIMIT 100").fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok(DiagnosticSummary {
                    report_id: row.try_get("id")?,
                    received_at_unix_ms: row.try_get("received_at_ms")?,
                    kind: row.try_get("kind")?,
                    engine_commit: row.try_get("engine_commit")?,
                })
            })
            .collect()
    }
    pub async fn diagnostic_report(&self, id: &str) -> Result<DiagnosticPayload, DbError> {
        let row = sqlx::query("SELECT payload, encoding FROM diagnostic_reports WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        Ok(DiagnosticPayload {
            bytes: row.try_get("payload")?,
            encoding: row.try_get("encoding")?,
        })
    }

    pub async fn delete_diagnostic_report(&self, id: &str) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM diagnostic_reports WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound);
        }
        Ok(())
    }
}

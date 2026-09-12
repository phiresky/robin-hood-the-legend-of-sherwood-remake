use super::*;
use robin_run_protocol::diagnostics::{
    DiagnosticReceiptV1, DiagnosticReportV1, MAX_DIAGNOSTIC_BODY_BYTES,
};
use sha2::{Digest as _, Sha256};

#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticSummary {
    pub report_id: String,
    pub received_at_unix_ms: i64,
    pub kind: String,
    pub engine_commit: String,
}
impl Database {
    pub async fn insert_diagnostic(
        &self,
        report: &DiagnosticReportV1,
        ip_hash: [u8; 32],
    ) -> Result<DiagnosticReceiptV1, DbError> {
        report
            .validate()
            .map_err(|e| DbError::ResultInvariant(e.into()))?;
        let payload =
            serde_json::to_string(report).map_err(|e| DbError::ResultInvariant(e.to_string()))?;
        if payload.len() > MAX_DIAGNOSTIC_BODY_BYTES {
            return Err(DbError::ResultInvariant("diagnostic body too large".into()));
        }
        let id = hex::encode(Sha256::digest(payload.as_bytes()));
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
            sqlx::query("INSERT INTO diagnostic_reports (id, received_at_ms, reporter_ip_hash, payload, payload_bytes) VALUES (?, ?, ?, ?, ?)")
                .bind(&id).bind(now).bind(ip_hash.as_slice()).bind(&payload).bind(payload.len() as i64).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(DiagnosticReceiptV1 {
            schema_version: 1,
            report_id: id,
        })
    }
    pub async fn diagnostic_reports(&self) -> Result<Vec<DiagnosticSummary>, DbError> {
        let rows = sqlx::query("SELECT id, received_at_ms, json_extract(payload, '$.kind') AS kind, json_extract(payload, '$.engine_commit') AS engine_commit FROM diagnostic_reports ORDER BY received_at_ms DESC, id DESC LIMIT 100").fetch_all(&self.pool).await?;
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
    pub async fn diagnostic_report(&self, id: &str) -> Result<DiagnosticReportV1, DbError> {
        let payload: String =
            sqlx::query_scalar("SELECT payload FROM diagnostic_reports WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(DbError::NotFound)?;
        serde_json::from_str(&payload).map_err(|e| DbError::Corrupt(e.to_string()))
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

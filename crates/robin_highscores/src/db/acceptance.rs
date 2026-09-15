//! Publication of one verifier-verified run in a single write transaction.
use super::*;
use robin_run_protocol::{
    BoardMetricV1, BoardV2, InputProvenanceStatusV1, TerminalOutcomeV1, Validate as _,
    VerificationStatusV2, VerifierOutputV2,
};

impl Database {
    /// Publish the verified result for a leased submission and return the new
    /// run ID. `job_sha256` is the digest of the exact job document the worker
    /// handed to the verifier; `board` is the board the job was built from.
    pub async fn accept_job(
        &self,
        submission_id: &str,
        worker_id: &str,
        job_sha256: Digest32,
        board: &BoardV2,
        output: &VerifierOutputV2,
    ) -> Result<String, DbError> {
        output
            .validate()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let VerificationStatusV2::Verified(run) = &output.status else {
            return Err(DbError::ResultInvariant(
                "only a typed verified result can enter the acceptance path".to_owned(),
            ));
        };
        if output.job_sha256 != job_sha256 {
            return Err(DbError::ResultInvariant(
                "verifier result does not bind the dispatched job".to_owned(),
            ));
        }
        let provenance = output
            .input_provenance
            .as_ref()
            .ok_or_else(|| DbError::ResultInvariant("verified result has no provenance".into()))?;
        if !matches!(provenance, InputProvenanceStatusV1::Rankable) {
            return Err(DbError::ResultInvariant(
                "only rankable provenance can enter ranked storage".to_owned(),
            ));
        }
        if run.outcome != TerminalOutcomeV1::Won {
            return Err(DbError::ResultInvariant(
                "only verifier-derived successful terminal runs can rank".to_owned(),
            ));
        }
        validate_ranked_score(
            run.starting_campaign_score,
            run.final_campaign_score,
            run.original_score_delta,
        )?;
        let integer = |value: u64, message: &str| {
            i64::try_from(value).map_err(|_| DbError::ResultInvariant(message.to_owned()))
        };
        let active_simulation_ticks = integer(
            run.active_simulation_ticks,
            "simulation tick count does not fit SQLite INTEGER",
        )?;
        let ransom_collected = integer(
            run.ransom_collected,
            "ransom amount does not fit SQLite INTEGER",
        )?;
        let sim_config_json = canonical_json_string(&run.sim_config)?;
        let input_provenance_json = canonical_json_string(provenance)?;

        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        ensure_worker_lease(&mut tx, submission_id, worker_id, now).await?;
        let submitted = sqlx::query(
            "SELECT board_id, mission_id, replay_sha256, requested_metrics_json \
             FROM submissions WHERE id = ? AND tombstoned_at_ms IS NULL",
        )
        .bind(submission_id)
        .fetch_one(&mut *tx)
        .await?;
        let mission_id: String = submitted.try_get("mission_id")?;
        if submitted.try_get::<String, _>("board_id")? != board.board_id.as_str()
            || board.mission(&mission_id).is_none()
        {
            return Err(DbError::ResultInvariant(
                "verification board no longer admits the submitted mission".to_owned(),
            ));
        }
        if digest_from_row(&submitted, "replay_sha256")? != output.replay_sha256 {
            return Err(DbError::ResultInvariant(
                "verifier read a different replay than the stored submission".to_owned(),
            ));
        }
        let requested: Vec<BoardMetricV1> =
            serde_json::from_str(submitted.try_get("requested_metrics_json")?)
                .map_err(|error| DbError::Corrupt(format!("requested metrics JSON: {error}")))?;
        if requested.is_empty()
            || requested
                .iter()
                .any(|metric| !board.metrics.contains(metric))
        {
            return Err(DbError::ResultInvariant(
                "requested metrics are no longer offered by the board".to_owned(),
            ));
        }

        let run_id = uuid::Uuid::now_v7().to_string();
        let accepted_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO acceptance_sequences (created_at_ms) VALUES (?) RETURNING sequence",
        )
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO verified_runs (\
                id, submission_id, board_id, mission_id, edition, recorded_engine_version, \
                sim_config_json, max_concurrent_players, participant_instance_count, \
                starting_campaign_score, final_campaign_score, original_score_delta, \
                final_state_sha256, replay_frames, active_simulation_ticks, ransom_collected, \
                input_provenance_json, job_sha256, accepted_sequence, verified_at_ms\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&run_id)
        .bind(submission_id)
        .bind(board.board_id.as_str())
        .bind(&mission_id)
        .bind(edition_name(board.edition))
        .bind(&run.recorded_engine_version)
        .bind(&sim_config_json)
        .bind(i64::from(run.max_concurrent_players))
        .bind(i64::from(run.participant_instance_count))
        .bind(i64::from(run.starting_campaign_score))
        .bind(i64::from(run.final_campaign_score))
        .bind(run.original_score_delta)
        .bind(run.final_state_sha256.as_bytes().as_slice())
        .bind(i64::from(run.replay_frames))
        .bind(active_simulation_ticks)
        .bind(ransom_collected)
        .bind(&input_provenance_json)
        .bind(job_sha256.as_bytes().as_slice())
        .bind(accepted_sequence)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        for achievement in &run.achievements {
            sqlx::query(
                "INSERT INTO verified_run_achievements \
                 (run_id, achievement_id, evaluation, evidence_json) VALUES (?, ?, ?, ?)",
            )
            .bind(&run_id)
            .bind(achievement.achievement_id.as_str())
            .bind(achievement_evaluation_name(achievement.evaluation))
            .bind(canonical_json_string(&achievement.evidence)?)
            .execute(&mut *tx)
            .await?;
        }
        for metric in &requested {
            let (name, value) = match metric {
                BoardMetricV1::OriginalScore => ("original_score", run.original_score_delta),
                BoardMetricV1::FastestSuccess => ("fastest_success", active_simulation_ticks),
            };
            sqlx::query(
                "INSERT INTO verified_run_metrics (run_id, metric, value) VALUES (?, ?, ?)",
            )
            .bind(&run_id)
            .bind(name)
            .bind(value)
            .execute(&mut *tx)
            .await?;
        }
        let accepted = sqlx::query(
            "UPDATE submissions SET status = 'accepted', lease_owner = NULL, \
                lease_expires_at_ms = NULL, updated_at_ms = ? \
             WHERE id = ? AND tombstoned_at_ms IS NULL AND status = 'verifying'",
        )
        .bind(now)
        .bind(submission_id)
        .execute(&mut *tx)
        .await?;
        if accepted.rows_affected() != 1 {
            return Err(DbError::LeaseLost);
        }
        insert_worker_event(&mut tx, submission_id, "accepted", worker_id, "", now).await?;
        tx.commit().await?;
        Ok(run_id)
    }
}

pub(crate) const fn edition_name(
    edition: robin_run_protocol::OfficialContentEditionV1,
) -> &'static str {
    match edition {
        robin_run_protocol::OfficialContentEditionV1::Demo => "demo",
        robin_run_protocol::OfficialContentEditionV1::Full => "full",
    }
}

/// Mission replays retain the original game's wrapping `u32` subtotal, but a
/// ranked score is never allowed to wrap its signed `i32` owner. The verifier
/// result already proves the bit-preserving wrapping relation; this policy
/// check rejects the wraparound cases instead of ranking a small wrapped
/// subtotal.
pub(super) fn validate_ranked_score(
    starting_campaign_score: i32,
    final_campaign_score: i32,
    original_score_delta: i64,
) -> Result<(), DbError> {
    let checked_delta = i64::from(final_campaign_score) - i64::from(starting_campaign_score);
    if checked_delta < 0 || checked_delta != original_score_delta {
        return Err(DbError::ResultInvariant(
            "ranked score overflowed, decreased, or differs from the mission subtotal".to_owned(),
        ));
    }
    Ok(())
}

//! Worker lease and verification request transitions. Acceptance/publication remains in acceptance.rs; all transitions retain live-lease validation.
//! No independent pool, repository transaction, or fence is created here.
use super::*;

impl Database {
    pub async fn lease_next(
        &self,
        worker_id: &str,
        lease_duration: Duration,
    ) -> Result<Option<WorkerJob>, DbError> {
        if worker_id.is_empty() || worker_id.len() > 128 {
            return Err(DbError::ResultInvariant("invalid worker ID".to_owned()));
        }
        let now = now_epoch_ms()?;
        let lease_ms = i64::try_from(lease_duration.as_millis())
            .map_err(|_| DbError::ResultInvariant("worker lease is too long".to_owned()))?;
        let lease_expires = now
            .checked_add(lease_ms)
            .ok_or_else(|| DbError::ResultInvariant("worker lease overflow".to_owned()))?;

        for _ in 0..4 {
            let mut tx = self.pool.begin().await?;
            sqlx::query(
                "UPDATE submissions SET status = 'retry_pending', lease_owner = NULL, \
                    lease_expires_at_ms = NULL, next_attempt_at_ms = ?, updated_at_ms = ? \
                 WHERE status = 'verifying' AND lease_expires_at_ms < ?",
            )
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let row = sqlx::query(
                "SELECT id, replay_sha256, replay_bytes, starting_campaign_sha256, starting_campaign_bytes, \
                        envelope_json, attempts \
                 FROM submissions \
                 WHERE status IN ('queued', 'retry_pending') AND tombstoned_at_ms IS NULL \
                     AND NOT EXISTS (SELECT 1 FROM submission_terminal_failures f \
                                     WHERE f.submission_id = submissions.id) \
                     AND next_attempt_at_ms <= ? \
                 ORDER BY created_at_ms, id LIMIT 1",
            )
            .bind(now)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(row) = row else {
                tx.commit().await?;
                return Ok(None);
            };
            let id: String = row.try_get("id")?;
            let claimed = sqlx::query(
                "UPDATE submissions SET status = 'verifying', attempts = attempts + 1, \
                    lease_owner = ?, lease_expires_at_ms = ?, updated_at_ms = ? \
                 WHERE id = ? AND tombstoned_at_ms IS NULL \
                     AND status IN ('queued', 'retry_pending')",
            )
            .bind(worker_id)
            .bind(lease_expires)
            .bind(now)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
            if claimed.rows_affected() != 1 {
                tx.rollback().await?;
                continue;
            }
            sqlx::query(
                "INSERT INTO worker_events (submission_id, kind, worker_id, created_at_ms) \
                 VALUES (?, 'leased', ?, ?)",
            )
            .bind(&id)
            .bind(worker_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let replay_hash = fixed_32(row.try_get::<Vec<u8>, _>("replay_sha256")?)?;
            let replay_bytes = nonnegative_u64(row.try_get("replay_bytes")?, "replay_bytes")?;
            let starting_campaign_sha256 = fixed_32(row.try_get("starting_campaign_sha256")?)?;
            let starting_campaign_bytes = nonnegative_u64(
                row.try_get("starting_campaign_bytes")?,
                "starting_campaign_bytes",
            )?;
            let attempts = nonnegative_u64(row.try_get("attempts")?, "attempts")?;
            let job = WorkerJob {
                submission_id: id,
                replay_sha256: replay_hash,
                replay_bytes,
                starting_campaign_sha256,
                starting_campaign_bytes,
                envelope_json: row.try_get("envelope_json")?,
                attempts: u32::try_from(attempts + 1)
                    .map_err(|_| DbError::Corrupt("attempt count exceeds u32".to_owned()))?,
            };
            tx.commit().await?;
            return Ok(Some(job));
        }
        Err(DbError::Sql(sqlx::Error::Protocol(
            "could not acquire a pending job after concurrent claims".to_owned(),
        )))
    }

    pub async fn retry_job(
        &self,
        submission_id: &str,
        worker_id: &str,
        retry_after: Duration,
        private_detail: &str,
    ) -> Result<(), DbError> {
        let now = now_epoch_ms()?;
        let retry_ms = i64::try_from(retry_after.as_millis())
            .map_err(|_| DbError::ResultInvariant("retry delay is too long".to_owned()))?;
        let next = now
            .checked_add(retry_ms)
            .ok_or_else(|| DbError::ResultInvariant("retry time overflow".to_owned()))?;
        let mut tx = self.pool.begin().await?;
        ensure_worker_lease(&mut tx, submission_id, worker_id, now).await?;
        sqlx::query(
            "UPDATE submissions SET status = 'retry_pending', next_attempt_at_ms = ?, \
                lease_owner = NULL, lease_expires_at_ms = NULL, updated_at_ms = ? WHERE id = ?",
        )
        .bind(next)
        .bind(now)
        .bind(submission_id)
        .execute(&mut *tx)
        .await?;
        insert_worker_event(
            &mut tx,
            submission_id,
            "retry",
            worker_id,
            private_detail,
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn campaign_authority_for_verification(
        &self,
        submission_id: &str,
        worker_id: &str,
        request: &VerificationRequestV1,
        ruleset: &robin_run_protocol::RulesetManifestV1,
    ) -> Result<VerificationCampaignAuthority, DbError> {
        request
            .validate()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        if request.request_id.as_str() != submission_id {
            return Err(DbError::ResultInvariant(
                "verification request ID differs from leased submission".to_owned(),
            ));
        }
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin().await?;
        ensure_worker_lease(&mut tx, submission_id, worker_id, now).await?;
        let row = sqlx::query(
            "SELECT envelope_json, canonical_campaign_state_json, campaign_chain_id, predecessor_run_id \
             FROM submissions WHERE id = ? AND tombstoned_at_ms IS NULL",
        )
        .bind(submission_id)
        .fetch_one(&mut *tx)
        .await?;
        let stored: robin_run_protocol::SignedSubmissionV1 =
            serde_json::from_str(row.try_get::<String, _>("envelope_json")?.as_str())
                .map_err(|error| DbError::Corrupt(format!("signed submission JSON: {error}")))?;
        if request.submission != stored {
            return Err(DbError::ResultInvariant(
                "verification request does not contain the exact stored submission".to_owned(),
            ));
        }
        let offer = &request.submission.submission.offer;
        let canonical_campaign_state_json: String = row.try_get("canonical_campaign_state_json")?;
        let canonical_campaign_state: CanonicalCampaignStatePinV1 =
            serde_json::from_str(&canonical_campaign_state_json).map_err(|error| {
                DbError::Corrupt(format!("canonical campaign-state pin JSON: {error}"))
            })?;
        canonical_campaign_state
            .validate()
            .map_err(|error| DbError::Corrupt(format!("canonical campaign-state pin: {error}")))?;
        if ruleset
            .canonical_digest()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?
            != offer.ruleset_manifest_sha256
        {
            return Err(DbError::ResultInvariant(
                "campaign authority received a different ruleset".into(),
            ));
        }
        let signed_requirement = offer.starting_state.campaign_state_requirement();
        let ranked = &offer.session_genesis.claim.ranked_session;
        if canonical_campaign_state.requirement != signed_requirement
            || canonical_campaign_state.requirement.rules_config_sha256 != offer.rules_config_sha256
            || canonical_campaign_state.requirement.edition != ranked.content_edition
            || (ruleset
                .canonical_start_policy
                .requires_exact_operator_artifact()
                && matches!(
                    offer.starting_state,
                    InitialStateExpectationV1::IndividualLevel { .. }
                        | InitialStateExpectationV1::CampaignGenesis { .. }
                )
                && (canonical_campaign_state.artifact
                    != request.submission.submission.artifacts.starting_campaign
                    || canonical_campaign_state.artifact.sha256
                        != offer.starting_state.campaign_sha256()
                    || canonical_campaign_state.artifact.byte_length
                        != offer.starting_state.starting_campaign_byte_length()))
        {
            return Err(DbError::Corrupt(
                "persisted campaign-state pin differs from the signed starting-state authority"
                    .to_owned(),
            ));
        }
        let subject = &offer.session_genesis.claim.ranked_session.content_subject;
        let campaign_kind = |hq_sequence| match subject {
            robin_run_protocol::OfficialContentSubjectV1::FieldMission { mission_id } => {
                CampaignSessionKindV1::FieldMission {
                    mission_id: mission_id.clone(),
                }
            }
            robin_run_protocol::OfficialContentSubjectV1::Headquarters { .. } => {
                CampaignSessionKindV1::Headquarters { hq_sequence }
            }
        };
        let binding = match &offer.starting_state {
            InitialStateExpectationV1::IndividualLevel { .. } => {
                if row
                    .try_get::<Option<String>, _>("campaign_chain_id")?
                    .is_some()
                    || row
                        .try_get::<Option<String>, _>("predecessor_run_id")?
                        .is_some()
                {
                    return Err(DbError::Corrupt(
                        "individual verifier job has campaign linkage".to_owned(),
                    ));
                }
                None
            }
            InitialStateExpectationV1::CampaignGenesis { .. } => {
                if row
                    .try_get::<Option<String>, _>("campaign_chain_id")?
                    .is_none()
                    || row
                        .try_get::<Option<String>, _>("predecessor_run_id")?
                        .is_some()
                {
                    return Err(DbError::Corrupt(
                        "campaign-genesis verifier job has invalid linkage".to_owned(),
                    ));
                }
                Some(CampaignSessionBindingV1 {
                    kind: campaign_kind(1),
                    ordinal: 0,
                })
            }
            InitialStateExpectationV1::CampaignContinuation {
                chain_id,
                predecessor_run_id,
                ..
            } => {
                if row
                    .try_get::<Option<String>, _>("campaign_chain_id")?
                    .as_deref()
                    != Some(chain_id.as_str())
                    || row
                        .try_get::<Option<String>, _>("predecessor_run_id")?
                        .as_deref()
                        != Some(predecessor_run_id.as_str())
                {
                    return Err(DbError::Corrupt(
                        "campaign-continuation verifier job differs from stored linkage".to_owned(),
                    ));
                }
                let predecessor = sqlx::query(
                    "SELECT r.campaign_chain_id, r.campaign_session_ordinal \
                     FROM verified_runs r JOIN submissions s ON s.id = r.submission_id \
                     WHERE r.id = ? AND s.status = 'accepted' \
                       AND s.tombstoned_at_ms IS NULL AND r.scope_kind = 'campaign'",
                )
                .bind(predecessor_run_id.as_str())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| {
                    DbError::ResultInvariant(
                        "campaign verifier job predecessor is not accepted".to_owned(),
                    )
                })?;
                if predecessor
                    .try_get::<Option<String>, _>("campaign_chain_id")?
                    .as_deref()
                    != Some(chain_id.as_str())
                {
                    return Err(DbError::ResultInvariant(
                        "campaign verifier job predecessor belongs to a different chain".to_owned(),
                    ));
                }
                let predecessor_ordinal = predecessor
                    .try_get::<Option<i64>, _>("campaign_session_ordinal")?
                    .ok_or_else(|| {
                        DbError::Corrupt("campaign predecessor has no ordinal".to_owned())
                    })?;
                let ordinal = u32::try_from(predecessor_ordinal)
                    .ok()
                    .and_then(|value| value.checked_add(1))
                    .filter(|value| *value < robin_run_protocol::MAX_CAMPAIGN_SESSIONS_V1)
                    .ok_or_else(|| {
                        DbError::ResultInvariant(
                            "campaign predecessor ordinal cannot be continued".to_owned(),
                        )
                    })?;
                let prior_hq: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM verified_runs r \
                     JOIN submissions s ON s.id = r.submission_id \
                     WHERE r.campaign_chain_id = ? AND r.campaign_session_kind = 'headquarters' \
                       AND s.status = 'accepted' AND s.tombstoned_at_ms IS NULL \
                       AND r.campaign_session_ordinal <= ?",
                )
                .bind(chain_id.as_str())
                .bind(predecessor_ordinal)
                .fetch_one(&mut *tx)
                .await?;
                let hq_sequence = u32::try_from(prior_hq)
                    .ok()
                    .and_then(|value| value.checked_add(1))
                    .ok_or_else(|| {
                        DbError::ResultInvariant(
                            "campaign headquarters sequence cannot be continued".to_owned(),
                        )
                    })?;
                Some(CampaignSessionBindingV1 {
                    kind: campaign_kind(hq_sequence),
                    ordinal,
                })
            }
        };
        tx.commit().await?;
        Ok(VerificationCampaignAuthority {
            canonical_campaign_state,
            campaign_session: binding,
        })
    }

    pub async fn record_verification_request(
        &self,
        submission_id: &str,
        worker_id: &str,
        request: &VerificationRequestV1,
        route: &VerifierJobRouteV1,
        job_config_sha256: Digest32,
        verifier_policy_manifest_sha256: Digest32,
    ) -> Result<[u8; 32], DbError> {
        request
            .validate()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        if request.request_id.as_str() != submission_id {
            return Err(DbError::ResultInvariant(
                "verification request ID differs from leased submission".to_owned(),
            ));
        }
        route
            .validate()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        if route != &VerifierJobRouteV1::from_request(request)
            || job_config_sha256.is_zero()
            || verifier_policy_manifest_sha256.is_zero()
        {
            return Err(DbError::ResultInvariant(
                "verifier authority does not match the exact request".to_owned(),
            ));
        }
        let now = now_epoch_ms()?;
        let digest = request
            .canonical_digest()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?
            .into_bytes();
        let json = serde_json::to_string(request)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let route_json = serde_json::to_string(route)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let mut tx = self.pool.begin().await?;
        ensure_worker_lease(&mut tx, submission_id, worker_id, now).await?;
        let stored_envelope: String =
            sqlx::query_scalar("SELECT envelope_json FROM submissions WHERE id = ?")
                .bind(submission_id)
                .fetch_one(&mut *tx)
                .await?;
        let stored_submission: robin_run_protocol::SignedSubmissionV1 =
            serde_json::from_str(&stored_envelope)
                .map_err(|error| DbError::Corrupt(format!("signed submission JSON: {error}")))?;
        if request.submission != stored_submission {
            return Err(DbError::ResultInvariant(
                "verification request does not contain the exact stored submission".to_owned(),
            ));
        }
        let changed = sqlx::query(
            "UPDATE submissions SET verification_request_sha256 = ?, \
                 verification_request_json = ?, verifier_job_route_json = ?, \
                 verifier_job_config_sha256 = ?, verifier_policy_manifest_sha256 = ?, \
                 updated_at_ms = ? \
             WHERE id = ? AND tombstoned_at_ms IS NULL AND status = 'verifying' \
               AND (verification_request_sha256 IS NULL \
                    OR (verification_request_sha256 = ? \
                        AND verifier_job_route_json = ? \
                        AND verifier_job_config_sha256 = ? \
                        AND verifier_policy_manifest_sha256 = ?))",
        )
        .bind(digest.as_slice())
        .bind(&json)
        .bind(&route_json)
        .bind(job_config_sha256.as_bytes().as_slice())
        .bind(verifier_policy_manifest_sha256.as_bytes().as_slice())
        .bind(now)
        .bind(submission_id)
        .bind(digest.as_slice())
        .bind(&route_json)
        .bind(job_config_sha256.as_bytes().as_slice())
        .bind(verifier_policy_manifest_sha256.as_bytes().as_slice())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::ResultInvariant(
                "leased submission already has a different verification request".to_owned(),
            ));
        }
        tx.commit().await?;
        Ok(digest)
    }

    /// End a submission after its bounded verifier-infrastructure retry policy
    /// is exhausted. This is intentionally distinct from a replay rejection:
    /// no public rejection code is written and private worker diagnostics are
    /// never projected by the API.
    pub async fn fail_job(
        &self,
        submission_id: &str,
        worker_id: &str,
        request_artifact_sha256: Option<&[u8; 32]>,
        private_detail: &str,
    ) -> Result<(), DbError> {
        if private_detail.is_empty() || private_detail.len() > 2000 {
            return Err(DbError::ResultInvariant(
                "private infrastructure failure detail must contain 1..=2000 bytes".to_owned(),
            ));
        }
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin().await?;
        ensure_worker_lease(&mut tx, submission_id, worker_id, now).await?;
        sqlx::query(
            "INSERT INTO submission_terminal_failures \
             (submission_id, code, request_artifact_sha256, private_detail, failed_at_ms) \
             VALUES (?, 'verification_infrastructure', ?, ?, ?)",
        )
        .bind(submission_id)
        .bind(request_artifact_sha256.map(|digest| digest.as_slice()))
        .bind(private_detail)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let changed = sqlx::query(
            "UPDATE submissions SET status = 'retry_pending', lease_owner = NULL, \
                 lease_expires_at_ms = NULL, updated_at_ms = ? \
             WHERE id = ? AND status = 'verifying'",
        )
        .bind(now)
        .bind(submission_id)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::LeaseLost);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn reject_job(
        &self,
        submission_id: &str,
        worker_id: &str,
        rejection_code: &str,
        private_detail: &str,
    ) -> Result<(), DbError> {
        if !is_public_rejection_code(rejection_code) {
            return Err(DbError::ResultInvariant(format!(
                "unknown public rejection code `{rejection_code}`"
            )));
        }
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin().await?;
        ensure_worker_lease(&mut tx, submission_id, worker_id, now).await?;
        sqlx::query(
            "UPDATE submissions SET status = 'rejected', rejection_code = ?, \
                lease_owner = NULL, lease_expires_at_ms = NULL, updated_at_ms = ? WHERE id = ?",
        )
        .bind(rejection_code)
        .bind(now)
        .bind(submission_id)
        .execute(&mut *tx)
        .await?;
        insert_worker_event(
            &mut tx,
            submission_id,
            "rejected",
            worker_id,
            private_detail,
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

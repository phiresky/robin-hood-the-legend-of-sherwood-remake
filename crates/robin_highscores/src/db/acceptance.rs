//! Submission acceptance owns one write transaction from lease validation to publication.
//! Dispatch binding is loaded and checked through that same transaction; no
//! authoritative read is moved outside its lock.
use super::*;

impl Database {
    pub async fn accept_job(
        &self,
        submission_id: &str,
        worker_id: &str,
        verifier_executable_sha256: [u8; 32],
        verifier_job_config_sha256: Digest32,
        result: &VerificationResultV1,
        build_manifest: &LoadedBuildManifest,
        content_manifest: &ContentManifestV1,
        campaign_content_manifest: Option<&CampaignContentManifestV1>,
        published_ruleset: &PublishedRulesetV1,
        competition_manifest: Option<&CompetitionManifestV1>,
    ) -> Result<String, DbError> {
        result
            .validate()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        if result.request_id.as_str() != submission_id {
            return Err(DbError::ResultInvariant(
                "verifier request ID differs from the leased submission".to_owned(),
            ));
        }
        let verified = match &result.status {
            VerificationStatusV1::Verified(verified) => verified,
            VerificationStatusV1::Rejected(_) | VerificationStatusV1::FailedInfrastructure(_) => {
                return Err(DbError::ResultInvariant(
                    "only a typed verified result can enter the acceptance path".to_owned(),
                ));
            }
        };
        let now = now_epoch_ms()?;
        // A write lock makes the predecessor consumption check and acceptance
        // one compare-and-swap operation across competing workers.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        ensure_worker_lease(&mut tx, submission_id, worker_id, now).await?;
        let (submitted, verification_request, stored_signed, request_digest) =
            load_dispatched_submission(
                &mut tx,
                submission_id,
                verifier_job_config_sha256,
                result,
                published_ruleset,
                campaign_content_manifest,
            )
            .await?;
        let signed_offer = &stored_signed.submission.offer;
        let canonical_campaign_state_json: String =
            submitted.try_get("canonical_campaign_state_json")?;
        let canonical_campaign_state: CanonicalCampaignStatePinV1 =
            serde_json::from_str(&canonical_campaign_state_json).map_err(|error| {
                DbError::Corrupt(format!("canonical campaign-state pin JSON: {error}"))
            })?;
        canonical_campaign_state
            .validate()
            .map_err(|error| DbError::Corrupt(format!("canonical campaign-state pin: {error}")))?;
        if canonical_campaign_state.requirement
            != signed_offer.starting_state.campaign_state_requirement()
            || !published_ruleset
                .manifest
                .admits_campaign_state_requirement(canonical_campaign_state.requirement)
            || canonical_campaign_state.requirement.rules_config_sha256
                != signed_offer.rules_config_sha256
            || (published_ruleset
                .manifest
                .canonical_start_policy
                .requires_exact_operator_artifact()
                && matches!(
                    signed_offer.starting_state,
                    InitialStateExpectationV1::IndividualLevel { .. }
                        | InitialStateExpectationV1::CampaignGenesis { .. }
                )
                && canonical_campaign_state.artifact
                    != stored_signed.submission.artifacts.starting_campaign)
        {
            return Err(DbError::ResultInvariant(
                "submission does not reference its exact canonical campaign-state pin".to_owned(),
            ));
        }
        validate_ranked_policy(
            result,
            &stored_signed,
            verifier_executable_sha256,
            build_manifest.public_digest(),
            build_manifest.semantics(),
            content_manifest,
            campaign_content_manifest,
            published_ruleset,
            competition_manifest,
        )?;
        let scope_kind = check_offer_matches_result(
            &mut tx,
            submission_id,
            &submitted,
            &stored_signed,
            result,
            verified,
        )
        .await?;
        let PublicationDocuments {
            verification_result_json,
            result_sha256,
            public_verification_request_sha256,
            public_verification_result_sha256,
            public_verification_request_json,
            public_verification_result_json,
            public_projection_binding_json,
        } = publication_documents(request_digest, &verification_request, result)?;
        let result_columns =
            VerifiedResultColumns::derive(verified, result.input_provenance.as_ref())?;
        let build_manifest_id = result.build_manifest_sha256.into_bytes();
        let content_manifest_id = result.content_manifest_sha256.into_bytes();
        let campaign_content_manifest_id = signed_offer
            .session_genesis
            .claim
            .ranked_session
            .campaign_content_manifest_sha256
            .map(robin_run_protocol::Digest32::into_bytes);
        let stored_campaign_content_manifest_id = submitted
            .try_get::<Option<Vec<u8>>, _>("campaign_content_manifest_id")?
            .map(fixed_32)
            .transpose()?;
        if stored_campaign_content_manifest_id != campaign_content_manifest_id {
            return Err(DbError::ResultInvariant(
                "stored campaign content catalog differs from the signed genesis".to_owned(),
            ));
        }
        let config_id = result.rules_config_sha256.into_bytes();
        let ruleset_id = result.ruleset_manifest_sha256.into_bytes();
        let mission_id: String = submitted.try_get("mission_id")?;
        let competition_manifest_id = submitted
            .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
            .map(fixed_32)
            .transpose()?;
        if competition_manifest_id
            != result
                .competition_manifest_sha256
                .map(robin_run_protocol::Digest32::into_bytes)
        {
            return Err(DbError::ResultInvariant(
                "verifier competition manifest differs from the signed offer".to_owned(),
            ));
        }
        let (campaign_session_kind, campaign_hq_sequence) = match &verified.campaign_session_kind {
            Some(CampaignSessionKindV1::FieldMission {
                mission_id: verified_mission,
            }) => {
                if verified_mission != &mission_id {
                    return Err(DbError::ResultInvariant(
                        "verifier campaign mission differs from the signed offer".to_owned(),
                    ));
                }
                (Some("field_mission"), None)
            }
            Some(CampaignSessionKindV1::Headquarters { hq_sequence }) => {
                (Some("headquarters"), Some(*hq_sequence))
            }
            None => (None, None),
        };
        let campaign_chain_id: Option<String> = submitted.try_get("campaign_chain_id")?;
        let predecessor_run_id: Option<String> = submitted.try_get("predecessor_run_id")?;
        let expected_campaign_ordinal = validate_campaign_predecessor(
            &mut tx,
            &stored_signed,
            campaign_chain_id.as_deref(),
            predecessor_run_id.as_deref(),
            result,
            &canonical_campaign_state_json,
        )
        .await?;
        if verified.campaign_session_ordinal != expected_campaign_ordinal {
            return Err(DbError::ResultInvariant(
                "verifier campaign ordinal is not the next server-recognized chain position"
                    .to_owned(),
            ));
        }
        if let Some(predecessor_run_id) = &predecessor_run_id {
            let already_consumed: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM submissions \
                 WHERE predecessor_run_id = ? AND status = 'accepted' AND id <> ?)",
            )
            .bind(predecessor_run_id)
            .bind(submission_id)
            .fetch_one(&mut *tx)
            .await?;
            if already_consumed != 0 {
                return Err(DbError::CampaignFork);
            }
        }

        let requested: Vec<String> =
            serde_json::from_str(submitted.try_get("requested_metrics_json")?)
                .map_err(|error| DbError::Corrupt(format!("requested metrics JSON: {error}")))?;
        if requested.is_empty()
            || requested
                .iter()
                .any(|metric| !matches!(metric.as_str(), "original_score" | "fastest_success"))
        {
            return Err(DbError::Corrupt(
                "stored requested metrics are empty or invalid".to_owned(),
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
                id, submission_id, verifier_build_id, build_manifest_id, content_manifest_id, \
                campaign_content_manifest_id, config_id, ruleset_id, mission_id, scope_kind, competition_manifest_id, \
                canonical_campaign_state_json, \
                starting_campaign_sha256, starting_campaign_bytes, final_campaign_sha256, \
                final_campaign_bytes, campaign_chain_id, \
                predecessor_run_id, final_state_sha256, result_sha256, verification_result_json, \
                public_verification_request_sha256, public_verification_request_json, \
                public_verification_result_sha256, public_verification_result_json, \
                public_projection_binding_json, \
                verification_request_sha256, input_provenance_json, terminal_outcome, \
                replay_frames, diagnostics_json, original_score_delta, active_simulation_ticks, \
                ransom_collected, starting_campaign_score, final_campaign_score, \
                campaign_session_kind, campaign_session_ordinal, campaign_hq_sequence, \
                campaign_complete_evidence_sha256, max_concurrent_players, \
                participant_instance_count, named_participant_instance_count, \
                anonymous_participant_instance_count, accepted_sequence, \
                verified_at_ms\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
                       ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&run_id)
        .bind(submission_id)
        .bind(verifier_executable_sha256.as_slice())
        .bind(build_manifest_id.as_slice())
        .bind(content_manifest_id.as_slice())
        .bind(
            campaign_content_manifest_id
                .as_ref()
                .map(|digest| digest.as_slice()),
        )
        .bind(config_id.as_slice())
        .bind(ruleset_id.as_slice())
        .bind(&mission_id)
        .bind(scope_kind)
        .bind(
            competition_manifest_id
                .as_ref()
                .map(|digest| digest.as_slice()),
        )
        .bind(&canonical_campaign_state_json)
        .bind(verified.starting_campaign.sha256.as_bytes().as_slice())
        .bind(result_columns.starting_campaign_bytes)
        .bind(verified.final_campaign.sha256.as_bytes().as_slice())
        .bind(result_columns.final_campaign_bytes)
        .bind(&campaign_chain_id)
        .bind(&predecessor_run_id)
        .bind(verified.final_state_sha256.as_bytes().as_slice())
        .bind(result_sha256.as_bytes().as_slice())
        .bind(&verification_result_json)
        .bind(public_verification_request_sha256.as_bytes().as_slice())
        .bind(&public_verification_request_json)
        .bind(public_verification_result_sha256.as_bytes().as_slice())
        .bind(&public_verification_result_json)
        .bind(&public_projection_binding_json)
        .bind(request_digest.as_bytes().as_slice())
        .bind(&result_columns.input_provenance_json)
        .bind("won")
        .bind(i64::from(verified.replay_frames))
        .bind(&result_columns.diagnostics_json)
        .bind(verified.original_score_delta)
        .bind(result_columns.active_simulation_ticks)
        .bind(result_columns.ransom_collected)
        .bind(i64::from(verified.starting_campaign_score))
        .bind(i64::from(verified.final_campaign_score))
        .bind(campaign_session_kind)
        .bind(verified.campaign_session_ordinal.map(i64::from))
        .bind(campaign_hq_sequence.map(i64::from))
        .bind(
            result_columns.campaign_complete_evidence_sha256
                .as_ref()
                .map(|digest| digest.as_slice()),
        )
        .bind(i64::from(verified.max_concurrent_players))
        .bind(i64::from(verified.participant_instance_count))
        .bind(i64::from(verified.named_participant_instance_count))
        .bind(i64::from(verified.anonymous_participant_instance_count))
        .bind(accepted_sequence)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        insert_verified_run_details(&mut tx, &run_id, verified, &requested).await?;
        let accepted = sqlx::query(
            "UPDATE submissions SET status = 'accepted', lease_owner = NULL, \
                lease_expires_at_ms = NULL, updated_at_ms = ? \
             WHERE id = ? AND tombstoned_at_ms IS NULL \
                 AND (predecessor_run_id IS NULL OR NOT EXISTS (\
                 SELECT 1 FROM submissions other WHERE other.predecessor_run_id = \
                     submissions.predecessor_run_id AND other.status = 'accepted' AND other.id != ?\
             ))",
        )
        .bind(now)
        .bind(submission_id)
        .bind(submission_id)
        .execute(&mut *tx)
        .await?;
        if accepted.rows_affected() != 1 {
            return Err(DbError::ResultInvariant(
                "campaign predecessor already has an accepted continuation".to_owned(),
            ));
        }
        if verified.campaign_aggregation_consent
            == CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1
            && let Some(evidence) = result_columns.campaign_complete_evidence_sha256
        {
            self.create_full_campaign_aggregate(
                &mut tx,
                &run_id,
                evidence,
                now,
                published_ruleset,
                campaign_content_manifest.ok_or_else(|| {
                    DbError::ResultInvariant(
                        "campaign-complete result has no campaign content catalog".to_owned(),
                    )
                })?,
            )
            .await?;
        }
        insert_worker_event(&mut tx, submission_id, "accepted", worker_id, "", now).await?;
        tx.commit().await?;
        Ok(run_id)
    }
}

/// Validate the indexed and signed session together under the acceptance lock.
/// No publication write can precede this complete roster/artifact comparison.
async fn validate_campaign_predecessor(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    stored_signed: &robin_run_protocol::SignedSubmissionV1,
    campaign_chain_id: Option<&str>,
    predecessor_run_id: Option<&str>,
    result: &VerificationResultV1,
    canonical_campaign_state_json: &str,
) -> Result<Option<u32>, DbError> {
    let signed_offer = &stored_signed.submission.offer;
    let campaign_content_manifest_id = signed_offer
        .session_genesis
        .claim
        .ranked_session
        .campaign_content_manifest_sha256
        .map(Digest32::into_bytes);
    let config_id = result.rules_config_sha256.into_bytes();
    let ruleset_id = result.ruleset_manifest_sha256.into_bytes();
    let competition_manifest_id = result.competition_manifest_sha256.map(Digest32::into_bytes);
    match &signed_offer.starting_state {
        robin_run_protocol::InitialStateExpectationV1::IndividualLevel { .. } => {
            if campaign_chain_id.is_some() || predecessor_run_id.is_some() {
                return Err(DbError::Corrupt(
                    "individual run unexpectedly belongs to a campaign chain".to_owned(),
                ));
            }
            Ok(None)
        }
        robin_run_protocol::InitialStateExpectationV1::CampaignGenesis { .. } => {
            if campaign_chain_id.is_none() || predecessor_run_id.is_some() {
                return Err(DbError::Corrupt(
                    "campaign genesis has invalid server chain identity".to_owned(),
                ));
            }
            Ok(Some(0))
        }
        robin_run_protocol::InitialStateExpectationV1::CampaignContinuation {
            chain_id,
            predecessor_run_id: offered_predecessor,
            predecessor_verification_sha256,
            campaign_sha256,
            starting_campaign_byte_length,
            ..
        } => {
            if campaign_chain_id != Some(chain_id.as_str())
                || predecessor_run_id != Some(offered_predecessor.as_str())
            {
                return Err(DbError::ResultInvariant(
                    "stored campaign continuation identity differs from the signed offer"
                        .to_owned(),
                ));
            }
            let predecessor = sqlx::query(
                "SELECT r.campaign_chain_id, r.campaign_session_ordinal, r.result_sha256, \
                        r.final_campaign_sha256, r.final_campaign_bytes, \
                        r.campaign_content_manifest_id, r.config_id, \
                        r.ruleset_id, r.competition_manifest_id, \
                        r.canonical_campaign_state_json \
                 FROM verified_runs r JOIN submissions s ON s.id = r.submission_id \
                 WHERE r.id = ? AND s.status = 'accepted' \
                   AND s.tombstoned_at_ms IS NULL AND r.scope_kind = 'campaign'",
            )
            .bind(offered_predecessor.as_str())
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                DbError::ResultInvariant(
                    "campaign predecessor is no longer an accepted public run".to_owned(),
                )
            })?;
            let broken_ancestor: i64 = sqlx::query_scalar(
                "WITH RECURSIVE chain(run_id, predecessor_run_id) AS ( \
                     SELECT r.id, s.predecessor_run_id FROM verified_runs r \
                     JOIN submissions s ON s.id = r.submission_id WHERE r.id = ? \
                     UNION ALL \
                     SELECT predecessor.id, predecessor_submission.predecessor_run_id \
                     FROM chain current \
                     JOIN verified_runs predecessor \
                       ON predecessor.id = current.predecessor_run_id \
                     JOIN submissions predecessor_submission \
                       ON predecessor_submission.id = predecessor.submission_id \
                 ) \
                 SELECT EXISTS(SELECT 1 FROM chain \
                     JOIN verified_runs chain_run ON chain_run.id = chain.run_id \
                     JOIN submissions chain_submission \
                       ON chain_submission.id = chain_run.submission_id \
                     WHERE chain_submission.status != 'accepted' \
                        OR chain_submission.tombstoned_at_ms IS NOT NULL)",
            )
            .bind(offered_predecessor.as_str())
            .fetch_one(&mut **tx)
            .await?;
            if broken_ancestor != 0 {
                return Err(DbError::ResultInvariant(
                    "campaign predecessor chain contains a deleted session".to_owned(),
                ));
            }
            let predecessor_ordinal = predecessor
                .try_get::<Option<i64>, _>("campaign_session_ordinal")?
                .ok_or_else(|| {
                    DbError::Corrupt("campaign predecessor has no ordinal".to_owned())
                })?;
            let next_ordinal = u32::try_from(predecessor_ordinal)
                .ok()
                .and_then(|ordinal| ordinal.checked_add(1))
                .ok_or_else(|| {
                    DbError::ResultInvariant(
                        "campaign predecessor ordinal cannot be continued".to_owned(),
                    )
                })?;
            let predecessor_competition = predecessor
                .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
                .map(fixed_32)
                .transpose()?;
            if predecessor
                .try_get::<Option<String>, _>("campaign_chain_id")?
                .as_deref()
                != Some(chain_id.as_str())
                || fixed_32(predecessor.try_get("result_sha256")?)?
                    != predecessor_verification_sha256.into_bytes()
                || fixed_32(predecessor.try_get("final_campaign_sha256")?)?
                    != campaign_sha256.into_bytes()
                || predecessor.try_get::<i64, _>("final_campaign_bytes")?
                    != i64::try_from(*starting_campaign_byte_length).map_err(|_| {
                        DbError::ResultInvariant(
                            "continuation campaign length exceeds SQLite INTEGER".to_owned(),
                        )
                    })?
                || fixed_32(predecessor.try_get("campaign_content_manifest_id")?)?
                    != campaign_content_manifest_id.ok_or_else(|| {
                        DbError::ResultInvariant(
                            "campaign continuation has no content catalog".to_owned(),
                        )
                    })?
                || fixed_32(predecessor.try_get("config_id")?)? != config_id
                || fixed_32(predecessor.try_get("ruleset_id")?)? != ruleset_id
                || predecessor_competition != competition_manifest_id
                || predecessor
                    .try_get::<String, _>("canonical_campaign_state_json")?
                    .as_str()
                    != canonical_campaign_state_json
            {
                return Err(DbError::ResultInvariant(
                    "campaign continuation is not exactly cross-bound to its predecessor"
                        .to_owned(),
                ));
            }
            let owner_rows = sqlx::query(
                "SELECT owner_genesis.host_public_key \
                 FROM verified_runs owner_run \
                 JOIN submissions owner_submission \
                   ON owner_submission.id = owner_run.submission_id \
                 JOIN used_replay_session_geneses owner_genesis \
                   ON owner_genesis.submission_id = owner_submission.id \
                 WHERE owner_run.campaign_chain_id = ? \
                   AND owner_run.campaign_session_ordinal = 0 \
                   AND owner_run.scope_kind = 'campaign' \
                   AND owner_submission.status = 'accepted' \
                   AND owner_submission.tombstoned_at_ms IS NULL",
            )
            .bind(chain_id.as_str())
            .fetch_all(&mut **tx)
            .await?;
            if owner_rows.len() != 1 {
                return Err(DbError::Corrupt(
                    "campaign chain does not have exactly one live ordinal-zero owner".to_owned(),
                ));
            }
            let owner = fixed_32(owner_rows[0].try_get("host_public_key")?)?;
            let authorization = stored_signed
                .submission
                .campaign_continuation_authorization
                .as_ref()
                .ok_or_else(|| {
                    DbError::ResultInvariant(
                        "campaign continuation has no controller authorization".to_owned(),
                    )
                })?;
            let authorization_bytes = authorization
                .signing_bytes(&stored_signed.submission.offer)
                .map_err(|error| {
                    DbError::ResultInvariant(format!(
                        "campaign continuation authorization is not canonical: {error}"
                    ))
                })?;
            if authorization
                .claim
                .campaign_controller_public_key
                .as_bytes()
                != &owner
                || verify_signature(
                    &owner,
                    authorization.signature.as_bytes(),
                    &authorization_bytes,
                )
                .is_err()
                || !signed_offer
                    .participant_claims
                    .iter()
                    .any(|claim| claim.public_key.as_bytes() == &owner)
                || !stored_signed
                    .participant_signatures
                    .iter()
                    .any(|signature| signature.public_key.as_bytes() == &owner)
            {
                return Err(DbError::ResultInvariant(
                    "campaign continuation lacks the ordinal-zero owner's signed authorization"
                        .to_owned(),
                ));
            }
            Ok(Some(next_ordinal))
        }
    }
}
async fn insert_verified_run_details(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    run_id: &str,
    verified: &robin_run_protocol::VerifiedRunV1,
    requested: &[String],
) -> Result<(), DbError> {
    for (role, artifact) in [
        ("starting", &verified.starting_campaign),
        ("final", &verified.final_campaign),
    ] {
        ensure_live_campaign_object(tx, artifact).await?;
        sqlx::query(
            "INSERT INTO verified_run_campaign_objects (run_id, role, sha256) \
             VALUES (?, ?, ?)",
        )
        .bind(run_id)
        .bind(role)
        .bind(artifact.sha256.as_bytes().as_slice())
        .execute(&mut **tx)
        .await?;
    }
    for achievement in &verified.achievements {
        sqlx::query(
            "INSERT INTO verified_run_achievements \
             (run_id, achievement_id, earned, evaluation, evidence_json) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(run_id)
        .bind(achievement.achievement_id.as_str())
        .bind(i64::from(achievement.is_awarded()))
        .bind(achievement_evaluation_name(achievement.evaluation))
        .bind(
            serde_json::to_string(&achievement.evidence)
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
        )
        .execute(&mut **tx)
        .await?;
    }
    for metric in requested {
        let value = match metric.as_str() {
            "original_score" => verified.original_score_delta,
            "fastest_success" => i64::try_from(verified.active_simulation_ticks).map_err(|_| {
                DbError::ResultInvariant(
                    "simulation tick count does not fit SQLite INTEGER".to_owned(),
                )
            })?,
            _ => unreachable!("requested metrics were validated above"),
        };
        sqlx::query("INSERT INTO verified_run_metrics (run_id, metric, value) VALUES (?, ?, ?)")
            .bind(run_id)
            .bind(metric)
            .bind(value)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}
async fn check_offer_matches_result(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    submission_id: &str,
    submitted: &SqliteRow,
    signed: &robin_run_protocol::SignedSubmissionV1,
    result: &VerificationResultV1,
    verified: &robin_run_protocol::VerifiedRunV1,
) -> Result<&'static str, DbError> {
    let signed_offer = &signed.submission.offer;
    let session_genesis_sha256 = signed_offer
        .session_genesis
        .canonical_digest()
        .map_err(|error| DbError::Corrupt(format!("stored session genesis digest: {error}")))?;
    if verified.authenticated_participant_claims != signed_offer.participant_claims
        || verified.replay_session_transcript.session_genesis_sha256 != session_genesis_sha256
        || verified.replay_session_transcript.replay_session_id
            != signed_offer.session_genesis.claim.replay_session_id
        || verified.max_concurrent_players != signed_offer.max_concurrent_players
        || verified.participant_instance_count != signed_offer.participant_instance_count
        || verified.campaign_aggregation_consent != signed.submission.campaign_aggregation_consent
        || result.competition_manifest_sha256 != signed_offer.competition_manifest_sha256
    {
        return Err(DbError::ResultInvariant(
            "verifier session transcript does not match the exact signed offer".to_owned(),
        ));
    }
    if usize::from(verified.participant_instance_count)
        != verified.authenticated_participant_claims.len()
    {
        return Err(DbError::ResultInvariant(
            "ranked participant count includes an unsigned or unauthenticated instance".to_owned(),
        ));
    }
    compare_blob(
        &submitted,
        "replay_sha256",
        result.artifacts.replay.artifact.sha256.as_bytes(),
    )?;
    if nonnegative_u64(submitted.try_get("replay_bytes")?, "replay_bytes")?
        != result.artifacts.replay.artifact.byte_length
        || nonnegative_u64(
            submitted.try_get("starting_campaign_bytes")?,
            "starting_campaign_bytes",
        )? != result.artifacts.starting_campaign.byte_length
    {
        return Err(DbError::ResultInvariant(
            "verifier artifact byte lengths differ from stored uploads".to_owned(),
        ));
    }
    compare_blob(
        &submitted,
        "build_manifest_id",
        result.build_manifest_sha256.as_bytes(),
    )?;
    compare_blob(
        &submitted,
        "content_manifest_id",
        result.content_manifest_sha256.as_bytes(),
    )?;
    compare_blob(
        &submitted,
        "config_id",
        result.rules_config_sha256.as_bytes(),
    )?;
    compare_blob(
        &submitted,
        "ruleset_id",
        result.ruleset_manifest_sha256.as_bytes(),
    )?;
    compare_blob(
        &submitted,
        "starting_campaign_sha256",
        verified.starting_campaign.sha256.as_bytes(),
    )?;
    let scope_kind = match verified.scope_kind {
        RunScopeKindV1::IndividualLevel => "individual_level",
        RunScopeKindV1::Campaign => "campaign",
    };
    compare_value(&submitted, "scope_kind", scope_kind)?;
    let expected_max_concurrent: i64 = submitted.try_get("max_concurrent_players")?;
    let expected_instances: i64 = submitted.try_get("participant_instance_count")?;
    if expected_max_concurrent != i64::from(verified.max_concurrent_players)
        || expected_instances != i64::from(verified.participant_instance_count)
    {
        return Err(DbError::ResultInvariant(
            "verifier-derived session counts differ from the signed offer".to_owned(),
        ));
    }

    let submitted_participants = sqlx::query(
        "SELECT seat, participant_instance_id, public_key, public_disclosure \
         FROM submission_participants \
         WHERE submission_id = ? ORDER BY seat, participant_instance_id",
    )
    .bind(submission_id)
    .fetch_all(&mut **tx)
    .await?;
    if submitted_participants.len() != verified.authenticated_participant_claims.len() {
        return Err(DbError::ResultInvariant(
            "verifier did not authenticate every claimed participant".to_owned(),
        ));
    }
    for (stored, verified) in submitted_participants
        .into_iter()
        .zip(&verified.authenticated_participant_claims)
    {
        if stored.try_get::<i64, _>("seat")? != i64::from(verified.seat)
            || stored
                .try_get::<Vec<u8>, _>("participant_instance_id")?
                .as_slice()
                != verified.participant_instance_id.as_bytes()
            || stored.try_get::<Vec<u8>, _>("public_key")?.as_slice()
                != verified.public_key.as_bytes()
            || stored.try_get::<String, _>("public_disclosure")?
                != match verified.public_disclosure {
                    ParticipantPublicDisclosureV1::NamedProfile => "named_profile",
                    ParticipantPublicDisclosureV1::Anonymous => "anonymous",
                }
        {
            return Err(DbError::ResultInvariant(
                "verifier-derived participant roster differs from signed claims".to_owned(),
            ));
        }
    }

    if verified.outcome != TerminalOutcomeV1::Won {
        return Err(DbError::ResultInvariant(
            "only verifier-derived successful terminal runs can rank".to_owned(),
        ));
    }
    Ok(scope_kind)
}

/// SQL-ready verifier-authored columns; this projection cannot authorize chain advancement.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct VerifiedResultColumns {
    input_provenance_json: String,
    diagnostics_json: String,
    campaign_complete_evidence_sha256: Option<[u8; 32]>,
    starting_campaign_bytes: i64,
    final_campaign_bytes: i64,
    active_simulation_ticks: i64,
    ransom_collected: i64,
}

impl VerifiedResultColumns {
    fn derive(
        verified: &robin_run_protocol::VerifiedRunV1,
        provenance: Option<&InputProvenanceStatusV1>,
    ) -> Result<Self, DbError> {
        let provenance = provenance
            .ok_or_else(|| DbError::ResultInvariant("verified result has no provenance".into()))?;
        if !matches!(provenance, InputProvenanceStatusV1::Rankable) {
            return Err(DbError::ResultInvariant(
                "only verifier-derived Rankable provenance can enter ranked storage".to_owned(),
            ));
        }
        let input_provenance_json = serde_json::to_string(provenance)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let diagnostics_json = serde_json::to_string(&verified.diagnostics)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        if !(0..=i64::from(u32::MAX)).contains(&verified.original_score_delta) {
            return Err(DbError::ResultInvariant(
                "verifier score delta is outside the canonical wrapped-u32 range".to_owned(),
            ));
        }
        let campaign_complete_evidence_sha256 = verified
            .campaign_complete_evidence
            .as_ref()
            .map(|evidence| {
                evidence
                    .canonical_digest()
                    .map(robin_run_protocol::Digest32::into_bytes)
            })
            .transpose()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let integer = |value, message: &str| {
            i64::try_from(value).map_err(|_| DbError::ResultInvariant(message.to_owned()))
        };
        Ok(Self {
            input_provenance_json,
            diagnostics_json,
            campaign_complete_evidence_sha256,
            starting_campaign_bytes: integer(
                verified.starting_campaign.byte_length,
                "starting campaign length exceeds SQLite INTEGER",
            )?,
            final_campaign_bytes: integer(
                verified.final_campaign.byte_length,
                "final campaign length exceeds SQLite INTEGER",
            )?,
            active_simulation_ticks: integer(
                verified.active_simulation_ticks,
                "simulation tick count does not fit SQLite INTEGER",
            )?,
            ransom_collected: integer(
                verified.ransom_collected,
                "ransom amount does not fit SQLite INTEGER",
            )?,
        })
    }
}

/// Derived publication encoding, not an admission capability. No database writes
/// occur while constructing this document set.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PublicationDocuments {
    verification_result_json: String,
    result_sha256: Digest32,
    public_verification_request_sha256: Digest32,
    public_verification_result_sha256: Digest32,
    public_verification_request_json: String,
    public_verification_result_json: String,
    public_projection_binding_json: String,
}

fn publication_documents(
    request_digest: Digest32,
    verification_request: &VerificationRequestV1,
    result: &VerificationResultV1,
) -> Result<PublicationDocuments, DbError> {
    let verification_result_json = serde_json::to_string(result)
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    let result_sha256 = result
        .canonical_digest()
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    let public_verification_proof =
        PublicVerificationProofV1::from_private(verification_request, result)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    let public_verification_request_sha256 = public_verification_proof
        .public_request
        .canonical_digest()
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    if public_verification_request_sha256 != public_verification_proof.public_request_sha256 {
        return Err(DbError::ResultInvariant(
            "public verification request projection has an inconsistent digest".to_owned(),
        ));
    }
    let public_verification_result_sha256 = public_verification_proof
        .canonical_digest()
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    let public_verification_request_json =
        canonical_json_string(&public_verification_proof.public_request)?;
    let public_verification_result_json = canonical_json_string(&public_verification_proof)?;
    let public_projection_binding_json = projection_binding_json(
        request_digest,
        result_sha256,
        public_verification_request_sha256,
        public_verification_result_sha256,
    )?;
    Ok(PublicationDocuments {
        verification_result_json,
        result_sha256,
        public_verification_request_sha256,
        public_verification_result_sha256,
        public_verification_request_json,
        public_verification_result_json,
        public_projection_binding_json,
    })
}

/// Load and bind the exact sealed dispatch before applying score/chain policy.
async fn load_dispatched_submission(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    submission_id: &str,
    verifier_job_config_sha256: Digest32,
    result: &VerificationResultV1,
    published_ruleset: &PublishedRulesetV1,
    campaign_content_manifest: Option<&CampaignContentManifestV1>,
) -> Result<
    (
        SqliteRow,
        VerificationRequestV1,
        robin_run_protocol::SignedSubmissionV1,
        Digest32,
    ),
    DbError,
> {
    let submitted = sqlx::query(
            "SELECT replay_sha256, replay_bytes, starting_campaign_bytes, build_manifest_id, content_manifest_id, \
                    campaign_content_manifest_id, config_id, \
                    ruleset_id, mission_id, \
                    scope_kind, competition_manifest_id, starting_campaign_sha256, campaign_chain_id, \
                    predecessor_run_id, max_concurrent_players, participant_instance_count, \
                    requested_metrics_json, \
                    envelope_json, canonical_campaign_state_json, \
                    verification_request_sha256, verification_request_json, \
                    verifier_job_route_json, verifier_job_config_sha256, \
                    verifier_policy_manifest_sha256, created_at_ms \
             FROM submissions WHERE id = ? AND tombstoned_at_ms IS NULL",
        )
        .bind(submission_id)
        .fetch_one(&mut **tx)
        .await?;
    let verification_request_json = submitted
        .try_get::<Option<String>, _>("verification_request_json")?
        .ok_or_else(|| {
            DbError::ResultInvariant(
                "submission has no exact recorded verification request".to_owned(),
            )
        })?;
    let verification_request: VerificationRequestV1 =
        serde_json::from_str(&verification_request_json)
            .map_err(|error| DbError::Corrupt(format!("verification request JSON: {error}")))?;
    verification_request
        .validate()
        .map_err(|error| DbError::Corrupt(format!("verification request: {error}")))?;
    let request_digest = verification_request
        .canonical_digest()
        .map_err(|error| DbError::Corrupt(format!("verification request digest: {error}")))?;
    let stored_request_digest = fixed_32(
        submitted
            .try_get::<Option<Vec<u8>>, _>("verification_request_sha256")?
            .ok_or_else(|| {
                DbError::ResultInvariant("submission has no verification request digest".to_owned())
            })?,
    )?;
    let stored_route: VerifierJobRouteV1 = serde_json::from_str(
        &submitted
            .try_get::<Option<String>, _>("verifier_job_route_json")?
            .ok_or_else(|| {
                DbError::ResultInvariant("submission has no recorded verifier job route".to_owned())
            })?,
    )
    .map_err(|error| DbError::Corrupt(format!("verifier job route JSON: {error}")))?;
    stored_route
        .validate()
        .map_err(|error| DbError::Corrupt(format!("verifier job route: {error}")))?;
    let stored_job_config_sha256 = fixed_32(
        submitted
            .try_get::<Option<Vec<u8>>, _>("verifier_job_config_sha256")?
            .ok_or_else(|| {
                DbError::ResultInvariant(
                    "submission has no sealed verifier job-config identity".to_owned(),
                )
            })?,
    )?;
    let stored_policy_sha256 = fixed_32(
        submitted
            .try_get::<Option<Vec<u8>>, _>("verifier_policy_manifest_sha256")?
            .ok_or_else(|| {
                DbError::ResultInvariant(
                    "submission has no immutable verifier-policy identity".to_owned(),
                )
            })?,
    )?;
    let stored_signed: robin_run_protocol::SignedSubmissionV1 =
        serde_json::from_str(submitted.try_get::<String, _>("envelope_json")?.as_str())
            .map_err(|error| DbError::Corrupt(format!("signed submission JSON: {error}")))?;
    match &stored_signed
        .submission
        .offer
        .session_genesis
        .claim
        .competition_run_grant
    {
        None => {}
        Some(grant) => {
            let submitted_at = u64::try_from(submitted.try_get::<i64, _>("created_at_ms")?)
                .map_err(|_| DbError::Corrupt("negative submission creation time".into()))?;
            if submitted_at < grant.claim.admitted_at_unix_ms
                || submitted_at > grant.claim.expires_at_unix_ms
            {
                return Err(DbError::ResultInvariant(
                    "competition upload was not completed inside its admitted server interval"
                        .to_owned(),
                ));
            }
        }
    }
    if request_digest.as_bytes() != &stored_request_digest
        || result.verification_request_sha256 != request_digest
        || verification_request.request_id.as_str() != submission_id
        || verification_request.submission != stored_signed
        || stored_route != VerifierJobRouteV1::from_request(&verification_request)
        || stored_job_config_sha256 != verifier_job_config_sha256.into_bytes()
        || stored_policy_sha256
            != published_ruleset
                .manifest
                .verifier_policy
                .manifest_sha256
                .into_bytes()
    {
        return Err(DbError::ResultInvariant(
            "verifier result does not bind the exact dispatched request and authority".to_owned(),
        ));
    }
    result
        .validate_campaign_complete_evidence(
            &verification_request,
            &published_ruleset.manifest,
            campaign_content_manifest,
        )
        .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
    Ok((
        submitted,
        verification_request,
        stored_signed,
        request_digest,
    ))
}

//! Campaign aggregation within the accepting submission transaction.
//! Chain reads, proof validation and publication never acquire a separate connection.
use super::*;

impl Database {
    pub(super) async fn create_full_campaign_aggregate(
        &self,
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        terminal_run_id: &str,
        campaign_complete_evidence_sha256: [u8; 32],
        verified_at_ms: i64,
        published_ruleset: &PublishedRulesetV1,
        campaign_content_manifest: &CampaignContentManifestV1,
    ) -> Result<String, DbError> {
        let reversed = load_campaign_chain(tx, terminal_run_id).await?;

        let first = reversed.first().expect("nonempty campaign chain checked");
        let terminal = reversed.last().expect("nonempty campaign chain checked");
        let chain_id: String = terminal
            .try_get::<Option<String>, _>("campaign_chain_id")?
            .ok_or_else(|| DbError::ResultInvariant("campaign run has no chain ID".to_owned()))?;
        let campaign_content_manifest_id =
            fixed_32(terminal.try_get("campaign_content_manifest_id")?)?;
        let config_id = fixed_32(terminal.try_get("config_id")?)?;
        let ruleset_id = fixed_32(terminal.try_get("ruleset_id")?)?;
        let competition_manifest_id = terminal
            .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
            .map(fixed_32)
            .transpose()?;
        let profile: AdmissionProfile = serde_json::from_str(
            terminal
                .try_get::<String, _>("public_metadata_json")?
                .as_str(),
        )
        .map_err(|error| DbError::Corrupt(format!("campaign admission profile: {error}")))?;
        if !profile
            .allowed_scopes
            .iter()
            .any(|scope| scope == "campaign_continuation")
            || published_ruleset
                .manifest
                .board_scopes
                .binary_search(&robin_run_protocol::RulesetBoardScopeV1::FullCampaign)
                .is_err()
        {
            return Err(DbError::ResultInvariant(
                "ruleset does not publish full-campaign boards".to_owned(),
            ));
        }
        let expected_genesis = profile.canonical_campaign_state.artifact.sha256;
        let canonical_campaign_state_json =
            serde_json::to_string(&profile.canonical_campaign_state).map_err(|error| {
                DbError::Corrupt(format!("canonical campaign-state pin JSON: {error}"))
            })?;
        let starting_state: InitialStateExpectationV1 =
            serde_json::from_str(first.try_get::<String, _>("starting_state_json")?.as_str())
                .map_err(|error| DbError::Corrupt(format!("campaign starting state: {error}")))?;
        let starting_state_requirement = starting_state.campaign_state_requirement();
        let InitialStateExpectationV1::CampaignGenesis {
            campaign_sha256, ..
        } = starting_state
        else {
            return Err(DbError::ResultInvariant(
                "full campaign does not start at canonical genesis".to_owned(),
            ));
        };
        if (published_ruleset
            .manifest
            .canonical_start_policy
            .requires_exact_operator_artifact()
            && campaign_sha256 != expected_genesis)
            || starting_state_requirement != profile.canonical_campaign_state.requirement
            || profile
                .canonical_campaign_state
                .requirement
                .rules_config_sha256
                != Digest32::from_bytes(config_id)
        {
            return Err(DbError::ResultInvariant(
                "full campaign genesis differs from its rules-config-bound operator pin".to_owned(),
            ));
        }

        let mut sessions = Vec::with_capacity(reversed.len());
        let mut session_public_proofs = Vec::with_capacity(reversed.len());
        let mut previous_final = None;
        let mut previous_score = None;
        let mut aggregate_ticks = 0_u64;
        let mut aggregate_ransom = 0_u64;
        let mut aggregate_participant_instances = 0_u32;
        let mut aggregate_named_instances = 0_u32;
        let mut aggregate_anonymous_instances = 0_u32;
        let mut aggregate_max_concurrent = 0_u16;
        let mut authenticated_participant_keys = BTreeSet::new();
        let mut public_named_keys = BTreeSet::new();
        let mut campaign_controller_public_key = None;
        for (session_index, row) in reversed.iter().enumerate() {
            let row_chain: Option<String> = row.try_get("campaign_chain_id")?;
            let row_content = fixed_32(row.try_get("content_manifest_id")?)?;
            let row_campaign_content = fixed_32(row.try_get("campaign_content_manifest_id")?)?;
            let row_config = fixed_32(row.try_get("config_id")?)?;
            let row_ruleset = fixed_32(row.try_get("ruleset_id")?)?;
            let row_competition = row
                .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
                .map(fixed_32)
                .transpose()?;
            let row_players = checked_count(
                &row,
                "max_concurrent_players",
                "campaign player count exceeds u16",
            )?;
            let row_instances = checked_count(
                &row,
                "participant_instance_count",
                "campaign participant count exceeds u16",
            )?;
            if row_chain.as_deref() != Some(chain_id.as_str())
                || row_campaign_content != campaign_content_manifest_id
                || row_config != config_id
                || row_ruleset != ruleset_id
                || row_competition != competition_manifest_id
                || row
                    .try_get::<String, _>("canonical_campaign_state_json")?
                    .as_str()
                    != canonical_campaign_state_json.as_str()
            {
                return Err(DbError::ResultInvariant(
                    "campaign chain changes immutable board identity".to_owned(),
                ));
            }
            let CampaignSessionDocuments {
                result: stored_result,
                signed,
                public_proof,
            } = validate_campaign_session_documents(
                row,
                (session_index == 0).then_some(&starting_state),
                published_ruleset,
                campaign_content_manifest,
            )?;
            let VerificationStatusV1::Verified(verified) = &stored_result.status else {
                return Err(DbError::Corrupt(
                    "accepted campaign row does not contain a verified result".to_owned(),
                ));
            };
            session_public_proofs.push(public_proof);
            let mut row_authenticated_keys = verified
                .authenticated_participant_claims
                .iter()
                .map(|claim| claim.public_key)
                .collect::<Vec<_>>();
            row_authenticated_keys.sort_unstable();
            for key in &row_authenticated_keys {
                authenticated_participant_keys.insert(*key);
            }
            for claim in &verified.authenticated_participant_claims {
                if claim.public_disclosure == ParticipantPublicDisclosureV1::NamedProfile {
                    public_named_keys.insert(claim.public_key);
                }
            }
            aggregate_participant_instances = aggregate_participant_instances
                .checked_add(u32::from(row_instances))
                .ok_or_else(|| {
                    DbError::ResultInvariant(
                        "campaign participant instance count overflow".to_owned(),
                    )
                })?;
            aggregate_named_instances = aggregate_named_instances
                .checked_add(u32::from(verified.named_participant_instance_count))
                .ok_or_else(|| {
                    DbError::ResultInvariant("campaign named participant count overflow".to_owned())
                })?;
            aggregate_anonymous_instances = aggregate_anonymous_instances
                .checked_add(u32::from(verified.anonymous_participant_instance_count))
                .ok_or_else(|| {
                    DbError::ResultInvariant(
                        "campaign anonymous participant count overflow".to_owned(),
                    )
                })?;
            aggregate_max_concurrent = aggregate_max_concurrent.max(row_players);
            let starting_campaign = ArtifactRefV1 {
                sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
                    row.try_get("starting_campaign_sha256")?,
                )?),
                byte_length: nonnegative_u64(
                    row.try_get("starting_campaign_bytes")?,
                    "starting_campaign_bytes",
                )?,
                media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
            };
            let final_campaign = ArtifactRefV1 {
                sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
                    row.try_get("final_campaign_sha256")?,
                )?),
                byte_length: nonnegative_u64(
                    row.try_get("final_campaign_bytes")?,
                    "final_campaign_bytes",
                )?,
                media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
            };
            let start_score = i32::try_from(row.try_get::<i64, _>("starting_campaign_score")?)
                .map_err(|_| DbError::Corrupt("campaign start score exceeds i32".to_owned()))?;
            let final_score = i32::try_from(row.try_get::<i64, _>("final_campaign_score")?)
                .map_err(|_| DbError::Corrupt("campaign final score exceeds i32".to_owned()))?;
            if previous_final
                .as_ref()
                .is_some_and(|artifact| artifact != &starting_campaign)
                || previous_score.is_some_and(|score| score != start_score)
            {
                return Err(DbError::ResultInvariant(
                    "campaign chain state or score continuity is broken".to_owned(),
                ));
            }
            previous_final = Some(final_campaign.clone());
            previous_score = Some(final_score);
            let active_ticks = nonnegative_u64(
                row.try_get("active_simulation_ticks")?,
                "active_simulation_ticks",
            )?;
            let ransom = nonnegative_u64(row.try_get("ransom_collected")?, "ransom_collected")?;
            aggregate_ticks = aggregate_ticks.checked_add(active_ticks).ok_or_else(|| {
                DbError::ResultInvariant("campaign active tick sum overflow".to_owned())
            })?;
            aggregate_ransom = aggregate_ransom.checked_add(ransom).ok_or_else(|| {
                DbError::ResultInvariant("campaign ransom sum overflow".to_owned())
            })?;
            let kind = verified.campaign_session_kind.clone().ok_or_else(|| {
                DbError::Corrupt("campaign result is missing its session kind".to_owned())
            })?;
            let ordinal = verified.campaign_session_ordinal.ok_or_else(|| {
                DbError::Corrupt("campaign result is missing its session ordinal".to_owned())
            })?;
            let content_subject = signed
                .submission
                .offer
                .session_genesis
                .claim
                .ranked_session
                .content_subject
                .clone();
            if campaign_content_manifest.content_for(&content_subject)
                != Some(robin_run_protocol::Digest32::from_bytes(row_content))
            {
                return Err(DbError::ResultInvariant(
                    "campaign session content does not resolve through the exact catalog"
                        .to_owned(),
                ));
            }
            if ordinal == 0
                && campaign_controller_public_key
                    .replace(
                        signed
                            .submission
                            .offer
                            .session_genesis
                            .claim
                            .host_public_key,
                    )
                    .is_some()
            {
                return Err(DbError::ResultInvariant(
                    "campaign chain contains multiple ordinal-zero controllers".to_owned(),
                ));
            }
            let build_manifest_sha256 = stored_result.build_manifest_sha256;
            if verified.starting_campaign != starting_campaign
                || verified.final_campaign != final_campaign
                || verified.starting_campaign_score != start_score
                || verified.final_campaign_score != final_score
                || verified.active_simulation_ticks != active_ticks
                || verified.ransom_collected != ransom
                || verified.max_concurrent_players != row_players
                || verified.participant_instance_count != row_instances
                || stored_result
                    .competition_manifest_sha256
                    .map(|value| value.into_bytes())
                    != row_competition
                || signed.submission.artifacts != stored_result.artifacts
                || row.try_get::<Vec<u8>, _>("replay_sha256")?.as_slice()
                    != stored_result.artifacts.replay.artifact.sha256.as_bytes()
                || row.try_get::<i64, _>("replay_bytes")?
                    != i64::try_from(stored_result.artifacts.replay.artifact.byte_length).map_err(
                        |_| DbError::Corrupt("replay byte length exceeds i64".to_owned()),
                    )?
            {
                return Err(DbError::Corrupt(
                    "campaign typed result differs from indexed storage".to_owned(),
                ));
            }
            sessions.push(VerifiedCampaignSessionV1 {
                ordinal,
                run_id: OpaqueId::new(row.try_get::<String, _>("id")?)
                    .map_err(|error| DbError::Corrupt(error.to_string()))?,
                kind,
                content_subject,
                campaign_aggregation_consent: verified.campaign_aggregation_consent,
                replay: stored_result.artifacts.replay.clone(),
                build_manifest_sha256,
                content_manifest_sha256: stored_result.content_manifest_sha256,
                rules_config_sha256: stored_result.rules_config_sha256,
                ruleset_manifest_sha256: stored_result.ruleset_manifest_sha256,
                competition_manifest_sha256: stored_result.competition_manifest_sha256,
                verification_request_sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
                    row.try_get("verification_request_sha256")?,
                )?),
                verification_result_sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(
                    row.try_get("result_sha256")?,
                )?),
                starting_campaign,
                final_campaign,
                starting_campaign_score: start_score,
                final_campaign_score: final_score,
                max_concurrent_players: row_players,
                participant_instance_count: row_instances,
                named_participant_instance_count: verified.named_participant_instance_count,
                anonymous_participant_instance_count: verified.anonymous_participant_instance_count,
                authenticated_participant_keys: row_authenticated_keys,
                active_simulation_ticks: active_ticks,
                ransom_collected: ransom,
                campaign_complete_evidence_sha256: verified
                    .campaign_complete_evidence
                    .as_ref()
                    .map(|evidence| evidence.canonical_digest())
                    .transpose()
                    .map_err(|error| {
                        DbError::Corrupt(format!("campaign completion evidence digest: {error}"))
                    })?,
            });
        }
        let authenticated_participant_keys = authenticated_participant_keys
            .into_iter()
            .collect::<Vec<_>>();
        let public_named_keys = public_named_keys.into_iter().collect::<Vec<_>>();
        let campaign_controller_public_key = campaign_controller_public_key.ok_or_else(|| {
            DbError::ResultInvariant("campaign chain has no ordinal-zero controller".to_owned())
        })?;

        let full_campaign_run_id = uuid::Uuid::now_v7().to_string();
        let request = PrivateCampaignAggregateRequestV1 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
            chain_id: OpaqueId::new(chain_id.clone())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            full_campaign_run_id: OpaqueId::new(full_campaign_run_id.clone())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            terminal_run_id: OpaqueId::new(terminal_run_id.to_owned())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            campaign_complete_evidence_sha256: robin_run_protocol::Digest32::from_bytes(
                campaign_complete_evidence_sha256,
            ),
            sessions: sessions
                .iter()
                .map(|session| PrivateCampaignAggregateSessionRequestV1 {
                    ordinal: session.ordinal,
                    run_id: session.run_id.clone(),
                    verification_request_sha256: session.verification_request_sha256,
                    verification_result_sha256: session.verification_result_sha256,
                })
                .collect(),
        };
        let aggregate_request_sha256 = request.canonical_digest()?;
        let aggregate_request_json = canonical_json_string(&request)?;
        let canonical_genesis_campaign = sessions
            .first()
            .expect("nonempty sessions")
            .starting_campaign
            .clone();
        let final_campaign = sessions
            .last()
            .expect("nonempty sessions")
            .final_campaign
            .clone();
        let starting_campaign_score = sessions
            .first()
            .expect("nonempty sessions")
            .starting_campaign_score;
        let final_campaign_score = sessions
            .last()
            .expect("nonempty sessions")
            .final_campaign_score;
        let aggregate = VerifiedCampaignAggregateV1 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
            aggregate_request_sha256,
            chain_id: OpaqueId::new(chain_id.clone())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            full_campaign_run_id: OpaqueId::new(full_campaign_run_id.clone())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            campaign_complete_terminal_run_id: OpaqueId::new(terminal_run_id.to_owned())
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
            campaign_complete_evidence_sha256: robin_run_protocol::Digest32::from_bytes(
                campaign_complete_evidence_sha256,
            ),
            sessions,
            max_concurrent_players: aggregate_max_concurrent,
            participant_instance_count: aggregate_participant_instances,
            named_participant_instance_count: aggregate_named_instances,
            anonymous_participant_instance_count: aggregate_anonymous_instances,
            authenticated_participant_keys,
            campaign_controller_public_key,
            campaign_content_manifest_sha256: robin_run_protocol::Digest32::from_bytes(
                campaign_content_manifest_id,
            ),
            rules_config_sha256: robin_run_protocol::Digest32::from_bytes(config_id),
            ruleset_manifest_sha256: robin_run_protocol::Digest32::from_bytes(ruleset_id),
            competition_manifest_sha256: competition_manifest_id
                .map(robin_run_protocol::Digest32::from_bytes),
            canonical_genesis_campaign: canonical_genesis_campaign.clone(),
            final_campaign: final_campaign.clone(),
            starting_campaign_score,
            final_campaign_score,
            active_simulation_ticks: aggregate_ticks,
            ransom_collected: aggregate_ransom,
        };
        aggregate
            .validate_against_ruleset(published_ruleset, campaign_content_manifest)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let aggregate_sha256 = aggregate
            .canonical_digest()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let aggregate_json = canonical_json_string(&aggregate)?;
        let public_aggregate_proof =
            PublicCampaignAggregateProofV1::from_private(&aggregate, &session_public_proofs)
                .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let public_aggregate_request_sha256 = public_aggregate_proof
            .public_request
            .canonical_digest()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        if public_aggregate_request_sha256 != public_aggregate_proof.public_request_sha256 {
            return Err(DbError::ResultInvariant(
                "public aggregate request projection has an inconsistent digest".to_owned(),
            ));
        }
        let public_aggregate_result_sha256 = public_aggregate_proof
            .canonical_digest()
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?;
        let public_aggregate_request_json =
            canonical_json_string(&public_aggregate_proof.public_request)?;
        let public_aggregate_result_json = canonical_json_string(&public_aggregate_proof)?;
        let public_projection_binding_json = projection_binding_json(
            aggregate_request_sha256,
            aggregate_sha256,
            public_aggregate_request_sha256,
            public_aggregate_result_sha256,
        )?;
        let accepted_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO acceptance_sequences (created_at_ms) VALUES (?) RETURNING sequence",
        )
        .bind(verified_at_ms)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO full_campaign_runs (id, chain_id, terminal_run_id, \
                aggregate_request_sha256, aggregate_sha256, aggregate_json, \
                aggregate_request_json, public_aggregate_request_sha256, \
                public_aggregate_request_json, public_aggregate_result_sha256, \
                public_aggregate_result_json, public_projection_binding_json, \
                campaign_complete_evidence_sha256, campaign_content_manifest_id, \
                config_id, ruleset_id, \
                canonical_campaign_state_json, \
                competition_manifest_id, starting_campaign_sha256, starting_campaign_bytes, \
                final_campaign_sha256, final_campaign_bytes, \
                starting_campaign_score, final_campaign_score, active_simulation_ticks, \
                ransom_collected, max_concurrent_players, participant_instance_count, \
                named_participant_instance_count, anonymous_participant_instance_count, \
                accepted_sequence, verified_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
                     ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&full_campaign_run_id)
        .bind(&chain_id)
        .bind(terminal_run_id)
        .bind(aggregate_request_sha256.as_bytes().as_slice())
        .bind(aggregate_sha256.as_bytes().as_slice())
        .bind(&aggregate_json)
        .bind(&aggregate_request_json)
        .bind(public_aggregate_request_sha256.as_bytes().as_slice())
        .bind(&public_aggregate_request_json)
        .bind(public_aggregate_result_sha256.as_bytes().as_slice())
        .bind(&public_aggregate_result_json)
        .bind(&public_projection_binding_json)
        .bind(campaign_complete_evidence_sha256.as_slice())
        .bind(campaign_content_manifest_id.as_slice())
        .bind(config_id.as_slice())
        .bind(ruleset_id.as_slice())
        .bind(&canonical_campaign_state_json)
        .bind(
            competition_manifest_id
                .as_ref()
                .map(|digest| digest.as_slice()),
        )
        .bind(canonical_genesis_campaign.sha256.as_bytes().as_slice())
        .bind(
            i64::try_from(canonical_genesis_campaign.byte_length).map_err(|_| {
                DbError::ResultInvariant("campaign length exceeds SQLite INTEGER".to_owned())
            })?,
        )
        .bind(final_campaign.sha256.as_bytes().as_slice())
        .bind(i64::try_from(final_campaign.byte_length).map_err(|_| {
            DbError::ResultInvariant("campaign length exceeds SQLite INTEGER".to_owned())
        })?)
        .bind(i64::from(starting_campaign_score))
        .bind(i64::from(final_campaign_score))
        .bind(i64::try_from(aggregate_ticks).map_err(|_| {
            DbError::ResultInvariant("campaign tick sum exceeds SQLite INTEGER".to_owned())
        })?)
        .bind(i64::try_from(aggregate_ransom).map_err(|_| {
            DbError::ResultInvariant("campaign ransom sum exceeds SQLite INTEGER".to_owned())
        })?)
        .bind(i64::from(aggregate_max_concurrent))
        .bind(i64::from(aggregate_participant_instances))
        .bind(i64::from(aggregate_named_instances))
        .bind(i64::from(aggregate_anonymous_instances))
        .bind(accepted_sequence)
        .bind(verified_at_ms)
        .execute(&mut **tx)
        .await?;
        for (ordinal, session) in aggregate.sessions.iter().enumerate() {
            sqlx::query(
                "INSERT INTO full_campaign_sessions (full_campaign_run_id, ordinal, run_id) \
                 VALUES (?, ?, ?)",
            )
            .bind(&full_campaign_run_id)
            .bind(i64::try_from(ordinal).map_err(|_| {
                DbError::ResultInvariant("campaign session ordinal exceeds i64".to_owned())
            })?)
            .bind(session.run_id.as_str())
            .execute(&mut **tx)
            .await?;
        }
        for public_key in &public_named_keys {
            sqlx::query(
                "INSERT INTO full_campaign_participants \
                 (full_campaign_run_id, public_key) VALUES (?, ?)",
            )
            .bind(&full_campaign_run_id)
            .bind(public_key.as_bytes().as_slice())
            .execute(&mut **tx)
            .await?;
        }
        let aggregate_score = i64::from(final_campaign_score) - i64::from(starting_campaign_score);
        for (metric, value) in [
            ("original_score", aggregate_score),
            (
                "fastest_success",
                i64::try_from(aggregate_ticks).map_err(|_| {
                    DbError::ResultInvariant("campaign tick sum exceeds SQLite INTEGER".to_owned())
                })?,
            ),
        ] {
            sqlx::query(
                "INSERT INTO full_campaign_metrics (full_campaign_run_id, metric, value) \
                 VALUES (?, ?, ?)",
            )
            .bind(&full_campaign_run_id)
            .bind(metric)
            .bind(value)
            .execute(&mut **tx)
            .await?;
        }
        Ok(full_campaign_run_id)
    }
}

async fn load_campaign_chain(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    terminal_run_id: &str,
) -> Result<Vec<SqliteRow>, DbError> {
    let mut reversed = Vec::new();
    let mut current = terminal_run_id.to_owned();
    for _ in 0..4_096 {
        let row = sqlx::query(
            "SELECT r.id, r.submission_id, r.mission_id, r.verification_request_sha256, r.result_sha256, \
                    r.starting_campaign_sha256, r.starting_campaign_bytes, \
                    r.final_campaign_sha256, r.final_campaign_bytes, \
                    r.starting_campaign_score, r.final_campaign_score, \
                    r.active_simulation_ticks, r.ransom_collected, r.campaign_session_kind, \
                    r.campaign_session_ordinal, r.campaign_hq_sequence, \
                    r.campaign_complete_evidence_sha256, r.verification_result_json, \
                    r.public_verification_request_sha256, r.public_verification_request_json, \
                    r.public_verification_result_sha256, r.public_verification_result_json, \
                    r.public_projection_binding_json, \
                    r.build_manifest_id, r.content_manifest_id, r.campaign_content_manifest_id, \
                    r.config_id, r.ruleset_id, r.canonical_campaign_state_json, \
                    r.max_concurrent_players, r.participant_instance_count, \
                    r.named_participant_instance_count, r.anonymous_participant_instance_count, \
                    s.predecessor_run_id, s.campaign_chain_id, s.competition_manifest_id, \
                    s.starting_state_json, s.public_metadata_json, s.envelope_json, \
                    s.replay_sha256, s.replay_bytes, \
                    s.verification_request_json \
             FROM verified_runs r JOIN submissions s ON s.id = r.submission_id \
             WHERE r.id = ? AND r.scope_kind = 'campaign' AND s.status = 'accepted' \
               AND s.tombstoned_at_ms IS NULL",
        )
        .bind(&current)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| {
            DbError::ResultInvariant(
                "campaign aggregate chain contains a missing or unpublished run".to_owned(),
            )
        })?;
        let predecessor: Option<String> = row.try_get("predecessor_run_id")?;
        reversed.push(row);
        let Some(predecessor) = predecessor else {
            break;
        };
        current = predecessor;
    }
    let exceeds_limit = if reversed.len() == 4_096 {
        reversed
            .last()
            .expect("4096-row chain is nonempty")
            .try_get::<Option<String>, _>("predecessor_run_id")?
            .is_some()
    } else {
        false
    };
    if reversed.is_empty() || exceeds_limit {
        return Err(DbError::ResultInvariant(
            "campaign aggregate chain is empty or exceeds 4096 sessions".to_owned(),
        ));
    }
    reversed.reverse();

    Ok(reversed)
}
#[derive(serde::Serialize, serde::Deserialize)]
struct CampaignSessionDocuments {
    result: VerificationResultV1,
    signed: robin_run_protocol::SignedSubmissionV1,
    public_proof: PublicVerificationProofV1,
}

fn validate_campaign_session_documents(
    row: &SqliteRow,
    genesis_starting_state: Option<&InitialStateExpectationV1>,
    published_ruleset: &PublishedRulesetV1,
    campaign_content_manifest: &CampaignContentManifestV1,
) -> Result<CampaignSessionDocuments, DbError> {
    let stored_result: VerificationResultV1 = serde_json::from_str(
        row.try_get::<String, _>("verification_result_json")?
            .as_str(),
    )
    .map_err(|error| DbError::Corrupt(format!("campaign verification result: {error}")))?;
    stored_result
        .validate()
        .map_err(|error| DbError::Corrupt(format!("campaign verification result: {error}")))?;
    let stored_result_sha256 = stored_result
        .canonical_digest()
        .map_err(|error| DbError::Corrupt(format!("campaign result digest: {error}")))?;
    if stored_result_sha256.as_bytes() != &fixed_32(row.try_get::<Vec<u8>, _>("result_sha256")?)? {
        return Err(DbError::Corrupt(
            "campaign result JSON does not match its stored digest".to_owned(),
        ));
    }
    if !matches!(&stored_result.status, VerificationStatusV1::Verified(_)) {
        return Err(DbError::Corrupt(
            "accepted campaign row does not contain a verified result".to_owned(),
        ));
    }
    let signed: robin_run_protocol::SignedSubmissionV1 =
        serde_json::from_str(row.try_get::<String, _>("envelope_json")?.as_str())
            .map_err(|error| DbError::Corrupt(format!("campaign submission envelope: {error}")))?;
    signed
        .validate()
        .map_err(|error| DbError::Corrupt(format!("campaign submission envelope: {error}")))?;
    if let Some(starting_state) = genesis_starting_state {
        let offer = &signed.submission.offer;
        if &offer.starting_state != starting_state {
            return Err(DbError::Corrupt(
                "campaign genesis offer differs from its indexed starting state".to_owned(),
            ));
        }
        let ranked = &offer.session_genesis.claim.ranked_session;
        validate_aggregate_genesis_scope_subject(
            ranked.content_edition,
            &ranked.content_subject,
            &offer.starting_state,
        )?;
    }
    let stored_request: VerificationRequestV1 = serde_json::from_str(
        row.try_get::<Option<String>, _>("verification_request_json")?
            .ok_or_else(|| {
                DbError::Corrupt(
                    "accepted campaign row has no recorded verification request".to_owned(),
                )
            })?
            .as_str(),
    )
    .map_err(|error| DbError::Corrupt(format!("campaign verification request: {error}")))?;
    let stored_request_sha256 = stored_request
        .canonical_digest()
        .map_err(|error| DbError::Corrupt(format!("campaign request digest: {error}")))?;
    if stored_request.submission != signed
        || stored_request_sha256.as_bytes()
            != &fixed_32(row.try_get::<Vec<u8>, _>("verification_request_sha256")?)?
    {
        return Err(DbError::Corrupt(
            "campaign verification request does not match indexed storage".to_owned(),
        ));
    }
    stored_result
        .validate_campaign_complete_evidence(
            &stored_request,
            &published_ruleset.manifest,
            Some(campaign_content_manifest),
        )
        .map_err(|error| DbError::Corrupt(format!("campaign completion evidence: {error}")))?;
    let stored_public_proof = stored_public_verification_proof(row)?;
    let recomputed_public_proof =
        PublicVerificationProofV1::from_private(&stored_request, &stored_result).map_err(
            |error| DbError::Corrupt(format!("campaign public proof projection: {error}")),
        )?;
    if stored_public_proof != recomputed_public_proof {
        return Err(DbError::Corrupt(
            "stored campaign public proof differs from its immutable private documents".to_owned(),
        ));
    }
    Ok(CampaignSessionDocuments {
        result: stored_result,
        signed,
        public_proof: stored_public_proof,
    })
}

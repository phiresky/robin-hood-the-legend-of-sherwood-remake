//! Public projections and artifact authorization queries. These share the owning Database connection and visibility policy.
//! No independent pool, repository transaction, or fence is created here.
use super::*;

impl Database {
    /// Serve custom settings only after a verifier accepted the signed offer.
    pub async fn public_custom_rules_config(
        &self,
        digest: robin_run_protocol::Digest32,
    ) -> Result<Option<robin_run_protocol::RulesConfigIdentityV1>, DbError> {
        let offer: Option<String> = sqlx::query_scalar(
            "SELECT s.offer_json FROM submissions s JOIN verified_runs r ON r.submission_id = s.id \
             WHERE s.status = 'accepted' AND s.tombstoned_at_ms IS NULL AND r.config_id = ? LIMIT 1",
        ).bind(digest.into_bytes().to_vec()).fetch_optional(&self.pool).await?;
        let Some(offer) = offer else {
            return Ok(None);
        };
        let offer: robin_run_protocol::SubmissionOfferV1 = serde_json::from_str(&offer)
            .map_err(|error| DbError::Corrupt(format!("accepted custom rules offer: {error}")))?;
        let rules = offer
            .session_genesis
            .claim
            .ranked_session
            .custom_rules_config
            .ok_or_else(|| {
                DbError::Corrupt("accepted custom rules offer has no configuration".into())
            })?;
        if rules
            .canonical_digest()
            .map_err(|error| DbError::Corrupt(error.to_string()))?
            != digest
        {
            return Err(DbError::Corrupt(
                "accepted custom rules configuration digest differs".into(),
            ));
        }
        Ok(Some(rules))
    }

    pub async fn leaderboard_rows(
        &self,
        filter: &robin_run_protocol::RunFilterV1,
        cursor: Option<&BoardCursor>,
        limit: u32,
        accepted_sequence_watermark: u64,
        allowed_rulesets: &[robin_run_protocol::Digest32],
    ) -> Result<Vec<BoardRow>, DbError> {
        let metric = match filter.metric {
            robin_run_protocol::BoardMetricV1::OriginalScore => "original_score",
            robin_run_protocol::BoardMetricV1::FastestSuccess => "fastest_success",
        };
        let content_digest = filter.content.digest();
        let (mission_id, scope) = match &filter.subject {
            robin_run_protocol::LeaderboardSubjectV1::Mission {
                mission_id,
                category,
            } => (
                mission_id,
                match category {
                    robin_run_protocol::BoardCategoryV1::IndividualLevel => "individual_level",
                    robin_run_protocol::BoardCategoryV1::Campaign => "campaign",
                },
            ),
            robin_run_protocol::LeaderboardSubjectV1::FullCampaign => {
                return self
                    .full_campaign_leaderboard_rows(
                        filter,
                        cursor,
                        limit,
                        accepted_sequence_watermark,
                        allowed_rulesets,
                    )
                    .await;
            }
        };
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT r.id, s.replay_sha256, m.value, r.max_concurrent_players, \
                    r.participant_instance_count, r.named_participant_instance_count, \
                    r.anonymous_participant_instance_count, r.accepted_sequence, r.verified_at_ms \
             FROM verified_runs r \
             JOIN submissions s ON s.id = r.submission_id \
             JOIN verified_run_metrics m ON m.run_id = r.id \
             WHERE s.status = 'accepted' AND s.tombstoned_at_ms IS NULL \
               AND (r.campaign_session_kind IS NULL \
                    OR r.campaign_session_kind = 'field_mission') AND m.metric = ",
        );
        query
            .push_bind(metric)
            .push(" AND r.scope_kind = ")
            .push_bind(scope)
            .push(" AND r.mission_id = ")
            .push_bind(mission_id)
            .push(" AND r.content_manifest_id = ")
            .push_bind(content_digest.as_bytes().as_slice())
            .push(" AND (")
            .push_bind(
                filter
                    .rules_config_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(" IS NULL OR r.config_id = ")
            .push_bind(
                filter
                    .rules_config_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(")")
            .push(" AND (")
            .push_bind(
                filter
                    .ruleset_manifest_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(" IS NULL OR r.ruleset_id = ")
            .push_bind(
                filter
                    .ruleset_manifest_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(")")
            .push(" AND r.accepted_sequence <= ")
            .push_bind(i64::try_from(accepted_sequence_watermark).map_err(|_| {
                DbError::ResultInvariant("acceptance watermark exceeds i64".to_owned())
            })?);
        query.push(" AND ");
        push_ruleset_filter(
            &mut query,
            "r.ruleset_id",
            allowed_rulesets.iter().map(|digest| digest.into_bytes()),
        );
        match &filter.competition_manifest_sha256 {
            Some(competition) => {
                query
                    .push(" AND r.competition_manifest_id = ")
                    .push_bind(competition.as_bytes().as_slice());
            }
            None => {
                query.push(" AND r.competition_manifest_id IS NULL");
            }
        }
        if let Some(max_concurrent_players) = filter.max_concurrent_players {
            query
                .push(" AND r.max_concurrent_players = ")
                .push_bind(i64::from(max_concurrent_players));
        }
        if let Some(player) = filter.player_public_key {
            query
                .push(
                    " AND EXISTS (SELECT 1 FROM submission_participants player \
                        WHERE player.submission_id = r.submission_id \
                          AND player.public_disclosure = 'named_profile' AND player.public_key = ",
                )
                .push_bind(player.into_bytes().to_vec())
                .push(")");
        }
        if let Some(cursor) = cursor {
            let comparison = match filter.metric {
                robin_run_protocol::BoardMetricV1::OriginalScore => "<",
                robin_run_protocol::BoardMetricV1::FastestSuccess => ">",
            };
            query
                .push(" AND (m.value ")
                .push(comparison)
                .push(" ")
                .push_bind(cursor.metric_value)
                .push(" OR (m.value = ")
                .push_bind(cursor.metric_value)
                .push(" AND (r.accepted_sequence > ")
                .push_bind(cursor.accepted_sequence)
                .push(" OR (r.accepted_sequence = ")
                .push_bind(cursor.accepted_sequence)
                .push(" AND r.id > ")
                .push_bind(&cursor.run_id)
                .push("))))");
        }
        match filter.metric {
            robin_run_protocol::BoardMetricV1::OriginalScore => {
                query.push(" ORDER BY m.value DESC, r.accepted_sequence, r.id");
            }
            robin_run_protocol::BoardMetricV1::FastestSuccess => {
                query.push(" ORDER BY m.value ASC, r.accepted_sequence, r.id");
            }
        }
        query.push(" LIMIT ").push_bind(i64::from(limit));
        let rows = query.build().fetch_all(&self.pool).await?;
        let run_ids = rows
            .iter()
            .map(|row| row.get::<String, _>("id"))
            .collect::<Vec<_>>();
        // Each run appears once: submissions.id is unique and the selected metric
        // has PRIMARY KEY(run_id, metric). Consume each owned projection once.
        let mut participant_map = self.public_participants_for_runs(&run_ids).await?;
        let metric_values = rows
            .iter()
            .map(|row| row.get::<i64, _>("value"))
            .collect::<Vec<_>>();
        let ranks = self
            .mission_ranks(
                filter,
                &metric_values,
                accepted_sequence_watermark,
                allowed_rulesets,
            )
            .await?;
        let mut output = Vec::with_capacity(rows.len());
        for row in rows {
            let run_id: String = row.try_get("id")?;
            let metric_value: i64 = row.try_get("value")?;
            output.push(BoardRow {
                rank: *ranks.get(&metric_value).ok_or_else(|| {
                    DbError::Corrupt("batch mission rank omitted a requested value".to_owned())
                })?,
                // Fully anonymous runs legitimately have no public participants.
                named_participants: participant_map.remove(&run_id).unwrap_or_default(),
                aggregate_named_participants: Vec::new(),
                run_id: run_id.clone(),
                composition: BoardComposition::Mission {
                    replay_sha256: fixed_32(row.try_get("replay_sha256")?)?,
                },
                metric_value,
                max_concurrent_players: checked_count(
                    &row,
                    "max_concurrent_players",
                    "player count out of range",
                )?,
                participant_instance_count: checked_count(
                    &row,
                    "participant_instance_count",
                    "participant instance count out of range",
                )?,
                named_participant_instance_count: checked_count(
                    &row,
                    "named_participant_instance_count",
                    "named participant count out of range",
                )?,
                anonymous_participant_instance_count: checked_count(
                    &row,
                    "anonymous_participant_instance_count",
                    "anonymous participant count out of range",
                )?,
                accepted_sequence: nonnegative_u64(
                    row.try_get("accepted_sequence")?,
                    "accepted_sequence",
                )?,
                verified_at_ms: nonnegative_u64(row.try_get("verified_at_ms")?, "verified_at_ms")?,
            });
        }
        Ok(output)
    }

    pub(super) async fn full_campaign_leaderboard_rows(
        &self,
        filter: &robin_run_protocol::RunFilterV1,
        cursor: Option<&BoardCursor>,
        limit: u32,
        accepted_sequence_watermark: u64,
        allowed_rulesets: &[robin_run_protocol::Digest32],
    ) -> Result<Vec<BoardRow>, DbError> {
        let metric = match filter.metric {
            robin_run_protocol::BoardMetricV1::OriginalScore => "original_score",
            robin_run_protocol::BoardMetricV1::FastestSuccess => "fastest_success",
        };
        let content_digest = filter.content.digest();
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT fc.id, m.value, fc.max_concurrent_players, \
                    fc.participant_instance_count, fc.named_participant_instance_count, \
                    fc.anonymous_participant_instance_count, fc.accepted_sequence, fc.verified_at_ms \
             FROM full_campaign_runs fc \
             JOIN full_campaign_metrics m ON m.full_campaign_run_id = fc.id \
             WHERE fc.tombstoned_at_ms IS NULL \
               AND m.metric = ",
        );
        query
            .push_bind(metric)
            .push(" AND fc.campaign_content_manifest_id = ")
            .push_bind(content_digest.as_bytes().as_slice())
            .push(" AND (")
            .push_bind(
                filter
                    .rules_config_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(" IS NULL OR fc.config_id = ")
            .push_bind(
                filter
                    .rules_config_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(")")
            .push(" AND (")
            .push_bind(
                filter
                    .ruleset_manifest_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(" IS NULL OR fc.ruleset_id = ")
            .push_bind(
                filter
                    .ruleset_manifest_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(")")
            .push(" AND fc.accepted_sequence <= ")
            .push_bind(i64::try_from(accepted_sequence_watermark).map_err(|_| {
                DbError::ResultInvariant("acceptance watermark exceeds i64".to_owned())
            })?)
            .push(
                " AND NOT EXISTS (SELECT 1 FROM full_campaign_sessions fcs \
                   JOIN verified_runs vr ON vr.id = fcs.run_id \
                   JOIN submissions s ON s.id = vr.submission_id \
                   WHERE fcs.full_campaign_run_id = fc.id \
                     AND s.tombstoned_at_ms IS NOT NULL)",
            );
        query.push(" AND ");
        push_ruleset_filter(
            &mut query,
            "fc.ruleset_id",
            allowed_rulesets.iter().map(|digest| digest.into_bytes()),
        );
        match &filter.competition_manifest_sha256 {
            Some(competition) => {
                query
                    .push(" AND fc.competition_manifest_id = ")
                    .push_bind(competition.as_bytes().as_slice());
            }
            None => {
                query.push(" AND fc.competition_manifest_id IS NULL");
            }
        }
        if let Some(max_concurrent_players) = filter.max_concurrent_players {
            query
                .push(" AND fc.max_concurrent_players = ")
                .push_bind(i64::from(max_concurrent_players));
        }
        if let Some(player) = filter.player_public_key {
            query
                .push(
                    " AND EXISTS (SELECT 1 FROM full_campaign_participants player \
                        WHERE player.full_campaign_run_id = fc.id AND player.public_key = ",
                )
                .push_bind(player.into_bytes().to_vec())
                .push(")");
        }
        if let Some(cursor) = cursor {
            let comparison = match filter.metric {
                robin_run_protocol::BoardMetricV1::OriginalScore => "<",
                robin_run_protocol::BoardMetricV1::FastestSuccess => ">",
            };
            query
                .push(" AND (m.value ")
                .push(comparison)
                .push(" ")
                .push_bind(cursor.metric_value)
                .push(" OR (m.value = ")
                .push_bind(cursor.metric_value)
                .push(" AND (fc.accepted_sequence > ")
                .push_bind(cursor.accepted_sequence)
                .push(" OR (fc.accepted_sequence = ")
                .push_bind(cursor.accepted_sequence)
                .push(" AND fc.id > ")
                .push_bind(&cursor.run_id)
                .push("))))");
        }
        match filter.metric {
            robin_run_protocol::BoardMetricV1::OriginalScore => {
                query.push(" ORDER BY m.value DESC, fc.accepted_sequence, fc.id");
            }
            robin_run_protocol::BoardMetricV1::FastestSuccess => {
                query.push(" ORDER BY m.value ASC, fc.accepted_sequence, fc.id");
            }
        }
        query.push(" LIMIT ").push_bind(i64::from(limit));
        let rows = query.build().fetch_all(&self.pool).await?;
        let run_ids = rows
            .iter()
            .map(|row| row.get::<String, _>("id"))
            .collect::<Vec<_>>();
        // The selected metric has PRIMARY KEY(full_campaign_run_id, metric),
        // so each campaign id consumes its own projections exactly once.
        let mut session_map = self.full_campaign_sessions_for_runs(&run_ids).await?;
        let mut participant_map = self
            .public_participants_for_full_campaigns(&run_ids)
            .await?;
        let metric_values = rows
            .iter()
            .map(|row| row.get::<i64, _>("value"))
            .collect::<Vec<_>>();
        let ranks = self
            .full_campaign_ranks(
                filter,
                &metric_values,
                accepted_sequence_watermark,
                allowed_rulesets,
            )
            .await?;
        let mut output = Vec::with_capacity(rows.len());
        for row in rows {
            let run_id: String = row.try_get("id")?;
            let metric_value: i64 = row.try_get("value")?;
            output.push(BoardRow {
                rank: *ranks.get(&metric_value).ok_or_else(|| {
                    DbError::Corrupt("batch campaign rank omitted a requested value".to_owned())
                })?,
                named_participants: Vec::new(),
                aggregate_named_participants: participant_map.remove(&run_id).unwrap_or_default(),
                run_id: run_id.clone(),
                composition: BoardComposition::FullCampaign {
                    ordered_session_run_ids: session_map.remove(&run_id).unwrap_or_default(),
                },
                metric_value,
                max_concurrent_players: checked_count(
                    &row,
                    "max_concurrent_players",
                    "player count out of range",
                )?,
                participant_instance_count: checked_count(
                    &row,
                    "participant_instance_count",
                    "participant instance count out of range",
                )?,
                named_participant_instance_count: checked_count(
                    &row,
                    "named_participant_instance_count",
                    "named participant count out of range",
                )?,
                anonymous_participant_instance_count: checked_count(
                    &row,
                    "anonymous_participant_instance_count",
                    "anonymous participant count out of range",
                )?,
                accepted_sequence: nonnegative_u64(
                    row.try_get("accepted_sequence")?,
                    "accepted_sequence",
                )?,
                verified_at_ms: nonnegative_u64(row.try_get("verified_at_ms")?, "verified_at_ms")?,
            });
        }
        Ok(output)
    }

    pub(super) async fn mission_ranks(
        &self,
        filter: &robin_run_protocol::RunFilterV1,
        values: &[i64],
        accepted_sequence_watermark: u64,
        allowed_rulesets: &[robin_run_protocol::Digest32],
    ) -> Result<BTreeMap<i64, u64>, DbError> {
        let unique = values.iter().copied().collect::<BTreeSet<_>>();
        if unique.is_empty() {
            return Ok(BTreeMap::new());
        }
        let metric = match filter.metric {
            robin_run_protocol::BoardMetricV1::OriginalScore => "original_score",
            robin_run_protocol::BoardMetricV1::FastestSuccess => "fastest_success",
        };
        let content_digest = filter.content.digest();
        let (mission_id, scope) = match &filter.subject {
            robin_run_protocol::LeaderboardSubjectV1::Mission {
                mission_id,
                category,
            } => (
                mission_id,
                match category {
                    robin_run_protocol::BoardCategoryV1::IndividualLevel => "individual_level",
                    robin_run_protocol::BoardCategoryV1::Campaign => "campaign",
                },
            ),
            robin_run_protocol::LeaderboardSubjectV1::FullCampaign => {
                return Err(DbError::ResultInvariant(
                    "mission rank received campaign filter".to_owned(),
                ));
            }
        };
        let mut query = QueryBuilder::<Sqlite>::new("WITH candidates(value) AS (VALUES ");
        {
            let mut separated = query.separated(", ");
            for value in unique {
                separated
                    .push("(")
                    .push_bind_unseparated(value)
                    .push_unseparated(")");
            }
        }
        query.push(
            "), board(value) AS MATERIALIZED (SELECT metric.value FROM verified_runs run \
             JOIN submissions submission ON submission.id = run.submission_id \
             JOIN verified_run_metrics metric ON metric.run_id = run.id \
             WHERE submission.status = 'accepted' AND submission.tombstoned_at_ms IS NULL \
               AND (run.campaign_session_kind IS NULL \
                    OR run.campaign_session_kind = 'field_mission') \
               AND metric.metric = ",
        );
        query
            .push_bind(metric)
            .push(" AND run.scope_kind = ")
            .push_bind(scope)
            .push(" AND run.mission_id = ")
            .push_bind(mission_id)
            .push(" AND run.content_manifest_id = ")
            .push_bind(content_digest.as_bytes().as_slice())
            .push(" AND (")
            .push_bind(
                filter
                    .rules_config_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(" IS NULL OR run.config_id = ")
            .push_bind(
                filter
                    .rules_config_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(")")
            .push(" AND (")
            .push_bind(
                filter
                    .ruleset_manifest_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(" IS NULL OR run.ruleset_id = ")
            .push_bind(
                filter
                    .ruleset_manifest_sha256
                    .map(|digest| digest.into_bytes().to_vec()),
            )
            .push(")")
            .push(" AND run.accepted_sequence <= ")
            .push_bind(i64::try_from(accepted_sequence_watermark).map_err(|_| {
                DbError::ResultInvariant("acceptance watermark exceeds i64".to_owned())
            })?);
        query.push(" AND ");
        push_ruleset_filter(
            &mut query,
            "run.ruleset_id",
            allowed_rulesets.iter().map(|digest| digest.into_bytes()),
        );
        match filter.competition_manifest_sha256 {
            Some(competition) => {
                query
                    .push(" AND run.competition_manifest_id = ")
                    .push_bind(competition.into_bytes().to_vec());
            }
            None => {
                query.push(" AND run.competition_manifest_id IS NULL");
            }
        }
        if let Some(players) = filter.max_concurrent_players {
            query
                .push(" AND run.max_concurrent_players = ")
                .push_bind(i64::from(players));
        }
        if let Some(player) = filter.player_public_key {
            query.push(" AND EXISTS (SELECT 1 FROM submission_participants participant WHERE participant.submission_id = run.submission_id AND participant.public_disclosure = 'named_profile' AND participant.public_key = ")
                .push_bind(player.into_bytes().to_vec()).push(")");
        }
        let comparison = match filter.metric {
            robin_run_protocol::BoardMetricV1::OriginalScore => ">",
            robin_run_protocol::BoardMetricV1::FastestSuccess => "<",
        };
        query.push(") SELECT candidates.value, COUNT(board.value) + 1 AS rank FROM candidates LEFT JOIN board ON board.value ")
            .push(comparison).push(" candidates.value GROUP BY candidates.value");
        let mut ranks = BTreeMap::new();
        for row in query.build().fetch_all(&self.pool).await? {
            ranks.insert(
                row.try_get("value")?,
                nonnegative_u64(row.try_get("rank")?, "rank")?,
            );
        }
        Ok(ranks)
    }

    pub(super) async fn full_campaign_ranks(
        &self,
        filter: &robin_run_protocol::RunFilterV1,
        values: &[i64],
        accepted_sequence_watermark: u64,
        allowed_rulesets: &[robin_run_protocol::Digest32],
    ) -> Result<BTreeMap<i64, u64>, DbError> {
        let unique = values.iter().copied().collect::<BTreeSet<_>>();
        if unique.is_empty() {
            return Ok(BTreeMap::new());
        }
        let metric = match filter.metric {
            robin_run_protocol::BoardMetricV1::OriginalScore => "original_score",
            robin_run_protocol::BoardMetricV1::FastestSuccess => "fastest_success",
        };
        let content_digest = filter.content.digest();
        let mut query = QueryBuilder::<Sqlite>::new("WITH candidates(value) AS (VALUES ");
        {
            let mut separated = query.separated(", ");
            for value in unique {
                separated
                    .push("(")
                    .push_bind_unseparated(value)
                    .push_unseparated(")");
            }
        }
        query.push(
            "), board(value) AS MATERIALIZED (SELECT metric.value FROM full_campaign_runs run \
             JOIN full_campaign_metrics metric ON metric.full_campaign_run_id = run.id \
             WHERE run.tombstoned_at_ms IS NULL AND metric.metric = ",
        );
        query
            .push_bind(metric)
            .push(" AND run.campaign_content_manifest_id = ").push_bind(content_digest.as_bytes().as_slice())
            .push(" AND (").push_bind(filter.rules_config_sha256.map(|digest| digest.into_bytes().to_vec())).push(" IS NULL OR run.config_id = ").push_bind(filter.rules_config_sha256.map(|digest| digest.into_bytes().to_vec())).push(")")
            .push(" AND (").push_bind(filter.ruleset_manifest_sha256.map(|digest| digest.into_bytes().to_vec())).push(" IS NULL OR run.ruleset_id = ").push_bind(filter.ruleset_manifest_sha256.map(|digest| digest.into_bytes().to_vec())).push(")")
            .push(" AND run.accepted_sequence <= ").push_bind(i64::try_from(accepted_sequence_watermark).map_err(|_| DbError::ResultInvariant("acceptance watermark exceeds i64".to_owned()))?)
            .push(" AND NOT EXISTS (SELECT 1 FROM full_campaign_sessions session JOIN verified_runs child ON child.id = session.run_id JOIN submissions submission ON submission.id = child.submission_id WHERE session.full_campaign_run_id = run.id AND submission.tombstoned_at_ms IS NOT NULL)");
        query.push(" AND ");
        push_ruleset_filter(
            &mut query,
            "run.ruleset_id",
            allowed_rulesets.iter().map(|digest| digest.into_bytes()),
        );
        match filter.competition_manifest_sha256 {
            Some(competition) => {
                query
                    .push(" AND run.competition_manifest_id = ")
                    .push_bind(competition.into_bytes().to_vec());
            }
            None => {
                query.push(" AND run.competition_manifest_id IS NULL");
            }
        }
        if let Some(players) = filter.max_concurrent_players {
            query
                .push(" AND run.max_concurrent_players = ")
                .push_bind(i64::from(players));
        }
        if let Some(player) = filter.player_public_key {
            query.push(" AND EXISTS (SELECT 1 FROM full_campaign_participants participant WHERE participant.full_campaign_run_id = run.id AND participant.public_key = ")
                .push_bind(player.into_bytes().to_vec()).push(")");
        }
        let comparison = match filter.metric {
            robin_run_protocol::BoardMetricV1::OriginalScore => ">",
            robin_run_protocol::BoardMetricV1::FastestSuccess => "<",
        };
        query.push(") SELECT candidates.value, COUNT(board.value) + 1 AS rank FROM candidates LEFT JOIN board ON board.value ")
            .push(comparison).push(" candidates.value GROUP BY candidates.value");
        let mut ranks = BTreeMap::new();
        for row in query.build().fetch_all(&self.pool).await? {
            ranks.insert(
                row.try_get("value")?,
                nonnegative_u64(row.try_get("rank")?, "rank")?,
            );
        }
        Ok(ranks)
    }

    pub async fn player_run_history(
        &self,
        public_key: &[u8; 32],
        active_ruleset_ids: &[[u8; 32]],
        accepted_sequence_watermark: u64,
        cursor: Option<(u64, &str)>,
        limit: u32,
    ) -> Result<Vec<PlayerHistoryRecord>, DbError> {
        if limit == 0 || limit > 101 {
            return Err(DbError::ResultInvariant(
                "player history limit must be in 1..=101".to_owned(),
            ));
        }
        let watermark = i64::try_from(accepted_sequence_watermark)
            .map_err(|_| DbError::ResultInvariant("acceptance watermark exceeds i64".to_owned()))?;
        let cursor_sequence = cursor
            .map(|(sequence, _)| i64::try_from(sequence))
            .transpose()
            .map_err(|_| DbError::ResultInvariant("history cursor exceeds i64".to_owned()))?;
        let cursor_id = cursor.map(|(_, id)| id);
        let mut query = QueryBuilder::<Sqlite>::new("");
        query.push("WITH history AS ( SELECT 'mission' AS composition_kind, run.id, run.mission_id, run.scope_kind, submission.replay_sha256, run.original_score_delta, run.active_simulation_ticks, run.ransom_collected, run.content_manifest_id, run.config_id, run.ruleset_id, run.competition_manifest_id, run.max_concurrent_players, run.participant_instance_count, run.accepted_sequence, run.verified_at_ms FROM verified_runs run JOIN submissions submission ON submission.id = run.submission_id WHERE submission.status = 'accepted' AND submission.tombstoned_at_ms IS NULL AND ");
        push_ruleset_filter(
            &mut query,
            "run.ruleset_id",
            active_ruleset_ids.iter().copied(),
        );
        query.push(" AND (run.campaign_session_kind IS NULL OR run.campaign_session_kind = 'field_mission') AND EXISTS (SELECT 1 FROM submission_participants participant WHERE participant.submission_id = submission.id AND participant.public_disclosure = 'named_profile' AND participant.public_key = ");
        query.push_bind(public_key.as_slice());
        query.push(") UNION ALL SELECT 'full_campaign', aggregate.id, NULL, 'campaign', NULL, aggregate.final_campaign_score - aggregate.starting_campaign_score, aggregate.active_simulation_ticks, aggregate.ransom_collected, aggregate.campaign_content_manifest_id AS content_manifest_id, aggregate.config_id, aggregate.ruleset_id, aggregate.competition_manifest_id, aggregate.max_concurrent_players, aggregate.participant_instance_count, aggregate.accepted_sequence, aggregate.verified_at_ms FROM full_campaign_runs aggregate WHERE aggregate.tombstoned_at_ms IS NULL AND ");
        push_ruleset_filter(
            &mut query,
            "aggregate.ruleset_id",
            active_ruleset_ids.iter().copied(),
        );
        query.push(" AND EXISTS (SELECT 1 FROM full_campaign_participants participant WHERE participant.full_campaign_run_id = aggregate.id AND participant.public_key = ");
        query.push_bind(public_key.as_slice());
        query.push(") AND NOT EXISTS (SELECT 1 FROM full_campaign_sessions session JOIN verified_runs child ON child.id = session.run_id JOIN submissions submission ON submission.id = child.submission_id WHERE session.full_campaign_run_id = aggregate.id AND submission.tombstoned_at_ms IS NOT NULL) ) SELECT * FROM history WHERE accepted_sequence <= ");
        query.push_bind(watermark);
        query.push(" AND (");
        query.push_bind(cursor_sequence);
        query.push(" IS NULL OR accepted_sequence < ");
        query.push_bind(cursor_sequence);
        query.push(" OR (accepted_sequence = ");
        query.push_bind(cursor_sequence);
        query.push(" AND id > ");
        query.push_bind(cursor_id);
        query.push(")) ORDER BY accepted_sequence DESC, id LIMIT ");
        query.push_bind(i64::from(limit));
        let rows = query.build().fetch_all(&self.pool).await?;
        let mission_ids = rows
            .iter()
            .filter(|row| row.get::<String, _>("composition_kind") == "mission")
            .map(|row| row.get::<String, _>("id"))
            .collect::<Vec<_>>();
        let aggregate_ids = rows
            .iter()
            .filter(|row| row.get::<String, _>("composition_kind") == "full_campaign")
            .map(|row| row.get::<String, _>("id"))
            .collect::<Vec<_>>();
        // Each UNION ALL arm selects primary-key ids with EXISTS filters, without
        // multiplying rows. Separate maps keep the two id namespaces independent.
        let mut mission_participants = self.public_participants_for_runs(&mission_ids).await?;
        let mut aggregate_participants = self
            .public_participants_for_full_campaigns(&aggregate_ids)
            .await?;
        let mut aggregate_sessions = self.full_campaign_sessions_for_runs(&aggregate_ids).await?;
        rows.into_iter()
            .map(|row| {
                let run_id: String = row.try_get("id")?;
                let composition_kind: String = row.try_get("composition_kind")?;
                let (composition, named_participants, aggregate_named_participants) =
                    match composition_kind.as_str() {
                        "mission" => (
                            BoardComposition::Mission {
                                replay_sha256: fixed_32(row.try_get("replay_sha256")?)?,
                            },
                            mission_participants.remove(&run_id).unwrap_or_default(),
                            Vec::new(),
                        ),
                        "full_campaign" => (
                            BoardComposition::FullCampaign {
                                ordered_session_run_ids: aggregate_sessions
                                    .remove(&run_id)
                                    .unwrap_or_default(),
                            },
                            Vec::new(),
                            aggregate_participants.remove(&run_id).unwrap_or_default(),
                        ),
                        _ => {
                            return Err(DbError::Corrupt(
                                "player history contains invalid composition".to_owned(),
                            ));
                        }
                    };
                Ok(PlayerHistoryRecord {
                    run_id,
                    composition,
                    mission_id: row.try_get("mission_id")?,
                    scope_kind: row.try_get("scope_kind")?,
                    original_score_delta: row.try_get("original_score_delta")?,
                    active_simulation_ticks: nonnegative_u64(
                        row.try_get("active_simulation_ticks")?,
                        "active_simulation_ticks",
                    )?,
                    ransom_collected: nonnegative_u64(
                        row.try_get("ransom_collected")?,
                        "ransom_collected",
                    )?,
                    content_manifest_id: fixed_32(row.try_get("content_manifest_id")?)?,
                    config_id: fixed_32(row.try_get("config_id")?)?,
                    ruleset_id: fixed_32(row.try_get("ruleset_id")?)?,
                    competition_manifest_id: row
                        .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
                        .map(fixed_32)
                        .transpose()?,
                    max_concurrent_players: checked_count(
                        &row,
                        "max_concurrent_players",
                        "player count exceeds u16",
                    )?,
                    participant_instance_count: checked_count(
                        &row,
                        "participant_instance_count",
                        "participant count exceeds u32",
                    )?,
                    accepted_sequence: nonnegative_u64(
                        row.try_get("accepted_sequence")?,
                        "accepted_sequence",
                    )?,
                    verified_at_ms: nonnegative_u64(
                        row.try_get("verified_at_ms")?,
                        "verified_at_ms",
                    )?,
                    named_participants,
                    aggregate_named_participants,
                })
            })
            .collect()
    }

    pub async fn player_personal_bests(
        &self,
        public_key: &[u8; 32],
        active_ruleset_ids: &[[u8; 32]],
        accepted_sequence_watermark: u64,
    ) -> Result<Vec<PlayerBestRecord>, DbError> {
        let watermark = i64::try_from(accepted_sequence_watermark)
            .map_err(|_| DbError::ResultInvariant("acceptance watermark exceeds i64".to_owned()))?;
        let mut query = QueryBuilder::<Sqlite>::new("");
        query.push("WITH candidates AS ( SELECT 'mission' AS subject_kind, run.id, run.mission_id, run.scope_kind, metric.metric, metric.value, run.content_manifest_id, run.config_id, run.ruleset_id, run.competition_manifest_id, run.max_concurrent_players, run.accepted_sequence FROM verified_runs run JOIN submissions submission ON submission.id = run.submission_id JOIN verified_run_metrics metric ON metric.run_id = run.id WHERE submission.status = 'accepted' AND submission.tombstoned_at_ms IS NULL AND ");
        push_ruleset_filter(
            &mut query,
            "run.ruleset_id",
            active_ruleset_ids.iter().copied(),
        );
        query.push(" AND run.accepted_sequence <= ");
        query.push_bind(watermark);
        query.push(" AND (run.campaign_session_kind IS NULL OR run.campaign_session_kind = 'field_mission') AND EXISTS (SELECT 1 FROM submission_participants participant WHERE participant.submission_id = submission.id AND participant.public_disclosure = 'named_profile' AND participant.public_key = ");
        query.push_bind(public_key.as_slice());
        query.push(") UNION ALL SELECT 'full_campaign', aggregate.id, NULL, 'campaign', metric.metric, metric.value, aggregate.campaign_content_manifest_id AS content_manifest_id, aggregate.config_id, aggregate.ruleset_id, aggregate.competition_manifest_id, aggregate.max_concurrent_players, aggregate.accepted_sequence FROM full_campaign_runs aggregate JOIN full_campaign_metrics metric ON metric.full_campaign_run_id = aggregate.id WHERE aggregate.tombstoned_at_ms IS NULL AND aggregate.accepted_sequence <= ");
        query.push_bind(watermark);
        query.push(" AND ");
        push_ruleset_filter(
            &mut query,
            "aggregate.ruleset_id",
            active_ruleset_ids.iter().copied(),
        );
        query.push(" AND EXISTS (SELECT 1 FROM full_campaign_participants participant WHERE participant.full_campaign_run_id = aggregate.id AND participant.public_key = ");
        query.push_bind(public_key.as_slice());
        query.push(") AND NOT EXISTS (SELECT 1 FROM full_campaign_sessions session JOIN verified_runs child ON child.id = session.run_id JOIN submissions submission ON submission.id = child.submission_id WHERE session.full_campaign_run_id = aggregate.id AND submission.tombstoned_at_ms IS NOT NULL) ), ranked AS ( SELECT *, ROW_NUMBER() OVER (PARTITION BY subject_kind, mission_id, scope_kind, metric, content_manifest_id, config_id, ruleset_id, competition_manifest_id, max_concurrent_players ORDER BY CASE WHEN metric = 'original_score' THEN value END DESC, CASE WHEN metric = 'fastest_success' THEN value END ASC, accepted_sequence, id) AS position FROM candidates ) SELECT * FROM ranked WHERE position = 1 ORDER BY subject_kind, mission_id, scope_kind, metric, content_manifest_id, config_id, ruleset_id, competition_manifest_id, max_concurrent_players LIMIT 512");
        let rows = query.build().fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok(PlayerBestRecord {
                    run_id: row.try_get("id")?,
                    mission_id: row.try_get("mission_id")?,
                    scope_kind: row.try_get("scope_kind")?,
                    metric: row.try_get("metric")?,
                    value: row.try_get("value")?,
                    content_manifest_id: fixed_32(row.try_get("content_manifest_id")?)?,
                    config_id: fixed_32(row.try_get("config_id")?)?,
                    ruleset_id: fixed_32(row.try_get("ruleset_id")?)?,
                    competition_manifest_id: row
                        .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
                        .map(fixed_32)
                        .transpose()?,
                    max_concurrent_players: checked_count(
                        &row,
                        "max_concurrent_players",
                        "player count exceeds u16",
                    )?,
                })
            })
            .collect()
    }

    /// Load a run's validated public projection for public endpoint assembly.
    ///
    /// This deliberately includes headquarters sessions because a visible
    /// full-campaign aggregate must validate every ordered child. Callers that
    /// serve raw run IDs remain responsible for hiding headquarters and
    /// terminal sessions; replay and campaign-object raw-ID queries enforce
    /// that restriction independently in SQL.
    pub async fn public_run(&self, run_id: &str) -> Result<PublicRunRecord, DbError> {
        let row = sqlx::query(
            "SELECT r.id, s.replay_sha256, s.replay_bytes, r.build_manifest_id, \
                    r.content_manifest_id, r.campaign_content_manifest_id, r.config_id, r.ruleset_id, r.mission_id, \
                    r.scope_kind, r.competition_manifest_id, \
                    (r.campaign_chain_id IS NOT NULL) AS has_campaign_chain, \
                    fc.id AS full_campaign_run_id, \
                    EXISTS (SELECT 1 FROM full_campaign_runs terminal_aggregate \
                        WHERE terminal_aggregate.terminal_run_id = r.id) AS campaign_terminal, \
                    r.starting_campaign_sha256, r.starting_campaign_bytes, \
                    r.final_campaign_sha256, r.final_campaign_bytes, \
                    r.result_sha256, r.input_provenance_json, \
                    r.verification_request_sha256, \
                    r.public_verification_request_sha256, r.public_verification_request_json, \
                    r.public_verification_result_sha256, r.public_verification_result_json, \
                    r.public_projection_binding_json, \
                    r.original_score_delta, r.active_simulation_ticks, r.ransom_collected, \
                    r.starting_campaign_score, r.final_campaign_score, r.campaign_session_kind, \
                    r.campaign_session_ordinal, r.campaign_hq_sequence, \
                    r.max_concurrent_players, \
                    r.participant_instance_count, r.named_participant_instance_count, \
                    r.anonymous_participant_instance_count, \
                    r.verified_at_ms, s.public_metadata_json \
             FROM verified_runs r JOIN submissions s ON s.id = r.submission_id \
             LEFT JOIN full_campaign_runs fc ON fc.chain_id = r.campaign_chain_id \
               AND fc.tombstoned_at_ms IS NULL \
               AND NOT EXISTS (SELECT 1 FROM full_campaign_sessions linked_session \
                   JOIN verified_runs linked_run ON linked_run.id = linked_session.run_id \
                   JOIN submissions linked_submission \
                     ON linked_submission.id = linked_run.submission_id \
                     WHERE linked_session.full_campaign_run_id = fc.id \
                     AND linked_submission.tombstoned_at_ms IS NOT NULL) \
             WHERE r.id = ? AND s.status = 'accepted' AND s.tombstoned_at_ms IS NULL",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        let scope_kind: String = row.try_get("scope_kind")?;
        let has_campaign_chain = row.try_get::<i64, _>("has_campaign_chain")? != 0;
        match (scope_kind.as_str(), has_campaign_chain) {
            ("individual_level", false) | ("campaign", true) => {}
            _ => {
                return Err(DbError::Corrupt(
                    "run has inconsistent scope and campaign chain ID".to_owned(),
                ));
            }
        }
        let verification_proof = stored_public_verification_proof(&row)?;
        let record = PublicRunRecord {
            run_id: row.try_get("id")?,
            replay_sha256: fixed_32(row.try_get("replay_sha256")?)?,
            replay_bytes: nonnegative_u64(row.try_get("replay_bytes")?, "replay_bytes")?,
            build_manifest_id: fixed_32(row.try_get("build_manifest_id")?)?,
            content_manifest_id: fixed_32(row.try_get("content_manifest_id")?)?,
            campaign_content_manifest_id: row
                .try_get::<Option<Vec<u8>>, _>("campaign_content_manifest_id")?
                .map(fixed_32)
                .transpose()?,
            config_id: fixed_32(row.try_get("config_id")?)?,
            ruleset_id: fixed_32(row.try_get("ruleset_id")?)?,
            mission_id: row.try_get("mission_id")?,
            scope_kind,
            competition_manifest_id: row
                .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
                .map(fixed_32)
                .transpose()?,
            full_campaign_run_id: row.try_get("full_campaign_run_id")?,
            campaign_terminal: row.try_get::<i64, _>("campaign_terminal")? != 0,
            starting_campaign_sha256: fixed_32(row.try_get("starting_campaign_sha256")?)?,
            starting_campaign_bytes: nonnegative_u64(
                row.try_get("starting_campaign_bytes")?,
                "starting_campaign_bytes",
            )?,
            final_campaign_sha256: fixed_32(row.try_get("final_campaign_sha256")?)?,
            final_campaign_bytes: nonnegative_u64(
                row.try_get("final_campaign_bytes")?,
                "final_campaign_bytes",
            )?,
            public_verification_request_sha256: fixed_32(
                row.try_get("public_verification_request_sha256")?,
            )?,
            public_verification_result_sha256: fixed_32(
                row.try_get("public_verification_result_sha256")?,
            )?,
            verification_proof,
            input_provenance_json: row.try_get("input_provenance_json")?,
            original_score_delta: row.try_get("original_score_delta")?,
            active_simulation_ticks: nonnegative_u64(
                row.try_get("active_simulation_ticks")?,
                "active_simulation_ticks",
            )?,
            ransom_collected: nonnegative_u64(
                row.try_get("ransom_collected")?,
                "ransom_collected",
            )?,
            starting_campaign_score: i32::try_from(
                row.try_get::<i64, _>("starting_campaign_score")?,
            )
            .map_err(|_| DbError::Corrupt("starting campaign score exceeds i32".to_owned()))?,
            final_campaign_score: i32::try_from(row.try_get::<i64, _>("final_campaign_score")?)
                .map_err(|_| DbError::Corrupt("final campaign score exceeds i32".to_owned()))?,
            campaign_session_kind: row.try_get("campaign_session_kind")?,
            campaign_session_ordinal: optional_u32(&row, "campaign_session_ordinal")?,
            campaign_hq_sequence: optional_u32(&row, "campaign_hq_sequence")?,
            max_concurrent_players: checked_count(
                &row,
                "max_concurrent_players",
                "player count out of range",
            )?,
            participant_instance_count: checked_count(
                &row,
                "participant_instance_count",
                "participant instance count out of range",
            )?,
            named_participant_instance_count: checked_count(
                &row,
                "named_participant_instance_count",
                "named participant count out of range",
            )?,
            anonymous_participant_instance_count: checked_count(
                &row,
                "anonymous_participant_instance_count",
                "anonymous participant count out of range",
            )?,
            verified_at_ms: nonnegative_u64(row.try_get("verified_at_ms")?, "verified_at_ms")?,
            public_metadata_json: row.try_get("public_metadata_json")?,
            named_participants: self.public_participants_for_run(run_id).await?,
        };
        let proof = &record.verification_proof;
        let request = &proof.public_request;
        let indexed_provenance: InputProvenanceStatusV1 =
            serde_json::from_str(&record.input_provenance_json)
                .map_err(|error| DbError::Corrupt(format!("input provenance JSON: {error}")))?;
        let indexed_session_kind = match (
            record.campaign_session_kind.as_deref(),
            record.campaign_hq_sequence,
        ) {
            (None, None) => None,
            (Some("field_mission"), None) => Some(CampaignSessionKindV1::FieldMission {
                mission_id: record.mission_id.clone(),
            }),
            (Some("headquarters"), Some(hq_sequence)) => {
                Some(CampaignSessionKindV1::Headquarters { hq_sequence })
            }
            _ => {
                return Err(DbError::Corrupt(
                    "indexed campaign session kind is inconsistent".to_owned(),
                ));
            }
        };
        if request.replay.artifact.sha256.as_bytes() != &record.replay_sha256
            || request.replay.artifact.byte_length != record.replay_bytes
            || request.build_manifest_sha256.as_bytes() != &record.build_manifest_id
            || request.content_manifest_sha256.as_bytes() != &record.content_manifest_id
            || request
                .campaign_content_manifest_sha256
                .map(robin_run_protocol::Digest32::into_bytes)
                != record.campaign_content_manifest_id
            || request.rules_config_sha256.as_bytes() != &record.config_id
            || request.ruleset_manifest_sha256.as_bytes() != &record.ruleset_id
            || request
                .competition_manifest_sha256
                .map(robin_run_protocol::Digest32::into_bytes)
                != record.competition_manifest_id
            || proof.starting_campaign.sha256.as_bytes() != &record.starting_campaign_sha256
            || proof.starting_campaign.byte_length != record.starting_campaign_bytes
            || proof.final_campaign.sha256.as_bytes() != &record.final_campaign_sha256
            || proof.final_campaign.byte_length != record.final_campaign_bytes
            || proof.input_provenance != indexed_provenance
            || proof.metrics.original_score_delta != record.original_score_delta
            || proof.metrics.active_simulation_ticks != record.active_simulation_ticks
            || proof.metrics.ransom_collected != record.ransom_collected
            || proof.starting_campaign_score != record.starting_campaign_score
            || proof.final_campaign_score != record.final_campaign_score
            || proof.campaign_session_kind != indexed_session_kind
            || proof.campaign_session_ordinal != record.campaign_session_ordinal
            || u32::from(proof.participant_instance_count) != record.participant_instance_count
            || u32::from(proof.named_participant_instance_count)
                != record.named_participant_instance_count
            || u32::from(proof.anonymous_participant_instance_count)
                != record.anonymous_participant_instance_count
            || proof.max_concurrent_players != record.max_concurrent_players
        {
            return Err(DbError::Corrupt(
                "public verification proof differs from indexed storage".to_owned(),
            ));
        }
        Ok(record)
    }

    pub async fn public_full_campaign(
        &self,
        run_id: &str,
    ) -> Result<PublicFullCampaignRecord, DbError> {
        let row = sqlx::query(
            "SELECT id, chain_id, aggregate_request_sha256, aggregate_request_json, \
                    aggregate_sha256, aggregate_json, \
                    public_aggregate_request_sha256, public_aggregate_request_json, \
                    public_aggregate_result_sha256, public_aggregate_result_json, \
                    public_projection_binding_json, \
                    campaign_content_manifest_id, config_id, ruleset_id, competition_manifest_id, \
                    starting_campaign_sha256, starting_campaign_bytes, \
                    final_campaign_sha256, final_campaign_bytes, \
                    starting_campaign_score, final_campaign_score, \
                    active_simulation_ticks, ransom_collected, max_concurrent_players, \
                    participant_instance_count, named_participant_instance_count, \
                    anonymous_participant_instance_count, verified_at_ms \
             FROM full_campaign_runs fc WHERE id = ? AND tombstoned_at_ms IS NULL \
               AND NOT EXISTS (SELECT 1 FROM full_campaign_sessions fcs \
                   JOIN verified_runs vr ON vr.id = fcs.run_id \
                   JOIN submissions s ON s.id = vr.submission_id \
                   WHERE fcs.full_campaign_run_id = fc.id \
                     AND s.tombstoned_at_ms IS NOT NULL)",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        let indexed_private_chain_id: String = row.try_get("chain_id")?;
        let ordered_session_run_ids = sqlx::query_scalar(
            "SELECT run_id FROM full_campaign_sessions \
             WHERE full_campaign_run_id = ? ORDER BY ordinal",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        let (aggregate_proof, private_chain_id) = stored_public_aggregate_proof(&row)?;
        if private_chain_id.as_str() != indexed_private_chain_id {
            return Err(DbError::Corrupt(
                "private aggregate proof differs from indexed campaign chain".to_owned(),
            ));
        }
        let record = PublicFullCampaignRecord {
            run_id: row.try_get("id")?,
            public_aggregate_request_sha256: fixed_32(
                row.try_get("public_aggregate_request_sha256")?,
            )?,
            public_aggregate_result_sha256: fixed_32(
                row.try_get("public_aggregate_result_sha256")?,
            )?,
            aggregate_proof,
            campaign_content_manifest_id: fixed_32(row.try_get("campaign_content_manifest_id")?)?,
            config_id: fixed_32(row.try_get("config_id")?)?,
            ruleset_id: fixed_32(row.try_get("ruleset_id")?)?,
            competition_manifest_id: row
                .try_get::<Option<Vec<u8>>, _>("competition_manifest_id")?
                .map(fixed_32)
                .transpose()?,
            starting_campaign_sha256: fixed_32(row.try_get("starting_campaign_sha256")?)?,
            starting_campaign_bytes: nonnegative_u64(
                row.try_get("starting_campaign_bytes")?,
                "starting_campaign_bytes",
            )?,
            final_campaign_sha256: fixed_32(row.try_get("final_campaign_sha256")?)?,
            final_campaign_bytes: nonnegative_u64(
                row.try_get("final_campaign_bytes")?,
                "final_campaign_bytes",
            )?,
            starting_campaign_score: i32::try_from(
                row.try_get::<i64, _>("starting_campaign_score")?,
            )
            .map_err(|_| DbError::Corrupt("starting campaign score exceeds i32".to_owned()))?,
            final_campaign_score: i32::try_from(row.try_get::<i64, _>("final_campaign_score")?)
                .map_err(|_| DbError::Corrupt("final campaign score exceeds i32".to_owned()))?,
            active_simulation_ticks: nonnegative_u64(
                row.try_get("active_simulation_ticks")?,
                "active_simulation_ticks",
            )?,
            ransom_collected: nonnegative_u64(
                row.try_get("ransom_collected")?,
                "ransom_collected",
            )?,
            max_concurrent_players: checked_count(
                &row,
                "max_concurrent_players",
                "max concurrent players exceeds u16",
            )?,
            participant_instance_count: checked_count(
                &row,
                "participant_instance_count",
                "participant count exceeds u32",
            )?,
            named_participant_instance_count: checked_count(
                &row,
                "named_participant_instance_count",
                "named participant count exceeds u32",
            )?,
            anonymous_participant_instance_count: checked_count(
                &row,
                "anonymous_participant_instance_count",
                "anonymous participant count exceeds u32",
            )?,
            verified_at_ms: nonnegative_u64(row.try_get("verified_at_ms")?, "verified_at_ms")?,
            named_participants: self.public_participants_for_full_campaign(run_id).await?,
            ordered_session_run_ids,
        };
        let proof = &record.aggregate_proof;
        if proof.public_request.full_campaign_run_id.as_str() != record.run_id
            || proof
                .public_request
                .sessions
                .iter()
                .map(|session| session.run_id.as_str())
                .ne(record.ordered_session_run_ids.iter().map(String::as_str))
            || proof
                .public_request
                .campaign_content_manifest_sha256
                .as_bytes()
                != &record.campaign_content_manifest_id
            || proof.public_request.rules_config_sha256.as_bytes() != &record.config_id
            || proof.public_request.ruleset_manifest_sha256.as_bytes() != &record.ruleset_id
            || proof
                .public_request
                .competition_manifest_sha256
                .map(robin_run_protocol::Digest32::into_bytes)
                != record.competition_manifest_id
            || proof.canonical_genesis_campaign.sha256.as_bytes()
                != &record.starting_campaign_sha256
            || proof.canonical_genesis_campaign.byte_length != record.starting_campaign_bytes
            || proof.final_campaign.sha256.as_bytes() != &record.final_campaign_sha256
            || proof.final_campaign.byte_length != record.final_campaign_bytes
            || proof.starting_campaign_score != record.starting_campaign_score
            || proof.final_campaign_score != record.final_campaign_score
            || proof.metrics.active_simulation_ticks != record.active_simulation_ticks
            || proof.metrics.ransom_collected != record.ransom_collected
            || proof.max_concurrent_players != record.max_concurrent_players
            || proof.participant_instance_count != record.participant_instance_count
            || proof.named_participant_instance_count != record.named_participant_instance_count
            || proof.anonymous_participant_instance_count
                != record.anonymous_participant_instance_count
        {
            return Err(DbError::Corrupt(
                "public aggregate proof differs from indexed storage".to_owned(),
            ));
        }
        Ok(record)
    }

    pub(super) async fn public_participants_for_run(
        &self,
        run_id: &str,
    ) -> Result<Vec<PublicParticipantRecord>, DbError> {
        let rows = sqlx::query(
            "SELECT sp.seat, i.public_key, i.username \
             FROM submission_participants sp \
             JOIN verified_runs r ON r.submission_id = sp.submission_id \
             JOIN identities i ON i.public_key = sp.public_key WHERE r.id = ? \
               AND sp.public_disclosure = 'named_profile' \
             ORDER BY sp.seat, i.public_key",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PublicParticipantRecord {
                    seat: u16::try_from(row.try_get::<i64, _>("seat")?).map_err(|_| {
                        DbError::Corrupt("participant seat out of range".to_owned())
                    })?,
                    identity: PublicIdentity {
                        public_key: fixed_32(row.try_get("public_key")?)?,
                        username: row.try_get("username")?,
                    },
                })
            })
            .collect()
    }

    pub(super) async fn public_participants_for_runs(
        &self,
        run_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<PublicParticipantRecord>>, DbError> {
        let mut output = BTreeMap::new();
        if run_ids.is_empty() {
            return Ok(output);
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT r.id AS run_id, sp.seat, i.public_key, \
                    i.username FROM submission_participants sp \
             JOIN verified_runs r ON r.submission_id = sp.submission_id \
             JOIN identities i ON i.public_key = sp.public_key WHERE r.id IN (",
        );
        let mut separated = query.separated(", ");
        for run_id in run_ids {
            separated.push_bind(run_id);
        }
        separated.push_unseparated(
            ") AND sp.public_disclosure = 'named_profile' \
             ORDER BY r.id, sp.seat, i.public_key",
        );
        for row in query.build().fetch_all(&self.pool).await? {
            output
                .entry(row.try_get("run_id")?)
                .or_insert_with(Vec::new)
                .push(PublicParticipantRecord {
                    seat: u16::try_from(row.try_get::<i64, _>("seat")?).map_err(|_| {
                        DbError::Corrupt("participant seat out of range".to_owned())
                    })?,
                    identity: PublicIdentity {
                        public_key: fixed_32(row.try_get("public_key")?)?,
                        username: row.try_get("username")?,
                    },
                });
        }
        Ok(output)
    }

    pub(super) async fn public_participants_for_full_campaign(
        &self,
        run_id: &str,
    ) -> Result<Vec<PublicIdentity>, DbError> {
        let rows = sqlx::query(
            "SELECT i.public_key, i.username FROM full_campaign_participants fcp \
             JOIN identities i ON i.public_key = fcp.public_key \
             WHERE fcp.full_campaign_run_id = ? ORDER BY fcp.public_key",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PublicIdentity {
                    public_key: fixed_32(row.try_get("public_key")?)?,
                    username: row.try_get("username")?,
                })
            })
            .collect()
    }

    pub(super) async fn public_participants_for_full_campaigns(
        &self,
        run_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<PublicIdentity>>, DbError> {
        let mut output = BTreeMap::new();
        if run_ids.is_empty() {
            return Ok(output);
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT fcp.full_campaign_run_id AS run_id, i.public_key, i.username \
             FROM full_campaign_participants fcp \
             JOIN identities i ON i.public_key = fcp.public_key \
             WHERE fcp.full_campaign_run_id IN (",
        );
        let mut separated = query.separated(", ");
        for run_id in run_ids {
            separated.push_bind(run_id);
        }
        separated.push_unseparated(") ORDER BY fcp.full_campaign_run_id, fcp.public_key");
        for row in query.build().fetch_all(&self.pool).await? {
            output
                .entry(row.try_get("run_id")?)
                .or_insert_with(Vec::new)
                .push(PublicIdentity {
                    public_key: fixed_32(row.try_get("public_key")?)?,
                    username: row.try_get("username")?,
                });
        }
        Ok(output)
    }

    pub(super) async fn full_campaign_sessions_for_runs(
        &self,
        run_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<String>>, DbError> {
        let mut output = BTreeMap::new();
        if run_ids.is_empty() {
            return Ok(output);
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT full_campaign_run_id, run_id FROM full_campaign_sessions \
             WHERE full_campaign_run_id IN (",
        );
        let mut separated = query.separated(", ");
        for run_id in run_ids {
            separated.push_bind(run_id);
        }
        separated.push_unseparated(") ORDER BY full_campaign_run_id, ordinal");
        for row in query.build().fetch_all(&self.pool).await? {
            output
                .entry(row.try_get("full_campaign_run_id")?)
                .or_insert_with(Vec::new)
                .push(row.try_get("run_id")?);
        }
        Ok(output)
    }

    pub async fn replay_for_run(&self, run_id: &str) -> Result<([u8; 32], u64, [u8; 32]), DbError> {
        let row = sqlx::query(
            "SELECT s.replay_sha256, s.replay_bytes, r.ruleset_id, \
                    r.verification_request_sha256, r.result_sha256, \
                    r.public_verification_request_sha256, r.public_verification_request_json, \
                    r.public_verification_result_sha256, r.public_verification_result_json, \
                    r.public_projection_binding_json \
             FROM verified_runs r \
             JOIN submissions s ON s.id = r.submission_id \
             WHERE r.id = ? AND s.status = 'accepted' AND s.tombstoned_at_ms IS NULL \
               AND (r.campaign_session_kind IS NULL OR r.campaign_session_kind = 'field_mission') \
               AND NOT EXISTS (SELECT 1 FROM full_campaign_runs terminal_aggregate \
                   WHERE terminal_aggregate.terminal_run_id = r.id)",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        let proof = stored_public_verification_proof(&row)?;
        let digest = fixed_32(row.try_get("replay_sha256")?)?;
        let byte_length = nonnegative_u64(row.try_get("replay_bytes")?, "replay_bytes")?;
        let expected = &proof.public_request.replay;
        if expected.artifact.sha256.as_bytes() != &digest
            || expected.artifact.byte_length != byte_length
        {
            return Err(DbError::Corrupt(
                "replay index differs from the stored verification proof".to_owned(),
            ));
        }
        Ok((digest, byte_length, fixed_32(row.try_get("ruleset_id")?)?))
    }

    pub async fn replay_for_campaign_session(
        &self,
        aggregate_run_id: &str,
        ordinal: u32,
    ) -> Result<([u8; 32], u64, [u8; 32]), DbError> {
        let row = sqlx::query(
            "SELECT s.replay_sha256, s.replay_bytes, fc.ruleset_id, \
                    vr.verification_request_sha256, vr.result_sha256, \
                    vr.public_verification_request_sha256, vr.public_verification_request_json, \
                    vr.public_verification_result_sha256, vr.public_verification_result_json, \
                    vr.public_projection_binding_json \
             FROM full_campaign_runs fc \
             JOIN full_campaign_sessions fcs ON fcs.full_campaign_run_id = fc.id \
             JOIN verified_runs vr ON vr.id = fcs.run_id \
             JOIN submissions s ON s.id = vr.submission_id \
             WHERE fc.id = ? AND fcs.ordinal = ? AND fc.tombstoned_at_ms IS NULL \
               AND s.status = 'accepted' AND s.tombstoned_at_ms IS NULL \
               AND NOT EXISTS (SELECT 1 FROM full_campaign_sessions linked_session \
                   JOIN verified_runs linked_run ON linked_run.id = linked_session.run_id \
                   JOIN submissions linked_submission \
                     ON linked_submission.id = linked_run.submission_id \
                   WHERE linked_session.full_campaign_run_id = fc.id \
                     AND linked_submission.tombstoned_at_ms IS NOT NULL)",
        )
        .bind(aggregate_run_id)
        .bind(i64::from(ordinal))
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        let proof = stored_public_verification_proof(&row)?;
        let digest = fixed_32(row.try_get("replay_sha256")?)?;
        let byte_length = nonnegative_u64(row.try_get("replay_bytes")?, "replay_bytes")?;
        let expected = &proof.public_request.replay;
        if expected.artifact.sha256.as_bytes() != &digest
            || expected.artifact.byte_length != byte_length
        {
            return Err(DbError::Corrupt(
                "campaign session replay index differs from its public proof".to_owned(),
            ));
        }
        Ok((digest, byte_length, fixed_32(row.try_get("ruleset_id")?)?))
    }

    /// Resolve only a campaign link cross-bound to a verified redacted public
    /// proof. Raw digest lookup is deliberately absent from this query.
    pub async fn public_campaign_for_run(
        &self,
        run_id: &str,
        role: &str,
    ) -> Result<(ArtifactRefV1, [u8; 32]), DbError> {
        let starting = match role {
            "starting" => true,
            "final" => false,
            _ => return Err(DbError::NotFound),
        };
        let rows = sqlx::query(
            "SELECT 'mission' AS source_kind, object.sha256, object.byte_length, run.ruleset_id \
             FROM verified_runs run \
             JOIN submissions submission ON submission.id = run.submission_id \
             JOIN verified_run_campaign_objects link \
               ON link.run_id = run.id AND link.role = ? \
             JOIN campaign_objects object ON object.sha256 = link.sha256 \
             WHERE run.id = ? AND submission.status = 'accepted' \
               AND submission.tombstoned_at_ms IS NULL AND object.purge_state = 'live' \
               AND (run.campaign_session_kind IS NULL \
                    OR run.campaign_session_kind = 'field_mission') \
               AND NOT EXISTS (SELECT 1 FROM full_campaign_runs terminal_aggregate \
                   WHERE terminal_aggregate.terminal_run_id = run.id) \
             UNION ALL \
             SELECT 'aggregate' AS source_kind, object.sha256, object.byte_length, run.ruleset_id \
             FROM full_campaign_runs aggregate \
             JOIN full_campaign_sessions session ON session.full_campaign_run_id = aggregate.id \
             JOIN verified_runs run ON run.id = session.run_id \
             JOIN submissions submission ON submission.id = run.submission_id \
             JOIN verified_run_campaign_objects link \
               ON link.run_id = run.id AND link.role = ? \
             JOIN campaign_objects object ON object.sha256 = link.sha256 \
             WHERE aggregate.id = ? AND aggregate.tombstoned_at_ms IS NULL \
               AND submission.status = 'accepted' AND submission.tombstoned_at_ms IS NULL \
               AND object.purge_state = 'live' \
               AND session.ordinal = CASE WHEN ? = 'starting' THEN \
                   (SELECT MIN(first.ordinal) FROM full_campaign_sessions first \
                    WHERE first.full_campaign_run_id = aggregate.id) ELSE \
                   (SELECT MAX(last.ordinal) FROM full_campaign_sessions last \
                    WHERE last.full_campaign_run_id = aggregate.id) END \
               AND NOT EXISTS (SELECT 1 FROM full_campaign_sessions linked \
                   JOIN verified_runs linked_run ON linked_run.id = linked.run_id \
                   JOIN submissions linked_submission \
                     ON linked_submission.id = linked_run.submission_id \
                   WHERE linked.full_campaign_run_id = aggregate.id \
                     AND linked_submission.tombstoned_at_ms IS NOT NULL)",
        )
        .bind(role)
        .bind(run_id)
        .bind(role)
        .bind(run_id)
        .bind(role)
        .fetch_all(&self.pool)
        .await?;
        if rows.len() != 1 {
            return Err(if rows.is_empty() {
                DbError::NotFound
            } else {
                DbError::Corrupt("run resolves to multiple public campaign objects".to_owned())
            });
        }
        let artifact = ArtifactRefV1 {
            sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(rows[0].try_get("sha256")?)?),
            byte_length: nonnegative_u64(rows[0].try_get("byte_length")?, "byte_length")?,
            media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
        };
        let ruleset_id = fixed_32(rows[0].try_get("ruleset_id")?)?;
        let expected = match rows[0].try_get::<String, _>("source_kind")?.as_str() {
            "mission" => {
                let run = self.public_run(run_id).await?;
                if run.ruleset_id != ruleset_id {
                    return Err(DbError::Corrupt(
                        "campaign object ruleset differs from its run".to_owned(),
                    ));
                }
                if starting {
                    run.verification_proof.starting_campaign
                } else {
                    run.verification_proof.final_campaign
                }
            }
            "aggregate" => {
                let aggregate = self.public_full_campaign(run_id).await?;
                if aggregate.ruleset_id != ruleset_id {
                    return Err(DbError::Corrupt(
                        "campaign object ruleset differs from its aggregate".to_owned(),
                    ));
                }
                if starting {
                    aggregate.aggregate_proof.canonical_genesis_campaign
                } else {
                    aggregate.aggregate_proof.final_campaign
                }
            }
            _ => {
                return Err(DbError::Corrupt(
                    "invalid public campaign source".to_owned(),
                ));
            }
        };
        if artifact != expected {
            return Err(DbError::Corrupt(
                "public campaign object link differs from its stored public proof".to_owned(),
            ));
        }
        Ok((artifact, ruleset_id))
    }

    /// Resolve a session campaign only through its visible aggregate. This is
    /// the sole public path for HQ campaign artifacts; a raw child run ID can
    /// never bypass aggregate tombstone or broken-chain visibility.
    pub async fn public_campaign_for_campaign_session(
        &self,
        aggregate_run_id: &str,
        ordinal: u32,
        role: &str,
    ) -> Result<(ArtifactRefV1, [u8; 32]), DbError> {
        let starting = match role {
            "starting" => true,
            "final" => false,
            _ => return Err(DbError::NotFound),
        };
        let row = sqlx::query(
            "SELECT object.sha256, object.byte_length, run.ruleset_id \
             FROM full_campaign_runs aggregate \
             JOIN full_campaign_sessions session \
               ON session.full_campaign_run_id = aggregate.id AND session.ordinal = ? \
             JOIN verified_runs run ON run.id = session.run_id \
             JOIN submissions submission ON submission.id = run.submission_id \
             JOIN verified_run_campaign_objects link \
               ON link.run_id = run.id AND link.role = ? \
             JOIN campaign_objects object ON object.sha256 = link.sha256 \
             WHERE aggregate.id = ? AND aggregate.tombstoned_at_ms IS NULL \
               AND submission.status = 'accepted' AND submission.tombstoned_at_ms IS NULL \
               AND object.purge_state = 'live' \
               AND NOT EXISTS (SELECT 1 FROM full_campaign_sessions linked_session \
                   JOIN verified_runs linked_run ON linked_run.id = linked_session.run_id \
                   JOIN submissions linked_submission \
                     ON linked_submission.id = linked_run.submission_id \
                   WHERE linked_session.full_campaign_run_id = aggregate.id \
                     AND linked_submission.tombstoned_at_ms IS NOT NULL)",
        )
        .bind(i64::from(ordinal))
        .bind(role)
        .bind(aggregate_run_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        let artifact = ArtifactRefV1 {
            sha256: robin_run_protocol::Digest32::from_bytes(fixed_32(row.try_get("sha256")?)?),
            byte_length: nonnegative_u64(row.try_get("byte_length")?, "byte_length")?,
            media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
        };
        let ruleset_id = fixed_32(row.try_get("ruleset_id")?)?;
        let aggregate = self.public_full_campaign(aggregate_run_id).await?;
        let session_id = aggregate
            .ordered_session_run_ids
            .get(usize::try_from(ordinal).map_err(|_| DbError::NotFound)?)
            .ok_or(DbError::NotFound)?;
        let session = self.public_run(session_id).await?;
        if session.ruleset_id != ruleset_id {
            return Err(DbError::Corrupt(
                "session campaign object ruleset differs from its aggregate".to_owned(),
            ));
        }
        let expected = if starting {
            session.verification_proof.starting_campaign
        } else {
            session.verification_proof.final_campaign
        };
        if artifact != expected {
            return Err(DbError::Corrupt(
                "session campaign object differs from its stored public proof".to_owned(),
            ));
        }
        Ok((artifact, ruleset_id))
    }
}

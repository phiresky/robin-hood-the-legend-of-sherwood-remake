//! Public projections. Only accepted, untombstoned runs of configured boards
//! are visible; a named uploader is joined from the durable identity table.
use super::*;
use robin_run_protocol::BoardMetricV1;

const fn metric_name(metric: BoardMetricV1) -> &'static str {
    match metric {
        BoardMetricV1::OriginalScore => "original_score",
        BoardMetricV1::FastestSuccess => "fastest_success",
    }
}

const RUN_UPLOADER_JOIN: &str = "FROM verified_runs r \
     JOIN submissions s ON s.id = r.submission_id \
     LEFT JOIN identities i ON i.public_key = s.uploader_public_key ";

fn watermark_i64(watermark: u64) -> Result<i64, DbError> {
    i64::try_from(watermark)
        .map_err(|_| DbError::ResultInvariant("acceptance watermark exceeds i64".to_owned()))
}

/// Board, mission, metric, watermark, player-count and player conditions shared
/// by page and rank queries. Expects `r`, `s` and `m` aliases.
fn push_board_conditions(query: &mut QueryBuilder<Sqlite>, board: &BoardQuery<'_>, watermark: i64) {
    query
        .push(" s.status = 'accepted' AND s.tombstoned_at_ms IS NULL AND m.metric = ")
        .push_bind(metric_name(board.metric))
        .push(" AND r.board_id = ")
        .push_bind(board.board_id.to_owned())
        .push(" AND r.mission_id = ")
        .push_bind(board.mission_id.to_owned())
        .push(" AND r.accepted_sequence <= ")
        .push_bind(watermark);
    if let Some(players) = board.max_concurrent_players {
        query
            .push(" AND r.max_concurrent_players = ")
            .push_bind(i64::from(players));
    }
    if let Some(player) = board.player_public_key {
        query
            .push(" AND s.public_disclosure = 'named_profile' AND s.uploader_public_key = ")
            .push_bind(player.to_vec());
    }
}

impl Database {
    /// Shareable progress contains no owner identity, rejection detail, or
    /// unsigned score claim. Deleted submissions are indistinguishable from
    /// missing submissions, including after a successful verification.
    pub async fn public_submission_status(
        &self,
        submission_id: &OpaqueId,
    ) -> Result<Option<robin_run_protocol::PublicSubmissionStatusV1>, DbError> {
        use robin_run_protocol::{PublicSubmissionStateV1 as State, PublicSubmissionStatusV1};
        let row = sqlx::query(
            "SELECT s.status, r.id AS run_id \
             FROM submissions s LEFT JOIN verified_runs r ON r.submission_id = s.id \
             WHERE s.id = ? AND s.tombstoned_at_ms IS NULL",
        )
        .bind(submission_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let status: String = row.try_get("status")?;
        let state = match status.as_str() {
            "queued" => State::Queued,
            "verifying" => State::Verifying,
            "retry_pending" => State::RetryPending,
            "accepted" => {
                let id: Option<String> = row.try_get("run_id")?;
                State::Verified {
                    run_id: opaque_id(id.ok_or_else(|| {
                        DbError::Corrupt("accepted submission has no run".into())
                    })?)?,
                }
            }
            "rejected" => State::Rejected,
            "failed" => State::Failed,
            other => {
                return Err(DbError::Corrupt(format!(
                    "unknown submission state {other}"
                )));
            }
        };
        Ok(Some(PublicSubmissionStatusV1 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
            submission_id: submission_id.clone(),
            state,
        }))
    }

    /// One page of a board in rank order: best metric first, then acceptance
    /// order, then run ID.
    pub async fn leaderboard_rows(
        &self,
        board: &BoardQuery<'_>,
        cursor: Option<&BoardCursor>,
        limit: u32,
        accepted_sequence_watermark: u64,
    ) -> Result<Vec<BoardRow>, DbError> {
        let watermark = watermark_i64(accepted_sequence_watermark)?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT r.id, s.replay_sha256, m.value, r.max_concurrent_players, \
                    r.participant_instance_count, r.accepted_sequence, r.verified_at_ms, \
                    s.public_disclosure, s.uploader_public_key, i.username ",
        );
        query.push(RUN_UPLOADER_JOIN);
        query.push("JOIN verified_run_metrics m ON m.run_id = r.id WHERE");
        push_board_conditions(&mut query, board, watermark);
        if let Some(cursor) = cursor {
            let comparison = match board.metric {
                BoardMetricV1::OriginalScore => "<",
                BoardMetricV1::FastestSuccess => ">",
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
                .push_bind(cursor.run_id.clone())
                .push("))))");
        }
        match board.metric {
            BoardMetricV1::OriginalScore => {
                query.push(" ORDER BY m.value DESC, r.accepted_sequence, r.id");
            }
            BoardMetricV1::FastestSuccess => {
                query.push(" ORDER BY m.value ASC, r.accepted_sequence, r.id");
            }
        }
        query.push(" LIMIT ").push_bind(i64::from(limit));
        let rows = query.build().fetch_all(&self.pool).await?;
        let metric_values = rows
            .iter()
            .map(|row| row.try_get::<i64, _>("value"))
            .collect::<Result<Vec<_>, _>>()?;
        let ranks = self
            .board_ranks(board, &metric_values, accepted_sequence_watermark)
            .await?;
        rows.into_iter()
            .map(|row| {
                let metric_value: i64 = row.try_get("value")?;
                Ok(BoardRow {
                    rank: *ranks.get(&metric_value).ok_or_else(|| {
                        DbError::Corrupt("batch board rank omitted a requested value".to_owned())
                    })?,
                    run_id: row.try_get("id")?,
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
                    uploader: decode_uploader(&row)?,
                    replay_sha256: fixed_32(row.try_get("replay_sha256")?)?,
                    accepted_sequence: nonnegative_u64(
                        row.try_get("accepted_sequence")?,
                        "accepted_sequence",
                    )?,
                    verified_at_ms: nonnegative_u64(
                        row.try_get("verified_at_ms")?,
                        "verified_at_ms",
                    )?,
                })
            })
            .collect()
    }

    /// Competition rank (one plus the number of strictly better entries) for
    /// each requested metric value, in one query.
    async fn board_ranks(
        &self,
        board: &BoardQuery<'_>,
        values: &[i64],
        accepted_sequence_watermark: u64,
    ) -> Result<BTreeMap<i64, u64>, DbError> {
        let unique = values
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.is_empty() {
            return Ok(BTreeMap::new());
        }
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
        query.push("), board(value) AS MATERIALIZED (SELECT m.value ");
        query.push(RUN_UPLOADER_JOIN);
        query.push("JOIN verified_run_metrics m ON m.run_id = r.id WHERE");
        push_board_conditions(
            &mut query,
            board,
            watermark_i64(accepted_sequence_watermark)?,
        );
        let comparison = match board.metric {
            BoardMetricV1::OriginalScore => ">",
            BoardMetricV1::FastestSuccess => "<",
        };
        query
            .push(
                ") SELECT candidates.value, COUNT(board.value) + 1 AS rank FROM candidates \
                 LEFT JOIN board ON board.value ",
            )
            .push(comparison)
            .push(" candidates.value GROUP BY candidates.value");
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
        visible_board_ids: &[String],
        accepted_sequence_watermark: u64,
        cursor: Option<(u64, &str)>,
        limit: u32,
    ) -> Result<Vec<PlayerHistoryRecord>, DbError> {
        if limit == 0 || limit > 101 {
            return Err(DbError::ResultInvariant(
                "player history limit must be in 1..=101".to_owned(),
            ));
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT r.id, r.board_id, r.mission_id, r.original_score_delta, \
                    r.active_simulation_ticks, r.ransom_collected, r.max_concurrent_players, \
                    r.participant_instance_count, r.accepted_sequence, r.verified_at_ms, \
                    s.public_disclosure, s.uploader_public_key, i.username ",
        );
        query.push(RUN_UPLOADER_JOIN);
        query.push(
            "WHERE s.status = 'accepted' AND s.tombstoned_at_ms IS NULL \
               AND s.public_disclosure = 'named_profile' AND s.uploader_public_key = ",
        );
        query.push_bind(public_key.to_vec());
        query.push(" AND ");
        push_text_filter(&mut query, "r.board_id", visible_board_ids);
        query
            .push(" AND r.accepted_sequence <= ")
            .push_bind(watermark_i64(accepted_sequence_watermark)?);
        if let Some((sequence, run_id)) = cursor {
            let sequence = i64::try_from(sequence)
                .map_err(|_| DbError::ResultInvariant("history cursor exceeds i64".to_owned()))?;
            query
                .push(" AND (r.accepted_sequence < ")
                .push_bind(sequence)
                .push(" OR (r.accepted_sequence = ")
                .push_bind(sequence)
                .push(" AND r.id > ")
                .push_bind(run_id.to_owned())
                .push("))");
        }
        query
            .push(" ORDER BY r.accepted_sequence DESC, r.id LIMIT ")
            .push_bind(i64::from(limit));
        let rows = query.build().fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok(PlayerHistoryRecord {
                    run_id: row.try_get("id")?,
                    board_id: row.try_get("board_id")?,
                    mission_id: row.try_get("mission_id")?,
                    original_score_delta: row.try_get("original_score_delta")?,
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
                        "player count exceeds u16",
                    )?,
                    participant_instance_count: checked_count(
                        &row,
                        "participant_instance_count",
                        "participant count exceeds u16",
                    )?,
                    accepted_sequence: nonnegative_u64(
                        row.try_get("accepted_sequence")?,
                        "accepted_sequence",
                    )?,
                    verified_at_ms: nonnegative_u64(
                        row.try_get("verified_at_ms")?,
                        "verified_at_ms",
                    )?,
                    uploader: decode_uploader(&row)?,
                })
            })
            .collect()
    }

    pub async fn player_personal_bests(
        &self,
        public_key: &[u8; 32],
        visible_board_ids: &[String],
        accepted_sequence_watermark: u64,
    ) -> Result<Vec<PlayerBestRecord>, DbError> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "WITH candidates AS (SELECT r.id, r.board_id, r.mission_id, m.metric, m.value, \
                    r.max_concurrent_players, r.accepted_sequence \
             FROM verified_runs r JOIN submissions s ON s.id = r.submission_id \
             JOIN verified_run_metrics m ON m.run_id = r.id \
             WHERE s.status = 'accepted' AND s.tombstoned_at_ms IS NULL \
               AND s.public_disclosure = 'named_profile' AND s.uploader_public_key = ",
        );
        query.push_bind(public_key.to_vec());
        query.push(" AND ");
        push_text_filter(&mut query, "r.board_id", visible_board_ids);
        query
            .push(" AND r.accepted_sequence <= ")
            .push_bind(watermark_i64(accepted_sequence_watermark)?);
        query.push(
            "), ranked AS (SELECT *, ROW_NUMBER() OVER (PARTITION BY board_id, mission_id, \
                 metric, max_concurrent_players ORDER BY \
                 CASE WHEN metric = 'original_score' THEN value END DESC, \
                 CASE WHEN metric = 'fastest_success' THEN value END ASC, \
                 accepted_sequence, id) AS position FROM candidates) \
             SELECT * FROM ranked WHERE position = 1 \
             ORDER BY board_id, mission_id, metric, max_concurrent_players LIMIT 512",
        );
        let rows = query.build().fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok(PlayerBestRecord {
                    run_id: row.try_get("id")?,
                    board_id: row.try_get("board_id")?,
                    mission_id: row.try_get("mission_id")?,
                    metric: row.try_get("metric")?,
                    value: row.try_get("value")?,
                    max_concurrent_players: checked_count(
                        &row,
                        "max_concurrent_players",
                        "player count exceeds u16",
                    )?,
                })
            })
            .collect()
    }

    pub async fn public_run(
        &self,
        run_id: &str,
        visible_board_ids: &[String],
    ) -> Result<PublicRunRecord, DbError> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT r.id, r.board_id, r.mission_id, r.recorded_engine_version, \
                    r.sim_config_json, r.max_concurrent_players, r.participant_instance_count, \
                    r.starting_campaign_score, r.final_campaign_score, r.original_score_delta, \
                    r.active_simulation_ticks, r.ransom_collected, r.verified_at_ms, \
                    s.replay_sha256, s.replay_bytes, s.replay_schema_version, \
                    s.public_disclosure, s.uploader_public_key, i.username ",
        );
        query.push(RUN_UPLOADER_JOIN);
        query.push("WHERE s.status = 'accepted' AND s.tombstoned_at_ms IS NULL AND r.id = ");
        query.push_bind(run_id.to_owned());
        query.push(" AND ");
        push_text_filter(&mut query, "r.board_id", visible_board_ids);
        let row = query
            .build()
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        let achievements = sqlx::query(
            "SELECT achievement_id, evaluation FROM verified_run_achievements \
             WHERE run_id = ? ORDER BY achievement_id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|achievement| {
            Ok((
                achievement.try_get("achievement_id")?,
                achievement_evaluation(&achievement.try_get::<String, _>("evaluation")?)?,
            ))
        })
        .collect::<Result<Vec<_>, DbError>>()?;
        let score = |column: &str| -> Result<i32, DbError> {
            i32::try_from(row.try_get::<i64, _>(column)?)
                .map_err(|_| DbError::Corrupt(format!("{column} exceeds i32")))
        };
        Ok(PublicRunRecord {
            run_id: row.try_get("id")?,
            board_id: row.try_get("board_id")?,
            mission_id: row.try_get("mission_id")?,
            recorded_engine_version: row.try_get("recorded_engine_version")?,
            sim_config_json: row.try_get("sim_config_json")?,
            max_concurrent_players: checked_count(
                &row,
                "max_concurrent_players",
                "player count exceeds u16",
            )?,
            participant_instance_count: checked_count(
                &row,
                "participant_instance_count",
                "participant count exceeds u16",
            )?,
            starting_campaign_score: score("starting_campaign_score")?,
            final_campaign_score: score("final_campaign_score")?,
            original_score_delta: row.try_get("original_score_delta")?,
            active_simulation_ticks: nonnegative_u64(
                row.try_get("active_simulation_ticks")?,
                "active_simulation_ticks",
            )?,
            ransom_collected: nonnegative_u64(
                row.try_get("ransom_collected")?,
                "ransom_collected",
            )?,
            verified_at_ms: nonnegative_u64(row.try_get("verified_at_ms")?, "verified_at_ms")?,
            replay_sha256: fixed_32(row.try_get("replay_sha256")?)?,
            replay_bytes: nonnegative_u64(row.try_get("replay_bytes")?, "replay_bytes")?,
            replay_schema_version: checked_count(
                &row,
                "replay_schema_version",
                "replay schema version exceeds u32",
            )?,
            uploader: decode_uploader(&row)?,
            achievements,
        })
    }

    pub async fn replay_for_run(
        &self,
        run_id: &str,
        visible_board_ids: &[String],
    ) -> Result<([u8; 32], u64), DbError> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT s.replay_sha256, s.replay_bytes FROM verified_runs r \
             JOIN submissions s ON s.id = r.submission_id \
             WHERE s.status = 'accepted' AND s.tombstoned_at_ms IS NULL AND r.id = ",
        );
        query.push_bind(run_id.to_owned());
        query.push(" AND ");
        push_text_filter(&mut query, "r.board_id", visible_board_ids);
        let row = query
            .build()
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        Ok((
            fixed_32(row.try_get("replay_sha256")?)?,
            nonnegative_u64(row.try_get("replay_bytes")?, "replay_bytes")?,
        ))
    }
}

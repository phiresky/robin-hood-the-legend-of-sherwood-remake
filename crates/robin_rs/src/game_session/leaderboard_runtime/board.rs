//! Select the published board that admits a recorded replay.
use super::RankedError;
use robin_engine::engine::SimConfig;
use robin_run_protocol::{
    BoardSimulationPolicyV1, BoardV2, LeaderboardMetadataV2, OfficialContentEditionV1,
};

/// The unique board of `edition` listing `mission_id` whose simulation
/// policy admits `sim_config`. Exact preset boards win over any-config
/// boards; any remaining ambiguity is an unavailable leaderboard, never an
/// arbitrary pick.
pub(super) fn select_board<'a>(
    metadata: &'a LeaderboardMetadataV2,
    edition: OfficialContentEditionV1,
    mission_id: &str,
    sim_config: SimConfig,
) -> Result<&'a BoardV2, RankedError> {
    let mut candidates = metadata
        .boards
        .iter()
        .filter(|board| {
            board.edition == edition
                && board.mission(mission_id).is_some()
                && robin_engine::ranked_rules::ranked_policy_for_board(
                    board.simulation_policy,
                    sim_config,
                )
                .is_ok()
        })
        .collect::<Vec<_>>();
    if candidates.iter().any(|board| {
        matches!(
            board.simulation_policy,
            BoardSimulationPolicyV1::Fixed { .. }
        )
    }) {
        candidates.retain(|board| {
            matches!(
                board.simulation_policy,
                BoardSimulationPolicyV1::Fixed { .. }
            )
        });
    }
    match candidates.as_slice() {
        [board] => Ok(board),
        [] => Err(RankedError::unavailable(format!(
            "no {edition:?} leaderboard accepts mission `{mission_id}` with this gameplay configuration"
        ))),
        boards => Err(RankedError::unavailable(format!(
            "{} {edition:?} leaderboards accept mission `{mission_id}` with this gameplay configuration ({}); the server must publish exactly one",
            boards.len(),
            boards
                .iter()
                .map(|board| board.board_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leaderboard::test_fixtures::{MISSION_ID, board, standard_medium_policy};
    use robin_engine::engine::RankedSimulationPolicy;
    use robin_run_protocol::{SCHEMA_VERSION_V2, TickDurationV1};

    fn metadata(boards: Vec<BoardV2>) -> LeaderboardMetadataV2 {
        let mut boards = boards;
        boards.sort_by(|left, right| left.board_id.cmp(&right.board_id));
        LeaderboardMetadataV2 {
            schema_version: SCHEMA_VERSION_V2,
            tick_duration: TickDurationV1 {
                numerator_micros: 40_000,
                denominator: 1,
            },
            boards,
        }
    }

    fn standard_config() -> SimConfig {
        RankedSimulationPolicy::standard_medium().expected_config()
    }

    fn selected(metadata: &LeaderboardMetadataV2, config: SimConfig) -> Result<String, String> {
        select_board(metadata, OfficialContentEditionV1::Demo, MISSION_ID, config)
            .map(|board| board.board_id.as_str().to_owned())
            .map_err(|error| error.to_string())
    }

    #[test]
    fn edition_mission_and_policy_must_all_admit_the_replay() {
        let demo = board(
            "demo-standard-normal",
            OfficialContentEditionV1::Demo,
            standard_medium_policy(),
        );
        let full = board(
            "full-standard-normal",
            OfficialContentEditionV1::Full,
            standard_medium_policy(),
        );
        let catalog = metadata(vec![demo.clone(), full]);
        assert_eq!(
            selected(&catalog, standard_config()).unwrap(),
            "demo-standard-normal"
        );
        assert!(
            select_board(
                &catalog,
                OfficialContentEditionV1::Demo,
                "Demo_Lin",
                standard_config()
            )
            .is_err()
        );
        let mut changed = standard_config();
        changed.enable_unbinding = !changed.enable_unbinding;
        assert!(
            selected(&catalog, changed)
                .unwrap_err()
                .contains("no Demo leaderboard")
        );
    }

    #[test]
    fn fixed_boards_are_preferred_over_any_config_boards() {
        let catalog = metadata(vec![
            board(
                "demo-any",
                OfficialContentEditionV1::Demo,
                BoardSimulationPolicyV1::AnyConfig,
            ),
            board(
                "demo-standard-normal",
                OfficialContentEditionV1::Demo,
                standard_medium_policy(),
            ),
        ]);
        assert_eq!(
            selected(&catalog, standard_config()).unwrap(),
            "demo-standard-normal"
        );
        let mut custom = standard_config();
        custom.enable_unbinding = !custom.enable_unbinding;
        assert_eq!(selected(&catalog, custom).unwrap(), "demo-any");
    }

    #[test]
    fn remaining_ambiguity_is_unavailable() {
        let catalog = metadata(vec![
            board(
                "demo-any-a",
                OfficialContentEditionV1::Demo,
                BoardSimulationPolicyV1::AnyConfig,
            ),
            board(
                "demo-any-b",
                OfficialContentEditionV1::Demo,
                BoardSimulationPolicyV1::AnyConfig,
            ),
        ]);
        let error = selected(&catalog, standard_config()).unwrap_err();
        assert!(error.contains("demo-any-a, demo-any-b"), "{error}");
    }
}

//! Isolated deterministic replay execution for ranked verification.
//!
//! This loop intentionally owns only an already-constructed [`Engine`], its
//! manifest-approved [`LevelAssets`], and immutable replay data. It has no
//! network transport, HTTP queue, wall clock, renderer, audio backend, modal
//! auto-dismissal, or debugger stepping surface to accidentally consult.

use crate::campaign::Campaign;
use crate::engine::{Engine, LevelAssets};
use crate::game_operation::GameCode;
use crate::replay::{ReplayData, state_hash};
use robin_run_protocol::Digest32;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Fully replay-derived terminal state. The final campaign is typed rather
/// than an unchecked submitted claim and is encoded only after deterministic
/// execution reaches the exact EOF boundary.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedResimulation {
    pub outcome: GameCode,
    pub replay_frames: u32,
    pub active_simulation_ticks: u64,
    /// Deterministic multiplayer hash at terminal EOF.
    pub final_deterministic_state_hash: u64,
    pub final_state_sha256: Digest32,
    pub final_campaign_sha256: Digest32,
    pub final_campaign: Campaign,
    /// Exact frozen terminal counters captured before engine ownership is
    /// consumed into the final campaign.
    pub mission_stat: crate::mission_stat::MissionStat,
    pub mission_achievement_results: Option<crate::achievement::MissionAchievementResults>,
}

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RankedResimulationError {
    #[error("replay admission failed: {message}")]
    Admission { message: String },
    #[error("ranked replay terminal recorder shape is invalid: {message}")]
    TerminalCommandShape { message: String },
    #[error("ranked replay terminal command claims {recorded:?}, engine observed {observed:?}")]
    TerminalCommandOutcomeMismatch {
        recorded: GameCode,
        observed: GameCode,
    },
    #[error("frame {frame} is missing its mandatory pre-frame state hash")]
    MissingPeriodicHash { frame: u32 },
    #[error("frame {frame} pre-state hash differs: expected {expected:016x}, got {actual:016x}")]
    StateHashMismatch {
        frame: u32,
        expected: u64,
        actual: u64,
    },
    #[error("frame {frame} engine admission failed: {message}")]
    FrameAdvance { frame: u32, message: String },
    #[error("frame {frame} reached terminal {outcome:?} before replay EOF {replay_frames}")]
    TerminalBeforeEof {
        frame: u32,
        replay_frames: u32,
        outcome: GameCode,
    },
    #[error("replay EOF was reached while the mission remained in progress")]
    EofBeforeTerminal,
    #[error("replay reached unsupported terminal outcome {outcome:?}")]
    UnsupportedTerminal { outcome: GameCode },
    #[error("frame {frame} carries conflicting main/post-initialize outcomes {main:?}/{post:?}")]
    ConflictingTerminalOutcomes {
        frame: u32,
        main: GameCode,
        post: GameCode,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StateHashPolicy {
    Validate,
    Regenerate,
}

/// Execute the uploaded canonical replay after independently proving its
/// deterministic checkpoint coverage and command admission.
pub fn resimulate_canonical_ranked_replay(
    mut engine: Engine,
    assets: &LevelAssets,
    replay: &ReplayData,
) -> Result<RankedResimulation, RankedResimulationError> {
    let (resimulation, _) =
        resimulate_ranked_replay_inner(&mut engine, assets, replay, StateHashPolicy::Validate)?;
    Ok(resimulation)
}

/// Regenerate every periodic checkpoint by executing the canonical timeline.
/// Callers must still byte-compare and independently resimulate the uploaded
/// canonical artifact.
pub fn regenerate_canonical_ranked_replay(
    mut engine: Engine,
    assets: &LevelAssets,
    replay: &ReplayData,
) -> Result<(ReplayData, RankedResimulation), RankedResimulationError> {
    let mut normalized = replay.clone();
    normalized
        .replace_state_hashes(Default::default())
        .map_err(|message| RankedResimulationError::Admission { message })?;
    let (resimulation, hashes) = resimulate_ranked_replay_inner(
        &mut engine,
        assets,
        &normalized,
        StateHashPolicy::Regenerate,
    )?;
    normalized
        .replace_state_hashes(hashes)
        .map_err(|message| RankedResimulationError::Admission { message })?;
    normalized
        .validate_ranked_hash_coverage()
        .map_err(|message| RankedResimulationError::Admission { message })?;
    Ok((normalized, resimulation))
}

fn resimulate_ranked_replay_inner(
    engine: &mut Engine,
    assets: &LevelAssets,
    replay: &ReplayData,
    hash_policy: StateHashPolicy,
) -> Result<(RankedResimulation, std::collections::BTreeMap<u32, u64>), RankedResimulationError> {
    replay
        .validate_layout()
        .map_err(|message| RankedResimulationError::Admission { message })?;
    replay
        .ranked_submission_verdict()
        .map_err(|error| RankedResimulationError::Admission {
            message: format!("{error:?}"),
        })?;
    replay
        .validate_canonical_ranked_command_admission()
        .map_err(|message| RankedResimulationError::Admission { message })?;
    let recorded_terminal = validate_terminal_recorder_shape(replay, engine.sim_config())?;

    let replay_frames = replay.frame_count();
    let mut active_simulation_ticks = 0_u64;
    let mut terminal = None;
    let mut regenerated_hashes = std::collections::BTreeMap::new();
    for frame in 0..replay_frames {
        // The current canonical recording writes the deterministic pre-command
        // state at frame zero and every real second thereafter. Missing loses:
        // a submitter cannot delete the checkpoint which would expose a
        // divergent prefix.
        if frame.is_multiple_of(crate::multiplayer::STATE_HASH_INTERVAL) {
            let actual = state_hash(engine);
            match hash_policy {
                StateHashPolicy::Validate => {
                    let expected = replay
                        .hash_for_frame(frame)
                        .ok_or(RankedResimulationError::MissingPeriodicHash { frame })?;
                    if actual != expected {
                        return Err(RankedResimulationError::StateHashMismatch {
                            frame,
                            expected,
                            actual,
                        });
                    }
                }
                StateHashPolicy::Regenerate => {
                    regenerated_hashes.insert(frame, actual);
                }
            }
        }

        let input = replay
            .frame(frame)
            .ok_or_else(|| RankedResimulationError::Admission {
                message: format!("replay frame {frame} is absent"),
            })?
            .input
            .clone();
        let output = engine.advance_frame(assets, input).map_err(|error| {
            RankedResimulationError::FrameAdvance {
                frame,
                message: error.to_string(),
            }
        })?;
        if output.hourglass_ran {
            active_simulation_ticks = active_simulation_ticks.checked_add(1).ok_or_else(|| {
                RankedResimulationError::Admission {
                    message: "active simulation tick count overflow".into(),
                }
            })?;
        }
        let main = output.game_code();
        let post = output
            .post_initialize_events
            .as_ref()
            .map(|events| events.game_code())
            .unwrap_or(GameCode::LevelInProgress);
        let outcome = match (main, post) {
            (GameCode::LevelInProgress, post) => post,
            (main, GameCode::LevelInProgress) => main,
            (main, post) if main == post => main,
            (main, post) => {
                return Err(RankedResimulationError::ConflictingTerminalOutcomes {
                    frame,
                    main,
                    post,
                });
            }
        };
        if outcome != GameCode::LevelInProgress {
            if frame.checked_add(1) != Some(replay_frames) {
                return Err(RankedResimulationError::TerminalBeforeEof {
                    frame,
                    replay_frames,
                    outcome,
                });
            }
            terminal = Some(outcome);
        }
    }

    if hash_policy == StateHashPolicy::Validate {
        replay
            .validate_ranked_hash_coverage()
            .map_err(|message| RankedResimulationError::Admission { message })?;
    }
    let outcome = terminal.ok_or(RankedResimulationError::EofBeforeTerminal)?;
    if !matches!(
        outcome,
        GameCode::LevelSucceeded | GameCode::LevelFailed | GameCode::LevelInterrupted
    ) {
        return Err(RankedResimulationError::UnsupportedTerminal { outcome });
    }
    if recorded_terminal != outcome {
        return Err(RankedResimulationError::TerminalCommandOutcomeMismatch {
            recorded: recorded_terminal,
            observed: outcome,
        });
    }

    let final_deterministic_state_hash = state_hash(engine);
    let final_state_sha256 =
        Digest32::from_bytes(Sha256::digest(engine.encode_native_snapshot()).into());
    let mission_stat = engine.mission_stat().clone();
    let mission_achievement_results = engine.mission_achievement_results().copied();
    let final_campaign = engine.campaign().clone();
    let final_campaign_sha256 =
        Digest32::from_bytes(Sha256::digest(bitcode::encode(&final_campaign)).into());
    Ok((
        RankedResimulation {
            outcome,
            replay_frames,
            active_simulation_ticks,
            final_deterministic_state_hash,
            final_state_sha256,
            final_campaign_sha256,
            final_campaign,
            mission_stat,
            mission_achievement_results,
        },
        regenerated_hashes,
    ))
}

fn validate_terminal_recorder_shape(
    replay: &ReplayData,
    sim_config: crate::engine::SimConfig,
) -> Result<GameCode, RankedResimulationError> {
    use crate::player_command::PlayerCommand;

    let replay_frames = replay.frame_count();
    let mut recorded_terminal = None;
    for replay_ordinal in 0..replay_frames {
        let frame = replay.frame(replay_ordinal).ok_or_else(|| {
            RankedResimulationError::TerminalCommandShape {
                message: format!("replay frame {replay_ordinal} is absent"),
            }
        })?;
        if frame.input.commands.iter().any(|command| {
            matches!(
                &command.player_input().command,
                PlayerCommand::ApplyQuitMissionUpdates { .. }
            )
        }) {
            return Err(RankedResimulationError::TerminalCommandShape {
                message: format!(
                    "ApplyQuitMissionUpdates appears in pre-commands at ordinal {replay_ordinal}"
                ),
            });
        }
        for command in &frame.input.post_commands {
            let PlayerCommand::ApplyQuitMissionUpdates {
                exit_code,
                difficulty,
                completed_at_unix_seconds,
                campaign_run_nonce,
            } = &command.player_input().command
            else {
                continue;
            };
            if recorded_terminal.is_some() {
                return Err(RankedResimulationError::TerminalCommandShape {
                    message: "ApplyQuitMissionUpdates appears more than once".into(),
                });
            }
            if replay_ordinal.checked_add(1) != Some(replay_frames) {
                return Err(RankedResimulationError::TerminalCommandShape {
                    message: format!(
                        "ApplyQuitMissionUpdates appears before EOF at ordinal {replay_ordinal}"
                    ),
                });
            }
            if *difficulty != sim_config.difficulty {
                return Err(RankedResimulationError::TerminalCommandShape {
                    message: "ApplyQuitMissionUpdates difficulty differs from SimConfig".into(),
                });
            }
            let _ = (completed_at_unix_seconds, campaign_run_nonce);
            recorded_terminal = Some(*exit_code);
        }
    }
    recorded_terminal.ok_or_else(|| RankedResimulationError::TerminalCommandShape {
        message: "EOF frame has no ApplyQuitMissionUpdates post-command".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SimulationFrameInput;
    use crate::player_command::{PlayerCommand, PlayerInput};
    use crate::replay::{REPLAY_SCHEMA_VERSION, ReplayFile, ReplayFrame, ReplayHeader};
    use crate::replay_rankability::ReplayRankability;
    use std::collections::BTreeMap;

    const FIXTURE_MISSION_ID: &str = "ranked-resim-fixture";

    fn fixture_campaign() -> Campaign {
        Campaign::default()
    }

    fn fixture_mission_assets() -> crate::mission_assets::MissionAssetDescriptor {
        crate::mission_assets::MissionAssetDescriptor::built_in(
            FIXTURE_MISSION_ID,
            FIXTURE_MISSION_ID,
            FIXTURE_MISSION_ID,
        )
        .expect("valid built-in ranked resimulation fixture")
    }

    fn terminal_update(
        exit_code: GameCode,
        difficulty: crate::player_profile::DifficultyLevel,
    ) -> PlayerInput {
        PlayerInput::host(PlayerCommand::ApplyQuitMissionUpdates {
            exit_code,
            difficulty,
            completed_at_unix_seconds: None,
            campaign_run_nonce: Some(1),
        })
    }

    fn terminal_input(
        exit_code: GameCode,
        difficulty: crate::player_profile::DifficultyLevel,
    ) -> SimulationFrameInput {
        let mut input = SimulationFrameInput::no_hourglass();
        input
            .post_commands
            .push(terminal_update(exit_code, difficulty).into());
        input
    }

    fn fixture_with_inputs(
        include_hash: bool,
        inputs: impl FnOnce(crate::engine::SimConfig) -> Vec<SimulationFrameInput>,
    ) -> (Engine, LevelAssets, ReplayData) {
        let mut assets = LevelAssets::new();
        let engine = Engine::new_for_test(1024.0, 768.0, fixture_campaign(), &mut assets)
            .expect("fixture engine");
        let sim_config = engine.sim_config();
        let inputs = inputs(sim_config);
        let total_frames = u32::try_from(inputs.len()).expect("fixture frame count fits u32");
        let replay = ReplayFile {
            header: ReplayHeader {
                mission_id: FIXTURE_MISSION_ID.into(),
                mission_assets: fixture_mission_assets(),
                rng_seed: 0,
                sim_config: engine.sim_config(),
                spellforge_package: None,
                version: REPLAY_SCHEMA_VERSION,
                total_frames,
                rankability: ReplayRankability::rankable(),
                campaign: bitcode::encode(engine.campaign()),
            },
            frames: inputs
                .into_iter()
                .enumerate()
                .map(|(ordinal, input)| {
                    let ordinal = u32::try_from(ordinal).expect("fixture ordinal fits u32");
                    (
                        ordinal,
                        ReplayFrame {
                            timeline_before: ordinal,
                            timeline_after: ordinal + 1,
                            input,
                            host_controls: Vec::new(),
                        },
                    )
                })
                .collect(),
            hashes: include_hash
                .then(|| BTreeMap::from([(0, state_hash(&engine))]))
                .unwrap_or_default(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .try_into()
        .expect("valid replay fixture");
        (engine, assets, replay)
    }

    fn fixture_with_input(
        include_hash: bool,
        input: impl FnOnce(crate::engine::SimConfig) -> SimulationFrameInput,
    ) -> (Engine, LevelAssets, ReplayData) {
        fixture_with_inputs(include_hash, |sim_config| vec![input(sim_config)])
    }

    fn fixture(include_hash: bool) -> (Engine, LevelAssets, ReplayData) {
        fixture_with_input(include_hash, |sim_config| {
            terminal_input(GameCode::LevelSucceeded, sim_config.difficulty)
        })
    }

    #[test]
    fn missing_mandatory_hash_fails_before_simulation() {
        let (engine, assets, replay) = fixture(false);
        assert!(matches!(
            resimulate_canonical_ranked_replay(engine, &assets, &replay),
            Err(RankedResimulationError::MissingPeriodicHash { frame: 0 })
        ));
    }

    #[test]
    fn canonical_replay_rejects_extra_out_of_cadence_hashes() {
        let (engine, assets, mut replay) = fixture_with_inputs(true, |config| {
            vec![
                SimulationFrameInput::no_hourglass(),
                terminal_input(GameCode::LevelSucceeded, config.difficulty),
            ]
        });
        replay
            .replace_state_hashes(BTreeMap::from([(0, state_hash(&engine)), (1, 0xfeed_face)]))
            .unwrap();
        assert!(
            replay
                .validate_ranked_hash_coverage()
                .unwrap_err()
                .contains("out-of-cadence")
        );
        assert!(matches!(
            resimulate_canonical_ranked_replay(engine, &assets, &replay),
            Err(RankedResimulationError::Admission { .. })
        ));
    }

    #[test]
    fn eof_before_an_independent_terminal_fails_closed() {
        let (engine, assets, replay) = fixture(true);
        assert!(matches!(
            resimulate_canonical_ranked_replay(engine, &assets, &replay),
            Err(RankedResimulationError::EofBeforeTerminal)
        ));
    }

    #[test]
    fn terminal_update_is_forbidden_in_pre_commands() {
        let (engine, assets, replay) = fixture_with_input(true, |sim_config| {
            SimulationFrameInput::from_player_inputs(vec![terminal_update(
                GameCode::LevelSucceeded,
                sim_config.difficulty,
            )])
        });
        assert!(matches!(
            resimulate_canonical_ranked_replay(engine, &assets, &replay),
            Err(RankedResimulationError::TerminalCommandShape { .. })
        ));
    }

    #[test]
    fn missing_terminal_update_is_rejected() {
        let (engine, assets, replay) =
            fixture_with_input(true, |_sim_config| SimulationFrameInput::no_hourglass());
        assert!(matches!(
            resimulate_canonical_ranked_replay(engine, &assets, &replay),
            Err(RankedResimulationError::TerminalCommandShape { .. })
        ));
    }

    #[test]
    fn sole_terminal_update_before_eof_is_rejected() {
        let (engine, assets, replay) = fixture_with_inputs(true, |sim_config| {
            vec![
                terminal_input(GameCode::LevelSucceeded, sim_config.difficulty),
                SimulationFrameInput::no_hourglass(),
            ]
        });
        assert!(matches!(
            resimulate_canonical_ranked_replay(engine, &assets, &replay),
            Err(RankedResimulationError::TerminalCommandShape { .. })
        ));
    }

    #[test]
    fn duplicate_terminal_updates_are_rejected() {
        let (engine, assets, replay) = fixture_with_input(true, |sim_config| {
            let mut input = terminal_input(GameCode::LevelSucceeded, sim_config.difficulty);
            input
                .post_commands
                .push(terminal_update(GameCode::LevelSucceeded, sim_config.difficulty).into());
            input
        });
        assert!(matches!(
            resimulate_canonical_ranked_replay(engine, &assets, &replay),
            Err(RankedResimulationError::TerminalCommandShape { .. })
        ));
    }

    #[test]
    fn terminal_update_difficulty_must_match_the_simulation() {
        let (engine, assets, replay) = fixture_with_input(true, |_sim_config| {
            terminal_input(
                GameCode::LevelSucceeded,
                crate::player_profile::DifficultyLevel::Hard,
            )
        });
        assert!(matches!(
            resimulate_canonical_ranked_replay(engine, &assets, &replay),
            Err(RankedResimulationError::TerminalCommandShape { .. })
        ));
    }

    #[test]
    fn terminal_update_must_match_the_independently_observed_outcome() {
        let (engine, assets, replay) = fixture_with_input(true, |sim_config| {
            let mut input = SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
                PlayerCommand::QuitMissionRequested,
            )]);
            input
                .post_commands
                .push(terminal_update(GameCode::LevelSucceeded, sim_config.difficulty).into());
            input
        });
        assert!(matches!(
            resimulate_canonical_ranked_replay(engine, &assets, &replay),
            Err(RankedResimulationError::TerminalCommandOutcomeMismatch {
                recorded: GameCode::LevelSucceeded,
                observed: GameCode::LevelInterrupted,
            })
        ));
    }

    #[test]
    fn canonical_replay_uses_late_rollback_corrected_input_and_hash_schedule() {
        let mut assets = LevelAssets::new();
        let engine = Engine::new_for_test(1024.0, 768.0, fixture_campaign(), &mut assets)
            .expect("fixture engine");
        let initial_hash = state_hash(&engine);
        let mut corrected_engine = engine.clone();
        let mut corrected_checkpoint = None;
        let mut terminal = None;
        for ordinal in 0..27 {
            if ordinal == 25 {
                corrected_checkpoint = Some(state_hash(&corrected_engine));
            }
            let input = if ordinal == 0 {
                SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
                    PlayerCommand::CrouchDown,
                )])
            } else if ordinal == 26 {
                let mut input = SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
                    PlayerCommand::QuitMissionRequested,
                )]);
                input.post_commands.push(
                    terminal_update(GameCode::LevelInterrupted, engine.sim_config().difficulty)
                        .into(),
                );
                input
            } else {
                SimulationFrameInput::default()
            };
            terminal = Some(
                corrected_engine
                    .advance_frame(&assets, input)
                    .unwrap()
                    .game_code(),
            );
        }
        assert_eq!(terminal, Some(GameCode::LevelInterrupted));
        let corrected_checkpoint = corrected_checkpoint.expect("frame 25 checkpoint");

        let header = ReplayHeader {
            mission_id: FIXTURE_MISSION_ID.into(),
            mission_assets: fixture_mission_assets(),
            rng_seed: 0,
            sim_config: engine.sim_config(),
            spellforge_package: None,
            version: REPLAY_SCHEMA_VERSION,
            total_frames: 27,
            rankability: ReplayRankability::rankable(),
            campaign: bitcode::encode(engine.campaign()),
        };
        let mut lines = vec![serde_json::to_string(&header).unwrap()];
        for ordinal in 0..27_u32 {
            // A live rollback may correct an earlier command, but the durable
            // replay is the canonical final command stream. It does not carry
            // the retired `c`/`hc` correction side channel.
            let input = if ordinal == 0 {
                SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
                    PlayerCommand::CrouchDown,
                )])
            } else if ordinal == 26 {
                let mut input = SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
                    PlayerCommand::QuitMissionRequested,
                )]);
                input.post_commands.push(
                    terminal_update(GameCode::LevelInterrupted, engine.sim_config().difficulty)
                        .into(),
                );
                input
            } else {
                SimulationFrameInput::default()
            };
            let hash = match ordinal {
                0 => Some(initial_hash),
                25 => Some(corrected_checkpoint),
                _ => None,
            };
            lines.push(
                serde_json::json!({
                    "f": ordinal,
                    "i": ReplayFrame {
                        timeline_before: ordinal,
                        timeline_after: ordinal + 1,
                        input,
                        host_controls: Vec::new(),
                    },
                    "h": hash,
                })
                .to_string(),
            );
        }
        let replay =
            ReplayData::from_reader(std::io::Cursor::new(format!("{}\n", lines.join("\n"))))
                .expect("corrected ranked replay");

        let result = resimulate_canonical_ranked_replay(engine, &assets, &replay)
            .expect("late-corrected replay must verify");
        assert_eq!(result.outcome, GameCode::LevelInterrupted);
        assert_eq!(result.replay_frames, 27);
    }
}

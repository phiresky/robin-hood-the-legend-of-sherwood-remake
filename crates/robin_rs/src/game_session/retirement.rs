//! The enclosing completion boundary for callback-owning session entrypoints.
//!
//! A body's early return ends only the body, never the mandatory save drain.
//! Restart iterations stay inside that body, so they retain the same owners.
//! This is explicit asynchronous completion, not an asynchronous Drop promise:
//! callers must await it; cancellation and unwinding retain existing owner Drop
//! fallbacks and are not reported as successful retirement.

use super::error::legacy_result_debug;
use super::{MissionError, MissionOutcome, SessionOutcome};
use crate::main_entry::RustCallbacks;

/// The owner is borrowed for the whole run, but the body cannot bypass its
/// completion step. Kept narrow so tests exercise actual control flow without
/// constructing a renderer or opening an application profile.
pub(super) trait SaveRetirement {
    fn retire_saves(&mut self) -> Result<(), MissionError>;
}

impl SaveRetirement for RustCallbacks {
    fn retire_saves(&mut self) -> Result<(), MissionError> {
        // This callback boundary drains both the special-save writer and autosave
        // coordinator even if the first fails; do not short-circuit its owners.
        self.finish_save_operations()
    }
}

pub(super) trait CompletionOutcome {
    fn complete_retirement(&mut self, retirement: Result<(), MissionError>);
}

/// Preserve the original success code or failure diagnostic as context;
/// campaign, simulation policy and other outcome fields remain intact.
fn retirement_failure<T: std::fmt::Debug, E: std::fmt::Display>(
    result: &Result<T, E>,
    scope: &'static str,
    error: MissionError,
) -> MissionError {
    MissionError::SaveRetirement {
        scope,
        error: Box::new(error),
        result: legacy_result_debug(result),
    }
}

impl CompletionOutcome for MissionOutcome {
    fn complete_retirement(&mut self, retirement: Result<(), MissionError>) {
        if let Err(error) = retirement {
            self.result = Err(retirement_failure(&self.result, "mission", error));
        }
    }
}

impl CompletionOutcome for SessionOutcome {
    fn complete_retirement(&mut self, retirement: Result<(), MissionError>) {
        if let Err(error) = retirement {
            // The session result is the menu-facing text boundary.
            self.result = Err(retirement_failure(&self.result, "session", error).to_string());
        }
    }
}

pub(super) async fn run<Owner: SaveRetirement, Outcome: CompletionOutcome>(
    owner: &mut Owner,
    body: impl AsyncFnOnce(&mut Owner) -> Outcome,
) -> Outcome {
    let mut outcome = body(owner).await;
    outcome.complete_retirement(owner.retire_saves());
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_session::SessionResult;
    use robin_engine::campaign::Campaign;
    use serde::{Deserialize, Serialize};

    #[derive(Default, Serialize, Deserialize)]
    struct Owner {
        events: Vec<String>,
        fail_retirement: bool,
    }

    impl SaveRetirement for Owner {
        fn retire_saves(&mut self) -> Result<(), MissionError> {
            self.events.push("retired".into());
            if self.fail_retirement {
                Err(MissionError::save("accepted write failed"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn early_return_is_enclosed_by_retirement() {
        for early_failure in [false, true] {
            let mut owner = Owner::default();
            let outcome = futures::executor::block_on(run(&mut owner, async |owner| {
                owner.events.push("entered".into());
                if early_failure {
                    return SessionOutcome {
                        campaign: Campaign::default(),
                        result: Err("preflight rejected".into()),
                    };
                }
                owner.events.push("ran".into());
                SessionOutcome {
                    campaign: Campaign::default(),
                    result: Ok(SessionResult::QuitToMenu),
                }
            }));
            assert_eq!(owner.events.last().unwrap(), "retired");
            assert_eq!(
                owner
                    .events
                    .iter()
                    .filter(|event| *event == "retired")
                    .count(),
                1
            );
            if early_failure {
                assert_eq!(outcome.result, Err("preflight rejected".into()));
                assert_eq!(owner.events, ["entered", "retired"]);
            } else {
                assert_eq!(outcome.result, Ok(SessionResult::QuitToMenu));
                assert_eq!(owner.events, ["entered", "ran", "retired"]);
            }
        }
    }

    #[test]
    fn retirement_failure_preserves_original_outcome_context() {
        for original in [
            Ok(SessionResult::ExitRequested),
            Err("launch failed".into()),
        ] {
            let mut owner = Owner {
                fail_retirement: true,
                ..Default::default()
            };
            let expected = format!(
                "session save retirement failed: accepted write failed; session result: {original:?}"
            );
            let outcome =
                futures::executor::block_on(run(&mut owner, async move |_| SessionOutcome {
                    campaign: Campaign::default(),
                    result: original,
                }));
            assert_eq!(outcome.result, Err(expected));
            assert_eq!(owner.events, ["retired"]);
        }
    }

    #[test]
    fn mission_completion_preserves_simulation_metadata() {
        for original in [
            Ok(super::super::GameCode::LevelSucceeded),
            Err::<_, String>("mission setup failed".into()),
        ] {
            let campaign = Campaign {
                current_mission_idx: Some(7),
                ..Default::default()
            };
            let expected = format!(
                "mission save retirement failed: autosave failed; mission result: {original:?}"
            );
            let mut outcome = MissionOutcome::new(
                campaign,
                73,
                Default::default(),
                original.map_err(MissionError::launch),
            );
            let sim_config = outcome.sim_config;
            outcome.complete_retirement(Err(MissionError::save("autosave failed")));
            assert_eq!(outcome.rng_seed, 73);
            assert_eq!(outcome.sim_config, sim_config);
            assert_eq!(outcome.campaign.current_mission_idx, Some(7));
            assert_eq!(
                outcome.result.map_err(|error| error.to_string()),
                Err(expected)
            );
        }
    }

    #[test]
    fn restart_iterations_keep_owner_until_enclosing_completion() {
        let mut owner = Owner::default();
        futures::executor::block_on(run(&mut owner, async |owner| {
            for attempt in 0..3 {
                assert!(!owner.events.iter().any(|event| event == "retired"));
                owner.events.push(format!("attempt {attempt}"));
            }
            SessionOutcome {
                campaign: Campaign::default(),
                result: Ok(SessionResult::QuitToMenu),
            }
        }));
        assert_eq!(
            owner.events,
            ["attempt 0", "attempt 1", "attempt 2", "retired"]
        );
    }
}

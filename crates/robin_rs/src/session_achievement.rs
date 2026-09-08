//! Achievement persistence facts owned by one launched mission, not its renderer.
//!
//! This policy does not participate in deterministic save/replay state. Loading a
//! save retains the receiving session's policy; it cannot turn playback into a
//! live, achievement-eligible campaign.

use robin_engine::achievement::{AchievementRunContext, AchievementRunKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionExecutionMode {
    Interactive,
    Headless,
}

/// Immutable launch facts. Deliberately has no permissive `Default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAchievementEligibility {
    kind: AchievementRunKind,
    replay_playback: bool,
    mode: SessionExecutionMode,
}

impl SessionAchievementEligibility {
    pub(crate) fn from_launch(
        args: &crate::main_entry::CliArgs,
        mode: SessionExecutionMode,
    ) -> Self {
        Self::from_launch_facts(
            args.custom_mission.is_some(),
            args.pending_lua_mission.is_some(),
            args.resolved_mission_assets
                .as_ref()
                .is_some_and(|resolved| resolved.is_archive()),
            args.replay.is_some(),
            args.replay_data.is_some(),
            mode,
        )
    }

    fn from_launch_facts(
        custom_mission: bool,
        lua_mission: bool,
        archive: bool,
        replay_path: bool,
        replay_data: bool,
        mode: SessionExecutionMode,
    ) -> Self {
        Self {
            kind: if custom_mission || lua_mission || archive {
                AchievementRunKind::CustomMission
            } else {
                AchievementRunKind::Campaign
            },
            replay_playback: replay_path || replay_data,
            mode,
        }
    }

    /// Transport membership remains a settlement-time fact, as before. The
    /// engine adds its authoritative cheat-used observation during promotion.
    pub(crate) fn promotion_context(self, multiplayer: bool) -> AchievementRunContext {
        AchievementRunContext {
            kind: self.kind,
            multiplayer,
            replay_playback: self.replay_playback,
            headless: self.mode == SessionExecutionMode::Headless,
            cheat_used: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_achievement_launch_matrix_preserves_all_previous_facts() {
        // Exercise every combination, including simultaneous CLI and resolved
        // launch facts. Custom and replay flags must never cancel each other.
        for flags in 0u8..32 {
            for mode in [
                SessionExecutionMode::Interactive,
                SessionExecutionMode::Headless,
            ] {
                let eligibility = SessionAchievementEligibility::from_launch_facts(
                    flags & 1 != 0,
                    flags & 2 != 0,
                    flags & 4 != 0,
                    flags & 8 != 0,
                    flags & 16 != 0,
                    mode,
                );
                for multiplayer in [false, true] {
                    assert_eq!(
                        eligibility.promotion_context(multiplayer),
                        AchievementRunContext {
                            kind: if flags & 7 != 0 {
                                AchievementRunKind::CustomMission
                            } else {
                                AchievementRunKind::Campaign
                            },
                            multiplayer,
                            replay_playback: flags & 24 != 0,
                            headless: mode == SessionExecutionMode::Headless,
                            cheat_used: false,
                        }
                    );
                }
                let json = serde_json::to_string(&eligibility).unwrap();
                assert_eq!(
                    serde_json::from_str::<SessionAchievementEligibility>(&json).unwrap(),
                    eligibility
                );
            }
        }
    }

    #[test]
    fn session_achievement_cli_path_flags_are_derived_once() {
        let mut args = crate::main_entry::CliArgs::default();
        args.custom_mission = Some("mission.rhm".into());
        args.replay = Some("playback.rhrec".into());
        let eligibility =
            SessionAchievementEligibility::from_launch(&args, SessionExecutionMode::Interactive);
        args.custom_mission = None;
        args.replay = None;
        let context = eligibility.promotion_context(false);
        assert_eq!(context.kind, AchievementRunKind::CustomMission);
        assert!(context.replay_playback);
        assert!(!context.headless);
    }

    #[test]
    fn session_achievement_binding_is_required_once_and_survives_load_reset() {
        let mut host = crate::host::Host::scratch(640.0, 480.0);
        assert!(host.session_achievement_eligibility().is_err());
        let policy = SessionAchievementEligibility::from_launch(
            &crate::main_entry::CliArgs::default(),
            SessionExecutionMode::Headless,
        );
        host.bind_session_achievement_eligibility(policy).unwrap();
        assert!(host.bind_session_achievement_eligibility(policy).is_err());
        host.post_load_reset();
        assert_eq!(host.session_achievement_eligibility().unwrap(), policy);
        assert!(
            crate::host::Host::default()
                .session_achievement_eligibility()
                .is_err()
        );
    }
}

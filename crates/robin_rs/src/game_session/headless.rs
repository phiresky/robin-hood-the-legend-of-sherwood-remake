//! Complete ownership and policy for a loaded true-headless mission.

use super::runtime::MissionRuntime;
use serde::{Deserialize, Serialize};

/// Frontend behavior which intentionally differs from interactive play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HeadlessPolicy {
    pub(super) auto_dismiss_modals: bool,
    pub(super) exit_when_replay_finishes: bool,
    pub(super) wait_for_multiplayer_start: bool,
}

impl HeadlessPolicy {
    pub(super) const fn replay_runner() -> Self {
        Self {
            auto_dismiss_modals: true,
            exit_when_replay_finishes: true,
            // TODO(parity): Teach the true-headless driver to drain the
            // multiplayer start barrier before enabling this.
            wait_for_multiplayer_start: false,
        }
    }
}

/// Complete process owner returned by true-headless bootstrap.
///
/// It deliberately does not implement serde: timeline writers, network
/// handles, and rollback workers are process resources.
pub(super) struct HeadlessMission {
    pub(super) runtime: MissionRuntime,
    pub(super) policy: HeadlessPolicy,
}

#[cfg(test)]
mod tests {
    use super::HeadlessPolicy;

    #[test]
    fn replay_runner_policy_keeps_current_headless_contract_explicit() {
        let policy = HeadlessPolicy::replay_runner();

        assert!(policy.auto_dismiss_modals);
        assert!(policy.exit_when_replay_finishes);
        assert!(!policy.wait_for_multiplayer_start);
    }
}

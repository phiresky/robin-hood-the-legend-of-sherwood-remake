//! Behaviour shared by the two typed AI roles (`EnemyAi`, `FriendlyAi`).
//!
//! Both roles share local admission before the script filter.

use super::{AiController, Stimulus, StimulusInfo};

pub(crate) trait AiRole {
    fn base_mut(&mut self) -> &mut AiController;

    /// Decision-tick admission work which precedes the script `FilterAIEvent` call.
    /// Kept separate so script-native SetAIState can yield through the VM at
    /// the exact callback boundary without aliasing the typed brain.
    fn start_think_pre_filter(&mut self, stimulus: &Stimulus) {
        let base = self.base_mut();

        // Reset per-think flags
        base.couldnt_reachpoint = false;
        base.already_on_point = false;
        base.already_turned = false;
        base.old_state = base.current_state as i32;

        // Track stimulus actor
        if let StimulusInfo::Human(h) = stimulus.info {
            base.last_stimulus_actor = Some(h);
        }
    }
}

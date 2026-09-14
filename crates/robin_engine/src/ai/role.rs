//! Behaviour shared by the two typed AI roles (`EnemyAi`, `FriendlyAi`).
//!
//! Both roles share admission before the script filter and supply their
//! own alert-status setter.

use super::{AiController, AlertLevel, Position, Stimulus, StimulusInfo, StimulusType};

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct AiAdmission {
    pub frame: u32,
    pub original_creation_order: Option<u32>,
    pub think_depth: u8,
    pub in_building: bool,
    pub self_is_rider: bool,
    pub self_is_dead: bool,
    pub self_is_unconscious: bool,
    pub posture: crate::element::Posture,
    pub position: Position,
}

pub(crate) trait AiRole {
    fn base_mut(&mut self) -> &mut AiController;

    /// The role's own `set_alert_status`.
    fn role_set_alert_status(&mut self, level: AlertLevel);

    /// Decision-tick admission work which precedes the script `FilterAIEvent` call.
    /// Kept separate so script-native SetAIState can yield through the VM at
    /// the exact callback boundary without aliasing the typed brain.
    fn start_think_pre_filter(&mut self, stimulus: &Stimulus) {
        let stimulus_type = stimulus.stimulus_type;
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

        // LOSE_CONSCIOUSNESS always drops the alert to green regardless of
        // the downstream refusal — even when the event is otherwise filtered
        // out.
        if stimulus_type == StimulusType::EventLoseConsciousness {
            self.role_set_alert_status(AlertLevel::Green);
        }
    }
}

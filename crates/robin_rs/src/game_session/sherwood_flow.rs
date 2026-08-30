//! Frame-owned Sherwood campaign-map flow.
//!
//! The enum keeps each visual screen alive across outer mission frames.  The
//! driver remains in `mouse_input` because it owns the campaign mutations and
//! mission-selection commands produced when a screen completes.

use crate::campaign_map::CampaignMapModalState;
use crate::ingame_menu::{
    DebriefingModalState, YesNoModalState, mission_description::MissionDescriptionModalState,
};
use robin_engine::engine::ExternalAction;

pub(super) enum SherwoodCampaignFlow {
    Map(CampaignMapModalState),
    PseudoDebrief {
        state: DebriefingModalState,
    },
    MissionDescription {
        mission_index: usize,
        state: MissionDescriptionModalState,
        admitted_actions: Vec<ExternalAction>,
    },
    Confirmation {
        state: YesNoModalState,
        action: SherwoodConfirmationAction,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SherwoodConfirmationAction {
    ReturnToMap,
    StartMission { men_to_blazon: bool },
}

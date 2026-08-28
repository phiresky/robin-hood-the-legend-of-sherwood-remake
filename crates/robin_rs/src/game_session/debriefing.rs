//! Mission debriefing policy and frame-driven lost-campaign gate.

use super::interactive::{MissionPresentation, MissionResources};
use crate::host::Host;
use crate::ingame_menu::resources::MT_MSG_STRATEGICAL_MISSION_LOST;
use crate::ingame_menu::widget_bridge::default_modal_cursor;
use crate::window::GameWindow;
use robin_assets::res_descr as assets_res_descr;
use robin_engine::engine::Engine;
use robin_engine::mission::MissionStatus;
use robin_engine::player_command::DialogResult;

pub(super) struct LostSherwoodGateState {
    checked: bool,
    modal: Option<crate::ingame_menu::DebriefingModalState>,
}

impl LostSherwoodGateState {
    pub(super) fn new() -> Self {
        Self {
            checked: false,
            modal: None,
        }
    }

    pub(super) fn blocks_mission(&self, is_sherwood: bool, engine: &Engine) -> bool {
        !self.checked && is_sherwood && engine.campaign().get_ares() == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LostSherwoodGateProgress {
    Inactive,
    Pending,
    Exit,
}

/// Resolved outcome of the post-mission debriefing flow after the caller has
/// driven any Load-picker re-entry loop.
pub(super) enum SettledDebriefingOutcome {
    Ok,
    Restart,
    Load { slot: usize },
    EmergencyEnd,
}

pub(super) fn final_debriefing_result(outcome: &SettledDebriefingOutcome) -> DialogResult {
    match outcome {
        SettledDebriefingOutcome::Ok => DialogResult::Completed,
        SettledDebriefingOutcome::Restart => DialogResult::Restart,
        SettledDebriefingOutcome::Load { slot } => DialogResult::Load { slot: *slot as u32 },
        SettledDebriefingOutcome::EmergencyEnd => DialogResult::Aborted,
    }
}

pub(super) fn final_debriefing_outcome_from_replay(
    result: DialogResult,
) -> SettledDebriefingOutcome {
    match result {
        DialogResult::Completed => SettledDebriefingOutcome::Ok,
        DialogResult::Aborted => SettledDebriefingOutcome::EmergencyEnd,
        DialogResult::Restart => SettledDebriefingOutcome::Restart,
        DialogResult::Load { slot } => SettledDebriefingOutcome::Load {
            slot: slot as usize,
        },
    }
}

/// Advance the original defeat debriefing for a Sherwood save whose campaign
/// already has no remaining ARES by one outer mission frame.
pub(super) fn drive_lost_sherwood_gate(
    state: &mut LostSherwoodGateState,
    window: &mut GameWindow,
    host: &Host,
    engine: &Engine,
    is_sherwood: bool,
    resources: &mut MissionResources,
    presentation: &mut MissionPresentation,
) -> LostSherwoodGateProgress {
    if state.checked {
        return LostSherwoodGateProgress::Inactive;
    }
    if !is_sherwood {
        state.checked = true;
        return LostSherwoodGateProgress::Inactive;
    }

    let campaign = engine.campaign();
    if campaign.get_ares() != 0 {
        state.checked = true;
        return LostSherwoodGateProgress::Inactive;
    }

    let Some(menu_resources) = resources.menu.as_ref() else {
        tracing::warn!("Lost-Leicester: menu resources unavailable — skipping debriefing popup");
        state.checked = true;
        return LostSherwoodGateProgress::Exit;
    };
    if state.modal.is_none() {
        let (last_id, last_status) = {
            (
                campaign.last_pseudo_mission_id,
                campaign.last_pseudo_mission_status,
            )
        };
        if last_status != MissionStatus::Lost {
            tracing::warn!(
                ?last_status,
                "Lost-Leicester gate: ARES=0 but last pseudo-mission status != Lost"
            );
        }

        let pseudo_red = {
            let filename = assets_res_descr::red_filename(last_id);
            host.shipping
                .as_deref()
                .and_then(|datadir| datadir.red_files.get(&filename).cloned())
                .or_else(|| {
                    let path = format!("Data/Text/{filename}");
                    assets_res_descr::load(&path)
                        .map_err(|error| {
                            tracing::warn!(
                                "Lost-Leicester: failed to load pseudo-mission .red {path}: {error}"
                            );
                            error
                        })
                        .ok()
                })
        };
        let per_mission_text = pseudo_red.as_ref().and_then(|descriptor| {
            let table_id = descriptor.debriefing.lose_text_table_id;
            if !resources.text.has_text_resource(table_id) {
                return None;
            }
            match resources.text.get_string(table_id, 0) {
                Ok(text) => Some(text.to_string()),
                Err(error) => {
                    tracing::warn!(
                        "Lost-Leicester: lose_text_table_id {table_id} sub 0 not found: {error}"
                    );
                    None
                }
            }
        });

        let text = per_mission_text.unwrap_or_else(|| {
            menu_resources
                .menu_text
                .get(MT_MSG_STRATEGICAL_MISSION_LOST)
        });
        state.modal = Some(crate::ingame_menu::DebriefingModalState::new(
            menu_resources,
            text,
            None,
            0,
            false,
            false,
            None,
            false,
            false,
        ));
    }
    let cursor = default_modal_cursor(
        &mut presentation.sprites.cursor_renderer,
        &mut resources.cursor,
        &mut presentation.renderer,
    );
    if state
        .modal
        .as_mut()
        .and_then(|modal| {
            modal.tick(
                window,
                &mut presentation.renderer,
                menu_resources,
                Some(cursor),
            )
        })
        .is_none()
    {
        return LostSherwoodGateProgress::Pending;
    }

    tracing::info!("Sherwood entry with ARES=0 (lost campaign) — returning to main menu");
    state.checked = true;
    state.modal = None;
    LostSherwoodGateProgress::Exit
}

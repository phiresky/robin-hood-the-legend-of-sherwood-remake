//! Mission debriefing policy and blocking pre-loop debriefing flows.

use super::interactive::InteractiveFrontendAssembly;
use crate::host::Host;
use crate::ingame_menu::resources::MT_MSG_STRATEGICAL_MISSION_LOST;
use crate::ingame_menu::widget_bridge::default_modal_cursor;
use crate::window::GameWindow;
use robin_assets::res_descr as assets_res_descr;
use robin_engine::engine::Engine;
use robin_engine::game_operation::GameCode;
use robin_engine::mission::MissionStatus;
use robin_engine::player_command::DialogResult;

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

/// Display the original pre-loop defeat debriefing for a Sherwood save whose
/// campaign already has no remaining ARES.
///
/// Returns `Some(Quit)` only after the blocking debriefing has closed. The
/// caller still owns the mission and must consume it to recover the campaign.
pub(super) async fn run_lost_sherwood_gate(
    window: &mut GameWindow,
    host: &Host,
    engine: &Engine,
    frontend: &mut InteractiveFrontendAssembly,
) -> Option<GameCode> {
    if !frontend.is_sherwood {
        return None;
    }

    let campaign = engine.campaign();
    if campaign.get_ares() != 0 {
        return None;
    }

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
        if !frontend.resources.text.has_text_resource(table_id) {
            return None;
        }
        match frontend.resources.text.get_string(table_id, 0) {
            Ok(text) => Some(text.to_string()),
            Err(error) => {
                tracing::warn!(
                    "Lost-Leicester: lose_text_table_id {table_id} sub 0 not found: {error}"
                );
                None
            }
        }
    });

    if let Some(resources) = frontend.resources.menu.as_ref() {
        let text = per_mission_text
            .unwrap_or_else(|| resources.menu_text.get(MT_MSG_STRATEGICAL_MISSION_LOST));
        let cursor = Some(default_modal_cursor(
            &mut frontend.sprites.cursor_renderer,
            &mut frontend.resources.cursor,
            &mut frontend.renderer,
        ));
        let _ = crate::ingame_menu::show_debriefing(
            window,
            &mut frontend.renderer,
            resources,
            cursor,
            &text,
            None,
            0,
            false,
            false,
            None,
            false,
            false,
        )
        .await;
    } else {
        tracing::warn!("Lost-Leicester: menu resources unavailable — skipping debriefing popup");
    }

    tracing::info!("Sherwood entry with ARES=0 (lost campaign) — returning to main menu");
    Some(GameCode::Quit)
}

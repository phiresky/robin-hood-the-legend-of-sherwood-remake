//! Blocking mission-end popup, debriefing renderer, and load picker.
//!
//! These helpers remain inline in the graphical modal phase. They never run on
//! the headless path and return only the control decision needed by the caller.

use super::interactive::{
    MissionAudio, MissionInput, MissionPresentation, MissionResources, MissionUi,
};
use super::*;
use crate::game::Game;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum TerminalDebriefingAction {
    Continue,
    LoadRestart,
    Load { slot: usize, mission_id: u32 },
    EmergencyExit,
}

fn terminal_debriefing_action(
    outcome: &SettledDebriefingOutcome,
    mission_id: u32,
) -> TerminalDebriefingAction {
    match outcome {
        SettledDebriefingOutcome::Ok => TerminalDebriefingAction::Continue,
        SettledDebriefingOutcome::Restart => TerminalDebriefingAction::LoadRestart,
        SettledDebriefingOutcome::Load { slot } => TerminalDebriefingAction::Load {
            slot: *slot,
            mission_id,
        },
        SettledDebriefingOutcome::EmergencyEnd => TerminalDebriefingAction::EmergencyExit,
    }
}

fn mission_completion_clock() -> (Option<i64>, Option<u64>) {
    let Ok(duration) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return (None, None);
    };
    (
        i64::try_from(duration.as_secs()).ok(),
        u64::try_from(duration.as_nanos()).ok(),
    )
}

fn debriefing_text_table_id(won: bool, win_table_id: i32, lose_table_id: i32) -> i32 {
    if won { win_table_id } else { lose_table_id }
}

struct TerminalDebriefingPage {
    kind: engine_player_command::ModalKind,
    body: String,
    mission_length: u32,
    quick_load_key: Option<winit::keyboard::KeyCode>,
    restart_snapshot_exists: bool,
    mission_id: u32,
    won: bool,
}

/// Explicit owners borrowed by the blocking terminal graphical flow.
pub(super) struct TerminalDebriefingContext<'a> {
    pub(super) tick_exit_code: Option<GameCode>,
    pub(super) host: &'a mut Host,
    pub(super) game: &'a mut Game,
    pub(super) manager: &'a mut robin_engine::engine_manager::EngineManager,
    pub(super) assets: &'a robin_engine::engine::LevelAssets,
    pub(super) window: &'a mut GameWindow,
    pub(super) callbacks: &'a mut RustCallbacks,
    pub(super) input: &'a mut MissionInput,
    pub(super) audio: &'a mut MissionAudio,
    pub(super) resources: &'a mut MissionResources,
    pub(super) ui: &'a mut MissionUi,
    pub(super) presentation: &'a mut MissionPresentation,
    pub(super) frame: &'a mut MissionFrame,
}

fn terminal_debriefing_page(
    context: &mut TerminalDebriefingContext<'_>,
    won: bool,
    terminal_mission_id: u32,
) -> TerminalDebriefingPage {
    let index = context.manager.engine.mission().victory_defeat_id as usize;
    let kind = engine_player_command::ModalKind::FinalDebriefing {
        text_id: engine_player_command::DebriefingTextId::from_outcome(won, index),
    };
    let mut body = if let Some(descriptors) = context.resources.level_descriptors.as_ref() {
        let table_id = debriefing_text_table_id(
            won,
            descriptors.debriefing.win_text_table_id,
            descriptors.debriefing.lose_text_table_id,
        );
        match context.resources.text.get_string(table_id, index) {
            Ok(text) => text.to_string(),
            Err(error) => {
                tracing::warn!(
                    "Debriefing text lookup failed (table={table_id}, index={index}): {error}"
                );
                "Invalid debriefing ID...".to_string()
            }
        }
    } else {
        tracing::warn!("Debriefing text lookup: level descriptors unavailable");
        "No dynamic resources for this level...".to_string()
    };
    if won
        && context.host.gameplay_config.show_achievement_debrief
        && let Some(results) = context.manager.engine.mission_achievement_results()
    {
        body.push_str("\n\n");
        body.push_str(&crate::achievement_hud::format_attempt_summary(*results));
    }
    let mission_length = <RustCallbacks as crate::game::GameCallbacks>::get_current_playing_time(
        context.callbacks,
        context.manager.engine.campaign(),
    );
    let quick_load_key = context.input.translator.get_binding(GameKey::QuickLoad1);
    let restart_snapshot_exists =
        context.ui.restart_allowed && context.callbacks.save_manager.has_restart_save();
    TerminalDebriefingPage {
        kind,
        body,
        mission_length,
        quick_load_key,
        restart_snapshot_exists,
        mission_id: terminal_mission_id,
        won,
    }
}

async fn show_terminal_mission_state(
    context: &mut TerminalDebriefingContext<'_>,
    popup_title: &str,
    won: bool,
) {
    let kind = engine_player_command::ModalKind::MissionState {
        kind: engine_player_command::MissionStateModalKind::EndState { won },
    };
    let result = match pop_matching_dismissal(&mut context.frame.replay_modal_dismissals, &kind) {
        Some(
            result @ (engine_player_command::DialogResult::Completed
            | engine_player_command::DialogResult::Aborted),
        ) => result,
        Some(result) => {
            tracing::warn!(
                ?result,
                "final mission-state replay result is only yes/no; treating as aborted"
            );
            engine_player_command::DialogResult::Aborted
        }
        None => {
            let menu_resources = context
                .resources
                .menu
                .as_ref()
                .expect("terminal mission-state resources were checked by the caller");
            let cursor = Some(default_modal_cursor(
                &mut context.presentation.sprites.cursor_renderer,
                &mut context.resources.cursor,
                &mut context.presentation.renderer,
            ));
            if crate::ingame_menu::show_mission_state_popup(
                context.window,
                &mut context.presentation.renderer,
                menu_resources,
                cursor,
                popup_title,
                won,
                None,
            )
            .await
            {
                engine_player_command::DialogResult::Completed
            } else {
                engine_player_command::DialogResult::Aborted
            }
        }
    };
    context
        .frame
        .modal_dismissals
        .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
}

/// Render the current debriefing page and, when requested, run the load picker.
/// Cancelling the picker re-enters the exact body/stat page that launched it.
async fn render_terminal_debriefing_and_picker(
    context: &mut TerminalDebriefingContext<'_>,
    page: &TerminalDebriefingPage,
) -> SettledDebriefingOutcome {
    if let Some(result) =
        pop_matching_dismissal(&mut context.frame.replay_modal_dismissals, &page.kind)
    {
        return final_debriefing_outcome_from_replay(result);
    }

    let mut current_body = page.body.clone();
    let mut start_at_stat = false;
    loop {
        let menu_resources = context
            .resources
            .menu
            .as_ref()
            .expect("terminal debriefing resources were checked by the caller");
        let cursor = Some(default_modal_cursor(
            &mut context.presentation.sprites.cursor_renderer,
            &mut context.resources.cursor,
            &mut context.presentation.renderer,
        ));
        let outcome = crate::ingame_menu::show_debriefing(
            context.window,
            &mut context.presentation.renderer,
            menu_resources,
            cursor,
            &current_body,
            Some(context.manager.engine.mission_stat()),
            page.mission_length,
            page.won,
            context.ui.restart_allowed,
            page.quick_load_key,
            page.restart_snapshot_exists,
            start_at_stat,
        )
        .await;
        match outcome {
            DebriefingOutcome::LoadAttempt {
                body_remaining,
                was_on_stat,
            } => {
                let cursor = Some(default_modal_cursor(
                    &mut context.presentation.sprites.cursor_renderer,
                    &mut context.resources.cursor,
                    &mut context.presentation.renderer,
                ));
                let picker_outcome = crate::ingame_menu::show_save_load(
                    context.window,
                    &mut context.presentation.renderer,
                    menu_resources,
                    cursor,
                    &mut context.callbacks.save_manager,
                    page.mission_id,
                    Some(&context.assets.profile_manager),
                    SaveLoadMode::Load,
                    Some(&mut context.host.audio.sound),
                    context
                        .audio
                        .backend
                        .as_mut()
                        .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
                    Some(&context.audio.sample_loader),
                )
                .await;
                match picker_outcome {
                    SaveLoadOutcome::Slot(slot) => {
                        break SettledDebriefingOutcome::Load { slot };
                    }
                    SaveLoadOutcome::Cancel => {
                        current_body = body_remaining;
                        start_at_stat = was_on_stat;
                    }
                }
            }
            DebriefingOutcome::Ok { .. } => break SettledDebriefingOutcome::Ok,
            DebriefingOutcome::Restart => break SettledDebriefingOutcome::Restart,
            DebriefingOutcome::EmergencyEnd => break SettledDebriefingOutcome::EmergencyEnd,
        }
    }
}

fn apply_terminal_debriefing_action(
    context: &mut TerminalDebriefingContext<'_>,
    outcome: &SettledDebriefingOutcome,
    mission_id: u32,
) -> bool {
    match terminal_debriefing_action(outcome, mission_id) {
        TerminalDebriefingAction::Continue => false,
        TerminalDebriefingAction::LoadRestart => {
            context.callbacks.pending = Some(SaveLoadRequest::LoadRestart);
            context.game.operation.set(GameCode::LevelInProgress);
            false
        }
        TerminalDebriefingAction::Load { slot, mission_id } => {
            context.callbacks.pending = Some(SaveLoadRequest::Load {
                slot: Some(slot),
                mission_id,
                save: None,
            });
            context.game.operation.set(GameCode::LevelInProgress);
            false
        }
        TerminalDebriefingAction::EmergencyExit => true,
    }
}

/// Resolve an engine tick exit through the original mission-state/debriefing
/// flow. Returns true only for an emergency window close.
pub(super) async fn drive_tick_exit_modals(mut context: TerminalDebriefingContext<'_>) -> bool {
    let Some(exit_code) = context.tick_exit_code else {
        return false;
    };
    tracing::info!("Engine tick returned: {:?}", exit_code);
    // A history replay restores campaign progression while applying terminal
    // updates. Freeze the loaded mission identity first so the debriefing and
    // load/restart actions continue to refer to the mission just played.
    let terminal_mission_id = current_mission_id(
        context.manager.engine.campaign(),
        &context.assets.profile_manager,
    );
    let (completed_at_unix_seconds, campaign_run_nonce) = mission_completion_clock();

    // Campaign/stat updates precede both terminal graphical surfaces, matching
    // RHgame.cpp's mission-end operation handling.
    let difficulty = context.manager.engine.sim_config().difficulty;
    dispatch_local_command(
        context.host,
        &mut context.manager.engine,
        &mut context.frame.post_commands,
        context.assets,
        &PlayerCommand::ApplyQuitMissionUpdates {
            exit_code,
            difficulty,
            completed_at_unix_seconds,
            campaign_run_nonce,
        },
    );
    if exit_code == GameCode::LevelSucceeded {
        let run_context = robin_engine::achievement::AchievementRunContext {
            kind: context.host.achievement_run_kind,
            multiplayer: context.host.transport.net.is_some(),
            replay_playback: context.host.achievement_replay_playback,
            headless: context.host.achievement_headless,
            // The engine ORs its authoritative cheat flag into this value.
            cheat_used: false,
        };
        let update = context
            .manager
            .engine
            .promote_mission_achievement_results(
                robin_engine::achievement::AchievementUnlockPolicy::default(),
                run_context,
                &context.assets.profile_manager,
            )
            .unwrap_or_else(|error| panic!("achievement history promotion failed: {error}"));
        if let Some(update) = update
            && !update.blockers.is_empty()
        {
            tracing::info!(
                ?run_context,
                "achievement results calculated but unlock/history persistence was blocked"
            );
        }
    }

    // ApplyQuitMissionUpdates first appends the immutable raw attempt and the
    // success path above attaches its exactly-once eligibility attestation.
    // Only then promote that canonical campaign history into the profile, so
    // failed/interrupted attempts and policy-blocked calculations remain
    // losslessly auditable without becoming awarded badges.
    let campaign = context.manager.engine.campaign().clone();
    context
        .host
        .application_context
        .with_player_profiles_mut(|profiles| {
            let profile = profiles.get_active_mut().unwrap_or_else(|| {
                panic!("campaign-history promotion has no active player profile")
            });
            profile
                .promote_campaign_history(&campaign, &context.assets.profile_manager)
                .unwrap_or_else(|error| panic!("campaign-history promotion failed: {error}"));
            if let Err(error) = profiles.save() {
                #[cfg(not(target_arch = "wasm32"))]
                panic!("failed to persist campaign history: {error}");
                #[cfg(target_arch = "wasm32")]
                tracing::warn!(
                    "Failed to persist campaign history in browser storage; keeping it in memory for this session: {error}"
                );
            }
        })
        .unwrap_or_else(|error| panic!("campaign profile synchronization failed: {error}"));

    let Some((popup_title, _)) = crate::ingame_menu::mission_state_text(exit_code) else {
        return false;
    };
    if context.resources.menu.is_none() {
        return false;
    }

    let won = exit_code == GameCode::LevelSucceeded;
    show_terminal_mission_state(&mut context, popup_title, won).await;
    let page = terminal_debriefing_page(&mut context, won, terminal_mission_id);
    let outcome = render_terminal_debriefing_and_picker(&mut context, &page).await;
    context
        .frame
        .modal_dismissals
        .push(engine_player_command::PlayerCommand::ModalDismiss {
            kind: page.kind.clone(),
            result: final_debriefing_result(&outcome),
        });
    apply_terminal_debriefing_action(&mut context, &outcome, page.mission_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_debriefing_maps_to_explicit_control_actions() {
        let mission_id = 42;
        let cases = [
            (
                SettledDebriefingOutcome::Ok,
                TerminalDebriefingAction::Continue,
            ),
            (
                SettledDebriefingOutcome::Restart,
                TerminalDebriefingAction::LoadRestart,
            ),
            (
                SettledDebriefingOutcome::Load { slot: 7 },
                TerminalDebriefingAction::Load {
                    slot: 7,
                    mission_id,
                },
            ),
            (
                SettledDebriefingOutcome::EmergencyEnd,
                TerminalDebriefingAction::EmergencyExit,
            ),
        ];
        for (outcome, expected) in cases {
            assert_eq!(terminal_debriefing_action(&outcome, mission_id), expected);
        }
    }

    #[test]
    fn terminal_text_table_follows_win_loss_outcome() {
        assert_eq!(debriefing_text_table_id(true, 100, 200), 100);
        assert_eq!(debriefing_text_table_id(false, 100, 200), 200);
    }
}

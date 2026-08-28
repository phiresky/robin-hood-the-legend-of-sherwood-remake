//! Frame-owned cross-mission QuickLoad confirmation.
//!
//! The validated save payload stays pinned while the yes/no screen is open,
//! so neither save metadata nor the file on disk can change the eventual load.

use super::{required_menu_resources, validated_save_reload_target};
use crate::ingame_menu::YesNoModalState;
use crate::ingame_menu::resources::MT_MSG_REALLY_LOAD_QUICKSAVE;
use crate::ingame_menu::widget_bridge::default_modal_cursor;
use crate::main_entry::{RustCallbacks, SaveLoadRequest, current_mission_id};
use crate::save_file::{GameSaveFile, special_slots};
use crate::window::GameWindow;
use robin_engine::engine::Engine;
use robin_engine::profiles::ProfileManager;

pub(super) enum QuickLoadConfirmationFlow {
    Awaiting {
        slot: usize,
        mission_id: u32,
        save: GameSaveFile,
        modal: YesNoModalState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuickLoadConfirmationProgress {
    Inactive,
    Pending,
    Resolved,
}

/// Advance the cross-mission confirmation by one outer mission frame.
///
/// A newly queued same-mission QuickLoad is rewritten to an exact `Load`
/// immediately. A cross-mission request opens a retained yes/no state and
/// clears the public callback slot until the player decides.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_quickload_confirmation(
    flow: &mut Option<QuickLoadConfirmationFlow>,
    callbacks: &mut RustCallbacks,
    engine: &Engine,
    profiles: &ProfileManager,
    window: &mut GameWindow,
    presentation: &mut super::interactive::MissionPresentation,
    resources: &mut super::interactive::MissionResources,
) -> QuickLoadConfirmationProgress {
    if flow.is_none() {
        let use_backup = match callbacks.pending {
            Some(SaveLoadRequest::QuickLoad { use_backup }) => use_backup,
            _ => return QuickLoadConfirmationProgress::Inactive,
        };
        let slot_name = if use_backup {
            special_slots::EX_QUICK
        } else {
            special_slots::QUICK
        };
        let Some(slot) = callbacks.save_manager.find_by_filename(slot_name) else {
            return QuickLoadConfirmationProgress::Inactive;
        };
        if !callbacks.save_manager.slot_file_exists(slot) {
            return QuickLoadConfirmationProgress::Inactive;
        }
        let save = match callbacks.save_manager.preflight_exact_slot(slot) {
            Ok(save) => save,
            Err(error) => {
                tracing::error!(
                    "QuickLoad confirmation preflight failed for {slot_name}: {error:#}"
                );
                callbacks.pending = None;
                return QuickLoadConfirmationProgress::Resolved;
            }
        };
        if let Err(error) = callbacks.save_manager.validate_slot_identity(slot, &save) {
            tracing::error!("QuickLoad confirmation rejected stale {slot_name} slot: {error:#}");
            callbacks.pending = None;
            return QuickLoadConfirmationProgress::Resolved;
        }
        let mission_id = current_mission_id(engine.campaign(), profiles);
        let target_mission_id = match validated_save_reload_target(&save, profiles, mission_id) {
            Ok(target) => target,
            Err(error) => {
                tracing::error!("QuickLoad confirmation rejected {slot_name}: {error}");
                callbacks.pending = None;
                return QuickLoadConfirmationProgress::Resolved;
            }
        };
        if target_mission_id.is_none() {
            callbacks.pending = Some(SaveLoadRequest::Load {
                slot: Some(slot),
                mission_id,
                save: Some(save),
            });
            return QuickLoadConfirmationProgress::Resolved;
        }

        let menu = required_menu_resources(&resources.menu, "cross-mission QuickLoad confirmation");
        let message = menu.menu_text.get(MT_MSG_REALLY_LOAD_QUICKSAVE);
        let modal = YesNoModalState::new(window, &presentation.renderer, menu, message);
        callbacks.pending = None;
        *flow = Some(QuickLoadConfirmationFlow::Awaiting {
            slot,
            mission_id,
            save,
            modal,
        });
    }

    let Some(QuickLoadConfirmationFlow::Awaiting { modal, .. }) = flow.as_mut() else {
        unreachable!("QuickLoad flow was initialized above")
    };
    let menu = required_menu_resources(&resources.menu, "cross-mission QuickLoad confirmation");
    let cursor = default_modal_cursor(
        &mut presentation.sprites.cursor_renderer,
        &mut resources.cursor,
        &mut presentation.renderer,
    );
    let Some(confirmed) = modal.tick(window, &mut presentation.renderer, menu, Some(&cursor))
    else {
        return QuickLoadConfirmationProgress::Pending;
    };

    let Some(QuickLoadConfirmationFlow::Awaiting {
        slot,
        mission_id,
        save,
        ..
    }) = flow.take()
    else {
        unreachable!("resolved QuickLoad flow disappeared")
    };
    callbacks.pending = confirmed.then_some(SaveLoadRequest::Load {
        slot: Some(slot),
        mission_id,
        save: Some(save),
    });
    QuickLoadConfirmationProgress::Resolved
}

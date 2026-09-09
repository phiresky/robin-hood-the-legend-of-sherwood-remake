//! Main-menu "Load Game" entry.
//!
//! Opens the shared save/load slot picker in load-only mode against the
//! main-menu `Renderer` + `IngameMenuResources`, reads the chosen slot's
//! cached mission id from the save index, and returns a
//! [`crate::main_menu::MainMenuChoice::Load`] so the caller can start a
//! session seeded with a `SaveLoadRequest::Load`.

use crate::host::ApplicationContext;
use crate::ingame_menu::widget_bridge::ModalCursor;
use crate::ingame_menu::{IngameMenuResources, SaveLoadOutcome, show_load_picker};
use crate::main_menu::MainMenuChoice;
use crate::renderer::Renderer;
use crate::savegame::SaveGameManager;

/// Display the slot picker in Load mode.  Returns `Some(MainMenuChoice::Load)`
/// when the player picked a slot, `None` when they cancelled.
pub(crate) async fn run_main_menu_load(
    application_context: &ApplicationContext,
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: ModalCursor<'_>,
    save_manager: &mut SaveGameManager,
) -> Option<MainMenuChoice> {
    let detailed_metadata = application_context
        .active_profile_snapshot()
        .unwrap_or_else(|error| panic!("Load Game requires an active profile: {error}"))
        .gameplay_config
        .detailed_save_metadata;
    let outcome = show_load_picker(
        event_pump,
        renderer,
        resources,
        Some(cursor),
        save_manager,
        detailed_metadata,
    )
    .await;

    let slot = match outcome {
        SaveLoadOutcome::Slot(slot) => slot,
        SaveLoadOutcome::Cancel => return None,
    };

    match save_manager.slot_mission_id(slot) {
        Some(mission_id) => Some(MainMenuChoice::Load {
            slot: save_manager
                .slot_name(slot)
                .expect("selected save identity is valid"),
            mission_id,
        }),
        None => {
            tracing::error!("Load: selected slot {slot} has no cached mission id — cancelling");
            None
        }
    }
}

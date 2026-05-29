//! Main-menu "Load Game" entry.
//!
//! Opens the shared save/load slot picker in load-only mode against the
//! main-menu `Renderer` + `IngameMenuResources`, reads the chosen slot's
//! header to find its mission id, and returns a
//! [`crate::main_menu::MainMenuChoice::Load`] so the caller can start a
//! session seeded with a `SaveLoadRequest::Load`.

use crate::ingame_menu::widget_bridge::ModalCursor;
use crate::ingame_menu::{IngameMenuResources, SaveLoadMode, SaveLoadOutcome, show_save_load};
use crate::main_menu::MainMenuChoice;
use crate::renderer::Renderer;
use crate::savegame::SaveGameManager;

/// Display the slot picker in Load mode.  Returns `Some(MainMenuChoice::Load)`
/// when the player picked a slot, `None` when they cancelled.
pub(crate) async fn run_main_menu_load(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: ModalCursor<'_>,
    save_manager: &mut SaveGameManager,
) -> Option<MainMenuChoice> {
    // `mission_id` is only written onto freshly-created Save slots; in
    // Load mode `show_save_load` ignores it.  Pass 0 — there's no active
    // mission at main-menu time.
    // Main-menu entry has no live `SoundManager` plumbed through, so the
    // widget noisy events are silent here. `None` on all three slots
    // short-circuits `play_widget_noise` / `WidgetInputField::play_noise`.
    let outcome = show_save_load(
        event_pump,
        renderer,
        resources,
        Some(cursor),
        save_manager,
        0,
        None,
        SaveLoadMode::Load,
        None,
        None,
        None,
    )
    .await;

    let slot = match outcome {
        SaveLoadOutcome::Slot(slot) => slot,
        SaveLoadOutcome::Cancel => return None,
    };

    // Read the header to determine which mission the session must be
    // set up for — the target mission is read from the save before the
    // engine is re-created.
    match save_manager.read_slot_header(slot) {
        Ok(header) => Some(MainMenuChoice::Load {
            slot,
            mission_id: header.mission_id,
        }),
        Err(err) => {
            tracing::error!(
                "Load: selected slot {slot} but failed to read header: {err:#} — cancelling"
            );
            None
        }
    }
}

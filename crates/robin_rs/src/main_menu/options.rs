//! Main-menu "Options" entry.
//!
//! Dispatches to the shared options dialog (`ingame_menu::show_options`)
//! using the active player profile's graphic + sound configs as the
//! backing store.  The same options window is shown regardless of whether
//! the game is in-session or at the main menu — the dialog always writes
//! back to the active player profile.

use crate::audio_backend::KiraAudioBackend;
use crate::host::ApplicationContext;
use crate::ingame_menu::widget_bridge::ModalCursor;
use crate::ingame_menu::{IngameMenuResources, show_options};
use crate::key_config_store::ProfileKeyConfig;
use crate::renderer::Renderer;
use crate::sound::SoundManager;
use robin_engine::engine as engine_api;

/// Show the options dialog over the main-menu background.
///
/// Edits the active profile's configs in place and persists the manager
/// so changes survive across runs.  Key bindings are routed through the
/// application-owned key-config store so the active and custom key-config slots
/// persist across sessions.
///
/// Spins up a short-lived [`KiraAudioBackend`] + [`SoundManager`] +
/// sample loader for the duration of the dialog so the Sounds
/// sub-screen's volume sliders fire their slider-tick noises the same
/// way the in-game Options dialog does. The audio lives only while the
/// Options modal is open — `run_session` creates its own backend when
/// a mission starts, so there's no conflict.
pub(crate) async fn show_main_menu_options(
    application_context: &ApplicationContext,
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor_renderer: &mut crate::cursor::CursorRenderer,
) -> bool {
    let (active_profile_id, mut graphic, mut gameplay, mut multiplayer, mut sound_cfg) =
        application_context
            .with_active_profile(|profile| {
                (
                    profile.id,
                    profile.graphic_config.clone(),
                    profile.gameplay_config,
                    profile.multiplayer_config,
                    profile.sound_config,
                )
            })
            .unwrap_or_else(|error| {
                panic!("Main menu Options requires an active profile: {error}")
            });
    let (active, custom) = application_context
        .active_key_configs()
        .unwrap_or_else(|error| panic!("Main menu Options requires active key configs: {error}"));
    let mut key_cfg = ProfileKeyConfig { active, custom };

    // Short-lived audio setup so slider ticks play at the main menu.
    // Audio is optional, but missing authority/device failures remain visible.
    let sound_dir = std::path::PathBuf::from(&application_context.options().sound_directory);
    let mut sound_mgr = SoundManager::default();
    let sample_loader = if application_context.options().sound_enabled {
        match super::prepare_menu_sound(application_context, &mut sound_mgr) {
            Ok(loader) => Some(loader),
            Err(error) => {
                tracing::warn!("Main-menu Options audio disabled: {error}");
                None
            }
        }
    } else {
        None
    };
    let mut audio_backend = if sample_loader.is_some() {
        match KiraAudioBackend::new_for_application(
            application_context,
            &sound_dir,
            crate::sound::NUM_CHANNELS,
        ) {
            Ok(backend) => Some(backend),
            Err(error) => {
                tracing::warn!("Main-menu Options audio device unavailable: {error}");
                None
            }
        }
    } else {
        None
    };
    if let Some(ref mut backend) = audio_backend
        && let Err(e) = sound_mgr.initialize(backend, sound_cfg.sound_3d)
    {
        tracing::warn!("Main-menu Options: SoundManager init failed: {e}");
        audio_backend = None;
    }

    // Reborrow helper: turn `Option<&mut KiraAudioBackend>` into the
    // trait object form that `show_options` expects.  See the note in
    // `ingame_menu::sounds::show_sounds` — `Option<&mut dyn Trait>`
    // can't be shortened with `as_deref_mut` across the call boundary,
    // so we do the `&mut **b as &mut dyn _` dance instead.
    let backend_opt: Option<&mut dyn crate::sound::AudioBackend> = audio_backend
        .as_mut()
        .map(|b| b as &mut dyn crate::sound::AudioBackend);

    let outcome = show_options(
        application_context,
        true,
        event_pump,
        renderer,
        resources,
        Some(ModalCursor::new(
            cursor_renderer,
            engine_api::input::MOUSE_OPACITY_DEFAULT,
            0,
        )),
        &mut graphic,
        &mut gameplay,
        &mut multiplayer,
        &mut sound_cfg,
        &mut key_cfg.active,
        &mut key_cfg.custom,
        true,
        Some(&mut sound_mgr),
        backend_opt,
        sample_loader.as_deref(),
    )
    .await;
    if outcome.resolution_changed {
        event_pump.set_logical_resolution_policy(&graphic);
        renderer.sync_window_size(event_pump);
    }
    renderer.apply_upscale_config(&graphic);
    event_pump.set_native_refresh_presentation(graphic.native_refresh_presentation);
    renderer.configure_native_refresh_presentation(
        graphic.native_refresh_presentation,
        event_pump.surface_config.width,
        event_pump.surface_config.height,
    );

    if outcome.changed {
        application_context
            .update_and_retain_player_profiles(|mgr| {
                let profile = mgr
                    .profiles
                    .iter_mut()
                    .find(|profile| profile.id == active_profile_id)
                    .expect("active profile disappeared while Options was open");
                profile.graphic_config = graphic;
                profile.gameplay_config = gameplay;
                profile.multiplayer_config = multiplayer;
                profile.sound_config = sound_cfg;
            })
            .unwrap_or_else(|error| panic!("Main menu Options profile update failed: {error}"))
            .log_persistence_error("Main menu Options: failed to save profile manager");
    }
    if outcome.key_config_changed {
        application_context
            .with_key_configs_mut(|store| {
                *store.entry_or_default(active_profile_id) = key_cfg;
                if let Err(err) = store.save() {
                    tracing::error!("Main menu Options: failed to save key configs: {err:#}");
                }
            })
            .unwrap_or_else(|error| panic!("Main menu Options key update failed: {error}"));
    }
    // `audio_backend` drops here: KiraAudioBackend::drop stops playback and
    // releases its audio resources, so the next session can re-initialize.
    outcome.language_changed
}

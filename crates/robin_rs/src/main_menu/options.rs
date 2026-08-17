//! Main-menu "Options" entry.
//!
//! Dispatches to the shared options dialog (`ingame_menu::show_options`)
//! using the active player profile's graphic + sound configs as the
//! backing store.  The same options window is shown regardless of whether
//! the game is in-session or at the main menu — the dialog always writes
//! back to the active player profile.

use crate::audio_backend::{self, KiraAudioBackend};
use crate::host::ApplicationContext;
use crate::ingame_menu::widget_bridge::ModalCursor;
use crate::ingame_menu::{IngameMenuResources, show_options};
use crate::key_config_store::ProfileKeyConfig;
use crate::renderer::Renderer;
use crate::sound::SoundManager;
use robin_engine::engine as engine_api;
use robin_engine::graphic_config::GraphicConfig;

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
) {
    let profile = application_context
        .active_profile_snapshot()
        .unwrap_or_else(|error| panic!("Main menu Options requires an active profile: {error}"));
    let active_profile_id = profile.id;
    let mut graphic = profile.graphic_config;
    let mut gameplay = profile.gameplay_config;
    let mut sound_cfg = profile.sound_config;
    let (active, custom) = application_context
        .active_key_configs()
        .unwrap_or_else(|error| panic!("Main menu Options requires active key configs: {error}"));
    let mut key_cfg = ProfileKeyConfig { active, custom };

    // Short-lived audio setup so slider ticks play at the main menu.
    // Falls back silently (`None`) on any failure — the menu still
    // works without sound, matching what happens when the system has
    // no audio device.
    //
    let sound_dir = std::path::PathBuf::from(&application_context.options().sound_directory);
    let mut audio_backend = KiraAudioBackend::new(&sound_dir, crate::sound::NUM_CHANNELS).ok();
    let mut sound_mgr = SoundManager::default();
    if let Some(ref mut backend) = audio_backend
        && let Err(e) = sound_mgr.initialize(backend, sound_cfg.sound_3d)
    {
        tracing::warn!("Main-menu Options: SoundManager init failed: {e}");
    }
    // Load the menu sound bank so the slider's `(noisy_id << 16) +
    // event_id` lookup actually finds entries.  Same path + parse as
    // `game_session::run_session`.
    {
        let menu_bank_path = robin_engine::sbfile::resolve_case_insensitive(std::path::Path::new(
            "Data/Sounds/Menu/menu.fxg",
        ))
        .unwrap_or_else(|| std::path::PathBuf::from("Data/Sounds/Menu/menu.fxg"));
        match std::fs::read(&menu_bank_path) {
            Ok(data) => match robin_engine::sound_cache::parse_menu_bank(&data) {
                Ok(entries) => {
                    sound_mgr.sound_cache.initialize_menu_cache(&entries);
                }
                Err(e) => tracing::warn!("Main-menu Options: menu bank parse failed: {e}"),
            },
            Err(e) => tracing::warn!(
                "Main-menu Options: menu bank unreadable at {}: {e}",
                menu_bank_path.display()
            ),
        }
    }
    let sample_loader = audio_backend::create_sample_loader(sound_dir);

    // Reborrow helper: turn `Option<&mut KiraAudioBackend>` into the
    // trait object form that `show_options` expects.  See the note in
    // `ingame_menu::sounds::show_sounds` — `Option<&mut dyn Trait>`
    // can't be shortened with `as_deref_mut` across the call boundary,
    // so we do the `&mut **b as &mut dyn _` dance instead.
    let backend_opt: Option<&mut dyn crate::sound::AudioBackend> = audio_backend
        .as_mut()
        .map(|b| b as &mut dyn crate::sound::AudioBackend);

    let outcome = show_options(
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
        &mut sound_cfg,
        &mut key_cfg.active,
        &mut key_cfg.custom,
        Some(&mut sound_mgr),
        backend_opt,
        Some(&*sample_loader),
    )
    .await;
    if outcome.resolution_changed {
        apply_resolution_to_renderer(renderer, &graphic);
        event_pump.set_logical_size(
            renderer.screen_width() as u32,
            renderer.screen_height() as u32,
        );
    }
    renderer.set_shader_preset(graphic.shader_preset.clone());

    if outcome.changed {
        application_context
            .with_player_profiles_mut(|mgr| {
                let profile = mgr
                    .profiles
                    .iter_mut()
                    .find(|profile| profile.id == active_profile_id)
                    .expect("active profile disappeared while Options was open");
                profile.graphic_config = graphic;
                profile.gameplay_config = gameplay;
                profile.sound_config = sound_cfg;
                if let Err(err) = mgr.save() {
                    tracing::error!("Main menu Options: failed to save profile manager: {err:#}");
                }
            })
            .unwrap_or_else(|error| panic!("Main menu Options profile update failed: {error}"));
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
}

/// Apply a resolution change coming out of the Graphics sub-menu to the
/// renderer only.  No engine resize here because the main menu has no
/// active engine; game_session runs the full resize pipeline when it's
/// in control.
fn apply_resolution_to_renderer(renderer: &mut Renderer, config: &GraphicConfig) {
    let new_w = config.resolution_x.round() as u16;
    let new_h = config.resolution_y.round() as u16;
    renderer.resize(new_w, new_h);
}

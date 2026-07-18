//! Main-menu "Options" entry.
//!
//! Dispatches to the shared options dialog (`ingame_menu::show_options`)
//! using the active player profile's graphic + sound configs as the
//! backing store.  The same options window is shown regardless of whether
//! the game is in-session or at the main menu — the dialog always writes
//! back to the active player profile.

use crate::audio_backend::{self, KiraAudioBackend};
use crate::graphic_config::GraphicConfig;
use crate::ingame_menu::widget_bridge::ModalCursor;
use crate::ingame_menu::{IngameMenuResources, show_options};
use crate::key_config_store::{KeyConfigStore, ProfileKeyConfig};
use crate::player_profile::PlayerProfileManager;
use crate::renderer::Renderer;
use crate::sound::SoundManager;
use crate::sound_config::SoundConfig;
use robin_engine::engine as engine_api;

/// Show the options dialog over the main-menu background.
///
/// Edits the active profile's configs in place and persists the manager
/// so changes survive across runs.  Key bindings are routed through the
/// global [`KeyConfigStore`] so the active and custom key-config slots
/// persist across sessions.
///
/// Spins up a short-lived [`KiraAudioBackend`] + [`SoundManager`] +
/// sample loader for the duration of the dialog so the Sounds
/// sub-screen's volume sliders fire their slider-tick noises the same
/// way the in-game Options dialog does. The audio lives only while the
/// Options modal is open — `run_session` creates its own backend when
/// a mission starts, so there's no conflict.
pub(crate) async fn show_main_menu_options(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor_renderer: &mut crate::cursor::CursorRenderer,
) {
    let (active_profile_id, mut graphic, mut sound_cfg, mut key_cfg) = {
        let profile_guard = PlayerProfileManager::global();
        let store_guard = KeyConfigStore::global();
        match (
            profile_guard.as_ref().and_then(|mgr| mgr.get_active()),
            store_guard.as_ref(),
        ) {
            (Some(profile), Some(store)) => {
                let key_cfg = store
                    .get(profile.id)
                    .cloned()
                    .unwrap_or_else(ProfileKeyConfig::fresh);
                (
                    Some(profile.id),
                    profile.graphic_config.clone(),
                    profile.sound_config,
                    key_cfg,
                )
            }
            _ => {
                tracing::warn!(
                    "Main menu Options: missing active profile or key-config store — editing temporary configs only"
                );
                (
                    None,
                    GraphicConfig::default(),
                    SoundConfig::default(),
                    ProfileKeyConfig::fresh(),
                )
            }
        }
    };

    // Short-lived audio setup so slider ticks play at the main menu.
    // Falls back silently (`None`) on any failure — the menu still
    // works without sound, matching what happens when the system has
    // no audio device.
    //
    // Sound/music directory defaults (`Data/Sounds` / `Data/Musics`)
    // match `engine::GlobalOptions::default()`.  The main menu runs
    // before we have a `Game` instance that could carry user-tweaked
    // paths through `-SOUNDDIR` / `-MUSICDIR` flags, so this is best
    // effort — command-line overrides only affect session-time audio.
    let sound_dir = std::path::PathBuf::from("Data/Sounds");
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
        let menu_bank_path = crate::sbfile::resolve_case_insensitive(std::path::Path::new(
            "Data/Sounds/Menu/menu.fxg",
        ))
        .unwrap_or_else(|| std::path::PathBuf::from("Data/Sounds/Menu/menu.fxg"));
        match std::fs::read(&menu_bank_path) {
            Ok(data) => match crate::sound_cache::parse_menu_bank(&data) {
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

    if let Some(profile_id) = active_profile_id {
        if outcome.changed {
            let mut profile_guard = PlayerProfileManager::global();
            if let Some(mgr) = profile_guard.as_mut()
                && let Some(profile) = mgr.profiles.iter_mut().find(|p| p.id == profile_id)
            {
                profile.graphic_config = graphic;
                profile.sound_config = sound_cfg;
                if let Err(err) = mgr.save() {
                    tracing::error!("Main menu Options: failed to save profile manager: {err:#}");
                }
            }
        }
        if outcome.key_config_changed {
            let mut store_guard = KeyConfigStore::global();
            if let Some(store) = store_guard.as_mut() {
                *store.entry_or_default(profile_id) = key_cfg;
                if let Err(err) = store.save() {
                    tracing::error!("Main menu Options: failed to save key configs: {err:#}");
                }
            }
        }
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

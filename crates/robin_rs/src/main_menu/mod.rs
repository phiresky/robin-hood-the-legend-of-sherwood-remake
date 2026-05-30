//! Graphical main menu screen.
//!
//! Like every other menu in the game, the main menu is built from the
//! shared [`IngameMenuResources`] + [`FrameWnd`] + widget infrastructure
//! — the button column on the bottom-right is laid out by
//! [`align_bottom_right`](crate::ingame_menu::layout::align_bottom_right)
//! and the left-side profile info block renders the active
//! [`PlayerProfile`]'s stats via [`render_text_virt`].

use crate::gfx_types::Keycode;
use robin_assets::shipping_datadir as assets_shipping_datadir;
use robin_engine::engine::input::MOUSE_OPACITY_DEFAULT;
use robin_engine::graphic_config::TextureScaleMode;
use robin_engine::profiles as engine_profiles;
use robin_engine::sound_cache::SampleLoader;
use robin_engine::sprite::BBox;

use crate::campaign::Campaign;
use crate::cursor::CursorRenderer;
use crate::geo2d;
use crate::gfx_types::GameEvent;
use crate::ingame_menu::IngameMenuResources;
use crate::ingame_menu::layout::{
    MENU_H, MENU_W, MenuTransform, align_bottom_right, button_sprite_state,
};
use crate::ingame_menu::resources::{
    MT_BTN_LOAD, MT_BTN_OPTIONS, MT_BTN_QUIT_GAME, MT_BTN_SELECT_PLAYER, MT_BTN_SHOW_CREDITS,
    MT_BTN_SHOW_MOVIES, MT_BTN_START_GAME, MT_MSG_RETURN_TO_WINDOWS, MT_STR_CARNAGE_FACTOR,
    MT_STR_DIFFICULTY_EASY, MT_STR_DIFFICULTY_HARD, MT_STR_DIFFICULTY_LEVEL,
    MT_STR_DIFFICULTY_MEDIUM, MT_STR_MONEY, MT_STR_PLAYING_TIME, MT_STR_PROGRESSION, MT_STR_SCORE,
};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::ingame_menu::yesno::show_yesno;
use crate::renderer::BLIT_SOURCE_TRANSPARENT;
use crate::renderer::Renderer;
use crate::resource_ids;
use crate::savegame::SaveGameManager;
use crate::sdl_audio::{self, SdlMixerBackend};
use crate::sound::SoundManager;
use crate::sound_config::SoundConfig;
use crate::ui::UiState;
use crate::widget::FrameWnd;
use crate::window::GameWindow;
use robin_engine::player_profile::{DifficultyLevel, PlayerProfile, PlayerProfileManager};

pub(crate) mod credits;
pub(crate) mod custom_missions;
pub(crate) mod movies;
pub(crate) mod multiplayer_lobby;
pub(crate) mod options;
pub(crate) mod player_select;
pub(crate) mod save_load;

/// What the player chose from the main menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MainMenuChoice {
    Start,
    Multiplayer(multiplayer_lobby::MultiplayerLaunch),
    /// Player chose a save slot to load — the caller should start a
    /// session seeded with a `SaveLoadRequest::Load` for that slot.
    Load {
        slot: usize,
        mission_id: u32,
    },
    /// Player picked a custom mission to launch.  The caller mounts the
    /// zip via `mod_pack::mount_for_launch`, then drives a session via
    /// `Campaign::force_next_mission_by_name`, then returns *here* so
    /// the picker can be reopened for another launch.
    CustomMission(custom_missions::CustomMissionLaunch),
    Exit,
}

/// Action associated with a main-menu button click.
#[derive(Debug, Clone)]
enum ClickAction {
    /// Exit the main-menu loop with this choice.
    Return(MainMenuChoice),
    /// Open the save/load picker in Load mode; on slot selection, return
    /// [`MainMenuChoice::Load`].
    LoadGame,
    /// Connect to the configured lobby server and select/create a game.
    Multiplayer,
    /// Open the player-profile selector in place.  Mutates the global
    /// [`robin_engine::player_profile::PlayerProfileManager`].
    SelectPlayer,
    /// Open the options dialog (graphics / sounds / shortcuts) in place.
    Options,
    /// Open the Show Movies sub-screen (Intro / Outro playback) in place.
    ShowMovies,
    /// Scroll the credits bitmap in place until the player dismisses.
    ShowCredits,
    /// Open the custom-mission picker; on selection, return
    /// [`MainMenuChoice::CustomMission`].
    CustomMissions,
}

// Button widget IDs — order matches the bottom-right widget list.
const ID_START: u32 = 0;
const ID_MULTIPLAYER: u32 = 1;
const ID_LOAD: u32 = 2;
const ID_CUSTOM_MISSIONS: u32 = 3;
const ID_SELECT_PLAYER: u32 = 4;
const ID_OPTIONS: u32 = 5;
const ID_SHOW_MOVIES: u32 = 6;
const ID_SHOW_CREDITS: u32 = 7;
const ID_EXIT: u32 = 8;

/// Left-side profile info block position:
///
/// - profile name: `(0, 100)..(480, 480)`
/// - info block:   `(0, 125)..(480, 480)`
///
/// Each line is centred horizontally inside the box, so every line of
/// the info block is individually centred within x = 0..480.
const PROFILE_NAME_Y: i32 = 100;
const PROFILE_INFO_Y: i32 = 125;
const PROFILE_INFO_BOX_X: i32 = 0;
const PROFILE_INFO_BOX_W: i32 = 480;

struct MainMenuAudio {
    backend: SdlMixerBackend,
    sound: SoundManager,
    sample_loader: Box<SampleLoader>,
    noisy_tracker: widget_bridge::NoisyTracker,
}

impl MainMenuAudio {
    fn new() -> Option<Self> {
        // Main menu starts before a `Game` exists, so it cannot see
        // command-line sound-directory overrides yet. Keep this in
        // lockstep with the existing main-menu options bootstrap.
        let sound_dir = std::path::PathBuf::from("Data/Sounds");
        let mut backend = match SdlMixerBackend::new(&sound_dir, crate::sound::NUM_CHANNELS) {
            Ok(backend) => backend,
            Err(e) => {
                tracing::warn!("Main menu: failed to initialize audio: {e}");
                return None;
            }
        };
        let mut sound = SoundManager::default();
        let sound_cfg = SoundConfig::default();
        if let Err(e) = sound.initialize(&mut backend, sound_cfg.sound_3d) {
            tracing::warn!("Main menu: SoundManager init failed: {e}");
            return None;
        }
        sound.apply_volumes(&sound_cfg);

        let menu_bank_path = crate::sbfile::resolve_case_insensitive(std::path::Path::new(
            "Data/Sounds/Menu/menu.fxg",
        ))
        .unwrap_or_else(|| std::path::PathBuf::from("Data/Sounds/Menu/menu.fxg"));
        match std::fs::read(&menu_bank_path) {
            Ok(data) => match crate::sound_cache::parse_menu_bank(&data) {
                Ok(entries) => sound.sound_cache.initialize_menu_cache(&entries),
                Err(e) => tracing::warn!("Main menu: menu bank parse failed: {e}"),
            },
            Err(e) => tracing::warn!(
                "Main menu: menu bank unreadable at {}: {e}",
                menu_bank_path.display()
            ),
        }

        Some(Self {
            backend,
            sound,
            sample_loader: sdl_audio::create_sample_loader(sound_dir),
            noisy_tracker: widget_bridge::NoisyTracker::new(),
        })
    }

    fn play_button_noise(&mut self, events: &[crate::ui::UiEvent], frame: &FrameWnd) {
        widget_bridge::play_frame_widget_noise(
            events,
            frame,
            widget_bridge::WIDGET_NOISY_BUTTON,
            &mut self.sound,
            Some(&mut self.backend),
            &*self.sample_loader,
            &mut self.noisy_tracker,
        );
    }
}

/// Display the graphical main menu matching the original game.
///
/// Loads the shared menu resources (button sprites + fonts + menu text
/// table) via [`IngameMenuResources`], puts the button column flush to
/// the bottom-right of the virtual 640x480 window, and renders the
/// active [`PlayerProfile`]'s info on the left.
pub(crate) async fn show_main_menu(
    window: &mut GameWindow,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
) -> Result<MainMenuChoice, String> {
    window.set_logical_size(MENU_W as u32, MENU_H as u32);

    // Main menu runs before a profile is loaded, so fall back to the
    // default texture scale mode.  Once the player selects a profile or
    // opens options mid-game, the active profile's `graphic_config.scale_mode`
    // is applied via [`Renderer::set_scale_mode`].
    let mut renderer = Renderer::new(
        window,
        MENU_W as u16,
        MENU_H as u16,
        TextureScaleMode::default(),
    );

    // Shared menu resources (buttons, fonts, menu text table) — reused
    // by every sub-menu launched from here.
    let Some(mut menu_resources) = IngameMenuResources::new(&mut renderer, shipping) else {
        return Err("Main menu: Data/Interface/DEFAULT.RES unavailable".into());
    };

    // Background image — RHID_MENU_BACKGROUND_1, loaded into
    // `menu_bg[1]` by `IngameMenuResources::new`.
    let bg = menu_resources.menu_bg[1];
    if bg.is_none() {
        tracing::warn!(
            "Main menu: RHID_MENU_BACKGROUND_1 missing from DEFAULT.RES — rendering with no background"
        );
    }
    let mut menu_audio = MainMenuAudio::new();

    // Local save manager for browsing/loading at the main menu.  The
    // session layer builds its own in `RustCallbacks::new`; both read
    // the same on-disk save directory so the slot indices match.
    let mut save_manager = SaveGameManager::open_default();

    // Cursor — hide the OS cursor and render the in-game arrow sprite
    // (the default cursor is set at start-up, before the menu comes up).
    // Reuses the DEFAULT.RES already opened by `IngameMenuResources`.
    let mut cursor_renderer = CursorRenderer::new();
    cursor_renderer.init(&mut renderer);
    if !cursor_renderer.load_cursor(
        resource_ids::RHMOUSE_DEFAULT,
        &mut menu_resources.res,
        &mut renderer,
    ) {
        tracing::warn!("Main menu: failed to load RHMOUSE_DEFAULT cursor — using fallback arrow");
    }

    // ── Button layout (align_bottom_right, spacing=2) ────────────────
    let (btn_w, btn_h) = menu_resources.button_dimensions();

    let buttons: [(u32, String, ClickAction); 9] = [
        (
            ID_START,
            menu_resources.menu_text.get(MT_BTN_START_GAME),
            ClickAction::Return(MainMenuChoice::Start),
        ),
        (
            ID_MULTIPLAYER,
            "Multiplayer".to_string(),
            ClickAction::Multiplayer,
        ),
        (
            ID_LOAD,
            menu_resources.menu_text.get(MT_BTN_LOAD),
            ClickAction::LoadGame,
        ),
        (
            ID_CUSTOM_MISSIONS,
            "Custom Missions".to_string(),
            ClickAction::CustomMissions,
        ),
        (
            ID_SELECT_PLAYER,
            menu_resources.menu_text.get(MT_BTN_SELECT_PLAYER),
            ClickAction::SelectPlayer,
        ),
        (
            ID_OPTIONS,
            menu_resources.menu_text.get(MT_BTN_OPTIONS),
            ClickAction::Options,
        ),
        (
            ID_SHOW_MOVIES,
            menu_resources.menu_text.get(MT_BTN_SHOW_MOVIES),
            ClickAction::ShowMovies,
        ),
        (
            ID_SHOW_CREDITS,
            menu_resources.menu_text.get(MT_BTN_SHOW_CREDITS),
            ClickAction::ShowCredits,
        ),
        (
            ID_EXIT,
            menu_resources.menu_text.get(MT_BTN_QUIT_GAME),
            ClickAction::Return(MainMenuChoice::Exit),
        ),
    ];

    let labels: Vec<(&str, bool)> = buttons
        .iter()
        .map(|(_, label, _)| (label.as_str(), true))
        .collect();
    let positions = align_bottom_right(&labels, btn_w, btn_h);

    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    for (i, mb) in positions.iter().enumerate() {
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            buttons[i].0,
            &mb.label,
            mb.enabled,
            mb.x,
            mb.y,
            mb.w,
            mb.h,
        ));
    }

    // ── First-launch default-profile prompt ──────────────────────────
    //
    // When the profile manager was initialised from an empty
    // profiles.json and auto-created a placeholder (`default_profiles`
    // flag set), show the new-player prompt so the user can pick a name
    // + difficulty, then delete profile 0 (the placeholder) and save
    // the manager.  The flag is cleared unconditionally (even if the
    // user cancels the dialog) so the prompt never repeats.
    if let Some(bg) = bg {
        renderer.begin_gpu_frame_clear();
        let bg_x = (MENU_W - bg.width) / 2;
        let bg_y = (MENU_H - bg.height) / 2;
        let src = BBox::new(
            geo2d::pt(0.0, 0.0),
            geo2d::pt(bg.width as f32, bg.height as f32),
        );
        let dst = BBox::new(
            geo2d::pt(bg_x as f32, bg_y as f32),
            geo2d::pt((bg_x + bg.width) as f32, (bg_y + bg.height) as f32),
        );
        renderer.blit_to_screen(bg.id, Some(&src), Some(&dst), 0);
        renderer.present();
    }
    prompt_first_launch_new_player(
        &mut *window,
        &mut renderer,
        &menu_resources,
        &mut cursor_renderer,
    )
    .await;

    // ── Event-loop state ─────────────────────────────────────────────
    let mut input_state = ModalInputState::new();
    let mut keyboard_selection: u32 = ID_START;

    loop {
        // Recomputed each frame so a resolution change from the Options
        // / Select Player sub-menus re-centres the virtual 640x480 menu
        // on the new physical surface without an explicit "redisplay"
        // round-trip.
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );

        // ── Events ──────────────────────────────────────────────
        let mut activated: Option<u32> = None;
        let mut exit_requested = false;
        for event in window.poll_events() {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => exit_requested = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => exit_requested = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => move_keyboard_selection(&frame, &mut keyboard_selection, -1),
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => move_keyboard_selection(&frame, &mut keyboard_selection, 1),
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::Space,
                    ..
                } => {
                    activated = Some(keyboard_selection);
                }
                _ => {}
            }
        }

        let widget_input = input_state.as_widget_input();
        let events = frame.process_input(&widget_input);
        input_state.end_frame();
        if let Some(audio) = menu_audio.as_mut() {
            audio.play_button_noise(&events, &frame);
        }

        // Sync keyboard focus with the mouse-hovered widget so keyboard
        // + mouse don't fight each other.
        for w in frame.widgets() {
            if w.base().state != UiState::Default && w.base().enabled {
                keyboard_selection = w.id();
            }
        }

        if let Some(id) = widget_bridge::find_activated(&events) {
            activated = Some(id);
        }

        // ── Dispatch ────────────────────────────────────────────
        if let Some(id) = activated {
            let action = buttons[id as usize].2.clone();
            // Clicking Exit goes through the same confirmation path as
            // Escape / window-close below: show the
            // `MT_MSG_RETURN_TO_WINDOWS` yes/no before committing to
            // Exit and saving the profile manager.
            if matches!(action, ClickAction::Return(MainMenuChoice::Exit)) {
                exit_requested = true;
            } else if let Some(choice) = dispatch_click(
                action,
                &mut *window,
                &mut renderer,
                &mut menu_resources,
                &mut save_manager,
                &mut cursor_renderer,
                campaign,
                profiles,
                shipping,
            )
            .await
            {
                return Ok(choice);
            }
        }

        if exit_requested {
            let msg = menu_resources.menu_text.get(MT_MSG_RETURN_TO_WINDOWS);
            if show_yesno(&mut *window, &mut renderer, &menu_resources, None, &msg).await {
                // Persist the profile manager right before closing so
                // unsaved profile-level changes (active selection,
                // renames, etc.) survive the exit.
                let guard = PlayerProfileManager::global();
                if let Some(mgr) = guard.as_ref()
                    && let Err(err) = mgr.save()
                {
                    tracing::error!("Main menu Exit: failed to save profile manager: {err:#}");
                }
                return Ok(MainMenuChoice::Exit);
            }
            // Cancelled — stay in the menu and redraw next frame.
        }

        // ── Render ──────────────────────────────────────────────
        //
        // Background, button sprites, text, and cursor all draw through the
        // GPU queue; no menu frame mutates a retained software surface.

        renderer.begin_gpu_frame_clear();

        if let Some(bg) = bg {
            let bg_x = (MENU_W - bg.width) / 2;
            let bg_y = (MENU_H - bg.height) / 2;
            let src = BBox::new(
                geo2d::pt(0.0, 0.0),
                geo2d::pt(bg.width as f32, bg.height as f32),
            );
            let dst = BBox::new(
                geo2d::pt(bg_x as f32, bg_y as f32),
                geo2d::pt((bg_x + bg.width) as f32, (bg_y + bg.height) as f32),
            );
            renderer.blit_to_screen(bg.id, Some(&src), Some(&dst), 0);
        }

        // Buttons (sprite layer).
        for widget in frame.widgets() {
            let base = widget.base();
            let enabled = base.enabled;
            let hovered = matches!(base.state, UiState::Focused | UiState::Pushed)
                || (widget.id() == keyboard_selection && base.state == UiState::Default);
            let pressed = base.state == UiState::Pushed;
            let state_idx = button_sprite_state(enabled, hovered, pressed);
            let Some(rect) = base.bbox.0 else { continue };
            let bx = rect.min().x as i32;
            let by = rect.min().y as i32;
            let bw = (rect.max().x - rect.min().x) as i32;
            let bh = (rect.max().y - rect.min().y) as i32;
            if let Some(surf) = menu_resources.button_surface(state_idx) {
                let src = BBox::new(geo2d::pt(0.0, 0.0), geo2d::pt(bw as f32, bh as f32));
                let dst = BBox::new(
                    geo2d::pt(bx as f32, by as f32),
                    geo2d::pt((bx + bw) as f32, (by + bh) as f32),
                );
                renderer.blit_to_screen(surf, Some(&src), Some(&dst), BLIT_SOURCE_TRANSPARENT);
            }
        }

        // Text layer (profile info + button labels). Places button text
        // on the sprite at `(btn_h - font_height) / 2`, but emits atlas
        // glyph quads instead of drawing into a software surface.
        render_text_layer(&mut renderer, &menu_resources, &frame, keyboard_selection);

        // Custom cursor on top — the OS cursor is hidden, so skip this
        // and the mouse appears to vanish.
        cursor_renderer.advance_animation();
        ModalCursor::new(&mut cursor_renderer, MOUSE_OPACITY_DEFAULT, 0).draw(
            &mut renderer,
            transform,
            &input_state,
        );

        renderer.present();
        crate::window::sleep_ms(16).await;
    }
}

/// Default-profile prompt: runs before the event loop so the user picks
/// a name + difficulty on first launch — the manager starts with
/// `default_profiles = true` after [`PlayerProfileManager::load`]
/// auto-creates a placeholder "Robin".
///
/// Clears the flag unconditionally (even on cancel) so the prompt never
/// repeats.  On OK, delete the placeholder (profile 0) and install the
/// user's new profile.  On cancel, keep the placeholder as-is.
async fn prompt_first_launch_new_player(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor_renderer: &mut CursorRenderer,
) {
    let needs_prompt = {
        let guard = PlayerProfileManager::global();
        guard.as_ref().is_some_and(|mgr| mgr.default_profiles)
    };
    if !needs_prompt {
        return;
    }

    // Use the placeholder's name as the modal's initial value — the
    // autogenerated "Robin" default plays the role of the original's
    // anonymous fallback.
    let initial_name = {
        let guard = PlayerProfileManager::global();
        guard
            .as_ref()
            .and_then(|mgr| mgr.get_active())
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Robin".to_string())
    };

    let outcome = player_select::show_new_player_prompt(
        event_pump,
        renderer,
        resources,
        initial_name,
        Some(ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0)),
    )
    .await;

    let mut guard = PlayerProfileManager::global();
    let Some(mgr) = guard.as_mut() else {
        return;
    };
    // Clear the flag unconditionally so the prompt doesn't fire again
    // on the next menu entry even if the user cancelled.
    mgr.default_profiles = false;

    if let Some((name, difficulty)) = outcome {
        // Replace the auto-created placeholder (index 0) with the
        // user-provided profile: delete profile 0, add the new one,
        // promote it to active, and save.
        if !mgr.profiles.is_empty() {
            mgr.delete_profile(0);
        }
        // The window is already open at this point; pass its dimensions so
        // the new profile inherits live screen size rather than the 800×600
        // fallback (the "screen open" arm of profile creation).
        let screen_dims = Some((
            renderer.screen_width() as u32,
            renderer.screen_height() as u32,
        ));
        let idx = mgr.create_profile_with_screen_dims(name, difficulty, screen_dims);
        mgr.set_active(idx);
        // Save only inside the OK branch — on cancel the on-disk
        // `default_profiles = true` flag stays armed so the prompt
        // re-fires on the next launch.
        if let Err(err) = mgr.save() {
            tracing::error!("Main menu first-launch: failed to persist profile manager: {err:#}");
        }
    }
}

fn move_keyboard_selection(frame: &FrameWnd, selection: &mut u32, direction: i32) {
    let len = frame.widget_count() as i32;
    if len == 0 {
        return;
    }
    let mut idx = *selection as i32;
    for _ in 0..len {
        idx = (idx + direction).rem_euclid(len);
        if let Some(w) = frame.widget_at(idx as usize)
            && w.base().enabled
        {
            *selection = idx as u32;
            break;
        }
    }
}

/// Dispatch a button click to either an immediate return or an in-place
/// sub-menu.  Returns `Some` when the main menu should exit with that
/// choice; `None` when control should stay on the menu.
#[allow(clippy::too_many_arguments)]
async fn dispatch_click(
    action: ClickAction,
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    menu_resources: &mut IngameMenuResources,
    save_manager: &mut SaveGameManager,
    cursor_renderer: &mut CursorRenderer,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
) -> Option<MainMenuChoice> {
    match action {
        ClickAction::Return(c) => Some(c),
        ClickAction::LoadGame => {
            save_load::run_main_menu_load(
                event_pump,
                renderer,
                menu_resources,
                ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0),
                save_manager,
            )
            .await
        }
        ClickAction::Multiplayer => multiplayer_lobby::show_multiplayer_lobby(
            event_pump,
            renderer,
            menu_resources,
            cursor_renderer,
            campaign,
            profiles,
        )
        .await
        .map(MainMenuChoice::Multiplayer),
        ClickAction::SelectPlayer => {
            player_select::show_select_player(
                event_pump,
                renderer,
                menu_resources,
                Some(ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0)),
            )
            .await;
            // Active profile may have changed — reopen the save manager so
            // subsequent "Load Game" clicks read the new profile's
            // `Profile_NNN/saves.json` index rather than the prior one.
            *save_manager = SaveGameManager::open_default();
            // If the new active profile carries a different resolution,
            // resize so the surrounding menu re-lays out at the new size
            // on the next frame. `MenuTransform::centered` picks up the
            // new dimensions automatically.
            //
            // Sound-settings re-application is deliberately omitted here:
            // the main menu has no persistent `SoundManager` to apply to
            // (no menu music plays at this layer; the only main-menu
            // `SoundManager` is the transient one inside
            // `show_main_menu_options` for slider-tick noises, and it
            // gets torn down when Options exits). The new profile's
            // sound settings are picked up at the next session boot via
            // `game_session::init_audio_backend`, which reads the active
            // profile's `sound_config` when constructing the session-time
            // `SoundManager`. Hosting menu music at the main-menu level
            // would require a top-level main-menu `SoundManager` first;
            // that is a structural change beyond the scope of this arm.
            // See parity-audit/RHMenuIntro-03.md (`OnSelectPlayer` entry).
            let active_res = {
                let guard = PlayerProfileManager::global();
                guard.as_ref().and_then(|mgr| mgr.get_active()).map(|p| {
                    (
                        p.graphic_config.resolution_x.round() as u16,
                        p.graphic_config.resolution_y.round() as u16,
                    )
                })
            };
            if let Some((w, h)) = active_res
                && (renderer.screen_width() != w || renderer.screen_height() != h)
            {
                renderer.resize(w, h);
                event_pump.set_logical_size(w as u32, h as u32);
            }
            None
        }
        ClickAction::Options => {
            options::show_main_menu_options(event_pump, renderer, menu_resources, cursor_renderer)
                .await;
            event_pump.set_logical_size(
                renderer.screen_width() as u32,
                renderer.screen_height() as u32,
            );
            None
        }
        ClickAction::ShowCredits => {
            credits::show_credits(event_pump, renderer, shipping).await;
            None
        }
        ClickAction::ShowMovies => {
            movies::show_movies(event_pump, renderer, menu_resources).await;
            None
        }
        ClickAction::CustomMissions => {
            let mods_root = crate::mod_pack::default_mods_root();
            custom_missions::show_custom_missions(
                event_pump,
                renderer,
                menu_resources,
                ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0),
                &mods_root,
            )
            .await
            .map(MainMenuChoice::CustomMission)
        }
    }
}

/// Render every piece of text in the main menu (profile info block on
/// the left, button labels on the right).
fn render_text_layer(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    frame: &FrameWnd,
    keyboard_selection: u32,
) {
    // Build the text-block lines up-front so we don't hold references
    // into the global profile manager while the surface is locked.
    let profile_guard = PlayerProfileManager::global();
    let profile = profile_guard.as_ref().and_then(|mgr| mgr.get_active());
    let profile_name = profile.map(|p| p.name.clone());
    let profile_info_lines = profile
        .map(|p| build_profile_info_lines(resources, p))
        .unwrap_or_default();

    let name_font = resources
        .edit_field_font()
        .or_else(|| resources.title_font());
    let info_font = resources
        .menu_text_font()
        .or_else(|| resources.edit_field_font());
    let enabled_font = resources.menu_button_font(true);
    let disabled_font = resources.menu_button_font(false);

    // ── Profile info block (left side) ──────────────────────────────
    if let (Some(name), Some(font)) = (profile_name.as_deref(), name_font) {
        let tw = font.text_width(name);
        let x = PROFILE_INFO_BOX_X + (PROFILE_INFO_BOX_W - tw) / 2;
        renderer.render_text_argb(font, name, x, PROFILE_NAME_Y);
    }
    if let Some(font) = info_font {
        let line_h = font.height() as i32;
        for (i, line) in profile_info_lines.iter().enumerate() {
            let tw = font.text_width(line);
            let x = PROFILE_INFO_BOX_X + (PROFILE_INFO_BOX_W - tw) / 2;
            let y = PROFILE_INFO_Y + i as i32 * line_h;
            renderer.render_text_argb(font, line, x, y);
        }
    }

    // ── Button labels ───────────────────────────────────────────────
    for widget in frame.widgets() {
        let base = widget.base();
        let Some(rect) = base.bbox.0 else { continue };
        let bx = rect.min().x as i32;
        let by = rect.min().y as i32;
        let bw = (rect.max().x - rect.min().x) as i32;
        let bh = (rect.max().y - rect.min().y) as i32;

        let font = if base.enabled {
            enabled_font
        } else {
            disabled_font
        };
        let Some(font) = font else { continue };

        // Text box = exactly `font.height()` tall at
        // `(bh - font.height()) / 2` inside the button, text top = box
        // top.
        let tw = font.text_width(&base.text);
        let th = font.height() as i32;
        let tx = bx + (bw - tw) / 2;
        let ty = by + (bh - th) / 2;
        renderer.render_text_argb(font, &base.text, tx, ty);
        // Keyboard-selected widget keyboard-only: the hover sprite is
        // already handled by `button_sprite_state` in the sprite pass,
        // so no extra work here.
        let _ = keyboard_selection;
    }
}

fn build_profile_info_lines(
    resources: &IngameMenuResources,
    profile: &PlayerProfile,
) -> Vec<String> {
    let difficulty_label = resources.menu_text.get(MT_STR_DIFFICULTY_LEVEL);
    let difficulty_value = difficulty_to_string(resources, profile.difficulty);
    let money = substitute_i(
        &resources.menu_text.get(MT_STR_MONEY),
        profile.ransom as i64,
    );
    let score_label = resources.menu_text.get(MT_STR_SCORE);
    let spared_label = resources.menu_text.get(MT_STR_CARNAGE_FACTOR);
    let progress_label = resources.menu_text.get(MT_STR_PROGRESSION);
    let time_label = resources.menu_text.get(MT_STR_PLAYING_TIME);
    let time = seconds_to_time(profile.play_time);

    vec![
        format!("{difficulty_label} : {difficulty_value}"),
        money,
        format!("{score_label} : {}", profile.score),
        format!("{spared_label} : {} %", profile.preserved_lives),
        format!("{progress_label} : {} %", profile.progression),
        format!("{time_label} : {time}"),
    ]
}

/// Returns the localised difficulty label via the menu text table.
fn difficulty_to_string(resources: &IngameMenuResources, level: DifficultyLevel) -> String {
    let id = match level {
        DifficultyLevel::Easy => MT_STR_DIFFICULTY_EASY,
        DifficultyLevel::Medium => MT_STR_DIFFICULTY_MEDIUM,
        DifficultyLevel::Hard => MT_STR_DIFFICULTY_HARD,
    };
    resources.menu_text.get(id)
}

/// Format a duration as `HH:MM` with zero padding.
fn seconds_to_time(seconds: u32) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds - hours * 3600) / 60;
    format!("{hours:02}:{minutes:02}")
}

/// Substitute the first `%i` / `%d` placeholder in `template` with
/// `value`.  Used for printf-style format strings that ship in the
/// menu text table (e.g. `MT_STR_MONEY` → "Money: £%i").  If no
/// placeholder is present the template is returned verbatim — the
/// localised table is assumed trustworthy.
fn substitute_i(template: &str, value: i64) -> String {
    for marker in ["%i", "%d"] {
        if let Some(pos) = template.find(marker) {
            let mut out = String::with_capacity(template.len() + 8);
            out.push_str(&template[..pos]);
            out.push_str(&value.to_string());
            out.push_str(&template[pos + marker.len()..]);
            return out;
        }
    }
    template.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_to_time_zero() {
        assert_eq!(seconds_to_time(0), "00:00");
    }

    #[test]
    fn seconds_to_time_mixed() {
        // 1h 23m 45s => hours=1, minutes=23.
        assert_eq!(seconds_to_time(3600 + 23 * 60 + 45), "01:23");
    }

    #[test]
    fn substitute_i_basic() {
        assert_eq!(substitute_i("Money: £%i", 100), "Money: £100");
        assert_eq!(substitute_i("Ransom: %d", 42), "Ransom: 42");
    }

    #[test]
    fn substitute_i_no_placeholder() {
        assert_eq!(substitute_i("no format", 5), "no format");
    }
}

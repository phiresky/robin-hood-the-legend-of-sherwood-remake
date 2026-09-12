//! Graphical main menu screen.
//!
//! Like every other menu in the game, the main menu is built from the
//! shared [`IngameMenuResources`] + [`FrameWnd`] + widget infrastructure
//! — the button column on the bottom-right is laid out by
//! [`align_bottom_right`](crate::ingame_menu::layout::align_bottom_right)
//! and the left-side profile info block renders the active
//! [`PlayerProfile`]'s stats via [`render_text_virt`].

use crate::gfx_types::Keycode;
use robin_engine::engine::input::MOUSE_OPACITY_DEFAULT;
use robin_engine::profiles as engine_profiles;
use robin_engine::sound_cache::SampleLoader;
use robin_engine::sprite::BBox;

use crate::cursor::CursorRenderer;
use robin_engine::campaign::Campaign;

use crate::audio_backend::{self, KiraAudioBackend};
use crate::gfx_types::GameEvent;
use crate::host::ApplicationContext;
use crate::ingame_menu::IngameMenuResources;
use crate::ingame_menu::layout::{
    MENU_H, MENU_W, MenuTransform, align_bottom_right, button_sprite_state,
};
use crate::ingame_menu::resources::{
    MT_BTN_LOAD, MT_BTN_OPTIONS, MT_BTN_QUIT_GAME, MT_BTN_SELECT_PLAYER, MT_BTN_SHOW_CREDITS,
    MT_BTN_SHOW_MOVIES, MT_BTN_START_GAME, MT_MSG_RETURN_TO_WINDOWS, MT_PORT_STR_DIFFICULTY_CUSTOM,
    MT_PORT_STR_DIFFICULTY_LEGENDARY, MT_STR_CARNAGE_FACTOR, MT_STR_DIFFICULTY_EASY,
    MT_STR_DIFFICULTY_HARD, MT_STR_DIFFICULTY_LEVEL, MT_STR_DIFFICULTY_MEDIUM, MT_STR_MONEY,
    MT_STR_PLAYING_TIME, MT_STR_PROGRESSION, MT_STR_SCORE, substitute_integer,
};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::ingame_menu::yesno::show_yesno;
use crate::renderer::BLIT_SOURCE_TRANSPARENT;
use crate::renderer::Renderer;
use crate::savegame::SaveGameManager;
use crate::sound::SoundManager;
use crate::ui::UiState;
use crate::widget::FrameWnd;
use crate::window::GameWindow;
use robin_engine::player_profile::{DifficultyLevel, PlayerProfile};
use robin_engine::resource_ids;
use robin_engine::sound_config::SoundConfig;

pub(crate) mod credits;
pub(crate) mod custom_missions;
pub(crate) mod movies;
#[cfg(feature = "multiplayer")]
pub(crate) mod multiplayer_menu;
pub(crate) mod options;
pub(crate) mod player_select;
pub(crate) mod save_load;

/// What the player chose from the main menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MainMenuChoice {
    /// Reconstruct all eager menu labels/fonts after a host language change,
    /// then reopen Options at the same navigation depth.
    RedisplayOptions,
    Start,
    #[cfg(feature = "multiplayer")]
    Multiplayer(multiplayer_menu::MultiplayerLaunch),
    /// Player chose a save slot to load — the caller should start a
    /// session seeded with a `SaveLoadRequest::Load` for that slot.
    Load {
        slot: crate::savegame::SlotName,
        mission_id: u32,
    },
    /// Player picked a custom mission to launch.  For mod packs the
    /// caller mounts the zip via `mod_pack::mount_for_launch`; hackable
    /// levels launch directly from their always-mounted overlay.  Either
    /// way the session runs via `Campaign::force_next_mission_by_name`
    /// and control returns *here* so the picker can be reopened.
    CustomMission(custom_missions::CustomMissionChoice),
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
    /// Browse the selected player's campaign without starting a mission.
    CampaignManager,
    /// Open the serverless matchmaking browser and select/create a game.
    #[cfg(feature = "multiplayer")]
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

// Button widget IDs are the button's index in the bottom-right widget
// list; there are no fixed per-button constants.

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

/// Project one anchor from the main menu's virtual 640x480 window into the
/// active logical canvas. Keeping sprite and text projection on this shared
/// path makes their draw coordinates agree with `ModalInputState`, which
/// applies the inverse [`MenuTransform`] to pointer input.
fn main_menu_to_screen(transform: MenuTransform, x: i32, y: i32) -> (i32, i32) {
    transform.to_screen(x, y)
}

struct MainMenuAudio {
    backend: KiraAudioBackend,
    sound: SoundManager,
    sample_loader: Box<SampleLoader>,
    noisy_tracker: widget_bridge::NoisyTracker,
}

/// Prepare both menu audio paths from the application's owned resource reader.
/// Missing authority or an invalid bank is an error; callers may explicitly
/// disable optional menu audio after reporting it.
fn prepare_menu_sound(
    application_context: &ApplicationContext,
    sound: &mut SoundManager,
) -> Result<Box<SampleLoader>, String> {
    let files = application_context.preparation_files()?.clone();
    let shipping = application_context.shipping_arc()?;
    let path = "Data/Sounds/Menu/menu.fxg";
    let data = files
        .read_shared(path)
        .map_err(|error| format!("menu sound bank unreadable at {path}: {error}"))?;
    let entries = robin_engine::sound_cache::parse_menu_bank(&data)
        .map_err(|error| format!("menu sound bank parse failed: {error}"))?;
    sound.sound_cache_mut().initialize_menu_cache(&entries);
    Ok(audio_backend::create_sample_loader_with_files(
        std::path::PathBuf::from(&application_context.options().sound_directory),
        files,
        shipping,
    ))
}

impl MainMenuAudio {
    fn new(application_context: &ApplicationContext) -> Option<Self> {
        if !application_context.options().sound_enabled {
            return None;
        }
        let mut sound = SoundManager::default();
        let sample_loader = match prepare_menu_sound(application_context, &mut sound) {
            Ok(loader) => loader,
            Err(error) => {
                tracing::warn!("Main menu audio disabled: {error}");
                return None;
            }
        };
        let sound_dir = std::path::PathBuf::from(&application_context.options().sound_directory);
        let backend = KiraAudioBackend::new_for_application(
            application_context,
            &sound_dir,
            crate::sound::NUM_CHANNELS,
        );
        let mut backend = match backend {
            Ok(backend) => backend,
            Err(e) => {
                tracing::warn!("Main menu: failed to initialize audio: {e}");
                return None;
            }
        };
        let sound_cfg = SoundConfig::default();
        if let Err(e) = sound.initialize(&mut backend, sound_cfg.sound_3d) {
            tracing::warn!("Main menu: SoundManager init failed: {e}");
            return None;
        }
        sound.apply_volumes(&sound_cfg);

        Some(Self {
            backend,
            sound,
            sample_loader,
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
    application_context: &ApplicationContext,
    open_options_initially: bool,
    initial_direct_invite: Option<&str>,
) -> Result<MainMenuChoice, String> {
    #[cfg(not(feature = "multiplayer"))]
    if initial_direct_invite.is_some() {
        return Err("multiplayer invitation requires the multiplayer feature".to_owned());
    }
    let shipping = application_context.shipping()?;
    let initial_graphic = application_context
        .with_active_profile(|profile| profile.graphic_config.clone())
        .map_err(|error| format!("main menu requires an active profile: {error}"))?;
    window.set_logical_resolution_policy(&initial_graphic);
    let (logical_width, logical_height) = window.logical_size();
    let initial_native_refresh = initial_graphic.native_refresh_presentation;
    window.set_native_refresh_presentation(initial_native_refresh);
    let mut renderer = Renderer::new(
        window,
        logical_width as u16,
        logical_height as u16,
        initial_graphic.scale_mode,
    );
    renderer.apply_upscale_config(&initial_graphic);
    renderer.configure_native_refresh_presentation(
        initial_native_refresh,
        window.surface_config.width,
        window.surface_config.height,
    );

    // Shared menu resources (buttons, fonts, menu text table) — reused
    // by every sub-menu launched from here.
    let Some(mut menu_resources) = IngameMenuResources::new(
        &mut renderer,
        shipping,
        application_context.preparation_files()?.clone(),
    ) else {
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
    let mut menu_audio = MainMenuAudio::new(application_context);

    // Cursor — prepare the in-game arrow sprite (the window hides the OS cursor).
    // (the default cursor is set at start-up, before the menu comes up).
    // Reuses the DEFAULT.RES already opened by `IngameMenuResources`.
    let mut cursor_renderer = CursorRenderer::new();
    cursor_renderer.init(&renderer);
    if !cursor_renderer.load_cursor(
        resource_ids::RHMOUSE_DEFAULT,
        &mut menu_resources.res,
        &renderer,
    ) {
        tracing::warn!("Main menu: failed to load RHMOUSE_DEFAULT cursor — using fallback arrow");
    }

    // The stable browser shell scrubs `#join` before wasm boot and passes the
    // authenticated artifact directly into Rust. Consume that explicit
    // handoff here, where the multiplayer consent UI and exact-package
    // preflight have all rendering resources available. It must never fall
    // through to direct mission construction.
    #[cfg(feature = "multiplayer")]
    {
        if initial_direct_invite.is_some()
            && let Some(launch) = multiplayer_menu::show_multiplayer_menu(
                window,
                &mut renderer,
                &menu_resources,
                &mut cursor_renderer,
                campaign,
                profiles,
                application_context,
                initial_direct_invite,
            )
            .await
        {
            return Ok(MainMenuChoice::Multiplayer(launch));
        }
    }

    // ── Button layout (align_bottom_right, spacing=2) ────────────────
    let (btn_w, btn_h) = menu_resources.button_dimensions();

    let mut buttons: Vec<(String, ClickAction)> = vec![(
        menu_resources.menu_text.get(MT_BTN_START_GAME),
        ClickAction::Return(MainMenuChoice::Start),
    )];
    #[cfg(feature = "multiplayer")]
    buttons.push(("Multiplayer".to_string(), ClickAction::Multiplayer));
    buttons.extend([
        (
            menu_resources.menu_text.get(MT_BTN_LOAD),
            ClickAction::LoadGame,
        ),
        // TODO: Localize this label with the campaign history UI.
        ("Campaign Manager".to_string(), ClickAction::CampaignManager),
        ("Custom Missions".to_string(), ClickAction::CustomMissions),
    ]);
    buttons.extend([
        (
            menu_resources.menu_text.get(MT_BTN_SELECT_PLAYER),
            ClickAction::SelectPlayer,
        ),
        (
            menu_resources.menu_text.get(MT_BTN_OPTIONS),
            ClickAction::Options,
        ),
        (
            menu_resources.menu_text.get(MT_BTN_SHOW_MOVIES),
            ClickAction::ShowMovies,
        ),
        (
            menu_resources.menu_text.get(MT_BTN_SHOW_CREDITS),
            ClickAction::ShowCredits,
        ),
        (
            menu_resources.menu_text.get(MT_BTN_QUIT_GAME),
            ClickAction::Return(MainMenuChoice::Exit),
        ),
    ]);

    let labels: Vec<(&str, bool)> = buttons
        .iter()
        .map(|(label, _)| (label.as_str(), true))
        .collect();
    let positions = align_bottom_right(&labels, btn_w, btn_h);

    let mut frame = FrameWnd::interactive();
    for (i, mb) in positions.iter().enumerate() {
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            i as u32, &mb.label, mb.enabled, mb.x, mb.y, mb.w, mb.h,
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
        renderer.begin_ui_only_frame();
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let bg_x = transform.origin_x + (MENU_W - bg.width) / 2;
        let bg_y = transform.origin_y + (MENU_H - bg.height) / 2;
        let src = BBox::from_coords(0.0, 0.0, bg.width as f32, bg.height as f32);
        let dst = BBox::from_coords(
            bg_x as f32,
            bg_y as f32,
            (bg_x + bg.width) as f32,
            (bg_y + bg.height) as f32,
        );
        renderer
            .draw_surface(bg.id, Some(&src), Some(&dst), 0)
            .expect("live menu background");
        renderer.present();
    }
    prompt_first_launch_new_player(
        application_context,
        &mut *window,
        &mut renderer,
        &menu_resources,
        &mut cursor_renderer,
    )
    .await;

    // First launch may replace placeholder profile 0 with a newly allocated
    // profile id. Construct the save manager only after that atomic
    // profile/key-config transition so it can never retain Profile_000 as a
    // stale target. The session layer follows the same rule by constructing
    // callbacks only after the menu returns.
    let mut save_manager = match crate::save_recovery::open_with_recovery(
        application_context,
        window,
        &mut renderer,
        &menu_resources,
        Some(&ModalCursor::new(
            &mut cursor_renderer,
            MOUSE_OPACITY_DEFAULT,
            0,
        )),
    )
    .await
    {
        crate::save_recovery::OpenedSaveStore::Ready(manager) => manager,
        crate::save_recovery::OpenedSaveStore::Cancelled
        | crate::save_recovery::OpenedSaveStore::ExitRequested => return Ok(MainMenuChoice::Exit),
    };

    if open_options_initially
        && options::show_main_menu_options(
            application_context,
            window,
            &mut renderer,
            &menu_resources,
            &mut cursor_renderer,
        )
        .await
    {
        return Ok(MainMenuChoice::RedisplayOptions);
    }

    let mut state = MainMenuState::new(frame);
    loop {
        if let Some(choice) = state
            .tick(
                window,
                &mut renderer,
                &mut menu_resources,
                &mut save_manager,
                &mut cursor_renderer,
                campaign,
                profiles,
                application_context,
                bg,
                &buttons,
                &mut menu_audio,
            )
            .await?
        {
            return Ok(choice);
        }
        crate::window::sleep_ui_frame().await;
    }
}

/// Owns the live menu widget/input state; phase resources stay borrowed.
struct MainMenuState {
    frame: FrameWnd,
    input_state: ModalInputState,
    keyboard_selection: u32,
}

impl MainMenuState {
    fn new(frame: FrameWnd) -> Self {
        Self {
            frame,
            input_state: ModalInputState::new(),
            keyboard_selection: 0,
        }
    }

    fn process_events(
        &mut self,
        events: Vec<GameEvent>,
        transform: MenuTransform,
        menu_audio: &mut Option<MainMenuAudio>,
    ) -> (Option<u32>, bool) {
        // ── Events ──────────────────────────────────────────────
        let mut activated: Option<u32> = None;
        let mut exit_requested = false;
        for event in events {
            self.input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => exit_requested = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => exit_requested = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => move_keyboard_selection(&self.frame, &mut self.keyboard_selection, -1),
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => move_keyboard_selection(&self.frame, &mut self.keyboard_selection, 1),
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
                    activated = Some(self.keyboard_selection);
                }
                _ => {}
            }
        }

        let widget_input = self.input_state.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        self.input_state.end_frame();
        if let Some(audio) = menu_audio.as_mut() {
            audio.play_button_noise(&events, &self.frame);
        }

        // Sync keyboard focus with the mouse-hovered widget so keyboard
        // + mouse don't fight each other.
        for w in self.frame.widgets() {
            if w.base().state != UiState::Default && w.base().enabled {
                self.keyboard_selection = w.id();
            }
        }

        if let Some(id) = widget_bridge::find_activated(&events) {
            activated = Some(id);
        }

        (activated, exit_requested)
    }

    #[allow(clippy::too_many_arguments)]
    async fn tick(
        &mut self,
        window: &mut GameWindow,
        renderer: &mut Renderer,
        menu_resources: &mut IngameMenuResources,
        save_manager: &mut SaveGameManager,
        cursor_renderer: &mut CursorRenderer,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        application_context: &ApplicationContext,
        bg: Option<crate::ingame_menu::resources::MenuSurface>,
        buttons: &[(String, ClickAction)],
        menu_audio: &mut Option<MainMenuAudio>,
    ) -> Result<Option<MainMenuChoice>, String> {
        // Queued score verification is application work, not mission/UI work.
        // Keep it moving while the player remains at the main menu.
        application_context.poll_leaderboard_receipts();
        let events = window.poll_events();
        // A nested modal may have consumed the resize event; the window still
        // retains the latest policy-derived logical dimensions.
        renderer.sync_window_size(window);
        // Recomputed each frame so a resolution change from the Options
        // / Select Player sub-menus re-centres the virtual 640x480 menu
        // on the new physical surface without an explicit "redisplay"
        // round-trip.
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );

        let (activated, mut exit_requested) = self.process_events(events, transform, menu_audio);

        // ── Dispatch ────────────────────────────────────────────
        // OS close takes priority over a simultaneous Start/other activation:
        // dispatching first can reset the campaign or load another mission.
        if let Some(id) = activated.filter(|_| !window.close_requested) {
            let action = buttons[id as usize].1.clone();
            let browsing_campaign = matches!(action, ClickAction::CampaignManager);
            // Clicking Exit goes through the same confirmation path as
            // Escape below: show the
            // `MT_MSG_RETURN_TO_WINDOWS` yes/no before committing to
            // Exit and saving the profile manager.
            if matches!(action, ClickAction::Return(MainMenuChoice::Exit)) {
                exit_requested = true;
            } else if let Some(choice) = dispatch_click(
                action,
                &mut *window,
                renderer,
                menu_resources,
                save_manager,
                cursor_renderer,
                campaign,
                profiles,
                application_context,
            )
            .await?
            {
                return Ok(Some(choice));
            }
            if browsing_campaign {
                // The browser consumed pointer releases; do not carry its
                // opening press into the restored main menu.
                self.input_state = ModalInputState::new();
                self.input_state.seed_mouse_from_window(window, transform);
                for widget in self.frame.widgets_mut() {
                    widget.base_mut().state = UiState::Default;
                }
            }
        }

        if exit_requested {
            let msg = menu_resources.menu_text.get(MT_MSG_RETURN_TO_WINDOWS);
            // OS close is already a durable shutdown request. Escape and the
            // menu's Exit button still ask, but a nested dialog must not turn
            // WM_DELETE_WINDOW into "No" and reopen forever.
            if window.close_requested
                || show_yesno(
                    &mut *window,
                    renderer,
                    menu_resources,
                    Some(ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0)),
                    &msg,
                )
                .await
            {
                // Persist the profile manager right before closing so
                // unsaved profile-level changes (active selection,
                // renames, etc.) survive the exit.
                application_context
                    .save_player_profiles()
                    .unwrap_or_else(|error| {
                        panic!("Main menu Exit lost its ApplicationContext: {error}")
                    })
                    .log_persistence_error("Main menu Exit: failed to save profile manager");
                return Ok(Some(MainMenuChoice::Exit));
            }
            // Cancelled — stay in the menu and redraw next frame.
        }

        // ── Render ──────────────────────────────────────────────
        //
        // Background, button sprites, text, and cursor all draw through the
        // GPU queue; no menu frame mutates a retained software surface.

        renderer.begin_gpu_frame_clear();
        renderer.begin_ui_only_frame();

        if let Some(bg) = bg {
            let bg_x = transform.origin_x + (MENU_W - bg.width) / 2;
            let bg_y = transform.origin_y + (MENU_H - bg.height) / 2;
            let src = BBox::from_coords(0.0, 0.0, bg.width as f32, bg.height as f32);
            let dst = BBox::from_coords(
                bg_x as f32,
                bg_y as f32,
                (bg_x + bg.width) as f32,
                (bg_y + bg.height) as f32,
            );
            renderer
                .draw_surface(bg.id, Some(&src), Some(&dst), 0)
                .expect("live menu background");
        }

        // Buttons (sprite layer).
        for widget in self.frame.widgets() {
            let base = widget.base();
            let enabled = base.enabled;
            let hovered = matches!(base.state, UiState::Focused | UiState::Pushed)
                || (widget.id() == self.keyboard_selection && base.state == UiState::Default);
            let pressed = base.state == UiState::Pushed;
            let state_idx = button_sprite_state(enabled, hovered, pressed);
            let Some(rect) = base.bbox.0 else { continue };
            let (bx, by) = main_menu_to_screen(transform, rect.min().x as i32, rect.min().y as i32);
            let bw = (rect.max().x - rect.min().x) as i32;
            let bh = (rect.max().y - rect.min().y) as i32;
            if let Some(surf) = menu_resources.button_surface(state_idx) {
                let src = BBox::from_coords(0.0, 0.0, bw as f32, bh as f32);
                let dst =
                    BBox::from_coords(bx as f32, by as f32, (bx + bw) as f32, (by + bh) as f32);
                renderer
                    .draw_surface(surf, Some(&src), Some(&dst), BLIT_SOURCE_TRANSPARENT)
                    .expect("live menu button");
            }
        }

        // Text layer (profile info + button labels). Places button text
        // on the sprite at `(btn_h - font_height) / 2`, but emits atlas
        // glyph quads instead of drawing into a software surface.
        render_text_layer(
            application_context,
            renderer,
            menu_resources,
            &self.frame,
            self.keyboard_selection,
            transform,
        );

        // Custom cursor on top — the OS cursor is hidden, so skip this
        // and the mouse appears to vanish.
        cursor_renderer.advance_ui_animation();
        ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0).draw(
            renderer,
            transform,
            &self.input_state,
        );

        renderer.present();

        Ok(None)
    }
}

/// Default-profile prompt: runs before the event loop so the user picks
/// a name + difficulty on first launch — the manager starts with
/// `default_profiles = true` after [`crate::player_profile_store::PlayerProfileStore::load`]
/// auto-creates a placeholder "Robin".
///
/// Clears the flag unconditionally (even on cancel) so the prompt never
/// repeats. On OK, replaces the placeholder and its key configuration as one
/// context transition. On cancel, the placeholder becomes the final profile.
async fn prompt_first_launch_new_player(
    application_context: &ApplicationContext,
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor_renderer: &mut CursorRenderer,
) {
    let needs_prompt = application_context
        .with_player_profiles(|mgr| mgr.default_profiles)
        .unwrap_or_else(|error| panic!("first-launch prompt lost its ApplicationContext: {error}"));
    if !needs_prompt {
        return;
    }

    // Use the placeholder's name as the modal's initial value — the
    // autogenerated "Robin" default plays the role of the original's
    // anonymous fallback.
    let (initial_name, base_resolution) = application_context
        .with_active_profile(|profile| {
            (
                profile.name.clone(),
                (
                    profile.graphic_config.resolution_x.round() as u32,
                    profile.graphic_config.resolution_y.round() as u32,
                ),
            )
        })
        .unwrap_or_else(|error| panic!("first-launch prompt requires an active profile: {error}"));

    let outcome = player_select::show_new_player_prompt(
        event_pump,
        renderer,
        resources,
        initial_name,
        Some(ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0)),
    )
    .await;

    if let Err(error) = application_context.complete_first_launch_profile(outcome, base_resolution)
    {
        // Keep the auto-created placeholder as the active ordinary-game
        // profile. In particular, an unavailable Spellforge trust store must
        // fail closed for remote code without turning profile recovery into a
        // process-wide startup failure.
        tracing::error!(
            "first-launch profile update failed; continuing with Spellforge trust unavailable: {error}"
        );
    }
}

fn move_keyboard_selection(frame: &FrameWnd, selection: &mut u32, direction: i32) {
    if let Some(next) = frame.next_enabled_widget(*selection, direction > 0) {
        *selection = next;
    }
}

/// Build presentation-only profiles for a saved campaign. Overlay missions
/// launched with `--mission` have a forced profile even when their asset
/// descriptor uses BuiltIn. Browsing their history does not restore assets.
fn campaign_browser_profiles(
    profiles: &engine_profiles::ProfileManager,
    campaign: &Campaign,
    descriptor: &robin_engine::mission_assets::MissionAssetDescriptor,
) -> Result<engine_profiles::ProfileManager, String> {
    let mission = campaign
        .current_mission_idx
        .and_then(|index| campaign.missions.get(index))
        .ok_or_else(|| "saved campaign has no valid current mission".to_owned())?;
    let index = mission
        .profile_idx
        .ok_or_else(|| "saved current mission has no profile index".to_owned())?
        as usize;
    let mut view = profiles.clone();
    if index == view.missions.len() {
        view.add_forced_mission(
            descriptor.proto_level_filename.clone(),
            descriptor.mission_basename.clone(),
            descriptor.mission_basename.clone(),
        );
    } else if index > view.missions.len() {
        return Err(format!("saved campaign references missing profile {index}"));
    }
    Ok(view)
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
    application_context: &ApplicationContext,
) -> Result<Option<MainMenuChoice>, String> {
    #[cfg(not(feature = "multiplayer"))]
    let _ = campaign;
    Ok(match action {
        ClickAction::Return(c) => Some(c),
        ClickAction::CampaignManager => {
            let mut view_profiles = profiles.clone();
            let view_campaign = if let Some(index) = save_manager.find_resume_target() {
                let save = save_manager
                    .preflight_exact_slot(index)
                    .map_err(|error| format!("Cannot open Campaign Manager: {error:#}"))?;
                // Reconstruct only the static descriptor used for presentation.
                // No mission assets are mounted and no saved simulation is applied.
                view_profiles = campaign_browser_profiles(
                    profiles,
                    save.engine.campaign(),
                    &save.header.mission_assets,
                )?;
                crate::game_session::install_and_validate_saved_profile(&mut view_profiles, &save)?;
                save.engine.campaign().clone()
            } else {
                let difficulty =
                    application_context.with_active_profile(|profile| profile.difficulty)?;
                let mut fresh = Campaign::default();
                fresh.reset(&view_profiles, difficulty);
                fresh
            };
            let mut state = crate::campaign_map::CampaignMapModalState::new_browser(
                application_context,
                renderer,
                &view_campaign,
                &view_profiles,
                menu_resources,
            );
            loop {
                let cursor = ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0);
                if let Some(exit) = state.tick_browser(event_pump, renderer, Some(&cursor)) {
                    break if exit {
                        Some(MainMenuChoice::Exit)
                    } else {
                        None
                    };
                }
                crate::window::sleep_ui_frame().await;
            }
        }
        ClickAction::LoadGame => {
            save_load::run_main_menu_load(
                application_context,
                event_pump,
                renderer,
                menu_resources,
                ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0),
                save_manager,
            )
            .await
        }
        #[cfg(feature = "multiplayer")]
        ClickAction::Multiplayer => multiplayer_menu::show_multiplayer_menu(
            event_pump,
            renderer,
            menu_resources,
            cursor_renderer,
            campaign,
            profiles,
            application_context,
            None,
        )
        .await
        .map(MainMenuChoice::Multiplayer),
        ClickAction::SelectPlayer => {
            player_select::show_select_player(
                application_context,
                event_pump,
                renderer,
                menu_resources,
                Some(ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0)),
            )
            .await;
            // Active profile may have changed — reopen the save manager so
            // subsequent "Load Game" clicks read the new profile's
            // `Profile_NNN/saves.json` index rather than the prior one.
            match crate::save_recovery::open_with_recovery(
                application_context,
                event_pump,
                renderer,
                menu_resources,
                Some(&ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0)),
            )
            .await
            {
                crate::save_recovery::OpenedSaveStore::Ready(manager) => *save_manager = manager,
                crate::save_recovery::OpenedSaveStore::Cancelled
                | crate::save_recovery::OpenedSaveStore::ExitRequested => {
                    return Ok(Some(MainMenuChoice::Exit));
                }
            }
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
            // Preserve the original game's player-selection entry behavior.
            let graphic = application_context
                .with_active_profile(|profile| profile.graphic_config.clone())
                .unwrap_or_else(|error| {
                    panic!("Select Player removed the active profile: {error}")
                });
            event_pump.set_logical_resolution_policy(&graphic);
            renderer.sync_window_size(event_pump);
            renderer.apply_upscale_config(&graphic);
            event_pump.set_native_refresh_presentation(graphic.native_refresh_presentation);
            renderer.configure_native_refresh_presentation(
                graphic.native_refresh_presentation,
                event_pump.surface_config.width,
                event_pump.surface_config.height,
            );
            None
        }
        ClickAction::Options => {
            let language_changed = options::show_main_menu_options(
                application_context,
                event_pump,
                renderer,
                menu_resources,
                cursor_renderer,
            )
            .await;
            if language_changed {
                return Ok(Some(MainMenuChoice::RedisplayOptions));
            }
            let graphic = application_context
                .with_active_profile(|profile| profile.graphic_config.clone())
                .unwrap_or_else(|error| panic!("Options removed the active profile: {error}"));
            event_pump.set_logical_resolution_policy(&graphic);
            renderer.sync_window_size(event_pump);
            None
        }
        ClickAction::ShowCredits => {
            credits::show_credits(application_context, event_pump, renderer).await;
            None
        }
        ClickAction::ShowMovies => {
            movies::show_movies(application_context, event_pump, renderer, menu_resources).await;
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
                application_context.preparation_files()?,
            )
            .await
            .map(MainMenuChoice::CustomMission)
        }
    })
}

/// Render every piece of text in the main menu (profile info block on
/// the left, button labels on the right).
fn render_text_layer(
    application_context: &ApplicationContext,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    frame: &FrameWnd,
    keyboard_selection: u32,
    transform: MenuTransform,
) {
    // Prepare owned display text so the profile lock never reaches the GPU path.
    let (profile_name, profile_info_lines) = application_context
        .with_active_profile(|profile| {
            (
                Some(profile.name.clone()),
                build_profile_info_lines(resources, profile),
            )
        })
        .unwrap_or_else(|error| panic!("main-menu rendering requires an active profile: {error}"));

    let name_font = resources
        .edit_field_font_any()
        .or_else(|| resources.title_font_any());
    let info_font = resources
        .menu_text_font_any()
        .or_else(|| resources.edit_field_font_any());
    let enabled_font = resources.menu_button_font_any(true);
    let disabled_font = resources.menu_button_font_any(false);

    // ── Profile info block (left side) ──────────────────────────────
    if let (Some(name), Some(font)) = (profile_name.as_deref(), name_font) {
        let tw = font.text_width(name);
        let x = PROFILE_INFO_BOX_X + (PROFILE_INFO_BOX_W - tw) / 2;
        crate::ingame_menu::layout::render_text_virt_font(
            renderer,
            font,
            transform,
            name,
            x,
            PROFILE_NAME_Y,
        );
    }
    if let Some(font) = info_font {
        let line_h = font.height() as i32;
        for (i, line) in profile_info_lines.iter().enumerate() {
            let tw = font.text_width(line);
            let x = PROFILE_INFO_BOX_X + (PROFILE_INFO_BOX_W - tw) / 2;
            let y = PROFILE_INFO_Y + i as i32 * line_h;
            crate::ingame_menu::layout::render_text_virt_font(
                renderer, font, transform, line, x, y,
            );
        }
    }

    // ── Auto-update status (bottom-left corner) ─────────────────────
    if let Some(font) = info_font {
        let status = update_status_text();
        let line_h = font.height() as i32;
        let top = MENU_H - status.lines().count() as i32 * line_h - 4;
        for (i, line) in status.lines().enumerate() {
            let y = top + i as i32 * line_h;
            crate::ingame_menu::layout::render_text_virt_font(
                renderer, font, transform, line, 8, y,
            );
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
        crate::ingame_menu::layout::render_text_virt_font(
            renderer, font, transform, &base.text, tx, ty,
        );
        // Keyboard-selected widget keyboard-only: the hover sprite is
        // already handled by `button_sprite_state` in the sprite pass,
        // so no extra work here.
        let _ = keyboard_selection;
    }
}

/// Human-readable version and auto-update progress for the menu's bottom-left
/// corner.
fn update_status_text() -> String {
    let version = crate::version::version_label();
    #[cfg(all(
        feature = "auto-update",
        any(target_os = "windows", target_os = "linux", target_os = "macos")
    ))]
    {
        use crate::auto_update::UpdateStatus;
        match crate::auto_update::update_status() {
            Some(UpdateStatus::Downloading {
                version: update_version,
            }) => {
                format!("{version}\nDownloading update v{update_version}...")
            }
            Some(UpdateStatus::ReadyOnExit {
                version: update_version,
            }) => {
                format!("{version}\nUpdate v{update_version} will install on exit")
            }
            None => format!("{version}\nUp to date"),
        }
    }
    #[cfg(not(all(
        feature = "auto-update",
        any(target_os = "windows", target_os = "linux", target_os = "macos")
    )))]
    {
        version
    }
}

fn build_profile_info_lines(
    resources: &IngameMenuResources,
    profile: &PlayerProfile,
) -> Vec<String> {
    let difficulty_label = resources.menu_text.get(MT_STR_DIFFICULTY_LEVEL);
    let difficulty_value = difficulty_to_string(&resources.menu_text, profile.difficulty);
    let money = substitute_integer(
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
fn difficulty_to_string(
    menu_text: &crate::ingame_menu::resources::MenuText,
    level: DifficultyLevel,
) -> String {
    match level {
        DifficultyLevel::Easy => menu_text.get(MT_STR_DIFFICULTY_EASY),
        DifficultyLevel::Medium => menu_text.get(MT_STR_DIFFICULTY_MEDIUM),
        DifficultyLevel::Hard => menu_text.get(MT_STR_DIFFICULTY_HARD),
        DifficultyLevel::Legendary => menu_text
            .get_port(MT_PORT_STR_DIFFICULTY_LEGENDARY)
            .to_owned(),
        DifficultyLevel::Custom(_) => menu_text.get_port(MT_PORT_STR_DIFFICULTY_CUSTOM).to_owned(),
    }
}

/// Format a duration as `HH:MM` with zero padding.
fn seconds_to_time(seconds: u32) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds - hours * 3600) / 60;
    format!("{hours:02}:{minutes:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_menu_frame_state_retains_navigation_and_reports_close_with_activation() {
        let mut frame = FrameWnd::interactive();
        for id in 0..3 {
            frame.add_widget_absolute(widget_bridge::make_button_enabled(
                id,
                "choice",
                id != 1,
                100,
                100 + id as i32 * 30,
                80,
                20,
            ));
        }
        let mut state = MainMenuState::new(frame);
        let transform = MenuTransform::centered(640, 480);
        let key = |keycode| GameEvent::KeyDown {
            keycode,
            physical_key: None,
        };
        assert_eq!(
            state.process_events(vec![key(Keycode::Down)], transform, &mut None),
            (None, false)
        );
        assert_eq!(state.keyboard_selection, 2);
        assert_eq!(
            state.process_events(vec![], transform, &mut None),
            (None, false)
        );
        assert_eq!(state.keyboard_selection, 2);
        assert_eq!(
            state.process_events(
                vec![key(Keycode::Return), GameEvent::Quit],
                transform,
                &mut None
            ),
            (Some(2), true)
        );
        assert_eq!(
            state.process_events(vec![key(Keycode::Up)], transform, &mut None),
            (None, false)
        );
        assert_eq!(state.keyboard_selection, 0);
    }

    #[test]
    fn difficulty_labels_preserve_localized_legacy_and_application_namespaces() {
        let mut menu_text = crate::ingame_menu::resources::MenuText::english_fallbacks_only();
        let mut strings = vec![
            String::new();
            MT_STR_DIFFICULTY_HARD
                .max(MT_STR_DIFFICULTY_MEDIUM)
                .max(MT_STR_DIFFICULTY_EASY)
                + 1
        ];
        strings[MT_STR_DIFFICULTY_EASY] = "Facile".into();
        strings[MT_STR_DIFFICULTY_MEDIUM] = "Moyen".into();
        strings[MT_STR_DIFFICULTY_HARD] = "Difficile".into();
        menu_text.replace_strings_for_test(strings);
        for (difficulty, expected) in [
            (DifficultyLevel::Easy, "Facile"),
            (DifficultyLevel::Medium, "Moyen"),
            (DifficultyLevel::Hard, "Difficile"),
            (DifficultyLevel::Legendary, "Legendary"),
            (
                DifficultyLevel::Custom(robin_engine::player_profile::DifficultyRules::EASY),
                "Custom",
            ),
            (
                DifficultyLevel::Custom(robin_engine::player_profile::DifficultyRules::HARD),
                "Custom",
            ),
        ] {
            assert_eq!(difficulty_to_string(&menu_text, difficulty), expected);
        }
    }

    #[test]
    fn campaign_browser_restores_overlay_profile_without_changing_runtime_catalog() {
        let mut profiles = engine_profiles::ProfileManager::new();
        profiles.missions.push(Default::default());
        let mut campaign = Campaign::default();
        let mut mission = robin_engine::mission::Mission::new();
        mission.profile_idx = Some(1);
        campaign.missions.push(mission);
        campaign.current_mission_idx = Some(0);
        let descriptor = robin_engine::mission_assets::MissionAssetDescriptor::built_in(
            "Fabri18SpriteGallery",
            "OpenBattlefield",
            "OpenBattlefield",
        )
        .unwrap();
        let view = campaign_browser_profiles(&profiles, &campaign, &descriptor).unwrap();
        assert_eq!(profiles.missions.len(), 1);
        assert_eq!(view.missions.len(), 2);
        assert_eq!(view.missions[1].id, 1);
        assert_eq!(view.missions[1].mission_filename, "Fabri18SpriteGallery");
        assert_eq!(view.missions[1].proto_level_filename, "OpenBattlefield");
        let existing = campaign_browser_profiles(&view, &campaign, &descriptor).unwrap();
        assert_eq!(existing.missions.len(), 2);
        campaign.missions[0].profile_idx = Some(2);
        assert!(campaign_browser_profiles(&profiles, &campaign, &descriptor).is_err());
        campaign.current_mission_idx = None;
        assert!(campaign_browser_profiles(&profiles, &campaign, &descriptor).is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn menu_audio_context(
        root: &std::path::Path,
        files: Option<std::sync::Arc<robin_engine::sbfile::SbFileSystem>>,
    ) -> ApplicationContext {
        let directory = root.to_str().unwrap();
        let mut profiles =
            robin_engine::player_profile::PlayerProfileManager::new(directory.into());
        let active = profiles.create_profile(
            "Menu audio".into(),
            robin_engine::player_profile::DifficultyLevel::Medium,
        );
        profiles.set_active(active);
        ApplicationContext::complete_with_localization_and_files(
            crate::player_profile_store::PlayerProfileStore::for_directory(directory),
            robin_engine::engine::GlobalOptions::default(),
            profiles,
            crate::key_config_store::KeyConfigStore::new(directory.into()),
            None,
            crate::localization::LocalizationService::disabled(),
            files,
        )
        .unwrap()
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn menu_audio_preparation_uses_independent_application_readers() {
        use std::sync::Arc;
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for (root, sample_count) in [(first.path(), 1u32), (second.path(), 2)] {
            let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
            let mut bank = b"FXBK".to_vec();
            bank.resize(12, 0);
            bank.extend_from_slice(&1u32.to_le_bytes()); // One widget group.
            bank.extend_from_slice(&0u32.to_le_bytes()); // Material group.
            bank.extend_from_slice(&1u32.to_le_bytes()); // Group id.
            bank.extend_from_slice(&0u16.to_le_bytes()); // Volume.
            bank.extend_from_slice(&0u16.to_le_bytes()); // Gaps.
            bank.extend_from_slice(&1u32.to_le_bytes()); // One entry.
            bank.extend_from_slice(&0u32.to_le_bytes()); // Entry type.
            bank.extend_from_slice(&0u32.to_le_bytes()); // Entry group.
            bank.extend_from_slice(&0u16.to_le_bytes()); // Entry volume.
            bank.extend_from_slice(&9u16.to_le_bytes());
            bank.extend_from_slice(b"owned.wav");
            assets
                .install_preloaded_asset("Data/Sounds/Menu/menu.fxg", bank)
                .unwrap();
            let size = sample_count * 2;
            let mut wav = b"RIFF".to_vec();
            wav.extend_from_slice(&(36 + size).to_le_bytes());
            wav.extend_from_slice(b"WAVEfmt ");
            wav.extend_from_slice(&16u32.to_le_bytes());
            wav.extend_from_slice(&1u16.to_le_bytes()); // PCM.
            wav.extend_from_slice(&1u16.to_le_bytes()); // Mono.
            wav.extend_from_slice(&1u32.to_le_bytes()); // One sample/sec.
            wav.extend_from_slice(&2u32.to_le_bytes()); // Two bytes/sec.
            wav.extend_from_slice(&2u16.to_le_bytes());
            wav.extend_from_slice(&16u16.to_le_bytes());
            wav.extend_from_slice(b"data");
            wav.extend_from_slice(&size.to_le_bytes());
            wav.resize(44 + size as usize, 0);
            assets
                .install_preloaded_asset("Data/Sounds/Menu/owned.wav", wav)
                .unwrap();
            let files = Arc::new(robin_engine::sbfile::SbFileSystem::new(assets).snapshot());
            let context = menu_audio_context(root, Some(files));
            let mut sound = SoundManager::default();
            let loader = prepare_menu_sound(&context, &mut sound).unwrap();
            let entry = &sound.sound_cache_mut().menu_cache.entries[&(1 << 16)];
            assert_eq!(entry.file_name, "Menu/owned.wav");
            assert_eq!(loader(&entry.file_name).unwrap().2, sample_count * 1000);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn menu_audio_preparation_reports_missing_authority_and_invalid_bank() {
        let root = tempfile::tempdir().unwrap();
        let context = menu_audio_context(root.path(), None);
        assert!(prepare_menu_sound(&context, &mut SoundManager::default()).is_err());

        let assets = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        assets
            .install_preloaded_asset("Data/Sounds/Menu/menu.fxg", b"broken bank".to_vec())
            .unwrap();
        let files = std::sync::Arc::new(robin_engine::sbfile::SbFileSystem::new(assets));
        let context = menu_audio_context(root.path(), Some(files));
        let error = prepare_menu_sound(&context, &mut SoundManager::default())
            .err()
            .unwrap();
        assert!(error.contains("menu sound bank parse failed"), "{error}");
    }

    fn assert_main_menu_projection(
        screen_w: i32,
        screen_h: i32,
        expected_origin: (i32, i32),
        expected_first_button: (i32, i32),
        expected_profile_anchor: (i32, i32),
        expected_status_anchor: (i32, i32),
    ) {
        let transform = MenuTransform::centered(screen_w, screen_h);
        assert_eq!((transform.origin_x, transform.origin_y), expected_origin);

        // DEFAULT.RES uses 168x39 main-menu buttons. Nine entries occupy the
        // bottom-right of the virtual 640x480 frame on current main.
        let labels = [("button", true); 9];
        let buttons = align_bottom_right(&labels, 168, 39);
        let first = &buttons[0];
        assert_eq!((first.x, first.y), (472, 113));
        let first_button = main_menu_to_screen(transform, first.x, first.y);
        assert_eq!(first_button, expected_first_button);
        assert_eq!(
            transform.from_screen(first_button.0, first_button.1),
            (first.x, first.y)
        );

        // Representative text anchors cover the profile block and the
        // bottom-left status line. They must receive the identical offset as
        // the sprites rather than remaining at raw virtual coordinates.
        let profile_anchor = main_menu_to_screen(transform, 200, PROFILE_NAME_Y);
        assert_eq!(profile_anchor, expected_profile_anchor);
        assert_eq!(
            transform.from_screen(profile_anchor.0, profile_anchor.1),
            (200, PROFILE_NAME_Y)
        );

        let status_anchor = main_menu_to_screen(transform, 8, 458);
        assert_eq!(status_anchor, expected_status_anchor);
        assert_eq!(
            transform.from_screen(status_anchor.0, status_anchor.1),
            (8, 458)
        );
    }

    #[test]
    fn main_menu_draw_and_input_align_on_low_widescreen_canvas() {
        assert_main_menu_projection(853, 480, (106, 0), (578, 113), (306, 100), (114, 458));
    }

    #[test]
    fn main_menu_draw_and_input_align_on_high_widescreen_canvas() {
        assert_main_menu_projection(1280, 720, (320, 120), (792, 233), (520, 220), (328, 578));
    }

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
        assert_eq!(substitute_integer("Money: £%i", 100), "Money: £100");
        assert_eq!(substitute_integer("Ransom: %d", 42), "Ransom: 42");
    }

    #[test]
    fn substitute_i_uses_first_position_not_specifier_priority() {
        assert_eq!(substitute_integer("£%d / %i", -42), "£-42 / %i");
        assert_eq!(substitute_integer("%i / %d", 42), "42 / %d");
        assert_eq!(
            substitute_integer("%d %d", i64::MIN),
            format!("{} %d", i64::MIN)
        );
        assert_eq!(substitute_integer("%u / %s", 42), "%u / %s");
    }

    #[test]
    fn substitute_i_no_placeholder() {
        assert_eq!(substitute_integer("no format", 5), "no format");
    }

    #[test]
    fn update_text_matches_the_available_update_integration() {
        let line = update_status_text();
        assert!(line.starts_with(&crate::version::version_label()));
        #[cfg(all(
            feature = "auto-update",
            any(target_os = "windows", target_os = "linux", target_os = "macos")
        ))]
        assert_eq!(
            line,
            format!("{}\nUp to date", crate::version::version_label())
        );
        #[cfg(not(all(
            feature = "auto-update",
            any(target_os = "windows", target_os = "linux", target_os = "macos")
        )))]
        assert_eq!(line, crate::version::version_label());
    }
}

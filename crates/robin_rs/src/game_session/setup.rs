//! Mission-startup helpers extracted from `game_session`:
//! audio bank loading, sound-duration tables, level/sprite-bank
//! initialization, sprite renderer setup, and the Kira audio backend
//! bootstrap.

mod custom_sprites;
mod error;
mod localization;
mod preparation;
mod resources;

use custom_sprites::prepare_custom_character_dirs;
use localization::apply_mission_descriptor_patch;
pub use localization::{load_fixed_vip_name_map, load_peasant_name_pool};
pub(super) use resources::{
    DecodingInterfaceResources, MissionEngineResources, MissionProcessResources,
};

use crate::audio_backend::KiraAudioBackend;
use crate::cursor::CursorRenderer;
use crate::game::Game;
use crate::host::Host;
use crate::hud_text::HudFonts;
use crate::input::ThreadedInput;
use crate::input_translator::{GameKey, InputTranslator};
use crate::main_entry::picture_to_surface;
use crate::markers::SelectionMarkRenderer;
use crate::mouse_trail::MouseTrailRenderer;
use crate::renderer::Renderer;
use crate::sound::{NUM_CHANNELS, SoundMode};
use crate::titbit_renderer::TitbitRenderer;
use crate::ui_panel::{PortraitCache, load_localized_character_names};
use robin_assets::frame_holder as assets_frame_holder;
use robin_assets::res_descr as assets_res_descr;
use robin_assets::resource_manager::ResourceManager;
use robin_assets::scb as assets_scb;
use robin_engine::campaign::Campaign;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::ScreenSize;
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::player_command::PlayerCommand;
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::MissionLocation;
use robin_engine::resource_ids;
use robin_engine::sbfile as engine_sbfile;
use robin_engine::script_manager as engine_script_manager;
use robin_engine::sprite_script as engine_sprite_script;

/// Wall-clock step timer for the mission setup phase.  Each [`step`] logs the
/// time since the previous step at debug level (info when it crossed
/// `SLOW_STEP_MS`), so a slow launch shows exactly which setup stage ate the
/// time on both native and wasm builds.
pub(crate) struct PhaseTimer {
    phase: &'static str,
    started: web_time::Instant,
    last: web_time::Instant,
}

impl PhaseTimer {
    const SLOW_STEP_MS: u64 = 50;

    pub(crate) fn new(phase: &'static str) -> Self {
        let now = web_time::Instant::now();
        Self {
            phase,
            started: now,
            last: now,
        }
    }

    /// Log the elapsed time of the step that just finished.
    pub(crate) fn step(&mut self, label: &str) {
        let now = web_time::Instant::now();
        let ms = now.duration_since(self.last).as_millis() as u64;
        self.last = now;
        if ms >= Self::SLOW_STEP_MS {
            tracing::info!(elapsed_ms = ms, "{}: {label}", self.phase);
        } else {
            tracing::debug!(elapsed_ms = ms, "{}: {label}", self.phase);
        }
    }

    /// Log the total time since construction.
    pub(crate) fn total(&self) {
        tracing::info!(
            elapsed_ms = self.started.elapsed().as_millis() as u64,
            "{}: total",
            self.phase
        );
    }
}

// Tail-phase loading targets share one monotonic schedule. Keeping these in
// one place prevents a slow earlier phase (notably map decompression) from
// advancing beyond a later phase's ceiling and making the loading bar stall.
// Referenced by the wasm synchronous map-decode branch and the
// monotonic-schedule test; native decodes the map on a worker thread
// without loading-bar status updates.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) const LOADING_MAP_DECODE_PROGRESS: f32 = 0.85;
pub(super) const LOADING_SPRITE_VARIANTS_PROGRESS: f32 = 0.88;
pub(super) const LOADING_AUDIO_PROGRESS: f32 = 0.91;
pub(super) const LOADING_DESCRIPTORS_PROGRESS: f32 = 0.95;
pub(super) const LOADING_HUD_FONTS_PROGRESS: f32 = 0.98;
pub(super) const LOADING_FINAL_PROGRESS: f32 = 1.0;

/// Load mission-specific sound banks and switch to mission music.
///
/// Loads the FX / menu / exclamation caches, populates the music pool
/// from the mission profile, and switches the mixer to mission mode after
/// required script startup and before the loading screen closes. Pure
/// host-side work — reads
/// profile/sound metadata off the engine but does not mutate it.
pub(super) fn setup_mission_audio(
    host: &mut Host,
    backend: Option<&mut KiraAudioBackend>,
    engine: &Engine,
    assets: &mut LevelAssets,
    profiles: &engine_profiles::ProfileManager,
    location: MissionLocation,
    sound_dir: &str,
) -> Result<(), String> {
    let mut timer = PhaseTimer::new("mission audio setup");
    let loader = crate::audio_backend::create_sample_loader_with_files(
        std::path::PathBuf::from(sound_dir),
        host.preparation_files()?.clone(),
        host.frontend.resources.shipping.clone(),
    );

    // Initialize music pools from the mission profile.
    let campaign = engine.campaign();
    if let Some(idx) = campaign.current_mission_idx {
        let prof = campaign.missions[idx].profile(profiles);
        host.audio.sound.sound_cache_mut().initialize_music(
            &prof.green_music,
            &prof.yellow_music,
            &prof.red_music,
        );
    }

    // The deterministic banks and duration metadata were populated before
    // engine preparation so the exact same bytes are sealed for ordinary
    // missions and projection export. Only engine-derived source finalization
    // remains in this post-construction phase.
    host.audio
        .sound
        .sound_cache_mut()
        .finalize_sound_sources(&engine.sound_sim().sources);
    timer.step("bank registration");
    let _ = (assets, profiles);

    // Per-entry sample validation block.  When
    // sound-data checking is enabled, the engine validates
    // each sample as it's added; we run the equivalent load+unload
    // sweep here because `add_entry` doesn't have a loader at insert
    // time. The resulting `data_check_succeeded` flag is consulted by
    // `SoundManager::activate` (fatal panic on miss).
    let check = host.application_context().options().check_sound_data;
    if check {
        host.audio.sound.sound_cache_mut().validate_data(&loader);
        timer.step("check_sound_data validation");
    }

    if let Some(backend) = backend {
        host.audio.sound.activate(
            location == MissionLocation::Sherwood,
            &engine.sound_sim().sources,
        );
        // Enter mission mode during the final loading stage, including
        // replay viewers that skipped loading-screen menu music. This raises
        // load_music so normal mission music starts from the pool.
        host.audio.sound.set_mode(SoundMode::Mission, backend);
        timer.step("mixer activation");
    }
    timer.total();
    Ok(())
}

fn initialize_mission_sound_caches(
    host: &mut Host,
    profiles: &engine_profiles::ProfileManager,
    required_source_ids: &std::collections::BTreeSet<u32>,
    files: std::sync::Arc<engine_sbfile::SbFileSystem>,
) -> Result<(), String> {
    // Mission transitions must not retain any preceding mission's group or
    // source closure: IndexedCache correctly rejects duplicate group IDs.
    host.audio.sound.sound_cache_mut().flush(true);
    let asset_cache = host.application_context().asset_cache()?.get_or_build(
        host.frontend.resources.shipping.as_deref(),
        profiles,
        files,
    );
    if let Some(elements) = asset_cache.fx_bank.as_ref() {
        host.audio
            .sound
            .sound_cache_mut()
            .initialize_fx_cache(elements);
        tracing::info!("Loaded FX bank: {} elements", elements.len());
    }
    if let Some(entries) = asset_cache.menu_bank.as_ref() {
        host.audio
            .sound
            .sound_cache_mut()
            .initialize_menu_cache(entries);
        tracing::info!("Loaded menu sound bank: {} entries", entries.len());
    }
    for resolved in &asset_cache.exclamations {
        host.audio
            .sound
            .sound_cache_mut()
            .initialize_exclamations_for_profile(resolved);
    }
    host.audio
        .sound
        .sound_cache_mut()
        .initialize_sound_source_cache(required_source_ids);
    Ok(())
}

fn required_mission_exclamation_ids(
    loaded: &robin_engine::level_data::LoadedLevel,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
) -> Result<std::collections::BTreeSet<u32>, String> {
    use std::collections::BTreeSet;

    let forest_level = loaded
        .proto
        .misc
        .as_ref()
        .is_some_and(|misc| misc.forest_level);
    let normalize_character = |index: u32| -> Result<usize, String> {
        let profile = profiles
            .characters
            .get(index as usize)
            .ok_or_else(|| format!("required speech character profile {index} does not exist"))?;
        if !matches!(profile.filename.as_str(), "RobinHood" | "RobinTown") {
            return Ok(index as usize);
        }
        let wanted = if forest_level {
            "RobinHood"
        } else {
            "RobinTown"
        };
        profiles
            .characters
            .iter()
            .position(|candidate| candidate.filename == wanted)
            .ok_or_else(|| format!("required normalized speech profile {wanted} is absent"))
    };
    let mut ids = BTreeSet::new();
    let add_character = |ids: &mut BTreeSet<u32>, index: u32| -> Result<(), String> {
        let index = normalize_character(index)?;
        let id = profiles.characters[index].exclamation_id;
        if id != 0 {
            ids.insert(id);
        }
        Ok(())
    };

    let mission_index = campaign
        .current_mission_idx
        .ok_or_else(|| "speech closure requires a current campaign mission".to_owned())?;
    let mission = campaign
        .missions
        .get(mission_index)
        .ok_or_else(|| format!("current campaign mission {mission_index} does not exist"))?;
    let mission_profile = mission.profile(profiles);
    for &index in &mission_profile.required_character_indices {
        add_character(&mut ids, index)?;
    }
    for soldier in &loaded.mission.soldiers {
        let index = soldier.profile_index(profiles)?;
        let profile = profiles
            .get_soldier(index)
            .expect("resolved soldier profile");
        if profile.exclamation_id != 0 {
            ids.insert(profile.exclamation_id);
        }
    }
    for civilian in &loaded.mission.civilians {
        let profile = profiles
            .civilians
            .get(civilian.profile_number as usize)
            .ok_or_else(|| {
                format!(
                    "mission civilian references missing speech profile {}",
                    civilian.profile_number
                )
            })?;
        if profile.exclamation_id != 0 {
            ids.insert(profile.exclamation_id);
        }
    }
    for rescued in &loaded.mission.pcs_to_rescue {
        add_character(&mut ids, rescued.profile_index)?;
    }
    for &character_index in &campaign.mission_team_indices {
        let description = campaign.characters.get(character_index).ok_or_else(|| {
            format!("mission team references missing character {character_index}")
        })?;
        let profile = description
            .character_profile_idx
            .ok_or_else(|| format!("mission-team character {character_index} has no profile"))?;
        add_character(&mut ids, profile.0)?;
    }
    for &character_index in &campaign.gang_indices {
        let description = campaign
            .characters
            .get(character_index)
            .ok_or_else(|| format!("gang references missing character {character_index}"))?;
        if description.instanced {
            continue;
        }
        let profile_index = description
            .character_profile_idx
            .ok_or_else(|| format!("gang character {character_index} has no profile"))?;
        let profile = profiles.get_character(profile_index).ok_or_else(|| {
            format!(
                "gang character references missing profile {}",
                profile_index.0
            )
        })?;
        if !profile.vip {
            add_character(&mut ids, profile_index.0)?;
        }
    }
    Ok(ids)
}

/// Frontend-only resources loaded while the interactive loading screen is
/// still visible.
///
/// This process data is consumed by `InteractiveFrontend` and deliberately is
/// not serialized as part of deterministic mission state.
pub(super) struct LoadedInteractiveResources {
    pub(super) level_descriptors: Option<assets_res_descr::LevelDescriptors>,
    pub(super) hud_fonts: Option<HudFonts>,
}

/// Pre-decode the background map + minimap and attach the interface /
/// text resource files while the loading screen is still visible.
///
/// Second progress-closure scope (the first one was dropped at the end
/// of the CPU-only loading block, so audio setup could borrow the window).
/// The closure must be dropped before we close the loading screen and hand
/// the GPU context to the game renderer.
///
/// Runs the slow CPU work *before* closing the loading screen:
///  - bzip2-decompress `.map` / `.min` + mask composition
///  - attach interface / text resource files (pure file I/O)
///  - load HUD font glyphs
///  - load level-descriptor `.red` file
///
/// Everything that needs the game renderer happens after the close.
/// `.map` / `.min` and resource attachments happen well before the
/// loading screen closes.
pub(super) fn pre_decode_maps_and_resources(
    mut event_pump: Option<&mut crate::window::GameWindow>,
    loading_screen: &mut Option<crate::loading_screen::LoadingScreenRenderer>,
    engine: &mut Engine,
    profiles: &engine_profiles::ProfileManager,
    host: &Host,
    game: &Game,
) -> Result<LoadedInteractiveResources, String> {
    let files = host.preparation_files()?;
    let mut timer = PhaseTimer::new("descriptor+font setup");
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    // `text_res` (Data/Text/Level.res) + `cursor_res` (DEFAULT.RES) were
    // attached earlier so `Engine::new` could absorb the peasant name
    // pool, ground-mark sprite data, and titbit row counts.

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Loading level descriptors...", LOADING_DESCRIPTORS_PROGRESS);
    }

    // Level descriptors (`.red` file) and HUD fonts — file I/O only.
    let mut level_descriptors = {
        let campaign = engine.campaign();
        let mission_id = crate::main_entry::current_mission_id(campaign, profiles);
        crate::mission_descriptors::for_presentation(
            host.application_context(),
            host.frontend.resources.shipping.as_deref(),
            mission_id,
        )
    };
    if let Some(descriptors) = level_descriptors.as_mut() {
        apply_mission_descriptor_patch(engine.campaign(), profiles, descriptors, files)
            .map_err(|error| error.to_string())?;
    }
    timer.step("level descriptors");
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Loading HUD fonts...", LOADING_HUD_FONTS_PROGRESS);
    }
    let hud_fonts = HudFonts::load(files);
    timer.step("HUD fonts");
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    // Background + minimap bitmaps are pre-decoded inside
    // `prepare_mission` — they must be decoded *before*
    // `Engine::new` so the engine can be constructed with real grid
    // dimensions (RAII).  This function now only handles the
    // post-engine resources (level descriptors + HUD fonts).  Let the
    // caller use the bg/mm carried through the mission preparation stages.
    let _ = (engine, game, host, event_pump);

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Finalizing...", LOADING_FINAL_PROGRESS);
    }
    Ok(LoadedInteractiveResources {
        level_descriptors,
        hud_fonts,
    })
}

/// Tick the loading-screen progress bar by `delta` and drain any
/// pending WM resize events so the canvas stays in sync during a
/// long-running mission load.
pub(super) fn tick_progress(
    loading_screen: &mut Option<crate::loading_screen::LoadingScreenRenderer>,
    event_pump: Option<&mut crate::window::GameWindow>,
    delta: f32,
) {
    if let Some(ls) = loading_screen.as_mut() {
        ls.increment(delta);
        if let Some(event_pump) = event_pump {
            ls.drain_events(event_pump);
        }
    }
}

/// Bundle of host-side renderers + caches populated from DEFAULT.RES
/// during mission setup.  Returned by [`load_mission_sprites`].
pub(super) struct MissionSprites {
    pub(super) cursor_renderer: CursorRenderer,
    pub(super) selection_mark_renderer: SelectionMarkRenderer,
    pub(super) mouse_trail_renderer: Option<MouseTrailRenderer>,
    pub(super) titbit_renderer: TitbitRenderer,
    pub(super) portrait_cache: PortraitCache,
}

/// Load the cursor, minimap button/dots, ground-focus marker, selection
/// mark, mouse trail, titbits, portraits, and peasant names — every
/// renderer that takes its frames from `DEFAULT.RES` / `Level.res`
/// during mission start-up.
///
/// Each subsystem fetches its surfaces from the shared interface bank.
/// Also pushes the derived data into the engine
/// (`setup_minimap_widget`, `set_ground_mark_sprite_data`,
/// `set_titbit_row_frame_counts`, `set_peasant_names`). The frame-holder
/// dictionaries were already finalized and published by the CPU loading phase.
pub(super) fn load_mission_sprites(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    text_res: &mut ResourceManager,
) -> MissionSprites {
    // ── Cursor setup ──
    // `cursor_res` (DEFAULT.RES) was pre-attached above while the
    // loading screen was still visible.
    let mut timer = PhaseTimer::new("mission sprite setup");
    host.frontend
        .resources
        .mission_surfaces
        .retire_sprites(renderer);
    let mut cursor_renderer = CursorRenderer::new();
    cursor_renderer.init(renderer);

    // Load the default game cursor.
    if !cursor_renderer.load_cursor(resource_ids::RHMOUSE_DEFAULT, cursor_res, renderer) {
        tracing::warn!("Failed to load default cursor — using fallback arrow");
    }
    timer.step("cursor");

    // ── Minimap corner button ──
    // Corner-sprite dims + hit mask were pre-computed from
    // `cursor_res` and handed to `Engine::new` via
    // `EngineArgs::minimap_widget`; this block only uploads the
    // corner GPU textures and stashes the corner size host-side for
    // the HUD layout.
    match cursor_res.get_dimension(resource_ids::RHMAP_CORNER) {
        Ok((btn_w, btn_h)) => {
            let corner_surfaces = match cursor_res.get_pictures(resource_ids::RHMAP_CORNER) {
                Ok(pics) => pics
                    .iter()
                    .filter_map(|opt| opt.as_ref().map(|p| picture_to_surface(renderer, p)))
                    .collect(),
                Err(error) => {
                    tracing::warn!(
                        "Failed to load RHMAP_CORNER pictures; preserving known layout dimensions: {error}"
                    );
                    Vec::new()
                }
            };
            host.frontend.resources.mission_surfaces.replace_corners(
                renderer,
                ScreenSize::new(btn_w as f32, btn_h as f32),
                corner_surfaces,
            );
            tracing::info!(
                "Minimap corner button: {}x{}, button at ({:.0}, {:.0}), map at ({:.0}, {:.0})",
                btn_w,
                btn_h,
                host.frontend
                    .presentation
                    .engine_display
                    .minimap()
                    .button_box()
                    .top_left()
                    .x,
                host.frontend
                    .presentation
                    .engine_display
                    .minimap()
                    .button_box()
                    .top_left()
                    .y,
                host.frontend
                    .presentation
                    .engine_display
                    .minimap()
                    .map_box()
                    .top_left()
                    .x,
                host.frontend
                    .presentation
                    .engine_display
                    .minimap()
                    .map_box()
                    .top_left()
                    .y,
            );
        }
        _ => {
            tracing::warn!("Failed to load RHMAP_CORNER resource — minimap button unavailable");
        }
    }

    // ── Minimap dot sprites (RHMAP_ITEMS) ──
    // 21 dot sprites (hero/enemy/civilian/scroll/etc.). Upload all
    // frames at mission start so `render_minimap` can blit them
    // without touching the resource manager each frame.
    match cursor_res.get_pictures(resource_ids::RHMAP_ITEMS) {
        Ok(pics) => {
            let surfaces: Vec<Option<crate::renderer::OwnedSurface>> = pics
                .iter()
                .map(|opt| opt.as_ref().map(|p| picture_to_surface(renderer, p)))
                .collect();
            tracing::info!("Loaded RHMAP_ITEMS: {} dot frames", surfaces.len());
            host.frontend
                .resources
                .mission_surfaces
                .replace_dots(renderer, surfaces);
        }
        Err(e) => {
            tracing::warn!("Failed to load RHMAP_ITEMS resource — minimap dots disabled: {e}");
        }
    }

    // ── Destination marker sprite (RHID_GROUND_FOCUS) ──
    // Loads a row of sprite frames from the global DEFAULT.RES
    // resource bank that get blitted at the click destination after a
    // move order is issued.
    match cursor_res.get_pictures(resource_ids::RHID_GROUND_FOCUS) {
        Ok(pics) => {
            let first_pic = pics.iter().find_map(|opt| opt.as_ref());
            let surfaces: Vec<(crate::renderer::OwnedSurface, u16, u16)> = pics
                .iter()
                .filter_map(|opt| {
                    opt.as_ref().map(|p| {
                        let id = picture_to_surface(renderer, p);
                        (id, p.width, p.height)
                    })
                })
                .collect();
            if surfaces.is_empty() {
                tracing::warn!("RHID_GROUND_FOCUS has no frames — destination marker disabled");
            } else {
                // The ground-mark sprite MoveBox is the auto-cropped
                // tight bounds of frame 0 (the dictionary-packed
                // sprite scans non-0x07C0 pixels and records the
                // cropped w/h plus a per-frame offset).  We store the
                // uncropped Picture; scan for the opaque bounds so the
                // half-diagonal lines up exactly.  Fall back to the
                // raw Picture size if the scan can't run (non-16-bit).
                let (cw, ch) = first_pic
                    .and_then(|p| p.opaque_bounds_16().map(|(_, _, cw, ch)| (cw, ch)))
                    .unwrap_or((surfaces[0].1, surfaces[0].2));
                tracing::info!(
                    "Loaded RHID_GROUND_FOCUS: {} frames, raw {}x{}, cropped {}x{}",
                    surfaces.len(),
                    surfaces[0].1,
                    surfaces[0].2,
                    cw,
                    ch,
                );
            }
            // Sprite-data + half-diagonal were absorbed into the
            // engine at construction via
            // `EngineArgs::ground_mark_sprite`; the GPU surfaces below
            // are pure host-side rendering state.
            host.frontend
                .resources
                .mission_surfaces
                .replace_ground_marks(
                    renderer,
                    surfaces.into_iter().map(|(id, _, _)| id).collect(),
                );
        }
        Err(e) => {
            tracing::warn!("Failed to load RHID_GROUND_FOCUS resource: {e}");
        }
    }

    timer.step("minimap + ground-focus sprites");

    // ── Selection mark renderer ──
    // Loads RHID_GROUND_SELECT (green idle) and RHID_GROUND_SELECT_SWORD
    // (red combat) sprites from DEFAULT.RES.
    let dynamic_ambience_visuals = host
        .application_context()
        .with_active_profile(|profile| profile.graphic_config.dynamic_ambience_visuals)
        .unwrap_or_else(|error| panic!("mission sprite setup requires an active profile: {error}"));
    let visual_shadow_color = if dynamic_ambience_visuals {
        engine.weather().night_color
    } else {
        engine.initial_mission_night_color()
    };
    let mut selection_mark_renderer = SelectionMarkRenderer::new();
    selection_mark_renderer.load(cursor_res, renderer, visual_shadow_color);
    timer.step("selection marks");

    // ── Swordfight mouse-trail renderer ──
    // Loads RHID_MOUSE_TRAIL, builds the 32-level alpha pattern table,
    // and creates one managed surface per alpha level.  Rendered each
    // frame while the player drags the left mouse button during a
    // swordfight.
    let mouse_trail_renderer = match cursor_res.get_picture(resource_ids::RHID_MOUSE_TRAIL, 0) {
        Ok(pic) => match MouseTrailRenderer::from_picture(pic, renderer) {
            Ok(trail) => {
                tracing::info!("Loaded RHID_MOUSE_TRAIL: pattern height {}", pic.height);
                Some(trail)
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to prepare RHID_MOUSE_TRAIL: {error:#} — swordfight trail disabled"
                );
                None
            }
        },
        Err(e) => {
            tracing::warn!("Failed to load RHID_MOUSE_TRAIL resource: {e}");
            None
        }
    };

    // ── Titbit renderer ──
    // Upload and retain the GPU textures for every titbit sprite row.
    let mut titbit_renderer = TitbitRenderer::new();
    titbit_renderer.load(cursor_res, renderer, visual_shadow_color);
    // Row frame counts were absorbed by the engine at construction via
    // `EngineArgs::titbit_row_frame_counts`; no post-load setter needed.
    timer.step("titbit renderer");

    // ── Portrait pictures (character faces in the bottom panel) ──
    // Portraits live in the same DEFAULT.RES file as cursors.
    let mut portrait_cache = PortraitCache::new();
    portrait_cache
        .load(
            cursor_res,
            renderer,
            host.preparation_files()
                .expect("portrait preparation requires resource authority"),
        )
        .expect("portrait artwork preparation failed");
    timer.step("portrait cache");

    // ── Localized character names ──
    // `text_res` (Data/Text/Level.res) was pre-attached in the loading-
    // screen block above.  The peasant firstname/surname *pool* is
    // loaded earlier and handed to `Engine::new` via `EngineArgs`; this
    // call assigns per-civilian display names on top of that pool.
    //
    // Display names belong to the cache, but the generated names also enter
    // hashed campaign identity. Share that non-rendering bootstrap operation
    // with true-headless; keep its graphical ordering at this exact point.
    let mut localized_names = load_localized_character_names(text_res);
    register_mission_peasant_names(&mut localized_names, engine, assets);
    portrait_cache.install_localized_names(localized_names);
    timer.step("localized + peasant names");
    timer.total();

    MissionSprites {
        cursor_renderer,
        selection_mark_renderer,
        mouse_trail_renderer,
        titbit_renderer,
        portrait_cache,
    }
}

/// Pre-compute the minimap corner-button widget setup from
/// `cursor_res`: corner-sprite dimensions plus the pixel-level hit
/// mask built from frame 1 of `RHMAP_CORNER` (the frame used for the
/// pixel-level opacity test). Returns `None` when the resource
/// is missing or has no pictures.
pub(super) fn extract_minimap_widget_setup(
    cursor_res: &mut ResourceManager,
) -> Option<engine_api::MinimapWidgetSetup> {
    if !cursor_res.has_resource(resource_ids::RHMAP_CORNER) {
        return None;
    }
    let metadata = cursor_res
        .get_picture_opacity_metadata(resource_ids::RHMAP_CORNER)
        .unwrap_or_else(|error| panic!("minimap engine picture metadata: {error:#}"));
    // An empty optional picture collection is supported. Metadata errors for
    // a present resource must not masquerade as the widget being absent.
    if metadata.iter().all(Option::is_none) {
        return None;
    }
    // Metadata already validates the image dimensions. Keep independent maxima
    // across the original slots without reading shipping image headers again.
    let (btn_w, btn_h) = metadata
        .iter()
        .flatten()
        .fold((0u16, 0u16), |(w, h), picture| {
            (w.max(picture.width), h.max(picture.height))
        });
    assert!(
        btn_w != 0 || btn_h != 0,
        "Data/Interface/DEFAULT.RES minimap corner dimensions: no valid sub-pictures"
    );
    let corner_size = ScreenSize::new(btn_w as f32, btn_h as f32);
    let button_hit_mask = metadata.into_iter().nth(1).flatten().map(|metadata| {
        metadata
            .into_hit_mask()
            .expect("validated minimap hit mask")
    });
    Some(engine_api::MinimapWidgetSetup {
        corner_size,
        button_hit_mask,
    })
}

/// Pre-compute the destination-marker (`RHID_GROUND_FOCUS`) sprite
/// metadata from `cursor_res`: half-diagonal (half-width, half-height)
/// in world pixel units plus the per-frame `(w, h)` sizes.
///
/// Returns `None` if the resource is missing or has no frames — the
/// caller passes that through to [`EngineArgs::ground_mark_sprite`]
/// and the engine leaves the marker disabled.
pub(super) fn extract_ground_mark_sprite_data(
    cursor_res: &mut ResourceManager,
) -> Option<engine_api::GroundMarkSpriteData> {
    robin_assets::interface_metadata::ground_mark_sprite_data(cursor_res)
        .unwrap_or_else(|error| panic!("ground marker engine picture metadata: {error:#}"))
}

/// Pre-compute titbit sprite-row frame counts from `cursor_res`.
/// Indexed by the engine's sprite-row discriminant.  Counts sub-pictures without
/// decoding them — enough for `TitbitManager::num_frames_for_row` to
/// drive animation.
pub(super) fn extract_titbit_row_frame_counts(cursor_res: &mut ResourceManager) -> Vec<u16> {
    robin_assets::interface_metadata::titbit_row_frame_counts(cursor_res)
        .unwrap_or_else(|error| panic!("titbit engine picture metadata: {error:#}"))
}

/// Register the generated Merry Men names before replay frame zero.
///
/// The name pool is already part of CPU-loaded LevelAssets in both drivers.
/// No portrait textures, window, resource archive, or host input is required.
/// Preserve the original auxiliary RNG seed, draw order and rejection budget:
/// registration changes campaign identity but never advances the engine RNG.
pub(super) fn register_mission_peasant_names(
    localized_names: &mut [Option<String>; robin_engine::character_kind::CharacterKind::COUNT],
    engine: &mut Engine,
    assets: &engine_api::LevelAssets,
) {
    use robin_engine::character_kind::CharacterKind;
    use robin_engine::sim_rng::{self, AuxiliaryRngSite};

    const MAX_ATTEMPTS: usize = 10;
    let firstnames = &assets.peasant_firstnames;
    let surnames = &assets.peasant_surnames;
    if firstnames.is_empty() || surnames.is_empty() {
        tracing::warn!(
            "Peasant name generation: no firstname/surname strings found ({}/{})",
            firstnames.len(),
            surnames.len(),
        );
        return;
    }
    let seed = engine.rng_seed();
    sim_rng::with_auxiliary_seed(AuxiliaryRngSite::PeasantNames, seed, |rng| {
        for kind in [
            CharacterKind::MerryManA,
            CharacterKind::MerryManB,
            CharacterKind::MerryManC,
        ] {
            let slot = kind.as_index();
            if localized_names[slot].is_some() {
                continue;
            }
            let mut generated = None;
            for _ in 0..MAX_ATTEMPTS {
                // Campaign identity must use the same draws on native and wasm32.
                let first = &firstnames[rng.u64(0..firstnames.len() as u64) as usize];
                let last = &surnames[rng.u64(0..surnames.len() as u64) as usize];
                let full = format!("{first} {last}");
                if !engine.is_peasant_name_registered(&full) {
                    engine
                        .advance_frame(
                            assets,
                            engine_api::SimulationFrameInput::new(vec![
                                engine_api::SimCommand::from(PlayerCommand::RegisterPeasantName {
                                    name: full.clone(),
                                }),
                            ])
                            .with_hourglass(false),
                        )
                        .expect("peasant-name registration admission");
                    generated = Some(full);
                    break;
                }
            }
            // Preserve the existing display-only exhausted-pool label. It is
            // never registered as a substitute for missing authored names.
            let display_name = generated.unwrap_or_else(|| "Misteryman".to_owned());
            tracing::info!("Peasant {kind:?} → {display_name:?}");
            localized_names[slot] = Some(display_name);
        }
    });
}

/// Run the CPU-only loading phase: sprite bank, campaign install +
/// level load (folded into a single `Engine::new` constructor call),
/// CLI-flag apply, mission script StartUp, Sherwood production
/// bonuses, and night/fog sprite-variant generation.
///
/// Constructs and returns the freshly-initialized `Engine`,
/// `LevelAssets`, and `DevState` — none of them are needed before this
/// phase, so the constructors live at the bottom of the loading
/// pipeline where all the required data is already in hand.
///
/// All slow work (map decompression, entity spawn, script init)
/// happens between `Initialize` and `Close` of the loading screen.
/// The `progress` closure captures the loading screen + event-pump
/// fields so each call ticks the bar and drains WM events.
/// CPU-loaded mission state consumed by the frontend-specific bootstrap.
///
/// This is a process-lifetime ownership seam, not persisted game state: the
/// decoded bitmaps are host upload scratch and `LevelAssets` contains runtime
/// caches. Consequently it deliberately does not implement serde.
pub(super) struct LoadedMissionCore {
    pub(super) engine: Engine,
    /// Exact campaign input captured immediately before `Engine::new`.
    /// Level initialization mutates the engine-owned campaign (notably,
    /// Sherwood clears `mission_team_indices` after spawning PCs), so replay
    /// reconstruction must use this pre-construction snapshot rather than
    /// `engine.campaign()`.
    pub(super) replay_campaign: Campaign,
    pub(super) assets: engine_api::LevelAssets,
    pub(super) dev: engine_api::DevState,
    pub(super) pre_decoded_background: Option<engine_api::level_loading::PreDecodedBackground>,
    pub(super) pre_decoded_minimap: Option<engine_api::level_loading::PreDecodedMinimap>,
    /// Still-running background+minimap decode for interactive missions
    /// (`TerrainJoinPoint::BeforePresentationUpload`): joined during frontend assembly, right before
    /// the GPU upload needs the pixels.
    pub(super) pending_terrain: Option<crate::level_loading_host::PendingTerrainDecode>,
    /// Grid dimensions the engine was constructed with, for the divergence
    /// assert at the deferred join.
    pub(super) bg_pixel_dims: (f32, f32),
    pub(super) pre_decoded_ambience_backgrounds: Vec<(
        engine_api::Ambiance,
        engine_api::level_loading::PreDecodedBackground,
    )>,
    pub(super) pre_decoded_ambience_minimaps: Vec<(
        engine_api::Ambiance,
        engine_api::level_loading::PreDecodedMinimap,
    )>,
    pub(super) engine_rng_seed: u64,
    pub(super) engine_sim_config: engine_api::SimConfig,
    /// Exact immutable-authority/prepared-input admission fixed before frame 0.
    pub(super) ranked_admission: super::leaderboard_runtime::PreparedRankedAdmission,
}

pub(super) struct MissionLoadError {
    pub(super) message: String,
    pub(super) campaign: Campaign,
}

impl MissionLoadError {
    fn new(campaign: Campaign, message: String) -> Self {
        Self { message, campaign }
    }
}

pub(crate) fn initial_sim_config(args: &crate::main_entry::MissionLaunch) -> engine_api::SimConfig {
    let mut sim_config = args.global_options.sim_config();
    sim_config.golden_eye |= args.goldeneye;
    if args.mission_start_map_output.is_some() {
        sim_config.fog_of_war = args.mission_start_fog_of_war;
    }
    if args
        .mission
        .as_deref()
        .is_some_and(robin_engine::level_data::hackable_level_exists)
    {
        // Hackable JSON levels are unscripted sandboxes, not legacy scripted
        // missions, so requiring an SCB StartUp class would reject them.
        sim_config.script_enabled = false;
    }
    sim_config
}

/// Loading feedback borrows presentation devices; it never owns mission state.
pub(super) type MissionLoadFeedback<'a> = (
    Option<&'a mut crate::window::GameWindow>,
    &'a mut Option<crate::loading_screen::LoadingScreenRenderer>,
);

/// CPU interface metadata needed before simulation construction. Dimensions
/// belong to minimap placement, not to the engine's terrain grid.
pub(super) struct MissionInterfaceSetup {
    pub(super) ground_mark: Option<engine_api::GroundMarkSpriteData>,
    pub(super) titbit_rows: Vec<u16>,
    pub(super) minimap_widget: Option<engine_api::MinimapWidgetSetup>,
    pub(super) screen_dimensions: (f32, f32),
}

/// Admission and deterministic options travel together until construction.
/// In particular, ranked authority cannot be reconstructed from diagnostics.
pub(super) struct MissionLaunchSetup {
    pub(super) rng_seed: u64,
    pub(super) sim_config: engine_api::SimConfig,
    pub(super) ranked_plan: super::leaderboard_runtime::RankedPreFramePlan,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum TerrainJoinPoint {
    BeforeHeadlessRuntime,
    BeforePresentationUpload,
}

/// Prepared CPU inputs, including the exact campaign allocation to recover if
/// ingestion fails. Only preparation can create this capability; construction
/// consumes it, so callers cannot run startup callbacks twice.
pub(super) struct PreparedMission {
    campaign: Campaign,
    assets: engine_api::LevelAssets,
    loaded: robin_engine::level_data::LoadedLevel,
    mission_name: Option<String>,
    level_directory: String,
    ground_mark_sprite: Option<engine_api::GroundMarkSpriteData>,
    titbit_row_frame_counts: Vec<u16>,
    launch: MissionLaunchSetup,
    presentation: PreparedMissionPresentation,
}

/// Host-only products carried across engine construction without joining the
/// terrain worker prematurely. This is not a save format or a second engine.
struct PreparedMissionPresentation {
    dev: engine_api::DevState,
    background: Option<engine_api::level_loading::PreDecodedBackground>,
    minimap: Option<engine_api::level_loading::PreDecodedMinimap>,
    pending_terrain: Option<crate::level_loading_host::PendingTerrainDecode>,
    bg_pixel_dims: (f32, f32),
    ambience_backgrounds: Vec<(
        engine_api::Ambiance,
        engine_api::level_loading::PreDecodedBackground,
    )>,
    ambience_minimaps: Vec<(
        engine_api::Ambiance,
        engine_api::level_loading::PreDecodedMinimap,
    )>,
    legacy_capture_scb: Option<assets_scb::ScbFile>,
    dynamic_visuals: bool,
    initial_shadow_key: u16,
    timer: PhaseTimer,
}

/// A live engine that has consumed ranked admission but has not yet attached
/// the host's viewport/legacy display state. Only attachment publishes a
/// LoadedMissionCore to frontend bootstrap.
pub(super) struct ConstructedMission {
    engine: Engine,
    replay_campaign: Campaign,
    assets: engine_api::LevelAssets,
    rng_seed: u64,
    sim_config: engine_api::SimConfig,
    ranked_admission: super::leaderboard_runtime::PreparedRankedAdmission,
    presentation: PreparedMissionPresentation,
}

// These linear runtime capabilities own authority, jobs, or non-persisted
// resource metadata. Serde provides an honest diagnostic label, never a route
// to fabricate a stage or silently default its skipped runtime fields.
macro_rules! diagnostic_stage_serde {
    ($($stage:ty),+ $(,)?) => {$(
        impl serde::Serialize for $stage {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(stringify!($stage))
            }
        }
        impl<'de> serde::Deserialize<'de> for $stage {
            fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                Err(serde::de::Error::custom(concat!(stringify!($stage), " is a runtime-only mission capability")))
            }
        }
    )+};
}
diagnostic_stage_serde!(
    MissionInterfaceSetup,
    MissionLaunchSetup,
    PreparedMission,
    PreparedMissionPresentation,
    ConstructedMission
);

pub(super) fn prepare_mission(
    feedback: &mut MissionLoadFeedback<'_>,
    host: &mut Host,
    game: &mut Game,
    campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    text_res: &mut ResourceManager,
    args: &crate::main_entry::MissionLaunch,
    interface: MissionInterfaceSetup,
    launch: MissionLaunchSetup,
) -> Result<PreparedMission, MissionLoadError> {
    let MissionInterfaceSetup {
        ground_mark: ground_mark_sprite,
        titbit_rows: titbit_row_frame_counts,
        minimap_widget,
        screen_dimensions: (screen_width, screen_height),
    } = interface;
    let MissionLaunchSetup {
        rng_seed: authoritative_rng_seed,
        sim_config: authoritative_sim_config,
        ranked_plan,
    } = launch;
    let files = match host.preparation_files() {
        Ok(files) => std::sync::Arc::new(files.snapshot()),
        Err(message) => return Err(MissionLoadError::new(campaign, message)),
    };
    let mut assets = engine_api::LevelAssets::new();
    let mission_name = campaign.current_mission_idx.map(|i| {
        campaign.missions[i]
            .profile(profiles)
            .mission_filename
            .clone()
    });
    assets.attachments.spellforge_runtime = host
        .scripting
        .lua_session
        .as_ref()
        .map(crate::lua_session::LuaSession::runtime);
    // Stamp the canonical loaded profile manager onto LevelAssets — the
    // engine reads profiles via `&assets.profile_manager` everywhere now
    // (Campaign no longer owns its own copy).
    assets.profile_manager = std::sync::Arc::new(profiles.clone());
    let mut dev = engine_api::DevState::new();
    dev.debug.surface_display = game.global_options.options().debug_surfaces;
    let mut timer = PhaseTimer::new("level+bank setup");

    // Load the mission binaries FIRST — they're cheap and they carry the
    // mission header (map filename + ambiance), which lets the slow
    // background-map decode start on a worker thread (native) while this
    // thread continues with the sprite bank, scripts, and minimap.
    let level_directory = game.global_options.level_directory.clone();
    let loaded_result = preparation::load_mission_binaries(
        host,
        &campaign,
        &assets.profile_manager,
        mission_name.as_deref(),
        &level_directory,
        &files,
        feedback,
    );
    let loaded = match loaded_result {
        Ok(loaded) => loaded,
        Err(message) => return Err(MissionLoadError::new(campaign, message)),
    };
    timer.step("mission binaries");
    let (event_pump, loading_screen) = feedback;

    let authored_initial_ambiance = engine_api::Ambiance::from_raw(loaded.mission.header.ambiance);
    let effective_initial_ambiance = loaded
        .mission
        .ambience_schedule
        .iter()
        .take_while(|cue| cue.at_seconds == 0)
        .last()
        .map_or(authored_initial_ambiance, |cue| cue.ambiance);
    let ambiance_dir = effective_initial_ambiance.directory().to_string();
    let map_name = loaded.mission.header.map_filename.clone();

    // Start the background-map + minimap decode (bzip2/JXL — the slowest
    // CPU-only step of level setup) off the loading path so it overlaps the
    // rest of this function and, for interactive missions, everything up to
    // the GPU upload during frontend assembly: `Engine::new` needs only the
    // pixel *dimensions* (probed cheaply from the map header below).
    // Native uses a dedicated thread; wasm uses the rayon worker pool when
    // the `wasm-threads` build initialized one, and otherwise decodes
    // synchronously right here (single-threaded browser fallback — the
    // progress closure keeps feeding the loading bar in that case).
    let pending_terrain = match preparation::start_mission_terrain(
        host,
        mission_name.as_deref(),
        &map_name,
        &ambiance_dir,
        &level_directory,
        &files,
    ) {
        Ok(pending) => pending,
        Err(message) => return Err(MissionLoadError::new(campaign, message)),
    };
    timer.step("terrain decode start");

    // Shipping may already have decoded pixels alongside the VQ tail. The
    // handoff above validates those immutable bytes against this reader;
    // remaining occlusion/minimap reads retain this preparation snapshot.
    // Resource environments clone/validate mission RHS and scripts. Start the
    // independent terrain job first so this work overlaps pixel decoding.
    let resources = match host.frontend.resources.shipping.as_ref() {
        Some(shipping) => match mission_name
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("shipping launch has no current mission"))
            .and_then(|name| shipping.mission_resource_environment(name))
        {
            Ok(resources) => resources,
            Err(error) => {
                return Err(MissionLoadError::new(
                    campaign,
                    format!("prepare shipping mission resources: {error:#}"),
                ));
            }
        },
        None => std::sync::Arc::new(
            engine_sprite_script::MissionResourceEnvironment::from_files(&files),
        ),
    };
    assets.sprite_scriptor = std::sync::Arc::new(
        engine_sprite_script::SpriteScriptor::with_resources(resources.clone()),
    );
    timer.step("mission resource environment");

    // Install the sprite bank — must happen before entity sprite
    // loading in initialize_for_mission. The parsed bank comes from the
    // application-owned asset cache (warmed on a background thread at
    // startup); the mission gets a clone so its runtime overlay sprites
    // don't leak into later missions. Bank sprites carry mmap spans,
    // not pixel data, so the clone is cheap.
    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Loading sprite bank...", 0.56);
    }
    {
        let cache_owner = match host.application_context().asset_cache() {
            Ok(cache) => cache,
            Err(message) => return Err(MissionLoadError::new(campaign, message)),
        };
        let asset_cache = cache_owner.get_or_build(
            host.frontend.resources.shipping.as_deref(),
            profiles,
            files.clone(),
        );
        match asset_cache.sprite_bank.as_ref() {
            Some(bank) => host
                .frontend
                .resources
                .install_frame_holder_before_publication(bank.clone()),
            None => tracing::warn!("Sprite bank unavailable in application asset cache"),
        }
        tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);
    }
    timer.step("sprite bank from application asset cache");
    let custom_sprites =
        match prepare_custom_character_dirs(&campaign, &assets.profile_manager, &files) {
            Ok(prepared) => prepared,
            Err(error) => return Err(MissionLoadError::new(campaign, error.to_string())),
        };
    if let Err(error) = custom_sprites.install(
        host.frontend
            .resources
            .frame_holder_before_publication_mut(),
        assets.sprite_scriptor_mut(),
    ) {
        return Err(MissionLoadError::new(campaign, error.to_string()));
    }
    timer.step("hackable character preload");
    // Publish the sprite-bank signature into LevelAssets so engine-side
    // sprite-script loaders can detect bank changes.
    assets.bank_signature = host.frontend.resources.frame_holder().signature();
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Initializing level...", 0.73);
    }

    // Engine LevelAssets already owns profile_manager (loaded at startup);
    // Campaign no longer has its own copy.

    // Hand the engine the parsed mission scripts it'll need.
    //
    // Engine doesn't depend on robin_assets, so it can't open `.scb`
    // files itself; the host parses them (preferring shipping, falling
    // back to disk for the current mission), decodes immutable bytecode,
    // and stores the programs in `LevelAssets` before level load.
    // Shipping programs have already crossed bytecode validation. Retain their
    // Arcs rather than cloning SCBs and decoding a second runtime copy.
    let legacy_capture_scb = match preparation::prepare_mission_programs(
        &resources,
        &mut assets,
        mission_name.as_deref(),
        authoritative_sim_config.script_enabled,
        args.mission_start_legacy_save.is_some(),
    ) {
        Ok(capture) => capture,
        Err(message) => return Err(MissionLoadError::new(campaign, message)),
    };
    timer.step("mission scripts");

    // Initialize Game's per-mission state from the campaign before we
    // hand it off to the engine.
    game.initialize_for_mission(&campaign, &assets.profile_manager);

    // Construct the engine with campaign ownership + level load folded
    // in.  The old split constructor followed by
    // `initialize_from_campaign` + `initialize` sequence collapses to
    // this single call.  Mission script was already loaded inside
    // `load_level()` → `load_mission_script()` so the level loader
    // does not re-load it.
    if let Err(message) = preparation::prepare_localized_names(&mut assets, text_res) {
        return Err(MissionLoadError::new(campaign, message));
    }

    // Run the single-threaded wasm fallback decode here — the exact point
    // the old synchronous branch used — so the loading bar behaves the same
    // when no worker pool exists. Threaded decodes pass through untouched.
    let pending_terrain = {
        let mut sync_progress = |u: assets_frame_holder::ProgressUpdate| match u {
            assets_frame_holder::ProgressUpdate::Tick(d) => {
                tick_progress(loading_screen, event_pump.as_deref_mut(), d);
            }
            assets_frame_holder::ProgressUpdate::Phase(text, _local) => {
                if let Some(ls) = loading_screen.as_mut() {
                    ls.set_status(text, LOADING_MAP_DECODE_PROGRESS);
                }
            }
        };
        pending_terrain.decode_inline_if_pending(&mut sync_progress)
    };

    // `Engine::new` needs the background bitmap's pixel dimensions to size
    // the fast-find grid. They are probed cheaply from the map header while
    // the decode keeps running; when the probe cannot say (missing/corrupt
    // map, or no map at all) the decode outcome is resolved right here so
    // the existing pre-engine error path reports it.
    let mut pre_decoded_bg: Option<engine_api::level_loading::PreDecodedBackground> = None;
    let mut pre_decoded_mm: Option<engine_api::level_loading::PreDecodedMinimap> = None;
    let install_decoded_terrain =
        |decoded: crate::level_loading_host::DecodedTerrainBitmaps,
         bg: &mut Option<engine_api::level_loading::PreDecodedBackground>,
         mm: &mut Option<engine_api::level_loading::PreDecodedMinimap>|
         -> Result<(f32, f32), String> {
            let background = decoded.background?;
            let dims = background
                .as_ref()
                .map(|b| (b.width as f32, b.height as f32))
                .unwrap_or((0.0, 0.0));
            *bg = background;
            *mm = decoded.minimap;
            Ok(dims)
        };
    let (bg_pixel_dims, bg_pending) = match pending_terrain.try_take_ready() {
        Ok(decoded) => {
            match install_decoded_terrain(decoded, &mut pre_decoded_bg, &mut pre_decoded_mm) {
                Ok(dims) => (dims, None),
                Err(message) => return Err(MissionLoadError::new(campaign, message)),
            }
        }
        Err(pending) => match pending.known_dimensions().or_else(|| {
            crate::level_loading_host::probe_background_map_dims_with_files(
                &map_name,
                &ambiance_dir,
                &level_directory,
                host.frontend.resources.shipping.as_deref(),
                &files,
            )
        }) {
            Some((w, h)) => ((w as f32, h as f32), Some(pending)),
            None => {
                let decoded = pending.join_now_or_redecode(&mut |_| {});
                match install_decoded_terrain(decoded, &mut pre_decoded_bg, &mut pre_decoded_mm) {
                    Ok(dims) => (dims, None),
                    Err(message) => return Err(MissionLoadError::new(campaign, message)),
                }
            }
        },
    };
    timer.step("background map dims");

    // Populate every simulation-visible audio dependency before preparing
    // the engine. PreparedMissionInputs seals LevelAssets immediately, so a
    // post-construction host reread would leave replay identity incomplete.
    let dynamic_ambience_enabled = if host.transport.net().is_some() {
        host.transport
            .mission_sim_config()
            .unwrap_or_else(|| {
                panic!("active multiplayer transport is missing its Welcome SimConfig")
            })
            .enable_dynamic_ambience
    } else {
        authoritative_sim_config.enable_dynamic_ambience
    };
    let mut ambiance_mask = effective_initial_ambiance.to_bitmask();
    if dynamic_ambience_enabled {
        for cue in &loaded.mission.ambience_schedule {
            ambiance_mask |= cue.ambiance.to_bitmask();
        }
    }
    if let Err(message) = preparation::prepare_deterministic_audio(
        host,
        &mut assets,
        &loaded,
        &campaign,
        profiles,
        &files,
        ambiance_mask,
    ) {
        return Err(MissionLoadError::new(campaign, message));
    }
    timer.step("deterministic audio metadata");

    // Decode every additional authored ambience once during mission loading.
    // Feature 14's active-mission-only prefetch remains scoped to this load;
    // nothing is retained in the process cache for unrelated missions.
    let (pre_decoded_ambience_backgrounds, pre_decoded_ambience_minimaps) =
        match preparation::prepare_scheduled_ambiances(
            host,
            &loaded,
            effective_initial_ambiance,
            bg_pixel_dims,
            &level_directory,
            &files,
            &mut (event_pump.as_deref_mut(), &mut **loading_screen),
        ) {
            Ok(decoded) => decoded,
            Err(message) => return Err(MissionLoadError::new(campaign, message)),
        };

    timer.step("scheduled ambiance preparation");

    // Resolve the engine's initial RNG seed before construction so
    // `Engine::new` is the only site that touches RNG state during
    // setup. Campaign selection has already advanced the single-player /
    // replay sequence. A negotiated multiplayer mission seed remains the
    // authority for a network mission.
    if let Some(mm) = minimap_widget {
        host.frontend
            .presentation
            .engine_display
            .setup_minimap_widget(
                engine_coordinates::ScreenPoint::new(screen_width - 83.0, 38.0),
                mm.corner_size,
                mm.button_hit_mask,
                screen_width,
                screen_height,
            );
    }

    let (rng_seed, sim_config) = if host.transport.net().is_some() {
        let rng_seed = host.transport.mission_seed().unwrap_or_else(|| {
            panic!("active multiplayer transport is missing its Welcome mission seed")
        });
        let sim_config = host.transport.mission_sim_config().unwrap_or_else(|| {
            panic!("active multiplayer transport is missing its Welcome SimConfig")
        });
        (rng_seed, sim_config)
    } else {
        (authoritative_rng_seed, authoritative_sim_config)
    };

    // Generate sprite variants once through the same helper ordinary runtime
    // rebinding uses, then publish the immutable hit-testing generation before
    // mission inputs are sealed.
    let dynamic_visuals = host
        .application_context()
        .with_active_profile(|profile| profile.graphic_config.dynamic_ambience_visuals)
        .unwrap_or_else(|error| {
            panic!("mission presentation preparation requires an active profile: {error}")
        });
    let presentation_initial_ambiance = if dynamic_visuals {
        effective_initial_ambiance
    } else {
        authored_initial_ambiance
    };
    crate::level_loading_host::initialize_sprite_variants_for_ambiance(
        host,
        presentation_initial_ambiance,
        sim_config.bypass_fog_sprites_crash,
    );
    timer.step("initial sprite variants");
    let (night_r, night_g, night_b) = presentation_initial_ambiance.night_color_rgb();
    let initial_shadow_key = robin_util::color::rgb565(night_r, night_g, night_b);
    host.frontend
        .resources
        .frame_holder_before_publication_mut()
        .apply_arno_law(initial_shadow_key);
    assets.attachments.pixel_opacity = Some(host.frontend.resources.publish_frame_holder_opacity());
    timer.step("initial sprite shadow and opacity publication");

    Ok(PreparedMission {
        campaign,
        assets,
        loaded,
        mission_name,
        level_directory,
        ground_mark_sprite,
        titbit_row_frame_counts,
        launch: MissionLaunchSetup {
            rng_seed,
            sim_config,
            ranked_plan,
        },
        presentation: PreparedMissionPresentation {
            dev,
            background: pre_decoded_bg,
            minimap: pre_decoded_mm,
            pending_terrain: bg_pending,
            bg_pixel_dims,
            ambience_backgrounds: pre_decoded_ambience_backgrounds,
            ambience_minimaps: pre_decoded_ambience_minimaps,
            legacy_capture_scb,
            dynamic_visuals,
            initial_shadow_key,
            timer,
        },
    })
}

impl PreparedMission {
    pub(super) fn construct_engine(
        self,
        args: &crate::main_entry::MissionLaunch,
        feedback: &mut MissionLoadFeedback<'_>,
    ) -> Result<ConstructedMission, MissionLoadError> {
        let Self {
            campaign,
            mut assets,
            loaded,
            mission_name,
            level_directory,
            ground_mark_sprite,
            titbit_row_frame_counts,
            launch,
            mut presentation,
        } = self;
        #[cfg(not(all(feature = "projection-export", not(target_arch = "wasm32"))))]
        let _ = (args, &mission_name);
        let MissionLaunchSetup {
            rng_seed,
            sim_config,
            ranked_plan,
        } = launch;
        let (event_pump, loading_screen) = feedback;
        let bg_pixel_dims = presentation.bg_pixel_dims;
        // This is the only point at which setup transfers campaign ownership.
        // Every fallible file/decode step above borrows the session campaign, and
        // the preserving constructor returns the exact allocation on ingestion
        // failure.
        let replay_campaign = campaign.clone();
        let (engine, ranked_admission) = {
            let mut progress = |delta: f32| {
                tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
            };
            let engine_args = engine_api::EngineArgs {
                campaign,
                level: engine_api::LevelLoadArgs {
                    assets: &mut assets,
                    level_directory: &level_directory,
                    progress: &mut progress,
                    loaded,
                    bg_pixel_dims,
                },
                ground_mark_sprite,
                titbit_row_frame_counts,
                rng_seed,
                original_rng_replay: None,
                sim_config,
            };
            let needs_projection = matches!(
                ranked_plan,
                super::leaderboard_runtime::RankedPreFramePlan::Authority(_)
            );
            #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
            let needs_projection = needs_projection || args.simulation_content_export.is_some();
            if !needs_projection {
                // Ordinary play has no consumer for the verification projection.
                // Construct the same engine without cloning/serializing its inputs
                // or hashing the sprite opacity surface.
                let super::leaderboard_runtime::RankedPreFramePlan::BrowseOnly { reason } =
                    ranked_plan
                else {
                    unreachable!("authority sessions require a prepared projection");
                };
                let engine =
                    Engine::new_preserving_campaign(engine_args).map_err(|(error, campaign)| {
                        MissionLoadError::new(campaign, format!("Level init failed: {error}"))
                    })?;
                (
                    engine,
                    super::leaderboard_runtime::PreparedRankedAdmission::BrowseOnly { reason },
                )
            } else {
                match Engine::prepare_preserving_campaign(engine_args) {
                    Ok(prepared) => {
                        #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
                        if let Some(request) = args.simulation_content_export.as_ref() {
                            let exact_mission = match mission_name.as_deref() {
                                Some(mission) => mission,
                                None => {
                                    let campaign = Engine::from_prepared(prepared).into_campaign();
                                    return Err(MissionLoadError::new(
                                        campaign,
                                        "simulation-content export has no prepared mission identity"
                                            .to_owned(),
                                    ));
                                }
                            };
                            let components = prepared
                                .static_projection()
                                .components()
                                .iter()
                                .map(|component| {
                                    crate::official_projection_export::CanonicalProjectionComponent {
                                        document: component.document.clone(),
                                        canonical_bytes: component.canonical_bytes.clone(),
                                        sha256: component.sha256,
                                    }
                                })
                                .collect::<Vec<_>>();
                            if let Err(error) =
                            crate::official_projection_export::write_simulation_content_projection(
                                request,
                                exact_mission,
                                &robin_engine::simulation_inputs::SIMULATION_CONTENT_COMPONENT_ORDER_V1,
                                &components,
                            )
                        {
                            let campaign = Engine::from_prepared(prepared).into_campaign();
                            return Err(MissionLoadError::new(
                                campaign,
                                format!("simulation-content export failed: {error:#}"),
                            ));
                        }
                        }
                        ranked_plan.consume_prepared(prepared)
                    }
                    Err((error, campaign)) => {
                        return Err(MissionLoadError::new(
                            campaign,
                            format!("Level init failed: {error}"),
                        ));
                    }
                }
            }
        };
        presentation.timer.step("engine construction");
        Ok(ConstructedMission {
            engine,
            replay_campaign,
            assets,
            rng_seed,
            sim_config,
            ranked_admission,
            presentation,
        })
    }
}

impl ConstructedMission {
    pub(super) fn attach_presentation(
        self,
        host: &mut Host,
        args: &crate::main_entry::MissionLaunch,
        feedback: &mut MissionLoadFeedback<'_>,
        terrain_join: TerrainJoinPoint,
    ) -> Result<LoadedMissionCore, MissionLoadError> {
        let Self {
            mut engine,
            replay_campaign,
            assets,
            rng_seed,
            sim_config,
            ranked_admission,
            presentation,
        } = self;
        let PreparedMissionPresentation {
            mut dev,
            background: mut pre_decoded_bg,
            minimap: mut pre_decoded_mm,
            pending_terrain: bg_pending,
            bg_pixel_dims,
            ambience_backgrounds: pre_decoded_ambience_backgrounds,
            ambience_minimaps: pre_decoded_ambience_minimaps,
            legacy_capture_scb,
            dynamic_visuals,
            initial_shadow_key,
            mut timer,
        } = presentation;
        let (event_pump, loading_screen) = feedback;

        // Engine construction ran on probed header dimensions. Callers that
        // cannot defer (true-headless bootstrap) collect the decoded pixels now
        // — a decode failure still fails the mission load (via the replay
        // campaign clone — `campaign` moved into the engine), and diverging
        // dimensions would corrupt the already-built grid, so that is a hard
        // error rather than a fallback. Interactive callers instead carry the
        // pending decode into frontend assembly and join right before the GPU
        // upload, so the decode also overlaps audio setup, descriptors, HUD
        // fonts, and renderer bring-up.
        let mut pending_terrain_out = None;
        if let Some(pending) = bg_pending {
            if matches!(terrain_join, TerrainJoinPoint::BeforePresentationUpload) {
                pending_terrain_out = Some(pending);
            } else {
                let decoded = pending.join_blocking();
                let background = match decoded.background {
                    Ok(background) => background,
                    Err(message) => return Err(MissionLoadError::new(replay_campaign, message)),
                };
                if let Some(bg) = background.as_ref() {
                    assert_eq!(
                        (bg.width as f32, bg.height as f32),
                        bg_pixel_dims,
                        "background map header dimensions diverge from decoded bitmap"
                    );
                }
                pre_decoded_bg = background;
                pre_decoded_mm = decoded.minimap;
                timer.step("background map join");
            }
        }

        if let Some(save_bytes) = args.mission_start_legacy_save.as_ref() {
            let mission_scb = legacy_capture_scb
                .as_ref()
                .expect("legacy frame-zero capture lost its mission script");
            let save = robin_engine::legacy_save::initialized::decode_initialized_v48_save(
                save_bytes.clone(),
                "frame-zero parity capture",
                &engine,
                &assets,
                mission_scb,
                &robin_engine::legacy_save::body::LegacySaveBodyLimits::default(),
            )
            .map_err(|error| {
                MissionLoadError::new(
                    replay_campaign.clone(),
                    format!("decode frame-zero Original save: {error}"),
                )
            })?;
            let loaded_host =
                robin_engine::legacy_save::adopt_engine::adopt_known_linux_v48_replay(
                    &mut engine,
                    &assets,
                    &save,
                )
                .map_err(|error| {
                    MissionLoadError::new(
                        replay_campaign.clone(),
                        format!("adopt frame-zero Original save: {error}"),
                    )
                })?;
            loaded_host.apply_display_to(&mut host.frontend.presentation.engine_display);
            host.frontend
                .set_selected_view_element(loaded_host.selected_view_element());
            tracing::info!("adopted Original v48 save for frame-zero viewport capture");
        }
        if rng_seed != 0 {
            tracing::info!(seed = rng_seed, "engine RNG seeded at construction");
        }
        host.frontend
            .viewport
            .set_level_size(bg_pixel_dims.0, bg_pixel_dims.1);

        // Multiplayer snapshots are cached after the host seat is bootstrapped
        // and then refreshed at the same sampling point as state hashes. That
        // gives early handshakes a frame-0 snapshot while late joiners still
        // adopt a hash-aligned state.

        // GoldenEye is now applied inside `Engine::new` via
        // `EngineArgs::goldeneye` — no post-construction dispatch.
        dev.debug.all_view_cones = args.view_cones;
        tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

        if let Some(ls) = loading_screen.as_mut() {
            ls.set_status(
                "Generating sprite variants...",
                LOADING_SPRITE_VARIANTS_PROGRESS,
            );
        }

        // Variant dictionaries and opacity were generated with the effective
        // initial ambiance before publication. Engine preparation must resolve the
        // same key: rebinding here would copy the entire published sprite bank just
        // to apply the color it already has. Runtime ambiance changes still use the
        // synchronized COW rebind path.
        let engine_shadow_key = if dynamic_visuals {
            engine.weather().night_color
        } else {
            engine.initial_mission_night_color()
        };
        assert_eq!(
            engine_shadow_key, initial_shadow_key,
            "engine initial shadow key diverged from the published sprite generation"
        );
        tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);
        timer.step("sprite variants + Arno's Law");
        timer.total();

        Ok(LoadedMissionCore {
            engine,
            replay_campaign,
            assets,
            dev,
            pre_decoded_background: pre_decoded_bg,
            pre_decoded_minimap: pre_decoded_mm,
            pending_terrain: pending_terrain_out,
            bg_pixel_dims,
            pre_decoded_ambience_backgrounds,
            pre_decoded_ambience_minimaps,
            engine_rng_seed: rng_seed,
            engine_sim_config: sim_config,
            ranked_admission,
        })
    }
}

/// Install the local deterministic seat and publish the host's authoritative
/// frame-zero state. This admission setup is shared by interactive and true-
/// headless missions and deliberately has no renderer, UI, input-device, or
/// audio dependency.
pub(super) fn setup_local_seat_and_multiplayer_snapshot(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    args: &crate::main_entry::MissionLaunch,
) {
    // Clients adopt the server snapshot (which already includes seat 0) and
    // receive their own ConnectSeat through the server-ordered input stream.
    if args.connect.is_some() {
        return;
    }

    let nickname = args.mp_nickname.clone();
    engine
        .advance_frame(
            assets,
            engine_api::SimulationFrameInput::new(vec![engine_api::SimCommand::from(
                PlayerCommand::ConnectSeat {
                    player_id: host.transport.local_seat(),
                    nickname,
                },
            )])
            .with_hourglass(false),
        )
        .expect("bootstrap ConnectSeat admission");
    tracing::info!(
        seat = ?host.transport.local_seat(),
        "bootstrap ConnectSeat applied to local engine",
    );
    if let Some(net) = host.transport.net() {
        match net
            .publish_initial_snapshot(0, engine)
            .and_then(|()| net.send_ready_to_sim(0))
        {
            Ok(()) => tracing::info!("multiplayer: cached and published frame-0 host snapshot"),
            Err(error) => {
                tracing::error!(%error, "multiplayer initial snapshot publication failed")
            }
        }
    }
}

/// Build `ThreadedInput` + `InputTranslator`, load the active profile's
/// key bindings into both the host cache and the translator, push the
/// `DisplayMap` accelerator into the engine minimap, center the camera
/// on the first PC, and grab the mouse for edge-scrolling.
///
/// Bundles the pre-loop actions performed during mission initialization.
pub(super) fn setup_input_and_camera(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    args: &crate::main_entry::MissionLaunch,
    window_width: u32,
    window_height: u32,
    mission_idx: usize,
) -> (ThreadedInput, InputTranslator) {
    let mut threaded_input = ThreadedInput::new();
    threaded_input.set_clipping(robin_engine::coordinates::ScreenBBox::from_coords(
        0.0,
        0.0,
        window_width as f32,
        window_height as f32,
    ));

    // Playback and legacy-save parity capture must not admit post-port live
    // planning input. Recorded QueueQuickAction commands remain authoritative
    // and still replay through the deterministic command stream.
    if args.replay_data.is_some()
        || args.replay.is_some()
        || args.mission_start_legacy_save.is_some()
        || args.mission_start_viewport_capture
    {
        host.frontend.force_planning_off_for_session();
    }

    // Host construction snapshots the active profile's bindings from the
    // ApplicationContext. The Original copies that active config at this
    // exact input-translator boundary (`ReflectActiveKeyConfig`).
    let mut input_translator = InputTranslator::new(
        window_width as f32,
        window_height as f32,
        host.frontend.preferences().key_config(),
    );

    // The `DisplayMap` minimap accelerator is stored host-side on
    // `host.frontend.minimap_fast_key` — the game loop reads it out to emit a
    // minimap-toggle command on key release.  Rebind via the pause
    // menu updates the same host field.
    host.frontend.minimap_fast_key = input_translator.get_binding(GameKey::DisplayMap);

    // Install the four HUD-adjacent edge-scroll dead-zone strips so
    // edge-scroll ignores the cursor while it's parked on or beside
    // the bottom HUD panels.
    input_translator.install_hud_dead_zones();

    tracing::info!(
        "Entering mission game loop ({} entities, mission idx: {})",
        engine.entity_count(),
        mission_idx,
    );

    // Bootstrap the local seat: apply `ConnectSeat(local_seat,
    // nickname)` directly to the engine — setup, not gameplay
    // input.  This creates the host's `SeatState`, defaults the
    // CameraState, and centers `view_position` on the first PC's
    // world coords (handler in `engine/commands.rs`).  Going
    // through `dispatch_local_command` would wire-route in MP and
    // make the local engine miss its own seat at frame 0; instead,
    // setup-state is what `InitialSnapshot` captures and ships to
    // joining peers, so they adopt a state that already includes
    // the host's seat.
    //
    // Only SP / server processes bootstrap directly:
    //
    // - **SP (`--connect == None && --server == None`)**: net is
    //   None, just create the seat locally.
    // - **Server (`--server`)**: snapshot is taken AFTER this so
    //   joining clients adopt an engine that already has seat 0.
    //
    // **Clients (`--connect`)** intentionally do NOT bootstrap —
    // they adopt the server's `InitialSnapshot` (which already has
    // the server's seat) and dispatch their own `ConnectSeat` as a
    // per-frame input later, landing at `sim_frame +
    // INPUT_DELAY_FRAMES` symmetrically on every machine.
    //
    // **Headless dedicated server** is a future scope: a `--server`
    // process without a local seat.  Today every `--server` is
    // also a player — keeping that path intact below.
    setup_local_seat_and_multiplayer_snapshot(engine, host, assets, args);
    let camera_focus = engine
        .pc_ids()
        .first()
        .and_then(|&pc_id| engine.get_entity(pc_id))
        .map(|entity| entity.element_data().position_map())
        .or_else(|| {
            spectator_actor_centroid(engine.active_entity_positions().filter_map(
                |(id, position)| {
                    engine
                        .get_entity(id)
                        .is_some_and(|entity| entity.human_data().is_some())
                        .then_some(position)
                },
            ))
        });
    if let Some(position) = camera_focus {
        host.frontend.viewport.center_on_point(position);
    }

    (threaded_input, input_translator)
}

fn spectator_actor_centroid(
    positions: impl Iterator<Item = engine_coordinates::MapPoint>,
) -> Option<engine_coordinates::MapPoint> {
    let (sum_x, sum_y, count) = positions.fold((0.0, 0.0, 0_u32), |acc, position| {
        (acc.0 + position.x, acc.1 + position.y, acc.2 + 1)
    });
    (count != 0)
        .then(|| engine_coordinates::MapPoint::new(sum_x / count as f32, sum_y / count as f32))
}

/// Initialize the mission mixer and optionally play loading-screen menu
/// music. Replay startup skips that unrelated track; `prepare_audio` still
/// enters mission mode and supplies the recorded mission's normal audio.
pub(super) fn init_audio_backend(
    host: &mut Host,
    game: &Game,
    play_loading_menu_music: bool,
) -> Option<KiraAudioBackend> {
    if !game.global_options.sound_enabled {
        tracing::info!("sound disabled via `-NOSOUND`; skipping audio backend init");
        return None;
    }
    let mut audio_backend = match KiraAudioBackend::new_for_application(
        host.application_context(),
        &game.global_options.sound_directory,
        NUM_CHANNELS,
    ) {
        Ok(backend) => Some(backend),
        Err(e) => {
            tracing::warn!("Failed to initialize audio: {}. Sound disabled.", e);
            None
        }
    };
    if let Some(backend) = audio_backend.as_mut() {
        host.audio
            .sound
            .set_music_directory(&game.global_options.music_directory);
        // Read the active profile's 3D-sound preference and forward
        // it to the sound manager.  The backend grants the request
        // only when `can_3d_sound()` is true; the kira backend never
        // is, so this lands in 2D with a non-fatal warning.
        let sound_config = host
            .application_context()
            .with_active_profile(|profile| profile.sound_config)
            .unwrap_or_else(|error| panic!("audio setup requires an active profile: {error}"));
        let want_3d = sound_config.sound_3d;
        if let Err(e) = host.audio.sound.initialize(backend, want_3d) {
            tracing::warn!("Sound manager init failed: {}", e);
        }
        // Apply volumes before set_mode(Menu) so menu music isn't silent.
        host.audio.sound.apply_volumes(&sound_config);
        if play_loading_menu_music {
            host.audio.sound.set_mode(SoundMode::Menu, backend);
        }
    }
    audio_backend
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_soldier_speech_preload_uses_the_spawn_profile() {
        let mut profiles = engine_profiles::ProfileManager::new();
        profiles.missions.push(Default::default());
        profiles.soldiers = vec![
            engine_profiles::SoldierProfile {
                filename: "Placeholder".into(),
                exclamation_id: 0x1111_0000,
                ..Default::default()
            },
            engine_profiles::SoldierProfile {
                filename: "Fabri18 OfficerGreen Officer".into(),
                exclamation_id: 0x464f_0000,
                ..Default::default()
            },
        ];
        let mut campaign = Campaign::new();
        let mut mission = robin_engine::mission::Mission::new();
        mission.profile_idx = Some(0);
        campaign.missions.push(mission);
        campaign.current_mission_idx = Some(0);
        let mut loaded = robin_engine::level_data::LoadedLevel::hackable_from_json(
            br#"{
            "map_filename":"Test", "spawn":[50,50], "spawn_player":false,
            "walkable_polygon":[[0,0],[100,0],[100,100]],
            "soldiers":[{"position":[20,20],"profile":"fabri18_officergreen_officer","allegiance":2}]
        }"#,
        )
        .unwrap();
        assert_eq!(loaded.mission.soldiers[0].profile_number, 0);
        let mut audio = engine_api::LevelAudioAssets::default();
        audio.required_exclamation_ids =
            required_mission_exclamation_ids(&loaded, &campaign, &profiles).unwrap();
        assert_eq!(audio.required_exclamation_ids, [0x464f_0000].into());
        let timing = robin_engine::audio_durations::AudioDurations {
            version: 1,
            locale: "en-US".into(),
            samples_ms: [("officer.wav".into(), 1415)].into(),
            speech_groups: [(0x464f_003d, vec!["officer.wav".into()])].into(),
        };
        timing.populate(&mut audio, &profiles).unwrap();
        assert_eq!(
            robin_engine::audio_durations::speech_duration_frames(
                audio.speech_timing_catalog(),
                0x464f_003d,
                -1
            )
            .unwrap(),
            36
        );
        loaded.mission.soldiers[0].profile_id = Some("missing".into());
        assert!(required_mission_exclamation_ids(&loaded, &campaign, &profiles).is_err());
        loaded.mission.soldiers[0].profile_id = None;
        assert_eq!(
            required_mission_exclamation_ids(&loaded, &campaign, &profiles).unwrap(),
            [0x1111_0000].into()
        );
        loaded.mission.soldiers[0].profile_number = 99;
        assert!(required_mission_exclamation_ids(&loaded, &campaign, &profiles).is_err());
    }

    fn prepared_stage_fixture() -> PreparedMission {
        let mut assets = LevelAssets::new();
        let fixture = Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
            .expect("fixture campaign");
        let loaded = robin_engine::level_data::LoadedLevel::empty();
        let ambiance = engine_api::Ambiance::from_raw(loaded.mission.header.ambiance);
        let (r, g, b) = ambiance.night_color_rgb();
        PreparedMission {
            campaign: fixture.campaign().clone(),
            assets,
            loaded,
            mission_name: Some("stage fixture".into()),
            level_directory: String::new(),
            ground_mark_sprite: None,
            titbit_row_frame_counts: Vec::new(),
            launch: MissionLaunchSetup {
                rng_seed: 0x1234,
                sim_config: engine_api::SimConfig {
                    script_enabled: false,
                    golden_eye: true,
                    ..Default::default()
                },
                ranked_plan: super::super::leaderboard_runtime::RankedPreFramePlan::browse_only(
                    "stage fixture",
                ),
            },
            presentation: PreparedMissionPresentation {
                dev: Default::default(),
                background: None,
                minimap: None,
                pending_terrain: Some(crate::level_loading_host::PendingTerrainDecode::Ready(
                    crate::level_loading_host::DecodedTerrainBitmaps {
                        background: Ok(None),
                        minimap: None,
                    },
                )),
                bg_pixel_dims: (0.0, 0.0),
                ambience_backgrounds: Vec::new(),
                ambience_minimaps: Vec::new(),
                legacy_capture_scb: None,
                dynamic_visuals: true,
                initial_shadow_key: robin_util::color::rgb565(r, g, b),
                timer: PhaseTimer::new("stage fixture"),
            },
        }
    }

    #[test]
    fn production_stages_preserve_launch_and_choose_the_terrain_join_point() {
        for join in [
            TerrainJoinPoint::BeforeHeadlessRuntime,
            TerrainJoinPoint::BeforePresentationUpload,
        ] {
            let prepared = prepared_stage_fixture();
            let campaign_before = serde_json::to_value(&prepared.campaign).unwrap();
            let args = crate::main_entry::MissionLaunch {
                config: crate::main_entry::CliArgs {
                    view_cones: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut loading_screen = None;
            let mut feedback = (None, &mut loading_screen);
            let constructed = prepared
                .construct_engine(&args, &mut feedback)
                .unwrap_or_else(|error| panic!("construct stage: {}", error.message));
            assert_eq!(constructed.rng_seed, 0x1234);
            assert_eq!(constructed.engine.rng_seed(), 0x1234);
            assert_eq!(constructed.engine.sim_config(), constructed.sim_config);
            assert!(constructed.engine.sim_config().golden_eye);
            assert_eq!(
                serde_json::to_value(&constructed.replay_campaign).unwrap(),
                campaign_before
            );
            assert!(
                constructed.presentation.pending_terrain.is_some(),
                "construction must not join presentation work"
            );
            assert!(matches!(
                constructed.ranked_admission,
                super::super::leaderboard_runtime::PreparedRankedAdmission::BrowseOnly { .. }
            ));
            let mut host = Host::scratch(1024.0, 768.0);
            let loaded = constructed
                .attach_presentation(&mut host, &args, &mut feedback, join)
                .unwrap_or_else(|error| panic!("attach stage: {}", error.message));
            assert_eq!(
                loaded.pending_terrain.is_some(),
                matches!(join, TerrainJoinPoint::BeforePresentationUpload)
            );
            assert_eq!(loaded.engine_rng_seed, 0x1234);
            assert_eq!(loaded.engine.rng_seed(), 0x1234);
            assert!(loaded.dev.debug.all_view_cones);
        }
    }

    #[test]
    fn construction_stage_recovers_campaign_on_ingestion_failure() {
        let mut prepared = prepared_stage_fixture();
        prepared.launch.sim_config.script_enabled = true;
        let campaign_before = serde_json::to_value(&prepared.campaign).unwrap();
        let allocation_before = prepared.campaign.missions.as_ptr();
        let mut loading_screen = None;
        let error = match prepared
            .construct_engine(&Default::default(), &mut (None, &mut loading_screen))
        {
            Ok(_) => panic!("missing required mission script must reject construction"),
            Err(error) => error,
        };
        assert!(
            error.message.contains("Level init failed"),
            "{}",
            error.message
        );
        assert_eq!(
            serde_json::to_value(&error.campaign).unwrap(),
            campaign_before
        );
        assert_eq!(error.campaign.missions.as_ptr(), allocation_before);
    }

    #[test]
    fn headless_attachment_reports_decode_failure_without_fabricating_terrain() {
        let mut prepared = prepared_stage_fixture();
        prepared.presentation.pending_terrain =
            Some(crate::level_loading_host::PendingTerrainDecode::Ready(
                crate::level_loading_host::DecodedTerrainBitmaps {
                    background: Err("terrain fixture failed".into()),
                    minimap: None,
                },
            ));
        let campaign_before = serde_json::to_value(&prepared.campaign).unwrap();
        let mut loading_screen = None;
        let mut feedback = (None, &mut loading_screen);
        let args = Default::default();
        let constructed = prepared
            .construct_engine(&args, &mut feedback)
            .unwrap_or_else(|error| panic!("construct stage: {}", error.message));
        let error = match constructed.attach_presentation(
            &mut Host::scratch(1024.0, 768.0),
            &args,
            &mut feedback,
            TerrainJoinPoint::BeforeHeadlessRuntime,
        ) {
            Ok(_) => panic!("terrain failure must reject attachment"),
            Err(error) => error,
        };
        assert_eq!(error.message, "terrain fixture failed");
        assert_eq!(
            serde_json::to_value(&error.campaign).unwrap(),
            campaign_before
        );
    }

    #[test]
    fn stage_diagnostics_cannot_reconstruct_runtime_capabilities() {
        let diagnostic = serde_json::to_string(&prepared_stage_fixture()).unwrap();
        assert_eq!(diagnostic, "\"PreparedMission\"");
        assert!(serde_json::from_str::<PreparedMission>(&diagnostic).is_err());
        assert!(serde_json::from_str::<ConstructedMission>("{}").is_err());
        assert!(serde_json::from_str::<MissionLaunchSetup>("{}").is_err());
    }

    #[test]
    fn concurrent_preparation_keeps_descriptor_patches_in_their_reader() {
        use engine_sbfile::SbFileSystem;
        use robin_util::asset_fs::{AssetVfs, Bundle};
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = [1u32, 2]
            .into_iter()
            .map(|id| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let vfs = std::sync::Arc::new(AssetVfs::new());
                    let text = format!("installation {id}");
                    vfs.mount_bundle_first(std::sync::Arc::new(Bundle::from([(
                        "levels/shared.descriptors.patch.json".into(),
                        serde_json::to_vec(&serde_json::json!([
                            {"op":"replace", "path":"/custom_popup_texts", "value":[text]}
                        ]))
                        .unwrap()
                        .into(),
                    )])))
                    .unwrap();
                    let files = SbFileSystem::new(vfs).snapshot();
                    let mut profiles = engine_profiles::ProfileManager::new();
                    profiles.missions = vec![engine_profiles::MissionProfile {
                        id,
                        mission_filename: "shared".into(),
                        ..Default::default()
                    }];
                    let campaign = Campaign {
                        current_mission_idx: Some(0),
                        missions: vec![robin_engine::mission::Mission {
                            profile_idx: Some(0),
                            ..Default::default()
                        }],
                        ..Default::default()
                    };
                    barrier.wait();
                    for _ in 0..20 {
                        assert_eq!(
                            crate::main_entry::current_mission_id(&campaign, &profiles),
                            id
                        );
                        let mut descriptors = assets_res_descr::LevelDescriptors::default();
                        apply_mission_descriptor_patch(
                            &campaign,
                            &profiles,
                            &mut descriptors,
                            &files,
                        )
                        .unwrap();
                        assert_eq!(
                            descriptors.custom_popup_texts[0].as_deref(),
                            Some(text.as_str())
                        );
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn peasant_registration_has_identical_cpu_and_portrait_bootstrap_identity() {
        use robin_engine::character_kind::CharacterKind;
        let mut assets = LevelAssets::new();
        let engine = Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets).unwrap();
        assets.peasant_firstnames = ["Peter", "Matt", "John"].map(str::to_owned).to_vec();
        assets.peasant_surnames = ["Hunter", "Little", "Chopper"].map(str::to_owned).to_vec();
        let mut graphical = engine.clone();
        let mut headless = engine.clone();
        let mut portrait_names = std::array::from_fn(|_| None);
        portrait_names[CharacterKind::LittleJohn.as_index()] = Some("Little John".into());
        let mut cpu_names = std::array::from_fn(|_| None);
        register_mission_peasant_names(&mut portrait_names, &mut graphical, &assets);
        register_mission_peasant_names(&mut cpu_names, &mut headless, &assets);
        for kind in [
            CharacterKind::MerryManA,
            CharacterKind::MerryManB,
            CharacterKind::MerryManC,
        ] {
            let name = cpu_names[kind.as_index()].as_ref().unwrap();
            assert!(headless.is_peasant_name_registered(name));
            assert_eq!(portrait_names[kind.as_index()].as_ref(), Some(name));
        }
        assert_eq!(
            headless.rng_seed(),
            engine.rng_seed(),
            "auxiliary generation must not advance authoritative RNG"
        );
        assert_eq!(headless.frame_counter(), engine.frame_counter());
        let hash = robin_engine::replay::state_hash(&headless);
        assert_ne!(hash, robin_engine::replay::state_hash(&engine));
        assert_eq!(hash, robin_engine::replay::state_hash(&graphical));
        register_mission_peasant_names(&mut portrait_names, &mut graphical, &assets);
        assert_eq!(
            hash,
            robin_engine::replay::state_hash(&graphical),
            "installed names must not register twice"
        );
    }

    #[test]
    fn missing_peasant_pool_does_not_fabricate_campaign_names() {
        let mut assets = LevelAssets::new();
        let mut engine =
            Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets).unwrap();
        let before = robin_engine::replay::state_hash(&engine);
        let mut names = std::array::from_fn(|_| None);
        register_mission_peasant_names(&mut names, &mut engine, &assets);
        assert!(names.iter().all(Option::is_none));
        assert_eq!(before, robin_engine::replay::state_hash(&engine));
    }

    #[test]
    fn spectator_camera_focus_is_actor_centroid() {
        let focus = spectator_actor_centroid(
            [
                engine_coordinates::MapPoint::new(100.0, 200.0),
                engine_coordinates::MapPoint::new(300.0, 400.0),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(focus, engine_coordinates::MapPoint::new(200.0, 300.0));
        assert!(spectator_actor_centroid(std::iter::empty()).is_none());
    }

    #[test]
    fn loading_tail_phase_targets_are_monotonic() {
        let targets = [
            LOADING_MAP_DECODE_PROGRESS,
            LOADING_SPRITE_VARIANTS_PROGRESS,
            LOADING_AUDIO_PROGRESS,
            LOADING_DESCRIPTORS_PROGRESS,
            LOADING_HUD_FONTS_PROGRESS,
            LOADING_FINAL_PROGRESS,
        ];

        assert!(targets.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(targets.last().copied(), Some(1.0));
    }

    #[test]
    fn missions_and_map_exports_default_unfogged_and_allow_explicit_opt_in() {
        let ordinary = crate::main_entry::MissionLaunch::default();
        assert!(!initial_sim_config(&ordinary).fog_of_war);

        let unfogged_export = crate::main_entry::MissionLaunch {
            mission_start_map_output: Some("map.png".into()),
            ..Default::default()
        };
        assert!(!initial_sim_config(&unfogged_export).fog_of_war);

        let fogged_export = crate::main_entry::MissionLaunch {
            mission_start_fog_of_war: true,
            ..unfogged_export
        };
        assert!(initial_sim_config(&fogged_export).fog_of_war);
    }
}

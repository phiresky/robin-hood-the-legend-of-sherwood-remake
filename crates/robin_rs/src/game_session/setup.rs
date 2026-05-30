//! Mission-startup helpers extracted from `game_session`:
//! audio bank loading, sound-duration tables, level/sprite-bank
//! initialization, sprite renderer setup, and the SDL audio backend
//! bootstrap.

use crate::Host;
use crate::campaign::Campaign;
use crate::cursor::CursorRenderer;
use crate::game::Game;
use crate::geo2d::{self, BBox2D};
use crate::hud_text::HudFonts;
use crate::input::ThreadedInput;
use crate::input_translator::{GameKey, InputTranslator};
use crate::key_config_store::KeyConfigStore;
use crate::main_entry::{current_mission_id, picture_to_surface};
use crate::markers::SelectionMarkRenderer;
use crate::minimap::HitMask;
use crate::mouse_trail::MouseTrailRenderer;
use crate::player_command::PlayerCommand;
use crate::player_profile::PlayerProfileManager;
use crate::profiles::MissionLocation;
use crate::renderer::{Renderer, TRANSPARENT_COLOR_KEY_16};
use crate::resource_ids;
use crate::resource_manager::ResourceManager;
use crate::sbfile::SbFile;
use crate::sdl_audio::SdlMixerBackend;
use crate::sound::{NUM_CHANNELS, SoundMode};
use crate::sound_config::SoundConfig;
use crate::titbit::SpriteRow;
use crate::titbit_renderer::TitbitRenderer;
use crate::ui_panel::{PortraitCache, load_localized_character_names};
use robin_assets::frame_holder as assets_frame_holder;
use robin_assets::res_descr as assets_res_descr;
use robin_assets::scb as assets_scb;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::SpriteLocalPoint;
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::geo2d::Vec2D;
use robin_engine::profiles as engine_profiles;
use robin_engine::sbfile as engine_sbfile;
use robin_engine::scb as engine_scb;
use robin_engine::script_manager as engine_script_manager;
use robin_engine::sound::ExclamationGroup;
use robin_engine::sound_cache as engine_sound_cache;
use robin_engine::sprite_script::{NONANIMATION_END, SpriteInfo, SpriteScript, UNMAPPED};

#[derive(Debug, serde::Deserialize)]
struct HackableRhsManifest {
    profiles: Vec<HackableRhsProfile>,
}

#[derive(Debug, serde::Deserialize)]
struct HackableRhsProfile {
    name: String,
    width: f32,
    height: f32,
    center_x: f32,
    center_y: f32,
    rows: Vec<HackableRhsRow>,
}

#[derive(Debug, serde::Deserialize)]
struct HackableRhsRow {
    action_id: u16,
    action_done: u16,
    average_speed: f32,
    #[serde(default)]
    direction: u16,
    hotspot_x: f32,
    hotspot_y: f32,
    path: String,
    frames: Vec<HackableRhsFrame>,
}

#[derive(Debug, serde::Deserialize)]
struct HackableRhsFrame {
    file: String,
    delay: u16,
    distance: u16,
    offset_x: f32,
    offset_y: f32,
    sound_id: u16,
}

fn overlay_roots_from_env() -> Vec<std::path::PathBuf> {
    engine_sbfile::SbFile::overlay_paths()
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect()
}

fn decode_png_rgba(path: &std::path::Path) -> Result<(u16, u16, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("decode {}: {e}", path.display()))?;
    let mut buf = vec![
        0;
        reader.output_buffer_size().ok_or_else(|| format!(
            "unknown PNG output size for {}",
            path.display()
        ))?
    ];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("read frame {}: {e}", path.display()))?;
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for px in data.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        other => {
            return Err(format!(
                "unsupported PNG color type {:?} for {}",
                other,
                path.display()
            ));
        }
    };
    Ok((info.width as u16, info.height as u16, rgba))
}

fn preload_hackable_character_dirs(host: &mut Host, assets: &mut LevelAssets) {
    for root in overlay_roots_from_env() {
        let chars = root.join("Data/Characters");
        let Ok(entries) = std::fs::read_dir(&chars) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            let Some(filename) = name.strip_suffix(".rhs.d") else {
                continue;
            };
            let manifest_path = path.join("manifest.json");
            let Ok(manifest_json) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            let manifest: HackableRhsManifest = match serde_json::from_str(&manifest_json) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("Failed to parse {}: {e}", manifest_path.display());
                    continue;
                }
            };
            for profile in manifest.profiles {
                let mut scripts = Vec::with_capacity(profile.rows.len());
                let mut conversion = vec![UNMAPPED; NONANIMATION_END];
                for row in profile.rows {
                    let row_index = scripts.len() as u16;
                    if let Some(slot) = conversion.get_mut(row.action_id as usize)
                        && *slot == UNMAPPED
                    {
                        *slot = row_index;
                    }
                    // Minimal hackable characters may only provide idle
                    // and walking loops. Reuse those for the engine's
                    // nearby idle/transition action requests.
                    let aliases: &[usize] = match row.action_id {
                        3 => &[0, 1, 2, 4, 8],
                        6 => &[5, 7, 9, 10, 11, 12],
                        _ => &[],
                    };
                    if row.direction == 0 {
                        for &alias in aliases {
                            if let Some(slot) = conversion.get_mut(alias)
                                && *slot == UNMAPPED
                            {
                                *slot = row_index;
                            }
                        }
                    }
                    let mut script = SpriteScript {
                        action_id: row.action_id,
                        action_done: row.action_done,
                        average_speed: row.average_speed,
                        hotspot: SpriteLocalPoint::new(row.hotspot_x, row.hotspot_y),
                        ..SpriteScript::default()
                    };
                    for frame in row.frames {
                        let frame_path = path.join(&row.path).join(&frame.file);
                        let (w, h, rgba) = match decode_png_rgba(&frame_path) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!("{e}");
                                continue;
                            }
                        };
                        let bank_id = host.frame_holder_mut().append_rgba_sprite(w, h, &rgba);
                        script.frame_ids.push(bank_id);
                        script.delays.push(frame.delay);
                        script.distances.push(frame.distance);
                        script.offsets.push(Vec2D {
                            x: frame.offset_x,
                            y: frame.offset_y,
                        });
                        script.sound_ids.push(frame.sound_id);
                        script.sum_distance = script.sum_distance.saturating_add(frame.distance);
                    }
                    scripts.push(script);
                }
                let cache_key = format!("{filename}/{}", profile.name);
                assets.sprite_scriptor_mut().insert(
                    cache_key.clone(),
                    SpriteInfo {
                        scripts: std::sync::Arc::new(scripts),
                        conversion: std::sync::Arc::new(conversion),
                        size: Vec2D {
                            x: profile.width,
                            y: profile.height,
                        },
                        center: Vec2D {
                            x: profile.center_x,
                            y: profile.center_y,
                        },
                    },
                );
                tracing::info!("Loaded hackable character profile {cache_key}");
            }
        }
    }
}

/// Load mission-specific sound banks and switch to mission music.
///
/// Loads the FX / menu / exclamation caches, populates the music pool
/// from the mission profile, and switches the mixer to mission mode
/// right after the loading screen closes.  Pure host-side work — reads
/// profile/sound metadata off the engine but does not mutate it.
pub(super) fn setup_mission_audio(
    host: &mut Host,
    backend: Option<&mut SdlMixerBackend>,
    engine: &Engine,
    assets: &mut LevelAssets,
    profiles: &engine_profiles::ProfileManager,
    location: MissionLocation,
    sound_dir: &str,
) {
    let loader = crate::sdl_audio::create_sample_loader(std::path::PathBuf::from(sound_dir));

    // Load FX bank.
    {
        let fx_bank_path = "Data/Sounds/robin hood.fxg";
        match SbFile::read_all(fx_bank_path) {
            Ok(data) => match crate::sound_cache::parse_fx_bank(&data) {
                Ok(elements) => {
                    host.sound.sound_cache.initialize_fx_cache(&elements);
                    tracing::info!("Loaded FX bank: {} elements", elements.len());
                }
                Err(e) => tracing::warn!("Failed to parse FX bank: {}", e),
            },
            Err(e) => tracing::warn!("Failed to read FX bank '{}': error {}", fx_bank_path, e),
        }
    }

    // Load menu sound bank.
    {
        let menu_bank_path = "Data/Sounds/Menu/menu.fxg";
        match SbFile::read_all(menu_bank_path) {
            Ok(data) => match crate::sound_cache::parse_menu_bank(&data) {
                Ok(entries) => {
                    host.sound.sound_cache.initialize_menu_cache(&entries);
                    tracing::info!("Loaded menu sound bank: {} entries", entries.len());
                }
                Err(e) => tracing::warn!("Failed to parse menu sound bank: {}", e),
            },
            Err(e) => tracing::warn!(
                "Failed to read menu sound bank '{}': error {}",
                menu_bank_path,
                e
            ),
        }
    }

    // Initialize exclamation cache: load actors.res for variant-index
    // → WAV-filename resolution, then parse each profile's .dat file
    // and register speech entries.
    {
        let mut excl_res = ResourceManager::new();
        let shipping = host.shipping.as_deref();
        let res_loaded = excl_res
            .attach_or_from_shipping("Data/Sounds/Exclamations/actors.res", shipping)
            .is_ok();

        if res_loaded {
            // Collect unique exclamation IDs from all profile types
            let mut files_needed = std::collections::BTreeMap::<u32, String>::new();
            if engine.campaign().is_some() {
                for ch in &profiles.characters {
                    if ch.exclamation_id != 0 {
                        let bytes = ch.exclamation_id.to_le_bytes();
                        let name: String = bytes
                            .iter()
                            .filter(|&&b| b != 0)
                            .map(|&b| b as char)
                            .collect();
                        files_needed.insert(ch.exclamation_id, format!("actor{name}.dat"));
                    }
                }
                for s in &profiles.soldiers {
                    if s.exclamation_id != 0 {
                        let bytes = s.exclamation_id.to_le_bytes();
                        let name: String = bytes
                            .iter()
                            .filter(|&&b| b != 0)
                            .map(|&b| b as char)
                            .collect();
                        files_needed.insert(s.exclamation_id, format!("actor{name}.dat"));
                    }
                }
                for c in &profiles.civilians {
                    if c.exclamation_id != 0 {
                        let bytes = c.exclamation_id.to_le_bytes();
                        let name: String = bytes
                            .iter()
                            .filter(|&&b| b != 0)
                            .map(|&b| b as char)
                            .collect();
                        files_needed.insert(c.exclamation_id, format!("actor{name}.dat"));
                    }
                }
            }

            let mut total_exclamations = 0usize;
            for (&excl_id, dat_filename) in &files_needed {
                let dat_path = format!("Data/Sounds/Exclamations/{dat_filename}");

                let data = match SbFile::read_all(&dat_path) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!(
                            "Failed to read exclamation file '{}': error {}",
                            dat_path,
                            e
                        );
                        continue;
                    }
                };

                let prefix_id = excl_id & 0xFFFF_0000;
                let (table_id, exclamations) =
                    match crate::sound_cache::parse_exclamation_file(&data, prefix_id) {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(
                                "Failed to parse exclamation file '{}': {}",
                                dat_filename,
                                e
                            );
                            continue;
                        }
                    };

                // Resolve variant indices to WAV file paths via resource manager
                let resolved: Vec<(u32, Vec<String>)> = exclamations
                    .into_iter()
                    .map(|(action_id, variant_indices)| {
                        let paths: Vec<String> = variant_indices
                            .into_iter()
                            .filter_map(|vi| {
                                excl_res
                                    .get_sample(table_id as i32, vi as usize)
                                    .ok()
                                    .map(|s| s.to_string())
                            })
                            .collect();
                        (action_id, paths)
                    })
                    .collect();

                total_exclamations += resolved.len();
                host.sound
                    .sound_cache
                    .initialize_exclamations_for_profile(&resolved);
            }

            tracing::info!(
                "Loaded exclamation cache: {} profiles, {} exclamations",
                files_needed.len(),
                total_exclamations,
            );
        } else {
            tracing::warn!("Failed to load actors.res — exclamation cache not initialized");
        }
    }

    // Initialize music pools from the mission profile.
    if let Some(campaign) = engine.campaign()
        && let Some(idx) = campaign.current_mission_idx
    {
        let prof = campaign.missions[idx].profile(profiles);
        host.sound.sound_cache.initialize_music(
            &prof.green_music,
            &prof.yellow_music,
            &prof.red_music,
        );
    }

    // Populate the sound-source cache from the IDs collected during
    // proto-level loading, then finalize.  Without this the source
    // cache is empty and looped/ambient sources cannot play.
    host.sound
        .sound_cache
        .initialize_sound_source_cache(&assets.sound_source_required_ids);
    host.sound
        .sound_cache
        .finalize_sound_sources(&engine.sound_sim().sources);
    populate_sound_duration_tables(host, assets, profiles, &loader);

    // Per-entry sample validation block.  When
    // `gGlobalOptions.bCheckSoundData` is set, the engine validates
    // each sample as it's added; we run the equivalent load+unload
    // sweep here because `add_entry` doesn't have a loader at insert
    // time. The resulting `data_check_succeeded` flag is consulted by
    // `SoundManager::activate` (fatal panic on miss).
    let check = engine_api::GlobalOptions::global()
        .as_ref()
        .map(|o| o.check_sound_data)
        .unwrap_or(false);
    if check {
        host.sound.sound_cache.validate_data(&loader);
    }

    if let Some(backend) = backend {
        host.sound.activate(
            location == MissionLocation::Sherwood,
            &engine.sound_sim().sources,
        );
        // Switch from menu music to mission music after the loading
        // screen closes.  SetMode(Mission) halts the menu stream and
        // re-raises load_music so mission music starts from the pool.
        host.sound.set_mode(SoundMode::Mission, backend);
    }
}

fn populate_sound_duration_tables(
    host: &Host,
    assets: &mut LevelAssets,
    profiles: &engine_profiles::ProfileManager,
    loader: &engine_sound_cache::SampleLoader,
) {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    fn frames_from_ms(ms: u32) -> u32 {
        ((ms.saturating_add(39)) / 40).max(1)
    }

    let mut groups_by_profile: BTreeMap<u32, BTreeSet<ExclamationGroup>> = BTreeMap::new();
    for profile in &profiles.characters {
        if profile.exclamation_id != 0 {
            groups_by_profile
                .entry(profile.exclamation_id)
                .or_default()
                .insert(ExclamationGroup::Pc);
        }
    }
    for profile in &profiles.soldiers {
        if profile.exclamation_id != 0 {
            let entry = groups_by_profile.entry(profile.exclamation_id).or_default();
            // AI `Say` uses the civilian bank for ordinary soldier
            // remarks, while direct hit/death speech still tags
            // soldiers distinctly.
            entry.insert(ExclamationGroup::Civilian);
            entry.insert(ExclamationGroup::Soldier);
            if profile.vip {
                entry.insert(ExclamationGroup::Vip);
            }
        }
    }
    for profile in &profiles.civilians {
        if profile.exclamation_id != 0 {
            let entry = groups_by_profile.entry(profile.exclamation_id).or_default();
            entry.insert(ExclamationGroup::Civilian);
            if profile.civilian_type == engine_profiles::CivilianType::Vip {
                entry.insert(ExclamationGroup::Vip);
            }
        }
    }

    let mut exclamation_durations = BTreeMap::new();
    for (&group_id, group) in &host.sound.sound_cache.speech_cache.groups {
        let profile_prefix = group_id & 0xFFFF_0000;
        let exclamation_id = (group_id & 0xFFFF) as u16;
        let duration_ms = group
            .entry_indices
            .iter()
            .filter_map(|&idx| host.sound.sound_cache.speech_cache.entries.get(idx))
            .filter_map(|entry| loader(&entry.file_name).map(|(_, _, duration_ms)| duration_ms))
            .max();
        let Some(duration_ms) = duration_ms else {
            continue;
        };
        let frames = frames_from_ms(duration_ms);
        for (&profile_id, groups) in &groups_by_profile {
            if profile_id & 0xFFFF_0000 != profile_prefix {
                continue;
            }
            for &group_kind in groups {
                exclamation_durations.insert((group_kind, profile_id, exclamation_id), frames);
            }
        }
    }

    let mut source_durations = BTreeMap::new();
    for (&sample_id, entry) in &host.sound.sound_cache.source_cache.entries {
        if let Some((_, _, duration_ms)) = loader(&entry.file_name) {
            source_durations.insert(sample_id, frames_from_ms(duration_ms));
        }
    }

    tracing::info!(
        exclamations = exclamation_durations.len(),
        sources = source_durations.len(),
        "Populated deterministic sound duration tables"
    );
    assets.exclamation_durations = Arc::new(exclamation_durations);
    assets.source_durations = Arc::new(source_durations);
}

/// Pre-decode the background map + minimap and attach the interface /
/// text resource files while the loading screen is still visible.
///
/// Second progress-closure scope (the first one was dropped at the end
/// of the CPU-only loading block, so audio setup could borrow
/// `window.sdl`).  The closure must be dropped before we close the
/// loading screen and hand `window.canvas` to the game renderer.
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
#[allow(clippy::too_many_arguments)]
pub(super) fn pre_decode_maps_and_resources(
    mut event_pump: Option<&mut crate::window::GameWindow>,
    loading_screen: &mut Option<crate::loading_screen::LoadingScreenRenderer>,
    engine: &mut Engine,
    profiles: &engine_profiles::ProfileManager,
    host: &Host,
    game: &Game,
) -> (
    Option<engine_api::level_loading::PreDecodedBackground>,
    Option<engine_api::level_loading::PreDecodedMinimap>,
    Option<assets_res_descr::LevelDescriptors>,
    Option<HudFonts>,
) {
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    // `text_res` (Data/Text/Level.res) + `cursor_res` (DEFAULT.RES) were
    // attached earlier so `Engine::new` could absorb the peasant name
    // pool, ground-mark sprite data, and titbit row counts.

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Loading level descriptors...", 0.76);
    }

    // Level descriptors (`.red` file) and HUD fonts — file I/O only.
    let level_descriptors = engine.campaign().and_then(|campaign| {
        let mission_id = current_mission_id(campaign, profiles);
        let filename = assets_res_descr::red_filename(mission_id);
        if let Some(dd) = host.shipping.as_deref()
            && let Some(desc) = dd.red_files.get(&filename)
        {
            tracing::info!(
                "Level descriptors {filename}: loaded from shipping datadir ({} dialogues)",
                desc.dialogues.len()
            );
            return Some(desc.clone());
        }
        let path = format!("Data/Text/{filename}");
        match assets_res_descr::load(&path) {
            Ok(desc) => {
                tracing::info!(
                    "Loaded level descriptors from {path}: {} dialogues",
                    desc.dialogues.len()
                );
                Some(desc)
            }
            Err(e) => {
                tracing::warn!("Failed to load level descriptors from {path}: {e}");
                None
            }
        }
    });
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Loading HUD fonts...", 0.77);
    }
    let hud_fonts = HudFonts::load();
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    // Background + minimap bitmaps are pre-decoded inside
    // `load_level_and_sprite_bank` — they must be decoded *before*
    // `Engine::new` so the engine can be constructed with real grid
    // dimensions (RAII).  This function now only handles the
    // post-engine resources (level descriptors + HUD fonts).  Let the
    // caller use the bg/mm from `load_level_and_sprite_bank`.
    let _ = (engine, game, host, event_pump);

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Finalizing...", 1.0);
    }
    (None, None, level_descriptors, hud_fonts)
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
/// `set_titbit_row_frame_counts`, `set_peasant_names`) and bakes the
/// ambience shadow key into the sprite dictionaries via Arno's Law.
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
    let mut cursor_renderer = CursorRenderer::new();
    cursor_renderer.init(renderer);

    // Load the default game cursor.
    if !cursor_renderer.load_cursor(resource_ids::RHMOUSE_DEFAULT, cursor_res, renderer) {
        tracing::warn!("Failed to load default cursor — using fallback arrow");
    }

    // ── Minimap corner button ──
    // Corner-sprite dims + hit mask were pre-computed from
    // `cursor_res` and handed to `Engine::new` via
    // `EngineArgs::minimap_widget`; this block only uploads the
    // corner GPU textures and stashes the corner size host-side for
    // the HUD layout.
    match cursor_res.get_dimension(resource_ids::RHMAP_CORNER) {
        Ok((btn_w, btn_h)) => {
            host.minimap_corner_size = geo2d::pt(btn_w as f32, btn_h as f32);
            if let Ok(pics) = cursor_res.get_pictures(resource_ids::RHMAP_CORNER) {
                let corner_surfaces: Vec<u32> = pics
                    .iter()
                    .filter_map(|opt| opt.as_ref().map(|p| picture_to_surface(renderer, p)))
                    .collect();
                if !corner_surfaces.is_empty() {
                    host.minimap_corner_surfaces = corner_surfaces;
                }
            }
            tracing::info!(
                "Minimap corner button: {}x{}, button at ({:.0}, {:.0}), map at ({:.0}, {:.0})",
                btn_w,
                btn_h,
                host.engine_display.minimap().button_box().top_left().x,
                host.engine_display.minimap().button_box().top_left().y,
                host.engine_display.minimap().map_box().top_left().x,
                host.engine_display.minimap().map_box().top_left().y,
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
            let surfaces: Vec<(u32, u16, u16)> = pics
                .iter()
                .map(|opt| match opt {
                    Some(p) => (picture_to_surface(renderer, p), p.width, p.height),
                    None => (0, 0, 0),
                })
                .collect();
            tracing::info!("Loaded RHMAP_ITEMS: {} dot frames", surfaces.len());
            host.minimap_dot_surfaces = surfaces;
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
            let surfaces: Vec<(u32, u16, u16)> = pics
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
            host.ground_mark_surfaces = surfaces;
        }
        Err(e) => {
            tracing::warn!("Failed to load RHID_GROUND_FOCUS resource: {e}");
        }
    }

    // ── Arno Law: bake ambience shadow color into sprite dictionaries ──
    // Dictionary-compressed sprites store shadow pixels as `SHADOW_KEY`
    // (0x001F, pure blue). Walk every loaded dictionary and replace
    // those markers with the ambience-specific night-shadow color, so
    // the GPU sprite cache can recognize shadow pixels via the
    // per-ambience key (see `renderer::ensure_sprite_cached`). Without
    // this call, dictionary sprite shadows render as opaque pure blue.
    host.frame_holder_mut()
        .apply_arno_law(engine.weather().night_color);

    // ── Selection mark renderer ──
    // Loads RHID_GROUND_SELECT (green idle) and RHID_GROUND_SELECT_SWORD
    // (red combat) sprites from DEFAULT.RES.
    let mut selection_mark_renderer = SelectionMarkRenderer::new();
    selection_mark_renderer.load(cursor_res, renderer, engine.weather().night_color);

    // ── Swordfight mouse-trail renderer ──
    // Loads RHID_MOUSE_TRAIL, builds the 32-level alpha pattern table,
    // and creates one managed surface per alpha level.  Rendered each
    // frame while the player drags the left mouse button during a
    // swordfight.
    let mouse_trail_renderer = match cursor_res.get_picture(resource_ids::RHID_MOUSE_TRAIL, 0) {
        Ok(pic) => {
            let r = MouseTrailRenderer::from_picture(pic, renderer);
            if r.is_none() {
                tracing::warn!(
                    "RHID_MOUSE_TRAIL picture was not in an RGB16 format or was empty — swordfight trail disabled"
                );
            } else {
                tracing::info!("Loaded RHID_MOUSE_TRAIL: pattern height {}", pic.height);
            }
            r
        }
        Err(e) => {
            tracing::warn!("Failed to load RHID_MOUSE_TRAIL resource: {e}");
            None
        }
    };

    // ── Titbit renderer ──
    // Loads the GPU textures for every titbit sprite row from the
    // `titbit_texture_creator` we created up front (before the
    // renderer's canvas borrow).
    let mut titbit_renderer = TitbitRenderer::new();
    titbit_renderer.load(
        cursor_res,
        renderer.gpu(),
        engine.weather().night_color,
        renderer.scale_mode(),
    );
    // Row frame counts were absorbed by the engine at construction via
    // `EngineArgs::titbit_row_frame_counts`; no post-load setter needed.

    // ── Portrait pictures (character faces in the bottom panel) ──
    // Portraits live in the same DEFAULT.RES file as cursors.
    let mut portrait_cache = PortraitCache::new();
    portrait_cache.load(cursor_res, renderer);

    // ── Localized character names ──
    // `text_res` (Data/Text/Level.res) was pre-attached in the loading-
    // screen block above.  The peasant firstname/surname *pool* is
    // loaded earlier and handed to `Engine::new` via `EngineArgs`; this
    // call assigns per-civilian display names on top of that pool.
    //
    // The localized hero names + generated peasant names live entirely
    // in the host-side `PortraitCache.localized_names` map; the engine
    // never stores them on entities (HUD code resolves on demand from
    // the profile tables + this map).
    portrait_cache.install_localized_names(load_localized_character_names(text_res));
    portrait_cache.generate_peasant_names(
        text_res,
        engine,
        &mut host.engine_display,
        &mut host.input,
        assets,
    );

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
/// pixel-level `IsRealPoint` test).  Returns `None` when the resource
/// is missing or has no pictures.
pub(super) fn extract_minimap_widget_setup(
    cursor_res: &mut ResourceManager,
) -> Option<engine_api::MinimapWidgetSetup> {
    let (btn_w, btn_h) = cursor_res.get_dimension(resource_ids::RHMAP_CORNER).ok()?;
    let corner_size = geo2d::pt(btn_w as f32, btn_h as f32);
    let mut button_hit_mask = None;
    if let Ok(pics) = cursor_res.get_pictures(resource_ids::RHMAP_CORNER)
        && let Some(Some(pic)) = pics.get(1)
    {
        let pixels: Vec<u16> = pic
            .data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        button_hit_mask = Some(HitMask::from_pixels_u16(
            pic.width,
            pic.height,
            &pixels,
            TRANSPARENT_COLOR_KEY_16,
        ));
    }
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
    let pics = cursor_res
        .get_pictures(resource_ids::RHID_GROUND_FOCUS)
        .ok()?;
    let first_pic = pics.iter().find_map(|opt| opt.as_ref())?;
    let frame_sizes: Vec<(u16, u16)> = pics
        .iter()
        .filter_map(|opt| opt.as_ref().map(|p| (p.width, p.height)))
        .collect();
    if frame_sizes.is_empty() {
        return None;
    }
    // The destination marker uses the auto-cropped tight bounds of
    // frame 0; we store the uncropped Picture, so scan for the opaque
    // bounds and fall back to the raw size when the scan can't run.
    let (cw, ch) = first_pic
        .opaque_bounds_16()
        .map(|(_, _, cw, ch)| (cw, ch))
        .unwrap_or((frame_sizes[0].0, frame_sizes[0].1));
    // Per-frame offset = (x_min, y_min) of the opaque region.  Used
    // by `IsOnScreen`/`GenerateBlitBox` so the cull AABB tracks the
    // opaque region instead of the full uncropped surface.  Defaults
    // to (0, 0) for any frame whose opaque-bounds scan can't run
    // (non-16-bit or fully transparent).
    let per_frame_offsets: Vec<(i16, i16)> = pics
        .iter()
        .map(|opt| {
            opt.as_ref()
                .and_then(|p| p.opaque_bounds_16())
                .map(|(x, y, _, _)| (x as i16, y as i16))
                .unwrap_or((0, 0))
        })
        .collect();
    Some(engine_api::GroundMarkSpriteData {
        half_w: cw as f32 * 0.5,
        half_h: ch as f32 * 0.5,
        frame_sizes,
        per_frame_offsets,
    })
}

/// Pre-compute titbit sprite-row frame counts from `cursor_res`.
/// Indexed by `SpriteRow` discriminant.  Counts sub-pictures without
/// decoding them — enough for `TitbitManager::num_frames_for_row` to
/// drive animation.
pub(super) fn extract_titbit_row_frame_counts(cursor_res: &mut ResourceManager) -> Vec<u16> {
    use crate::titbit_renderer::titbit_sprite_row_resources;
    let num_rows = SpriteRow::NumberOfRows as usize;
    let mut counts = vec![0u16; num_rows];
    for &(row, res_id) in titbit_sprite_row_resources() {
        let n = cursor_res
            .get_pictures(res_id)
            .map(|pics| {
                pics.iter()
                    .filter(|o| o.as_ref().is_some_and(|p| p.width > 0 && p.height > 0))
                    .count() as u16
            })
            .unwrap_or(0);
        let idx = row as usize;
        if idx < counts.len() {
            counts[idx] = n;
        }
    }
    counts
}

/// Load the 22-firstname / 22-surname peasant name pool from
/// `Level.res` — the civilian display-name branch.  Sub-IDs 100-121
/// hold firstnames, 122-143 surnames, under one of three menu text
/// tables (full / demo / demo2).
fn load_peasant_name_pool(text_res: &mut ResourceManager) -> (Vec<String>, Vec<String>) {
    use crate::ui_panel::{MENU_TEXT_TABLE_ID, MENU_TEXT_TABLE_ID_DEMO, MENU_TEXT_TABLE_ID_DEMO2};
    const FIRSTNAME_BASE: usize = 100;
    const SURNAME_BASE: usize = 122;
    const NAME_COUNT: usize = 22;
    let table_ids = [
        MENU_TEXT_TABLE_ID,
        MENU_TEXT_TABLE_ID_DEMO,
        MENU_TEXT_TABLE_ID_DEMO2,
    ];
    let fetch = |res: &mut ResourceManager, sub_id: usize| -> Option<String> {
        for &tid in &table_ids {
            if let Ok(s) = res.get_string(tid, sub_id) {
                return Some(s.to_string());
            }
        }
        None
    };
    let firstnames: Vec<String> = (0..NAME_COUNT)
        .filter_map(|i| fetch(text_res, FIRSTNAME_BASE + i))
        .collect();
    let surnames: Vec<String> = (0..NAME_COUNT)
        .filter_map(|i| fetch(text_res, SURNAME_BASE + i))
        .collect();
    (firstnames, surnames)
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
pub(super) type LoadedLevelAndSpriteBank = (
    Engine,
    engine_api::LevelAssets,
    engine_api::DevState,
    Option<engine_api::level_loading::PreDecodedBackground>,
    Option<engine_api::level_loading::PreDecodedMinimap>,
    u64,
);

#[allow(clippy::too_many_arguments)]
pub(super) fn load_level_and_sprite_bank(
    mut event_pump: Option<&mut crate::window::GameWindow>,
    loading_screen: &mut Option<crate::loading_screen::LoadingScreenRenderer>,
    host: &mut Host,
    game: &mut Game,
    campaign_ref: &mut Campaign,
    profiles: &engine_profiles::ProfileManager,
    text_res: &mut ResourceManager,
    args: &crate::main_entry::CliArgs,
    _screen_width: f32,
    _screen_height: f32,
    ground_mark_sprite: Option<engine_api::GroundMarkSpriteData>,
    titbit_row_frame_counts: Vec<u16>,
    minimap_widget: Option<engine_api::MinimapWidgetSetup>,
) -> Result<LoadedLevelAndSpriteBank, String> {
    let mut assets = engine_api::LevelAssets::new();
    // Stamp the canonical loaded profile manager onto LevelAssets — the
    // engine reads profiles via `&assets.profile_manager` everywhere now
    // (Campaign no longer owns its own copy).
    assets.profile_manager = std::sync::Arc::new(profiles.clone());
    let mut dev = engine_api::DevState::new();
    if let Some(opts) = engine_api::GlobalOptions::global().as_ref() {
        dev.debug.surface_display = opts.debug_surfaces;
    }

    // Load sprite bank (robinhood.dic + robinhood.bks) — must happen
    // before entity sprite loading in initialize_for_mission.
    // The `.bks` file is big enough that plain I/O dominates the first
    // few seconds of load, so pass progress+phase updates in to keep the
    // bar moving and the status text alive during this 3-6s phase.
    //
    // Sub-phase labels emitted from inside the loader (Reading, Decoding,
    // Parsing, Unpacking) get mapped from local 0..1 fractions onto the
    // overall sprite-bank range 0.09 → 0.51.
    let shipping = host.shipping.clone();
    const SPRITE_BANK_START: f32 = 0.12;
    const SPRITE_BANK_END: f32 = 0.56;
    {
        let mut update = |u: assets_frame_holder::ProgressUpdate| match u {
            assets_frame_holder::ProgressUpdate::Tick(d) => {
                tick_progress(loading_screen, event_pump.as_deref_mut(), d);
            }
            assets_frame_holder::ProgressUpdate::Phase(text, local) => {
                if let Some(ls) = loading_screen.as_mut() {
                    let overall = SPRITE_BANK_START + local * (SPRITE_BANK_END - SPRITE_BANK_START);
                    // Use the end-of-local-phase target for the ceiling
                    // so intra-phase ticks still advance smoothly between
                    // sub-phase changes.
                    ls.set_status(text, overall);
                }
            }
        };
        if let Err(e) = host
            .frame_holder_mut()
            .initialize_sprite_bank_with_progress(".", &mut update, shipping.as_deref())
        {
            tracing::warn!("Failed to load sprite bank: {}", e);
        }
    }
    preload_hackable_character_dirs(host, &mut assets);
    // Publish the sprite-bank signature into LevelAssets so engine-side
    // sprite-script loaders can detect bank changes.
    assets.bank_signature = host.frame_holder.signature();
    // Hand the engine a pixel-opacity lookup so
    // `Engine::is_point_on_sprite` can do transparent + night-shadow
    // rejection without depending on `robin_assets`.
    assets.pixel_opacity = Some(host.frame_holder.clone());
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Initializing level...", 0.73);
    }

    // Move the campaign out of its cell so we can both seed `assets`
    // from it and hand ownership to the engine constructor.  All reads
    // against the campaign happen *before* construction; afterwards it
    // lives inside the engine and is reached via `engine.campaign()`.
    let campaign = std::mem::take(campaign_ref);

    // Engine LevelAssets already owns profile_manager (loaded at startup);
    // Campaign no longer has its own copy.

    // Hand the engine the parsed mission scripts it'll need.
    //
    // Engine doesn't depend on robin_assets, so it can't open `.scb`
    // files itself; the host parses them (preferring shipping, falling
    // back to disk for the current mission), decodes immutable bytecode,
    // and stores the programs in `LevelAssets` before level load.
    let mission_name = campaign.current_mission_idx.map(|i| {
        campaign.missions[i]
            .profile(&assets.profile_manager)
            .mission_filename
            .clone()
    });
    let mut scripts: std::collections::BTreeMap<String, engine_scb::ScbFile> = host
        .shipping
        .as_ref()
        .map(|dd| dd.scripts.clone().into_iter().collect())
        .unwrap_or_default();
    if let Some(name) = mission_name.as_ref()
        && !scripts.contains_key(name)
    {
        let path = format!("Data/Levels/{name}.scb");
        match engine_sbfile::SbFile::read_all(&path)
            .map_err(|e| format!("read {path}: error {e}"))
            .and_then(|b| assets_scb::parse_bytes(&b).map_err(|e| format!("parse {path}: {e}")))
        {
            Ok(scb) => {
                scripts.insert(name.clone(), scb);
            }
            Err(e) => tracing::warn!("Mission script {name}: {e}"),
        }
    }
    let script_programs = scripts
        .iter()
        .map(|(name, scb)| {
            (
                name.clone(),
                std::sync::Arc::new(engine_script_manager::ScriptProgram::from_scb(scb.clone())),
            )
        })
        .collect();
    assets.mission_script_programs = std::sync::Arc::new(script_programs);

    // Initialize Game's per-mission state from the campaign before we
    // hand it off to the engine.
    game.initialize_for_mission(&campaign, &assets.profile_manager);

    // Construct the engine with campaign install + level load folded
    // in.  The old `Engine::new(w, h)` + `install_campaign` +
    // `initialize_from_campaign` + `initialize` sequence collapses to
    // this single call.  Mission script was already loaded inside
    // `load_level()` → `load_mission_script()` so the level loader
    // does not re-load it.
    (assets.peasant_firstnames, assets.peasant_surnames) = load_peasant_name_pool(text_res);

    let level_directory = game.global_options.level_directory.clone();

    // RAII: load the mission binaries *before* `Engine::new` so the
    // mission header (map filename + ambiance) is available to pre-
    // decode the background bitmap, whose pixel dimensions then go
    // into `LevelLoadArgs::bg_pixel_dims`.  With those in hand the
    // constructor returns an engine whose `fast_grid.map_bbox`,
    // motion lines, pathfinder graph, and AI init are all already
    // live — no post-construction fixup, no patrol paths silently
    // failing `TestIfPathIsFine` because the grid hadn't been sized
    // yet.
    let loaded = {
        let mut progress = |delta: f32| {
            tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
        };
        engine_api::level_loading::load_mission_for_campaign(
            &campaign,
            &assets.profile_manager,
            &level_directory,
            &mut progress,
        )
        .map_err(|e| format!("Level load failed: {e}"))?
    };

    // Pre-decode the background bitmap before `Engine::new` — the
    // constructor wants `bg_pixel_dims` to size the fast-find grid.
    let ambiance_dir = engine_api::Ambiance::from_raw(loaded.mission.header.ambiance)
        .directory()
        .to_string();
    let map_name = loaded.mission.header.map_filename.clone();
    let pre_decoded_bg = {
        let mut update = |u: assets_frame_holder::ProgressUpdate| match u {
            assets_frame_holder::ProgressUpdate::Tick(d) => {
                tick_progress(loading_screen, event_pump.as_deref_mut(), d);
            }
            assets_frame_holder::ProgressUpdate::Phase(text, _local) => {
                if let Some(ls) = loading_screen.as_mut() {
                    ls.set_status(text, 0.85);
                }
            }
        };
        crate::level_loading_host::pre_decode_background_map(
            &map_name,
            &ambiance_dir,
            &level_directory,
            host.shipping.as_deref(),
            &mut update,
        )
        .map_err(|e| format!("Background map load failed: {e}"))?
    };
    let pre_decoded_mm = {
        let mut progress = |delta: f32| {
            tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
        };
        crate::level_loading_host::pre_decode_minimap(
            &map_name,
            &ambiance_dir,
            &level_directory,
            host.shipping.as_deref(),
            &mut progress,
        )
    };
    let bg_pixel_dims = pre_decoded_bg
        .as_ref()
        .map(|b| (b.width as f32, b.height as f32))
        .unwrap_or((0.0, 0.0));

    // Resolve the engine's initial RNG seed before construction so
    // `Engine::new` is the only site that touches RNG state during
    // setup.  Priority: already-decoded RPC replay > `--replay`
    // header seed (so the recording's recorded actions reproduce its
    // recorded state) > the multiplayer-negotiated `mp_mission_seed`
    // > the hardcoded single-player default of 0.
    let rng_seed = if let Some(data) = args.replay_data.as_ref() {
        data.header.rng_seed
    } else if let Some(spec) = args.replay.as_deref() {
        match crate::replay_format::load_replay_spec(spec) {
            Ok(data) => data.header.rng_seed,
            Err(e) => {
                tracing::warn!(
                    "could not preload replay header to extract seed ({e}); falling back to mp_mission_seed / 0"
                );
                host.mp_mission_seed.unwrap_or(0)
            }
        }
    } else {
        host.mp_mission_seed.unwrap_or(0)
    };
    let goldeneye_initial = args.goldeneye || args.global_options.golden_eye;
    if let Some(mm) = minimap_widget {
        host.engine_display.setup_minimap_widget(
            engine_coordinates::ScreenPoint::new(_screen_width - 83.0, 38.0),
            mm.corner_size,
            mm.button_hit_mask,
            _screen_width,
            _screen_height,
        );
    }

    let engine = {
        let mut progress = |delta: f32| {
            tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
        };
        Engine::new(engine_api::EngineArgs {
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
            goldeneye: goldeneye_initial,
        })
        .map_err(|e| format!("Level init failed: {e}"))?
    };
    if rng_seed != 0 {
        tracing::info!(seed = rng_seed, "engine RNG seeded at construction");
    }
    host.viewport
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
        ls.set_status("Generating sprite variants...", 0.74);
    }

    // Generate night/fog variant dictionaries based on ambiance.
    // Runs host-side because the engine crate doesn't reference
    // `FrameHolder`.
    crate::level_loading_host::initialize_sprite_variants(host, &engine);
    tick_progress(loading_screen, event_pump, 1.0);

    Ok((
        engine,
        assets,
        dev,
        pre_decoded_bg,
        pre_decoded_mm,
        rng_seed,
    ))
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
    args: &crate::main_entry::CliArgs,
    window_width: u32,
    window_height: u32,
    mission_idx: usize,
) -> (ThreadedInput, InputTranslator) {
    let mut threaded_input = ThreadedInput::new();
    threaded_input.set_clipping(BBox2D::from_coords(
        0.0,
        0.0,
        window_width as f32,
        window_height as f32,
    ));
    let mut input_translator = InputTranslator::new(window_width as f32, window_height as f32);

    // Load key bindings from the active player profile.  Source of
    // truth is the global KeyConfigStore; mirror into host's session
    // cache so the in-game options menu can edit it directly without a
    // store roundtrip every keystroke.
    {
        let ppm = PlayerProfileManager::global();
        let store = KeyConfigStore::global();
        if let Some(ref mgr) = *ppm
            && let Some(profile) = mgr.get_active()
            && let Some(ref s) = *store
            && let Some(entry) = s.get(profile.id)
        {
            host.key_config = entry.active.clone();
            host.custom_key_config = entry.custom.clone();
            input_translator.load_bindings_from_keyconfig(&host.key_config);
            tracing::info!("Loaded key bindings for profile {} from store", profile.id);
        } else if !host.key_config.bindings.is_empty() {
            input_translator.load_bindings_from_keyconfig(&host.key_config);
        }
    }

    // The `DisplayMap` minimap accelerator is stored host-side on
    // `host.minimap_fast_key` — the game loop reads it out to emit a
    // minimap-toggle command on key release.  Rebind via the pause
    // menu updates the same host field.
    host.minimap_fast_key = input_translator.get_binding(GameKey::DisplayMap);

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
    let is_client = args.connect.is_some();
    if !is_client {
        let nickname = args.mp_nickname.clone();
        engine.apply_command(
            &mut host.engine_display,
            &mut host.input,
            assets,
            &PlayerCommand::ConnectSeat {
                player_id: host.local_seat,
                nickname,
            },
        );
        tracing::info!(
            seat = ?host.local_seat,
            "bootstrap ConnectSeat applied to local engine",
        );
        if let Some(net) = host.net.as_ref() {
            match net.publish_initial_snapshot(0, engine) {
                Ok(()) => {
                    net.send_ready_to_sim(0);
                    tracing::info!("multiplayer: cached and published frame-0 host snapshot");
                }
                Err(e) => {
                    tracing::warn!("multiplayer: failed to cache frame-0 host snapshot: {e}");
                }
            }
        }
    }
    if let Some(&pc_id) = engine.pc_ids().first()
        && let Some(entity) = engine.get_entity(pc_id)
    {
        let pos = entity.element_data().position_map();
        host.viewport.center_on_point(pos);
    }

    (threaded_input, input_translator)
}

/// Initialize the SDL-mixer audio backend and switch the host sound
/// manager into `SoundMode::Menu` so menu music plays during the
/// loading screen.
pub(super) fn init_audio_backend(host: &mut Host, game: &Game) -> Option<SdlMixerBackend> {
    if !game.global_options.sound_enabled {
        tracing::info!("sound disabled via `-NOSOUND`; skipping audio backend init");
        return None;
    }
    let mut audio_backend =
        match SdlMixerBackend::new(&game.global_options.sound_directory, NUM_CHANNELS) {
            Ok(backend) => Some(backend),
            Err(e) => {
                tracing::warn!("Failed to initialize audio: {}. Sound disabled.", e);
                None
            }
        };
    if let Some(backend) = audio_backend.as_mut() {
        host.sound
            .set_music_directory(&game.global_options.music_directory);
        // Read the active profile's 3D-sound preference and forward
        // it to the sound manager.  The backend grants the request
        // only when `can_3d_sound()` is true; the kira backend never
        // is, so this lands in 2D with a non-fatal warning.
        let want_3d = {
            let guard = PlayerProfileManager::global();
            guard
                .as_ref()
                .and_then(|m| m.get_active())
                .map(|p| p.sound_config.sound_3d)
                .unwrap_or(false)
        };
        if let Err(e) = host.sound.initialize(backend, want_3d) {
            tracing::warn!("Sound manager init failed: {}", e);
        }
        // Apply volumes before set_mode(Menu) so menu music isn't silent.
        host.sound.apply_volumes(&SoundConfig::default());
        host.sound.set_mode(SoundMode::Menu, backend);
    }
    audio_backend
}

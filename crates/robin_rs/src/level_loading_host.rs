//! Host-side (renderer-using) half of level loading.
//!
//! The pure-sim half — parsing `LoadedLevel`, building entity state,
//! populating lifts/masks/jump-zones — lives in `robin_engine`.  The
//! slow CPU-only decode steps (bzip2 decompress, mask composition)
//! produce `PreDecodedBackground` / `PreDecodedMinimap`, and this module
//! consumes those to upload GPU textures via the renderer.  Also
//! contains `draw_background`, which blits the cached background
//! surface to a target every frame.

use crate::Host;
use crate::renderer::Renderer;
use robin_assets::frame_holder as assets_frame_holder;
use robin_assets::frame_holder::ProgressUpdate;
use robin_assets::picture::Picture;
use robin_assets::shipping_datadir as assets_shipping_datadir;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::{MapPoint, MinimapSize};
use robin_engine::engine::level_loading::{
    MinimapBitmapSetup, PreDecodedBackground, PreDecodedMinimap,
};
use robin_engine::engine::{Ambiance, Engine, GlobalOptions, PANNEL_HEIGHT};
use robin_engine::minimap as engine_minimap;
use robin_engine::player_profile::PlayerProfileManager;
use robin_engine::sbfile;
use robin_engine::sprite::BBox;
use robin_engine::sprite_variant::SpriteVariant;

/// Select the appropriate sprite-variant dictionaries for the engine's
/// current ambiance and adjust global shadow values on the host's
/// `FrameHolder`.  Lives host-side because the engine crate no longer
/// references `FrameHolder`.
pub fn initialize_sprite_variants(host: &mut Host, engine: &Engine) {
    let fh = host.frame_holder_mut();
    // When the launcher flag `bypass_fog_sprites_crash` is on, drop both
    // Night and Fog dictionaries regardless of ambiance and skip the
    // shadow-value set — the renderer then falls back to
    // `SpriteVariant::Day` via `EngineInner::default_variant`.
    if GlobalOptions::global()
        .as_ref()
        .map(|o| o.bypass_fog_sprites_crash)
        .unwrap_or(false)
    {
        fh.drop_variant_dictionaries(SpriteVariant::Night);
        fh.drop_variant_dictionaries(SpriteVariant::Fog);
        return;
    }
    match engine.weather().ambiance {
        // Only Day / Fog / Night have dedicated sprite dictionaries;
        // Attack / Custom1..4 fall through to the Day branch.
        Ambiance::Day
        | Ambiance::Attack
        | Ambiance::Custom1
        | Ambiance::Custom2
        | Ambiance::Custom3
        | Ambiance::Custom4 => {
            fh.drop_variant_dictionaries(SpriteVariant::Night);
            fh.drop_variant_dictionaries(SpriteVariant::Fog);
            fh.set_global_shadow(40);
            fh.set_global_blip_shadow(60);
        }
        Ambiance::Fog => {
            fh.drop_variant_dictionaries(SpriteVariant::Night);
            fh.generate_fog_dictionaries();
            fh.set_global_shadow(10);
            fh.set_global_blip_shadow(40);
        }
        Ambiance::Night => {
            fh.drop_variant_dictionaries(SpriteVariant::Fog);
            fh.generate_night_dictionaries();
            fh.set_global_shadow(40);
            fh.set_global_blip_shadow(60);
        }
    }
}

/// Decode the background-map bitmap from disk (or the shipping
/// bundle).  Free function — runs *before* `Engine::new` so the
/// resulting dimensions can be fed into `LevelLoadArgs::bg_pixel_dims`
/// and the engine is born with a real `map_bbox` / grid size.
///
/// `map_name` + `ambiance_dir` come from the mission header parsed by
/// [`robin_engine::engine::level_loading::load_mission_for_campaign`].
pub fn pre_decode_background_map(
    map_name: &str,
    ambiance_dir: &str,
    level_directory: &str,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    progress: &mut dyn FnMut(assets_frame_holder::ProgressUpdate),
) -> Result<Option<PreDecodedBackground>, String> {
    if map_name.is_empty() {
        tracing::warn!("No map name set, skipping background load");
        return Ok(None);
    }

    // Try the ambiance-specific directory, then fall back to `/Day/` and
    // finally the bare level directory.  Raise a fatal error only when
    // none exist.  Same candidate list for both shipping (bundle) keys
    // and disk paths.
    let shipping_keys = [
        format!("levels/{}/{}.map", ambiance_dir, map_name).to_ascii_lowercase(),
        format!("levels/day/{}.map", map_name).to_ascii_lowercase(),
        format!("levels/{}.map", map_name).to_ascii_lowercase(),
    ];
    let disk_candidates = [
        format!("{}/{}/{}.map", level_directory, ambiance_dir, map_name),
        format!("{}/Day/{}.map", level_directory, map_name),
        format!("{}/{}.map", level_directory, map_name),
    ];

    progress(ProgressUpdate::Phase(
        "Decompressing background map...",
        0.95,
    ));

    let mut picture = None;
    if let Some(dd) = shipping {
        for key in &shipping_keys {
            if let Some(bytes) = dd.raw.get(key) {
                tracing::info!(
                    "Loading background map from shipping datadir: {} ({} bytes)",
                    key,
                    bytes.len()
                );
                match Picture::load_terrain_from_bytes(bytes) {
                    Ok(p) => {
                        picture = Some(p);
                        break;
                    }
                    Err(e) => {
                        return Err(format!("failed to decode shipped map '{key}': {e}"));
                    }
                }
            }
        }
    }
    if picture.is_none() {
        for path in &disk_candidates {
            match sbfile::SbFile::open(path, sbfile::SB_FILE_READ) {
                Ok(mut file) => {
                    tracing::info!("Loading background map: {}", path);
                    match Picture::load_terrain_from_stream(&mut file) {
                        Ok(p) => {
                            picture = Some(p);
                            break;
                        }
                        Err(e) => {
                            return Err(format!("failed to load map image '{path}': {e}"));
                        }
                    }
                }
                Err(_) => continue,
            }
        }
    }
    let picture = match picture {
        Some(p) => p,
        None => {
            return Err(format!(
                "unable to find map file {map_name}.map in level directory {level_directory} \
                 (ambiance {ambiance_dir}); tried shipping keys {shipping_keys:?} and disk \
                 {disk_candidates:?}",
            ));
        }
    };
    progress(ProgressUpdate::Tick(1.0));

    let bg_pixels: Vec<u16> = bytemuck::cast_slice::<u8, u16>(&picture.data).to_vec();

    Ok(Some(PreDecodedBackground {
        width: picture.width,
        height: picture.height,
        pixels: bg_pixels,
    }))
}

/// Upload the decoded background bitmap to the renderer and upload the
/// level's mask textures. Runs *after* `Engine::new` so
/// `engine.fast_grid().level.masks` is populated.
pub fn apply_background_map(
    engine: &Engine,
    host: &mut Host,
    renderer: &mut Renderer,
    decoded: PreDecodedBackground,
) {
    if !renderer.upload_background_texture(
        decoded.width as u32,
        decoded.height as u32,
        &decoded.pixels,
    ) {
        panic!(
            "failed to upload background texture {}x{}",
            decoded.width, decoded.height
        );
    }

    tracing::info!(
        "Background map loaded: {}x{} pixels",
        decoded.width,
        decoded.height
    );

    // Upload each mask's static binary alpha once. The bg under the
    // mask is sampled live by `mask_overlay.wgsl` at draw time, so
    // there is no per-blit recompose / re-upload — what used to churn
    // the amdgpu GTT pool on every patch blit. Reuploading covers
    // both the initial load and any subsequent reload (level restart);
    // ambiance is per-mission so no swap-mid-level path is needed.
    renderer.clear_mask_alpha_cache();
    let bg_w = decoded.width as u32;
    let bg_h = decoded.height as u32;
    let mask_count = engine.fast_grid().level.masks.len();
    for (idx, mask) in engine.fast_grid().level.masks.iter().enumerate() {
        let bbox_min = (mask.bbox.x_min().max(0.0), mask.bbox.y_min().max(0.0));
        renderer.upload_mask_alpha(
            idx as u32,
            &mask.bitmap,
            mask.width,
            mask.height,
            bbox_min,
            bg_w,
            bg_h,
        );
    }
    if mask_count > 0 {
        tracing::debug!("Uploaded {} mask alpha textures", mask_count);
    }

    host.clear_background_decals();
}

/// Decode the minimap bitmap from disk (or the shipping bundle).
/// Free function — runs before `Engine::new`.
pub fn pre_decode_minimap(
    map_name: &str,
    ambiance_dir: &str,
    level_directory: &str,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    progress: &mut dyn FnMut(f32),
) -> Option<PreDecodedMinimap> {
    if map_name.is_empty() {
        tracing::warn!("No map name set, skipping minimap load");
        return None;
    }

    // Shipping datadir takes precedence (mirrors background_map).
    // Keys are lowercased per `robin_util::asset_fs::bundle_key`.
    let shipping_keys = [
        format!("levels/{}/{}.min", ambiance_dir, map_name).to_ascii_lowercase(),
        format!("levels/day/{}.min", map_name).to_ascii_lowercase(),
        format!("levels/{}.min", map_name).to_ascii_lowercase(),
    ];
    if let Some(dd) = shipping {
        for key in &shipping_keys {
            if let Some(bytes) = dd.raw.get(key) {
                tracing::info!("Loading minimap from shipping datadir: {key}");
                match Picture::load_terrain_from_bytes(bytes) {
                    Ok(p) => {
                        progress(1.0);
                        let pixels: Vec<u16> = bytemuck::cast_slice::<u8, u16>(&p.data).to_vec();
                        return Some(PreDecodedMinimap {
                            width: p.width,
                            height: p.height,
                            pixels,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Failed to decode shipped minimap '{key}': {e}");
                        return None;
                    }
                }
            }
        }
    }

    let candidates = [
        format!("{}/{}/{}.min", level_directory, ambiance_dir, map_name),
        format!("{}/Day/{}.min", level_directory, map_name),
        format!("{}/{}.min", level_directory, map_name),
    ];

    let mut file = None;
    for path in &candidates {
        match sbfile::SbFile::open(path, sbfile::SB_FILE_READ) {
            Ok(f) => {
                tracing::info!("Loading minimap: {}", path);
                file = Some(f);
                break;
            }
            Err(_) => continue,
        }
    }

    let mut file = match file {
        Some(f) => f,
        None => {
            tracing::warn!(
                "Unable to find minimap file {}.min in {}",
                map_name,
                level_directory
            );
            return None;
        }
    };

    let picture = match Picture::load_terrain_from_stream(&mut file) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to load minimap image: {}", e);
            return None;
        }
    };
    progress(1.0);

    let pixels: Vec<u16> = bytemuck::cast_slice::<u8, u16>(&picture.data).to_vec();

    Some(PreDecodedMinimap {
        width: picture.width,
        height: picture.height,
        pixels,
    })
}

/// Upload the decoded minimap bitmap to the renderer and build the
/// [`MinimapBitmapSetup`] for `LevelLoadArgs::minimap_setup`.  Free
/// function — runs *before* `Engine::new`; screen dimensions come
/// from the host's `Window` instead of the not-yet-constructed engine.
///
/// Reads the saved minimap top-left from the active player profile
/// (`PlayerProfileManager::global`) and forwards it to the engine so
/// `setup_minimap_map` can validate against the current screen and
/// snap to the default corner if the saved point is the
/// `(65536, 65536)` sentinel or fully off-screen.
pub fn apply_minimap(
    host: &mut Host,
    renderer: &mut Renderer,
    decoded: PreDecodedMinimap,
) -> MinimapBitmapSetup {
    let surface = renderer
        .create_surface_from_rgb565(decoded.width, decoded.height, &decoded.pixels)
        .expect("apply_minimap: decoded minimap dimensions must match RGB565 payload");
    host.map_surface = surface;

    let map_w = decoded.width as f32;
    let map_h = decoded.height as f32;

    let hit_mask = engine_minimap::HitMask::from_pixels_u16(
        decoded.width,
        decoded.height,
        &decoded.pixels,
        renderer.transparent_color(),
    );

    // The sentinel `(65536, 65536)` is the per-profile "never written"
    // default (`PlayerProfile::new` initializes both fields to that
    // value).
    let saved_position = {
        let guard = PlayerProfileManager::global();
        guard
            .as_ref()
            .and_then(|m| m.get_active())
            .map(|p| engine_coordinates::ScreenPoint::new(p.minimap_x, p.minimap_y))
            .unwrap_or_else(|| engine_coordinates::ScreenPoint::new(65536.0, 65536.0))
    };

    tracing::info!(
        "Minimap loaded: {}x{} pixels, surface ID {}, saved position ({:.0}, {:.0})",
        decoded.width,
        decoded.height,
        surface,
        saved_position.x,
        saved_position.y,
    );

    MinimapBitmapSetup {
        hit_mask,
        map_size: MinimapSize::new(map_w, map_h),
        saved_position,
    }
}

/// Draw the background map to the screen using the current view.
///
/// Stays as an `Engine` extension method for historical call sites, but
/// it reads host-local viewport state every frame.
///
/// The wgpu port draws this from the renderer-owned background texture.
/// Patch effects ([`super::blit_to_map`]) render as separate persistent
/// GPU decals immediately above the base map.
pub trait EngineLevelLoadExt {
    fn draw_background(&self, host: &mut Host, renderer: &mut Renderer);
}

impl EngineLevelLoadExt for Engine {
    fn draw_background(&self, host: &mut Host, renderer: &mut Renderer) {
        let view = &host.viewport.view_position;
        let screen = &host.viewport.screen_size;
        let zoom = host.viewport.zoom_factor;

        let src_min = *view;
        let src_max = MapPoint::new(
            view.x + (screen.x / zoom),
            view.y + ((screen.y - PANNEL_HEIGHT) / zoom),
        );
        let src = BBox::from_coords(src_min.x, src_min.y, src_max.x, src_max.y);

        let dst = BBox::from_coords(0.0, 0.0, screen.x, screen.y - PANNEL_HEIGHT);

        renderer.render_background_texture(Some(&src), Some(&dst));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_decode_background_map_reports_missing_map() {
        let mut progress_updates = 0;
        let err = match pre_decode_background_map(
            "definitely_missing_map_for_test",
            "Day",
            "definitely_missing_level_dir_for_test",
            None,
            &mut |_| progress_updates += 1,
        ) {
            Ok(_) => panic!("missing background map should be a load error"),
            Err(err) => err,
        };

        assert!(err.contains("unable to find map file definitely_missing_map_for_test.map"));
        assert!(progress_updates > 0);
    }

    #[test]
    fn pre_decode_background_map_allows_empty_map_name() {
        let mut progress_updates = 0;
        let decoded = pre_decode_background_map("", "Day", "Levels", None, &mut |_| {
            progress_updates += 1;
        })
        .expect("empty map name is intentionally skipped");

        assert!(decoded.is_none());
        assert_eq!(progress_updates, 0);
    }
}

//! Host-side (renderer-using) half of level loading.
//!
//! The pure-sim half — parsing `LoadedLevel`, building entity state,
//! populating lifts/masks/jump-zones — lives in `robin_engine`.  The
//! slow CPU-only decode steps (bzip2 decompress, mask composition)
//! produce `PreDecodedBackground` / `PreDecodedMinimap`, and this module
//! consumes those to upload GPU textures via the renderer.  Also
//! contains `draw_background`, which blits the cached background
//! surface to a target every frame.

use crate::host::Host;
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
use robin_engine::engine::{Ambiance, Engine, PANNEL_HEIGHT};
use robin_engine::minimap as engine_minimap;
use robin_engine::sbfile;
use robin_engine::sprite::BBox;
use robin_engine::sprite_variant::SpriteVariant;

fn decode_hackable_terrain_png(bytes: &[u8], path: &str) -> Result<Picture, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("failed to decode PNG header '{path}': {error}"))?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| format!("PNG '{path}' has no known output size"))?
    ];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("failed to decode PNG pixels '{path}': {error}"))?;
    let data = &buffer[..info.buffer_size()];
    let mut rgb565 = Vec::with_capacity(info.width as usize * info.height as usize * 2);
    let mut push_pixel = |r: u8, g: u8, b: u8| {
        let pixel = ((r as u16 & 0xf8) << 8) | ((g as u16 & 0xfc) << 3) | ((b as u16) >> 3);
        rgb565.extend_from_slice(&pixel.to_le_bytes());
    };
    match info.color_type {
        png::ColorType::Rgb => {
            for pixel in data.chunks_exact(3) {
                push_pixel(pixel[0], pixel[1], pixel[2]);
            }
        }
        png::ColorType::Rgba => {
            for pixel in data.chunks_exact(4) {
                push_pixel(pixel[0], pixel[1], pixel[2]);
            }
        }
        color_type => {
            return Err(format!(
                "hackable terrain PNG '{path}' uses unsupported color type {color_type:?}"
            ));
        }
    }
    let width = u16::try_from(info.width)
        .map_err(|_| format!("terrain PNG '{path}' is too wide: {}", info.width))?;
    let height = u16::try_from(info.height)
        .map_err(|_| format!("terrain PNG '{path}' is too tall: {}", info.height))?;
    Ok(Picture {
        width,
        height,
        pitch: width * 2,
        pixel_format: robin_assets::picture::PixelFormat::Rgb16,
        data: rgb565,
        palette: None,
    })
}

fn decode_occlusion_depth_png(
    bytes: &[u8],
    path: &str,
    expected_width: u16,
    expected_height: u16,
) -> Result<Vec<u16>, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("failed to decode occlusion-depth PNG '{path}': {error}"))?;
    let mut buffer = vec![
        0;
        reader.output_buffer_size().ok_or_else(|| format!(
            "occlusion-depth PNG '{path}' has no output size"
        ))?
    ];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("failed to decode occlusion-depth PNG '{path}': {error}"))?;
    if info.width != u32::from(expected_width) || info.height != u32::from(expected_height) {
        return Err(format!(
            "occlusion-depth PNG '{path}' is {}x{}, expected {}x{}",
            info.width, info.height, expected_width, expected_height
        ));
    }
    if info.color_type != png::ColorType::Grayscale || info.bit_depth != png::BitDepth::Sixteen {
        return Err(format!(
            "occlusion-depth PNG '{path}' must be 16-bit grayscale, got {:?} {:?}",
            info.color_type, info.bit_depth
        ));
    }
    let data = &buffer[..info.buffer_size()];
    Ok(data
        .chunks_exact(2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        .collect())
}

fn load_terrain_candidate(path: &str) -> Result<Option<Picture>, String> {
    let png_path = format!("{path}.png");
    if sbfile::SbFile::exists(&png_path) {
        let bytes = sbfile::SbFile::read_all(&png_path)
            .map_err(|error| format!("failed to read terrain PNG '{png_path}': {error}"))?;
        tracing::info!("Loading hackable terrain PNG: {png_path}");
        return decode_hackable_terrain_png(&bytes, &png_path).map(Some);
    }
    match sbfile::SbFile::open(path, sbfile::SB_FILE_READ) {
        Ok(mut file) => Picture::load_terrain_from_stream(&mut file)
            .map(Some)
            .map_err(|error| format!("failed to load terrain image '{path}': {error}")),
        Err(_) => Ok(None),
    }
}

/// Select the appropriate sprite-variant dictionaries for the engine's
/// current ambiance and adjust global shadow values on the host's
/// `FrameHolder`.  Lives host-side because the engine crate no longer
/// references `FrameHolder`.
pub fn initialize_sprite_variants(host: &mut Host, engine: &Engine) {
    let bypass_fog_sprites_crash = engine.sim_config().bypass_fog_sprites_crash;
    let fh = host.frame_holder_mut();
    // When the launcher flag `bypass_fog_sprites_crash` is on, drop both
    // Night and Fog dictionaries regardless of ambiance and skip the
    // shadow-value set — the renderer then falls back to
    // `SpriteVariant::Day` via `EngineInner::default_variant`.
    if bypass_fog_sprites_crash {
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
            if let Some(loaded) = load_terrain_candidate(path)? {
                picture = Some(loaded);
                break;
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

    let depth_candidates = [
        format!(
            "{}/{}/{}.occlusion-depth.png",
            level_directory, ambiance_dir, map_name
        ),
        format!("{}/Day/{}.occlusion-depth.png", level_directory, map_name),
        format!("{}/{}.occlusion-depth.png", level_directory, map_name),
    ];
    let mut occlusion_depth = None;
    for path in &depth_candidates {
        if !sbfile::SbFile::exists(path) {
            continue;
        }
        let bytes = sbfile::SbFile::read_all(path)
            .map_err(|error| format!("failed to read occlusion depth '{path}': {error}"))?;
        occlusion_depth = Some(decode_occlusion_depth_png(
            &bytes,
            path,
            picture.width,
            picture.height,
        )?);
        tracing::info!("Loaded continuous occlusion depth: {path}");
        break;
    }

    Ok(Some(PreDecodedBackground {
        width: picture.width,
        height: picture.height,
        pixels: bg_pixels,
        occlusion_depth,
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

    // Upload each mask's static binary alpha once. Masked draws rasterize it
    // into stencil so occluded sprite fragments never overwrite the scene.
    renderer.clear_mask_alpha_cache();
    let mask_count = engine.fast_grid().level.masks.len();
    for (idx, mask) in engine.fast_grid().level.masks.iter().enumerate() {
        assert!(
            renderer.upload_mask_alpha(idx as u32, &mask.bitmap, mask.width, mask.height),
            "invalid sprite mask {idx}: {}x{} bitmap has {} bytes",
            mask.width,
            mask.height,
            mask.bitmap.len()
        );
    }
    if mask_count > 0 {
        tracing::debug!("Uploaded {} mask alpha textures", mask_count);
    }
    if let Some(depth) = decoded.occlusion_depth.as_deref() {
        assert!(
            renderer.upload_occlusion_depth(depth, decoded.width, decoded.height),
            "invalid continuous occlusion depth {}x{} with {} values",
            decoded.width,
            decoded.height,
            depth.len()
        );
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

    let mut picture = None;
    for path in &candidates {
        match load_terrain_candidate(path) {
            Ok(Some(loaded)) => {
                picture = Some(loaded);
                break;
            }
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!("Failed to load minimap image: {error}");
                return None;
            }
        }
    }
    let picture = match picture {
        Some(picture) => picture,
        None => {
            tracing::warn!(
                "Unable to find minimap file {}.min in {}",
                map_name,
                level_directory
            );
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
/// Reads the saved minimap top-left from the active player profile in the
/// [`crate::host::ApplicationContext`] and forwards it to the engine so
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
    let profile = host
        .application_context
        .active_profile_snapshot()
        .unwrap_or_else(|error| panic!("minimap setup requires an active profile: {error}"));
    let saved_position = engine_coordinates::ScreenPoint::new(profile.minimap_x, profile.minimap_y);

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
    fn hackable_terrain_png_decodes_to_rgb565() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[0xff, 0x00, 0x00, 0x00, 0xff, 0x00])
                .unwrap();
        }
        let picture = decode_hackable_terrain_png(&bytes, "terrain.png").unwrap();
        assert_eq!((picture.width, picture.height), (2, 1));
        let pixels: &[u16] = bytemuck::cast_slice(&picture.data);
        assert_eq!(pixels, [0xf800, 0x07e0]);
    }

    #[test]
    fn continuous_occlusion_png_preserves_sixteen_bit_depth() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Sixteen);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[0x00, 0x00, 0x12, 0x34, 0xab, 0xcd, 0xff, 0xff])
                .unwrap();
        }
        let depth = decode_occlusion_depth_png(&bytes, "depth.png", 2, 2).unwrap();
        assert_eq!(depth, [0x0000, 0x1234, 0xabcd, 0xffff]);
    }

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

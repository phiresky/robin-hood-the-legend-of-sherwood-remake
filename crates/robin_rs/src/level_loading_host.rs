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
            for pixel in data.as_chunks::<3>().0 {
                push_pixel(pixel[0], pixel[1], pixel[2]);
            }
        }
        png::ColorType::Rgba => {
            for pixel in data.as_chunks::<4>().0 {
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
        .as_chunks::<2>()
        .0
        .iter()
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
    pre_decode_background_map_impl(
        map_name,
        ambiance_dir,
        level_directory,
        shipping,
        progress,
        false,
    )
}

/// [`pre_decode_background_map`] with `parallel` selecting rayon-parallel
/// JXL section decoding (see [`Picture::load_jxl_rgb565_parallel`] for the
/// threading contract — on wasm, parallel decode must run on a worker).
fn pre_decode_background_map_impl(
    map_name: &str,
    ambiance_dir: &str,
    level_directory: &str,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    progress: &mut dyn FnMut(assets_frame_holder::ProgressUpdate),
    parallel: bool,
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
            if let Some(bytes) = dd.raw_asset(key) {
                tracing::info!(
                    "Loading background map from shipping datadir: {} ({} bytes)",
                    key,
                    bytes.len()
                );
                let load = if parallel {
                    Picture::load_terrain_from_bytes_parallel
                } else {
                    Picture::load_terrain_from_bytes
                };
                match load(bytes) {
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

/// Probe the background map's pixel dimensions without decoding pixels,
/// using the same candidate order as [`pre_decode_background_map`].
///
/// `None` means the probe can't say (missing map, unreadable header, or an
/// empty map name); the caller must then wait for the full decode before
/// engine construction so the existing error path reports the failure.
pub fn probe_background_map_dims(
    map_name: &str,
    ambiance_dir: &str,
    level_directory: &str,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
) -> Option<(u16, u16)> {
    if map_name.is_empty() {
        return None;
    }
    let shipping_keys = [
        format!("levels/{}/{}.map", ambiance_dir, map_name).to_ascii_lowercase(),
        format!("levels/day/{}.map", map_name).to_ascii_lowercase(),
        format!("levels/{}.map", map_name).to_ascii_lowercase(),
    ];
    if let Some(dd) = shipping {
        for key in &shipping_keys {
            if let Some(bytes) = dd.raw_asset(key) {
                return Picture::terrain_dimensions(bytes).ok();
            }
        }
    }
    let disk_candidates = [
        format!("{}/{}/{}.map", level_directory, ambiance_dir, map_name),
        format!("{}/Day/{}.map", level_directory, map_name),
        format!("{}/{}.map", level_directory, map_name),
    ];
    for path in &disk_candidates {
        // Hackable PNG overlays take priority in `load_terrain_candidate`;
        // their pixel dimensions come straight from the PNG header.
        let png_path = format!("{path}.png");
        if sbfile::SbFile::exists(&png_path) {
            let bytes = sbfile::SbFile::read_all(&png_path).ok()?;
            let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            let reader = decoder.read_info().ok()?;
            let info = reader.info();
            return Some((
                u16::try_from(info.width).ok()?,
                u16::try_from(info.height).ok()?,
            ));
        }
        if let Ok(bytes) = sbfile::SbFile::read_all(path) {
            return Picture::terrain_dimensions(&bytes).ok();
        }
    }
    None
}

/// Background + minimap payloads decoded together off the loading path by
/// one [`PendingTerrainDecode`] job. Transient decode scratch — deliberately
/// no serde.
pub struct DecodedTerrainBitmaps {
    pub background: Result<Option<PreDecodedBackground>, String>,
    pub minimap: Option<PreDecodedMinimap>,
}

/// In-flight decode of the background map + minimap bitmaps.
///
/// Started right after the mission header names the map, joined as late as
/// the caller can afford (interactive missions join right before the GPU
/// upload, after the renderer exists), so the decode overlaps sprite-bank
/// install, script parse, engine construction, audio setup, and renderer
/// bring-up:
/// - native: a dedicated thread, rayon-parallel JXL section decode;
/// - wasm with an initialized `wasm-threads` pool: a rayon worker job
///   (parallel sections across the pool), joined by awaiting a oneshot so
///   the browser main thread never `atomics.wait`s;
/// - wasm otherwise: nothing runs at [`Self::start`]; the caller triggers
///   the synchronous fallback via [`Self::decode_inline_if_pending`] at the
///   same pre-engine point the old code decoded, preserving the
///   single-threaded behavior (loading bar included).
pub enum PendingTerrainDecode {
    /// Result already in hand (inline fallback decode, or a finished join).
    Ready(DecodedTerrainBitmaps),
    /// Single-threaded wasm fallback: nothing started yet. The decode runs
    /// inline at the same pre-engine point the old synchronous branch used
    /// (see [`Self::decode_inline_if_pending`]), so the loading bar behaves
    /// identically on non-cross-origin-isolated pages.
    #[cfg(target_arch = "wasm32")]
    Inline {
        map_name: String,
        ambiance_dir: String,
        level_directory: String,
        shipping: Option<std::sync::Arc<assets_shipping_datadir::ShippingDatadir>>,
    },
    #[cfg(not(target_arch = "wasm32"))]
    Thread(std::thread::JoinHandle<DecodedTerrainBitmaps>),
    /// Worker-pool job. Carries its own inputs so the probe-failure path
    /// ([`Self::join_now_or_redecode`]) can redecode inline without blocking
    /// on the worker.
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    Pool {
        receiver: robin_assets::wasm_threads::PoolReceiver<DecodedTerrainBitmaps>,
        map_name: String,
        ambiance_dir: String,
        level_directory: String,
        shipping: Option<std::sync::Arc<assets_shipping_datadir::ShippingDatadir>>,
    },
}

fn decode_terrain_bitmaps(
    map_name: &str,
    ambiance_dir: &str,
    level_directory: &str,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    progress: &mut dyn FnMut(assets_frame_holder::ProgressUpdate),
    parallel: bool,
) -> DecodedTerrainBitmaps {
    let background = pre_decode_background_map_impl(
        map_name,
        ambiance_dir,
        level_directory,
        shipping,
        progress,
        parallel,
    )
    .map_err(|e| format!("Background map load failed: {e}"));
    let minimap = pre_decode_minimap(
        map_name,
        ambiance_dir,
        level_directory,
        shipping,
        &mut |_| {},
    );
    DecodedTerrainBitmaps {
        background,
        minimap,
    }
}

impl PendingTerrainDecode {
    /// Start the decode: a dedicated thread on native, a rayon worker job on
    /// wasm when the `wasm-threads` pool is up, and the [`Self::Inline`]
    /// marker otherwise (single-threaded wasm decodes later, at the caller's
    /// pre-engine join point, exactly like the old synchronous branch).
    pub fn start(
        map_name: &str,
        ambiance_dir: &str,
        level_directory: &str,
        shipping: Option<std::sync::Arc<assets_shipping_datadir::ShippingDatadir>>,
    ) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let map_name = map_name.to_owned();
            let ambiance_dir = ambiance_dir.to_owned();
            let level_directory = level_directory.to_owned();
            let handle = std::thread::Builder::new()
                .name("terrain-decode".into())
                .spawn(move || {
                    decode_terrain_bitmaps(
                        &map_name,
                        &ambiance_dir,
                        &level_directory,
                        shipping.as_deref(),
                        &mut |_| {},
                        true,
                    )
                })
                .expect("failed to spawn terrain decode thread");
            Self::Thread(handle)
        }
        #[cfg(target_arch = "wasm32")]
        {
            #[cfg(feature = "wasm-threads")]
            if robin_assets::wasm_threads::pool_threads() > 0 {
                let receiver = {
                    let map_name = map_name.to_owned();
                    let ambiance_dir = ambiance_dir.to_owned();
                    let level_directory = level_directory.to_owned();
                    let shipping = shipping.clone();
                    robin_assets::wasm_threads::start_on_pool(move || {
                        // Serial inside the worker: the pool is here to get
                        // the decode OFF the main thread, not to split it.
                        // With jxl SIMD the whole map decodes in ~430 ms on
                        // one worker, while splitting it across the pool
                        // measured no faster and competes with the VQ chunk
                        // decode that is saturating those same workers
                        // during install.
                        decode_terrain_bitmaps(
                            &map_name,
                            &ambiance_dir,
                            &level_directory,
                            shipping.as_deref(),
                            &mut |_| {},
                            false,
                        )
                    })
                };
                return Self::Pool {
                    receiver,
                    map_name: map_name.to_owned(),
                    ambiance_dir: ambiance_dir.to_owned(),
                    level_directory: level_directory.to_owned(),
                    shipping,
                };
            }
            Self::Inline {
                map_name: map_name.to_owned(),
                ambiance_dir: ambiance_dir.to_owned(),
                level_directory: level_directory.to_owned(),
                shipping,
            }
        }
    }

    /// Run the single-threaded fallback decode if this is the [`Self::Inline`]
    /// marker (feeding the loading bar through `sync_progress`); all other
    /// variants pass through untouched. Callers invoke this at the same
    /// pre-engine point where the old wasm synchronous decode ran.
    pub fn decode_inline_if_pending(
        self,
        sync_progress: &mut dyn FnMut(assets_frame_holder::ProgressUpdate),
    ) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = sync_progress;
            self
        }
        #[cfg(target_arch = "wasm32")]
        match self {
            Self::Inline {
                map_name,
                ambiance_dir,
                level_directory,
                shipping,
            } => Self::Ready(decode_terrain_bitmaps(
                &map_name,
                &ambiance_dir,
                &level_directory,
                shipping.as_deref(),
                sync_progress,
                false,
            )),
            other => other,
        }
    }

    /// The result if it is already available without waiting, else `self`.
    pub fn try_take_ready(self) -> Result<DecodedTerrainBitmaps, Self> {
        match self {
            Self::Ready(decoded) => Ok(decoded),
            other => Err(other),
        }
    }

    /// Blocking join for callers that never defer (true-headless bootstrap).
    /// Panics on the pool variant — the browser main thread must never block
    /// on a worker; pool users join via [`Self::join`].
    pub fn join_blocking(self) -> DecodedTerrainBitmaps {
        match self.decode_inline_if_pending(&mut |_| {}) {
            Self::Ready(decoded) => decoded,
            #[cfg(not(target_arch = "wasm32"))]
            Self::Thread(handle) => handle.join().expect("terrain decode thread panicked"),
            #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
            Self::Pool { .. } => {
                unreachable!("pool-backed terrain decode must be joined asynchronously")
            }
            #[cfg(target_arch = "wasm32")]
            Self::Inline { .. } => {
                unreachable!("decode_inline_if_pending resolved the inline variant")
            }
        }
    }

    /// Join without blocking the wasm main thread: the native thread join
    /// still blocks (fine off-browser), the wasm pool variant awaits its
    /// oneshot, and the inline fallback decodes here serially.
    pub async fn join(self) -> DecodedTerrainBitmaps {
        match self.decode_inline_if_pending(&mut |_| {}) {
            Self::Ready(decoded) => decoded,
            #[cfg(not(target_arch = "wasm32"))]
            Self::Thread(handle) => handle.join().expect("terrain decode thread panicked"),
            #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
            Self::Pool { receiver, .. } => receiver
                .await
                .expect("terrain decode worker dropped its result"),
            #[cfg(target_arch = "wasm32")]
            Self::Inline { .. } => {
                unreachable!("decode_inline_if_pending resolved the inline variant")
            }
        }
    }

    /// Resolve the result *now* on the current thread. Used on the
    /// probe-failure path (missing/corrupt map), where engine construction
    /// cannot proceed without the decode outcome: joins where joining is
    /// legal, and on the wasm pool variant redecodes serially inline — the
    /// duplicate work only happens for a map that is about to fail the load.
    pub fn join_now_or_redecode(
        self,
        sync_progress: &mut dyn FnMut(assets_frame_holder::ProgressUpdate),
    ) -> DecodedTerrainBitmaps {
        match self.decode_inline_if_pending(sync_progress) {
            Self::Ready(decoded) => decoded,
            #[cfg(not(target_arch = "wasm32"))]
            Self::Thread(handle) => handle.join().expect("terrain decode thread panicked"),
            #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
            Self::Pool {
                receiver,
                map_name,
                ambiance_dir,
                level_directory,
                shipping,
            } => {
                // Redecode serially on this thread; the in-flight worker
                // result is abandoned (its send just fails).
                drop(receiver);
                decode_terrain_bitmaps(
                    &map_name,
                    &ambiance_dir,
                    &level_directory,
                    shipping.as_deref(),
                    sync_progress,
                    false,
                )
            }
            #[cfg(target_arch = "wasm32")]
            Self::Inline { .. } => {
                unreachable!("decode_inline_if_pending resolved the inline variant")
            }
        }
    }
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
            if let Some(bytes) = dd.raw_asset(key) {
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

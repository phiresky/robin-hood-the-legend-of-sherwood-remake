//! Loading screen state machine and sand dissolve effect.
//!
//! Manages the loading screen display including a "sand dissolve"
//! transition between an initial and final background image driven by a
//! grayscale height field.
//!
//! Captures:
//! - Progress tracking (level-based)
//! - Height field generation from pixel data (the sand dissolve mask)
//! - Sand dissolve threshold computation
//! - Data file path resolution
//! - Version string formatting
//!
//! Rendering uses GPU quads/textures. The dissolve itself is a shader that
//! samples the initial picture, final picture, and normalized mask texture.

use robin_assets::picture as assets_picture;
use robin_assets::picture::Picture;
use robin_assets::shipping_datadir as assets_shipping_datadir;
use robin_engine::graphic_config::TextureScaleMode;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::gfx_types::GameEvent;
use crate::loading_dissolve_gpu::LoadingDissolveTextures;
use crate::native_font::Font;
use crate::renderer::Renderer;
use robin_engine::sbfile::SbFile;

fn shipping_loading_pak_pictures(
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    pak_path: &str,
) -> Option<Vec<Picture>> {
    let dd = shipping?;
    let key = shipping_pak_key(pak_path);
    let encoded = dd.pak_files.get(&key)?;
    let mut pictures = Vec::with_capacity(encoded.len());
    for (idx, pic) in encoded.iter().enumerate() {
        match pic.decode() {
            Ok(decoded) => pictures.push(decoded),
            Err(e) => {
                tracing::warn!("Loading screen: shipping pak '{key}' picture {idx}: {e}");
                return None;
            }
        }
    }
    tracing::info!("Loading screen: loaded '{key}' from shipping datadir");
    Some(pictures)
}

fn shipping_pak_key(path: &str) -> String {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    if let Some((_, tail)) = normalized.rsplit_once("/data/") {
        return tail.to_string();
    }
    normalized
        .strip_prefix("data/")
        .unwrap_or(&normalized)
        .to_string()
}

// ---------------------------------------------------------------------------
// HeightField — sand dissolve mask
// ---------------------------------------------------------------------------

/// Grayscale height field used for the sand dissolve transition effect.
///
/// Each pixel has a normalized height value 0..=255. During rendering, pixels
/// whose height exceeds the current threshold show the final (loaded) image;
/// the rest show the initial (unloaded) image. As loading progresses the
/// threshold decreases, revealing more of the final image in a sand-like
/// dissolve pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeightField {
    /// Normalized height values (0..=255), stored row-major.
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl HeightField {
    /// Generate a height field from raw 8-bit grayscale pixel data.
    ///
    /// The values are normalized so the darkest pixel maps to 0 and the
    /// brightest to 255.
    ///
    /// # Panics
    /// Panics if `data.len() != width * height`.
    pub fn from_grayscale(data: &[u8], width: u32, height: u32) -> Self {
        let expected = (width as usize) * (height as usize);
        assert_eq!(
            data.len(),
            expected,
            "grayscale data length {} != width*height {}",
            data.len(),
            expected
        );

        let (mut min_h, mut max_h) = (255u8, 0u8);
        for &v in data {
            min_h = min_h.min(v);
            max_h = max_h.max(v);
        }

        let range = (max_h - min_h) as f32;
        let normalizer = if range > 0.0 { 255.0 / range } else { 0.0 };

        let normalized: Vec<u8> = data
            .iter()
            .map(|&v| ((v - min_h) as f32 * normalizer) as u8)
            .collect();

        Self {
            data: normalized,
            width,
            height,
        }
    }

    /// Generate a height field from 24-bit RGB pixel data.
    ///
    /// Converts to luminance using the weighted formula
    /// `(R * 39 + G * 50 + B * 11) / 100`, then normalizes.
    ///
    /// # Panics
    /// Panics if `rgb_data.len() != width * height * 3`.
    pub fn from_rgb(rgb_data: &[u8], width: u32, height: u32) -> Self {
        let expected = (width as usize) * (height as usize) * 3;
        assert_eq!(
            rgb_data.len(),
            expected,
            "RGB data length {} != width*height*3 {}",
            rgb_data.len(),
            expected
        );

        let grayscale: Vec<u8> = rgb_data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|px| {
                let (r, g, b) = (px[0] as u32, px[1] as u32, px[2] as u32);
                ((r * 39 + g * 50 + b * 11) / 100) as u8
            })
            .collect();

        Self::from_grayscale(&grayscale, width, height)
    }

    /// Generate a height field from RGB565 (16-bit) pixel data.
    ///
    /// - R = bits 15..11, shifted to 8-bit
    /// - G = bits 10..5,  shifted to 8-bit
    /// - B = bits 4..0,   shifted to 8-bit
    ///
    /// # Panics
    /// Panics if `pixel_data.len() != width * height`.
    pub fn from_rgb565(pixel_data: &[u16], width: u32, height: u32) -> Self {
        let expected = (width as usize) * (height as usize);
        assert_eq!(
            pixel_data.len(),
            expected,
            "RGB565 data length {} != width*height {}",
            pixel_data.len(),
            expected
        );

        let grayscale: Vec<u8> = pixel_data
            .iter()
            .map(|&color| {
                let r = ((color & 0xF800) >> 8) as u32;
                let g = ((color & 0x07E0) >> 3) as u32;
                let b = ((color & 0x001F) << 3) as u32;
                ((r * 39 + g * 50 + b * 11) / 100) as u8
            })
            .collect();

        Self::from_grayscale(&grayscale, width, height)
    }

    /// Generate a height field from RGB555 (15-bit) pixel data.
    ///
    /// - R = bits 14..10, shifted to 8-bit
    /// - G = bits 9..5,   shifted to 8-bit
    /// - B = bits 4..0,   shifted to 8-bit
    ///
    /// # Panics
    /// Panics if `pixel_data.len() != width * height`.
    pub fn from_rgb555(pixel_data: &[u16], width: u32, height: u32) -> Self {
        let expected = (width as usize) * (height as usize);
        assert_eq!(
            pixel_data.len(),
            expected,
            "RGB555 data length {} != width*height {}",
            pixel_data.len(),
            expected
        );

        let grayscale: Vec<u8> = pixel_data
            .iter()
            .map(|&color| {
                let r = ((color & 0x7C00) >> 7) as u32;
                let g = ((color & 0x03E0) >> 2) as u32;
                let b = ((color & 0x001F) << 3) as u32;
                ((r * 39 + g * 50 + b * 11) / 100) as u8
            })
            .collect();

        Self::from_grayscale(&grayscale, width, height)
    }

    /// Compute the sand dissolve threshold for a given progress value.
    ///
    /// Maps progress (0.0 = nothing loaded, 1.0 = fully loaded) to a pixel
    /// threshold using a quadratic curve:
    ///
    /// ```text
    /// relative_remaining = 1 - progress
    /// threshold = relative_remaining^2 * 256
    /// ```
    ///
    /// - At progress 0.0: threshold = 256 (all pixels show initial image)
    /// - At progress 1.0: threshold = 0   (all pixels show final image)
    pub fn compute_threshold(progress: f32) -> u32 {
        let remaining = 1.0 - progress.clamp(0.0, 1.0);
        (remaining * remaining * 256.0) as u32
    }

    /// For a given threshold, produce a per-pixel mask.
    ///
    /// Returns `true` where the final image should be shown (height > threshold),
    /// `false` where the initial image should be shown.
    pub fn compute_mask(&self, threshold: u32) -> Vec<bool> {
        self.data.iter().map(|&h| (h as u32) > threshold).collect()
    }
}

// ---------------------------------------------------------------------------
// LoadingScreen — full state machine
// ---------------------------------------------------------------------------

/// Loading screen state machine.
///
/// Tracks loading progress using a level-based system (`update`,
/// `increment` with absolute levels and deltas). Manages the sand
/// dissolve effect state via an optional [`HeightField`].
///
/// This struct captures only the logical state; rendering is delegated
/// to the renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadingScreen {
    /// Maximum progress level (set during initialization).
    pub max_level: f32,
    /// Current progress level (incremented during loading).
    pub current_level: f32,
    /// Resource string ID for the current loading status text.
    pub string_id: u32,
    /// Free-form status text shown below the sand-dissolve bar. Fed
    /// direct strings from the host-side loader.
    pub status_text: Option<String>,
    /// Whether the loading screen is currently active.
    pub active: bool,
    /// Screen width in pixels.
    pub screen_width: u32,
    /// Screen height in pixels.
    pub screen_height: u32,
    /// The height field for the sand dissolve effect.
    /// Skipped during serialization (regenerated from image data on load).
    #[serde(skip)]
    pub height_field: Option<HeightField>,
}

impl Default for LoadingScreen {
    fn default() -> Self {
        Self {
            max_level: 1.0,
            current_level: 0.0,
            string_id: 0,
            status_text: None,
            active: false,
            screen_width: 0,
            screen_height: 0,
            height_field: None,
        }
    }
}

impl LoadingScreen {
    /// Initialize the loading screen for a new loading sequence.
    ///
    /// Resets progress to zero and activates the screen. The height field
    /// should be set separately via [`set_height_field`](Self::set_height_field)
    /// after the dissolve images have been loaded.
    pub fn initialize(&mut self, screen_width: u32, screen_height: u32, max_level: f32) {
        self.max_level = max_level;
        self.current_level = 0.0;
        self.string_id = 0;
        self.status_text = None;
        self.active = true;
        self.screen_width = screen_width;
        self.screen_height = screen_height;
        self.height_field = None;
    }

    /// Attach a height field for the sand dissolve effect.
    pub fn set_height_field(&mut self, height_field: HeightField) {
        self.height_field = Some(height_field);
    }

    /// Set the free-form status text shown below the sand-dissolve bar.
    pub fn set_status_text(&mut self, text: Option<String>) {
        self.status_text = text;
    }

    fn set_level_monotonic(&mut self, level: f32) {
        if level < self.current_level {
            tracing::warn!(
                from = self.current_level,
                to = level,
                "Loading screen progress update would move backwards"
            );
            return;
        }
        self.current_level = level;
    }

    /// Update progress to an absolute level and set the status string ID.
    ///
    /// Loading progress is display state for one active load sequence, so it
    /// must be monotonic. Use [`initialize`](Self::initialize) to start a new
    /// sequence at zero.
    pub fn update(&mut self, string_id: u32, level: f32) {
        self.string_id = string_id;
        self.set_level_monotonic(level);
    }

    /// Update progress to an absolute level (keeping current string).
    pub fn update_level(&mut self, level: f32) {
        self.set_level_monotonic(level);
    }

    /// Increment progress by a delta and set the status string ID.
    pub fn increment(&mut self, string_id: u32, delta: f32) {
        self.string_id = string_id;
        self.current_level += delta;
    }

    /// Increment progress by a delta (keeping current string).
    pub fn increment_level(&mut self, delta: f32) {
        self.current_level += delta;
    }

    /// Normalized progress in `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        if self.max_level <= 0.0 {
            return 0.0;
        }
        (self.current_level / self.max_level).clamp(0.0, 1.0)
    }

    /// Compute the current sand dissolve threshold based on loading progress.
    ///
    /// Returns a value in 0..=256. Decreases as loading progresses.
    pub fn sand_threshold(&self) -> u32 {
        HeightField::compute_threshold(self.progress())
    }

    /// Close the loading screen, releasing the height field.
    ///
    /// After this call, [`is_active`](Self::is_active) returns `false`.
    pub fn close(&mut self) {
        self.active = false;
        self.height_field = None;
    }

    /// Whether the loading screen is currently active.
    pub fn is_active(&self) -> bool {
        self.active
    }
}

// ---------------------------------------------------------------------------
// Data file resolution
// ---------------------------------------------------------------------------

/// Resolve the loading screen data file path for a given mission and ambience.
///
/// Tries the mission-specific path first:
///   `{level_dir}/{ambience:02}/{proto_filename}.pak`
///
/// Falls back to the generic loading screen:
///   `{interface_dir}/Loading.pak`
///
/// # Panics
/// Panics if neither file exists.
pub fn get_data_file(
    level_dir: &str,
    interface_dir: &str,
    proto_filename: &str,
    ambience: u32,
) -> PathBuf {
    // Try mission-specific loading screen
    let mission_file = PathBuf::from(format!(
        "{}/{:02}/{}.pak",
        level_dir, ambience, proto_filename
    ));

    if mission_file.exists() {
        return mission_file;
    }

    // Fall back to generic loading screen
    let default_file = PathBuf::from(format!("{}/Loading.pak", interface_dir));

    if default_file.exists() {
        return default_file;
    }

    panic!(
        "Loading screen: unable to find data file. Tried '{}' and '{}'",
        mission_file.display(),
        default_file.display()
    );
}

/// Format a version string as `"v{major}.{minor} {release_name}"`.
pub fn format_version_string(major: u16, minor: u16, release_name: &str) -> String {
    format!("v{major}.{minor} {release_name}")
}

// ---------------------------------------------------------------------------
// LoadingScreenRenderer — active rendering during mission load
// ---------------------------------------------------------------------------

/// Convert little-endian u8 pixel data to u16 words.
fn bytes_to_u16_pixels(data: &[u8]) -> Vec<u16> {
    data.as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// Active loading screen renderer. Owns a temporary [`Renderer`] and the
/// uploaded loading-screen images.
///
/// Created at the start of mission loading, dropped before the game renderer
/// is constructed. The renderer scales the loading-screen images to fill the
/// window regardless of their native resolution.
pub struct LoadingScreenRenderer {
    state: LoadingScreen,
    renderer: Renderer,
    /// Persistent GPU textures for the initial/final/mask dissolve triplet.
    loading_dissolve: LoadingDissolveTextures,
    /// "Version" font for the version/demo overlay text.
    version_font: Option<Font>,
    /// "MenuText" font (same as the main-menu profile sidebar that
    /// shows "Difficulty level: Hard") for the status line below the
    /// sand-dissolve bar. Falls back to the version font when MenuText
    /// isn't resolvable in the current datadir.
    status_font: Option<Font>,
    /// Which datadir family is currently loaded, for the version overlay.
    datadir_kind: LoadingDatadirKind,
    /// Ceiling on `state.current_level` for the current phase. Intra-phase
    /// `increment` calls clamp to this so the bar can't overshoot the
    /// next phase's start target. `set_status` bumps this to the new
    /// phase's end target and snaps `current_level` forward if ticks
    /// under-shot.
    phase_ceiling: f32,
    /// Whether the game window currently has keyboard focus. When the
    /// window is defocused during mission load we skip rendering
    /// instead of busy-flipping the swapchain. `drain_events` flips
    /// this from `GameEvent::WindowFocusChanged`. Initialised `true`
    /// because the window is presumed focused at construction; the OS
    /// sends a `Focused(false)` event if that's wrong.
    window_focused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadingDatadirKind {
    FullGame,
    DemoI,
    DemoII,
}

impl LoadingScreenRenderer {
    /// Create a loading screen from a `.pak` file containing three
    /// sequential 16-bit picture images (initial, final, height-mask).
    ///
    /// Returns `None` if the `.pak` file or any image cannot be loaded (the
    /// caller should simply skip the loading screen in that case).
    pub fn new(
        window: &crate::window::GameWindow,
        shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
        pak_path: &str,
        datadir_kind: LoadingDatadirKind,
        max_level: f32,
        scale_mode: TextureScaleMode,
    ) -> Option<Self> {
        if let Some(pictures) = shipping_loading_pak_pictures(shipping, pak_path) {
            if pictures.len() >= 3 {
                return Self::from_pictures(
                    window,
                    &pictures[0],
                    &pictures[1],
                    &pictures[2],
                    datadir_kind,
                    max_level,
                    scale_mode,
                );
            }
            tracing::warn!(
                "Loading screen: shipping pak '{pak_path}' has {} pictures, expected 3",
                pictures.len()
            );
        }

        let mut file = match SbFile::open(pak_path, 0) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Loading screen: cannot open '{}': error {}", pak_path, e);
                return None;
            }
        };

        // The .pak contains 3 sequential 16-bit picture images.
        let pic_initial = Picture::load_sixteen_from_stream(&mut file)
            .map_err(|e| tracing::warn!("Loading screen: initial picture: {e}"))
            .ok()?;
        let pic_final = Picture::load_sixteen_from_stream(&mut file)
            .map_err(|e| tracing::warn!("Loading screen: final picture: {e}"))
            .ok()?;
        let pic_mask = Picture::load_sixteen_from_stream(&mut file)
            .map_err(|e| tracing::warn!("Loading screen: height mask picture: {e}"))
            .ok()?;

        Self::from_pictures(
            window,
            &pic_initial,
            &pic_final,
            &pic_mask,
            datadir_kind,
            max_level,
            scale_mode,
        )
    }

    fn from_pictures(
        window: &crate::window::GameWindow,
        pic_initial: &Picture,
        pic_final: &Picture,
        pic_mask: &Picture,
        datadir_kind: LoadingDatadirKind,
        max_level: f32,
        scale_mode: TextureScaleMode,
    ) -> Option<Self> {
        let width = pic_initial.width;
        let height = pic_initial.height;

        if width == 0 || height == 0 {
            tracing::warn!("Loading screen: zero-sized images");
            return None;
        }

        let initial_pixels = bytes_to_u16_pixels(&pic_initial.data);
        let final_pixels = bytes_to_u16_pixels(&pic_final.data);
        let mask_pixels = bytes_to_u16_pixels(&pic_mask.data);
        // `Picture::load_sixteen_from_stream` is the RGB565 path used by
        // the shipped loading paks. Keep this explicit because the shader
        // upload depends on the RGB565 layout.
        assert_eq!(
            pic_initial.pixel_format,
            assets_picture::PixelFormat::Rgb16,
            "loading-screen initial picture must be RGB565"
        );
        assert_eq!(
            pic_final.pixel_format,
            assets_picture::PixelFormat::Rgb16,
            "loading-screen final picture must be RGB565"
        );
        assert_eq!(
            pic_mask.pixel_format,
            assets_picture::PixelFormat::Rgb16,
            "loading-screen mask must be RGB565"
        );
        let height_field = HeightField::from_rgb565(&mask_pixels, width as u32, height as u32);
        // Create a renderer at the image's native resolution. Presentation
        // handles aspect-correct scaling (letterbox) to the actual window.
        let mut renderer = Renderer::new(window, width, height, scale_mode);
        let loading_dissolve = renderer.create_loading_dissolve_textures(
            width as u32,
            height as u32,
            &initial_pixels,
            &final_pixels,
            &height_field,
        )?;

        // Paint the framebuffer black and present *before* loading any
        // pictures/fonts, so the previous frame (main menu, window-
        // manager bg, etc.) doesn't bleed through during the multi-
        // hundred-ms pak/font load window.
        renderer.begin_gpu_frame_clear();
        renderer.present();

        let mut state = LoadingScreen::default();
        state.initialize(width as u32, height as u32, max_level);
        state.set_height_field(height_field);

        // Try to load the "Version" font for the overlay text. legacy implementation stores it
        // behind `SBFont`, so keep either native bitmap or TrueType resolves
        // and dispatch at render time.
        let font_config = crate::native_font::load_font_config().ok();
        let load_font = |name: &str| -> Option<Font> {
            let cfg = font_config.as_ref()?;
            match crate::native_font::load_font_by_name(cfg, name) {
                Ok(font) if font.is_renderable() => Some(font),
                Ok(Font::TrueType(tt)) => {
                    tracing::info!(
                        "Loading screen: {name} TrueType font '{}' has no loaded face",
                        tt.truetype_name_str()
                    );
                    None
                }
                Ok(Font::Native(f)) => Some(Font::Native(f)),
                Err(e) => {
                    tracing::debug!("Loading screen: {name} font not available: {e}");
                    None
                }
            }
        };
        let version_font = load_font("Version");
        let status_font = load_font("MenuText").or_else(|| load_font("Version"));
        tracing::info!(
            "Loading screen initialized: {}x{}, datadir={:?}",
            width,
            height,
            datadir_kind
        );

        Some(Self {
            state,
            renderer,
            loading_dissolve,
            version_font,
            status_font,
            datadir_kind,
            phase_ceiling: 0.0,
            window_focused: true,
        })
    }

    /// Start a new loading phase with `text` shown below the bar, targeted
    /// to reach `target_progress` (0..=1) by the phase's end.
    ///
    /// The bar snaps forward to the previous phase's ceiling if intra-phase
    /// `increment` ticks under-shot, then raises the ceiling so this phase's
    /// ticks can climb toward the new target without overshooting. Targets
    /// are calibrated from measured phase durations so the bar advances
    /// roughly linearly with wall-clock time — see the `[loading]` info
    /// logs (`RUST_LOG=info`) to re-measure and retune.
    pub fn set_status(&mut self, text: impl Into<String>, target_progress: f32) {
        let text = text.into();
        let new_ceiling = target_progress.clamp(0.0, 1.0) * self.state.max_level;
        // Snap forward: intra-phase ticks in the previous phase may have
        // stopped short of the ceiling. Jump up to it so the bar reflects
        // real progression at each phase boundary.
        if self.state.current_level < self.phase_ceiling {
            self.state.current_level = self.phase_ceiling;
        }
        if new_ceiling < self.phase_ceiling {
            tracing::warn!(
                from = self.phase_ceiling,
                to = new_ceiling,
                text,
                "Loading screen phase target would move backwards"
            );
        } else {
            self.phase_ceiling = new_ceiling;
        }
        tracing::info!(progress = self.state.progress(), "[loading] {text}");
        self.state.set_status_text(Some(text));
        self.refresh();
    }

    /// Drain pending window events (especially WM resizes) and snap the window
    /// to a supported 4:3 resolution.  Must be called periodically during
    /// long-running mission loads, otherwise resize events pile up in the
    /// queue and the canvas state goes stale.
    pub fn drain_events(&mut self, event_pump: &mut crate::window::GameWindow) {
        // GameWindow.poll_events handles resize internally now (it
        // reconfigures the wgpu surface on Resized), so the
        // surface dimensions are updated there. We still need to scan for
        // focus changes — when
        // the WM defocuses the game during a long load we stop
        // pushing frames until focus returns (`refresh` short-circuits
        // while `window_focused == false`).
        for ev in event_pump.poll_events() {
            if let GameEvent::WindowFocusChanged(focused) = ev {
                self.window_focused = focused;
            }
        }
    }

    /// Increment progress by `delta` and re-render, clamped to the current
    /// phase's ceiling so intra-phase ticks can't overshoot the next
    /// phase's start target.
    pub fn increment(&mut self, delta: f32) {
        self.state.increment_level(delta);
        if self.state.current_level > self.phase_ceiling {
            self.state.current_level = self.phase_ceiling;
        }
        self.refresh();
    }

    /// Set progress to an absolute level and re-render.
    pub fn update(&mut self, level: f32) {
        self.state.update_level(level);
        self.refresh();
    }

    /// Render the current loading screen state to the display.
    ///
    /// Composites initial/final images via the sand dissolve threshold,
    /// overlays version text, and flips to screen.
    ///
    /// Skips drawing when the window is unfocused — the caller pumps
    /// events via `drain_events`, so focus regain naturally unsticks
    /// subsequent refreshes.
    pub fn refresh(&mut self) {
        if !self.window_focused {
            return;
        }
        let w = self.state.screen_width as usize;
        let h = self.state.screen_height as usize;

        self.renderer.begin_gpu_frame_clear();
        self.renderer
            .render_loading_dissolve(&self.loading_dissolve, self.state.sand_threshold());

        // Overlay version / demo text
        self.render_version_text(w);

        // Phase status line centred near the bottom.
        self.render_status_text(w, h);

        self.renderer.present();
    }

    /// Render the current phase status ("Loading sprite bank…" etc.)
    /// centred horizontally near the bottom of the screen.
    fn render_status_text(&mut self, screen_w: usize, screen_h: usize) {
        let font = match self.status_font {
            Some(ref f) => f,
            None => return,
        };
        let Some(text) = self.state.status_text.as_deref() else {
            return;
        };
        if text.is_empty() {
            return;
        }

        let tw = font.text_width(text);
        let fh = font.height() as i32;
        let tx = (screen_w as i32 - tw) / 2;
        let ty = screen_h as i32 - 60 - fh / 2;

        match font {
            Font::Native(native) => self.renderer.render_text_argb(native, text, tx, ty),
            Font::TrueType(tt) => self.renderer.render_text_truetype(tt, text, tx, ty),
        }
    }

    /// Render version text right-aligned near the top of the screen,
    /// within the bounding box (0, 0, screen_w, 100).
    fn render_version_text(&mut self, screen_w: usize) {
        let font = match self.version_font {
            Some(ref f) => f,
            None => return,
        };

        let text = loading_version_text(self.datadir_kind);

        let tw = font.text_width(&text);
        let fh = font.height() as i32;
        // Right-aligned within the top 100px band, vertically centered
        let tx = screen_w as i32 - tw - 4; // small right margin
        let ty = (100 - fh) / 2; // centered in 0..100 band

        match font {
            Font::Native(native) => self.renderer.render_text_argb(native, &text, tx, ty),
            Font::TrueType(tt) => self.renderer.render_text_truetype(tt, &text, tx, ty),
        }
    }

    /// Close and consume the loading screen, dropping the renderer.
    pub fn close(mut self) {
        self.state.close();
        // Renderer is dropped here, freeing its GPU resources.
    }
}

fn loading_version_text(datadir_kind: LoadingDatadirKind) -> String {
    let base = crate::version::version_label();
    match datadir_kind {
        LoadingDatadirKind::FullGame => base,
        LoadingDatadirKind::DemoI => format!("{base} DEMO I"),
        LoadingDatadirKind::DemoII => format!("{base} DEMO II"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "loading_screen_tests.rs"]
mod tests;

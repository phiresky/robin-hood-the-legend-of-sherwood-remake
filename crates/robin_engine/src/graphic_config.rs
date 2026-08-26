//! Graphics configuration (resolution, display toggles, etc.).
//!
//! The struct is `#[repr(C)]` so it can be shared across the C ABI; the
//! first six fields preserve the original on-disk layout.

use serde::{Deserialize, Serialize};

/// Per-profile graphics settings.
///
/// Fields `display_anim` through `resolution_y` preserve the original
/// on-disk layout (display-anim flag, display-shadow flag, framed
/// view-cone flag, display-titbits flag, then resolution X/Y as floats).
/// Additional fields are appended for the Rust-side feature set.
#[repr(C)]
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct GraphicConfig {
    // --- ABI-compatible fields (must remain first, in this order) ---
    pub display_anim: bool,
    pub display_shadow: bool,
    pub framed_view_cone: bool,
    pub display_titbits: bool,
    pub resolution_x: f32,
    pub resolution_y: f32,

    // --- Rust-only fields (appended after the legacy layout) ---
    pub fullscreen: bool,
    pub hardware_cursor: bool,
    /// Texture scaling mode for the game framebuffer.
    /// `Nearest` gives a sharp pixelated look (original), `Linear` is smooth/blurry.
    #[serde(default = "default_scale_mode")]
    pub scale_mode: TextureScaleMode,
    /// Relative path under `third_party/slang-shaders` for the selected
    /// RetroArch `.slangp` preset when `scale_mode == RetroArch`.
    #[serde(default = "default_shader_preset")]
    pub shader_preset: String,
    /// Apply the generated fog/night sprite variant to every Day-based world
    /// sprite that the original game leaves untinted. Animation assets that
    /// already contain ambiance-specific pixels are left untouched.
    #[serde(default)]
    pub apply_fog_to_all_sprites: bool,
}

/// Serializable texture scaling mode.
///
/// `Linear` is the default and the option most people expect when the window
/// is upscaled beyond the game's native resolution. `PixelArt` keeps pixels
/// crisp while avoiding the wobbly artifacts plain nearest-neighbor produces
/// at non-integer scales.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "lowercase")]
pub enum TextureScaleMode {
    /// Nearest-neighbor sampling — sharpest but shows scaling artifacts at
    /// non-integer ratios.
    Nearest,
    /// Nearest with improved sampling for pixel art; avoids the "wobble"
    /// plain nearest has at fractional scales.
    /// `pixel_art` + the legacy `"nearest"` profile key both alias here
    /// so pre-existing profiles migrate cleanly.
    PixelArt,
    /// GPU-shader sharp-bilinear (blargg).  Nearest-neighbor to the
    /// nearest integer multiple, then bilinear on the sub-pixel
    /// remainder — crisp axis-aligned edges, no wobble at fractional
    /// scales, no interior blur.
    SharpBilinear,
    /// Bilinear filtering.
    #[default]
    Linear,
    /// GPU-shader bicubic (Mitchell–Netravali).
    Bicubic,
    /// GPU-shader Lanczos-2 (hand-unrolled 4×4 grid).
    Lanczos,
    /// GPU-shader CUT3 — Cheap Upscaling via Triangulation
    /// (swordfish90).  2×2 neighbourhood, diagonal chosen by luma,
    /// barycentric blend inside the triangle the subpixel falls in.
    Cut3,
    /// GPU-shader Scale2x (Andrea Mazzoleni).  **Broken on RADV
    /// (Mesa ≤26.0)** — caused a GPU reset in the previous rendering
    /// backend. Hidden from the UI until it is validated with wgpu or Mesa
    /// ships a fix.
    Scale2x,
    /// GPU-shader Scale3x. Same RADV crash as Scale2x.
    Scale3x,
    /// GPU-shader xBR level 1 (Hyllian). Same RADV crash.
    XbrLv1,
    /// User-selected upstream libretro `.slangp` preset.
    RetroArch,
}

impl TextureScaleMode {
    /// Whether this mode needs a custom fragment shader for the final
    /// target→backbuffer blit. Non-shader modes use a wgpu sampler directly.
    pub fn needs_shader(self) -> bool {
        matches!(
            self,
            Self::SharpBilinear
                | Self::Bicubic
                | Self::Lanczos
                | Self::Cut3
                | Self::Scale2x
                | Self::Scale3x
                | Self::XbrLv1
                | Self::RetroArch
        )
    }

    /// Human-readable label for the Options UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Nearest => "Nearest",
            Self::PixelArt => "Pixel Art",
            Self::SharpBilinear => "Sharp Bilinear",
            Self::Linear => "Linear",
            Self::Bicubic => "Bicubic",
            Self::Lanczos => "Lanczos",
            Self::Cut3 => "CUT3",
            Self::Scale2x => "Scale2x",
            Self::Scale3x => "Scale3x",
            Self::XbrLv1 => "xBR lv1",
            Self::RetroArch => "RetroArch Shader",
        }
    }

    /// All modes in UI order (sharp → soft → shader-based).  Scale2x
    /// / Scale3x / xBR-lv1 are intentionally omitted — they compile
    /// fine but GPU-reset inside `canvas.present()` on the Mesa/RADV
    /// Vulkan driver we tested on.  Root cause not nailed down, so
    /// hiding them from the UI until the pipeline is reworked.
    pub const ALL: &'static [Self] = &[
        Self::Nearest,
        Self::PixelArt,
        Self::SharpBilinear,
        Self::Linear,
        Self::Bicubic,
        Self::Lanczos,
        Self::Cut3,
        Self::RetroArch,
    ];
}

fn default_scale_mode() -> TextureScaleMode {
    TextureScaleMode::default()
}

fn default_shader_preset() -> String {
    String::new()
}

impl Default for GraphicConfig {
    fn default() -> Self {
        Self {
            display_anim: true,
            display_shadow: true,
            framed_view_cone: false,
            display_titbits: true,
            resolution_x: 800.0,
            resolution_y: 600.0,
            fullscreen: false,
            hardware_cursor: true,
            scale_mode: TextureScaleMode::default(),
            shader_preset: default_shader_preset(),
            apply_fog_to_all_sprites: true,
        }
    }
}

impl GraphicConfig {
    /// Set the display resolution.
    pub fn set_resolution(&mut self, x: f32, y: f32) {
        self.resolution_x = x;
        self.resolution_y = y;
    }

    /// Whether the game is running in fullscreen mode.
    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// Toggle between fullscreen and windowed mode.
    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let cfg = GraphicConfig::default();
        assert!(cfg.display_anim);
        assert!(cfg.display_shadow);
        assert!(!cfg.framed_view_cone);
        assert!(cfg.display_titbits);
        assert_eq!(cfg.resolution_x, 800.0);
        assert_eq!(cfg.resolution_y, 600.0);
        assert!(!cfg.fullscreen);
        assert!(cfg.hardware_cursor);
        assert!(cfg.apply_fog_to_all_sprites);
    }

    #[test]
    fn set_resolution() {
        let mut cfg = GraphicConfig::default();
        cfg.set_resolution(1920.0, 1080.0);
        assert_eq!(cfg.resolution_x, 1920.0);
        assert_eq!(cfg.resolution_y, 1080.0);
    }

    #[test]
    fn toggle_fullscreen() {
        let mut cfg = GraphicConfig::default();
        assert!(!cfg.is_fullscreen());
        cfg.toggle_fullscreen();
        assert!(cfg.is_fullscreen());
        cfg.toggle_fullscreen();
        assert!(!cfg.is_fullscreen());
    }

    #[test]
    fn serde_roundtrip() {
        let mut cfg = GraphicConfig::default();
        cfg.set_resolution(1280.0, 720.0);
        cfg.toggle_fullscreen();
        cfg.apply_fog_to_all_sprites = true;

        let json = serde_json::to_string(&cfg).unwrap();
        let restored: GraphicConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.resolution_x, 1280.0);
        assert_eq!(restored.resolution_y, 720.0);
        assert!(restored.fullscreen);
        assert!(restored.hardware_cursor);
        assert!(restored.apply_fog_to_all_sprites);
    }

    #[test]
    fn old_profiles_default_to_original_fog_rendering() {
        let mut json = serde_json::to_value(GraphicConfig::default()).unwrap();
        json.as_object_mut()
            .expect("graphics config serializes as an object")
            .remove("apply_fog_to_all_sprites");

        let restored: GraphicConfig = serde_json::from_value(json).unwrap();

        assert!(!restored.apply_fog_to_all_sprites);
    }

    #[test]
    fn repr_c_layout() {
        assert_eq!(std::mem::offset_of!(GraphicConfig, display_anim), 0);
        assert_eq!(std::mem::offset_of!(GraphicConfig, display_shadow), 1);
        assert_eq!(std::mem::offset_of!(GraphicConfig, framed_view_cone), 2);
        assert_eq!(std::mem::offset_of!(GraphicConfig, display_titbits), 3);
        assert_eq!(std::mem::offset_of!(GraphicConfig, resolution_x), 4);
        assert_eq!(std::mem::offset_of!(GraphicConfig, resolution_y), 8);
    }
}

//! Backend-independent graphics types used by the renderer and window layer.
//!
//! These are the types that the rest of the codebase passes around: colors,
//! rectangles, blend modes, and keys. The renderer/window backends consume
//! them and translate as needed.

// ---------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------

/// 8-bit RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    #[inline]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Linear-space `[f32; 4]` for shader uniforms.
    #[inline]
    pub fn to_f32_srgb(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }
}

// ---------------------------------------------------------------------
// Rect
// ---------------------------------------------------------------------

/// Integer rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    #[inline]
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self {
            x,
            y,
            w: w as i32,
            h: h as i32,
        }
    }

    #[inline]
    pub fn x(&self) -> i32 {
        self.x
    }
    #[inline]
    pub fn y(&self) -> i32 {
        self.y
    }
    #[inline]
    pub fn width(&self) -> u32 {
        self.w.max(0) as u32
    }
    #[inline]
    pub fn height(&self) -> u32 {
        self.h.max(0) as u32
    }
    #[inline]
    pub fn left(&self) -> i32 {
        self.x
    }
    #[inline]
    pub fn top(&self) -> i32 {
        self.y
    }
    #[inline]
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    #[inline]
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// Hit-test a point against this rect (inclusive of the top-left
    /// edge, exclusive of the bottom-right).
    #[inline]
    pub fn contains_point<P: Into<Point>>(&self, p: P) -> bool {
        let p = p.into();
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }
}

impl From<(i32, i32)> for Point {
    fn from((x, y): (i32, i32)) -> Self {
        Point { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

// ---------------------------------------------------------------------
// Blend mode
// ---------------------------------------------------------------------

/// Blend mode mapped to `wgpu::BlendState` inside the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendMode {
    /// No blending — overwrites destination.
    None,
    /// Standard alpha blend: `src * srcA + dst * (1 - srcA)`.
    Blend,
    /// Additive: `src + dst`.
    Add,
    /// Multiplicative: `src * dst`.
    Mod,
}

impl BlendMode {
    /// Map to a wgpu blend state for the textured-quad pipeline.
    pub fn to_wgpu(self) -> Option<wgpu::BlendState> {
        match self {
            BlendMode::None => None,
            BlendMode::Blend => Some(wgpu::BlendState::ALPHA_BLENDING),
            BlendMode::Add => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            BlendMode::Mod => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Dst,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_point_is_inclusive_top_left_exclusive_bottom_right() {
        let r = Rect::new(10, 20, 5, 6);
        // Corners on the top-left edge are inside.
        assert!(r.contains_point((10, 20)));
        assert!(r.contains_point((14, 25)));
        // Bottom-right edge is exclusive.
        assert!(!r.contains_point((15, 20)));
        assert!(!r.contains_point((10, 26)));
        assert!(!r.contains_point((15, 26)));
        // Clearly outside.
        assert!(!r.contains_point((9, 20)));
        assert!(!r.contains_point((10, 19)));
    }

    #[test]
    fn rect_accessors() {
        let r = Rect::new(-3, 4, 10, 20);
        assert_eq!(r.left(), -3);
        assert_eq!(r.top(), 4);
        assert_eq!(r.right(), 7);
        assert_eq!(r.bottom(), 24);
        assert_eq!(r.width(), 10);
        assert_eq!(r.height(), 20);
        // Negative sizes clamp to zero in the unsigned accessors.
        let neg = Rect {
            x: 0,
            y: 0,
            w: -5,
            h: -1,
        };
        assert_eq!(neg.width(), 0);
        assert_eq!(neg.height(), 0);
    }

    #[test]
    fn color_to_f32_srgb_normalizes_channels() {
        assert_eq!(Color::BLACK.to_f32_srgb(), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(Color::WHITE.to_f32_srgb(), [1.0, 1.0, 1.0, 1.0]);
        let c = Color::rgba(51, 102, 153, 204).to_f32_srgb();
        assert_eq!(c, [0.2, 0.4, 0.6, 0.8]);
    }

    #[test]
    fn blend_mode_to_wgpu_mapping() {
        assert_eq!(BlendMode::None.to_wgpu(), None);
        assert_eq!(
            BlendMode::Blend.to_wgpu(),
            Some(wgpu::BlendState::ALPHA_BLENDING)
        );

        // Add: src*srcA + dst, alpha channel untouched.
        let add = BlendMode::Add.to_wgpu().expect("Add must blend");
        assert_eq!(add.color.src_factor, wgpu::BlendFactor::SrcAlpha);
        assert_eq!(add.color.dst_factor, wgpu::BlendFactor::One);
        assert_eq!(add.alpha.src_factor, wgpu::BlendFactor::Zero);
        assert_eq!(add.alpha.dst_factor, wgpu::BlendFactor::One);

        // Mod: src*dst, alpha channel untouched.
        let modulate = BlendMode::Mod.to_wgpu().expect("Mod must blend");
        assert_eq!(modulate.color.src_factor, wgpu::BlendFactor::Dst);
        assert_eq!(modulate.color.dst_factor, wgpu::BlendFactor::Zero);
        assert_eq!(modulate.alpha.src_factor, wgpu::BlendFactor::Zero);
        assert_eq!(modulate.alpha.dst_factor, wgpu::BlendFactor::One);
    }
}

// ---------------------------------------------------------------------
// Keycodes
// ---------------------------------------------------------------------

/// Game-relevant keycodes.
///
/// The set is intentionally narrow — only the keys the menu / game
/// layer actually inspects by name. Text input goes through the
/// platform IME via `GameEvent::TextInput`, so character keys aren't
/// enumerated here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum Keycode {
    Escape,
    Return,
    KpEnter,
    Tab,
    Space,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    LShift,
    RShift,
    LCtrl,
    RCtrl,
    LAlt,
    RAlt,
    Insert,
    /// A printable character key that isn't otherwise enumerated.
    /// Holds the lowercased ASCII character if available.
    Char(u8),
    /// Anything we don't have a named variant for.
    Unknown,
}

// ---------------------------------------------------------------------
// Game events (re-exported from window)
// ---------------------------------------------------------------------

/// High-level events produced by the winit-based window in `crate::window`.
#[derive(Debug, Clone)]
pub enum GameEvent {
    Quit,
    KeyDown {
        keycode: Keycode,
        physical_key: Option<winit::keyboard::KeyCode>,
    },
    KeyUp {
        keycode: Keycode,
        physical_key: Option<winit::keyboard::KeyCode>,
    },
    MouseMove {
        x: i32,
        y: i32,
        xrel: i32,
        yrel: i32,
    },
    /// (x, y, button, clicks)
    MouseDown(i32, i32, u8, u8),
    MouseUp(i32, i32, u8),
    MouseWheel(i32),
    ViewportPan {
        xrel: i32,
        yrel: i32,
    },
    MenuToggleRequested,
    PauseRequested,
    Resized(u32, u32),
    TextInput {
        text: String,
    },
    GamepadAdded {
        which: u32,
    },
    GamepadRemoved {
        which: u32,
    },
    GamepadAxis {
        which: u32,
        axis: u8,
        value: i16,
    },
    GamepadButton {
        which: u32,
        button: u8,
        pressed: bool,
    },
    /// Window gained (`true`) or lost (`false`) keyboard focus. Drives the
    /// loading-screen pause that spins on window-message processing while
    /// the game is defocused.
    WindowFocusChanged(bool),
}

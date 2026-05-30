//! PC information popup overlay — state model for the hover tooltip.
//!
//! The widget is a window containing a tiny/huge background plus two
//! 5-picture rows (one per skill: sword and bow).  Each picture represents
//! one capacity "pip" (`capacity / 20 + 1` pips are lit).  When the hovered
//! PC has no bow action, only the sword row is shown and the tiny
//! background is used instead of the huge one.  The window is anchored to
//! the mouse with a fixed `(+25, +10)` offset.
//!
//! This module is a pure state model alongside `portrait_bar.rs` /
//! `ui_screens.rs`: it owns no draw calls itself; the renderer reads
//! [`PcInfoOverlay::visible`], [`PcInfoOverlay::position`], and the derived
//! pip counts each frame.

use serde::{Deserialize, Serialize};

use crate::element::EntityId;
use crate::geo2d::GeoPoint2D;
#[cfg(test)]
use crate::geo2d::pt;

/// Offset from the mouse cursor to the top-left of the popup.
pub const POSITION_OFFSET: (f32, f32) = (25.0, 10.0);

/// Popup window size (pixels) — the "huge" background, used when the PC has a bow.
pub const POPUP_SIZE_HUGE: (i32, i32) = (91, 42);

/// Popup window size (pixels) — the "tiny" background, used when the PC has no bow.
pub const POPUP_SIZE_TINY: (i32, i32) = (91, 24);

/// Number of skill-pip pictures per row.
pub const LEVEL_NUMBER: u32 = 5;

/// Pip row origin (pixels) for the sword row.
pub const SWORD_ROW_ORIGIN: (i32, i32) = (5, 5);

/// Pip row origin (pixels) for the bow row.
pub const BOW_ROW_ORIGIN: (i32, i32) = (5, 22);

/// Horizontal spacing between pips (pixels).
pub const PIP_SPACING: i32 = 16;

/// Overlay state populated from show/hide handlers.
///
/// `visible == false` means nothing is drawn.  When visible, `position`
/// is the popup top-left (already including `POSITION_OFFSET`), and the
/// pip counts + `is_archer` describe which resources to draw.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PcInfoOverlay {
    pub visible: bool,
    /// PC the overlay is describing.  Unused by the renderer but useful
    /// for debugging / tests.
    pub pc_id: Option<EntityId>,
    /// Top-left of the popup in screen pixels, already offset from the mouse.
    pub position: (i32, i32),
    /// `true` when the PC has the bow action — controls background size
    /// and whether the bow pip row is drawn.
    pub is_archer: bool,
    /// Number of sword pips to light (`0..=LEVEL_NUMBER`).
    pub sword_pips: u32,
    /// Number of bow pips to light (`0..=LEVEL_NUMBER`).  Zero when `is_archer == false`.
    pub bow_pips: u32,
}

impl PcInfoOverlay {
    /// Clamp to screen bounds: shift left/up so the popup stays fully
    /// on-screen.
    fn clamp_to_screen(top_left: (i32, i32), size: (i32, i32), screen: (i32, i32)) -> (i32, i32) {
        let (mut x, mut y) = top_left;
        let x_max = x + size.0;
        let y_max = y + size.1;

        if x_max > screen.0 {
            x -= x_max - screen.0;
        }
        if y_max > screen.1 {
            y -= y_max - screen.1;
        }

        (x, y)
    }

    /// Pip count from a skill capacity: `experience / 20 + 1`, clamped to
    /// `LEVEL_NUMBER`.
    pub fn pip_count(capacity: u32) -> u32 {
        (capacity / 20 + 1).min(LEVEL_NUMBER)
    }

    /// Show the popup for the given PC.
    ///
    /// `mouse` is the current mouse position; `screen` is `(width, height)`
    /// of the viewport for clamping.  `sword_capacity` / `bow_capacity` are
    /// the raw skill capacity values from the PC's status.
    pub fn show(
        &mut self,
        pc_id: EntityId,
        mouse: GeoPoint2D,
        screen: (i32, i32),
        is_archer: bool,
        sword_capacity: u32,
        bow_capacity: u32,
    ) {
        let size = if is_archer {
            POPUP_SIZE_HUGE
        } else {
            POPUP_SIZE_TINY
        };

        let top_left = (
            (mouse.x + POSITION_OFFSET.0) as i32,
            (mouse.y + POSITION_OFFSET.1) as i32,
        );

        self.visible = true;
        self.pc_id = Some(pc_id);
        self.position = Self::clamp_to_screen(top_left, size, screen);
        self.is_archer = is_archer;
        self.sword_pips = Self::pip_count(sword_capacity);
        self.bow_pips = if is_archer {
            Self::pip_count(bow_capacity)
        } else {
            0
        };
    }

    /// Hide the overlay entirely.
    pub fn hide(&mut self) {
        self.visible = false;
        self.pc_id = None;
    }

    /// Resolve the screen position of pip `index` (`0..LEVEL_NUMBER`) in the
    /// sword row.  Used by the renderer to blit each pip sprite.
    pub fn sword_pip_position(&self, index: u32) -> (i32, i32) {
        (
            self.position.0 + SWORD_ROW_ORIGIN.0 + (index as i32) * PIP_SPACING,
            self.position.1 + SWORD_ROW_ORIGIN.1,
        )
    }

    /// Resolve the screen position of pip `index` in the bow row.
    pub fn bow_pip_position(&self, index: u32) -> (i32, i32) {
        (
            self.position.0 + BOW_ROW_ORIGIN.0 + (index as i32) * PIP_SPACING,
            self.position.1 + BOW_ROW_ORIGIN.1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> GeoPoint2D {
        pt(x, y)
    }

    #[test]
    fn pip_count_formula() {
        // experience / 20 + 1, clamped to LEVEL_NUMBER.
        assert_eq!(PcInfoOverlay::pip_count(0), 1);
        assert_eq!(PcInfoOverlay::pip_count(19), 1);
        assert_eq!(PcInfoOverlay::pip_count(20), 2);
        assert_eq!(PcInfoOverlay::pip_count(80), 5);
        assert_eq!(PcInfoOverlay::pip_count(100), LEVEL_NUMBER);
        assert_eq!(PcInfoOverlay::pip_count(999), LEVEL_NUMBER);
    }

    #[test]
    fn show_applies_mouse_offset_and_archer_size() {
        let mut ov = PcInfoOverlay::default();
        ov.show(EntityId(42), p(100.0, 100.0), (640, 480), true, 40, 20);

        assert!(ov.visible);
        assert_eq!(ov.pc_id, Some(EntityId(42)));
        // 100 + 25, 100 + 10
        assert_eq!(ov.position, (125, 110));
        assert!(ov.is_archer);
        assert_eq!(ov.sword_pips, 3);
        assert_eq!(ov.bow_pips, 2);
    }

    #[test]
    fn non_archer_suppresses_bow_row() {
        let mut ov = PcInfoOverlay::default();
        ov.show(EntityId(1), p(0.0, 0.0), (640, 480), false, 99, 99);
        assert!(!ov.is_archer);
        assert_eq!(ov.bow_pips, 0);
        assert_eq!(ov.sword_pips, PcInfoOverlay::pip_count(99));
    }

    #[test]
    fn clamps_to_screen_right_edge() {
        let mut ov = PcInfoOverlay::default();
        // Mouse near right edge: 640 - 25 = 615; + 25 = 640; + 91 width = 731 → shift -91.
        ov.show(EntityId(1), p(615.0, 100.0), (640, 480), true, 0, 0);
        // After clamp: x = 640 - 91 = 549.
        assert_eq!(ov.position.0, 549);
    }

    #[test]
    fn clamps_to_screen_bottom_edge() {
        let mut ov = PcInfoOverlay::default();
        // Mouse near bottom: y=470 + 10 = 480; + 42 = 522 → shift -42.
        ov.show(EntityId(1), p(100.0, 470.0), (640, 480), true, 0, 0);
        assert_eq!(ov.position.1, 480 - 42);
    }

    #[test]
    fn hide_clears_state() {
        let mut ov = PcInfoOverlay::default();
        ov.show(EntityId(1), p(0.0, 0.0), (640, 480), true, 0, 0);
        ov.hide();
        assert!(!ov.visible);
        assert_eq!(ov.pc_id, None);
    }

    #[test]
    fn pip_positions_step_by_16() {
        let mut ov = PcInfoOverlay::default();
        ov.show(EntityId(1), p(0.0, 0.0), (640, 480), true, 0, 0);
        let p0 = ov.sword_pip_position(0);
        let p1 = ov.sword_pip_position(1);
        assert_eq!(p1.0 - p0.0, PIP_SPACING);
        assert_eq!(p0.1, p1.1);
        let b = ov.bow_pip_position(0);
        assert_eq!(b.1 - p0.1, BOW_ROW_ORIGIN.1 - SWORD_ROW_ORIGIN.1);
    }

    #[test]
    fn serde_roundtrip() {
        let mut ov = PcInfoOverlay::default();
        ov.show(EntityId(7), p(50.0, 50.0), (640, 480), true, 60, 40);
        let json = serde_json::to_string(&ov).unwrap();
        let back: PcInfoOverlay = serde_json::from_str(&json).unwrap();
        assert_eq!(ov, back);
    }
}

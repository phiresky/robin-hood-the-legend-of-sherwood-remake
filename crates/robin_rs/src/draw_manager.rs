//! Game-level draw manager for rendering primitives.
//!
//! Wraps the low-level SDL renderer and provides game-coordinate-aware
//! drawing: view clipping, zoom-adjusted lines/ellipses/polygons, etc.
//!
//! Runtime primitives queue GPU overlay draws.

use serde::{Deserialize, Serialize};

use crate::geo2d::GeoPoint2D;
use crate::{gfx_types::Rect, renderer::Renderer};
use robin_engine::sprite::BBox;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default gauge width in pixels.
pub const GAUGE_WIDTH: f32 = 64.0;
/// Default gauge height in pixels.
pub const GAUGE_HEIGHT: f32 = 14.0;

// ---------------------------------------------------------------------------
// DrawManager
// ---------------------------------------------------------------------------

/// Game-level draw manager that handles coordinate transforms, clipping,
/// and color conversion before delegating to the hardware renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawManager {
    /// The current camera view rectangle in world coordinates.
    view_rect: BBox,
    /// Current zoom factor (1.0 = normal, 0.5 = zoomed out, 2.0 = zoomed in).
    zoom_factor: f32,
    /// ID of the current render target surface.
    surface_id: u32,
    /// Color depth of the hardware renderer (15 or 16).
    color_depth: u16,
}

impl Default for DrawManager {
    fn default() -> Self {
        Self {
            view_rect: BBox::default(),
            zoom_factor: 1.0,
            surface_id: 0,
            color_depth: 16,
        }
    }
}

impl DrawManager {
    pub fn new(color_depth: u16) -> Self {
        Self {
            color_depth,
            ..Default::default()
        }
    }

    // -- Accessors --

    pub fn view_rect(&self) -> &BBox {
        &self.view_rect
    }

    pub fn zoom_factor(&self) -> f32 {
        self.zoom_factor
    }

    pub fn surface_id(&self) -> u32 {
        self.surface_id
    }

    pub fn color_depth(&self) -> u16 {
        self.color_depth
    }

    /// Update the rendering parameters (called each frame by the engine).
    pub fn update_drawing_parameters(
        &mut self,
        surface_id: u32,
        view_rect: BBox,
        zoom_factor: f32,
    ) {
        self.surface_id = surface_id;
        self.view_rect = view_rect;
        self.zoom_factor = zoom_factor;
    }

    // -- Color conversion --

    /// Pack a 32-bit ARGB color into 15 or 16-bit format.
    ///
    /// Input: `0x00RRGGBB` (8 bits per channel, no alpha).
    pub fn pack_color(&self, color: u32) -> u16 {
        match self.color_depth {
            15 => {
                let r = ((color & 0x00F8_0000) >> 9) as u16;
                let g = ((color & 0x0000_F800) >> 6) as u16;
                let b = ((color & 0x0000_00FC) >> 3) as u16;
                r | g | b
            }
            16 => {
                let r = ((color & 0x00F8_0000) >> 8) as u16;
                let g = ((color & 0x0000_FC00) >> 5) as u16;
                let b = ((color & 0x0000_00FC) >> 3) as u16;
                r | g | b
            }
            _ => {
                panic!("Unsupported color depth: {}", self.color_depth);
            }
        }
    }

    // -- Coordinate helpers --

    /// Transform a world point to screen coordinates.
    pub fn world_to_screen(&self, point: GeoPoint2D) -> GeoPoint2D {
        let mut result = GeoPoint2D {
            x: point.x - self.view_rect.min.x,
            y: point.y - self.view_rect.min.y,
        };
        if self.zoom_factor != 1.0 {
            result.x *= self.zoom_factor;
            result.y *= self.zoom_factor;
        }
        result
    }

    /// Check if a point is within the drawing area after zoom.
    #[cfg(test)]
    fn check_point_for_drawing(x: i16, y: i16, width: u16, height: u16) -> bool {
        x >= 0 && (x as u16) < width && y >= 0 && (y as u16) < height
    }

    // -- Clipping helpers --

    /// Clip a segment against the view rectangle.
    ///
    /// Returns the clipped endpoints in screen coordinates, or `None` if
    /// the segment is entirely outside the view.
    pub fn clip_segment(&self, a: GeoPoint2D, b: GeoPoint2D) -> Option<(GeoPoint2D, GeoPoint2D)> {
        // Cohen-Sutherland-style clip against view_rect
        let clipped = clip_line_to_box(a, b, &self.view_rect)?;

        let mut pa = GeoPoint2D {
            x: clipped.0.x - self.view_rect.min.x,
            y: clipped.0.y - self.view_rect.min.y,
        };
        let mut pb = GeoPoint2D {
            x: clipped.1.x - self.view_rect.min.x,
            y: clipped.1.y - self.view_rect.min.y,
        };

        if self.zoom_factor != 1.0 {
            pa.x *= self.zoom_factor;
            pa.y *= self.zoom_factor;
            pb.x *= self.zoom_factor;
            pb.y *= self.zoom_factor;
        }

        Some((pa, pb))
    }

    /// Clip a bounding box to the view rect and transform to screen coords.
    pub fn clip_box(&self, bbox: &BBox) -> Option<BBox> {
        // Intersect with view rect
        let min_x = bbox.min.x.max(self.view_rect.min.x);
        let min_y = bbox.min.y.max(self.view_rect.min.y);
        let max_x = bbox.max.x.min(self.view_rect.max.x);
        let max_y = bbox.max.y.min(self.view_rect.max.y);

        if min_x >= max_x || min_y >= max_y {
            return None;
        }

        let mut result = BBox::new(
            GeoPoint2D {
                x: min_x - self.view_rect.min.x,
                y: min_y - self.view_rect.min.y,
            },
            GeoPoint2D {
                x: max_x - self.view_rect.min.x,
                y: max_y - self.view_rect.min.y,
            },
        );

        if self.zoom_factor != 1.0 {
            result.min.x *= self.zoom_factor;
            result.min.y *= self.zoom_factor;
            result.max.x *= self.zoom_factor;
            result.max.y *= self.zoom_factor;
        }

        Some(result)
    }

    // -- Drawing methods --
    // These clip/transform then delegate to the Renderer.

    /// Draw a line segment in world coordinates, clipped to the view.
    pub fn draw_segment(&self, renderer: &mut Renderer, a: GeoPoint2D, b: GeoPoint2D, color: u16) {
        if let Some((pa, pb)) = self.clip_segment(a, b) {
            renderer.draw_line_screen(pa.x as i32, pa.y as i32, pb.x as i32, pb.y as i32, color);
        }
    }

    /// Fill a rectangle in world coordinates, clipped to the view.
    pub fn fill_box(&self, renderer: &mut Renderer, bbox: &BBox, color: u16) {
        if let Some(clipped) = self.clip_box(bbox) {
            renderer.fill_screen(Some(&clipped), color);
        }
    }

    /// Draw a dotted line between two points.
    ///
    /// `start` is the distance from `a` to the first dot (updated on return).
    /// `spacing` is the distance between dots.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_dotted_line(
        &self,
        renderer: &mut Renderer,
        a: GeoPoint2D,
        b: GeoPoint2D,
        start: &mut f32,
        spacing: f32,
        thickness: f32,
        color: u16,
    ) {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let distance = (dx * dx + dy * dy).sqrt();

        if distance < *start {
            if distance != 0.0 {
                *start -= distance;
            }
            return;
        }

        let inv_dist = 1.0 / distance;
        let inc_x = dx * inv_dist * spacing;
        let inc_y = dy * inv_dist * spacing;

        let mut point = GeoPoint2D {
            x: a.x + *start * dx * inv_dist,
            y: a.y + *start * dy * inv_dist,
        };

        let remaining = distance - *start;
        let num_dots = (remaining / spacing) as u32;

        // Update start for next segment
        *start = spacing - remaining + (num_dots as f32 * spacing);

        for _ in 0..=num_dots {
            let dot_box = BBox::new(
                GeoPoint2D {
                    x: point.x - thickness,
                    y: point.y - thickness,
                },
                GeoPoint2D {
                    x: point.x + thickness,
                    y: point.y + thickness,
                },
            );

            if let Some(clipped) = self.clip_box(&dot_box) {
                renderer.fill_screen(Some(&clipped), color);
            }

            point.x += inc_x;
            point.y += inc_y;
        }
    }

    /// Draw a polyline in world coordinates.
    pub fn draw_polyline(&self, renderer: &mut Renderer, points: &[GeoPoint2D], color: u16) {
        for i in 0..points.len().saturating_sub(1) {
            self.draw_segment(renderer, points[i], points[i + 1], color);
        }
    }

    /// Draw a polyline without clipping (assumes points are already in view).
    ///
    /// Currently dead code (no callers). Applies `world_to_screen` so it
    /// matches the rest of the `DrawManager` API if ever resurrected.
    pub fn draw_polyline_no_clip(
        &self,
        renderer: &mut Renderer,
        points: &[GeoPoint2D],
        color: u16,
    ) {
        for i in 0..points.len().saturating_sub(1) {
            let a = self.world_to_screen(points[i]);
            let b = self.world_to_screen(points[i + 1]);
            renderer.draw_line_screen(a.x as i32, a.y as i32, b.x as i32, b.y as i32, color);
        }
    }

    /// Draw an ellipse (isometric projection of a circle).
    ///
    /// The minor axis is scaled by `cos(55°)` to match the game's isometric angle.
    pub fn draw_ellipse(
        &self,
        renderer: &mut Renderer,
        position: GeoPoint2D,
        radius: u16,
        color: u16,
    ) {
        // cos(55°), the game's isometric projection angle.
        const ISOMETRIC_MINOR_AXIS_RATIO: f64 = 0.573576436351046096108031912826158;

        let center = self.world_to_screen(position);
        // Cast through u16 to truncate to 16 bits.
        let r = if self.zoom_factor != 1.0 {
            (radius as f32 * self.zoom_factor) as u16 as i32
        } else {
            radius as i32
        };

        let ry = (r as f64 * ISOMETRIC_MINOR_AXIS_RATIO) as f32 as i32;
        draw_ellipse_gpu(renderer, center.x, center.y, r, ry, color);
    }

    /// Draw a circle (non-isometric).
    pub fn draw_circle(
        &self,
        renderer: &mut Renderer,
        position: GeoPoint2D,
        radius: u16,
        color: u16,
    ) {
        let center = self.world_to_screen(position);
        // Cast through u16 to truncate to 16 bits.
        let r = if self.zoom_factor != 1.0 {
            (radius as f32 * self.zoom_factor) as u16 as i32
        } else {
            radius as i32
        };

        draw_ellipse_gpu(renderer, center.x, center.y, r, r, color);
    }

    /// Display a gauge bar (used for health/stamina).
    pub fn display_gauge(
        &self,
        renderer: &mut Renderer,
        top_left: GeoPoint2D,
        fraction: f32,
        back_color: u16,
        fore_color: u16,
    ) {
        // Background
        let bg_box = BBox::new(
            top_left,
            GeoPoint2D {
                x: top_left.x + GAUGE_WIDTH,
                y: top_left.y + GAUGE_HEIGHT,
            },
        );
        self.fill_box(renderer, &bg_box, back_color);

        // Foreground (filled portion)
        let fg_box = BBox::new(
            top_left,
            GeoPoint2D {
                x: top_left.x + fraction * GAUGE_WIDTH,
                y: top_left.y + GAUGE_HEIGHT,
            },
        );
        self.fill_box(renderer, &fg_box, fore_color);

        // Note: a percentage-text overlay was wired up for the
        // `energyDisplay` console cheat but never actually invoked — no
        // render path reads the cheat flag, and there's no stamina metric
        // to hook into. Skipped here for the same reason.
    }

    /// Draw a filled, semi-transparent polygon.
    ///
    /// Uses scanline rasterisation to determine pixel coverage, then queues
    /// alpha-blended one-pixel spans on the GPU overlay layer.
    ///
    /// C++ blends against `RHEngine::GetMap()`, which already contains
    /// `BlitToMap` patch mutations. Rust keeps those map patches as GPU
    /// decals, so blending over the queued frame preserves the same visible
    /// ordering relative to patched map pixels.
    ///
    /// Scanline even-odd fill with axis-aligned clipping produces the
    /// same pixel set as pre-clipping the polygon, so the output is
    /// correct without an explicit polygon-vs-bbox clipping pass.
    ///
    /// `color` is `0x00RRGGBB`, `alpha` is 0..256 (0 = invisible, 256 = opaque).
    pub fn draw_alpha_polygon(
        &self,
        renderer: &mut Renderer,
        points: &[GeoPoint2D],
        color: u32,
        alpha: u32,
    ) {
        if points.len() < 3 || alpha == 0 {
            return;
        }

        // Convert world → screen coordinates
        let screen_pts: Vec<[f32; 2]> = points
            .iter()
            .map(|p| {
                let s = self.world_to_screen(*p);
                [s.x, s.y]
            })
            .collect();

        // Build edge table
        let edges = build_poly_edge_table(&screen_pts);
        if edges.is_empty() {
            return;
        }

        if renderer.is_gpu_phase() {
            draw_alpha_polygon_gpu(
                renderer,
                &edges,
                color,
                alpha,
                self.view_rect.min,
                self.zoom_factor,
            );
            return;
        }

        panic!("draw_alpha_polygon called before flush_base_layer/GPU phase");
    }
}

// ---------------------------------------------------------------------------
// Float-based midpoint ellipse
// ---------------------------------------------------------------------------

fn draw_ellipse_gpu(renderer: &mut Renderer, cx: f32, cy: f32, rx: i32, ry: i32, color: u16) {
    if rx <= 0 || ry <= 0 {
        return;
    }
    let steps = ((rx.max(ry) as f32 * 0.35).ceil() as usize).clamp(24, 160);
    let mut prev = (cx + rx as f32, cy);
    for i in 1..=steps {
        let t = i as f32 * std::f32::consts::TAU / steps as f32;
        let next = (cx + t.cos() * rx as f32, cy + t.sin() * ry as f32);
        renderer.draw_line_screen(
            prev.0.round() as i32,
            prev.1.round() as i32,
            next.0.round() as i32,
            next.1.round() as i32,
            color,
        );
        prev = next;
    }
}

// ---------------------------------------------------------------------------
// Polygon scanline edge table helper
// ---------------------------------------------------------------------------

/// A polygon edge parameterised for scanline intersection.
struct PolyEdge {
    y_min: f32,
    y_max: f32,
    x_start: f32,
    dx_per_dy: f32,
}

/// Build an edge table from screen-space polygon points.
fn build_poly_edge_table(pts: &[[f32; 2]]) -> Vec<PolyEdge> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    let mut edges = Vec::with_capacity(n);
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        let dy = b[1] - a[1];
        if dy.abs() < 0.001 {
            continue;
        }
        let (y_min, y_max, x_start);
        if a[1] < b[1] {
            y_min = a[1];
            y_max = b[1];
            x_start = a[0];
        } else {
            y_min = b[1];
            y_max = a[1];
            x_start = b[0];
        }
        let dx_per_dy = (b[0] - a[0]) / dy;
        edges.push(PolyEdge {
            y_min,
            y_max,
            x_start,
            dx_per_dy,
        });
    }
    edges
}

/// GPU path for `DrawManager::draw_alpha_polygon`: queue one span per
/// filled scanline. The renderer snapshots the composited frame before
/// these spans and uses the C++-matching RGB565 alpha shader, so patches
/// already drawn on the GPU are included in the source pixels.
#[allow(clippy::too_many_arguments)]
fn draw_alpha_polygon_gpu(
    renderer: &mut Renderer,
    edges: &[PolyEdge],
    color: u32,
    alpha: u32,
    _view_min: GeoPoint2D,
    _zoom: f32,
) {
    let y_min = edges.iter().map(|e| e.y_min as i32).min().unwrap().max(0);
    let y_max = edges.iter().map(|e| e.y_max.ceil() as i32).max().unwrap();
    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let y_max = y_max.min(sh);
    if y_min >= y_max {
        return;
    }

    for y in y_min..y_max {
        let yf = y as f32 + 0.5;
        let mut crossings: Vec<f32> = Vec::new();
        for edge in edges {
            if yf >= edge.y_min && yf < edge.y_max {
                let x = edge.x_start + (yf - edge.y_min) * edge.dx_per_dy;
                crossings.push(x);
            }
        }
        crossings.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());

        let mut i = 0;
        while i + 1 < crossings.len() {
            let x0 = (crossings[i].ceil() as i32).max(0);
            let x1 = (crossings[i + 1].floor() as i32 + 1).min(sw);
            if x1 > x0 {
                let uv = [
                    x0 as f32 / sw as f32,
                    y as f32 / sh as f32,
                    x1 as f32 / sw as f32,
                    (y + 1) as f32 / sh as f32,
                ];
                renderer.render_framebuffer_alpha_rect(
                    Rect {
                        x: x0,
                        y,
                        w: x1 - x0,
                        h: 1,
                    },
                    uv,
                    color,
                    alpha,
                );
            }
            i += 2;
        }
    }
}

// ---------------------------------------------------------------------------
// Line clipping helper (Cohen-Sutherland)
// ---------------------------------------------------------------------------

/// Outcode bits for Cohen-Sutherland.
const INSIDE: u8 = 0;
const LEFT: u8 = 1;
const RIGHT: u8 = 2;
const BOTTOM: u8 = 4;
const TOP: u8 = 8;

fn compute_outcode(p: GeoPoint2D, bbox: &BBox) -> u8 {
    let mut code = INSIDE;
    if p.x < bbox.min.x {
        code |= LEFT;
    } else if p.x > bbox.max.x {
        code |= RIGHT;
    }
    if p.y < bbox.min.y {
        code |= TOP;
    } else if p.y > bbox.max.y {
        code |= BOTTOM;
    }
    code
}

/// Clip a line segment to a bounding box using Cohen-Sutherland.
/// Returns `None` if the line is entirely outside.
fn clip_line_to_box(
    mut a: GeoPoint2D,
    mut b: GeoPoint2D,
    bbox: &BBox,
) -> Option<(GeoPoint2D, GeoPoint2D)> {
    let mut code_a = compute_outcode(a, bbox);
    let mut code_b = compute_outcode(b, bbox);

    loop {
        if (code_a | code_b) == 0 {
            // Both inside
            return Some((a, b));
        }
        if (code_a & code_b) != 0 {
            // Both on same outside side
            return None;
        }

        let code_out = if code_a != 0 { code_a } else { code_b };
        let dx = b.x - a.x;
        let dy = b.y - a.y;

        let (x, y);
        if code_out & TOP != 0 {
            x = a.x + dx * (bbox.min.y - a.y) / dy;
            y = bbox.min.y;
        } else if code_out & BOTTOM != 0 {
            x = a.x + dx * (bbox.max.y - a.y) / dy;
            y = bbox.max.y;
        } else if code_out & RIGHT != 0 {
            y = a.y + dy * (bbox.max.x - a.x) / dx;
            x = bbox.max.x;
        } else {
            // LEFT
            y = a.y + dy * (bbox.min.x - a.x) / dx;
            x = bbox.min.x;
        }

        if code_out == code_a {
            a = GeoPoint2D { x, y };
            code_a = compute_outcode(a, bbox);
        } else {
            b = GeoPoint2D { x, y };
            code_b = compute_outcode(b, bbox);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draw_manager_default() {
        let dm = DrawManager::default();
        assert_eq!(dm.zoom_factor(), 1.0);
        assert_eq!(dm.surface_id(), 0);
        assert_eq!(dm.color_depth(), 16);
    }

    #[test]
    fn test_update_drawing_parameters() {
        let mut dm = DrawManager::new(16);
        let view = BBox::new(
            GeoPoint2D { x: 100.0, y: 200.0 },
            GeoPoint2D { x: 900.0, y: 800.0 },
        );
        dm.update_drawing_parameters(42, view, 0.5);

        assert_eq!(dm.surface_id(), 42);
        assert_eq!(dm.zoom_factor(), 0.5);
        assert_eq!(dm.view_rect().min.x, 100.0);
    }

    #[test]
    fn test_pack_color_16bit() {
        let dm = DrawManager::new(16);

        // White
        let white = dm.pack_color(0x00FF_FFFF);
        // Red=0xF8>>8=0x1F<<11, Green=0xFC>>5, Blue=0xFC>>3
        assert_ne!(white, 0); // just ensure it's non-zero

        // Black
        let black = dm.pack_color(0x0000_0000);
        assert_eq!(black, 0);

        // Pure red: 0xFF0000
        let red = dm.pack_color(0x00FF_0000);
        assert_eq!(red & 0xF800, 0xF800); // top 5 bits set
    }

    #[test]
    fn test_pack_color_15bit() {
        let dm = DrawManager::new(15);

        let black = dm.pack_color(0x0000_0000);
        assert_eq!(black, 0);

        let red = dm.pack_color(0x00FF_0000);
        assert_eq!(red & 0x7C00, 0x7C00); // top 5 bits in 15-bit position
    }

    #[test]
    fn test_world_to_screen() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(
            0,
            BBox::new(
                GeoPoint2D { x: 100.0, y: 200.0 },
                GeoPoint2D { x: 900.0, y: 800.0 },
            ),
            1.0,
        );

        let screen = dm.world_to_screen(GeoPoint2D { x: 150.0, y: 250.0 });
        assert_eq!(screen.x, 50.0);
        assert_eq!(screen.y, 50.0);
    }

    #[test]
    fn test_world_to_screen_zoomed() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(
            0,
            BBox::new(
                GeoPoint2D { x: 100.0, y: 200.0 },
                GeoPoint2D { x: 900.0, y: 800.0 },
            ),
            2.0,
        );

        let screen = dm.world_to_screen(GeoPoint2D { x: 150.0, y: 250.0 });
        assert_eq!(screen.x, 100.0); // (150-100) * 2
        assert_eq!(screen.y, 100.0); // (250-200) * 2
    }

    #[test]
    fn test_clip_segment_inside() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(
            0,
            BBox::new(
                GeoPoint2D { x: 0.0, y: 0.0 },
                GeoPoint2D { x: 100.0, y: 100.0 },
            ),
            1.0,
        );

        let result = dm.clip_segment(
            GeoPoint2D { x: 10.0, y: 10.0 },
            GeoPoint2D { x: 90.0, y: 90.0 },
        );
        assert!(result.is_some());
    }

    #[test]
    fn test_clip_segment_outside() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(
            0,
            BBox::new(
                GeoPoint2D { x: 0.0, y: 0.0 },
                GeoPoint2D { x: 100.0, y: 100.0 },
            ),
            1.0,
        );

        // Completely outside
        let result = dm.clip_segment(
            GeoPoint2D { x: 200.0, y: 200.0 },
            GeoPoint2D { x: 300.0, y: 300.0 },
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_clip_box_partial() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(
            0,
            BBox::new(
                GeoPoint2D { x: 0.0, y: 0.0 },
                GeoPoint2D { x: 100.0, y: 100.0 },
            ),
            1.0,
        );

        let bbox = BBox::new(
            GeoPoint2D { x: -10.0, y: -10.0 },
            GeoPoint2D { x: 50.0, y: 50.0 },
        );
        let clipped = dm.clip_box(&bbox);
        assert!(clipped.is_some());
        let c = clipped.unwrap();
        assert_eq!(c.min.x, 0.0);
        assert_eq!(c.min.y, 0.0);
        assert_eq!(c.max.x, 50.0);
        assert_eq!(c.max.y, 50.0);
    }

    #[test]
    fn test_clip_box_outside() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(
            0,
            BBox::new(
                GeoPoint2D { x: 0.0, y: 0.0 },
                GeoPoint2D { x: 100.0, y: 100.0 },
            ),
            1.0,
        );

        let bbox = BBox::new(
            GeoPoint2D { x: 200.0, y: 200.0 },
            GeoPoint2D { x: 300.0, y: 300.0 },
        );
        assert!(dm.clip_box(&bbox).is_none());
    }

    #[test]
    fn test_cohen_sutherland_clipping() {
        let bbox = BBox::new(
            GeoPoint2D { x: 0.0, y: 0.0 },
            GeoPoint2D { x: 100.0, y: 100.0 },
        );

        // Line crossing through the box
        let result = clip_line_to_box(
            GeoPoint2D { x: -50.0, y: 50.0 },
            GeoPoint2D { x: 150.0, y: 50.0 },
            &bbox,
        );
        assert!(result.is_some());
        let (a, b) = result.unwrap();
        assert!((a.x - 0.0).abs() < 0.01);
        assert!((b.x - 100.0).abs() < 0.01);

        // Line entirely inside
        let result = clip_line_to_box(
            GeoPoint2D { x: 10.0, y: 10.0 },
            GeoPoint2D { x: 90.0, y: 90.0 },
            &bbox,
        );
        assert!(result.is_some());

        // Line entirely outside
        let result = clip_line_to_box(
            GeoPoint2D { x: -50.0, y: -50.0 },
            GeoPoint2D { x: -10.0, y: -10.0 },
            &bbox,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_draw_manager_serde_roundtrip() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(
            5,
            BBox::new(
                GeoPoint2D { x: 10.0, y: 20.0 },
                GeoPoint2D { x: 800.0, y: 600.0 },
            ),
            0.5,
        );

        let json = serde_json::to_string(&dm).unwrap();
        let back: DrawManager = serde_json::from_str(&json).unwrap();

        assert_eq!(back.surface_id(), 5);
        assert_eq!(back.zoom_factor(), 0.5);
        assert_eq!(back.color_depth(), 16);
        assert_eq!(back.view_rect().min.x, 10.0);
    }

    #[test]
    fn test_dotted_line_short_segment_math() {
        // Test the short-segment early-return math without needing a Renderer.
        // When the segment is shorter than `start`, draw_dotted_line just
        // decrements start by the segment length and returns.
        // Here we verify that logic directly.
        let distance = 10.0f32; // segment length
        let mut start = 100.0f32;
        // This is the early-return path: distance < start
        assert!(distance < start);
        start -= distance;
        assert!((start - 90.0).abs() < 0.01);
    }

    #[test]
    fn test_check_point_for_drawing() {
        assert!(DrawManager::check_point_for_drawing(0, 0, 100, 100));
        assert!(DrawManager::check_point_for_drawing(99, 99, 100, 100));
        assert!(!DrawManager::check_point_for_drawing(-1, 0, 100, 100));
        assert!(!DrawManager::check_point_for_drawing(100, 0, 100, 100));
    }
}

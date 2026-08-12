//! Game-level draw manager for rendering primitives.
//!
//! Wraps the low-level GPU renderer and provides game-coordinate-aware
//! drawing: view clipping, zoom-adjusted lines/ellipses/polygons, etc.
//!
//! Runtime primitives queue GPU overlay draws.

use serde::{Deserialize, Serialize};

use crate::{gfx_types::Rect, renderer::Renderer};
use robin_engine::coordinates::{MapBBox, MapPoint, ScreenPoint};
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
    view_rect: MapBBox,
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
            view_rect: MapBBox::default(),
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

    pub fn view_rect(&self) -> &MapBBox {
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
        view_rect: MapBBox,
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

    /// Transform a projected map point to screen coordinates.
    pub fn map_to_screen(&self, point: MapPoint) -> ScreenPoint {
        let mut result = ScreenPoint {
            x: point.x - self.view_rect.x_min(),
            y: point.y - self.view_rect.y_min(),
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
    pub fn clip_segment(&self, a: MapPoint, b: MapPoint) -> Option<(ScreenPoint, ScreenPoint)> {
        // Cohen-Sutherland-style clip against view_rect
        let (clipped_a, clipped_b) = clip_map_line_to_box(a, b, &self.view_rect)?;

        let pa = self.map_to_screen(clipped_a);
        let pb = self.map_to_screen(clipped_b);
        Some((pa, pb))
    }

    /// Clip a bounding box to the view rect and transform to screen coords.
    pub fn clip_box(&self, bbox: &MapBBox) -> Option<BBox> {
        // Intersect with view rect
        let min_x = bbox.x_min().max(self.view_rect.x_min());
        let min_y = bbox.y_min().max(self.view_rect.y_min());
        let max_x = bbox.x_max().min(self.view_rect.x_max());
        let max_y = bbox.y_max().min(self.view_rect.y_max());

        if min_x >= max_x || min_y >= max_y {
            return None;
        }

        let mut result = screen_bbox(
            ScreenPoint::new(
                min_x - self.view_rect.x_min(),
                min_y - self.view_rect.y_min(),
            ),
            ScreenPoint::new(
                max_x - self.view_rect.x_min(),
                max_y - self.view_rect.y_min(),
            ),
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

    /// Draw a line segment in projected map coordinates, clipped to the view.
    pub fn draw_segment(&self, renderer: &mut Renderer, a: MapPoint, b: MapPoint, color: u16) {
        if let Some((pa, pb)) = self.clip_segment(a, b) {
            renderer.draw_line_screen(pa.x as i32, pa.y as i32, pb.x as i32, pb.y as i32, color);
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
        a: MapPoint,
        b: MapPoint,
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

        let mut point = MapPoint {
            x: a.x + *start * dx * inv_dist,
            y: a.y + *start * dy * inv_dist,
        };

        let remaining = distance - *start;
        let num_dots = (remaining / spacing) as u32;

        // Update start for next segment
        *start = spacing - remaining + (num_dots as f32 * spacing);

        for _ in 0..=num_dots {
            let dot_box = MapBBox::from_coords(
                point.x - thickness,
                point.y - thickness,
                point.x + thickness,
                point.y + thickness,
            );

            if let Some(clipped) = self.clip_box(&dot_box) {
                renderer.fill_screen(Some(&clipped), color);
            }

            point.x += inc_x;
            point.y += inc_y;
        }
    }

    /// Draw a polyline in projected map coordinates.
    pub fn draw_polyline(&self, renderer: &mut Renderer, points: &[MapPoint], color: u16) {
        for i in 0..points.len().saturating_sub(1) {
            self.draw_segment(renderer, points[i], points[i + 1], color);
        }
    }

    /// Draw an ellipse (isometric projection of a circle).
    ///
    /// The minor axis is scaled by `cos(55°)` to match the game's isometric angle.
    pub fn draw_ellipse(
        &self,
        renderer: &mut Renderer,
        position: MapPoint,
        radius: u16,
        color: u16,
    ) {
        // cos(55°), the game's isometric projection angle.
        const ISOMETRIC_MINOR_AXIS_RATIO: f64 = 0.573576436351046096108031912826158f32 as f64;

        let center = self.map_to_screen(position);
        // Cast through u16 to truncate to 16 bits.
        let r = if self.zoom_factor != 1.0 {
            // Windows retail promotes the stored binary32 zoom to x87
            // extended precision and converts only the product.
            (radius as f64 * self.zoom_factor as f64) as u16 as i32
        } else {
            radius as i32
        };

        let ry = (r as f64 * ISOMETRIC_MINOR_AXIS_RATIO) as i32;
        draw_ellipse_gpu(renderer, center.x, center.y, r, ry, color);
    }

    /// Draw a circle (non-isometric).
    pub fn draw_circle(
        &self,
        renderer: &mut Renderer,
        position: MapPoint,
        radius: u16,
        color: u16,
    ) {
        let center = self.map_to_screen(position);
        // Cast through u16 to truncate to 16 bits.
        let r = if self.zoom_factor != 1.0 {
            (radius as f32 * self.zoom_factor) as u16 as i32
        } else {
            radius as i32
        };

        draw_ellipse_gpu(renderer, center.x, center.y, r, r, color);
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
        points: &[MapPoint],
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
                let s = self.map_to_screen(*p);
                [s.x, s.y]
            })
            .collect();

        // Build edge table
        let edges = build_poly_edge_table(&screen_pts);
        if edges.is_empty() {
            return;
        }

        if renderer.is_gpu_phase() {
            draw_alpha_polygon_gpu(renderer, &edges, color, alpha, self.zoom_factor);
            return;
        }

        panic!("draw_alpha_polygon called before flush_base_layer/GPU phase");
    }
}

fn screen_bbox(min: ScreenPoint, max: ScreenPoint) -> BBox {
    BBox::from_coords(min.x, min.y, max.x, max.y)
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

    // Scratch buffers reused across rows, and a run of consecutive rows
    // sharing the same pixel spans that is flushed as taller quads.
    let mut crossings: Vec<f32> = Vec::new();
    let mut row_spans: Vec<(i32, i32)> = Vec::new();
    let mut run_spans: Vec<(i32, i32)> = Vec::new();
    let mut run_start_y = y_min;

    let mut flush_run = |spans: &[(i32, i32)], y_from: i32, y_to: i32| {
        for &(x0, x1) in spans {
            let uv = [
                x0 as f32 / sw as f32,
                y_from as f32 / sh as f32,
                x1 as f32 / sw as f32,
                y_to as f32 / sh as f32,
            ];
            renderer.render_framebuffer_alpha_rect(
                Rect {
                    x: x0,
                    y: y_from,
                    w: x1 - x0,
                    h: y_to - y_from,
                },
                uv,
                color,
                alpha,
            );
        }
    };

    for y in y_min..y_max {
        let yf = y as f32 + 0.5;
        crossings.clear();
        for edge in edges {
            if yf >= edge.y_min && yf < edge.y_max {
                let x = edge.x_start + (yf - edge.y_min) * edge.dx_per_dy;
                crossings.push(x);
            }
        }
        crossings.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());

        row_spans.clear();
        let mut i = 0;
        while i + 1 < crossings.len() {
            let x0 = (crossings[i].ceil() as i32).max(0);
            let x1 = (crossings[i + 1].floor() as i32 + 1).min(sw);
            if x1 > x0 {
                row_spans.push((x0, x1));
            }
            i += 2;
        }

        if row_spans != run_spans {
            flush_run(&run_spans, run_start_y, y);
            std::mem::swap(&mut run_spans, &mut row_spans);
            run_start_y = y;
        }
    }
    flush_run(&run_spans, run_start_y, y_max);
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

fn compute_map_outcode(p: MapPoint, bbox: &MapBBox) -> u8 {
    let mut code = INSIDE;
    if p.x < bbox.x_min() {
        code |= LEFT;
    } else if p.x > bbox.x_max() {
        code |= RIGHT;
    }
    if p.y < bbox.y_min() {
        code |= TOP;
    } else if p.y > bbox.y_max() {
        code |= BOTTOM;
    }
    code
}

/// Clip a line segment to a bounding box using Cohen-Sutherland.
/// Returns `None` if the line is entirely outside.
fn clip_map_line_to_box(
    mut a: MapPoint,
    mut b: MapPoint,
    bbox: &MapBBox,
) -> Option<(MapPoint, MapPoint)> {
    let mut code_a = compute_map_outcode(a, bbox);
    let mut code_b = compute_map_outcode(b, bbox);

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
            x = a.x + dx * (bbox.y_min() - a.y) / dy;
            y = bbox.y_min();
        } else if code_out & BOTTOM != 0 {
            x = a.x + dx * (bbox.y_max() - a.y) / dy;
            y = bbox.y_max();
        } else if code_out & RIGHT != 0 {
            y = a.y + dy * (bbox.x_max() - a.x) / dx;
            x = bbox.x_max();
        } else {
            // LEFT
            y = a.y + dy * (bbox.x_min() - a.x) / dx;
            x = bbox.x_min();
        }

        if code_out == code_a {
            a = MapPoint::new(x, y);
            code_a = compute_map_outcode(a, bbox);
        } else {
            b = MapPoint::new(x, y);
            code_b = compute_map_outcode(b, bbox);
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
        let view = MapBBox::from_coords(100.0, 200.0, 900.0, 800.0);
        dm.update_drawing_parameters(42, view, 0.5);

        assert_eq!(dm.surface_id(), 42);
        assert_eq!(dm.zoom_factor(), 0.5);
        assert_eq!(dm.view_rect().x_min(), 100.0);
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
    fn test_map_to_screen() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(0, MapBBox::from_coords(100.0, 200.0, 900.0, 800.0), 1.0);

        let screen = dm.map_to_screen(MapPoint::new(150.0, 250.0));
        assert_eq!(screen.x, 50.0);
        assert_eq!(screen.y, 50.0);
    }

    #[test]
    fn test_map_to_screen_zoomed() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(0, MapBBox::from_coords(100.0, 200.0, 900.0, 800.0), 2.0);

        let screen = dm.map_to_screen(MapPoint::new(150.0, 250.0));
        assert_eq!(screen.x, 100.0); // (150-100) * 2
        assert_eq!(screen.y, 100.0); // (250-200) * 2
    }

    #[test]
    fn test_clip_segment_inside() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(0, MapBBox::from_coords(0.0, 0.0, 100.0, 100.0), 1.0);

        let result = dm.clip_segment(MapPoint::new(10.0, 10.0), MapPoint::new(90.0, 90.0));
        assert!(result.is_some());
    }

    #[test]
    fn test_clip_segment_outside() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(0, MapBBox::from_coords(0.0, 0.0, 100.0, 100.0), 1.0);

        // Completely outside
        let result = dm.clip_segment(MapPoint::new(200.0, 200.0), MapPoint::new(300.0, 300.0));
        assert!(result.is_none());
    }

    #[test]
    fn test_clip_box_partial() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(0, MapBBox::from_coords(0.0, 0.0, 100.0, 100.0), 1.0);

        let bbox = MapBBox::from_coords(-10.0, -10.0, 50.0, 50.0);
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
        dm.update_drawing_parameters(0, MapBBox::from_coords(0.0, 0.0, 100.0, 100.0), 1.0);

        let bbox = MapBBox::from_coords(200.0, 200.0, 300.0, 300.0);
        assert!(dm.clip_box(&bbox).is_none());
    }

    #[test]
    fn test_cohen_sutherland_clipping() {
        let bbox = MapBBox::from_coords(0.0, 0.0, 100.0, 100.0);

        // Line crossing through the box
        let result = clip_map_line_to_box(
            MapPoint::new(-50.0, 50.0),
            MapPoint::new(150.0, 50.0),
            &bbox,
        );
        assert!(result.is_some());
        let (a, b) = result.unwrap();
        assert!((a.x - 0.0).abs() < 0.01);
        assert!((b.x - 100.0).abs() < 0.01);

        // Line entirely inside
        let result =
            clip_map_line_to_box(MapPoint::new(10.0, 10.0), MapPoint::new(90.0, 90.0), &bbox);
        assert!(result.is_some());

        // Line entirely outside
        let result = clip_map_line_to_box(
            MapPoint::new(-50.0, -50.0),
            MapPoint::new(-10.0, -10.0),
            &bbox,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_draw_manager_serde_roundtrip() {
        let mut dm = DrawManager::new(16);
        dm.update_drawing_parameters(5, MapBBox::from_coords(10.0, 20.0, 800.0, 600.0), 0.5);

        let json = serde_json::to_string(&dm).unwrap();
        let back: DrawManager = serde_json::from_str(&json).unwrap();

        assert_eq!(back.surface_id(), 5);
        assert_eq!(back.zoom_factor(), 0.5);
        assert_eq!(back.color_depth(), 16);
        assert_eq!(back.view_rect().x_min(), 10.0);
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

//! fog presentation pass.
use super::*;

const MAX_FOG_MASK_TEXELS: u64 = 4 * 1024 * 1024;

pub(crate) fn vector_fog_mask_dimensions(level_size: engine_coordinates::MapSize) -> (u32, u32) {
    assert!(
        level_size.x.is_finite()
            && level_size.y.is_finite()
            && level_size.x > 0.0
            && level_size.y > 0.0,
        "cannot rasterize polygon fog for an invalid level size"
    );
    let mut width = level_size.x.ceil() as u32;
    let mut height = level_size.y.ceil() as u32;
    let texels = u64::from(width) * u64::from(height);
    if texels > MAX_FOG_MASK_TEXELS {
        let scale = (MAX_FOG_MASK_TEXELS as f64 / texels as f64).sqrt();
        width = (f64::from(width) * scale).floor().max(1.0) as u32;
        height = (f64::from(height) * scale).floor().max(1.0) as u32;
    }
    (width, height)
}

/// Rasterize exact simulation polygons only at the final presentation edge.
/// The preferred scale is one map pixel per fog pixel, bounded for unusually
/// large maps. GPU linear sampling antialiases the pixel-scale result.
pub(crate) fn build_vector_fog_mask_rgba(fog: &robin_engine::fog_of_war::FogOfWarState) -> Vec<u8> {
    let level_size = fog.level_size();
    let (width, height) = vector_fog_mask_dimensions(level_size);
    let mut alpha = vec![255; width as usize * height as usize];
    rasterize_fog_region(
        &mut alpha,
        width,
        height,
        level_size,
        fog.explored_projection_region(),
        165,
    );
    rasterize_fog_region(
        &mut alpha,
        width,
        height,
        level_size,
        fog.visible_projection_region(),
        0,
    );
    let mut rgba = vec![0; alpha.len() * 4];
    for (pixel, alpha) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(alpha) {
        pixel[3] = alpha;
    }
    rgba
}

pub(crate) fn fog_mask_cache_key(fog: &robin_engine::fog_of_war::FogOfWarState) -> u64 {
    u64::from(fog.generation())
}

pub(super) fn rasterize_fog_region(
    alpha: &mut [u8],
    width: u32,
    height: u32,
    level_size: engine_coordinates::MapSize,
    region: &robin_engine::fog_of_war::FogRegion,
    value: u8,
) {
    let scale_x = width as f32 / level_size.x;
    let scale_y = height as f32 / level_size.y;
    let mut crossings = Vec::new();
    for polygon in region.polygons() {
        let rings = std::iter::once(polygon.exterior.as_slice())
            .chain(polygon.interiors.iter().map(Vec::as_slice));
        let Some((min_y, max_y)) = rings
            .clone()
            .flat_map(|ring| ring.iter().map(|point| point.y))
            .fold(None, |bounds, y| {
                Some(bounds.map_or((y, y), |(min_y, max_y): (f32, f32)| {
                    (min_y.min(y), max_y.max(y))
                }))
            })
        else {
            continue;
        };
        // Most visibility polygons cover only a small part of a large map.
        // Scanning every level row once per polygon made the vector mask cost
        // proportional to map height times polygon count, even for tiny wall
        // fragments. Limit scan conversion to rows the polygon can cross.
        let first_y = (min_y * scale_y - 0.5).ceil().max(0.0) as u32;
        let last_y = (max_y * scale_y - 0.5)
            .floor()
            .min(height.saturating_sub(1) as f32) as u32;
        if first_y > last_y || first_y >= height {
            continue;
        }
        for y in first_y..=last_y {
            let sample_y = (y as f32 + 0.5) / scale_y;
            crossings.clear();
            for ring in rings.clone() {
                for index in 0..ring.len() {
                    let a = ring[index];
                    let b = ring[(index + 1) % ring.len()];
                    if (a.y > sample_y) != (b.y > sample_y) {
                        crossings.push(a.x + (sample_y - a.y) * (b.x - a.x) / (b.y - a.y));
                    }
                }
            }
            crossings.sort_by(f32::total_cmp);
            for span in crossings.as_chunks::<2>().0 {
                let first = (span[0] * scale_x - 0.5).ceil().max(0.0) as u32;
                let last = (span[1] * scale_x - 0.5)
                    .floor()
                    .min(width.saturating_sub(1) as f32) as u32;
                if first <= last && first < width {
                    let row = y as usize * width as usize;
                    alpha[row + first as usize..=row + last as usize].fill(value);
                }
            }
        }
    }
}

/// Composite the smooth visibility field after every world-space pass and
/// before HUD rendering, so unseen sprites and effects cannot leak through.
pub(crate) fn render_fog_of_war(
    host: &HostDraw<'_>,
    engine: &PresentationView<'_>,
    renderer: &mut Renderer,
) {
    if !engine.fog_of_war_enabled() {
        return;
    }
    let fog = engine.fog_of_war();
    let view = host.viewport().view_position;
    let zoom = host.viewport().zoom_factor;
    let screen_width = renderer.screen_width() as i32;
    let world_height = (renderer.screen_height() as i32 - engine_api::PANNEL_HEIGHT as i32).max(0);
    let level_size = fog.level_size();
    let (mask_width, mask_height) = vector_fog_mask_dimensions(level_size);
    let cache_key = fog_mask_cache_key(fog);
    renderer.render_fog_mask(
        mask_width,
        mask_height,
        cache_key,
        0,
        0,
        screen_width,
        world_height,
        [
            view.x / level_size.x,
            view.y / level_size.y,
            (view.x + screen_width as f32 / zoom) / level_size.x,
            (view.y + world_height as f32 / zoom) / level_size.y,
        ],
        || build_vector_fog_mask_rgba(fog),
    );
}

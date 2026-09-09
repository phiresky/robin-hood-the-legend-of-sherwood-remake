//! Screenshot presentation policy, crop/resize, and PNG encoding.
use crate::http_server::{Reply, ReplyBody, RpcError, ScreenshotFlags, ScreenshotRequest};
use robin_engine::engine as engine_api;
use robin_engine::engine::PANNEL_HEIGHT;
use std::borrow::Cow;
pub(crate) fn can_capture_presented_ui(request: &ScreenshotRequest) -> bool {
    !request.hide_ui && !request.full_map && request.flags == ScreenshotFlags::default()
}

/// Merge a request's `Some(x)` overrides onto `debug`, mutating in
/// place.  Apply this to a **cloned** `DevState` so the live state
/// stays untouched — the caller keeps the original and passes the
/// clone to `render_frame`.
pub fn apply_screenshot_flags(debug: &mut engine_api::DebugFlags, flags: &ScreenshotFlags) {
    macro_rules! set {
        ($name:ident, $field:ident) => {
            if let Some(v) = flags.$name {
                debug.$field = v;
            }
        };
    }
    set!(view_cones, all_view_cones);
    set!(pc_sight, pc_sight);
    set!(motion_graph, motion_graph_display);
    set!(surface, surface_display);
    set!(all_obstacles, all_obstacles_display);
    set!(elevation, elevation_display);
    set!(noise, noise_display);
    set!(sound_source, sound_source_display);
    set!(actor_info, actor_info_display);
    set!(script_zones, script_zone_display);
    set!(door, door_display);
    set!(projection_areas, projection_areas_display);
    set!(railroad, railroad_display);
    set!(probability, prob_display);
    set!(company_number, company_number_display);
    set!(combat_energy, combat_energy_display);
    set!(light_zones, display_light_zones);
    set!(animation_lines, display_animation_lines);
    set!(seek_points, display_seek_points);
    set!(fps, fps_display);
    set!(sprite_masks, sprite_masks_display);
    set!(entity_ids, entity_ids);
}

/// Apply optional crop + resize, then encode as PNG.  Nearest-neighbour
/// scaling — good enough for a dev-inspection endpoint and avoids
/// pulling in an image crate.
pub(crate) fn encode_png(src_w: u32, src_h: u32, rgba: &[u8], req: &ScreenshotRequest) -> Reply {
    // Optional bottom-panel crop: strip the HUD strip before any resize.
    let (src, mut used_w, mut used_h) =
        if req.hide_ui && !req.full_map && src_h > PANNEL_HEIGHT as u32 {
            let new_h = src_h - PANNEL_HEIGHT as u32;
            let stride = (src_w as usize) * 4;
            let cropped: Vec<u8> = rgba[..stride * new_h as usize].to_vec();
            (Cow::Owned(cropped), src_w, new_h)
        } else {
            (Cow::Borrowed(rgba), src_w, src_h)
        };

    let (target_w, target_h) =
        screenshot_target_dimensions(used_w, used_h, req).map_err(RpcError::invalid_request)?;

    let resized;
    let pixels: &[u8] = if (target_w, target_h) != (used_w, used_h) {
        let mut out = vec![0u8; (target_w * target_h * 4) as usize];
        for dy in 0..target_h {
            let sy = (dy * used_h / target_h).min(used_h - 1);
            for dx in 0..target_w {
                let sx = (dx * used_w / target_w).min(used_w - 1);
                let si = ((sy * used_w + sx) * 4) as usize;
                let di = ((dy * target_w + dx) * 4) as usize;
                out[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
        resized = out;
        used_w = target_w;
        used_h = target_h;
        &resized
    } else {
        &src
    };

    let mut png_bytes: Vec<u8> = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, used_w, used_h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| RpcError::internal(format!("png header: {e}")))?;
        writer
            .write_image_data(pixels)
            .map_err(|e| RpcError::internal(format!("png data: {e}")))?;
    }
    Ok(ReplyBody::Binary {
        content_type: "image/png",
        data: png_bytes,
    })
}

fn screenshot_target_dimensions(
    src_w: u32,
    src_h: u32,
    req: &ScreenshotRequest,
) -> Result<(u32, u32), String> {
    let (Some(max_w), Some(max_h)) = (req.width, req.height) else {
        return Ok((src_w, src_h));
    };
    if max_w == 0 || max_h == 0 {
        return Err("screenshot width/height must be > 0".into());
    }

    let max_w = max_w as u32;
    let max_h = max_h as u32;
    let height_for_max_w = ((src_h as u64 * max_w as u64) / src_w as u64) as u32;
    if height_for_max_w <= max_h {
        Ok((max_w, height_for_max_w.max(1)))
    } else {
        let width_for_max_h = ((src_w as u64 * max_h as u64) / src_h as u64) as u32;
        Ok((width_for_max_h.max(1), max_h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_encoding_preserves_pixels_and_nearest_neighbor_resize() {
        let pixels = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        for (bounds, expected_size, expected_pixels) in [
            ((None, None), (2, 2), pixels.to_vec()),
            ((Some(1), Some(1)), (1, 1), vec![255, 0, 0, 255]),
        ] {
            let request = screenshot_request(bounds.0, bounds.1);
            let ReplyBody::Binary { content_type, data } =
                encode_png(2, 2, &pixels, &request).unwrap()
            else {
                panic!("screenshot must return binary PNG");
            };
            assert_eq!(content_type, "image/png");
            let mut reader = png::Decoder::new(std::io::Cursor::new(data))
                .read_info()
                .unwrap();
            let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut decoded).unwrap();
            assert_eq!((frame.width, frame.height), expected_size);
            assert_eq!(&decoded[..frame.buffer_size()], expected_pixels);
        }
    }
    fn screenshot_request(width: Option<u16>, height: Option<u16>) -> ScreenshotRequest {
        ScreenshotRequest {
            width,
            height,
            ..ScreenshotRequest::default()
        }
    }

    #[test]
    fn screenshot_dimensions_fit_width_limited_bounds() {
        let req = screenshot_request(Some(1280), Some(720));
        assert_eq!(
            screenshot_target_dimensions(1024, 768, &req).unwrap(),
            (960, 720)
        );
    }

    #[test]
    fn screenshot_dimensions_fit_height_limited_bounds() {
        let req = screenshot_request(Some(640), Some(480));
        assert_eq!(
            screenshot_target_dimensions(1920, 1080, &req).unwrap(),
            (640, 360)
        );
    }

    #[test]
    fn screenshot_dimensions_leave_size_when_bounds_missing() {
        let req = screenshot_request(Some(640), None);
        assert_eq!(
            screenshot_target_dimensions(1024, 768, &req).unwrap(),
            (1024, 768)
        );
    }

    #[test]
    fn screenshot_dimensions_reject_zero_bounds() {
        let req = screenshot_request(Some(0), Some(720));
        assert!(screenshot_target_dimensions(1024, 768, &req).is_err());
    }

    #[test]
    fn only_plain_ui_screenshots_use_presented_modal_frame() {
        let plain = ScreenshotRequest::default();
        assert!(can_capture_presented_ui(&plain));

        let hidden = ScreenshotRequest {
            hide_ui: true,
            ..plain.clone()
        };
        assert!(!can_capture_presented_ui(&hidden));

        let full_map = ScreenshotRequest {
            full_map: true,
            ..plain.clone()
        };
        assert!(!can_capture_presented_ui(&full_map));

        let overridden = ScreenshotRequest {
            flags: ScreenshotFlags {
                view_cones: Some(true),
                ..ScreenshotFlags::default()
            },
            ..plain
        };
        assert!(!can_capture_presented_ui(&overridden));
    }
}

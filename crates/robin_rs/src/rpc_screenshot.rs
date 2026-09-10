//! Screenshot presentation policy, crop/resize, and PNG encoding.
use crate::http_server::{Reply, ReplyBody, RpcError, ScreenshotFlags, ScreenshotRequest};
use robin_engine::engine as engine_api;
use robin_engine::engine::PANNEL_HEIGHT;
pub(crate) fn can_capture_presented_ui(request: &ScreenshotRequest) -> bool {
    !request.hide_ui && !request.full_map && request.flags == ScreenshotFlags::default()
}

/// Borrow unchanged developer state; isolate request-local overrides in a clone.
pub(crate) fn screenshot_dev_state<'a>(
    dev: &'a engine_api::DevState,
    flags: &ScreenshotFlags,
) -> std::borrow::Cow<'a, engine_api::DevState> {
    if *flags == ScreenshotFlags::default() {
        return std::borrow::Cow::Borrowed(dev);
    }
    let mut snapshot = dev.clone();
    apply_screenshot_flags(&mut snapshot.debug, flags);
    std::borrow::Cow::Owned(snapshot)
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
    let source_len = u64::from(src_w)
        .checked_mul(u64::from(src_h))
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok());
    if src_w == 0 || src_h == 0 || source_len != Some(rgba.len()) {
        return Err(RpcError::internal(format!(
            "invalid captured RGBA frame {src_w}x{src_h}: {} bytes",
            rgba.len()
        )));
    }
    // Optional bottom-panel crop: borrow the prefix before any resize.
    let (src, mut used_w, mut used_h) =
        if req.hide_ui && !req.full_map && src_h > PANNEL_HEIGHT as u32 {
            let new_h = src_h - PANNEL_HEIGHT as u32;
            let stride = (src_w as usize) * 4;
            (&rgba[..stride * new_h as usize], src_w, new_h)
        } else {
            (rgba, src_w, src_h)
        };

    let (target_w, target_h) =
        screenshot_target_dimensions(used_w, used_h, req).map_err(RpcError::invalid_request)?;

    let resized;
    let pixels: &[u8] = if (target_w, target_h) != (used_w, used_h) {
        // Preserve the 32-bit packed output layout, but reject overflow before
        // allocating. TODO: give RPC image processing an explicit memory budget.
        let output_len = target_w
            .checked_mul(target_h)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| RpcError::invalid_request("screenshot dimensions exceed RGBA layout"))?
            as usize;
        let mut out = Vec::new();
        out.try_reserve_exact(output_len)
            .map_err(|error| RpcError::internal(format!("allocating screenshot: {error}")))?;
        out.resize(output_len, 0);
        for dy in 0..target_h {
            let sy = (u64::from(dy) * u64::from(used_h) / u64::from(target_h)) as usize;
            for dx in 0..target_w {
                let sx = (u64::from(dx) * u64::from(used_w) / u64::from(target_w)) as usize;
                let si = (sy * used_w as usize + sx) * 4;
                let di = ((dy * target_w + dx) * 4) as usize;
                out[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
        resized = out;
        used_w = target_w;
        used_h = target_h;
        &resized
    } else {
        src
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
        writer
            .finish()
            .map_err(|e| RpcError::internal(format!("png finish: {e}")))?;
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
    if src_w == 0 || src_h == 0 {
        return Err("screenshot source width/height must be > 0".into());
    }
    let (Some(max_w), Some(max_h)) = (req.width, req.height) else {
        return Ok((src_w, src_h));
    };
    if max_w == 0 || max_h == 0 {
        return Err("screenshot width/height must be > 0".into());
    }

    let max_w = max_w as u32;
    let max_h = max_h as u32;
    let height_for_max_w = (u64::from(src_h) * u64::from(max_w)) / u64::from(src_w);
    if height_for_max_w <= u64::from(max_h) {
        Ok((max_w, height_for_max_w.max(1) as u32))
    } else {
        let width_for_max_h = (u64::from(src_w) * u64::from(max_h)) / u64::from(src_h);
        Ok((width_for_max_h.max(1) as u32, max_h))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn screenshot_state_borrows_without_overrides_and_isolates_explicit_false() {
        let mut live = engine_api::DevState::default();
        live.debug.fps_display = true;
        live.debug.noise_display = true;
        live.noise_display_start_radius = 42;
        let unchanged = super::screenshot_dev_state(&live, &ScreenshotFlags::default());
        assert!(matches!(unchanged, std::borrow::Cow::Borrowed(_)));
        assert!(std::ptr::eq(unchanged.as_ref(), &live));

        let flags = ScreenshotFlags {
            fps: Some(false),
            entity_ids: Some(true),
            ..Default::default()
        };
        let changed = super::screenshot_dev_state(&live, &flags);
        assert!(matches!(changed, std::borrow::Cow::Owned(_)));
        assert!(!changed.debug.fps_display);
        assert!(changed.debug.entity_ids);
        assert!(changed.debug.noise_display);
        assert_eq!(changed.noise_display_start_radius, 42);
        assert!(live.debug.fps_display);
        assert!(!live.debug.entity_ids);
        assert!(live.debug.noise_display);
    }

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
    #[test]
    fn png_encoding_rejects_invalid_sources_and_overflowing_output() {
        let request = ScreenshotRequest::default();
        for (width, height, bytes) in [
            (0, 1, vec![]),
            (1, 0, vec![]),
            (2, 2, vec![0; 15]),
            (2, 2, vec![0; 17]),
            (u32::MAX, u32::MAX, vec![]),
        ] {
            assert!(encode_png(width, height, &bytes, &request).is_err());
        }
        let huge = screenshot_request(Some(u16::MAX), Some(u16::MAX));
        assert!(encode_png(1, 1, &[0; 4], &huge).is_err());
    }

    #[test]
    fn png_encoding_crops_hud_before_resizing_and_keeps_full_maps() {
        let height = PANNEL_HEIGHT as u32 + 2;
        let mut pixels = vec![90; height as usize * 4];
        pixels[..8].copy_from_slice(&[10, 20, 30, 255, 40, 50, 60, 128]);
        for (full_map, bounds, expected_height) in [
            (false, (None, None), 2),
            (false, (Some(1), Some(1)), 1),
            (true, (None, None), height),
        ] {
            let request = ScreenshotRequest {
                hide_ui: true,
                full_map,
                ..screenshot_request(bounds.0, bounds.1)
            };
            let ReplyBody::Binary { data, .. } = encode_png(1, height, &pixels, &request).unwrap()
            else {
                panic!("screenshot must return binary PNG");
            };
            let mut reader = png::Decoder::new(std::io::Cursor::new(data))
                .read_info()
                .unwrap();
            let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut decoded).unwrap();
            assert_eq!((frame.width, frame.height), (1, expected_height));
            assert_eq!(
                &decoded[..frame.buffer_size()],
                &pixels[..expected_height as usize * 4]
            );
        }
    }

    #[test]
    fn screenshot_dimensions_reject_empty_sources_and_preserve_extreme_aspect_ratios() {
        let request = screenshot_request(Some(u16::MAX), Some(u16::MAX));
        for (width, height) in [(0, 1), (1, 0), (0, 0)] {
            assert!(screenshot_target_dimensions(width, height, &request).is_err());
            assert!(
                screenshot_target_dimensions(width, height, &ScreenshotRequest::default()).is_err()
            );
        }
        assert_eq!(
            screenshot_target_dimensions(1, 65_538, &request).unwrap(),
            (1, 65_535)
        );
        assert_eq!(
            screenshot_target_dimensions(65_538, 1, &request).unwrap(),
            (65_535, 1)
        );
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

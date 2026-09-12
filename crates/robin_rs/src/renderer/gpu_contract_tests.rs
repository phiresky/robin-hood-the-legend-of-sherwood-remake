use super::*;

#[cfg(all(test, not(target_arch = "wasm32")))]
fn verify_mouse_trail_pixels(gpu: GpuContext) {
    let mut renderer =
        Renderer::with_optional_surface(gpu, None, None, 1, 1, TextureScaleMode::Nearest);
    let picture = robin_assets::picture::Picture {
        width: 1,
        height: 1,
        pitch: 2,
        pixel_format: robin_assets::picture::PixelFormat::Rgb16,
        data: vec![0, 0],
        palette: None,
    };
    let trail =
        crate::mouse_trail::MouseTrailRenderer::from_picture(&picture, &mut renderer).unwrap();
    let mut way = crate::mouse_way::MouseWay::new();
    way.points
        .push_back(robin_engine::coordinates::ScreenPoint::new(0.0, 0.0));
    way.alpha.push_back(0.0);
    for level in 0u16..=32 {
        way.alpha[0] = f32::from(level) * 100.0 / 32.0;
        renderer.begin_gpu_frame_clear();
        trail.render(&way, &mut renderer);
        let used_alpha = (31 * level) >> 5;
        let pixel = (((0xFC80u16 & 0xF800) >> 5) * used_alpha & 0xF800)
            | (((0xFC80u16 & 0x07E0) >> 5) * used_alpha & 0x07E0);
        let (r, g, b) = robin_util::color::rgb565_to_rgb8(pixel);
        assert_eq!(
            renderer.try_capture_frame_rgba().unwrap(),
            (1, 1, vec![r, g, b, 255]),
            "trail alpha level {level}"
        );
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn verify_loading_dissolve_pixels(gpu: GpuContext) {
    let mut renderer =
        Renderer::with_optional_surface(gpu, None, None, 3, 2, TextureScaleMode::Nearest);
    let initial = [0xF800u16, 0x07E0, 0x001F, 0xFFFF, 0, 0xFFE0];
    let final_pixels = [0x001Fu16, 0xFFFF, 0xF800, 0, 0x07E0, 0xF81F];
    let mask = crate::loading_screen::HeightField {
        data: vec![0, 1, 127, 128, 254, 255],
        width: 3,
        height: 2,
    };
    let textures = renderer
        .create_loading_dissolve_textures(
            3,
            2,
            initial.into_iter(),
            final_pixels.into_iter(),
            &mask,
        )
        .unwrap();
    for threshold in [256, 255, 128, 0] {
        renderer.begin_gpu_frame_clear();
        renderer.render_loading_dissolve(&textures, threshold, Rect::new(0, 0, 3, 2));
        let expected: Vec<u8> = mask
            .data
            .iter()
            .enumerate()
            .flat_map(|(index, &height)| {
                let pixel = if u32::from(height) > threshold {
                    final_pixels[index]
                } else {
                    initial[index]
                };
                let (r, g, b) = robin_util::color::rgb565_to_rgb8(pixel);
                [r, g, b, 255]
            })
            .collect();
        assert_eq!(
            renderer.try_capture_frame_rgba().unwrap(),
            (3, 2, expected),
            "dissolve threshold {threshold}"
        );
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn verify_offscreen_gpu_contract(gpu: GpuContext) {
    verify_mouse_trail_pixels(gpu.clone());
    verify_loading_dissolve_pixels(gpu.clone());
    verify_mask_atlas_pixels(gpu.clone());
    let mut other_renderer =
        Renderer::with_optional_surface(gpu.clone(), None, None, 3, 2, TextureScaleMode::Nearest);
    let mut renderer =
        Renderer::with_optional_surface(gpu, None, None, 3, 2, TextureScaleMode::Nearest);
    assert_eq!(
        renderer.target_dimensions(SurfaceTarget::Screen).unwrap(),
        (3, 2)
    );
    assert_eq!(
        renderer.legacy_surface_target(0).unwrap(),
        renderer.legacy_surface_target(1).unwrap()
    );
    assert!(renderer.surface_handle(0).is_err());
    assert!(renderer.surface_handle(1).is_err());
    let local_id = renderer
        .create_surface_from_rgb565(1, 1, &[0xffff])
        .unwrap();
    let other_id = other_renderer
        .create_surface_from_rgb565(1, 1, &[0xffff])
        .unwrap();
    assert_eq!(local_id, other_id);
    let owned = renderer.adopt_surface(local_id);
    assert!(other_renderer.surface_dimensions(owned.handle()).is_err());
    assert!(
        other_renderer
            .draw_surface(owned.handle(), None, None, 0)
            .is_err()
    );
    let before_foreign_draw = other_renderer.draw_queue_checkpoint();
    assert!(
        other_renderer
            .draw_surface_alpha(owned.handle(), None, None, 25, 0)
            .is_err()
    );
    assert!(
        other_renderer
            .draw_surface_with_shadow(owned.handle(), None, None, 40, BLIT_SOURCE_TRANSPARENT)
            .is_err()
    );
    assert_eq!(other_renderer.draw_queue_checkpoint(), before_foreign_draw);
    assert!(other_renderer.surface_alpha_mask(owned.handle()).is_err());
    let restored: OwnedSurface =
        serde_json::from_str(&serde_json::to_string(&owned).unwrap()).unwrap();
    assert!(renderer.surface_dimensions(restored.handle()).is_err());
    assert!(renderer.try_retire_surface(restored).is_err());
    assert!(renderer.surface_dimensions(owned.handle()).is_ok());
    let (_, owned) = other_renderer.try_retire_surface(owned).unwrap_err();
    assert!(renderer.surface_dimensions(owned.handle()).is_ok());
    let mut mission = crate::mission_render_resources::MissionRenderResources::default();
    let other_owned = other_renderer.adopt_surface(other_id);
    mission.replace_map(&mut other_renderer, other_owned);
    assert!(mission.try_retire(&mut renderer).is_err());
    assert_eq!(
        mission.map(),
        Some(other_renderer.surface_handle(other_id).unwrap())
    );
    let borrowed_map = mission.map().unwrap();
    assert!(renderer.draw_surface(borrowed_map, None, None, 0).is_err());
    assert!(
        renderer
            .draw_surface_alpha(borrowed_map, None, None, 0, 0)
            .is_err()
    );
    assert!(other_renderer.surface_handle(other_id).is_ok());
    let (_, owned) = mission
        .try_replace_map(&mut other_renderer, owned)
        .unwrap_err();
    assert!(renderer.surface_dimensions(owned.handle()).is_ok());
    assert!(
        mission
            .try_replace_map(&mut other_renderer, OwnedSurface::synthetic(u32::MAX))
            .is_err()
    );
    assert_eq!(
        mission.map(),
        Some(other_renderer.surface_handle(other_id).unwrap())
    );
    assert!(renderer.try_adopt_surface(local_id).is_err());
    assert!(renderer.try_delete_legacy_surface(local_id).is_err());
    assert!(other_renderer.surface_handle(other_id).is_ok());
    renderer.retire_surface(owned);
    assert!(renderer.surface_handle(local_id).is_err());
    assert!(other_renderer.surface_handle(other_id).is_ok());
    mission.retire(&mut other_renderer);
    assert!(
        other_renderer
            .draw_surface(borrowed_map, None, None, 0)
            .is_err()
    );
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255,
        0, 255,
    ];
    let image = renderer
        .create_rgba_gpu_image(3, 2, &pixels, "capture contract")
        .unwrap();
    renderer
        .upload_mask_alphas([(7, &[2, 0, 0, 255, 0, 0][..], 3, 2)])
        .unwrap();
    let checkpoint = renderer.draw_queue_checkpoint();
    renderer.render_gpu_image(&image, None, None, BlendMode::None);
    renderer.mask_queued_draws(
        checkpoint,
        &[(7, Rect::new(0, 0, 3, 2))],
        Rect::new(0, 0, 3, 2),
    );
    // Zero-alpha overlay still exercises framebuffer snapshot/pass ordering.
    renderer.render_framebuffer_alpha_rect(Rect::new(0, 0, 3, 2), [0.0, 0.0, 1.0, 1.0], 0, 0);
    renderer.begin_ui_layer();
    renderer.render_gpu_rect(2, 0, 1, 1, 255, 255, 255, 255);
    let expected = vec![
        0, 0, 0, 255, 0, 255, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0,
        255,
    ];
    assert_eq!(
        renderer.try_capture_frame_rgba().unwrap(),
        (3, 2, expected.clone())
    );
    assert_eq!(renderer.draw_queue_checkpoint(), 0);
    renderer.freeze_scene_for_modal();
    assert_eq!(renderer.try_capture_frame_rgba().unwrap().2, expected);
    let queued_image = renderer
        .create_rgba_gpu_image(3, 2, &[255, 0, 0, 255].repeat(6), "pending live frame")
        .unwrap();
    renderer.render_gpu_image(&queued_image, None, None, BlendMode::None);
    let queued = renderer.draw_queue_checkpoint();
    let live_texture = renderer.frame.render_target_texture.clone();
    let live_backdrop = renderer.frame.frozen_scene.as_ref().unwrap().0.clone();
    let mut capture_texture = None;
    for fail in [false, true, false] {
        let result = (|| -> Result<(), CaptureError> {
            let mut target = renderer.capture_target(7, 5);
            if let Some(previous) = &capture_texture {
                assert_eq!(
                    &target.frame.render_target_texture, previous,
                    "same-size captures must reuse their target allocation"
                );
            } else {
                capture_texture = Some(target.frame.render_target_texture.clone());
            }
            assert_ne!(target.frame.render_target_texture, live_texture);
            assert!(target.frame.frozen_scene.is_none());
            let image = target
                .create_rgba_gpu_image(7, 5, &[0, 0, 255, 255].repeat(35), "capture frame")
                .unwrap();
            target.render_gpu_image(&image, None, None, BlendMode::None);
            target.begin_ui_layer();
            target.render_gpu_rect(0, 0, 1, 1, 0, 255, 0, 255);
            if fail {
                // Exercise the same early-return path as failed mapping,
                // including unsubmitted capture-only world/UI commands.
                return Err(CaptureError::CompletionLost);
            }
            let (w, h, pixels) = target.try_capture_frame_rgba()?;
            assert_eq!((w, h), (7, 5));
            assert_eq!(&pixels[..4], &[0, 255, 0, 255]);
            assert_eq!(&pixels[4..], &[0, 0, 255, 255].repeat(34));
            target.freeze_scene_for_modal();
            Ok(())
        })();
        assert_eq!(result.is_err(), fail);
        assert_eq!((renderer.screen_width(), renderer.screen_height()), (3, 2));
        assert_eq!(renderer.frame.render_target_texture, live_texture);
        assert_eq!(
            renderer.frame.frozen_scene.as_ref().unwrap().0,
            live_backdrop
        );
        assert_eq!(renderer.draw_queue_checkpoint(), queued);
        assert_eq!(
            renderer.try_capture_presented_frame_rgba().unwrap().2,
            expected
        );
    }
    assert_eq!(
        renderer.try_capture_presented_frame_rgba().unwrap().2,
        expected
    );
    assert_eq!(
        renderer.draw_queue_checkpoint(),
        queued,
        "presented capture must not consume pending commands"
    );
    assert_eq!(
        renderer.try_capture_frame_rgba().unwrap().2,
        [255, 0, 0, 255].repeat(6)
    );
    // Detaching a submitted capture must preserve the old frame, even when
    // another frame overwrites the logical target before mapping starts.
    renderer.render_gpu_rect(0, 0, 3, 2, 0, 255, 0, 255);
    let pending_capture = renderer.begin_capture_frame_rgba();
    assert_eq!(renderer.draw_queue_checkpoint(), 0);
    renderer.render_gpu_rect(0, 0, 3, 2, 0, 0, 255, 255);
    assert_eq!(
        renderer.try_capture_frame_rgba().unwrap().2,
        [0, 0, 255, 255].repeat(6)
    );
    assert_eq!(
        pollster::block_on(pending_capture).unwrap().2,
        [0, 255, 0, 255].repeat(6)
    );
    let id = renderer
        .create_surface_from_rgb565(1, 1, &[0xffff])
        .unwrap();
    let handle = renderer.surface_handle(id).unwrap();
    assert_eq!(renderer.surface_dimensions(handle).unwrap(), (1, 1));
    assert!(renderer.delete_surface(id));
    assert!(renderer.surface_dimensions(handle).is_err());
    let replacement = renderer
        .create_surface_from_rgb565(1, 1, &[0xffff])
        .unwrap();
    assert_ne!(id, replacement, "deleted surface IDs must not be reused");
    crate::mission_render_resources::verify_gpu_lifecycle(&mut renderer);
    crate::corner_hud::verify_gpu_ownership(&mut renderer);
    crate::hud_sprite::tests::verify_gpu_ownership(&mut renderer);
    crate::main_menu::credits::verify_gpu_retirement(&mut renderer);
    let mut portrait_renderer = Renderer::with_optional_surface(
        renderer.gpu.clone(),
        None,
        None,
        3,
        2,
        TextureScaleMode::Nearest,
    );
    let mut portrait_peer = Renderer::with_optional_surface(
        renderer.gpu.clone(),
        None,
        None,
        3,
        2,
        TextureScaleMode::Nearest,
    );
    crate::ui_panel::verify_portrait_gpu_ownership(&mut portrait_renderer, &mut portrait_peer);
    let mut menu_renderer = Renderer::with_optional_surface(
        renderer.gpu.clone(),
        None,
        None,
        3,
        2,
        TextureScaleMode::Nearest,
    );
    let mut menu_peer = Renderer::with_optional_surface(
        renderer.gpu.clone(),
        None,
        None,
        3,
        2,
        TextureScaleMode::Nearest,
    );
    crate::ingame_menu::resources::verify_menu_gpu_ownership(&mut menu_renderer, &mut menu_peer);
    verify_deferred_menu_surfaces(&mut renderer);
    verify_managed_surface_rectangles(&mut renderer);
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn verify_managed_surface_rectangles(renderer: &mut Renderer) {
    let upload = renderer.upload_rgb565(3, 2, &[0xffff; 6]).unwrap();
    let handle = upload.handle();
    let empty = BBox::from_coords(1.0, 0.0, 1.0, 2.0);
    let inverted = BBox::from_coords(2.0, 2.0, 1.0, 0.0);
    let subpixel = BBox::from_coords(0.0, 0.0, 0.5, 0.5);
    let full = BBox::from_coords(0.0, 0.0, 3.0, 2.0);
    let fractional_src = BBox::from_coords(-0.5, 0.25, 2.5, 1.75);
    let fractional_dst = BBox::from_coords(-0.75, 0.5, 2.75, 2.5);
    for mode in 0..3 {
        let draw = |renderer: &mut Renderer, src, dst| match mode {
            0 => renderer.draw_surface(handle, src, dst, BLIT_SOURCE_TRANSPARENT),
            1 => renderer.draw_surface_alpha(handle, src, dst, 0, BLIT_SOURCE_TRANSPARENT),
            2 => renderer.draw_surface_with_shadow(handle, src, dst, 50, BLIT_SOURCE_TRANSPARENT),
            _ => unreachable!(),
        };
        for (src, dst) in [
            (Some(&empty), None),
            (Some(&inverted), Some(&full)),
            (None, Some(&empty)),
            (None, Some(&inverted)),
            (Some(&subpixel), None),
            (None, Some(&subpixel)),
        ] {
            let queued = renderer.draw_queue_checkpoint();
            draw(renderer, src, dst).unwrap();
            assert_eq!(renderer.draw_queue_checkpoint(), queued, "mode {mode}");
        }
        // Fractional (including negative) source coordinates survive in UVs;
        // only screen geometry truncates to integer pixels.
        draw(renderer, Some(&fractional_src), Some(&fractional_dst)).unwrap();
        let quad = renderer.frame.queued.last().unwrap();
        assert_eq!(quad.dst, Rect::new(0, 0, 3, 2));
        assert_eq!(quad.uv, [-0.5 / 3.0, 0.125, 2.5 / 3.0, 0.875]);
        renderer.try_capture_frame_rgba().unwrap();
        // A subpixel source remains drawable when explicitly stretched.
        let queued = renderer.draw_queue_checkpoint();
        draw(renderer, Some(&subpixel), Some(&full)).unwrap();
        assert_eq!(renderer.draw_queue_checkpoint(), queued + 1);
        renderer.try_capture_frame_rgba().unwrap();
    }
    renderer.retire_surface(upload);
    // Empty geometry never hides a stale ownership error.
    assert!(
        renderer
            .draw_surface(handle, Some(&empty), None, 0)
            .is_err()
    );
    assert!(
        renderer
            .draw_surface_alpha(handle, Some(&empty), None, 0, 0)
            .is_err()
    );
    assert!(
        renderer
            .draw_surface_with_shadow(handle, Some(&empty), None, 50, 0)
            .is_err()
    );
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn verify_deferred_menu_surfaces(renderer: &mut Renderer) {
    let pixels = [
        0xf800,
        TRANSPARENT_COLOR_KEY_16,
        SHADOW_KEY,
        0x07e0,
        0x001f,
        0xffff,
    ];
    // Every rendering entry point must realize a deferred surface, including
    // the shadow override and whole-widget fade paths used by modal menus.
    for mode in 0..5 {
        let eager = renderer.create_surface_from_rgb565(3, 2, &pixels).unwrap();
        let deferred = renderer.create_deferred_surface_from_rgb565(3, 2, Box::new(pixels));
        let owned = renderer.adopt_surface(deferred);
        let handle = owned.handle();
        assert_eq!(renderer.surface_dimensions(handle).unwrap(), (3, 2));
        let mask = renderer.build_alpha_mask(deferred).unwrap();
        assert!(mask.is_opaque(0, 0));
        assert!(!mask.is_opaque(1, 0));
        assert!(
            matches!(
                renderer.resources.managed_surfaces[&deferred].pixels,
                ManagedSurfacePixels::Pending(_)
            ),
            "metadata access must not upload unopened menus"
        );
        renderer.set_shadow_alpha(eager, MENU_BUTTON_SHADOW_ALPHA);
        renderer.set_shadow_alpha(deferred, MENU_BUTTON_SHADOW_ALPHA);
        let mut captured = Vec::new();
        for id in [eager, deferred, deferred] {
            renderer.finish_loading_screen();
            renderer.render_gpu_rect(0, 0, 3, 2, 255, 255, 255, 255);
            let draw_handle = renderer.surface_handle(id).unwrap();
            let drawn = match mode {
                0 => renderer.draw_surface(draw_handle, None, None, 0),
                1 => renderer.draw_surface(draw_handle, None, None, BLIT_SOURCE_TRANSPARENT),
                2 => renderer.draw_surface_alpha(draw_handle, None, None, 35, 0),
                3 => renderer.draw_surface_alpha(
                    draw_handle,
                    None,
                    None,
                    35,
                    BLIT_SOURCE_TRANSPARENT,
                ),
                4 => renderer.draw_surface_with_shadow(
                    draw_handle,
                    None,
                    None,
                    50,
                    BLIT_SOURCE_TRANSPARENT,
                ),
                _ => unreachable!(),
            };
            drawn.unwrap();
            captured.push(renderer.try_capture_frame_rgba().unwrap());
        }
        assert_eq!(
            captured[0], captured[1],
            "deferred first draw differs for mode {mode}"
        );
        assert_eq!(
            captured[1], captured[2],
            "reopening menu differs for mode {mode}"
        );
        assert!(matches!(
            renderer.resources.managed_surfaces[&deferred].pixels,
            ManagedSurfacePixels::Resident(_)
        ));
        renderer.retire_surface(owned);
        assert!(renderer.surface_dimensions(handle).is_err());
        assert!(renderer.delete_surface(eager));
    }
    let never_opened = renderer.create_deferred_surface_from_rgb565(1, 1, Box::new([0xffff]));
    let owned = renderer.adopt_surface(never_opened);
    renderer.retire_surface(owned);
    assert!(
        !renderer
            .resources
            .managed_surfaces
            .contains_key(&never_opened)
    );

    // The loading renderer may have queued geometry, a frozen scene and a
    // UI-only presentation policy. Handoff must not carry its pixels into the
    // first mission frame, even when logical dimensions do not change.
    renderer.freeze_scene_for_modal();
    renderer.begin_ui_only_frame();
    renderer.render_gpu_rect(0, 0, 3, 2, 255, 0, 0, 255);
    let identity = renderer.identity;
    renderer.finish_loading_screen();
    assert_eq!(
        renderer.identity, identity,
        "handoff must retain renderer ownership"
    );
    assert_eq!(renderer.draw_queue_checkpoint(), 0);
    assert!(renderer.frame.frozen_scene.is_none());
    // Lost-Sherwood can open its debriefing before the first world render.
    renderer.freeze_scene_for_modal();
    assert_eq!(
        renderer.try_capture_presented_frame_rgba().unwrap().2,
        vec![0; 3 * 2 * 4],
        "handoff must not expose loading pixels before the first composition"
    );
    assert_eq!(
        renderer.try_capture_frame_rgba().unwrap().2,
        vec![0; 3 * 2 * 4],
        "an immediate modal must freeze the cleared target"
    );
    renderer.clear_frozen_scene();
    assert_eq!(
        renderer.try_capture_frame_rgba().unwrap().2,
        [0, 0, 0, 255].repeat(6)
    );

    // Reproduce normal presentation's split world/UI composition before a
    // pause snapshot. Captures normally draw the full queue and would hide
    // the regression where only the world survived in the logical target.
    renderer.render_gpu_rect(0, 0, 3, 2, 0, 255, 0, 255);
    renderer.begin_ui_layer();
    renderer.render_gpu_rect(2, 0, 1, 2, 255, 0, 0, 255);
    renderer.frame.push_implicit_base_quad();
    renderer.frame.upload_queue_geometry(&renderer.gpu);
    let mut encoder = renderer
        .gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("split UI modal regression"),
        });
    renderer
        .frame
        .encode_scene_to_rt(&mut encoder, &renderer.pipelines, &renderer.resources);
    renderer.frame.encode_ui_to_logical_frame(
        &mut encoder,
        &renderer.pipelines,
        &renderer.resources,
    );
    renderer.gpu.queue.submit(Some(encoder.finish()));
    renderer.frame.clear_recording();
    renderer.freeze_scene_for_modal();
    let expected = [0, 255, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255].repeat(2);
    assert_eq!(renderer.try_capture_frame_rgba().unwrap().2, expected);
    renderer.colorize_framebuffer(210.0, 0.35);
    let tinted = renderer.try_capture_frame_rgba().unwrap().2;
    for pixel in tinted.as_chunks::<4>().0 {
        assert!(
            pixel[2] > pixel[0],
            "pause tint must include world and portrait: {pixel:?}"
        );
        assert_eq!(pixel[3], 255);
    }
    renderer.clear_frozen_scene();
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn verify_mask_atlas_pixels(gpu: GpuContext) {
    let mut renderer =
        Renderer::with_optional_surface(gpu, None, None, 31, 19, TextureScaleMode::Nearest);
    let image = renderer
        .create_rgba_gpu_image(31, 19, &[255; 31 * 19 * 4], "atlas parity")
        .unwrap();
    let filler = vec![1; 2040 * 8];
    let narrow = [0, 2, 0, 255, 1, 0, 1];
    let pattern = [0, 1, 0, 1, 0, 1];
    let masks = [
        (10, filler.as_slice(), 2040, 8),
        (11, narrow.as_slice(), 1, 7),
        (12, pattern.as_slice(), 3, 2),
    ];
    renderer.upload_mask_alphas(masks).unwrap();
    assert!(
        renderer.resources.mask_alpha_cache[&11]
            .atlas
            .unwrap()
            .origin[0]
            > 2000
    );
    renderer.upload_mask_alpha(21, &narrow, 1, 7).unwrap();
    renderer.upload_mask_alpha(22, &pattern, 3, 2).unwrap();
    renderer
        .upload_occlusion_depth(&[0, 255, 256, 65535, 32767, 32768], 3, 2)
        .unwrap();
    for depth in [None, Some((Rect::new(0, 0, 31, 19), 0.4))] {
        for (atlas_id, standalone_id) in [(11, 21), (12, 22)] {
            for uv in [
                [0.0, 0.0, 1.0, 1.0],
                [-9.0, -3.0, 5.0, 8.0],
                [0.125, 0.13, 0.91, 0.87],
            ] {
                let mut captures = Vec::new();
                for id in [standalone_id, atlas_id] {
                    renderer.render_gpu_rect(0, 0, 31, 19, 0, 0, 0, 255);
                    let checkpoint = renderer.draw_queue_checkpoint();
                    renderer.render_gpu_image(&image, None, None, BlendMode::None);
                    renderer.mask_queued_draws_impl(
                        checkpoint,
                        &[(id, Rect::new(0, 0, 31, 19))],
                        Rect::new(0, 0, 31, 19),
                        depth,
                    );
                    for draw in &mut renderer.frame.queued[checkpoint..] {
                        if matches!(draw.operation, DrawOperation::MaskAlpha(index) if index == id)
                        {
                            draw.uv = uv;
                            draw.corners =
                                Some([(0.25, 0.75), (30.5, 0.75), (0.25, 18.25), (30.5, 18.25)]);
                        }
                    }
                    captures.push(renderer.try_capture_frame_rgba().unwrap().2);
                }
                assert_eq!(captures[0], captures[1], "atlas {atlas_id} UV {uv:?}");
                assert!(captures[0].as_chunks::<4>().0.iter().any(|p| p[0] == 255));
                assert!(captures[0].as_chunks::<4>().0.iter().any(|p| p[0] == 0));
            }
        }
    }
    assert!(
        renderer.resources.mask_alpha_cache[&OCCLUSION_DEPTH_TEXTURE_INDEX]
            .atlas
            .is_none()
    );
    assert!(renderer.upload_mask_alphas([(50, &[][..], 0, 0)]).is_err());
    assert!(renderer.upload_mask_alphas([(50, &[1][..], 2, 2)]).is_err());
    let page = vec![1; 2046 * 2046];
    let oversized = vec![1; 2048];
    renderer
        .upload_mask_alphas([
            (60, page.as_slice(), 2046, 2046),
            (61, page.as_slice(), 2046, 2046),
            (62, oversized.as_slice(), 2048, 1),
        ])
        .unwrap();
    assert!(renderer.resources.mask_alpha_cache[&60].atlas.is_some());
    assert!(renderer.resources.mask_alpha_cache[&61].atlas.is_some());
    assert!(renderer.resources.mask_alpha_cache[&62].atlas.is_none());
    assert_ne!(
        renderer.resources.mask_alpha_cache[&60]._texture,
        renderer.resources.mask_alpha_cache[&61]._texture
    );
    renderer.clear_mask_alpha_cache();
    renderer.upload_mask_alphas(std::iter::empty()).unwrap();
    assert!(renderer.resources.mask_alpha_cache.is_empty());
}

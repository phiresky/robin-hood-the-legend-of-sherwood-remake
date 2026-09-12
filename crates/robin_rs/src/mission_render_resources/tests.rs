use super::*;

// Synthetic IDs exist only in this pure ownership test; production ownership
// always validates live renderer uploads before accepting them.
fn frame(id: u32) -> SpriteSurface {
    SpriteSurface {
        upload: OwnedSurface::synthetic(id),
        width: 3,
        height: 4,
    }
}

#[test]
fn sparse_frames_keep_indices_and_retirement_is_idempotent() {
    let mut owner = MissionRenderResources {
        map: Some(frame(2)),
        ..Default::default()
    };
    owner.dots.frames = vec![Some(frame(3)), None, Some(frame(4))];
    assert!(owner.dots()[1].is_none());
    assert_eq!(
        owner.dots()[2].as_ref().unwrap().parts(),
        (OwnedSurface::synthetic(4).handle(), 3, 4)
    );
    let mut deleted = Vec::new();
    owner.retire_with(&mut |id| deleted.push(id.handle().legacy_id()));
    owner.retire_with(&mut |id| deleted.push(id.handle().legacy_id()));
    assert_eq!(deleted, [2, 3, 4]);
    assert!(owner.map().is_none());
}

#[test]
fn sprite_preparation_retires_partial_banks_but_keeps_map() {
    let mut owner = MissionRenderResources {
        map: Some(frame(2)),
        ..Default::default()
    };
    owner.corners.frames = vec![Some(frame(3))];
    owner.corner_size = ScreenSize::new(3.0, 4.0);
    let mut deleted = Vec::new();
    owner.retire_sprite_banks(&mut |id| deleted.push(id.handle().legacy_id()));
    assert_eq!(deleted, [3]);
    assert_eq!(owner.map(), Some(OwnedSurface::synthetic(2).handle()));
    assert_eq!(owner.corner_size(), ScreenSize::default());
    assert!(owner.corner(0).is_none());
}

#[test]
fn diagnostics_cannot_resurrect_gpu_ownership() {
    let owner = MissionRenderResources {
        map: Some(frame(2)),
        ..Default::default()
    };
    let restored: MissionRenderResources =
        serde_json::from_str(&serde_json::to_string(&owner).unwrap()).unwrap();
    assert!(restored.map().is_none());
    assert_eq!(owner.map(), Some(OwnedSurface::synthetic(2).handle()));
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn verify_gpu_lifecycle(renderer: &mut Renderer) {
    let mut host = crate::host::Host::scratch(3.0, 2.0);
    let upload = |renderer: &mut Renderer| renderer.upload_rgb565(1, 1, &[0xffff]).unwrap();
    let mut previous = None;
    for _ in 0..3 {
        let owned = upload(renderer);
        let id = owned.handle();
        if let Some(old) = previous {
            renderer.draw_surface(old, None, None, 0).unwrap();
        }
        host.frontend
            .resources
            .mission_surfaces
            .replace_map(renderer, owned);
        if let Some(old) = previous {
            assert!(renderer.surface_dimensions(old).is_err());
            assert_eq!(
                &renderer.try_capture_frame_rgba().unwrap().2[..4],
                &[248, 252, 248, 255],
                "retiring a managed ID must preserve already queued draw bindings"
            );
        }
        previous = Some(id);
        host.post_load_reset();
        assert_eq!(host.frontend.resources.mission_surfaces.map(), Some(id));
        assert!(renderer.surface_dimensions(id).is_ok());
    }
    let dot = upload(renderer);
    let corner_size = ScreenSize::new(12.0, 15.0);
    host.frontend
        .resources
        .mission_surfaces
        .replace_corners(renderer, corner_size, vec![]);
    assert_eq!(
        host.frontend.resources.mission_surfaces.corner_size(),
        corner_size,
        "known layout dimensions survive unavailable corner pictures"
    );
    assert!(host.frontend.resources.mission_surfaces.corner(0).is_none());
    assert!(renderer.upload_rgb565(2, 2, &[0xffff]).is_none());
    assert_eq!(
        host.frontend.resources.mission_surfaces.map(),
        previous,
        "a failed upload must leave the installed map intact"
    );
    host.frontend
        .resources
        .mission_surfaces
        .replace_dots(renderer, vec![None, Some(dot)]);
    assert!(host.frontend.resources.mission_surfaces.dots()[0].is_none());
    let borrowed_dot = host.frontend.resources.mission_surfaces.dots()[1]
        .as_ref()
        .unwrap()
        .parts()
        .0;
    renderer
        .draw_surface_alpha(
            borrowed_dot,
            None,
            None,
            0,
            crate::renderer::BLIT_SOURCE_TRANSPARENT,
        )
        .unwrap();
    assert_eq!(
        &renderer.try_capture_frame_rgba().unwrap().2[..4],
        &[248, 252, 248, 255]
    );
    renderer
        .draw_surface_with_shadow(
            borrowed_dot,
            None,
            None,
            40,
            crate::renderer::BLIT_SOURCE_TRANSPARENT,
        )
        .unwrap();
    assert_eq!(
        &renderer.try_capture_frame_rgba().unwrap().2[..4],
        &[248, 252, 248, 255]
    );
    // Decoding cannot duplicate an owner. Rejection returns the entire candidate
    // bank without retiring its valid prefix or modifying the installed bank.
    let candidate = upload(renderer);
    let candidate_handle = candidate.handle();
    let decoded: OwnedSurface =
        serde_json::from_value(serde_json::to_value(&candidate).unwrap()).unwrap();
    let (_, rejected) = host
        .frontend
        .resources
        .mission_surfaces
        .try_replace_dots(renderer, vec![Some(candidate), Some(decoded)])
        .unwrap_err();
    assert_eq!(
        host.frontend.resources.mission_surfaces.dots()[1]
            .as_ref()
            .unwrap()
            .parts()
            .0,
        borrowed_dot
    );
    assert!(renderer.surface_dimensions(candidate_handle).is_ok());
    let mut rejected = rejected.into_iter().flatten();
    renderer.retire_surface(rejected.next().unwrap());
    assert!(
        renderer
            .try_retire_surface(rejected.next().unwrap())
            .is_err()
    );
    host.frontend
        .resources
        .mission_surfaces
        .replace_dots(renderer, vec![]);
    assert!(renderer.draw_surface(borrowed_dot, None, None, 0).is_err());
    for reserved in [0, 1] {
        assert!(
            host.frontend
                .resources
                .mission_surfaces
                .try_replace_dots(renderer, vec![Some(OwnedSurface::synthetic(reserved))])
                .is_err()
        );
    }
    let candidate = upload(renderer);
    let (_, candidates) = host
        .frontend
        .resources
        .mission_surfaces
        .try_replace_corners(
            renderer,
            ScreenSize::new(99.0, 99.0),
            vec![candidate, OwnedSurface::synthetic(u32::MAX)],
        )
        .unwrap_err();
    assert_eq!(
        host.frontend.resources.mission_surfaces.corner_size(),
        corner_size
    );
    let (_, candidates) = host
        .frontend
        .resources
        .mission_surfaces
        .try_replace_ground_marks(renderer, candidates)
        .unwrap_err();
    let mut candidates = candidates.into_iter();
    renderer.retire_surface(candidates.next().unwrap());
    assert!(
        renderer
            .try_retire_surface(candidates.next().unwrap())
            .is_err()
    );
    host.frontend
        .request_print_screen(crate::host::PrintScreenRequest::Plain);
    host.frontend.retire_mission(renderer);
    assert!(host.frontend.take_print_screen().is_none());
    assert!(renderer.surface_dimensions(previous.unwrap()).is_err());
    host.frontend.retire_mission(renderer);
}

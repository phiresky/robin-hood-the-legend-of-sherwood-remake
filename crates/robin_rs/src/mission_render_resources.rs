//! Renderer-local resources retained across snapshot restores, retired with a mission.
//!
//! Handles are deliberately not serialized or cloned with their owner. Replacing a
//! bank retires its old uploads; absent frames remain absent rather than aliasing
//! the renderer's screen surface (legacy ID zero).

use crate::renderer::{
    MissingSurface, OwnedSurface, Renderer, SurfaceHandle, SurfaceOwnershipError,
};
use robin_engine::coordinates::ScreenSize;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SpriteSurface {
    upload: OwnedSurface,
    width: u16,
    height: u16,
}

impl SpriteSurface {
    pub fn parts(&self) -> (SurfaceHandle, u16, u16) {
        (self.upload.handle(), self.width, self.height)
    }

    fn uploaded(renderer: &Renderer, upload: OwnedSurface) -> Self {
        let (width, height) = renderer
            .surface_dimensions(upload.handle())
            .expect("mission upload must reference a live renderer surface");
        Self {
            upload,
            width,
            height,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct SpriteBank {
    frames: Vec<Option<SpriteSurface>>,
}

impl SpriteBank {
    fn retire(&mut self, delete: &mut impl FnMut(OwnedSurface)) {
        for frame in self.frames.drain(..).flatten() {
            delete(frame.upload);
        }
    }
}

/// Unique owner of mission minimap and marker uploads. Deserialize yields an
/// empty owner: GPU authority cannot be restored from diagnostic JSON.
#[derive(Default, Serialize, Deserialize)]
pub struct MissionRenderResources {
    #[serde(skip)]
    map: Option<SpriteSurface>,
    #[serde(skip)]
    corners: SpriteBank,
    #[serde(skip)]
    corner_size: ScreenSize,
    #[serde(skip)]
    dots: SpriteBank,
    #[serde(skip)]
    ground_marks: SpriteBank,
}

fn delete_surface(renderer: &mut Renderer, upload: OwnedSurface) {
    renderer.retire_surface(upload);
}

impl MissionRenderResources {
    pub fn map(&self) -> Option<SurfaceHandle> {
        self.map.as_ref().map(|frame| frame.upload.handle())
    }

    pub fn corner_size(&self) -> ScreenSize {
        self.corner_size
    }

    pub fn corner(&self, index: usize) -> Option<SurfaceHandle> {
        self.corners
            .frames
            .get(index)
            .or(self.corners.frames.first())
            .and_then(|frame| frame.as_ref())
            .map(|frame| frame.upload.handle())
    }

    pub fn dots(&self) -> &[Option<SpriteSurface>] {
        &self.dots.frames
    }
    pub fn ground_marks(&self) -> &[Option<SpriteSurface>] {
        &self.ground_marks.frames
    }

    pub fn replace_map(&mut self, renderer: &mut Renderer, upload: OwnedSurface) {
        self.try_replace_map(renderer, upload)
            .expect("mission map replacement requires a local owned upload");
    }

    pub fn try_replace_map(
        &mut self,
        renderer: &mut Renderer,
        upload: OwnedSurface,
    ) -> Result<(), (SurfaceOwnershipError, OwnedSurface)> {
        if let Err(error) = self.validate_replacement(renderer, [&upload]) {
            return Err((error, upload));
        }
        let frame = SpriteSurface::uploaded(renderer, upload);
        if let Some(previous) = self.map.replace(frame) {
            delete_surface(renderer, previous.upload);
        }
        Ok(())
    }

    pub fn replace_corners(
        &mut self,
        renderer: &mut Renderer,
        size: ScreenSize,
        ids: Vec<OwnedSurface>,
    ) {
        self.try_replace_corners(renderer, size, ids)
            .expect("mission corners require unique local owned uploads");
    }

    pub fn try_replace_corners(
        &mut self,
        renderer: &mut Renderer,
        size: ScreenSize,
        ids: Vec<OwnedSurface>,
    ) -> Result<(), (SurfaceOwnershipError, Vec<OwnedSurface>)> {
        if let Err(error) = self.validate_replacement(renderer, &ids) {
            return Err((error, ids));
        }
        Self::replace_bank(&mut self.corners, renderer, ids.into_iter().map(Some));
        self.corner_size = size;
        Ok(())
    }

    pub fn replace_dots(&mut self, renderer: &mut Renderer, ids: Vec<Option<OwnedSurface>>) {
        self.try_replace_dots(renderer, ids)
            .expect("mission dots require unique local owned uploads");
    }

    pub fn try_replace_dots(
        &mut self,
        renderer: &mut Renderer,
        ids: Vec<Option<OwnedSurface>>,
    ) -> Result<(), (SurfaceOwnershipError, Vec<Option<OwnedSurface>>)> {
        if let Err(error) = self.validate_replacement(renderer, ids.iter().flatten()) {
            return Err((error, ids));
        }
        Self::replace_bank(&mut self.dots, renderer, ids);
        Ok(())
    }

    pub fn replace_ground_marks(&mut self, renderer: &mut Renderer, ids: Vec<OwnedSurface>) {
        self.try_replace_ground_marks(renderer, ids)
            .expect("mission ground marks require unique local owned uploads");
    }

    pub fn try_replace_ground_marks(
        &mut self,
        renderer: &mut Renderer,
        ids: Vec<OwnedSurface>,
    ) -> Result<(), (SurfaceOwnershipError, Vec<OwnedSurface>)> {
        if let Err(error) = self.validate_replacement(renderer, &ids) {
            return Err((error, ids));
        }
        Self::replace_bank(&mut self.ground_marks, renderer, ids.into_iter().map(Some));
        Ok(())
    }

    fn validate_replacement<'a>(
        &self,
        renderer: &Renderer,
        ids: impl IntoIterator<Item = &'a OwnedSurface>,
    ) -> Result<(), SurfaceOwnershipError> {
        self.validate_renderer(renderer)?;
        let mut seen = std::collections::HashSet::new();
        for upload in ids {
            renderer.validate_surface_retirement(upload)?;
            if !seen.insert(upload.handle()) {
                return Err(SurfaceOwnershipError::AlreadyOwned(
                    upload.handle().legacy_id(),
                ));
            }
        }
        Ok(())
    }

    fn replace_bank(
        bank: &mut SpriteBank,
        renderer: &mut Renderer,
        ids: impl IntoIterator<Item = Option<OwnedSurface>>,
    ) {
        let ids: Vec<_> = ids.into_iter().collect();
        for id in ids.iter().flatten() {
            renderer
                .validate_surface_retirement(id)
                .expect("mission bank requires local owned uploads");
        }
        let frames = ids
            .into_iter()
            .map(|id| id.map(|id| SpriteSurface::uploaded(renderer, id)))
            .collect();
        bank.retire(&mut |id| delete_surface(renderer, id));
        bank.frames = frames;
    }

    /// Start sprite preparation without retaining stale resources on a missing
    /// optional bank. Does not retire the map, uploaded earlier in level loading.
    pub fn retire_sprites(&mut self, renderer: &mut Renderer) {
        self.validate_renderer(renderer)
            .expect("mission uploads require their originating renderer");
        self.retire_sprite_banks(&mut |id| delete_surface(renderer, id));
    }

    fn retire_sprite_banks(&mut self, delete: &mut impl FnMut(OwnedSurface)) {
        self.corners.retire(delete);
        self.dots.retire(delete);
        self.ground_marks.retire(delete);
        self.corner_size = ScreenSize::default();
    }

    pub fn retire(&mut self, renderer: &mut Renderer) {
        self.try_retire(renderer)
            .expect("mission uploads require their originating renderer");
    }

    pub fn try_retire(&mut self, renderer: &mut Renderer) -> Result<(), MissingSurface> {
        self.validate_renderer(renderer)?;
        self.retire_with(&mut |id| delete_surface(renderer, id));
        Ok(())
    }

    fn validate_renderer(&self, renderer: &Renderer) -> Result<(), MissingSurface> {
        for frame in self
            .map
            .iter()
            .chain(self.corners.frames.iter().flatten())
            .chain(self.dots.frames.iter().flatten())
            .chain(self.ground_marks.frames.iter().flatten())
        {
            renderer.surface_dimensions(frame.upload.handle())?;
        }
        Ok(())
    }

    fn retire_with(&mut self, delete: &mut impl FnMut(OwnedSurface)) {
        if let Some(map) = self.map.take() {
            delete(map.upload);
        }
        self.retire_sprite_banks(delete);
    }
}

#[cfg(test)]
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
        host.frontend.mission_surfaces.replace_map(renderer, owned);
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
        assert_eq!(host.frontend.mission_surfaces.map(), Some(id));
        assert!(renderer.surface_dimensions(id).is_ok());
    }
    let dot = upload(renderer);
    let corner_size = ScreenSize::new(12.0, 15.0);
    host.frontend
        .mission_surfaces
        .replace_corners(renderer, corner_size, vec![]);
    assert_eq!(
        host.frontend.mission_surfaces.corner_size(),
        corner_size,
        "known layout dimensions survive unavailable corner pictures"
    );
    assert!(host.frontend.mission_surfaces.corner(0).is_none());
    assert!(renderer.upload_rgb565(2, 2, &[0xffff]).is_none());
    assert_eq!(
        host.frontend.mission_surfaces.map(),
        previous,
        "a failed upload must leave the installed map intact"
    );
    host.frontend
        .mission_surfaces
        .replace_dots(renderer, vec![None, Some(dot)]);
    assert!(host.frontend.mission_surfaces.dots()[0].is_none());
    let borrowed_dot = host.frontend.mission_surfaces.dots()[1]
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
            0,
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
        .mission_surfaces
        .try_replace_dots(renderer, vec![Some(candidate), Some(decoded)])
        .unwrap_err();
    assert_eq!(
        host.frontend.mission_surfaces.dots()[1]
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
        .mission_surfaces
        .replace_dots(renderer, vec![]);
    assert!(renderer.draw_surface(borrowed_dot, None, None, 0).is_err());
    for reserved in [0, 1] {
        assert!(
            host.frontend
                .mission_surfaces
                .try_replace_dots(renderer, vec![Some(OwnedSurface::synthetic(reserved))])
                .is_err()
        );
    }
    let candidate = upload(renderer);
    let (_, candidates) = host
        .frontend
        .mission_surfaces
        .try_replace_corners(
            renderer,
            ScreenSize::new(99.0, 99.0),
            vec![candidate, OwnedSurface::synthetic(u32::MAX)],
        )
        .unwrap_err();
    assert_eq!(host.frontend.mission_surfaces.corner_size(), corner_size);
    let (_, candidates) = host
        .frontend
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
    host.frontend.mission_surfaces.retire(renderer);
    assert!(renderer.surface_dimensions(previous.unwrap()).is_err());
    host.frontend.mission_surfaces.retire(renderer);
}

#[cfg(test)]
mod tests {
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
}

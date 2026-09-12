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
mod tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use tests::verify_gpu_lifecycle;

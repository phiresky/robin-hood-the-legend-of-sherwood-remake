//! Renderer-local resources retained across snapshot restores, retired with a mission.
//!
//! Handles are deliberately not serialized or cloned with their owner. Replacing a
//! bank retires its old uploads; absent frames remain absent rather than aliasing
//! the renderer's screen surface (legacy ID zero).

use crate::renderer::{Renderer, SurfaceHandle};
use robin_engine::coordinates::ScreenSize;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpriteSurface {
    handle: SurfaceHandle,
    width: u16,
    height: u16,
}

impl SpriteSurface {
    pub fn parts(self) -> (u32, u16, u16) {
        (self.handle.legacy_id(), self.width, self.height)
    }

    fn uploaded(renderer: &Renderer, id: u32) -> Self {
        let handle = SurfaceHandle::from_legacy(id);
        assert_ne!(
            handle.legacy_id(),
            0,
            "mission upload cannot own the screen"
        );
        let (width, height) = renderer
            .surface_dimensions(handle)
            .expect("mission upload must reference a live renderer surface");
        Self {
            handle,
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
    fn retire(&mut self, delete: &mut impl FnMut(u32)) {
        for frame in self.frames.drain(..).flatten() {
            delete(frame.handle.legacy_id());
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

fn delete_surface(renderer: &mut Renderer, id: u32) {
    assert!(
        renderer.delete_surface(id),
        "owned mission surface {id} was already retired"
    );
}

impl MissionRenderResources {
    pub fn map(&self) -> Option<u32> {
        self.map.map(|frame| frame.handle.legacy_id())
    }

    pub fn corner_size(&self) -> ScreenSize {
        self.corner_size
    }

    pub fn corner(&self, index: usize) -> Option<u32> {
        self.corners
            .frames
            .get(index)
            .or(self.corners.frames.first())
            .and_then(|frame| *frame)
            .map(|frame| frame.handle.legacy_id())
    }

    pub fn dots(&self) -> &[Option<SpriteSurface>] {
        &self.dots.frames
    }
    pub fn ground_marks(&self) -> &[Option<SpriteSurface>] {
        &self.ground_marks.frames
    }

    pub fn replace_map(&mut self, renderer: &mut Renderer, id: u32) {
        self.validate_new_ids([id]);
        let frame = SpriteSurface::uploaded(renderer, id);
        if let Some(previous) = self.map.replace(frame) {
            delete_surface(renderer, previous.handle.legacy_id());
        }
    }

    pub fn replace_corners(&mut self, renderer: &mut Renderer, size: ScreenSize, ids: Vec<u32>) {
        self.validate_new_ids(ids.iter().copied());
        Self::replace_bank(&mut self.corners, renderer, ids.into_iter().map(Some));
        self.corner_size = size;
    }

    pub fn replace_dots(&mut self, renderer: &mut Renderer, ids: Vec<Option<u32>>) {
        self.validate_new_ids(ids.iter().flatten().copied());
        Self::replace_bank(&mut self.dots, renderer, ids);
    }

    pub fn replace_ground_marks(&mut self, renderer: &mut Renderer, ids: Vec<u32>) {
        self.validate_new_ids(ids.iter().copied());
        Self::replace_bank(&mut self.ground_marks, renderer, ids.into_iter().map(Some));
    }

    fn validate_new_ids(&self, ids: impl IntoIterator<Item = u32>) {
        let mut owned: std::collections::HashSet<_> = self
            .map
            .iter()
            .chain(self.corners.frames.iter().flatten())
            .chain(self.dots.frames.iter().flatten())
            .chain(self.ground_marks.frames.iter().flatten())
            .map(|frame| frame.handle.legacy_id())
            .collect();
        for id in ids {
            assert!(id > 1, "mission upload cannot own the screen");
            assert!(
                owned.insert(id),
                "mission surface {id} cannot have multiple owners"
            );
        }
    }

    fn replace_bank(
        bank: &mut SpriteBank,
        renderer: &mut Renderer,
        ids: impl IntoIterator<Item = Option<u32>>,
    ) {
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
        self.retire_sprite_banks(&mut |id| delete_surface(renderer, id));
    }

    fn retire_sprite_banks(&mut self, delete: &mut impl FnMut(u32)) {
        self.corners.retire(delete);
        self.dots.retire(delete);
        self.ground_marks.retire(delete);
        self.corner_size = ScreenSize::default();
    }

    pub fn retire(&mut self, renderer: &mut Renderer) {
        self.retire_with(&mut |id| delete_surface(renderer, id));
    }

    fn retire_with(&mut self, delete: &mut impl FnMut(u32)) {
        if let Some(map) = self.map.take() {
            delete(map.handle.legacy_id());
        }
        self.retire_sprite_banks(delete);
    }
}

#[cfg(test)]
pub(crate) fn verify_gpu_lifecycle(renderer: &mut Renderer) {
    let mut host = crate::host::Host::scratch(3.0, 2.0);
    let upload = |renderer: &mut Renderer| {
        renderer
            .create_surface_from_rgb565(1, 1, &[0xffff])
            .unwrap()
    };
    let mut previous = None;
    for _ in 0..3 {
        let id = upload(renderer);
        if let Some(old) = previous {
            assert!(renderer.blit_to_screen(old, None, None, 0));
        }
        host.frontend.mission_surfaces.replace_map(renderer, id);
        if let Some(old) = previous {
            assert!(
                renderer
                    .surface_dimensions(SurfaceHandle::from_legacy(old))
                    .is_err()
            );
            assert_eq!(
                &renderer.try_capture_frame_rgba().unwrap().2[..4],
                &[248, 252, 248, 255],
                "retiring a managed ID must preserve already queued draw bindings"
            );
        }
        previous = Some(id);
        host.post_load_reset();
        assert_eq!(host.frontend.mission_surfaces.map(), Some(id));
        assert!(
            renderer
                .surface_dimensions(SurfaceHandle::from_legacy(id))
                .is_ok()
        );
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
    assert!(
        renderer
            .create_surface_from_rgb565(2, 2, &[0xffff])
            .is_none()
    );
    assert_eq!(
        host.frontend.mission_surfaces.map(),
        previous,
        "a failed upload must leave the installed map intact"
    );
    host.frontend
        .mission_surfaces
        .replace_dots(renderer, vec![None, Some(dot)]);
    assert!(host.frontend.mission_surfaces.dots()[0].is_none());
    host.frontend
        .mission_surfaces
        .replace_dots(renderer, vec![]);
    assert!(
        renderer
            .surface_dimensions(SurfaceHandle::from_legacy(dot))
            .is_err()
    );
    host.frontend.mission_surfaces.retire(renderer);
    assert!(
        renderer
            .surface_dimensions(SurfaceHandle::from_legacy(previous.unwrap()))
            .is_err()
    );
    host.frontend.mission_surfaces.retire(renderer);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic IDs exist only in this pure ownership test; production ownership
    // always validates live renderer uploads before accepting them.
    fn frame(id: u32) -> SpriteSurface {
        SpriteSurface {
            handle: SurfaceHandle::from_legacy(id),
            width: 3,
            height: 4,
        }
    }

    #[test]
    fn sparse_frames_keep_indices_and_retirement_is_idempotent() {
        let mut owner = MissionRenderResources::default();
        owner.map = Some(frame(2));
        owner.dots.frames = vec![Some(frame(3)), None, Some(frame(4))];
        assert!(owner.dots()[1].is_none());
        assert_eq!(owner.dots()[2].unwrap().parts(), (4, 3, 4));
        let mut deleted = Vec::new();
        owner.retire_with(&mut |id| deleted.push(id));
        owner.retire_with(&mut |id| deleted.push(id));
        assert_eq!(deleted, [2, 3, 4]);
        assert!(owner.map().is_none());
    }

    #[test]
    fn sprite_preparation_retires_partial_banks_but_keeps_map() {
        let mut owner = MissionRenderResources::default();
        owner.map = Some(frame(2));
        owner.corners.frames = vec![Some(frame(3))];
        owner.corner_size = ScreenSize::new(3.0, 4.0);
        let mut deleted = Vec::new();
        owner.retire_sprite_banks(&mut |id| deleted.push(id));
        assert_eq!(deleted, [3]);
        assert_eq!(owner.map(), Some(2));
        assert_eq!(owner.corner_size(), ScreenSize::default());
        assert!(owner.corner(0).is_none());
    }

    #[test]
    fn diagnostics_cannot_resurrect_gpu_ownership() {
        let mut owner = MissionRenderResources::default();
        owner.map = Some(frame(2));
        let restored: MissionRenderResources =
            serde_json::from_str(&serde_json::to_string(&owner).unwrap()).unwrap();
        assert!(restored.map().is_none());
        assert_eq!(owner.map(), Some(2));
    }

    #[test]
    #[should_panic(expected = "cannot have multiple owners")]
    fn replacement_rejects_aliasing_existing_owner() {
        let mut owner = MissionRenderResources::default();
        owner.map = Some(frame(2));
        owner.validate_new_ids([2]);
    }

    #[test]
    #[should_panic(expected = "cannot have multiple owners")]
    fn bank_rejects_duplicate_ownership() {
        MissionRenderResources::default().validate_new_ids([2, 2]);
    }

    #[test]
    #[should_panic(expected = "cannot own the screen")]
    fn screen_is_never_an_absent_frame_placeholder() {
        MissionRenderResources::default().validate_new_ids([0]);
    }

    #[test]
    #[should_panic(expected = "cannot own the screen")]
    fn legacy_screen_alias_is_not_an_owned_upload() {
        MissionRenderResources::default().validate_new_ids([1]);
    }
}

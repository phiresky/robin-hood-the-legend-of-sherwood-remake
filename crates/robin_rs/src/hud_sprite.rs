//! Shared HUD sprite geometry, four-state fallback, and surface ownership.

use crate::ingame_menu::layout::{
    BTN_STATE_DISABLED, BTN_STATE_HOVER, BTN_STATE_NORMAL, BTN_STATE_PRESSED,
};
use crate::renderer::{OwnedSurface, Renderer, SurfaceHandle};

/// Preserve screen-space edge arithmetic before converting to sprite coordinates.
pub(crate) fn screen_rect_to_sprite_bbox(
    rect: crate::gfx_types::Rect,
) -> robin_engine::sprite::BBox {
    robin_engine::sprite::BBox::from_coords(
        rect.x() as f32,
        rect.y() as f32,
        (rect.x() + rect.width() as i32) as f32,
        (rect.y() + rect.height() as i32) as f32,
    )
}

pub(crate) type SpriteFrame = (OwnedSurface, u16, u16);
pub(crate) type SpriteBank = [Option<SpriteFrame>; 4];

pub(crate) fn frame(bank: &SpriteBank, state: usize) -> Option<(SurfaceHandle, u16, u16)> {
    bank[state]
        .as_ref()
        .or(bank[BTN_STATE_NORMAL].as_ref())
        .map(|(surface, width, height)| (surface.handle(), *width, *height))
}

pub(crate) fn size(bank: &SpriteBank) -> Option<(u16, u16)> {
    [
        BTN_STATE_NORMAL,
        BTN_STATE_HOVER,
        BTN_STATE_PRESSED,
        BTN_STATE_DISABLED,
    ]
    .into_iter()
    .find_map(|state| bank[state].as_ref())
    .map(|(_, width, height)| (*width, *height))
}

pub(crate) fn retire<const N: usize>(renderer: &mut Renderer, banks: [&mut SpriteBank; N]) {
    // Validate the entire owner before consuming any upload tokens.
    for bank in &banks {
        for (upload, _, _) in bank.iter().flatten() {
            renderer
                .validate_surface_retirement(upload)
                .expect("HUD bank belongs to its renderer");
        }
    }
    for bank in banks {
        for frame in bank {
            if let Some((upload, _, _)) = frame.take() {
                renderer.retire_surface(upload);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_rect_edges_are_added_before_float_conversion() {
        for (x, y, w, h) in [
            (0, 0, 24, 24),
            (-20, -30, 12, 8),
            (10, 20, 0, 0),
            (16_777_217, -16_777_217, 1, 3),
        ] {
            let rect = crate::gfx_types::Rect::new(x, y, w, h);
            let bbox = screen_rect_to_sprite_bbox(rect);
            assert_eq!((bbox.min.x, bbox.min.y), (x as f32, y as f32));
            assert_eq!(
                (bbox.max.x, bbox.max.y),
                ((x + w as i32) as f32, (y + h as i32) as f32)
            );
        }
        let bbox = screen_rect_to_sprite_bbox(crate::gfx_types::Rect::new(16_777_217, 0, 1, 1));
        assert_eq!(bbox.max.x, 16_777_218.0);
    }

    #[test]
    fn frame_and_size_fallbacks_cover_every_bank_presence_mask() {
        for mask in 0..16 {
            let bank: SpriteBank = std::array::from_fn(|state| {
                (mask & (1 << state) != 0).then(|| {
                    (
                        OwnedSurface::synthetic(state as u32 + 2),
                        state as u16 + 10,
                        20,
                    )
                })
            });
            let expected_size = [
                BTN_STATE_NORMAL,
                BTN_STATE_HOVER,
                BTN_STATE_PRESSED,
                BTN_STATE_DISABLED,
            ]
            .into_iter()
            .find(|&state| mask & (1 << state) != 0)
            .map(|state| (state as u16 + 10, 20));
            assert_eq!(size(&bank), expected_size);
            for state in 0..4 {
                let expected = if mask & (1 << state) != 0 {
                    Some(state)
                } else if mask & (1 << BTN_STATE_NORMAL) != 0 {
                    Some(BTN_STATE_NORMAL)
                } else {
                    None
                };
                assert_eq!(
                    frame(&bank, state).map(|(_, width, _)| width),
                    expected.map(|index| index as u16 + 10)
                );
            }
        }
    }
}

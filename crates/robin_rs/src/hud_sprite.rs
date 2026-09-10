//! Shared four-state HUD sprite fallback and surface ownership.

use crate::ingame_menu::layout::{
    BTN_STATE_DISABLED, BTN_STATE_HOVER, BTN_STATE_NORMAL, BTN_STATE_PRESSED,
};
use crate::renderer::{OwnedSurface, Renderer, SurfaceHandle};

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

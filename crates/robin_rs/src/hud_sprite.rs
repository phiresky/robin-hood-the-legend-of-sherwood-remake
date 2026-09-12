//! Shared HUD sprite geometry, four-state fallback, and surface ownership.

use crate::ingame_menu::layout::{
    BTN_STATE_DISABLED, BTN_STATE_HOVER, BTN_STATE_NORMAL, BTN_STATE_PRESSED,
};
use crate::renderer::{OwnedSurface, Renderer, SurfaceHandle};

/// The authored resources and drawing policy of one small HUD button family.
pub trait HudButton<const N: usize>: Copy + Eq + 'static {
    const ALL: [Self; N];
    fn index(self) -> usize;
    fn resource(self) -> (i32, &'static str);
    fn frame_state(self, state: usize, _frame_counter: u32) -> usize {
        state
    }
    fn shadow(self) -> bool {
        false
    }
}

/// One owner for all button banks; GPU handles never survive serialization.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound = "")]
pub struct ButtonSprites<B, const N: usize> {
    #[serde(skip, default = "empty_banks")]
    banks: [SpriteBank; N],
    #[serde(skip)]
    button: std::marker::PhantomData<B>,
}

fn empty_banks<const N: usize>() -> [SpriteBank; N] {
    std::array::from_fn(|_| std::array::from_fn(|_| None))
}

impl<B, const N: usize> Default for ButtonSprites<B, N> {
    fn default() -> Self {
        Self {
            banks: empty_banks(),
            button: std::marker::PhantomData,
        }
    }
}

impl<B: HudButton<N>, const N: usize> ButtonSprites<B, N> {
    pub fn load(
        resources: &mut robin_assets::resource_manager::ResourceManager,
        renderer: &mut Renderer,
    ) -> Self {
        Self {
            banks: std::array::from_fn(|index| {
                let button = B::ALL[index];
                assert_eq!(
                    button.index(),
                    index,
                    "HUD button resource order must match its index"
                );
                let (id, label) = button.resource();
                load_bank(resources, renderer, id, label)
            }),
            button: std::marker::PhantomData,
        }
    }

    pub(crate) fn retire(&mut self, renderer: &mut Renderer) {
        retire(renderer, self.banks.each_mut());
    }

    pub fn size(&self, button: B) -> Option<(u16, u16)> {
        size(&self.banks[button.index()])
    }

    fn frame(
        &self,
        button: B,
        state: usize,
        frame_counter: u32,
    ) -> Option<(SurfaceHandle, u16, u16)> {
        frame(
            &self.banks[button.index()],
            button.frame_state(state, frame_counter),
        )
    }

    pub(crate) fn draw(
        &self,
        renderer: &mut Renderer,
        button: B,
        rect: crate::gfx_types::Rect,
        state: usize,
        frame_counter: u32,
    ) {
        let Some((surface, _, _)) = self.frame(button, state, frame_counter) else {
            return;
        };
        let destination = screen_rect_to_sprite_bbox(rect);
        if button.shadow() {
            renderer
                .draw_surface_with_shadow(
                    surface,
                    None,
                    Some(&destination),
                    50,
                    crate::renderer::BLIT_SOURCE_TRANSPARENT,
                )
                .expect("live HUD upload");
        } else {
            renderer
                .draw_surface(
                    surface,
                    None,
                    Some(&destination),
                    crate::renderer::BLIT_SOURCE_TRANSPARENT,
                )
                .expect("live HUD upload");
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct HoverState<B> {
    pub hovered: Option<B>,
    pub mouse_pressed: bool,
}

impl<B> Default for HoverState<B> {
    fn default() -> Self {
        Self {
            hovered: None,
            mouse_pressed: false,
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(bound = "")]
pub struct ButtonTooltipTracker<B> {
    #[serde(skip)]
    inner: crate::ui_panel::HoverTooltipTracker<B>,
}

impl<B> Default for ButtonTooltipTracker<B> {
    fn default() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl<B: Copy + PartialEq> ButtonTooltipTracker<B> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn update(&mut self, hovered: Option<B>) {
        self.inner.update(hovered);
    }
    pub fn ready_button(&self) -> Option<B> {
        self.inner.ready_slot()
    }
}

/// Shared font and cursor-relative placement for all button-family tooltips.
#[derive(serde::Serialize)]
pub struct TooltipPlacement<'a> {
    #[serde(skip)]
    pub font: &'a crate::native_font::Font,
    #[serde(skip)]
    pub shadow: Option<&'a crate::native_font::Font>,
    pub mouse: (i32, i32),
    pub cursor_size: (i32, i32),
}

impl<'de, 'a> serde::Deserialize<'de> for TooltipPlacement<'a> {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "tooltip placement requires live font ownership",
        ))
    }
}

pub fn draw_tooltip<B, T: Into<Option<String>>>(
    renderer: &mut Renderer,
    ready_button: Option<B>,
    tooltip_text: impl Fn(B) -> T,
    placement: TooltipPlacement<'_>,
) {
    if let Some(button) = ready_button
        && let Some(text) = tooltip_text(button).into()
        && !text.is_empty()
    {
        crate::ui_panel::draw_screen_tooltip(
            renderer,
            placement.font,
            placement.shadow,
            &text,
            placement.mouse.0,
            placement.mouse.1,
            placement.cursor_size,
        );
    }
}

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

/// Load the four optional interaction frames in resource sub-id order.
pub(crate) fn load_bank(
    resources: &mut robin_assets::resource_manager::ResourceManager,
    renderer: &mut Renderer,
    resource_id: i32,
    label: &str,
) -> SpriteBank {
    std::array::from_fn(|sub| match resources.get_picture(resource_id, sub) {
        Ok(picture) => {
            let width = picture.width;
            let height = picture.height;
            let surface = crate::ui_panel::pic_to_surface(renderer, picture);
            tracing::info!(
                label,
                resource_id,
                sub,
                width,
                height,
                ?surface,
                "Loaded HUD sprite frame"
            );
            Some((surface, width, height))
        }
        Err(error) => {
            tracing::debug!(
                label, resource_id, sub, %error,
                "Optional HUD sprite frame unavailable"
            );
            None
        }
    })
}

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
pub(crate) mod tests {
    use super::*;

    fn sparse_bank_contract<B: HudButton<N>, const N: usize>() {
        let mut sprites = ButtonSprites::<B, N>::default();
        let button = B::ALL[0];
        sprites.banks[button.index()][BTN_STATE_NORMAL] = Some((OwnedSurface::synthetic(42), 7, 9));
        sprites.banks[button.index()][BTN_STATE_PRESSED] =
            Some((OwnedSurface::synthetic(43), 8, 10));
        assert_eq!(sprites.frame(button, BTN_STATE_HOVER, 0).unwrap().1, 7);
        assert_eq!(sprites.frame(button, BTN_STATE_PRESSED, 0).unwrap().1, 8);
        let restored: ButtonSprites<B, N> =
            serde_json::from_value(serde_json::to_value(&sprites).unwrap()).unwrap();
        assert!(restored.frame(button, BTN_STATE_NORMAL, 0).is_none());
        sprites.banks[button.index()][BTN_STATE_NORMAL] = None;
        assert!(sprites.frame(button, BTN_STATE_HOVER, 0).is_none());
        assert_eq!(sprites.frame(button, BTN_STATE_PRESSED, 0).unwrap().1, 8);
        for (index, button) in B::ALL.into_iter().enumerate() {
            assert_eq!(button.index(), index);
        }
    }

    #[test]
    fn sparse_owned_frames_keep_fallback_and_diagnostics_are_inert() {
        sparse_bank_contract::<crate::zoom_hud::ZoomButton, 2>();
        sparse_bank_contract::<crate::stature_hud::StatureButton, 2>();
        sparse_bank_contract::<crate::sherwood_hud::SherwoodButton, 5>();
    }

    #[test]
    fn sherwood_blink_and_shadow_policy_remain_button_specific() {
        use crate::sherwood_hud::SherwoodButton as Button;
        for frame in [0, 24, 25, 49, 50] {
            let expected = if (frame / 25) & 1 == 1 {
                BTN_STATE_HOVER
            } else {
                BTN_STATE_NORMAL
            };
            assert_eq!(
                Button::DisplayCampaignMap.frame_state(BTN_STATE_NORMAL, frame),
                expected
            );
            assert_eq!(
                Button::GoToExit.frame_state(BTN_STATE_NORMAL, frame),
                BTN_STATE_NORMAL
            );
            assert_eq!(
                Button::DisplayCampaignMap.frame_state(BTN_STATE_PRESSED, frame),
                BTN_STATE_PRESSED
            );
        }
        assert!(!Button::DisplayCampaignMap.shadow());
        assert!(!Button::GoToExit.shadow());
        assert!(Button::StartMission.shadow());
        assert!(Button::QuitMission.shadow());
        assert!(Button::SherwoodTrading.shadow());
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn verify_gpu_ownership(renderer: &mut Renderer) {
        fn verify<B: HudButton<N>, const N: usize>(renderer: &mut Renderer) {
            let mut sprites = ButtonSprites::<B, N>::default();
            let button = B::ALL[0];
            let upload = renderer.upload_rgb565(1, 1, &[0xffff]).unwrap();
            let handle = upload.handle();
            sprites.banks[button.index()][BTN_STATE_NORMAL] = Some((upload, 1, 1));
            assert_eq!(sprites.frame(button, BTN_STATE_HOVER, 0).unwrap().0, handle);
            renderer.draw_surface(handle, None, None, 0).unwrap();
            sprites.retire(renderer);
            sprites.retire(renderer);
            assert!(sprites.frame(button, BTN_STATE_NORMAL, 0).is_none());
            assert!(renderer.surface_dimensions(handle).is_err());
            assert_eq!(
                &renderer.try_capture_frame_rgba().unwrap().2[..4],
                &[248, 252, 248, 255]
            );
        }
        verify::<crate::zoom_hud::ZoomButton, 2>(renderer);
        verify::<crate::stature_hud::StatureButton, 2>(renderer);
        verify::<crate::sherwood_hud::SherwoodButton, 5>(renderer);
    }

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

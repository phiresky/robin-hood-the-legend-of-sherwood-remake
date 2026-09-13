//! GPU contract helper shared with `renderer/gpu_contract_tests.rs`, which
//! owns the one real renderer these checks run against.

use super::*;

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

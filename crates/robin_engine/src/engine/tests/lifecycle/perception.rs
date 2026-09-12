use super::*;

#[test]
fn camera_clip_view() {
    let mut camera = CameraState {
        level_size: MapSize::new(2000.0, 1500.0),
        zoom_factor: 1.0,
        view_position: crate::coordinates::MapPoint::new(-100.0, -50.0),
        ..Default::default()
    };
    let clipped = camera.clip_view();
    assert!(clipped);
    assert_eq!(camera.view_position.x, 0.0);
    assert_eq!(camera.view_position.y, 0.0);
}

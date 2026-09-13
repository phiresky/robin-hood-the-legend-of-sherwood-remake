use super::PresentationRect;

fn assert_rect_close(actual: PresentationRect, expected: PresentationRect) {
    for (actual, expected) in [
        (actual.x, expected.x),
        (actual.y, expected.y),
        (actual.width, expected.width),
        (actual.height, expected.height),
    ] {
        assert!(
            (actual - expected).abs() < 0.01,
            "presentation coordinate {actual} differs from {expected}"
        );
    }
}

#[test]
fn aspect_fit_letterboxes_wider_surfaces() {
    let rect = PresentationRect::aspect_fit(1280, 720, 3440, 1440);
    assert_rect_close(
        rect,
        PresentationRect {
            x: 440.0,
            y: 0.0,
            width: 2560.0,
            height: 1440.0,
        },
    );
}

#[test]
fn aspect_fit_letterboxes_taller_surfaces() {
    let rect = PresentationRect::aspect_fit(1024, 768, 1920, 1080);
    assert_rect_close(
        rect,
        PresentationRect {
            x: 240.0,
            y: 0.0,
            width: 1440.0,
            height: 1080.0,
        },
    );
}

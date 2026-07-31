//! Strict Linux-v48 adoption of the Original camera preamble.
//!
//! `RHEngine::Serialize` writes the camera and raw
//! `RHbackgroundTransform` near the start of the engine body. On read it then
//! cancels all zoom-transition flags, discards the serialized surface handles,
//! invalidates the background, and re-clamps the view and any active slide
//! target. This module reproduces those read-side rules while keeping the
//! deterministic director camera separate from the host display mirror.

use thiserror::Error;

use crate::{
    coordinates::{MapPoint, MapVec},
    engine::{BackgroundTransform, DisplayOpCode, EngineInner, peripherals::HostDisplayState},
};

use super::{
    LegacySaveAbiProfile,
    engine::{LegacyBackgroundTransform, LegacyEnginePreamble, LegacyPoint2},
};

const MAX_SCROLL_LEVEL: u16 = 31;
const MAX_ZOOM_LEVEL: u16 = 2;
const DIRECTOR_WIDTH: f32 = 1024.0;
const DIRECTOR_HEIGHT: f32 = 768.0;

/// Host-side camera state that must be installed at the same load boundary as
/// [`LegacyCameraAdoptionPlan`].
///
/// The minimap and other host UI fields are deliberately absent: they have
/// independent serialized owners and must not be reset as a side effect of
/// restoring the background cache state.
#[derive(Clone, Debug)]
pub struct LegacyCameraHostState {
    pub background_transform: BackgroundTransform,
    pub display_op: DisplayOpCode,
    pub frame_scrolled: [bool; 4],
}

impl LegacyCameraHostState {
    pub fn apply_to(self, display: &mut HostDisplayState) {
        display.background_transform = self.background_transform;
        display.display_op = self.display_op;
        display.frame_scrolled = self.frame_scrolled;
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LegacyCameraAdoptionPlan {
    view: MapPoint,
    zoom_factor: f32,
    camera_slide: MapPoint,
    camera_wanted: MapPoint,
    fixed_camera_speed: u16,
    desired_zoom_factor: f32,
    old_zoom_factor: f32,
    background_transform: BackgroundTransform,
    locker: bool,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub(crate) enum LegacyCameraAdoptionError {
    #[error("saved camera field {field} contains non-finite value {value}")]
    NonFinite { field: String, value: f32 },
    #[error("saved camera zoom_factor must be positive, got {value}")]
    NonPositiveZoom { value: f32 },
    #[error(
        "saved background current_x_scrolling_level {value} exceeds the Original 32-entry table"
    )]
    XScrollLevelOutOfRange { value: u16 },
    #[error(
        "saved background current_y_scrolling_level {value} exceeds the Original 32-entry table"
    )]
    YScrollLevelOutOfRange { value: u16 },
    #[error("saved background current_zoom_level {value} exceeds the Original three-level table")]
    ZoomLevelOutOfRange { value: u16 },
}

impl LegacyCameraAdoptionPlan {
    pub(crate) fn preflight(
        engine: &EngineInner,
        _abi: LegacySaveAbiProfile,
        saved: &LegacyEnginePreamble,
    ) -> Result<Self, LegacyCameraAdoptionError> {
        validate_point("view", saved.view)?;
        validate_finite("zoom_factor", saved.zoom_factor)?;
        if saved.zoom_factor <= 0.0 {
            return Err(LegacyCameraAdoptionError::NonPositiveZoom {
                value: saved.zoom_factor,
            });
        }
        validate_point("camera_slide", saved.camera_slide)?;
        validate_finite("desired_zoom_factor", saved.desired_zoom_factor)?;
        validate_finite("old_zoom_factor", saved.old_zoom_factor)?;
        validate_point("camera_wanted", saved.camera_wanted)?;
        validate_background(&saved.background_transform)?;

        // Original read-side normalization calls
        //
        //   CheckLocationIsValidForCamera(
        //       serialized_view + SCREENSIZE * 0.5 / zoom)
        //
        // and re-derives an active slide from camera_wanted. Use the runtime's
        // audited implementation on a detached clone so truncation, clipping,
        // 0.5x even-coordinate handling, and the small-level 1x fallback stay
        // exactly shared with normal camera commands.
        let (view, camera_slide, zoom_factor) = normalize_camera(
            engine,
            saved.view,
            saved.zoom_factor,
            saved.camera_slide,
            saved.camera_wanted,
        );
        let background_transform =
            convert_background(&saved.background_transform, view, zoom_factor);

        Ok(Self {
            view,
            zoom_factor,
            camera_slide,
            camera_wanted: point(saved.camera_wanted),
            fixed_camera_speed: saved.fixed_camera_speed,
            desired_zoom_factor: saved.desired_zoom_factor,
            old_zoom_factor: saved.old_zoom_factor,
            background_transform,
            locker: saved.locker,
        })
    }

    /// Apply deterministic camera owners and return the matching host display
    /// state without touching unrelated host UI.
    pub(crate) fn apply(self, engine: &mut EngineInner) -> LegacyCameraHostState {
        let camera = &mut engine.feedback.cutscene_camera;
        camera.view_position = self.view;
        camera.old_view_position = self.view;
        camera.zoom_factor = self.zoom_factor;
        camera.old_zoom_factor = self.old_zoom_factor;
        camera.camera_slide = self.camera_slide;
        camera.camera_wanted = self.camera_wanted;
        camera.fixed_camera_speed = self.fixed_camera_speed;
        camera.desired_zoom_factor = self.desired_zoom_factor;

        // None of this transient state is serialized. The Original load path
        // invalidates its cache and starts outside an active zoom operation.
        camera.zoom_init_done = false;
        camera.mechanized_zoom = false;
        camera.displacement = MapVec::ZERO;
        camera.displacement_counter = 0;
        camera.pending_zoom_mouse_screen = None;
        camera.display.background_transform = self.background_transform.clone();
        camera.display.display_op = DisplayOpCode::Redraw;
        camera.display.frame_scrolled = [false; 4];

        engine
            .players
            .seats
            .first_mut()
            .expect("initialized v48 mission has no host player seat")
            .locker_active = self.locker;
        engine.feedback.pending_side_effects.invalidate_background = true;

        LegacyCameraHostState {
            background_transform: self.background_transform,
            display_op: DisplayOpCode::Redraw,
            frame_scrolled: [false; 4],
        }
    }
}

fn normalize_camera(
    engine: &EngineInner,
    saved_view: LegacyPoint2,
    saved_zoom: f32,
    saved_slide: LegacyPoint2,
    saved_wanted: LegacyPoint2,
) -> (MapPoint, MapPoint, f32) {
    let mut probe = engine.clone();
    probe.feedback.cutscene_camera.zoom_factor = saved_zoom;
    let center = MapPoint::new(
        saved_view.x + DIRECTOR_WIDTH * 0.5 / saved_zoom,
        saved_view.y + DIRECTOR_HEIGHT * 0.5 / saved_zoom,
    );
    let view = probe.check_location_is_valid_for_camera(center);
    let camera_slide = if saved_slide.x != -1.0 {
        probe.check_location_is_valid_for_camera(point(saved_wanted))
    } else {
        point(saved_slide)
    };
    (
        view,
        camera_slide,
        probe.feedback.cutscene_camera.zoom_factor,
    )
}

fn convert_background(
    saved: &LegacyBackgroundTransform,
    view: MapPoint,
    zoom: f32,
) -> BackgroundTransform {
    BackgroundTransform {
        scroll_to_left: saved.scroll_to_left,
        scroll_to_up: saved.scroll_to_up,
        current_x_scrolling_level: saved.current_x_scrolling_level,
        current_y_scrolling_level: saved.current_y_scrolling_level,
        // `RHEngine::Serialize` forcibly clears these immediately after
        // reading the raw struct.
        zoom_to_up: false,
        zoom_to_down: false,
        required_zoom_up: false,
        required_zoom_down: false,
        zoom_count: saved.zoom_count,
        number_of_zoom_steps: saved.number_of_zoom_steps,
        x_scrolling_values: saved.x_scrolling_values,
        y_scrolling_values: saved.y_scrolling_values,
        current_zoom_level: saved.current_zoom_level,
        zoom_values: saved.zoom_values,
        center_zoom: vector(saved.center_zoom),
        clipped_zoom: vector(saved.clipped_zoom),
        scrolling_vector: vector(saved.scrolling),
        // Rust keeps explicit interpolation endpoints which the Original raw
        // struct does not. With read-side zoom cancellation they are neutral.
        zoom_from: zoom,
        zoom_to: zoom,
        view_from: view,
        view_to: view,
    }
}

fn validate_background(saved: &LegacyBackgroundTransform) -> Result<(), LegacyCameraAdoptionError> {
    if saved.current_x_scrolling_level > MAX_SCROLL_LEVEL {
        return Err(LegacyCameraAdoptionError::XScrollLevelOutOfRange {
            value: saved.current_x_scrolling_level,
        });
    }
    if saved.current_y_scrolling_level > MAX_SCROLL_LEVEL {
        return Err(LegacyCameraAdoptionError::YScrollLevelOutOfRange {
            value: saved.current_y_scrolling_level,
        });
    }
    if saved.current_zoom_level > MAX_ZOOM_LEVEL {
        return Err(LegacyCameraAdoptionError::ZoomLevelOutOfRange {
            value: saved.current_zoom_level,
        });
    }
    for (index, value) in saved.x_scrolling_values.iter().copied().enumerate() {
        validate_finite(format!("background.x_scrolling_values[{index}]"), value)?;
    }
    for (index, value) in saved.y_scrolling_values.iter().copied().enumerate() {
        validate_finite(format!("background.y_scrolling_values[{index}]"), value)?;
    }
    for (index, value) in saved.zoom_values.iter().copied().enumerate() {
        validate_finite(format!("background.zoom_values[{index}]"), value)?;
    }
    validate_point("background.center_zoom", saved.center_zoom)?;
    validate_point("background.clipped_zoom", saved.clipped_zoom)?;
    validate_point("background.scrolling", saved.scrolling)?;
    Ok(())
}

fn validate_point(
    field: impl Into<String>,
    value: LegacyPoint2,
) -> Result<(), LegacyCameraAdoptionError> {
    let field = field.into();
    validate_finite(format!("{field}.x"), value.x)?;
    validate_finite(format!("{field}.y"), value.y)
}

fn validate_finite(field: impl Into<String>, value: f32) -> Result<(), LegacyCameraAdoptionError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(LegacyCameraAdoptionError::NonFinite {
            field: field.into(),
            value,
        })
    }
}

fn point(value: LegacyPoint2) -> MapPoint {
    MapPoint::new(value.x, value.y)
}

fn vector(value: LegacyPoint2) -> MapVec {
    MapVec::new(value.x, value.y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coordinates::MapSize,
        engine::{EngineInner, peripherals::HostDisplayState},
    };

    fn background() -> LegacyBackgroundTransform {
        LegacyBackgroundTransform {
            scroll_to_left: true,
            scroll_to_up: false,
            current_x_scrolling_level: 7,
            current_y_scrolling_level: 8,
            padding_before_surface_ids: [0xaa, 0xbb],
            surface_id: 0x1234,
            final_surface_id: 0x5678,
            zoom_to_up: true,
            zoom_to_down: false,
            required_zoom_up: false,
            required_zoom_down: true,
            zoom_count: 3,
            number_of_zoom_steps: 8,
            x_scrolling_values: [2.0; 32],
            y_scrolling_values: [4.0; 32],
            current_zoom_level: 1,
            padding_before_zoom_values: [0xcc, 0xdd],
            zoom_values: [0.5, 1.0, 2.0],
            center_zoom: LegacyPoint2 { x: 10.0, y: 20.0 },
            clipped_zoom: LegacyPoint2 { x: 30.0, y: 40.0 },
            scrolling: LegacyPoint2 { x: 5.0, y: -6.0 },
        }
    }

    #[test]
    fn reproduces_original_read_side_camera_normalization() {
        let mut engine = EngineInner::new();
        engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);

        let (view, slide, zoom) = normalize_camera(
            &engine,
            LegacyPoint2 {
                x: 123.75,
                y: 245.75,
            },
            1.0,
            LegacyPoint2 { x: 1.0, y: 1.0 },
            LegacyPoint2 {
                x: 1000.0,
                y: 900.0,
            },
        );

        // CheckLocationIsValidForCamera truncates the reconstructed top-left.
        assert_eq!(view, MapPoint::new(123.0, 245.0));
        // Active slide coordinates are discarded and re-derived from wanted.
        assert_eq!(slide, MapPoint::new(488.0, 516.0));
        assert_eq!(zoom, 1.0);
    }

    #[test]
    fn apply_cancels_zoom_surfaces_and_keeps_host_ui_out_of_the_plan() {
        let mut engine = EngineInner::new();
        let view = MapPoint::new(100.0, 200.0);
        let converted = convert_background(&background(), view, 1.0);
        assert!(!converted.zoom_to_up);
        assert!(!converted.zoom_to_down);
        assert!(!converted.required_zoom_up);
        assert!(!converted.required_zoom_down);
        assert_eq!(converted.zoom_from, 1.0);
        assert_eq!(converted.view_from, view);

        let plan = LegacyCameraAdoptionPlan {
            view,
            zoom_factor: 1.0,
            camera_slide: MapPoint::new(-1.0, -1.0),
            camera_wanted: MapPoint::new(900.0, 800.0),
            fixed_camera_speed: 12,
            desired_zoom_factor: 2.0,
            old_zoom_factor: 0.5,
            background_transform: converted,
            locker: true,
        };
        let host = plan.apply(&mut engine);

        assert_eq!(engine.feedback.cutscene_camera.view_position, view);
        assert_eq!(engine.feedback.cutscene_camera.fixed_camera_speed, 12);
        assert_eq!(engine.feedback.cutscene_camera.desired_zoom_factor, 2.0);
        assert!(engine.players.seats[0].locker_active);
        assert_eq!(
            engine.feedback.cutscene_camera.display.display_op,
            DisplayOpCode::Redraw
        );
        assert!(engine.feedback.pending_side_effects.invalidate_background);

        let mut display = HostDisplayState::default();
        display.minimap.map_displayed = true;
        host.apply_to(&mut display);
        assert_eq!(display.background_transform.current_x_scrolling_level, 7);
        assert_eq!(display.display_op, DisplayOpCode::Redraw);
        assert!(display.minimap.map_displayed);
    }

    #[test]
    fn rejects_unrepresentable_background_indices_and_non_finite_values() {
        let mut saved = background();
        saved.current_x_scrolling_level = 32;
        assert_eq!(
            validate_background(&saved),
            Err(LegacyCameraAdoptionError::XScrollLevelOutOfRange { value: 32 })
        );

        saved.current_x_scrolling_level = 0;
        saved.scrolling.y = f32::NAN;
        assert!(matches!(
            validate_background(&saved),
            Err(LegacyCameraAdoptionError::NonFinite { field, .. })
                if field == "background.scrolling.y"
        ));
    }
}

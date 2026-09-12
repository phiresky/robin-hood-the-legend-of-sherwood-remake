//! Focused engine camera state ownership and behavior.
use super::*;

// ─── Background transform ────────────────────────────────────────────

/// All state related to background scrolling and zoom transitions.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct BackgroundTransform {
    // Scrolling state
    pub scroll_to_left: bool,
    pub scroll_to_up: bool,
    pub current_x_scrolling_level: u16,
    pub current_y_scrolling_level: u16,

    // Zoom state
    pub zoom_to_up: bool,
    pub zoom_to_down: bool,
    pub required_zoom_up: bool,
    pub required_zoom_down: bool,
    pub zoom_count: u16,
    pub number_of_zoom_steps: u16,

    /// Pre-computed scrolling speed tables (32 entries each).
    pub x_scrolling_values: [f32; SCROLLING_TABLE_SIZE],
    pub y_scrolling_values: [f32; SCROLLING_TABLE_SIZE],

    /// Current zoom level index (0 = half, 1 = normal, 2 = double).
    pub current_zoom_level: u16,
    /// The three zoom factors.
    pub zoom_values: [f32; ZOOM_LEVEL_COUNT],

    /// Center of the current zoom operation.
    pub center_zoom: MapVec,
    /// Clipped zoom offset.
    pub clipped_zoom: MapVec,
    /// Current scrolling vector for this frame.
    pub scrolling_vector: MapVec,

    /// Source zoom factor at the start of the active zoom transition.
    /// Valid only while `zoom_to_up` or `zoom_to_down` is set.
    pub zoom_from: f32,
    /// Target zoom factor for the active zoom transition.
    pub zoom_to: f32,
    /// Source view position at the start of the active zoom transition.
    pub view_from: MapPoint,
    /// Target view position for the active zoom transition.
    pub view_to: MapPoint,
}

impl Default for BackgroundTransform {
    fn default() -> Self {
        let mut bg = Self {
            scroll_to_left: false,
            scroll_to_up: false,
            current_x_scrolling_level: 0,
            current_y_scrolling_level: 0,
            zoom_to_up: false,
            zoom_to_down: false,
            required_zoom_up: false,
            required_zoom_down: false,
            zoom_count: 0,
            number_of_zoom_steps: 0,
            x_scrolling_values: [0.0; SCROLLING_TABLE_SIZE],
            y_scrolling_values: [0.0; SCROLLING_TABLE_SIZE],
            current_zoom_level: 1, // Start at 1x zoom
            zoom_values: [0.5, 1.0, 2.0],
            center_zoom: MapVec::ZERO,
            clipped_zoom: MapVec::ZERO,
            scrolling_vector: MapVec::ZERO,
            zoom_from: 1.0,
            zoom_to: 1.0,
            view_from: MapPoint::ZERO,
            view_to: MapPoint::ZERO,
        };
        bg.generate_scrolling_table();
        bg
    }
}

impl BackgroundTransform {
    /// Pre-compute the scrolling speed ramp.
    fn generate_scrolling_table(&mut self) {
        self.x_scrolling_values[0] = 0.0;
        self.y_scrolling_values[0] = 0.0;

        let mut value = DEFAULT_SCROLLING_START;
        for i in 1..SCROLLING_TABLE_SIZE {
            // Round up to even if odd
            if !(value as u16).is_multiple_of(2) {
                value += 1.0;
            }
            let floored = value.floor();
            self.x_scrolling_values[i] = floored;
            self.y_scrolling_values[i] = floored;

            if value < DEFAULT_SCROLLING_LIMIT {
                value *= DEFAULT_SCROLLING_ACCELERATION;
            }
        }
    }
}

// ─── Camera state ────────────────────────────────────────────────────

const DIRECTOR_CAMERA_VIEW_WIDTH: f32 = 1024.0;
const DIRECTOR_CAMERA_VIEW_HEIGHT: f32 = 768.0;

/// Read-only camera motion for a host to apply to its own viewport.
/// Positions use the director's fixed virtual view, independent of canvas size.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DirectorCameraFrame {
    pub view_position: MapPoint,
    pub zoom_factor: f32,
    pub slide_target: Option<MapPoint>,
    pub owns_view: bool,
}

/// Script/director camera state.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
// Retain the former save projection's serde name; all camera fields now persist.
#[serde(rename = "PersistedCameraState")]
pub struct CameraState {
    /// Top-left corner of the view in map coordinates.
    pub view_position: MapPoint,
    /// Target position for camera slide animations.
    pub camera_slide: MapPoint,
    /// Desired camera slide destination.
    pub camera_wanted: MapPoint,
    /// Speed of fixed camera movements (0 = not active).
    pub fixed_camera_speed: u16,

    /// Current zoom factor (0.5, 1.0, or 2.0).
    pub zoom_factor: f32,
    /// Target zoom factor for smooth zoom transitions.
    pub desired_zoom_factor: f32,
    /// Whether zoom initialization is done for the current transition.
    /// This gates gameplay advancement and controls when a camera sequence
    /// element terminates, so it is deterministic snapshot state.
    pub zoom_init_done: bool,
    /// Whether the current zoom was triggered programmatically.
    /// Consumed by the deterministic camera transition when choosing its
    /// anchor, so it must survive rollback between request and init.
    pub mechanized_zoom: bool,

    /// Level size in map units.
    pub level_size: MapSize,

    // Elastic/follow-camera state for the shared script camera. Both values
    // affect the next view position and therefore participate in snapshots.
    pub displacement: MapVec,
    pub displacement_counter: u16,

    /// Snapshot of the followed element's screen-space position when
    /// locker mode engaged (or was last retargeted).  The director work
    /// loop tries to keep the target at this exact screen point every
    /// frame.  Populated by `select_follow_element`. Not strictly
    /// serialization state, but while it lives on `EngineInner` it
    /// participates in serde/hash.
    pub position_saved: ScreenPoint,

    /// Currently-executing camera sequence element (zoom / scroll-to /
    /// lock-on). The dispatcher for `Command::CameraGoto`,
    /// `Command::ZoomLevel`, and `Command::LockCameraOn` stores the
    /// element here, and `perform_director_work` marks it terminated
    /// when the zoom / slide completes.
    pub sequence_element: Option<crate::sequence::SequenceElementRef>,

    /// Whether an external replay stream owns completion timing for latched
    /// `CameraGoto` and `ZoomLevel` sequence elements. The visual transition
    /// still advances normally, but reaching its target does not release the
    /// sequence until the replay applies the recorded director event.
    #[serde(default)]
    pub external_completion_replay: bool,

    /// Display-op/zoom-transition state for the shared script camera.
    ///
    /// This is engine-owned because it advances `view_position`,
    /// `zoom_factor`, and camera sequence completion. Host-local viewport
    /// scroll/zoom has its own state in `robin_rs::Host`.
    pub display: crate::engine::CameraDisplayState,

    /// Screen-space mouse position captured when a non-mechanized zoom
    /// request fires (the host sets this before the zoom request). When zoom
    /// initialization begins, the display state
    /// consumes it to bias `view_to` so the pixel under the mouse stays
    /// anchored during the zoom: `mouse_vector = (screen_center -
    /// mouse_screen) / zoom` when the UI is not focused and the zoom
    /// is not mechanized. `None` = no mouse recentering. The value is
    /// consumed after the command boundary and therefore belongs to the
    /// deterministic camera snapshot while pending.
    #[serde(deserialize_with = "Option::deserialize")]
    pub pending_zoom_mouse_screen: Option<ScreenPoint>,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            view_position: MapPoint::ZERO,
            camera_slide: MapPoint::new(-1.0, -1.0), // -1 = inactive
            camera_wanted: MapPoint::ZERO,
            fixed_camera_speed: 0,
            zoom_factor: 1.0,
            desired_zoom_factor: 1.0,
            zoom_init_done: false,
            mechanized_zoom: false,
            level_size: MapSize::ZERO,
            displacement: MapVec::ZERO,
            displacement_counter: 0,
            position_saved: ScreenPoint::ZERO,
            sequence_element: None,
            external_completion_replay: false,
            display: crate::engine::CameraDisplayState::default(),
            pending_zoom_mouse_screen: None,
        }
    }
}

/// A director-side sequence completion recorded between simulation frames.
///
/// The Original performs camera director work during drawing, after
/// simulation tick. Parity replays apply this event before the next
/// hourglass so synchronous sequence successors observe the same boundary.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    robin_state_hash_derive::StateHash,
)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum DirectorCompletion {
    CameraGoto,
    ZoomLevel,
}

impl CameraState {
    /// Whether the camera slide is currently active.
    pub fn is_sliding(&self) -> bool {
        self.camera_slide.x >= 0.0
    }

    /// Deactivate the camera slide.
    pub(crate) fn stop_slide(&mut self) {
        self.camera_slide = MapPoint::new(-1.0, -1.0);
    }

    /// Clamp the view position so the camera stays within the level bounds.
    /// On double-axis over-clip (level smaller than the zoomed-out viewport
    /// on that axis), reset `zoom_factor` to 1.0 and return the origin.
    pub(crate) fn clip_view(&mut self) -> bool {
        let mut clipped_h = false;
        let mut clipped_v = false;

        if self.view_position.x < 0.0 {
            self.view_position.x = 0.0;
            clipped_h = true;
        }
        if self.view_position.y < 0.0 {
            self.view_position.y = 0.0;
            clipped_v = true;
        }

        let view_w = DIRECTOR_CAMERA_VIEW_WIDTH / self.zoom_factor;
        let view_h = (DIRECTOR_CAMERA_VIEW_HEIGHT - PANNEL_HEIGHT) / self.zoom_factor;

        let right_edge = self.view_position.x + view_w;
        if right_edge > self.level_size.x {
            if clipped_h {
                // Level narrower than viewport at current zoom: fall back
                // to 1× zoom and park at the origin.
                self.zoom_factor = 1.0;
                self.view_position = MapPoint::ZERO;
                return true;
            } else {
                self.view_position.x = self.level_size.x - view_w;
            }
            clipped_h = true;
        }

        let bottom_edge = self.view_position.y + view_h;
        if bottom_edge > self.level_size.y {
            if clipped_v {
                // Level shorter than viewport at current zoom.
                self.zoom_factor = 1.0;
                self.view_position = MapPoint::ZERO;
                return true;
            } else {
                self.view_position.y = self.level_size.y - view_h;
            }
            clipped_v = true;
        }

        clipped_h || clipped_v
    }
}

// ─── Host-emitted ramp consumed by the FADE_TO_BLACK opcode ─────────
// (Host struct itself moved to robin_rs::host. FadeToBlack stays here
// because `SideEffects` carries it.)

/// Two-phase pixel ramp scheduled by the `FADE_TO_BLACK` script opcode.
#[derive(
    Default,
    Clone,
    Copy,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FadeToBlack {
    /// Total frames per phase (fade-out + fade-in each last `speed` frames).
    pub speed: u32,
    /// Frames left until the whole effect ends (counts down from `2*speed`).
    pub frames_remaining: u32,
}

impl FadeToBlack {
    /// Alpha (0..=255) of the black overlay for the current frame.
    ///
    /// Per-pixel ramp: fade-out iterates `pass = speed..1` with
    /// `scale = pass / speed` (first frame `scale = 1.0`, alpha `0`;
    /// last frame `scale = 1/speed`, alpha `(speed-1)*255/speed`), and
    /// fade-in is the symmetric reverse.
    pub fn current_alpha(self) -> u8 {
        if self.speed == 0 || self.frames_remaining == 0 {
            return 0;
        }
        // Phase 1 (fade-out): frames_remaining ∈ (speed..=2*speed], alpha rises.
        // Phase 2 (fade-in):  frames_remaining ∈ (0..=speed],      alpha falls.
        let num = if self.frames_remaining > self.speed {
            // pass = frames_remaining - speed; alpha = (1 - pass/speed) * 255
            //   = (speed - pass) * 255 / speed = (2*speed - frames_remaining) * 255 / speed.
            2 * self.speed - self.frames_remaining
        } else {
            // pass = frames_remaining; scale = (speed - (pass-1))/speed;
            //   alpha = (1 - scale) * 255 = (pass - 1) * 255 / speed.
            self.frames_remaining - 1
        };
        ((num * 255) / self.speed).min(255) as u8
    }

    /// Consume one frame after the live framebuffer has been presented.
    ///
    /// Returns whether another fade frame remains. Keeping this separate
    /// from drawing prevents throwaway screenshot and thumbnail renders
    /// from shortening the transition.
    pub fn advance_presented_frame(&mut self) -> bool {
        self.frames_remaining = self.frames_remaining.saturating_sub(1);
        self.frames_remaining > 0
    }
}

//! Platform-independent touch gesture classification.
//!
//! Winit reports independent pointer changes, while the game expects either a
//! complete mouse interaction or a camera gesture.  This module owns that
//! arbitration before either kind of event reaches gameplay.  In particular,
//! a pending tap never emits a mouse-down until it is known to be a tap, so a
//! second finger can cancel it without accidentally committing a world/UI
//! action.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

const DEFAULT_TOUCH_SLOP_PX: f64 = 12.0;
const DOUBLE_TAP_INTERVAL_MS: u32 = 300;
const DEFAULT_DOUBLE_TAP_SLOP_PX: f64 = 32.0;
const VELOCITY_FILTER_NEW_SAMPLE: f64 = 0.35;
const MAX_FLING_SAMPLE_AGE_MS: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct TouchPoint {
    start_x: f64,
    start_y: f64,
    x: f64,
    y: f64,
}

impl TouchPoint {
    fn new(x: f64, y: f64) -> Self {
        Self {
            start_x: x,
            start_y: y,
            x,
            y,
        }
    }

    fn current(self) -> (f64, f64) {
        (self.x, self.y)
    }

    fn origin(self) -> (f64, f64) {
        (self.start_x, self.start_y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) enum TouchOutput {
    /// A new primary contact immediately stops any camera momentum, before
    /// the contact is release-classified as a tap or drag.
    MotionStop,
    PointerMove {
        x: f64,
        y: f64,
    },
    PointerDown {
        x: f64,
        y: f64,
        clicks: u8,
    },
    PointerUp {
        x: f64,
        y: f64,
    },
    /// Abort a mouse drag without running its release/click action.
    PointerCancel,
    TransformStart {
        first_x: f64,
        first_y: f64,
        second_x: f64,
        second_y: f64,
    },
    TransformUpdate {
        centroid_x: f64,
        centroid_y: f64,
        pan_x: f64,
        pan_y: f64,
        scale: f64,
        velocity_x: f64,
        velocity_y: f64,
    },
    TransformEnd {
        velocity_x: f64,
        velocity_y: f64,
        cancelled: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
enum GestureState {
    Idle,
    PendingTap {
        id: u64,
        start_x: f64,
        start_y: f64,
    },
    PointerDrag {
        id: u64,
    },
    Transform {
        last_centroid_x: f64,
        last_centroid_y: f64,
        last_distance: f64,
        velocity_sample_centroid_x: f64,
        velocity_sample_centroid_y: f64,
        velocity_sample_ms: u32,
        velocity_x: f64,
        velocity_y: f64,
    },
    /// One finger remains after a transform.  It must be lifted before a new
    /// one-finger interaction can begin; promoting it would create a click at
    /// an unrelated location.
    Suppressed,
}

/// Pure touch recognizer. Timestamps are caller-supplied monotonic
/// milliseconds, which keeps unit tests deterministic and avoids serializing
/// platform `Instant` values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TouchClassifier {
    touches: BTreeMap<u64, TouchPoint>,
    ignored_touches: BTreeSet<u64>,
    state: GestureState,
    last_tap: Option<(u32, f64, f64)>,
    touch_slop_px: f64,
    double_tap_slop_px: f64,
}

impl Default for TouchClassifier {
    fn default() -> Self {
        Self::new(DEFAULT_TOUCH_SLOP_PX)
    }
}

impl TouchClassifier {
    pub(crate) fn new(touch_slop_px: f64) -> Self {
        assert!(touch_slop_px.is_finite() && touch_slop_px > 0.0);
        Self {
            touches: BTreeMap::new(),
            ignored_touches: BTreeSet::new(),
            state: GestureState::Idle,
            last_tap: None,
            touch_slop_px,
            double_tap_slop_px: touch_slop_px * DEFAULT_DOUBLE_TAP_SLOP_PX / DEFAULT_TOUCH_SLOP_PX,
        }
    }

    /// Winit touch locations are physical pixels. Scale the logical gesture
    /// tolerances to match the current monitor density.
    pub(crate) fn set_scale_factor(&mut self, scale_factor: f64) {
        assert!(scale_factor.is_finite() && scale_factor > 0.0);
        self.touch_slop_px = DEFAULT_TOUCH_SLOP_PX * scale_factor;
        self.double_tap_slop_px = DEFAULT_DOUBLE_TAP_SLOP_PX * scale_factor;
    }

    pub(crate) fn started(&mut self, id: u64, x: f64, y: f64, now_ms: u32) -> Vec<TouchOutput> {
        if self.touches.contains_key(&id) || self.ignored_touches.contains(&id) {
            return Vec::new();
        }
        if self.touches.len() >= 2 {
            self.ignored_touches.insert(id);
            return Vec::new();
        }
        self.touches.insert(id, TouchPoint::new(x, y));

        match self.state {
            GestureState::Idle if self.touches.len() == 1 => {
                self.state = GestureState::PendingTap {
                    id,
                    start_x: x,
                    start_y: y,
                };
                vec![TouchOutput::MotionStop]
            }
            GestureState::PendingTap { .. } | GestureState::PointerDrag { .. }
                if self.touches.len() == 2 =>
            {
                let mut output = Vec::new();
                if matches!(self.state, GestureState::PointerDrag { .. }) {
                    output.push(TouchOutput::PointerCancel);
                }
                let (first, second) = self.two_points();
                let ((first_x, first_y), (second_x, second_y)) = self.two_origins();
                let (centroid_x, centroid_y, distance) = transform_geometry(first, second);
                self.state = GestureState::Transform {
                    last_centroid_x: centroid_x,
                    last_centroid_y: centroid_y,
                    last_distance: distance,
                    velocity_sample_centroid_x: centroid_x,
                    velocity_sample_centroid_y: centroid_y,
                    velocity_sample_ms: now_ms,
                    velocity_x: 0.0,
                    velocity_y: 0.0,
                };
                self.last_tap = None;
                output.push(TouchOutput::TransformStart {
                    first_x,
                    first_y,
                    second_x,
                    second_y,
                });
                output
            }
            GestureState::Suppressed | GestureState::Transform { .. } => Vec::new(),
            state => {
                tracing::warn!(
                    ?state,
                    touch_count = self.touches.len(),
                    "unexpected touch start"
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn moved(&mut self, id: u64, x: f64, y: f64, now_ms: u32) -> Vec<TouchOutput> {
        if self.ignored_touches.contains(&id) || !self.touches.contains_key(&id) {
            return Vec::new();
        }
        let Some(point) = self.touches.get_mut(&id) else {
            return Vec::new();
        };
        point.x = x;
        point.y = y;
        match self.state {
            GestureState::PendingTap {
                id: primary,
                start_x,
                start_y,
            } if primary == id => {
                if distance((start_x, start_y), (x, y)) < self.touch_slop_px {
                    return Vec::new();
                }
                self.state = GestureState::PointerDrag { id };
                self.last_tap = None;
                vec![
                    TouchOutput::PointerDown {
                        x: start_x,
                        y: start_y,
                        clicks: 1,
                    },
                    TouchOutput::PointerMove { x, y },
                ]
            }
            GestureState::PointerDrag { id: primary } if primary == id => {
                vec![TouchOutput::PointerMove { x, y }]
            }
            GestureState::Transform {
                last_centroid_x,
                last_centroid_y,
                last_distance,
                velocity_sample_centroid_x,
                velocity_sample_centroid_y,
                velocity_sample_ms,
                velocity_x,
                velocity_y,
            } if self.touches.len() == 2 => {
                let (first, second) = self.two_points();
                let (centroid_x, centroid_y, current_distance) = transform_geometry(first, second);
                let pan_x = centroid_x - last_centroid_x;
                let pan_y = centroid_y - last_centroid_y;
                let scale = if last_distance > f64::EPSILON {
                    current_distance / last_distance
                } else {
                    1.0
                };
                let elapsed_ms = now_ms.wrapping_sub(velocity_sample_ms);
                let (
                    velocity_sample_centroid_x,
                    velocity_sample_centroid_y,
                    velocity_sample_ms,
                    velocity_x,
                    velocity_y,
                ) = if elapsed_ms > 0 {
                    let elapsed_ms = f64::from(elapsed_ms);
                    let raw_velocity_x =
                        (centroid_x - velocity_sample_centroid_x) * 1000.0 / elapsed_ms;
                    let raw_velocity_y =
                        (centroid_y - velocity_sample_centroid_y) * 1000.0 / elapsed_ms;
                    (
                        centroid_x,
                        centroid_y,
                        now_ms,
                        velocity_x * (1.0 - VELOCITY_FILTER_NEW_SAMPLE)
                            + raw_velocity_x * VELOCITY_FILTER_NEW_SAMPLE,
                        velocity_y * (1.0 - VELOCITY_FILTER_NEW_SAMPLE)
                            + raw_velocity_y * VELOCITY_FILTER_NEW_SAMPLE,
                    )
                } else {
                    // Winit can batch both contacts' move events under the
                    // same millisecond timestamp. Accumulate their centroid
                    // displacement until time advances instead of treating
                    // the second event as an implausible 1 ms sample.
                    (
                        velocity_sample_centroid_x,
                        velocity_sample_centroid_y,
                        velocity_sample_ms,
                        velocity_x,
                        velocity_y,
                    )
                };
                self.state = GestureState::Transform {
                    last_centroid_x: centroid_x,
                    last_centroid_y: centroid_y,
                    last_distance: current_distance,
                    velocity_sample_centroid_x,
                    velocity_sample_centroid_y,
                    velocity_sample_ms,
                    velocity_x,
                    velocity_y,
                };
                vec![TouchOutput::TransformUpdate {
                    centroid_x,
                    centroid_y,
                    pan_x,
                    pan_y,
                    scale,
                    velocity_x,
                    velocity_y,
                }]
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn ended(
        &mut self,
        id: u64,
        x: f64,
        y: f64,
        now_ms: u32,
        cancelled: bool,
    ) -> Vec<TouchOutput> {
        if self.ignored_touches.remove(&id) {
            if self.touches.is_empty() && self.ignored_touches.is_empty() {
                self.state = GestureState::Idle;
            }
            return Vec::new();
        }
        if !self.touches.contains_key(&id) {
            return Vec::new();
        }
        let final_transform_update = !cancelled
            && matches!(self.state, GestureState::Transform { .. })
            && self
                .touches
                .get(&id)
                .is_some_and(|point| point.x != x || point.y != y);
        let mut output = if final_transform_update {
            self.moved(id, x, y, now_ms)
        } else {
            Vec::new()
        };
        let Some(point) = self.touches.get_mut(&id) else {
            return Vec::new();
        };
        point.x = x;
        point.y = y;
        let state = self.state;
        output.extend(match state {
            GestureState::PendingTap {
                id: primary,
                start_x,
                start_y,
            } if primary == id => {
                if cancelled {
                    self.last_tap = None;
                    Vec::new()
                } else if distance((start_x, start_y), (x, y)) >= self.touch_slop_px {
                    // Some platforms coalesce a short drag into Start+End
                    // without an intermediate Move. Preserve release-time
                    // classification by materializing that complete drag now.
                    self.last_tap = None;
                    vec![
                        TouchOutput::PointerDown {
                            x: start_x,
                            y: start_y,
                            clicks: 1,
                        },
                        TouchOutput::PointerMove { x, y },
                        TouchOutput::PointerUp { x, y },
                    ]
                } else {
                    let clicks = self.tap_count(now_ms, x, y);
                    vec![
                        TouchOutput::PointerMove { x, y },
                        TouchOutput::PointerDown { x, y, clicks },
                        TouchOutput::PointerUp { x, y },
                    ]
                }
            }
            GestureState::PointerDrag { id: primary } if primary == id => {
                self.last_tap = None;
                if cancelled {
                    vec![TouchOutput::PointerCancel]
                } else {
                    vec![
                        TouchOutput::PointerMove { x, y },
                        TouchOutput::PointerUp { x, y },
                    ]
                }
            }
            GestureState::Transform {
                velocity_x,
                velocity_y,
                velocity_sample_ms,
                ..
            } => {
                self.last_tap = None;
                let velocity_is_fresh =
                    now_ms.wrapping_sub(velocity_sample_ms) <= MAX_FLING_SAMPLE_AGE_MS;
                vec![TouchOutput::TransformEnd {
                    velocity_x: if velocity_is_fresh { velocity_x } else { 0.0 },
                    velocity_y: if velocity_is_fresh { velocity_y } else { 0.0 },
                    cancelled,
                }]
            }
            _ => Vec::new(),
        });
        self.touches.remove(&id);
        self.state = if self.touches.is_empty() && self.ignored_touches.is_empty() {
            GestureState::Idle
        } else {
            GestureState::Suppressed
        };
        output
    }

    pub(crate) fn cancel_all(&mut self) -> Vec<TouchOutput> {
        let output = match self.state {
            GestureState::PointerDrag { .. } => vec![TouchOutput::PointerCancel],
            GestureState::Transform { .. } => vec![TouchOutput::TransformEnd {
                velocity_x: 0.0,
                velocity_y: 0.0,
                cancelled: true,
            }],
            _ => Vec::new(),
        };
        self.touches.clear();
        self.ignored_touches.clear();
        self.state = GestureState::Idle;
        self.last_tap = None;
        output
    }

    fn two_points(&self) -> ((f64, f64), (f64, f64)) {
        let mut values = self.touches.values().copied();
        let first = values
            .next()
            .expect("two-point gesture lost first touch")
            .current();
        let second = values
            .next()
            .expect("two-point gesture lost second touch")
            .current();
        assert!(
            values.next().is_none(),
            "touch classifier tracked too many pointers"
        );
        (first, second)
    }

    fn two_origins(&self) -> ((f64, f64), (f64, f64)) {
        let mut values = self.touches.values().copied();
        let first = values
            .next()
            .expect("two-point gesture lost first touch")
            .origin();
        let second = values
            .next()
            .expect("two-point gesture lost second touch")
            .origin();
        assert!(
            values.next().is_none(),
            "touch classifier tracked too many pointers"
        );
        (first, second)
    }

    fn tap_count(&mut self, now_ms: u32, x: f64, y: f64) -> u8 {
        let is_double = self.last_tap.is_some_and(|(last_ms, last_x, last_y)| {
            now_ms.wrapping_sub(last_ms) <= DOUBLE_TAP_INTERVAL_MS
                && distance((last_x, last_y), (x, y)) <= self.double_tap_slop_px
        });
        if is_double {
            self.last_tap = None;
            2
        } else {
            self.last_tap = Some((now_ms, x, y));
            1
        }
    }
}

fn transform_geometry(first: (f64, f64), second: (f64, f64)) -> (f64, f64, f64) {
    (
        (first.0 + second.0) * 0.5,
        (first.1 + second.1) * 0.5,
        distance(first, second).max(f64::EPSILON),
    )
}

fn distance(first: (f64, f64), second: (f64, f64)) -> f64 {
    (second.0 - first.0).hypot(second.1 - first.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_is_emitted_as_atomic_release_classified_click() {
        let mut touch = TouchClassifier::default();
        assert_eq!(
            touch.started(7, 10.0, 20.0, 100),
            vec![TouchOutput::MotionStop]
        );
        assert_eq!(
            touch.ended(7, 11.0, 21.0, 140, false),
            vec![
                TouchOutput::PointerMove { x: 11.0, y: 21.0 },
                TouchOutput::PointerDown {
                    x: 11.0,
                    y: 21.0,
                    clicks: 1
                },
                TouchOutput::PointerUp { x: 11.0, y: 21.0 }
            ]
        );
    }

    #[test]
    fn release_displacement_classifies_a_coalesced_drag_as_a_drag() {
        let mut touch = TouchClassifier::new(5.0);
        touch.started(7, 10.0, 20.0, 100);

        assert_eq!(
            touch.ended(7, 30.0, 20.0, 140, false),
            vec![
                TouchOutput::PointerDown {
                    x: 10.0,
                    y: 20.0,
                    clicks: 1,
                },
                TouchOutput::PointerMove { x: 30.0, y: 20.0 },
                TouchOutput::PointerUp { x: 30.0, y: 20.0 },
            ]
        );
    }

    #[test]
    fn second_finger_cancels_pending_tap_without_mouse_release() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 10.0, 10.0, 0);
        let output = touch.started(2, 30.0, 10.0, 20);
        assert!(matches!(
            output.as_slice(),
            [TouchOutput::TransformStart { .. }]
        ));
        assert!(!output.iter().any(|event| matches!(
            event,
            TouchOutput::PointerDown { .. } | TouchOutput::PointerUp { .. }
        )));
    }

    #[test]
    fn second_finger_explicitly_cancels_started_drag() {
        let mut touch = TouchClassifier::new(5.0);
        touch.started(1, 10.0, 10.0, 0);
        assert!(matches!(
            touch.moved(1, 20.0, 10.0, 10).as_slice(),
            [
                TouchOutput::PointerDown { .. },
                TouchOutput::PointerMove { .. }
            ]
        ));
        assert!(matches!(
            touch.started(2, 30.0, 10.0, 20).as_slice(),
            [
                TouchOutput::PointerCancel,
                TouchOutput::TransformStart { .. }
            ]
        ));
    }

    #[test]
    fn transform_reports_simultaneous_pan_scale_and_velocity() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 0);
        let output = touch.moved(2, 22.0, 0.0, 100);
        let [
            TouchOutput::TransformUpdate {
                centroid_x,
                pan_x,
                scale,
                velocity_x,
                ..
            },
        ] = output.as_slice()
        else {
            panic!("expected transform update: {output:?}");
        };
        assert_eq!(*centroid_x, 11.0);
        assert_eq!(*pan_x, 6.0);
        assert_eq!(*scale, 2.2);
        assert!(*velocity_x > 0.0);
    }

    #[test]
    fn transform_end_includes_a_coalesced_final_position() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 0);

        let output = touch.ended(2, 20.0, 0.0, 100, false);
        assert!(matches!(
            output.as_slice(),
            [
                TouchOutput::TransformUpdate {
                    centroid_x: 10.0,
                    pan_x: 5.0,
                    scale: 2.0,
                    ..
                },
                TouchOutput::TransformEnd {
                    cancelled: false,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn remaining_finger_after_transform_cannot_turn_into_tap() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 0);
        assert!(matches!(
            touch.ended(2, 10.0, 0.0, 50, false).as_slice(),
            [TouchOutput::TransformEnd { .. }]
        ));
        assert!(touch.ended(1, 0.0, 0.0, 60, false).is_empty());
    }

    #[test]
    fn nearby_taps_emit_double_click_but_distant_taps_do_not() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 10.0, 10.0, 0);
        touch.ended(1, 10.0, 10.0, 20, false);
        touch.started(2, 12.0, 11.0, 100);
        let second = touch.ended(2, 12.0, 11.0, 120, false);
        assert!(
            second
                .iter()
                .any(|event| matches!(event, TouchOutput::PointerDown { clicks: 2, .. }))
        );

        touch.started(3, 100.0, 100.0, 160);
        let distant = touch.ended(3, 100.0, 100.0, 170, false);
        assert!(
            distant
                .iter()
                .any(|event| matches!(event, TouchOutput::PointerDown { clicks: 1, .. }))
        );
    }

    #[test]
    fn third_pointer_is_ignored_without_disturbing_transform() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 0);
        assert!(touch.started(3, 20.0, 0.0, 0).is_empty());
        assert!(touch.moved(3, 30.0, 0.0, 10).is_empty());
        assert!(touch.ended(3, 30.0, 0.0, 20, false).is_empty());
        assert!(matches!(
            touch.moved(2, 15.0, 0.0, 30).as_slice(),
            [TouchOutput::TransformUpdate { .. }]
        ));
    }

    #[test]
    fn transform_start_reports_pointer_origins_for_world_eligibility() {
        let mut touch = TouchClassifier::new(5.0);
        touch.started(1, 10.0, 10.0, 0);
        touch.moved(1, 100.0, 100.0, 10);
        let output = touch.started(2, 120.0, 100.0, 20);

        let Some(TouchOutput::TransformStart {
            first_x,
            first_y,
            second_x,
            second_y,
        }) = output.last()
        else {
            panic!("expected transform start: {output:?}");
        };
        assert_eq!((*first_x, *first_y), (10.0, 10.0));
        assert_eq!((*second_x, *second_y), (120.0, 100.0));
    }

    #[test]
    fn ignored_third_pointer_cannot_leave_recognizer_suppressed() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 0);
        touch.started(3, 20.0, 0.0, 0);
        touch.ended(1, 0.0, 0.0, 10, false);
        touch.ended(2, 10.0, 0.0, 20, false);
        touch.ended(3, 20.0, 0.0, 30, false);

        assert_eq!(
            touch.started(4, 40.0, 40.0, 40),
            vec![TouchOutput::MotionStop]
        );
        assert!(
            touch
                .ended(4, 40.0, 40.0, 50, false)
                .iter()
                .any(|event| { matches!(event, TouchOutput::PointerDown { clicks: 1, .. }) })
        );
    }

    #[test]
    fn density_scale_applies_to_drag_and_double_tap_slop() {
        let mut touch = TouchClassifier::default();
        touch.set_scale_factor(2.0);
        touch.started(1, 0.0, 0.0, 0);
        assert!(touch.moved(1, 15.0, 0.0, 10).is_empty());
        touch.ended(1, 15.0, 0.0, 20, false);

        touch.started(2, 55.0, 0.0, 100);
        let second = touch.ended(2, 55.0, 0.0, 120, false);
        assert!(
            second
                .iter()
                .any(|event| { matches!(event, TouchOutput::PointerDown { clicks: 2, .. }) })
        );
    }

    #[test]
    fn same_millisecond_moves_accumulate_without_velocity_spike() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 0);
        touch.moved(1, 2.0, 0.0, 10);
        let output = touch.moved(2, 12.0, 0.0, 10);
        let [TouchOutput::TransformUpdate { velocity_x, .. }] = output.as_slice() else {
            panic!("expected transform update: {output:?}");
        };
        assert!((*velocity_x - 35.0).abs() < f64::EPSILON);

        let output = touch.moved(2, 14.0, 0.0, 20);
        let [TouchOutput::TransformUpdate { velocity_x, .. }] = output.as_slice() else {
            panic!("expected transform update: {output:?}");
        };
        assert!(*velocity_x < 200.0);
    }

    #[test]
    fn holding_transform_still_before_release_suppresses_stale_fling() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 0);
        touch.moved(2, 20.0, 0.0, 20);

        assert!(matches!(
            touch.ended(2, 20.0, 0.0, 200, false).as_slice(),
            [TouchOutput::TransformEnd {
                velocity_x: 0.0,
                velocity_y: 0.0,
                cancelled: false,
            }]
        ));
    }

    #[test]
    fn intervening_drag_or_transform_invalidates_double_tap_history() {
        let mut touch = TouchClassifier::new(5.0);
        touch.started(1, 10.0, 10.0, 0);
        touch.ended(1, 10.0, 10.0, 10, false);
        touch.started(2, 10.0, 10.0, 20);
        touch.moved(2, 20.0, 10.0, 30);
        touch.ended(2, 20.0, 10.0, 40, false);
        touch.started(3, 10.0, 10.0, 50);
        let after_drag = touch.ended(3, 10.0, 10.0, 60, false);
        assert!(
            after_drag
                .iter()
                .any(|event| matches!(event, TouchOutput::PointerDown { clicks: 1, .. }))
        );

        touch.started(4, 10.0, 10.0, 70);
        touch.started(5, 20.0, 10.0, 80);
        touch.ended(5, 20.0, 10.0, 90, false);
        touch.ended(4, 10.0, 10.0, 100, false);
        touch.started(6, 10.0, 10.0, 110);
        let after_transform = touch.ended(6, 10.0, 10.0, 120, false);
        assert!(
            after_transform
                .iter()
                .any(|event| matches!(event, TouchOutput::PointerDown { clicks: 1, .. }))
        );
    }
}

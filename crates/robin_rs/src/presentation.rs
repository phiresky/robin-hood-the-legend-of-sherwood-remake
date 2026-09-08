//! Immutable, frame-addressed presentation data prepared before drawing.
//!
//! Presentation state owns no GPU resources. Its update methods advance
//! transient UI state once per simulation frame, while renderers receive a
//! copyable snapshot that can be consumed repeatedly by live, screenshot,
//! and thumbnail passes.

use serde::{Deserialize, Serialize};

use crate::zoom_hud::{ZoomButton, ZoomButtonEnable, ZoomTooltipTracker};

/// Typed identity of the simulation frame whose presentation was prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PresentationFrameId(u32);

impl PresentationFrameId {
    pub const fn new(engine_frame: u32) -> Self {
        Self(engine_frame)
    }
}

/// Stable handle for a zoom control in immutable presentation data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoomControlHandle {
    ZoomUp,
    ZoomDown,
}

impl From<ZoomButton> for ZoomControlHandle {
    fn from(value: ZoomButton) -> Self {
        match value {
            ZoomButton::ZoomUp => Self::ZoomUp,
            ZoomButton::ZoomDown => Self::ZoomDown,
        }
    }
}

impl From<ZoomControlHandle> for ZoomButton {
    fn from(value: ZoomControlHandle) -> Self {
        match value {
            ZoomControlHandle::ZoomUp => Self::ZoomUp,
            ZoomControlHandle::ZoomDown => Self::ZoomDown,
        }
    }
}

/// Immutable visual state of one zoom button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoomButtonPresentation {
    pub enabled: bool,
    pub selected: bool,
}

/// Values collected by update for the zoom presentation area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoomPresentationUpdate {
    pub zoom_up: ZoomButtonPresentation,
    pub zoom_down: ZoomButtonPresentation,
    pub hovered: Option<ZoomControlHandle>,
    pub mouse_pressed: bool,
}

impl ZoomPresentationUpdate {
    pub fn new(enable: ZoomButtonEnable, hovered: Option<ZoomButton>, mouse_pressed: bool) -> Self {
        Self {
            zoom_up: ZoomButtonPresentation {
                enabled: enable.zoom_up,
                selected: enable.selected_up,
            },
            zoom_down: ZoomButtonPresentation {
                enabled: enable.zoom_down,
                selected: enable.selected_down,
            },
            hovered: hovered.map(ZoomControlHandle::from),
            mouse_pressed,
        }
    }
}

/// Immutable zoom-HUD data consumed by every render pass for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoomPresentation {
    pub frame_id: PresentationFrameId,
    pub zoom_up: ZoomButtonPresentation,
    pub zoom_down: ZoomButtonPresentation,
    pub hovered: Option<ZoomControlHandle>,
    pub mouse_pressed: bool,
    pub ready_tooltip: Option<ZoomControlHandle>,
}

impl ZoomPresentation {
    pub fn button_enable(self) -> ZoomButtonEnable {
        ZoomButtonEnable {
            zoom_up: self.zoom_up.enabled,
            zoom_down: self.zoom_down.enabled,
            selected_up: self.zoom_up.selected,
            selected_down: self.zoom_down.selected,
        }
    }
}

/// Typed failure returned when drawing asks for an unprepared frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("zoom presentation for frame {requested:?} is unavailable (prepared: {prepared:?})")]
pub struct ZoomPresentationUnavailable {
    pub requested: PresentationFrameId,
    pub prepared: Option<PresentationFrameId>,
}

/// Update-owned state for the zoom presentation area.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ZoomPresentationState {
    prepared: Option<ZoomPresentation>,
}

impl ZoomPresentationState {
    /// Advance hover state and publish a presentation snapshot once per
    /// simulation frame. Repeated screenshot/live preparation for the same
    /// frame is deliberately idempotent.
    pub fn update(
        &mut self,
        frame_id: PresentationFrameId,
        input: ZoomPresentationUpdate,
        tooltip_tracker: &mut ZoomTooltipTracker,
    ) {
        if self
            .prepared
            .is_some_and(|presentation| presentation.frame_id == frame_id)
        {
            return;
        }

        let hovered = input.hovered.map(ZoomButton::from);
        tooltip_tracker.update(hovered);
        let ready_tooltip = tooltip_tracker.ready_button().map(ZoomControlHandle::from);

        self.prepared = Some(ZoomPresentation {
            frame_id,
            zoom_up: input.zoom_up,
            zoom_down: input.zoom_down,
            hovered: input.hovered,
            mouse_pressed: input.mouse_pressed,
            ready_tooltip,
        });
    }

    /// Borrow the immutable render data for `frame_id`.
    pub fn presentation(
        &self,
        frame_id: PresentationFrameId,
    ) -> Result<&ZoomPresentation, ZoomPresentationUnavailable> {
        self.prepared
            .as_ref()
            .filter(|presentation| presentation.frame_id == frame_id)
            .ok_or(ZoomPresentationUnavailable {
                requested: frame_id,
                prepared: self.prepared.map(|presentation| presentation.frame_id),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zoom_up_input() -> ZoomPresentationUpdate {
        ZoomPresentationUpdate::new(
            ZoomButtonEnable {
                zoom_up: true,
                zoom_down: false,
                selected_up: false,
                selected_down: false,
            },
            Some(ZoomButton::ZoomUp),
            false,
        )
    }

    #[test]
    fn repeated_render_reads_are_pure() {
        let mut state = ZoomPresentationState::default();
        let mut tracker = ZoomTooltipTracker::new();
        let frame = PresentationFrameId::new(41);
        state.update(frame, zoom_up_input(), &mut tracker);
        let state_after_update = state.clone();

        for _ in 0..8 {
            assert_eq!(
                state.presentation(frame).expect("prepared presentation"),
                state_after_update
                    .presentation(frame)
                    .expect("prepared presentation")
            );
        }

        assert_eq!(state, state_after_update);
    }

    #[test]
    fn repeated_same_frame_updates_do_not_advance_tooltip_timer() {
        let mut state = ZoomPresentationState::default();
        let mut tracker = ZoomTooltipTracker::new();
        let input = zoom_up_input();

        for _ in 0..100 {
            state.update(PresentationFrameId::new(10), input, &mut tracker);
        }
        assert_eq!(
            state
                .presentation(PresentationFrameId::new(10))
                .expect("prepared presentation")
                .ready_tooltip,
            None
        );

        // The existing tracker uses a strict `> 75` comparison. The first
        // distinct frame arms it at zero, then 76 distinct update frames are
        // required before the tooltip is ready.
        for frame in 11..=85 {
            state.update(PresentationFrameId::new(frame), input, &mut tracker);
        }
        assert_eq!(
            state
                .presentation(PresentationFrameId::new(85))
                .expect("prepared presentation")
                .ready_tooltip,
            None
        );
        state.update(PresentationFrameId::new(86), input, &mut tracker);
        assert_eq!(
            state
                .presentation(PresentationFrameId::new(86))
                .expect("prepared presentation")
                .ready_tooltip,
            Some(ZoomControlHandle::ZoomUp)
        );
    }

    #[test]
    fn stale_frame_returns_typed_error() {
        let mut state = ZoomPresentationState::default();
        let mut tracker = ZoomTooltipTracker::new();
        state.update(PresentationFrameId::new(7), zoom_up_input(), &mut tracker);

        assert_eq!(
            state.presentation(PresentationFrameId::new(8)),
            Err(ZoomPresentationUnavailable {
                requested: PresentationFrameId::new(8),
                prepared: Some(PresentationFrameId::new(7)),
            })
        );
    }
}

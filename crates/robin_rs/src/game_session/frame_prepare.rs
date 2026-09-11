//! Interactive frame input and operation preparation.
//!
//! This phase owns the exclusive mission and application-service borrows until
//! it has finalized the deterministic command stream. No presentation borrow
//! escapes the phase or crosses into simulation.

mod input;
mod operation;
mod pre_tick;
mod transport;

pub(super) use pre_tick::process_pre_tick_state_hash;

use std::ops::ControlFlow;

use super::flow::{FrameControl, MissionServices};
use super::*;

/// Values produced by graphical network ingress at the frame boundary.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct FrameStart {
    pub(super) frame: MissionFrame,
    pub(super) mp_clock_pause: bool,
}

/// Deterministic and presentation flags carried across the tick boundary.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct PreparedFrame {
    pub(super) frame: MissionFrame,
    pub(super) rewind_active: bool,
    pub(super) paused: bool,
    pub(super) consumed_buffered: bool,
    pub(super) shift_held: bool,
    pub(super) modal_rendered: bool,
    pub(super) step_forward_pressed: bool,
    pub(super) step_back_pressed: bool,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum FramePreparation {
    Ready(PreparedFrame),
    Control(FrameControl),
}

/// Collect input, drive operation/save flows, and finalize the pre-tick
/// command stream.
pub(super) struct InteractiveFramePreparation<'mission, 'services, 'app> {
    mission: &'mission mut InteractiveMission,
    services: &'services mut MissionServices<'app>,
}

// Each phase consumes the previous phase's output, so skipping or repeating a
// phase cannot be represented by the preparation API.
#[derive(serde::Serialize, serde::Deserialize)]
struct InputPrepared(PreparationPhaseState);

#[derive(serde::Serialize, serde::Deserialize)]
struct SavesPrepared(PreparationPhaseState);

#[derive(serde::Serialize, serde::Deserialize)]
struct PreparationPhaseState {
    frame: MissionFrame,
    mp_clock_pause: bool,
    pause_closed_this_frame: bool,
    rewind_active: bool,
    shift_held: bool,
    step_forward_pressed: bool,
    step_back_pressed: bool,
    modal_rendered_this_frame: bool,
}

impl<'mission, 'services, 'app> InteractiveFramePreparation<'mission, 'services, 'app> {
    pub(super) fn new(
        mission: &'mission mut InteractiveMission,
        services: &'services mut MissionServices<'app>,
    ) -> Self {
        Self { mission, services }
    }

    /// Consume each phase's output before entering the next boundary. Only
    /// this coordinator borrows the entire mission and service collection.
    pub(super) async fn run(self) -> Result<FramePreparation, String> {
        let InteractiveMission {
            runtime,
            frontend,
            campaign_transition,
        } = self.mission;
        let MissionRuntime {
            world,
            timeline,
            control,
            leaderboard,
            ..
        } = runtime;
        let services = self.services;
        let input = match input::collect_input_and_menus(
            world,
            timeline,
            control,
            frontend,
            campaign_transition,
            services.window,
            services.callbacks,
            services.profiles,
        )
        .await?
        {
            ControlFlow::Break(control) => return Ok(FramePreparation::Control(control)),
            ControlFlow::Continue(input) => input,
        };
        let saves = match operation::process_operation_and_save(
            world.mutation(),
            timeline,
            frontend,
            campaign_transition,
            services.window,
            services.callbacks,
            services.profiles,
            services.args,
            input,
        )
        .await?
        {
            ControlFlow::Break(control) => return Ok(FramePreparation::Control(control)),
            ControlFlow::Continue(saves) => saves,
        };
        pre_tick::finalize_pre_tick(
            world.pre_tick_phase(),
            timeline,
            &mut control.manual_pause,
            leaderboard,
            &frontend.input,
            &frontend.ui,
            saves,
        )
    }
}

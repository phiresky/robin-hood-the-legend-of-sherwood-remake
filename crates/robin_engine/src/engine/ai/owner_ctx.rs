//! Shared per-owner execution context for AI behaviour code.

use super::*;
use crate::ai::{AiController, AiState, DutyFlags, Remark, SpeechFlags, Substate};
use crate::ai_enemy::EnemyAi;
use crate::engine::TickCtx;

/// The engine plus the immutable tick inputs and the actor whose AI is
/// currently executing.
pub(in crate::engine) struct AiOwnerCtx<'a> {
    pub(in crate::engine) engine: &'a mut EngineInner,
    pub(in crate::engine) tcx: TickCtx<'a>,
    pub(in crate::engine) owner: EntityId,
}

impl<'a> AiOwnerCtx<'a> {
    pub(in crate::engine) fn new(
        engine: &'a mut EngineInner,
        tcx: TickCtx<'a>,
        owner: EntityId,
    ) -> Self {
        Self { engine, tcx, owner }
    }

    pub(super) fn controller(&self) -> &AiController {
        self.engine.ai(self.owner, "AI owner")
    }

    pub(super) fn controller_mut(&mut self) -> &mut AiController {
        self.engine.ai_mut(self.owner, "AI owner")
    }

    pub(super) fn enemy(&self) -> &EnemyAi {
        self.engine.enemy_ai(self.owner, "AI owner")
    }

    pub(super) fn enemy_mut(&mut self) -> &mut EnemyAi {
        self.engine.enemy_ai_mut(self.owner, "AI owner")
    }

    pub(super) fn state(&mut self, state: AiState, substate: Substate) {
        self.engine
            .duty_set_state(self.tcx, self.owner, state, substate);
    }

    pub(super) fn seek_state(&mut self, substate: Substate) {
        self.state(AiState::Seeking, substate);
    }

    pub(super) fn timer(&mut self, frames: u32) {
        let frame = self.engine.control.frame_counter;
        self.enemy_mut().base.launch_timer(frames, frame);
    }

    pub(super) fn duty(&mut self) {
        self.engine
            .execute_ai_return_to_duty(self.tcx, self.owner, DutyFlags::empty());
    }

    /// The antagonist the owner is currently dealing with.
    pub(super) fn target(&self) -> EntityId {
        let handle = self
            .enemy()
            .base
            .antagonist
            .expect("AI owner requires an antagonist");
        self.engine
            .expect_human_id_for_ai_handle(handle.get(), "AI owner antagonist")
    }

    pub(super) fn say(&mut self, remark: Remark, flags: SpeechFlags) {
        self.engine.execute_ai_speech(
            self.tcx,
            self.owner,
            crate::ai::AiSpeechAttempt {
                remark,
                flags: flags.bits(),
            },
        );
    }
}

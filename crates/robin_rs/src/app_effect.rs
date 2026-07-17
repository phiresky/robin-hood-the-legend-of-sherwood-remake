//! Ordered application-shell effects emitted by game-flow code.
//!
//! These effects cross from [`crate::game::Game`] into host-owned audio
//! and input state. They must remain ordered: `RHGame::GameLoop` in
//! `original-code/RHgame.cpp` plays the mission-end jingle before changing
//! to menu sound, and changes back to mission sound afterward when the
//! debriefing requests a load. Independent last-write-wins mailboxes lose
//! that sequence.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Sound modes that game-flow transitions may request from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoundMode {
    Menu,
    Mission,
}

/// Mission-result jingles emitted by the game state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Jingle {
    MissionWon,
    MissionLost,
}

/// A typed request for an application-shell side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppEffect {
    SetSoundMode(SoundMode),
    PlayJingle(Jingle),
    SetMouseEnabled(bool),
}

/// FIFO queue of application effects waiting for the host executor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEffectQueue {
    pending: VecDeque<AppEffect>,
}

/// Failure returned while applying one effect from an [`AppEffectQueue`].
///
/// The failing effect remains at the front of the queue, together with all
/// later effects, so a caller can fix the host-side problem and retry without
/// losing or reordering requests.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppEffectExecutionError<E> {
    #[error(
        "failed to execute app effect {effect:?} after {executed} successful effect(s): {source}"
    )]
    Effect {
        effect: AppEffect,
        executed: usize,
        #[source]
        source: E,
    },
}

impl AppEffectQueue {
    pub fn push(&mut self, effect: AppEffect) {
        self.pending.push_back(effect);
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &AppEffect> {
        self.pending.iter()
    }

    /// Apply queued effects in insertion order.
    ///
    /// The callback must return `Err` only when it did not apply the effect.
    /// Successfully applied effects are removed immediately; a failed effect
    /// is retained at the front for a later retry.
    pub fn try_execute<E>(
        &mut self,
        mut execute: impl FnMut(AppEffect) -> Result<(), E>,
    ) -> Result<(), AppEffectExecutionError<E>> {
        let mut executed = 0;
        while let Some(effect) = self.pending.front().copied() {
            if let Err(source) = execute(effect) {
                return Err(AppEffectExecutionError::Effect {
                    effect,
                    executed,
                    source,
                });
            }
            self.pending.pop_front();
            executed += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executes_every_effect_in_fifo_order() {
        let effects = [
            AppEffect::PlayJingle(Jingle::MissionWon),
            AppEffect::SetSoundMode(SoundMode::Menu),
            AppEffect::SetSoundMode(SoundMode::Mission),
            AppEffect::SetMouseEnabled(true),
        ];
        let mut queue = AppEffectQueue::default();
        for effect in effects {
            queue.push(effect);
        }
        let mut executed = Vec::new();

        queue
            .try_execute(|effect| {
                executed.push(effect);
                Ok::<_, &'static str>(())
            })
            .expect("executor should accept every effect");

        assert_eq!(executed, effects);
        assert!(queue.is_empty());
    }

    #[test]
    fn error_retains_failed_effect_and_remaining_order() {
        let effects = [
            AppEffect::SetMouseEnabled(false),
            AppEffect::SetSoundMode(SoundMode::Menu),
            AppEffect::PlayJingle(Jingle::MissionLost),
        ];
        let mut queue = AppEffectQueue::default();
        for effect in effects {
            queue.push(effect);
        }

        let error = queue
            .try_execute(|effect| {
                if effect == AppEffect::SetSoundMode(SoundMode::Menu) {
                    Err("audio backend rejected mode")
                } else {
                    Ok(())
                }
            })
            .expect_err("the injected executor failure must be reported");

        assert_eq!(
            error,
            AppEffectExecutionError::Effect {
                effect: AppEffect::SetSoundMode(SoundMode::Menu),
                executed: 1,
                source: "audio backend rejected mode",
            }
        );
        assert_eq!(queue.iter().copied().collect::<Vec<_>>(), effects[1..]);

        let mut retried = Vec::new();
        queue
            .try_execute(|effect| {
                retried.push(effect);
                Ok::<_, &'static str>(())
            })
            .expect("retry should resume at the failed effect");
        assert_eq!(retried, effects[1..]);
        assert!(queue.is_empty());
    }
}

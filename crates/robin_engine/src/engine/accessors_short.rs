//! Short inherent accessors for the most common required-state lookups.
//!
//! Every owner-scoped accessor panics with the caller's context when the
//! entity or its required component is missing; none of them returns a
//! default. Each borrows the whole engine, so code that needs disjoint field
//! borrows keeps using the direct field path.

use super::EngineInner;
use crate::ai::AiController;
use crate::ai_enemy::EnemyAi;
use crate::element::{AiActorData, EntityId};
use crate::entities::Entities;
use crate::sequence::SequenceManager;

impl EngineInner {
    pub(crate) fn entities(&self) -> &Entities {
        &self.world.entities
    }

    pub(crate) fn entities_mut(&mut self) -> &mut Entities {
        &mut self.world.entities
    }

    pub(crate) fn seq(&self) -> &SequenceManager {
        &self.orders.sequence_manager
    }

    pub(crate) fn seq_mut(&mut self) -> &mut SequenceManager {
        &mut self.orders.sequence_manager
    }

    /// Required AI controller of `id`.
    #[track_caller]
    pub(crate) fn ai(&self, id: EntityId, context: &str) -> &AiController {
        self.world
            .entities
            .expect_ai_controller(id, format_args!("{context}"))
    }

    #[track_caller]
    pub(crate) fn ai_mut(&mut self, id: EntityId, context: &str) -> &mut AiController {
        self.world
            .entities
            .expect_ai_controller_mut(id, format_args!("{context}"))
    }

    /// Required enemy AI of `id`.
    #[track_caller]
    pub(crate) fn enemy_ai(&self, id: EntityId, context: &str) -> &EnemyAi {
        self.world
            .entities
            .expect_enemy_ai(id, format_args!("{context}"))
    }

    #[track_caller]
    pub(crate) fn enemy_ai_mut(&mut self, id: EntityId, context: &str) -> &mut EnemyAi {
        self.world
            .entities
            .expect_enemy_ai_mut(id, format_args!("{context}"))
    }

    /// Required NPC actor state of `id`.
    #[track_caller]
    pub(crate) fn ai_actor(&self, id: EntityId, context: &str) -> &AiActorData {
        self.world
            .entities
            .expect_ai_actor_data(id, format_args!("{context}"))
    }

    #[track_caller]
    pub(crate) fn ai_actor_mut(&mut self, id: EntityId, context: &str) -> &mut AiActorData {
        self.world
            .entities
            .expect_ai_actor_data_mut(id, format_args!("{context}"))
    }
}

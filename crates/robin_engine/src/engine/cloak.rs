//! Authoritative reusable-cloak command admission and runtime controls.

use crate::element::{ActionState, Command, Entity, Human, Posture};
use crate::entity_id::EntityId;
use crate::level_assets::LevelAssets;
use crate::order::OrderType;
use crate::player_command::PlayerCommand;
use crate::sequence::{Sequence, SequenceElement};

use super::EngineInner;

impl EngineInner {
    /// Resolve the cloak hotkey into ordinary, replayable per-actor commands.
    ///
    /// The host records these commands rather than a selection-dependent
    /// "toggle" token, so replay and multiplayer peers never have to
    /// reconstruct what was selected when the key was pressed.
    pub fn cloak_toggle_commands_for_seat(
        &self,
        seat: crate::player_command::PlayerId,
    ) -> Vec<PlayerCommand> {
        if !self.control.sim_config.reusable_cloaks {
            return Vec::new();
        }

        self.seat_selection(seat)
            .iter()
            .filter_map(|&actor| {
                let entity = self.world.entities.get(actor).unwrap_or_else(|| {
                    panic!("selected cloak actor {actor:?} is missing from the entity table")
                });
                let pc = entity
                    .as_pc()
                    .unwrap_or_else(|| panic!("selected cloak actor {actor:?} is not a PC"));
                if matches!(pc.element.posture, Posture::Cloaked | Posture::Spy) {
                    return Some(PlayerCommand::LaunchSelfAbility {
                        actor,
                        command: Command::LeaveSpy,
                    });
                }
                let may_enter = pc.element.active
                    && pc.pc.life_points > 0
                    && !pc.human.unconscious
                    && pc.element.posture == Posture::Upright
                    && pc.actor.action_state == ActionState::Waiting
                    && pc.element.sprite.has_animation(OrderType::WaitingCape)
                    && pc
                        .element
                        .sprite
                        .has_animation(OrderType::TransitionWaitingCapeWaitingUpright);
                may_enter.then_some(PlayerCommand::LaunchSelfAbility {
                    actor,
                    command: Command::EnterCloak,
                })
            })
            .collect()
    }

    /// Admit a recorded cloak entry against the current authoritative world.
    ///
    /// This is intentionally checked when the command is applied, not by the
    /// host UI: watched-state and modded animation availability are simulation
    /// facts and must agree on every replay/network peer.
    pub(crate) fn try_enter_reusable_cloak(
        &mut self,
        assets: &LevelAssets,
        actor: EntityId,
    ) -> bool {
        if !self.control.sim_config.reusable_cloaks {
            tracing::debug!(
                ?actor,
                "reusable cloak command ignored while feature is disabled"
            );
            return false;
        }

        let (target_camp, target_position) = {
            let Some(Entity::Pc(pc)) = self.world.entities.get(actor) else {
                tracing::error!(?actor, "reusable cloak command requires a live PC owner");
                return false;
            };
            if !pc.element.active
                || pc.pc.life_points <= 0
                || pc.human.unconscious
                || pc.element.posture != Posture::Upright
                || pc.actor.action_state != ActionState::Waiting
            {
                tracing::debug!(
                    ?actor,
                    posture = ?pc.element.posture,
                    action_state = ?pc.actor.action_state,
                    "reusable cloak command rejected by actor state"
                );
                return false;
            }
            let has_waiting = pc.element.sprite.has_animation(OrderType::WaitingCape);
            let has_transition = pc
                .element
                .sprite
                .has_animation(OrderType::TransitionWaitingCapeWaitingUpright);
            if !has_waiting || !has_transition {
                tracing::error!(
                    ?actor,
                    profile = %pc.element.sprite.frame_profile_name,
                    has_waiting,
                    has_transition,
                    "PC profile cannot use reusable cloaks because required Original cape art is missing"
                );
                return false;
            }
            (pc.pc.cached_camp, pc.element.position_map())
        };

        let viewers: Vec<EntityId> = self.world.entities.npc_ids().collect();
        for viewer_id in viewers {
            let hostile = self
                .world
                .entities
                .get(viewer_id)
                .is_some_and(|viewer| viewer.camp().is_hostile_to(target_camp));
            if !hostile {
                continue;
            }
            let close_scrutiny = self
                .world
                .entities
                .get(viewer_id)
                .filter(|viewer| {
                    viewer.is_active()
                        && !viewer.is_dead()
                        && viewer.human_data().is_some_and(|human| !human.unconscious)
                        && viewer
                            .npc_data()
                            .is_some_and(|npc| !npc.eye_status.is_blind())
                })
                .is_some_and(|viewer| {
                    let viewer_position = viewer.element_data().position_map();
                    let dx = viewer_position.x - target_position.x;
                    let dy = viewer_position.y - target_position.y;
                    dx * dx + dy * dy
                        <= crate::cloak::DIRECT_SCRUTINY_RADIUS
                            * crate::cloak::DIRECT_SCRUTINY_RADIUS
                });
            let watched = close_scrutiny
                || self.npc_is_detecting_human(
                    assets,
                    viewer_id,
                    actor,
                    self.control.frame_counter,
                );
            if watched {
                tracing::debug!(
                    ?actor,
                    ?viewer_id,
                    close_scrutiny,
                    "reusable cloak cannot be donned while watched"
                );
                return false;
            }
        }

        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new(1, Command::EnterCloak, Some(actor)));
        self.launch_sequence(sequence);
        true
    }

    pub(crate) fn set_reusable_cloaks_enabled(&mut self, enabled: bool) {
        if self.control.sim_config.reusable_cloaks == enabled {
            return;
        }
        self.control.sim_config.reusable_cloaks = enabled;
        if enabled {
            return;
        }

        // Disabling a gameplay feature must leave no invisible extension
        // state behind. Use the normal transition command so the cape art,
        // hidden marker, replay state and posture all unwind identically.
        let cloaked: Vec<EntityId> = self
            .world
            .entities
            .pcs()
            .filter_map(|(id, pc)| (pc.element.posture == Posture::Cloaked).then_some(id.into()))
            .collect();
        for actor in cloaked {
            let mut sequence = Sequence::new();
            sequence.append_element(SequenceElement::new(1, Command::LeaveSpy, Some(actor)));
            self.launch_sequence(sequence);
        }
    }
}

//! Authoritative reusable-cloak command admission and runtime controls.

use crate::element::{ActionState, Command, Entity, Posture};
use crate::entity_id::EntityId;
use crate::order::OrderType;
use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
use crate::sequence::{Sequence, SequenceElement};

use super::{EngineInner, LevelAssets};

impl EngineInner {
    /// Validate the feature-specific authority rules at the same deterministic
    /// frame boundary on live, replay, rollback, and multiplayer paths.
    pub(crate) fn reusable_cloak_command_is_authorized(
        &self,
        input: &PlayerInput,
        seat: usize,
    ) -> bool {
        let original_parity = self.control.rng.original_replay_cursor().is_some();
        match &input.command {
            PlayerCommand::SetReusableCloaks { enabled } => {
                if input.player_id != PlayerId::HOST {
                    tracing::warn!(
                        player_id = ?input.player_id,
                        "non-host seat cannot change reusable-cloak simulation policy"
                    );
                    return false;
                }
                if original_parity && *enabled {
                    tracing::warn!(
                        "Original parity playback cannot enable reusable-cloak mechanics"
                    );
                    return false;
                }
                true
            }
            PlayerCommand::LaunchSelfAbility {
                actor,
                command: Command::EnterCloak,
            } => {
                if original_parity {
                    tracing::warn!(
                        ?actor,
                        "Original parity playback rejected reusable-cloak entry"
                    );
                    return false;
                }
                self.players.seats[seat].selection.contains(actor)
            }
            PlayerCommand::LaunchSelfAbility {
                actor,
                command: Command::LeaveSpy,
            } if self
                .world
                .entities
                .get(*actor)
                .is_some_and(|entity| entity.element_data().posture == Posture::Cloaked) =>
            {
                self.players.seats[seat].selection.contains(actor)
            }
            _ => true,
        }
    }

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

        self.hero_selection(seat)
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

/// Original-RNG parity is an engine construction mode, not a caller
/// convention. Normalize the deterministic identity before mission setup so
/// no tool can accidentally run additive cloak behavior in an Original trace.
///
/// TODO(ranked-sessions): this rolling line has no ranked-session mode or
/// policy surface. If one is introduced, its constructor must apply the same
/// normalization before the starting snapshot is hashed.
pub(crate) fn preserve_original_cloak_behavior(mut config: super::SimConfig) -> super::SimConfig {
    config.reusable_cloaks = false;
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ActorPc, ElementData, ElementKind};

    fn selected_pc(engine: &mut EngineInner) -> EntityId {
        let actor = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        }));
        engine.players.seats[0].selection.push(actor);
        actor
    }

    #[test]
    fn global_cloak_setting_is_host_authoritative() {
        let mut engine = EngineInner::new();
        let remote_seat = engine.ensure_seat(PlayerId(1));
        let remote = PlayerInput::new(
            PlayerId(1),
            PlayerCommand::SetReusableCloaks { enabled: false },
        );
        assert!(!engine.reusable_cloak_command_is_authorized(&remote, remote_seat));

        let host = PlayerInput::host(PlayerCommand::SetReusableCloaks { enabled: false });
        assert!(engine.reusable_cloak_command_is_authorized(&host, 0));
    }

    #[test]
    fn explicit_cloak_commands_require_the_issuing_seat_selection() {
        let mut engine = EngineInner::new();
        let actor = selected_pc(&mut engine);
        let host = PlayerInput::host(PlayerCommand::LaunchSelfAbility {
            actor,
            command: Command::EnterCloak,
        });
        assert!(engine.reusable_cloak_command_is_authorized(&host, 0));

        let remote_seat = engine.ensure_seat(PlayerId(1));
        let remote = PlayerInput::new(PlayerId(1), host.command.clone());
        assert!(!engine.reusable_cloak_command_is_authorized(&remote, remote_seat));
    }

    #[test]
    fn original_parity_normalizes_and_seals_reusable_cloaks_off() {
        let mut engine = EngineInner::new();
        engine.control.rng = super::super::SimulationRng::with_original_replay(Vec::new());
        let actor = selected_pc(&mut engine);
        let enter = PlayerInput::host(PlayerCommand::LaunchSelfAbility {
            actor,
            command: Command::EnterCloak,
        });
        let enable = PlayerInput::host(PlayerCommand::SetReusableCloaks { enabled: true });
        assert!(!engine.reusable_cloak_command_is_authorized(&enter, 0));
        assert!(!engine.reusable_cloak_command_is_authorized(&enable, 0));
        assert!(
            !preserve_original_cloak_behavior(super::super::SimConfig::default()).reusable_cloaks
        );
    }

    #[test]
    fn disabling_the_feature_unwinds_every_active_cloak() {
        let mut engine = EngineInner::new();
        let actor = selected_pc(&mut engine);
        engine
            .world
            .entities
            .get_mut(actor)
            .expect("test PC exists")
            .element_data_mut()
            .posture = Posture::Cloaked;

        engine.set_reusable_cloaks_enabled(false);

        assert!(!engine.control.sim_config.reusable_cloaks);
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("disabling launches the normal cape-removal transition");
        assert_eq!(sequence.elements.len(), 1);
        assert_eq!(sequence.elements[0].command, Command::LeaveSpy);
        assert_eq!(sequence.elements[0].owner, Some(actor));
    }
}

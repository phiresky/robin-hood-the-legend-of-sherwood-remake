//! Shared element, actor and human behavior.
use super::*;

/// Base trait for all game entities.
pub trait Element {
    fn element_data(&self) -> &ElementData;
    fn element_data_mut(&mut self) -> &mut ElementData;

    fn kind(&self) -> ElementKind {
        self.element_data().kind
    }
    fn is_active(&self) -> bool {
        self.element_data().active
    }
    fn is_blipped(&self) -> bool {
        self.element_data().blipped
    }
    fn is_unreachable(&self) -> bool {
        self.element_data().unreachable
    }
    #[must_use = "method returns WorldPoint3D by value; `elem.position().x = v` silently modifies a temporary. Use `set_position` to mutate."]
    fn position(&self) -> WorldPoint3D {
        self.element_data().position()
    }
    #[must_use = "method returns MapPoint by value; `elem.position_map().x = v` silently modifies a temporary. Use `set_position_map` to mutate."]
    fn position_map(&self) -> MapPoint {
        self.element_data().position_map()
    }
    #[must_use]
    fn direction(&self) -> i16 {
        self.element_data().direction()
    }
    fn posture(&self) -> Posture {
        self.element_data().posture
    }
    /// Set posture through the corpse-transition guard.  Delegates to
    /// [`ElementData::set_posture`].
    fn set_posture(&mut self, p: Posture) {
        self.element_data_mut().set_posture(p);
    }
    fn class_id(&self) -> u16 {
        self.element_data().class_id
    }
    fn is_in_honolulu(&self) -> bool {
        self.element_data().in_honolulu
    }

    // Shared behavior with default implementations
    fn is_immortal(&self) -> bool {
        false
    }
    fn is_engine_locked(&self) -> bool {
        false
    }
    fn is_transporting(&self) -> bool {
        false
    }
    fn is_dead(&self) -> bool {
        true
    }
    fn is_obviously_hostile(&self) -> bool {
        false
    }

    /// Per-frame update tick. Returns false if the entity should be removed.
    fn hourglass(&mut self) -> bool {
        true
    }
}

/// Trait for actor entities.
pub trait Actor: Element {
    fn actor_data(&self) -> &ActorData;
    fn actor_data_mut(&mut self) -> &mut ActorData;

    fn action_state(&self) -> ActionState {
        self.actor_data().action_state
    }
    fn wait_time(&self) -> u32 {
        self.actor_data().wait_time
    }
    fn is_execution_frozen(&self) -> bool {
        self.actor_data().execution_frozen
    }
    fn is_tied(&self) -> bool {
        self.posture() == Posture::Tied
    }
    /// Motion query provided by the base trait since every caller is
    /// a PC/human and the human override strictly subsumes the actor
    /// version.  Returns true if the sprite is translating toward a
    /// non-zero goal OR has actually moved on the map between
    /// frames.
    fn is_in_motion(&self) -> bool {
        let pi = &self.element_data().sprite.position_iface;
        let goal = pi.map_goal();
        let pos = pi.map_position();
        (goal != pos && goal != MapPoint::ZERO) || pi.is_moving_map()
    }
    fn is_ignored(&self) -> bool {
        false
    }
}

/// Trait for human actors.
pub trait Human: Actor {
    fn human_data(&self) -> &HumanData;
    fn human_data_mut(&mut self) -> &mut HumanData;

    /// Life point source — must be provided by each concrete type.
    fn life_points(&self) -> i16;
    fn max_life_points(&self) -> i16;
    fn camp(&self) -> Camp;

    fn is_unconscious(&self) -> bool {
        self.human_data().unconscious
    }
    /// OR-combine death / unconsciousness / net entrapment /
    /// `posture == Tied` / `posture == Carried` /
    /// PC identity combined with coma.
    ///
    /// The coma branch isn't reachable here because `in_coma`
    /// lives on `Campaign` (`PcStatus`), not on `PcData` — callers
    /// that need the coma arm must compose it externally (see
    /// `ai_entity_view.rs` for the campaign-aware path).
    fn is_out_of_order(&self) -> bool {
        self.life_points() <= 0
            || self.is_unconscious()
            || self.is_stuck_under_net()
            || matches!(self.posture(), Posture::Tied | Posture::Carried)
    }
    fn concussion(&self) -> u16 {
        self.human_data().concussion_of_the_brain
    }
    fn is_invulnerable(&self) -> bool {
        self.human_data().invulnerable
    }
    fn is_carried(&self) -> bool {
        self.human_data().carrier.is_some()
    }
    fn is_stuck_under_net(&self) -> bool {
        self.human_data().stuck_under_nets_counter > 0
    }
    fn is_hollow_man(&self) -> bool {
        self.human_data().hollow_man
    }
    fn is_killed_by_accident(&self) -> bool {
        self.human_data().killed_by_accident
    }
    fn is_holding_shield(&self) -> bool {
        self.action_state().is_shield()
    }
    fn tiredness(&self) -> u16 {
        self.human_data().tiredness
    }
    fn is_robin(&self) -> bool {
        false
    }
    fn is_able_to_fight(&self) -> bool {
        false
    }
    fn is_able_to_help(&self) -> bool {
        false
    }
    fn fighting_ability(&self) -> u16 {
        0
    }
    fn shooting_ability(&self) -> u16 {
        0
    }
    fn endurance(&self) -> u16 {
        0
    }

    /// Whether this human is a credible menacer of `prisoner_pos`.
    ///
    /// Gates on the current `action_state` being one of `Waiting` /
    /// `AimingWithBow` / `Moving` / `MovingFast`, then dot-products
    /// the prisoner offset against the menacer's facing direction
    /// (positive = in front).
    ///
    /// No in-tree callers yet (this is vestigial); the body is
    /// provided so the first caller gets the right behaviour rather
    /// than the ad-hoc `true`/`false` defaults that used to live on
    /// `Element`/`ActorSoldier`.
    fn is_dangerous_as_menacer(&self, prisoner_pos: MapPoint) -> bool {
        match self.action_state() {
            ActionState::Waiting
            | ActionState::AimingWithBow
            | ActionState::Moving
            | ActionState::MovingFast => {}
            _ => return false,
        }
        let here = self.position_map();
        let dx = prisoner_pos.x - here.x;
        let dy = prisoner_pos.y - here.y;
        let [vx, vy] = crate::position_interface::sector_to_vector_iso(self.direction());
        dx * vx + dy * vy > 0.0
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Trait implementations
// ═══════════════════════════════════════════════════════════════════

macro_rules! impl_element_data {
    ($ty:ty) => {
        impl crate::element::Element for $ty {
            fn element_data(&self) -> &ElementData {
                &self.element
            }
            fn element_data_mut(&mut self) -> &mut ElementData {
                &mut self.element
            }
        }
    };
}

macro_rules! impl_actor_data {
    ($ty:ty) => {
        impl crate::element::Actor for $ty {
            fn actor_data(&self) -> &ActorData {
                &self.actor
            }
            fn actor_data_mut(&mut self) -> &mut ActorData {
                &mut self.actor
            }
        }
    };
}

// -- Element trait for all concrete types --

impl Element for ActorPc {
    fn element_data(&self) -> &ElementData {
        &self.element
    }
    fn element_data_mut(&mut self) -> &mut ElementData {
        &mut self.element
    }
    fn is_immortal(&self) -> bool {
        self.pc.immortal
    }
    fn is_transporting(&self) -> bool {
        self.posture() == Posture::CarryingCorpse
    }
    fn is_dead(&self) -> bool {
        self.pc.life_points <= 0
    }
    fn is_obviously_hostile(&self) -> bool {
        true
    }
}

impl Element for ActorSoldier {
    fn element_data(&self) -> &ElementData {
        &self.element
    }
    fn element_data_mut(&mut self) -> &mut ElementData {
        &mut self.element
    }
    fn is_dead(&self) -> bool {
        self.npc.life_points <= 0
    }
}

impl Element for ActorCivilian {
    fn element_data(&self) -> &ElementData {
        &self.element
    }
    fn element_data_mut(&mut self) -> &mut ElementData {
        &mut self.element
    }
    fn is_dead(&self) -> bool {
        self.npc.life_points <= 0
    }
}

impl_element_data!(ElementFx);
impl_element_data!(ElementTarget);
impl_element_data!(ElementBonus);
impl_element_data!(ElementProjectile);
impl_element_data!(ElementNet);

// -- Actor trait for actor types --

impl_actor_data!(ActorPc);
impl_actor_data!(ActorSoldier);
impl_actor_data!(ActorCivilian);

// -- Human trait for human types --

impl Human for ActorPc {
    fn human_data(&self) -> &HumanData {
        &self.human
    }
    fn human_data_mut(&mut self) -> &mut HumanData {
        &mut self.human
    }
    fn life_points(&self) -> i16 {
        self.pc.life_points
    }
    fn max_life_points(&self) -> i16 {
        crate::pc_status::LIFEPOINTS_PC
    }
    fn camp(&self) -> Camp {
        self.pc.cached_camp
    }
    fn is_robin(&self) -> bool {
        self.pc.robin
    }
    /// Guards on dead/unconscious/inactive, then returns false for
    /// disguised postures (`Tree`, `Spy`).
    fn is_able_to_fight(&self) -> bool {
        if self.pc.life_points <= 0 || self.is_unconscious() || !self.is_active() {
            return false;
        }
        !matches!(self.posture(), Posture::Tree | Posture::Spy)
    }
}

impl Human for ActorSoldier {
    fn human_data(&self) -> &HumanData {
        &self.human
    }
    fn human_data_mut(&mut self) -> &mut HumanData {
        &mut self.human
    }
    fn life_points(&self) -> i16 {
        self.npc.life_points
    }
    fn max_life_points(&self) -> i16 {
        // Level initialization already applies the difficulty modifier when
        // populating this cache. Scaling again here inflated Lacklandist HP.
        self.soldier.cached_max_life_points
    }
    fn camp(&self) -> Camp {
        self.soldier.cached_camp
    }
    /// Two layers: an early-false block (dead / unconscious / tied /
    /// carried / inactive), then a state-machine switch where
    /// `Sleeping`, `Menacing`, `Fleeing`, and the three hit-stun
    /// `Attacking` substates all return false.
    fn is_able_to_fight(&self) -> bool {
        if self.npc.life_points <= 0
            || self.is_unconscious()
            || self.is_tied()
            || self.is_carried()
            || !self.is_active()
        {
            return false;
        }
        match self.npc.ai_state() {
            AiTopState::Sleeping | AiTopState::Menacing | AiTopState::Fleeing => false,
            AiTopState::Default | AiTopState::Wondering | AiTopState::Seeking => true,
            AiTopState::Attacking => !matches!(
                self.npc.ai_substate(),
                AiSubstate::AttackingGotHit
                    | AiSubstate::AttackingGotHitStandingUp
                    | AiSubstate::AttackingHitting,
            ),
        }
    }
    /// Reject dead/unconscious, then state-machine:
    /// Default/Wondering → true; Seeking restricted to officer-report
    /// and reaction-time substates; Sleeping/Menacing/Fleeing/Attacking
    /// → false. Note: unlike `is_able_to_fight`, this predicate does
    /// NOT gate on tied/carried/inactive.
    fn is_able_to_help(&self) -> bool {
        if self.npc.life_points <= 0 || self.is_unconscious() {
            return false;
        }
        match self.npc.ai_state() {
            AiTopState::Default | AiTopState::Wondering => true,
            AiTopState::Seeking => matches!(
                self.npc.ai_substate(),
                AiSubstate::SeekingSoldierGiveReportToOfficer
                    | AiSubstate::SeekingSoldierGiveAlertingReportToOfficerStart
                    | AiSubstate::SeekingSoldierGiveAlertingReportToOfficerPoint
                    | AiSubstate::SeekingSoldierGiveAlertingReportToOfficerEnd
                    | AiSubstate::SeekingRunningToOfficer
                    | AiSubstate::SeekingRunningToOfficerSeen
                    | AiSubstate::SeekingHeardstepsReactiontime
                    | AiSubstate::SeekingBodyReactiontime
            ),
            AiTopState::Sleeping
            | AiTopState::Menacing
            | AiTopState::Fleeing
            | AiTopState::Attacking => false,
        }
    }
}

impl Human for ActorCivilian {
    fn human_data(&self) -> &HumanData {
        &self.human
    }
    fn human_data_mut(&mut self) -> &mut HumanData {
        &mut self.human
    }
    fn life_points(&self) -> i16 {
        self.npc.life_points
    }
    fn max_life_points(&self) -> i16 {
        crate::pc_status::LIFEPOINTS_PC
    } // civilians always 100
    fn camp(&self) -> Camp {
        self.civilian.cached_camp
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Concrete-type convenience methods
// ═══════════════════════════════════════════════════════════════════

//! Rules for the optional reusable-cloak gameplay extension.
//!
//! The shipped game already has the complete visual vocabulary: PCs can be
//! authored in `WaitingCape` (order 134 / `Posture::Spy`) and leave it with
//! `TransitionWaitingCapeWaitingUpright` (order 135).  Reusable cloaks play
//! that transition backwards and settle in the distinct
//! [`crate::element::Posture::Cloaked`] posture.  Keeping the new posture
//! separate is important: mission-authored `Spy` actors retain the Original's
//! absolute invisibility, while a player-donned cloak is a deception that an
//! already-alert observer or somebody at arm's length can see through.

use serde::{Deserialize, Serialize};

use crate::element::Command;

/// Maximum planar distance at which an otherwise-unaware hostile scrutinises
/// a player-donned cloak closely enough to see through it.
///
/// This matches the scale of the Original's immediate-contact boxes (30 map
/// units on their short axis) without inventing a new long-range perception
/// ability.
pub const DIRECT_SCRUTINY_RADIUS: f32 = 30.0;

/// Shipped content has no authored exception to reusable-cloak deception.
///
/// `original-code/RHelementactornpc.cpp::ComputeVisibility(Human*)` treats
/// every `RHPOSTURE_SPY` target alike, while `original-code/RHElement.h` and
/// `RHelementactoranimal.h` document the animal runtime as dead in shipped
/// missions. No character or mission profile field supplies a special sense.
/// Keep every production query on this conservative value until concrete
/// modded authoring data exists.
pub const SHIPPED_AUTHORED_DETECTOR: bool = false;

/// Complete, explicit policy input for one hostile looking at a cloaked PC.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CloakObservation {
    /// Whether this observer is hostile to the PC. Allies are never deceived.
    pub hostile: bool,
    /// The observer's retained enemy list / active primary target still
    /// contains this PC.
    pub remembers_target: bool,
    /// Authored special detector override.
    ///
    /// TODO(cloak-authoring): expose this through character/mission profile
    /// data only when a modded schema supplies an explicit special sense. The
    /// Original has no such profile field, and its animal runtime is dead in
    /// shipped missions, so production callers use
    /// [`SHIPPED_AUTHORED_DETECTOR`] rather than inventing class exceptions.
    pub authored_detector: bool,
    /// Squared planar distance between observer and target detection points.
    pub distance_squared: f32,
}

/// Whether the cloak successfully deceives this observer before ordinary
/// cone, range, elevation, blindness, and opaque-LOS tests run.
pub fn deceives_observer(observation: CloakObservation) -> bool {
    observation.hostile
        && !observation.remembers_target
        && !observation.authored_detector
        && observation.distance_squared > DIRECT_SCRUTINY_RADIUS * DIRECT_SCRUTINY_RADIUS
}

/// Commands which may remain selected while wearing a reusable cloak.
///
/// Cape art only supplies a stationary waiting pose. Every real actor action
/// therefore reveals the PC by inserting the normal cape-to-upright
/// transition first. `Wait`/`WaitTimer` are the only idle bodies that retain
/// it; `EnterCloak` and `LeaveSpy` are the transition commands themselves.
pub fn command_breaks_cloak(command: Command) -> bool {
    !matches!(
        command,
        Command::Wait | Command::WaitTimer | Command::EnterCloak | Command::LeaveSpy
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> CloakObservation {
        CloakObservation {
            hostile: true,
            remembers_target: false,
            authored_detector: false,
            distance_squared: (DIRECT_SCRUTINY_RADIUS + 1.0).powi(2),
        }
    }

    #[test]
    fn unaware_distant_hostile_is_deceived() {
        assert!(deceives_observer(observation()));
    }

    #[test]
    fn allies_memory_special_senses_and_scrutiny_are_explicit_exceptions() {
        let base = observation();
        assert!(!deceives_observer(CloakObservation {
            hostile: false,
            ..base
        }));
        assert!(!deceives_observer(CloakObservation {
            remembers_target: true,
            ..base
        }));
        assert!(!deceives_observer(CloakObservation {
            authored_detector: true,
            ..base
        }));
        assert!(!deceives_observer(CloakObservation {
            distance_squared: DIRECT_SCRUTINY_RADIUS.powi(2),
            ..base
        }));
    }

    #[test]
    fn shipped_content_has_no_invented_special_detector() {
        assert!(!SHIPPED_AUTHORED_DETECTOR);
    }

    #[test]
    fn any_non_idle_actor_command_breaks_the_stationary_disguise() {
        assert!(!command_breaks_cloak(Command::Wait));
        assert!(!command_breaks_cloak(Command::WaitTimer));
        assert!(!command_breaks_cloak(Command::EnterCloak));
        assert!(!command_breaks_cloak(Command::LeaveSpy));
        assert!(command_breaks_cloak(Command::Move));
        assert!(command_breaks_cloak(Command::TieCmd));
        assert!(command_breaks_cloak(Command::Untie));
        assert!(command_breaks_cloak(Command::ShootBow));
    }
}

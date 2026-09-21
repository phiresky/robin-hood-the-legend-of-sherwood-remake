//! Deterministic startup shared by live sessions and ranked replay loading.

use super::{Engine, FrameAdvanceError, LevelAssets, SimCommand, SimulationFrameInput};
use crate::character_kind::CharacterKind;
use crate::player_command::{PlayerCommand, PlayerId};
use crate::sim_rng::{self, AuxiliaryRngSite};

impl Engine {
    /// Register generated Merry Men names before replay frame zero. Display
    /// slots retain their names, while registrations become campaign identity.
    /// The auxiliary generator never advances the simulation RNG.
    pub fn register_mission_peasant_names(
        &mut self,
        assets: &LevelAssets,
        localized_names: &mut [Option<String>; CharacterKind::COUNT],
    ) -> Result<(), FrameAdvanceError> {
        const MAX_ATTEMPTS: usize = 10;
        let firstnames = &assets.peasant_firstnames;
        let surnames = &assets.peasant_surnames;
        if firstnames.is_empty() || surnames.is_empty() {
            tracing::warn!(
                "Peasant name generation: no firstname/surname strings found ({}/{})",
                firstnames.len(),
                surnames.len(),
            );
            return Ok(());
        }
        sim_rng::with_auxiliary_seed(AuxiliaryRngSite::PeasantNames, self.rng_seed(), |rng| {
            for kind in [
                CharacterKind::MerryManA,
                CharacterKind::MerryManB,
                CharacterKind::MerryManC,
            ] {
                let slot = kind.as_index();
                if localized_names[slot].is_some() {
                    continue;
                }
                let mut generated = None;
                for _ in 0..MAX_ATTEMPTS {
                    let first = &firstnames[rng.u64(0..firstnames.len() as u64) as usize];
                    let last = &surnames[rng.u64(0..surnames.len() as u64) as usize];
                    let full = format!("{first} {last}");
                    if !self.is_peasant_name_registered(&full) {
                        self.advance_frame(
                            assets,
                            SimulationFrameInput::new(vec![SimCommand::from(
                                PlayerCommand::RegisterPeasantName { name: full.clone() },
                            )])
                            .with_hourglass(false),
                        )?;
                        generated = Some(full);
                        break;
                    }
                }
                // An exhausted pool supplies a display label only; it must not
                // fabricate another campaign registration.
                let display_name = generated.unwrap_or_else(|| "Misteryman".to_owned());
                tracing::debug!("Peasant {kind:?} → {display_name:?}");
                localized_names[slot] = Some(display_name);
            }
            Ok(())
        })
    }

    /// Apply a seat connection at the startup boundary without advancing the
    /// hourglass. This is the same `ConnectSeat` command used during normal
    /// gameplay; callers use it only when constructing a mission snapshot.
    pub fn connect_seat(
        &mut self,
        assets: &LevelAssets,
        player_id: PlayerId,
        nickname: String,
    ) -> Result<(), FrameAdvanceError> {
        self.advance_frame(
            assets,
            SimulationFrameInput::new(vec![SimCommand::from(PlayerCommand::ConnectSeat {
                player_id,
                nickname,
            })])
            .with_hourglass(false),
        )?;
        Ok(())
    }
}

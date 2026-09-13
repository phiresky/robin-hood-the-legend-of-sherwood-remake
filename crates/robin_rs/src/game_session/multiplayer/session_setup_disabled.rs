//! `setup_multiplayer_session` without the `multiplayer` feature: a launch
//! that asks for multiplayer fails instead of silently starting a local game.

use super::SessionSetupFailure;
use crate::host::Host;

pub(super) async fn establish(
    _host: &mut Host,
    args: &crate::main_entry::MissionLaunch,
    _authoritative_mission_id: &str,
    _authoritative_rng_seed: u64,
    _authoritative_sim_config: robin_engine::engine::SimConfig,
    _campaign: &crate::multiplayer::MultiplayerCampaignSession,
) -> Result<(), SessionSetupFailure> {
    if args.server || args.connect.is_some() || args.join.is_some() {
        return Err(SessionSetupFailure::Unavailable(
            "multiplayer was requested but is unavailable in this build; rebuild with `--features multiplayer`",
        ));
    }
    Ok(())
}

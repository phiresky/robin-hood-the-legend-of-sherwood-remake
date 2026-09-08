//! Diagnostic persistence has no engine-application or replay-event authority.
//! Allocation and publication failures remain distinct caller-visible outcomes.

use crate::savegame::SaveGameManager;
use robin_engine::{engine as engine_api, profiles::ProfileManager};

pub(super) enum DiagnosticTarget {
    Existing(usize),
    New(&'static str),
}

pub(super) enum DiagnosticFailure {
    Allocation(anyhow::Error),
    Publication(anyhow::Error),
}

pub(super) fn diagnostic(
    target: DiagnosticTarget,
    manager: &mut SaveGameManager,
    host: &mut crate::host::Host,
    game: &crate::game::Game,
    engine: &engine_api::Engine,
    mission_id: u32,
    profiles: &ProfileManager,
    thumbnail: Option<&crate::save_file::Thumbnail>,
) -> Result<usize, DiagnosticFailure> {
    let index = match target {
        DiagnosticTarget::Existing(index) => index,
        DiagnosticTarget::New(label) => manager
            .create_draft(label.into(), mission_id)
            .and_then(|handle| manager.resolve_handle(&handle))
            .map_err(DiagnosticFailure::Allocation)?,
    };
    manager
        .write_multiplayer_diagnostic_from_engine(
            host,
            game,
            index,
            engine,
            mission_id,
            Some(profiles),
            thumbnail,
        )
        .and_then(|()| manager.save_index().map_err(anyhow::Error::msg))
        .map_err(DiagnosticFailure::Publication)?;
    Ok(index)
}

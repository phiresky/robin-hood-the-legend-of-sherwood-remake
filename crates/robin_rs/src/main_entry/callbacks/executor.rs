//! Staged save/load execution after the callback consume/publication barrier.

use super::{
    AutosaveNotices, OperationCompletion, OperationOutcome, PendingLevelLoad, PreparedLoad,
    SaveBannerKind, SaveLoadEvent, SaveLoadRequest, begin_multiplayer_snapshot_transition,
    current_mission_id, replay_save_written_event,
};
use crate::save_file::special_slots;
use crate::savegame::{SaveGameManager, SpecialSlot};
use robin_engine::{engine as engine_api, game_operation::GameCode, profiles::ProfileManager};

mod load;
mod persistence;

pub(super) fn execute(
    request: SaveLoadRequest,
    save_manager: &mut SaveGameManager,
    notices: &mut AutosaveNotices,
    host: &mut crate::host::Host,
    game: &mut crate::game::Game,
    engine: &mut engine_api::Engine,
    assets: &engine_api::LevelAssets,
    profiles: &ProfileManager,
    thumb_ref: Option<&crate::save_file::Thumbnail>,
) -> OperationOutcome {
    let mut outcome = OperationOutcome::NO_EVENT;
    let mut event = None;
    match request {
        SaveLoadRequest::Save { slot, mission_id } => {
            let slot = match slot
                .as_ref()
                .map(|handle| save_manager.resolve_handle(handle))
                .transpose()
            {
                Ok(slot) => slot,
                Err(error) => {
                    tracing::error!("Save rejected stale slot handle: {error:#}");
                    outcome.banner = Some(SaveBannerKind::SaveFailed);
                    return outcome;
                }
            };
            if host.transport.net.is_some() {
                let target = slot
                    .map(persistence::DiagnosticTarget::Existing)
                    .unwrap_or(persistence::DiagnosticTarget::New("Multiplayer diagnostic"));
                match persistence::diagnostic(
                    target,
                    save_manager,
                    host,
                    game,
                    engine,
                    mission_id,
                    profiles,
                    thumb_ref,
                ) {
                    Ok(idx) => {
                        outcome.banner = Some(SaveBannerKind::Saved);
                        tracing::info!(
                            slot = idx,
                            mission_id,
                            "multiplayer diagnostic save written locally"
                        );
                    }
                    Err(persistence::DiagnosticFailure::Allocation(error)) => {
                        tracing::error!("Multiplayer diagnostic draft failed: {error:#}");
                        outcome.banner = Some(SaveBannerKind::SaveFailed);
                    }
                    Err(persistence::DiagnosticFailure::Publication(error)) => {
                        tracing::error!("Multiplayer diagnostic save failed: {error:#}");
                        outcome.banner = Some(SaveBannerKind::SaveFailed);
                    }
                }
                return outcome;
            }
            // `slot = None` ⇒ auto Continue-save.
            // `slot = Some(idx)` ⇒ player-chosen slot.
            let (result, explicit_slot) = match slot {
                Some(idx) => (
                    save_manager
                        .write_save_from_engine(
                            host,
                            game,
                            idx,
                            engine,
                            mission_id,
                            Some(profiles),
                            thumb_ref,
                        )
                        .map(|_committed| ()),
                    true,
                ),
                None => (
                    save_manager.write_continue_save(
                        host,
                        game,
                        engine,
                        mission_id,
                        Some(profiles),
                        thumb_ref,
                    ),
                    false,
                ),
            };
            if let Err(err) = result {
                tracing::error!("Save failed: {err:#}");
                outcome.banner = Some(SaveBannerKind::SaveFailed);
            } else {
                tracing::info!("Save completed (mission={mission_id})");
                event = replay_save_written_event(engine, host, game);
                // Mirror the manual save into the Continue slot. The
                // guard keeps Continue→Continue copies from clobbering
                // themselves; Restart / Sherwood slots also skip the
                // mirror and the banner branch.
                if explicit_slot {
                    let is_special = slot
                        .and_then(|idx| save_manager.get(idx))
                        .and_then(|s| s.special);
                    let is_continue_or_restart = matches!(
                        is_special,
                        Some(SpecialSlot::Continue) | Some(SpecialSlot::Restart)
                    );
                    if !is_continue_or_restart
                        && let Err(err) = save_manager.write_continue_save(
                            host,
                            game,
                            engine,
                            mission_id,
                            Some(profiles),
                            thumb_ref,
                        )
                    {
                        tracing::warn!("Continue-mirror after save failed: {err:#}");
                        notices.enqueue_save_failed(format!("Continue mirror: {err:#}"));
                    }
                    // Show "Game saved." banner unless the slot is one
                    // of the filtered types (Restart / Sherwood).
                    let is_sherwood = matches!(is_special, Some(SpecialSlot::Sherwood));
                    if !is_continue_or_restart && !is_sherwood {
                        outcome.banner = Some(SaveBannerKind::Saved);
                    }
                }
            }
        }
        request @ (SaveLoadRequest::Load { .. } | SaveLoadRequest::ApplyLoad(_)) => {
            // If the save targets a different mission than the one currently
            // running, stash a `PendingLevelLoad` and let the session loop
            // switch missions before re-applying. This replaces the previous
            // warn-and-apply behaviour, which corrupted engine state when
            // the payload's mission didn't match the active level.
            let resolved = match match request {
                SaveLoadRequest::Load { slot, .. } => PreparedLoad::preflight(save_manager, slot),
                SaveLoadRequest::ApplyLoad(load) => {
                    load.validate_slot(save_manager).map(|()| Some(load))
                }
                _ => unreachable!("load request pattern"),
            } {
                Ok(resolved) => resolved,
                Err(error) => {
                    tracing::error!("Load preflight failed: {error:#}");
                    return outcome;
                }
            };
            match resolved {
                Some(save) => {
                    return execute_load(
                        save,
                        load::LoadCompletion::Selected(None),
                        save_manager,
                        host,
                        game,
                        engine,
                        assets,
                        profiles,
                        thumb_ref,
                    );
                }
                None => tracing::warn!("Load requested but no matching save slot found"),
            }
        }
        SaveLoadRequest::Restart => {
            let campaign = engine.campaign();
            let mid = current_mission_id(campaign, profiles);
            if let Err(err) =
                save_manager.write_restart_save(host, game, engine, mid, Some(profiles), thumb_ref)
            {
                tracing::error!("Restart save failed: {err:#}");
                outcome.banner = Some(SaveBannerKind::SaveFailed);
            } else {
                event = match save_manager.restart_session_identity() {
                    Some(identity) => Some(SaveLoadEvent::SaveWritten { identity }),
                    None => replay_save_written_event(engine, host, game),
                };
            }
        }
        SaveLoadRequest::LoadRestart => {
            match PreparedLoad::restart(save_manager) {
                Ok(Some(save)) => {
                    return execute_load(
                        save,
                        load::LoadCompletion::Restart,
                        save_manager,
                        host,
                        game,
                        engine,
                        assets,
                        profiles,
                        thumb_ref,
                    );
                }
                missing => {
                    tracing::error!("Restart snapshot unavailable: {missing:?}");
                    // Multiplayer cannot unilaterally fall back to LevelRestart.
                    if host.transport.net.is_none() {
                        outcome.completion = OperationCompletion::RestartRequested;
                        game.operation.set(GameCode::LevelRestart);
                    }
                }
            }
        }
        SaveLoadRequest::Continue { mission_id } => {
            if let Err(err) = save_manager.write_continue_save(
                host,
                game,
                engine,
                mission_id,
                Some(profiles),
                thumb_ref,
            ) {
                tracing::error!("Continue save failed: {err:#}");
                outcome.banner = Some(SaveBannerKind::SaveFailed);
            } else {
                event = replay_save_written_event(engine, host, game);
            }
        }
        SaveLoadRequest::QuickSave { mission_id } => {
            if host.transport.net.is_some() {
                match persistence::diagnostic(
                    persistence::DiagnosticTarget::New("Multiplayer quick diagnostic"),
                    save_manager,
                    host,
                    game,
                    engine,
                    mission_id,
                    profiles,
                    thumb_ref,
                ) {
                    Ok(idx) => {
                        outcome.banner = Some(SaveBannerKind::Saved);
                        tracing::info!(
                            slot = idx,
                            mission_id,
                            "multiplayer quick-save captured as a local diagnostic"
                        );
                    }
                    Err(persistence::DiagnosticFailure::Allocation(error)) => {
                        tracing::error!("Multiplayer quick diagnostic draft failed: {error:#}");
                        outcome.banner = Some(SaveBannerKind::SaveFailed);
                    }
                    Err(persistence::DiagnosticFailure::Publication(error)) => {
                        tracing::error!("Multiplayer quick diagnostic failed: {error:#}");
                        outcome.banner = Some(SaveBannerKind::SaveFailed);
                    }
                }
                return outcome;
            }
            match save_manager.write_quick_save(
                host,
                game,
                engine,
                mission_id,
                Some(profiles),
                thumb_ref,
            ) {
                Err(err) => {
                    tracing::error!("Quick save failed: {err:#}");
                    outcome.banner = Some(SaveBannerKind::SaveFailed);
                }
                _ => {
                    tracing::info!("Quick save written (mission={mission_id})");
                    event = replay_save_written_event(engine, host, game);
                    // QuickSave is neither Continue nor Restart, so the
                    // Continue-slot mirror runs.
                    if let Err(err) = save_manager.write_continue_save(
                        host,
                        game,
                        engine,
                        mission_id,
                        Some(profiles),
                        thumb_ref,
                    ) {
                        tracing::warn!("Continue-mirror after quick-save failed: {err:#}");
                        notices.enqueue_save_failed(format!("Continue mirror: {err:#}"));
                    }
                    outcome.banner = Some(SaveBannerKind::Saved);
                }
            }
        }
        SaveLoadRequest::QuickLoad { use_backup } => {
            // Shift+F12 loads `ExQuickSave` (the backup).
            // Plain F12 loads `QuickSave`.
            let slot_name = if use_backup {
                special_slots::EX_QUICK
            } else {
                special_slots::QUICK
            };
            let idx = save_manager.find_by_filename(slot_name);
            match idx {
                Some(i) if save_manager.slot_file_exists(i) => {
                    match save_manager
                        .slot_handle(i)
                        .and_then(|slot| PreparedLoad::preflight(save_manager, Some(slot)))
                    {
                        Err(error) => {
                            tracing::error!("Quick load ({slot_name}) preflight failed: {error:#}");
                        }
                        Ok(None) => {
                            tracing::error!(
                                "Quick load ({slot_name}) lost its selected slot during preflight"
                            );
                        }
                        Ok(Some(save)) => {
                            return execute_load(
                                save,
                                load::LoadCompletion::Quick,
                                save_manager,
                                host,
                                game,
                                engine,
                                assets,
                                profiles,
                                thumb_ref,
                            );
                        }
                    }
                }
                _ => tracing::warn!("Quick load requested but no {slot_name} save on disk"),
            }
        }
        SaveLoadRequest::Sherwood { mission_id } => {
            match save_manager.write_sherwood_save(
                host,
                game,
                engine,
                mission_id,
                Some(profiles),
                thumb_ref,
            ) {
                Err(err) => {
                    tracing::error!("Sherwood checkpoint save failed: {err:#}");
                    outcome.banner = Some(SaveBannerKind::SaveFailed);
                }
                _ => {
                    tracing::info!("Sherwood checkpoint saved (mission={mission_id})");
                    event = replay_save_written_event(engine, host, game);
                }
            }
        }
    }
    OperationOutcome { event, ..outcome }
}

/// Shared stages: validate local identity, publish to peers, route, apply, mirror,
/// then construct the receipt. Completion policy preserves each caller's UI and
/// fallback rules; no caller may mark a rejected application as restored.
fn execute_load(
    save: PreparedLoad,
    mut completion: load::LoadCompletion,
    save_manager: &mut SaveGameManager,
    host: &mut crate::host::Host,
    game: &mut crate::game::Game,
    engine: &mut engine_api::Engine,
    assets: &engine_api::LevelAssets,
    profiles: &ProfileManager,
    thumb_ref: Option<&crate::save_file::Thumbnail>,
) -> OperationOutcome {
    let restart = matches!(completion, load::LoadCompletion::Restart);
    let multiplayer = host.transport.net.is_some();
    let result = (|| -> anyhow::Result<OperationOutcome> {
        if matches!(completion, load::LoadCompletion::Selected(_)) {
            let special = save
                .slot()
                .map(|handle| {
                    let index = save_manager.resolve_handle(handle)?;
                    save_manager
                        .get(index)
                        .map(|metadata| metadata.special)
                        .ok_or_else(|| anyhow::anyhow!("selected slot metadata disappeared"))
                })
                .transpose()?
                .flatten();
            completion = load::LoadCompletion::Selected(special);
        }
        if multiplayer && !save.is_committed() {
            anyhow::ensure!(
                begin_multiplayer_snapshot_transition(host, save).map_err(anyhow::Error::msg)?,
                "multiplayer transport disappeared during publication"
            );
            return Ok(OperationOutcome::NO_EVENT);
        }
        let save = match load::route(save, engine, game, profiles).map_err(anyhow::Error::msg)? {
            load::LoadRoute::Current(save) => save,
            load::LoadRoute::OtherMission {
                save,
                target_mission_id,
                active_mission_id,
            } => {
                anyhow::ensure!(
                    !restart,
                    "restart mission {target_mission_id} does not match active mission {active_mission_id}"
                );
                return Ok(OperationOutcome {
                    completion: OperationCompletion::Transition(PendingLevelLoad::new(save)),
                    ..OperationOutcome::NO_EVENT
                });
            }
        };
        let applied = load::apply(save, engine, host, game, assets)?;
        if completion.mirrors_continue() {
            if let Err(error) = save_manager.write_continue_save_background(
                host,
                game,
                engine,
                applied.mission_id(),
                Some(profiles),
                thumb_ref,
            ) {
                tracing::warn!("Continue mirror after load could not start: {error:#}");
            }
        }
        Ok(applied.outcome(completion))
    })();
    match result {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::error!("Load failed: {error:#}");
            if restart && !multiplayer {
                game.operation.set(GameCode::LevelRestart);
                OperationOutcome {
                    completion: OperationCompletion::RestartRequested,
                    ..OperationOutcome::NO_EVENT
                }
            } else {
                OperationOutcome::NO_EVENT
            }
        }
    }
}

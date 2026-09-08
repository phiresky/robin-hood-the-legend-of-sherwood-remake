//! Staged save/load execution after the callback consume/publication barrier.

use super::{
    AutosaveNotices, OperationOrigin, OperationOutcome, PendingLevelLoad, SaveBannerKind,
    SaveLoadEvent, SaveLoadRequest, begin_multiplayer_snapshot_transition, current_mission_id,
    preflight_load_with_origin, replay_save_written_event,
};
use crate::save_file::special_slots;
use crate::savegame::{SaveGameManager, SpecialSlot};
use robin_engine::{engine as engine_api, game_operation::GameCode, profiles::ProfileManager};

mod load;
mod persistence;

pub(super) fn execute(
    request: SaveLoadRequest,
    origin: OperationOrigin,
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
                        .and_then(|()| save_manager.save_index().map_err(|e| anyhow::anyhow!(e))),
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
        SaveLoadRequest::Load {
            slot,
            mission_id: _,
            save,
        } => {
            let applying_multiplayer_transition = origin == OperationOrigin::CommittedMultiplayer;
            // If the save targets a different mission than the one currently
            // running, stash a `PendingLevelLoad` and let the session loop
            // switch missions before re-applying. This replaces the previous
            // warn-and-apply behaviour, which corrupted engine state when
            // the payload's mission didn't match the active level.
            let resolved = match preflight_load_with_origin(&save_manager, slot, save, origin) {
                Ok(resolved) => resolved,
                Err(error) => {
                    tracing::error!("Load preflight failed: {error:#}");
                    return outcome;
                }
            };
            match resolved {
                Some((slot, save)) => {
                    // Remote committed snapshots have no local slot metadata;
                    // all locally selected handles were validated by preflight.
                    let idx = match slot
                        .as_ref()
                        .map(|handle| save_manager.resolve_handle(handle))
                        .transpose()
                    {
                        Ok(index) => index,
                        Err(error) => {
                            tracing::error!("Load rejected stale slot handle: {error:#}");
                            return outcome;
                        }
                    };
                    if host.transport.net.is_some() && !applying_multiplayer_transition {
                        let Some(slot) = slot else {
                            tracing::error!("Local multiplayer load is missing its selected slot");
                            return outcome;
                        };
                        match begin_multiplayer_snapshot_transition(host, slot, save.into_payload())
                        {
                            Ok(true) => return outcome,
                            Ok(false) => unreachable!("multiplayer transition guard checked net"),
                            Err(error) => {
                                tracing::error!("Load rejected: {error}");
                                return outcome;
                            }
                        }
                    }
                    let save = match load::route(save, engine, game, profiles) {
                        Ok(load::LoadRoute::Current(save)) => save,
                        Ok(load::LoadRoute::OtherMission {
                            save,
                            target_mission_id,
                            active_mission_id,
                        }) => {
                            tracing::info!(
                                "Load slot {idx:?}: cross-mission load (header={}, current={}) — routing through session LevelLoad",
                                target_mission_id,
                                active_mission_id
                            );
                            outcome.transition = Some(PendingLevelLoad {
                                slot,
                                target_mission_id,
                                origin,
                                save: save.into_payload(),
                            });
                            return outcome;
                        }
                        Err(error) => {
                            tracing::error!("Load preflight rejected slot {idx:?}: {error}");
                            return outcome;
                        }
                    };
                    match load::apply(save, engine, host, game, assets) {
                        Err(err) => {
                            tracing::error!("Load failed: {err:#}");
                        }
                        Ok(applied) => {
                            // Thread the slot type through so the frame loop
                            // can replay the continue / campaign-map fix-ups.
                            let special = match idx {
                                Some(idx) => {
                                    let Some(metadata) = save_manager.get(idx) else {
                                        tracing::error!(
                                            "Loaded slot metadata disappeared after preflight"
                                        );
                                        return outcome;
                                    };
                                    metadata.special
                                }
                                // A remote snapshot is deliberately not any of
                                // this peer's local special slots.
                                None => None,
                            };
                            // Mirror the load into the Continue slot,
                            // guarded by IsContinue/IsRestart so we
                            // don't clobber the slot we just loaded.
                            if !matches!(
                                special,
                                Some(SpecialSlot::Continue | SpecialSlot::Restart)
                            ) && let Err(error) = save_manager.write_continue_save_background(
                                host,
                                game,
                                engine,
                                applied.mission_id(),
                                Some(profiles),
                                thumb_ref,
                            ) {
                                tracing::warn!(
                                    "Continue-mirror after load could not start: {error:#}"
                                );
                            }
                            tracing::info!("Load completed from slot {idx:?}");
                            return applied.outcome(load::LoadCompletion::Selected(special));
                        }
                    }
                }
                None => {
                    tracing::warn!("Load requested but no matching save slot found");
                }
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
            if host.transport.net.is_some() {
                match save_manager.preflight_restart_save() {
                    Ok(Some((idx, save))) => {
                        let slot = match save_manager.slot_handle(idx) {
                            Ok(slot) => slot,
                            Err(error) => {
                                tracing::error!(
                                    "Multiplayer restart rejected stale slot: {error:#}"
                                );
                                return outcome;
                            }
                        };
                        if let Err(error) =
                            begin_multiplayer_snapshot_transition(host, slot, save.into_payload())
                        {
                            tracing::error!("Multiplayer restart rejected: {error}");
                        }
                    }
                    Ok(None) => {
                        tracing::error!("Multiplayer restart rejected: no restart snapshot exists")
                    }
                    Err(error) => {
                        tracing::error!("Multiplayer restart snapshot preflight failed: {error:#}")
                    }
                }
                return outcome;
            }
            let restore_result = (|| -> anyhow::Result<_> {
                let (_idx, save) = save_manager
                    .preflight_restart_save()?
                    .ok_or_else(|| anyhow::anyhow!("no restart snapshot exists"))?;
                let save = match load::route(save, engine, game, profiles)
                    .map_err(anyhow::Error::msg)?
                {
                    load::LoadRoute::Current(save) => save,
                    load::LoadRoute::OtherMission {
                        target_mission_id,
                        active_mission_id,
                        ..
                    } => anyhow::bail!(
                        "save mission {target_mission_id} does not match active mission {active_mission_id}"
                    ),
                };
                load::apply(save, engine, host, game, assets)
            })();
            match restore_result {
                Ok(applied) => {
                    tracing::info!("Restart snapshot restored");
                    return applied.outcome(load::LoadCompletion::Restart);
                }
                Err(error) => {
                    tracing::error!(
                        "Restart snapshot could not be restored; routing through authoritative LevelRestart: {error:#}"
                    );
                    outcome.restart_requested = true;
                    game.operation.set(GameCode::LevelRestart);
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
                    match save_manager.preflight_load(Some(i)) {
                        Err(error) => {
                            tracing::error!("Quick load ({slot_name}) preflight failed: {error:#}");
                        }
                        Ok(None) => {
                            tracing::error!(
                                "Quick load ({slot_name}) lost its selected slot during preflight"
                            );
                        }
                        Ok(Some((decoded_idx, save))) => {
                            let slot = match save_manager.slot_handle(decoded_idx) {
                                Ok(slot) => slot,
                                Err(error) => {
                                    tracing::error!(
                                        "Quick load ({slot_name}) rejected stale slot: {error:#}"
                                    );
                                    return outcome;
                                }
                            };
                            if host.transport.net.is_some() {
                                match begin_multiplayer_snapshot_transition(
                                    host,
                                    slot,
                                    save.into_payload(),
                                ) {
                                    Ok(true) => return outcome,
                                    Ok(false) => {
                                        unreachable!("multiplayer transition guard checked net")
                                    }
                                    Err(error) => {
                                        tracing::error!(
                                            "Quick load ({slot_name}) rejected: {error}"
                                        );
                                        return outcome;
                                    }
                                }
                            }
                            let save = match load::route(save, engine, game, profiles) {
                                Ok(load::LoadRoute::Current(save)) => save,
                                Ok(load::LoadRoute::OtherMission {
                                    save,
                                    target_mission_id,
                                    ..
                                }) => {
                                    tracing::info!(
                                        "Quick load ({slot_name}): routing mission {target_mission_id} through session LevelLoad"
                                    );
                                    outcome.transition = Some(PendingLevelLoad {
                                        slot: Some(slot),
                                        target_mission_id,
                                        origin,
                                        save: save.into_payload(),
                                    });
                                    return outcome;
                                }
                                Err(error) => {
                                    tracing::error!("Quick load ({slot_name}) rejected: {error}");
                                    return outcome;
                                }
                            };
                            let applied = match load::apply(save, engine, host, game, assets) {
                                Ok(applied) => applied,
                                Err(error) => {
                                    tracing::error!("Quick load ({slot_name}) failed: {error:#}");
                                    return outcome;
                                }
                            };
                            // Mirror into the Continue slot — QuickSave is
                            // neither Continue nor Restart so it always
                            // mirrors.
                            if let Err(error) = save_manager.write_continue_save_background(
                                host,
                                game,
                                engine,
                                applied.mission_id(),
                                Some(profiles),
                                thumb_ref,
                            ) {
                                tracing::warn!(
                                    "Continue-mirror after quick-load could not start: {error:#}"
                                );
                            }
                            tracing::info!("Quick save loaded from {slot_name}");
                            return applied.outcome(load::LoadCompletion::Quick);
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
    OperationOutcome {
        processed: true,
        event,
        ..outcome
    }
}

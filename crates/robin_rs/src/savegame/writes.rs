//! Capture and publication entry points.
//!
//! Every named `write_*` operation is a thin constructor for a [`SaveRequest`]
//! (slot kind × sync/background × source). [`SaveGameManager::write`] is the
//! single dispatch; each leaf preserves its distinct replay-boundary,
//! quick-slot rotation, and synchronous/background ordering.
use super::*;

/// Live-session inputs shared by every capture-based write.
///
/// Not serde: this borrows the running host/game/engine for the duration of
/// one capture and has no persisted form.
#[derive(Clone, Copy)]
struct SaveCapture<'a> {
    host: &'a Host,
    game: &'a crate::game::Game,
    engine: &'a Engine,
    mission_id: u32,
    profiles: Option<&'a ProfileManager>,
    thumbnail: Option<&'a Thumbnail>,
}

/// A well-known special slot and the label used when it is first allocated.
#[derive(Debug, Clone, Copy)]
struct SpecialTarget {
    filename: &'static str,
    display_text: &'static str,
}

const CONTINUE: SpecialTarget = SpecialTarget {
    filename: save_file::special_slots::CONTINUE,
    display_text: "Continue",
};
const RESTART: SpecialTarget = SpecialTarget {
    filename: save_file::special_slots::RESTART,
    display_text: "Restart Point",
};
const SHERWOOD: SpecialTarget = SpecialTarget {
    filename: save_file::special_slots::SHERWOOD,
    display_text: "Sherwood",
};

#[derive(Debug, Clone, Copy)]
enum SaveMode {
    /// Serialize and publish on the calling thread.
    Sync,
    /// Capture on the calling thread; serialize and publish on the owned writer.
    Background,
}

#[derive(Debug, Clone, Copy)]
enum SaveSlot {
    Special {
        target: SpecialTarget,
        mode: SaveMode,
    },
    /// An existing catalog slot, always published synchronously.
    Manual {
        index: usize,
        multiplayer_diagnostic: bool,
    },
    /// QuickSave with ExQuickSave rotation, always published synchronously.
    Quick,
}

enum SaveSource<'a> {
    /// Capture the live session; the write attaches a fresh replay boundary.
    Capture(SaveCapture<'a>),
    /// Mirror an already-decoded payload without capturing a new replay marker.
    Loaded {
        save: GameSaveFile,
        profiles: &'a ProfileManager,
        thumbnail: Option<&'a Thumbnail>,
    },
}

struct SaveRequest<'a> {
    slot: SaveSlot,
    source: SaveSource<'a>,
}

enum SaveOutcome {
    /// Synchronously committed; the serialized payload may be mirrored.
    Committed {
        committed: CommittedSave,
        payload: save_file::SerializedSave,
    },
    /// QuickSave published after rotation.
    QuickPublished(save_file::SerializedSave),
    /// Background publication status, or an immediately completed browser
    /// session checkpoint.
    Status(SaveWriteStatus),
}

impl SaveOutcome {
    fn kind(&self) -> &'static str {
        match self {
            Self::Committed { .. } => "a committed save",
            Self::QuickPublished(_) => "a quick save",
            Self::Status(_) => "a write status",
        }
    }

    fn into_committed(self) -> (CommittedSave, save_file::SerializedSave) {
        match self {
            Self::Committed { committed, payload } => (committed, payload),
            other => panic!("manual-slot save request produced {}", other.kind()),
        }
    }

    fn into_quick_payload(self) -> save_file::SerializedSave {
        match self {
            Self::QuickPublished(payload) => payload,
            other => panic!("quick save request produced {}", other.kind()),
        }
    }

    fn into_status(self) -> SaveWriteStatus {
        match self {
            Self::Status(status) => status,
            other => panic!("background save request produced {}", other.kind()),
        }
    }
}

impl SaveGameManager {
    /// Save the current engine state to the "Continue" auto-save slot.
    /// Called after every successful manual save and at mission quit.
    ///
    pub fn write_continue_save(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        self.write(SaveRequest {
            slot: SaveSlot::Special {
                target: CONTINUE,
                mode: SaveMode::Sync,
            },
            source: SaveSource::Capture(SaveCapture {
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            }),
        })
        .map(|_| ())
    }

    /// Like [`write_continue_save`](Self::write_continue_save), but moves
    /// the expensive JSON serialization + disk write to a background
    /// thread. Used after load, where the player should regain control
    /// as soon as the save has been applied.
    pub fn write_continue_save_background(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<SaveWriteStatus> {
        self.write(SaveRequest {
            slot: SaveSlot::Special {
                target: CONTINUE,
                mode: SaveMode::Background,
            },
            source: SaveSource::Capture(SaveCapture {
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            }),
        })
        .map(SaveOutcome::into_status)
    }

    /// Mirror a successfully loaded save without capturing a new replay marker.
    /// Browser builds have no manual special-save slots and reject this.
    pub(crate) fn write_loaded_continue_background(
        &mut self,
        save: GameSaveFile,
        profiles: &ProfileManager,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<SaveWriteStatus> {
        self.write(SaveRequest {
            slot: SaveSlot::Special {
                target: CONTINUE,
                mode: SaveMode::Background,
            },
            source: SaveSource::Loaded {
                save,
                profiles,
                thumbnail,
            },
        })
        .map(SaveOutcome::into_status)
    }

    /// Save the current engine state to the "Restart" auto-save slot.
    ///
    /// Captures the level start state so the player can restart without
    /// reloading the whole level from disk. Browser builds publish a
    /// session-only Restart checkpoint without a thumbnail.
    pub fn write_restart_save(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        self.write(SaveRequest {
            slot: SaveSlot::Special {
                target: RESTART,
                mode: SaveMode::Sync,
            },
            source: SaveSource::Capture(SaveCapture {
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            }),
        })
        .map(|_| ())
    }

    /// Like [`write_restart_save`](Self::write_restart_save), but captures
    /// the engine state on the calling thread and moves the expensive JSON
    /// serialization + disk write to a background thread. Browser builds
    /// publish an immediately loadable session checkpoint without serialization.
    pub fn write_restart_save_background(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<SaveWriteStatus> {
        self.write(SaveRequest {
            slot: SaveSlot::Special {
                target: RESTART,
                mode: SaveMode::Background,
            },
            source: SaveSource::Capture(SaveCapture {
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            }),
        })
        .map(SaveOutcome::into_status)
    }

    /// Save the current engine state to the "Sherwood" checkpoint slot.
    ///
    /// Captures state when entering the Sherwood map so the campaign
    /// can be rewound one step.
    pub fn write_sherwood_save(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        self.write(SaveRequest {
            slot: SaveSlot::Special {
                target: SHERWOOD,
                mode: SaveMode::Sync,
            },
            source: SaveSource::Capture(SaveCapture {
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            }),
        })
        .map(|_| ())
    }

    /// Write a full save file (engine + campaign) to the given slot.
    ///
    /// The caller must supply the live engine; the engine must have an
    /// active campaign (panics otherwise).  If `thumbnail` is `Some`, it
    /// is also written to the sibling thumb file alongside the payload.
    pub fn write_save_from_engine(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        index: usize,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<CommittedSave> {
        self.write(SaveRequest {
            slot: SaveSlot::Manual {
                index,
                multiplayer_diagnostic: false,
            },
            source: SaveSource::Capture(SaveCapture {
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            }),
        })
        .map(|outcome| outcome.into_committed().0)
    }

    /// Write a local multiplayer diagnostic. It is deliberately tagged in
    /// both the payload and slot index and is never suitable as session state.
    pub fn write_multiplayer_diagnostic_from_engine(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        index: usize,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<CommittedSave> {
        self.write(SaveRequest {
            slot: SaveSlot::Manual {
                index,
                multiplayer_diagnostic: true,
            },
            source: SaveSource::Capture(SaveCapture {
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            }),
        })
        .map(|outcome| outcome.into_committed().0)
    }

    /// Publish the selected slot first, then mirror the same capture if its
    /// slot policy requires it. Some reports only mirror failure; Err means
    /// the primary publication failed.
    pub(crate) fn write_save_and_continue(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        index: usize,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<Option<String>> {
        let (committed, payload) = self
            .write(SaveRequest {
                slot: SaveSlot::Manual {
                    index,
                    multiplayer_diagnostic: false,
                },
                source: SaveSource::Capture(SaveCapture {
                    host,
                    game,
                    engine,
                    mission_id,
                    profiles,
                    thumbnail,
                }),
            })?
            .into_committed();
        let index = self.resolve_handle(committed.slot())?;
        if matches!(
            self.catalog[index].special,
            Some(SpecialSlot::Continue | SpecialSlot::Restart)
        ) {
            return Ok(None);
        }
        Ok(self
            .mirror_captured_continue(index, &payload, thumbnail)
            .err()
            .map(|e| format!("{e:#}")))
    }

    /// Save to "QuickSave" (rotating the previous one to "ExQuickSave"), then
    /// mirror the same payload to Continue. Err means the quick save failed;
    /// `Some` reports only a mirror failure.
    pub(crate) fn write_quick_save_and_continue(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<Option<String>> {
        let payload = self
            .write(SaveRequest {
                slot: SaveSlot::Quick,
                source: SaveSource::Capture(SaveCapture {
                    host,
                    game,
                    engine,
                    mission_id,
                    profiles,
                    thumbnail,
                }),
            })?
            .into_quick_payload();
        let index = self
            .find_by_filename(save_file::special_slots::QUICK)
            .context("published quick save lost its slot")?;
        Ok(self
            .mirror_captured_continue(index, &payload, thumbnail)
            .err()
            .map(|e| format!("{e:#}")))
    }

    #[cfg(test)]
    pub(super) fn write_quick_save(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        self.write(SaveRequest {
            slot: SaveSlot::Quick,
            source: SaveSource::Capture(SaveCapture {
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            }),
        })
        .map(|_| ())
    }

    /// The single dispatch for every named write entry point.
    fn write(&mut self, request: SaveRequest<'_>) -> Result<SaveOutcome> {
        self.require_storage()?;
        let SaveRequest { slot, source } = request;
        match slot {
            SaveSlot::Special { target, mode } => {
                #[cfg(target_arch = "wasm32")]
                {
                    // Browser Restart is a session checkpoint in either mode,
                    // published synchronously and without a thumbnail.
                    if target.filename == save_file::special_slots::RESTART
                        && let SaveSource::Capture(capture) = &source
                    {
                        self.write_session_restart(
                            capture.host,
                            capture.game,
                            capture.engine,
                            capture.mission_id,
                            capture.profiles,
                        )?;
                        return Ok(SaveOutcome::Status(SaveWriteStatus::Completed));
                    }
                }
                match (mode, source) {
                    (SaveMode::Sync, SaveSource::Capture(capture)) => {
                        let index =
                            self.ensure_special_slot(target.filename, target.display_text)?;
                        let (committed, payload) = self.write_manual(index, capture, false)?;
                        Ok(SaveOutcome::Committed { committed, payload })
                    }
                    (SaveMode::Background, source) => self
                        .write_special_background(target, source)
                        .map(SaveOutcome::Status),
                    (SaveMode::Sync, SaveSource::Loaded { .. }) => anyhow::bail!(
                        "loaded save payloads can only be mirrored by a background write, not {slot:?}"
                    ),
                }
            }
            SaveSlot::Manual {
                index,
                multiplayer_diagnostic,
            } => {
                let SaveSource::Capture(capture) = source else {
                    anyhow::bail!("loaded save payloads cannot be written to {slot:?}");
                };
                let (committed, payload) =
                    self.write_manual(index, capture, multiplayer_diagnostic)?;
                Ok(SaveOutcome::Committed { committed, payload })
            }
            SaveSlot::Quick => {
                let SaveSource::Capture(capture) = source else {
                    anyhow::bail!("loaded save payloads cannot be written to {slot:?}");
                };
                self.write_quick_save_payload(capture)
                    .map(SaveOutcome::QuickPublished)
            }
        }
    }

    /// Save the current engine state to the "QuickSave" slot.
    /// The previous quick save (if any) is rotated to "ExQuickSave".
    fn write_quick_save_payload(
        &mut self,
        capture: SaveCapture<'_>,
    ) -> Result<save_file::SerializedSave> {
        let SaveCapture {
            host,
            mission_id,
            profiles,
            thumbnail,
            ..
        } = capture;
        Self::require_synchronous_storage()?;
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        self.reconcile_quick_slots()?;
        // Validate and serialize before touching either published quick slot.
        // A failed capture must not rotate a player's recoverable saves.
        let quick_index = self.find_by_filename(save_file::special_slots::QUICK);
        let mut current = quick_index
            .map(|index| self.catalog[index].clone())
            .unwrap_or_else(|| {
                SaveGame::new(
                    save_file::special_slots::QUICK.to_owned(),
                    "Quick Save".to_owned(),
                    mission_id,
                )
            });
        let mut save = capture_save(capture, current.text.clone())?;
        host.application_context()
            .replay_recording()
            .attach_save_boundary(&mut save)?;
        let payload = save_file::SerializedSave::new(&save)?;
        let bytes = payload.encode(&current.text)?;
        current.update_snapshot_metadata(
            &save.header,
            save.engine.campaign(),
            profiles.context("quick save requires profiles")?,
        );
        let mut recovery = QuickSaveRecovery {
            slots: vec![(current, Sha256::digest(&bytes).into())],
        };
        if let Some(index) = quick_index
            && self.slot_file_exists(index)
        {
            let previous_digest = recovery::payload_digest(&self.save_path(index))
                .context("preparing previous quick save")?
                .context("previous quick save disappeared during preparation")?;
            let mut previous = self.catalog[index].clone();
            previous.filename = save_file::special_slots::EX_QUICK.to_owned();
            previous.special = Some(SpecialSlot::ExQuickSave);
            previous.text = self
                .find_by_filename(save_file::special_slots::EX_QUICK)
                .map(|index| self.catalog[index].text.clone())
                .unwrap_or_else(|| "Previous Quick Save".to_owned());
            previous.validate_published_metadata()?;
            recovery.slots.push((previous, previous_digest));
        }
        save_file::atomic_write(&self.quick_recovery_path(), &serde_json::to_vec(&recovery)?)?;
        // Rotate: QuickSave → ExQuickSave
        if let Some(quick_idx) = quick_index
            && self.slot_file_exists(quick_idx)
        {
            // Ensure an ExQuickSave slot exists, then copy the file.
            let ex_idx = self
                .ensure_special_slot(save_file::special_slots::EX_QUICK, "Previous Quick Save")?;
            self.copy_files(quick_idx, ex_idx)
                .map_err(|e| anyhow::anyhow!(e))?;
            self.copy_display_metadata(quick_idx, ex_idx)?;
        }
        let idx = self.ensure_special_slot(save_file::special_slots::QUICK, "Quick Save")?;
        save_file::atomic_write(&self.save_path(idx), &bytes)?;
        self.publish_thumbnail(idx, thumbnail);
        self.sync_slot_metadata_from_save(idx, &save, profiles)?;
        self.publish_index().map_err(anyhow::Error::msg)?;
        Ok(payload)
    }

    /// Queue a special-slot publication on the owned background writer.
    /// Capture happens here, on the calling thread, before the writer starts.
    #[cfg(not(target_arch = "wasm32"))]
    fn write_special_background(
        &mut self,
        target: SpecialTarget,
        source: SaveSource<'_>,
    ) -> Result<SaveWriteStatus> {
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        self.reconcile_quick_slots()?;
        let idx = self.ensure_special_slot(target.filename, target.display_text)?;
        let display_text = self.catalog[idx].text.clone();
        match source {
            SaveSource::Capture(capture) => {
                // Capture (clone) on the main thread before starting the writer.
                let mut save = capture_save(capture, display_text)?;
                capture
                    .host
                    .application_context()
                    .replay_recording()
                    .attach_save_boundary(&mut save)?;
                self.queue_special_save(
                    idx,
                    save,
                    capture
                        .profiles
                        .context("save metadata requires profiles")?,
                    capture.thumbnail,
                )
            }
            SaveSource::Loaded {
                mut save,
                profiles,
                thumbnail,
            } => {
                save.header.display_text = display_text;
                self.queue_special_save(idx, save, profiles, thumbnail)
            }
        }
    }

    /// Browser builds have no manual special-save slots. For a live capture,
    /// pending background work is still settled first, exactly as on native;
    /// a loaded payload is rejected immediately.
    #[cfg(target_arch = "wasm32")]
    fn write_special_background(
        &mut self,
        _target: SpecialTarget,
        source: SaveSource<'_>,
    ) -> Result<SaveWriteStatus> {
        match source {
            SaveSource::Capture(_) => {
                self.finish_background()?;
                self.ensure_no_pending_delete()?;
            }
            SaveSource::Loaded {
                save: _save,
                profiles: _profiles,
                thumbnail: _thumbnail,
            } => {}
        }
        anyhow::bail!(
            "browser manual special-save persistence is unavailable; use durable autosaves"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn queue_special_save(
        &mut self,
        idx: usize,
        save: GameSaveFile,
        profiles: &ProfileManager,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<SaveWriteStatus> {
        let path = self.save_path(idx);
        let thumb_data = thumbnail.cloned();
        let thumb_path = self.thumb_path(idx);
        let mut metadata = self.catalog[idx].clone();
        metadata.update_snapshot_metadata(&save.header, save.engine.campaign(), profiles);
        metadata.validate_published_metadata()?;
        let name = self.slot_name(idx).map_err(anyhow::Error::msg)?;
        let recovery_path = self.owned_recovery_path();
        self.operations.start(name, move || {
            save.validate_current_schema()?;
            let bytes = serde_json::to_vec_pretty(&save).context("serialize owned save payload")?;
            let receipt = SpecialSaveRecovery {
                slot: metadata,
                digest: Sha256::digest(&bytes).into(),
            };
            persistence::publish_payload(&recovery_path, &path, &receipt, &bytes, true)?;
            if let Some(thumb) = thumb_data
                && let Err(err) = thumb.write_to(&thumb_path)
            {
                tracing::warn!("Owned save thumbnail failed (payload completed): {err:#}");
            }
            Ok(receipt.slot)
        })?;
        Ok(SaveWriteStatus::Queued)
    }

    /// Capture and publish a session-only Restart atomically. No disk index is
    /// written: this checkpoint is intentionally gone with its owning manager.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(super) fn write_session_restart(
        &mut self,
        host: &Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
    ) -> Result<()> {
        // Invalidate the previous mission's checkpoint even if capture fails.
        self.session_restart = None;
        let provenance = required_save_provenance(host, engine, mission_id, profiles)?;
        let header = SaveHeader::new(
            mission_id,
            game.mission_assets().map_err(anyhow::Error::msg)?.clone(),
            "Restart Point".into(),
            provenance,
        )?;
        let mut save = PreparedGameSave::capture_session_restart(engine, host, game, header)?;
        save.record_replay_boundary(&host.application_context().replay_recording())?;
        let mut slot = SaveGame::new(
            save_file::special_slots::RESTART.into(),
            "Restart Point".into(),
            mission_id,
        );
        slot.update_snapshot_metadata(
            &save.header,
            save.engine.campaign(),
            profiles.context("restart requires mission profiles")?,
        );
        // This runtime-only checkpoint must not consult a filesystem receipt
        // or require a writable desktop save root.
        self.catalog.upsert(slot, SlotState::Session)?;
        self.session_restart = Some(std::sync::Arc::new(save));
        Ok(())
    }

    /// Capture into an existing catalog slot and publish it synchronously.
    fn write_manual(
        &mut self,
        index: usize,
        capture: SaveCapture<'_>,
        multiplayer_diagnostic: bool,
    ) -> Result<(CommittedSave, save_file::SerializedSave)> {
        Self::require_synchronous_storage()?;
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        self.reconcile_quick_slots()?;
        let display_text = self
            .catalog
            .get(index)
            .with_context(|| format!("cannot write missing save slot {index}"))?
            .text
            .clone();
        let mut save = capture_save(capture, display_text)?;
        save.header.multiplayer_diagnostic = multiplayer_diagnostic;
        let mut metadata = self.catalog[index].clone();
        metadata.update_snapshot_metadata(
            &save.header,
            save.engine.campaign(),
            capture
                .profiles
                .context("save metadata requires mission profiles")?,
        );
        metadata.validate_published_metadata()?;
        save.validate_current_schema()?;
        capture
            .host
            .application_context()
            .replay_recording()
            .attach_save_boundary(&mut save)?;
        let payload = save_file::SerializedSave::new(&save)?;
        let bytes = payload.encode(&metadata.text)?;
        let committed = self.commit_synchronous(index, metadata, &bytes, capture.thumbnail)?;
        Ok((committed, payload))
    }

    fn mirror_captured_continue(
        &mut self,
        source: usize,
        payload: &save_file::SerializedSave,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        let index = self.ensure_special_slot(CONTINUE.filename, CONTINUE.display_text)?;
        let metadata = self.catalog[source].cloned_for_slot(&self.catalog[index]);
        let bytes = payload.encode(&metadata.text)?;
        self.commit_synchronous(index, metadata, &bytes, thumbnail)
            .map(|_| ())
    }
}

/// Capture only: each publication workflow owns when to attach its replay boundary.
fn capture_save(capture: SaveCapture<'_>, display_text: String) -> Result<GameSaveFile> {
    let SaveCapture {
        host,
        game,
        engine,
        mission_id,
        profiles,
        ..
    } = capture;
    let provenance = required_save_provenance(host, engine, mission_id, profiles)?;
    GameSaveFile::capture_with_game(
        engine,
        host,
        game,
        mission_id,
        game.mission_assets().map_err(anyhow::Error::msg)?.clone(),
        display_text,
        provenance,
    )
}

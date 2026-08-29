//! Rotating, interruption-safe gameplay autosaves.
//!
//! Autosaves deliberately use their own manifest instead of racing the
//! player-managed `saves.json` index.  A writer commits a new, uniquely named
//! payload first and publishes the manifest second.  A crash can therefore
//! leave at most an unreferenced payload; it cannot make a previously visible
//! autosave point at partially replaced bytes.

use crate::game::Game;
use crate::host::Host;
use crate::save_file::{GameSaveFile, Thumbnail};
use crate::savegame::{SaveGame, SaveGameManager};
use anyhow::{Context, Result, bail};
use robin_engine::campaign::CampaignValue;
use robin_engine::engine::Engine;
use robin_engine::profiles::ProfileManager;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[cfg(target_arch = "wasm32")]
use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// Default autosave cadence: five minutes of admitted 25 Hz gameplay ticks.
pub const AUTOSAVE_INTERVAL_ACTIVE_SECONDS: u32 = 5 * 60;
pub const AUTOSAVE_INTERVAL_FRAMES: u64 = AUTOSAVE_INTERVAL_ACTIVE_SECONDS as u64 * 25;
/// Number of independently loadable autosave generations retained per profile.
pub const AUTOSAVE_SLOT_COUNT: usize = 3;
const AUTOSAVE_MANIFEST_VERSION: u32 = 1;
const AUTOSAVE_MANIFEST_FILE: &str = "autosaves.json";

pub(crate) const fn session_allows_autosave(
    setting_enabled: bool,
    multiplayer: bool,
    replay_playback: bool,
    headless: bool,
) -> bool {
    setting_enabled && !multiplayer && !replay_playback && !headless
}

/// Why an autosave snapshot was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutosaveReason {
    Periodic,
    Backgrounded,
    MissionTransition,
}

/// The authoritative list of autosaves published independently of manual saves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutosaveManifest {
    pub version: u32,
    pub saves: Vec<SaveGame>,
}

impl Default for AutosaveManifest {
    fn default() -> Self {
        Self {
            version: AUTOSAVE_MANIFEST_VERSION,
            saves: Vec::new(),
        }
    }
}

impl AutosaveManifest {
    fn validate(&self) -> Result<()> {
        if self.version != AUTOSAVE_MANIFEST_VERSION {
            bail!(
                "unsupported autosave manifest version: expected {}, got {}",
                AUTOSAVE_MANIFEST_VERSION,
                self.version
            );
        }
        if self.saves.len() > AUTOSAVE_SLOT_COUNT {
            bail!(
                "autosave manifest contains {} slots; policy permits {AUTOSAVE_SLOT_COUNT}",
                self.saves.len()
            );
        }
        let mut filenames = std::collections::BTreeSet::new();
        for save in &self.saves {
            if !crate::savegame::is_generated_autosave_filename(&save.filename) {
                bail!(
                    "autosave manifest contains non-autosave filename {:?}",
                    save.filename
                );
            }
            if !filenames.insert(&save.filename) {
                bail!(
                    "autosave manifest contains duplicate filename {:?}",
                    save.filename
                );
            }
        }
        Ok(())
    }
}

/// Fully captured write request. Engine state is cloned on the game thread;
/// serialization and persistence happen on the single writer.
#[derive(Clone, Serialize, Deserialize)]
struct AutosaveJob {
    save_directory: String,
    filename: String,
    payload: GameSaveFile,
    metadata: SaveGame,
    thumbnail: Option<Thumbnail>,
    /// Used only when this profile has never published an autosave manifest.
    /// Older builds could have autosave-looking entries in `saves.json`; seed
    /// from those once, then let the independent manifest be authoritative.
    manifest_seed: AutosaveManifest,
    reason: AutosaveReason,
}

#[derive(Serialize, Deserialize)]
enum AutosaveCompletion {
    Saved {
        manifest: AutosaveManifest,
        filename: String,
        reason: AutosaveReason,
    },
    Failed {
        filename: String,
        reason: AutosaveReason,
        error: String,
    },
}

impl AutosaveCompletion {
    fn failed(job: &AutosaveJob, error: anyhow::Error) -> Self {
        let error = format!("{error:#}");
        tracing::error!(
            filename = job.filename,
            reason = ?job.reason,
            "Autosave failed: {error}"
        );
        Self::Failed {
            filename: job.filename.clone(),
            reason: job.reason,
            error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct AutosaveSchedule {
    mission_id: Option<u32>,
    next_periodic_frame: u64,
    background_frame: Option<u64>,
    terminal_frame: Option<u64>,
}

impl Default for AutosaveSchedule {
    fn default() -> Self {
        Self {
            mission_id: None,
            next_periodic_frame: AUTOSAVE_INTERVAL_FRAMES,
            background_frame: None,
            terminal_frame: None,
        }
    }
}

impl AutosaveSchedule {
    fn preview(
        &self,
        mission_id: u32,
        frame: u64,
        backgrounded: bool,
        terminal_transition: bool,
    ) -> Option<AutosaveReason> {
        if self.mission_id != Some(mission_id) {
            return Some(AutosaveReason::MissionTransition);
        }
        if terminal_transition && self.terminal_frame != Some(frame) {
            return Some(AutosaveReason::MissionTransition);
        }
        if backgrounded && self.background_frame != Some(frame) {
            return Some(AutosaveReason::Backgrounded);
        }
        if frame >= self.next_periodic_frame {
            return Some(AutosaveReason::Periodic);
        }
        None
    }

    fn commit(&mut self, mission_id: u32, frame: u64, reason: AutosaveReason) {
        if self.mission_id != Some(mission_id) {
            self.mission_id = Some(mission_id);
            self.next_periodic_frame = frame.saturating_add(AUTOSAVE_INTERVAL_FRAMES);
            self.background_frame = None;
            self.terminal_frame = None;
        }
        match reason {
            AutosaveReason::Periodic => {
                self.next_periodic_frame = frame.saturating_add(AUTOSAVE_INTERVAL_FRAMES);
            }
            AutosaveReason::Backgrounded => self.background_frame = Some(frame),
            AutosaveReason::MissionTransition => self.terminal_frame = Some(frame),
        }
    }

    fn reset_disabled(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlannedAutosave {
    mission_id: u32,
    frame: u64,
    reason: AutosaveReason,
}

/// Result surfaced to the game loop after an asynchronous write completes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AutosavePollResult {
    Saved {
        filename: String,
        reason: AutosaveReason,
    },
    Failed {
        filename: String,
        reason: AutosaveReason,
        error: String,
    },
}

/// Process-local scheduler and exactly-one-writer queue.
pub(crate) struct AutosaveCoordinator {
    schedule: AutosaveSchedule,
    planned: Option<PlannedAutosave>,
    next_filename_sequence: u32,
    #[cfg(not(target_arch = "wasm32"))]
    command_tx: std::sync::mpsc::Sender<Option<AutosaveJob>>,
    #[cfg(not(target_arch = "wasm32"))]
    completion_rx: std::sync::mpsc::Receiver<AutosaveCompletion>,
    #[cfg(not(target_arch = "wasm32"))]
    writer_thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(target_arch = "wasm32")]
    completions: std::rc::Rc<std::cell::RefCell<VecDeque<AutosaveCompletion>>>,
    #[cfg(target_arch = "wasm32")]
    pending_jobs: std::rc::Rc<std::cell::RefCell<VecDeque<AutosaveJob>>>,
    #[cfg(target_arch = "wasm32")]
    writer_running: std::rc::Rc<std::cell::Cell<bool>>,
}

impl Default for AutosaveCoordinator {
    fn default() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (command_tx, command_rx) = std::sync::mpsc::channel::<Option<AutosaveJob>>();
            let (completion_tx, completion_rx) = std::sync::mpsc::channel::<AutosaveCompletion>();
            let writer_thread = std::thread::Builder::new()
                .name("autosave-writer".to_owned())
                .spawn(move || {
                    while let Ok(Some(job)) = command_rx.recv() {
                        let completion = match write_job(&job) {
                            Ok(completion) => completion,
                            Err(error) => AutosaveCompletion::failed(&job, error),
                        };
                        if completion_tx.send(completion).is_err() {
                            break;
                        }
                    }
                })
                .expect("failed to spawn the autosave writer thread");
            Self {
                schedule: AutosaveSchedule::default(),
                planned: None,
                next_filename_sequence: 0,
                command_tx,
                completion_rx,
                writer_thread: Some(writer_thread),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self {
                schedule: AutosaveSchedule::default(),
                planned: None,
                next_filename_sequence: 0,
                completions: std::rc::Rc::new(std::cell::RefCell::new(VecDeque::new())),
                pending_jobs: std::rc::Rc::new(std::cell::RefCell::new(VecDeque::new())),
                writer_running: std::rc::Rc::new(std::cell::Cell::new(false)),
            }
        }
    }
}

impl AutosaveCoordinator {
    pub(crate) fn plan(
        &mut self,
        enabled_and_allowed: bool,
        snapshot_available: bool,
        mission_id: u32,
        frame: u64,
        backgrounded: bool,
        terminal_transition: bool,
    ) -> Option<AutosaveReason> {
        if !enabled_and_allowed {
            self.schedule.reset_disabled();
            self.planned = None;
            return None;
        }
        if !snapshot_available {
            self.planned = None;
            return None;
        }
        let reason = self
            .schedule
            .preview(mission_id, frame, backgrounded, terminal_transition)?;
        self.planned = Some(PlannedAutosave {
            mission_id,
            frame,
            reason,
        });
        Some(reason)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enqueue(
        &mut self,
        manager: &SaveGameManager,
        host: &Host,
        game: &Game,
        engine: &Engine,
        mission_id: u32,
        profiles: &ProfileManager,
        thumbnail: Option<Thumbnail>,
        reason: AutosaveReason,
    ) -> Result<()> {
        let planned = self
            .planned
            .filter(|planned| planned.mission_id == mission_id && planned.reason == reason)
            .context("autosave enqueue did not match the current schedule decision")?;
        let filename = self.next_unique_filename(manager)?;
        let display_text = mission_display_name(engine, mission_id, profiles)?;
        let payload = GameSaveFile::capture_with_game(engine, host, game, mission_id, display_text);
        if payload.header.timestamp_unix == 0 {
            bail!("autosave payload clock returned the invalid Unix timestamp zero");
        }
        let metadata = metadata_from_payload(&filename, &payload, profiles)?;
        let manifest_seed = AutosaveManifest {
            version: AUTOSAVE_MANIFEST_VERSION,
            saves: manager
                .saves
                .iter()
                .filter(|save| save.is_autosave())
                .cloned()
                .collect(),
        };
        let job = AutosaveJob {
            save_directory: manager.save_directory.clone(),
            filename,
            payload,
            metadata,
            thumbnail,
            manifest_seed,
            reason,
        };

        #[cfg(not(target_arch = "wasm32"))]
        self.command_tx
            .send(Some(job))
            .context("autosave writer thread is unavailable")?;
        #[cfg(target_arch = "wasm32")]
        {
            if reason != AutosaveReason::Periodic {
                // A lifecycle callback may be the page's last executable
                // turn before it is frozen or discarded. Publish urgent
                // payload+manifest bytes now and return the error to the
                // caller instead of merely queueing a failed completion.
                let completion = write_job(&job)
                    .with_context(|| format!("publishing urgent {reason:?} browser autosave"))?;
                self.completions.borrow_mut().push_back(completion);
                self.writer_running.set(false);
            } else {
                self.pending_jobs.borrow_mut().push_back(job);
                if !self.writer_running.replace(true) {
                    let completions = self.completions.clone();
                    let pending_jobs = self.pending_jobs.clone();
                    let writer_running = self.writer_running.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        // No timer precedes persistence. Periodic writes move
                        // to the next microtask only.
                        drain_browser_jobs(&pending_jobs, &completions);
                        writer_running.set(false);
                    });
                }
            }
        }
        self.schedule
            .commit(planned.mission_id, planned.frame, planned.reason);
        self.planned = None;
        Ok(())
    }

    fn next_unique_filename(&mut self, manager: &SaveGameManager) -> Result<String> {
        let timestamp = current_unix_timestamp()?;
        loop {
            let sequence = self.next_filename_sequence;
            self.next_filename_sequence = self.next_filename_sequence.wrapping_add(1);
            let filename = format!("Autosave_{timestamp}_{sequence:04}");
            if manager.find_by_filename(&filename).is_none() {
                return Ok(filename);
            }
        }
    }

    pub(crate) fn poll(&mut self, manager: &mut SaveGameManager) -> Vec<AutosavePollResult> {
        let mut results = Vec::new();
        #[cfg(not(target_arch = "wasm32"))]
        let completions: Vec<_> = self.completion_rx.try_iter().collect();
        #[cfg(target_arch = "wasm32")]
        let completions: Vec<_> = self.completions.borrow_mut().drain(..).collect();

        for completion in completions {
            match completion {
                AutosaveCompletion::Saved {
                    manifest,
                    filename,
                    reason,
                } => {
                    manager.replace_autosaves(manifest.saves);
                    tracing::info!(filename, ?reason, "Autosave committed");
                    results.push(AutosavePollResult::Saved { filename, reason });
                }
                AutosaveCompletion::Failed {
                    filename,
                    reason,
                    error,
                } => results.push(AutosavePollResult::Failed {
                    filename,
                    reason,
                    error,
                }),
            }
        }
        results
    }

    /// Finish every accepted native write and surface every queued completion
    /// before the callback/save-manager pair is destroyed. Browser urgent
    /// writes are synchronous; periodic microtasks retain their own queue and
    /// are recovered from the manifest when the next manager opens.
    pub(crate) fn shutdown_and_poll(
        &mut self,
        manager: &mut SaveGameManager,
    ) -> Vec<AutosavePollResult> {
        #[cfg(not(target_arch = "wasm32"))]
        self.stop_native_writer();
        #[cfg(target_arch = "wasm32")]
        {
            if !self.pending_jobs.borrow().is_empty() {
                drain_browser_jobs(&self.pending_jobs, &self.completions);
                self.writer_running.set(false);
            }
        }
        self.poll(manager)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn stop_native_writer(&mut self) {
        if self.writer_thread.is_none() {
            return;
        }
        if self.command_tx.send(None).is_err() {
            tracing::error!("autosave writer command channel closed before shutdown");
        }
        if let Some(handle) = self.writer_thread.take()
            && handle.join().is_err()
        {
            tracing::error!("autosave writer thread panicked during shutdown");
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn drain_browser_jobs(
    pending_jobs: &std::rc::Rc<std::cell::RefCell<VecDeque<AutosaveJob>>>,
    completions: &std::rc::Rc<std::cell::RefCell<VecDeque<AutosaveCompletion>>>,
) {
    while let Some(job) = pending_jobs.borrow_mut().pop_front() {
        let completion = match write_job(&job) {
            Ok(completion) => completion,
            Err(error) => AutosaveCompletion::failed(&job, error),
        };
        completions.borrow_mut().push_back(completion);
    }
}

fn current_unix_timestamp() -> Result<u64> {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .context("autosave clock is before the Unix epoch")
        .map(|duration| duration.as_secs())
}

fn mission_display_name(
    engine: &Engine,
    mission_id: u32,
    profiles: &ProfileManager,
) -> Result<String> {
    let mission = engine
        .campaign()
        .get_mission(mission_id, profiles)
        .with_context(|| format!("autosave mission ID {mission_id} is absent from the campaign"))?;
    let profile = mission.profile(profiles);
    if !profile.mission_name.trim().is_empty() {
        return Ok(profile.mission_name.clone());
    }
    if !profile.mission_filename.trim().is_empty() {
        return Ok(profile.mission_filename.clone());
    }
    bail!("autosave mission ID {mission_id} has neither a display name nor a filename")
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for AutosaveCoordinator {
    fn drop(&mut self) {
        self.stop_native_writer();
    }
}

fn metadata_from_payload(
    filename: &str,
    payload: &GameSaveFile,
    profiles: &ProfileManager,
) -> Result<SaveGame> {
    let campaign = payload.engine.campaign();
    let mission_name = mission_display_name(&payload.engine, payload.header.mission_id, profiles)?;
    let mut metadata = SaveGame::new(
        filename.to_owned(),
        mission_name.clone(),
        payload.header.mission_id,
    );
    metadata.mission_id = payload.header.mission_id;
    metadata.version = payload.header.version;
    metadata.timestamp = payload.header.timestamp_unix.to_string();
    metadata.mission_name = mission_name;
    metadata.missions_done = Some(campaign.get_number_of_missions_done());
    metadata.missions_total = Some(campaign.missions.len());
    metadata.gang_size = Some(campaign.gang_indices.len());
    metadata.ransom = Some(campaign.values[CampaignValue::Ransom]);
    metadata.blazons = Some(campaign.values[CampaignValue::Blazon]);
    metadata.amulets = Some(campaign.values[CampaignValue::Amulets]);
    metadata.campaign_progress = Some(campaign.get_progression(profiles));
    if !metadata.is_autosave() {
        bail!("generated autosave filename was not classified as an autosave");
    }
    Ok(metadata)
}

fn staged_manifest(
    existing: AutosaveManifest,
    metadata: SaveGame,
) -> (AutosaveManifest, Vec<String>) {
    let mut saves = existing.saves;
    saves.retain(|save| save.filename != metadata.filename);
    saves.push(metadata);
    let remove_count = saves.len().saturating_sub(AUTOSAVE_SLOT_COUNT);
    let evicted_filenames = saves[..remove_count]
        .iter()
        .map(|save| save.filename.clone())
        .collect();
    saves.drain(..remove_count);
    (
        AutosaveManifest {
            version: AUTOSAVE_MANIFEST_VERSION,
            saves,
        },
        evicted_filenames,
    )
}

fn write_job(job: &AutosaveJob) -> Result<AutosaveCompletion> {
    tracing::info!(
        filename = job.filename,
        reason = ?job.reason,
        "Writing autosave"
    );
    let existing_manifest =
        load_manifest(&job.save_directory)?.unwrap_or_else(|| job.manifest_seed.clone());
    existing_manifest.validate()?;
    garbage_collect_orphans(&job.save_directory, &existing_manifest)?;
    validate_manifest_payloads(&job.save_directory, &existing_manifest)?;
    let manifest = commit_generation(
        existing_manifest,
        job.metadata.clone(),
        || {
            persist_payload(
                &job.save_directory,
                &job.filename,
                &job.payload,
                job.thumbnail.as_ref(),
            )
        },
        |manifest| persist_manifest(&job.save_directory, manifest),
        |filename| remove_payload(&job.save_directory, filename),
    )?;
    Ok(AutosaveCompletion::Saved {
        manifest,
        filename: job.filename.clone(),
        reason: job.reason,
    })
}

/// Commit one immutable generation. The closure ordering is the crash-safety
/// contract: payload first, manifest publication second, obsolete generation
/// cleanup only after the new manifest is durable. Cleanup failures cannot
/// roll back an already-published recovery point and are repaired by orphan
/// collection on the next open/write.
fn commit_generation(
    existing: AutosaveManifest,
    metadata: SaveGame,
    mut write_payload: impl FnMut() -> Result<()>,
    mut publish_manifest: impl FnMut(&AutosaveManifest) -> Result<()>,
    mut cleanup_generation: impl FnMut(&str) -> Result<()>,
) -> Result<AutosaveManifest> {
    let (manifest, evicted_filenames) = staged_manifest(existing, metadata);
    manifest.validate()?;
    write_payload().context("committing autosave payload before manifest publication")?;
    publish_manifest(&manifest).context("publishing autosave manifest after payload commit")?;
    for filename in evicted_filenames {
        if let Err(error) = cleanup_generation(&filename) {
            tracing::warn!(
                filename,
                "published autosave but could not remove rotated generation: {error:#}"
            );
        }
    }
    Ok(manifest)
}

/// Merge the independently committed autosave manifest into a save manager.
pub(crate) fn load_into_manager(manager: &mut SaveGameManager) -> Result<()> {
    // Never leave stale index entries visible after a corrupt or missing
    // payload. A validated manifest below is the only authority that may
    // republish autosave rows into the load menu.
    let legacy_seed = AutosaveManifest {
        version: AUTOSAVE_MANIFEST_VERSION,
        saves: manager
            .saves
            .iter()
            .filter(|save| save.is_autosave())
            .cloned()
            .collect(),
    };
    manager.replace_autosaves(Vec::new());
    let manifest = load_manifest(&manager.save_directory)?.unwrap_or(legacy_seed);
    manifest.validate()?;
    garbage_collect_orphans(&manager.save_directory, &manifest)?;
    validate_manifest_payloads(&manager.save_directory, &manifest)?;
    manager.replace_autosaves(manifest.saves);
    Ok(())
}

fn validate_manifest_payloads(save_directory: &str, manifest: &AutosaveManifest) -> Result<()> {
    for metadata in &manifest.saves {
        let payload = read_payload(save_directory, &metadata.filename)
            .with_context(|| format!("validating published autosave {:?}", metadata.filename))?;
        validate_metadata_payload_binding(metadata, &payload)?;
    }
    Ok(())
}

fn validate_metadata_payload_binding(metadata: &SaveGame, payload: &GameSaveFile) -> Result<()> {
    if !crate::savegame::is_generated_autosave_filename(&metadata.filename) {
        bail!("published autosave metadata has an invalid filename");
    }
    if metadata.mission_id != payload.header.mission_id {
        bail!(
            "autosave {:?} mission mismatch: manifest {}, payload {}",
            metadata.filename,
            metadata.mission_id,
            payload.header.mission_id
        );
    }
    if metadata.version != payload.header.version {
        bail!(
            "autosave {:?} version mismatch: manifest {}, payload {}",
            metadata.filename,
            metadata.version,
            payload.header.version
        );
    }
    let timestamp = metadata.timestamp.parse::<u64>().with_context(|| {
        format!(
            "autosave {:?} manifest timestamp is not an unsigned integer",
            metadata.filename
        )
    })?;
    if timestamp != payload.header.timestamp_unix {
        bail!(
            "autosave {:?} timestamp mismatch: manifest {}, payload {}",
            metadata.filename,
            timestamp,
            payload.header.timestamp_unix
        );
    }
    Ok(())
}

fn validate_generated_filename(filename: &str) -> Result<()> {
    if !crate::savegame::is_generated_autosave_filename(filename) {
        bail!("invalid autosave storage filename {filename:?}");
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn manifest_path(save_directory: &str) -> PathBuf {
    Path::new(save_directory).join(AUTOSAVE_MANIFEST_FILE)
}

#[cfg(not(target_arch = "wasm32"))]
fn load_manifest(save_directory: &str) -> Result<Option<AutosaveManifest>> {
    let path = manifest_path(save_directory);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let manifest: AutosaveManifest =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    manifest.validate()?;
    Ok(Some(manifest))
}

#[cfg(not(target_arch = "wasm32"))]
fn persist_manifest(save_directory: &str, manifest: &AutosaveManifest) -> Result<()> {
    manifest.validate()?;
    let bytes = serde_json::to_vec_pretty(manifest).context("serializing autosave manifest")?;
    crate::save_file::atomic_write(&manifest_path(save_directory), &bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn persist_payload(
    save_directory: &str,
    filename: &str,
    payload: &GameSaveFile,
    thumbnail: Option<&Thumbnail>,
) -> Result<()> {
    validate_generated_filename(filename)?;
    let path = Path::new(save_directory)
        .join(filename)
        .with_extension("json");
    payload.write_to(&path)?;
    if let Some(thumbnail) = thumbnail {
        let path = Path::new(save_directory).join(format!("{filename}_thumb.png"));
        if let Err(error) = thumbnail.write_to(&path) {
            // A thumbnail is auxiliary: never discard a valid recovery point
            // because its preview could not be written, but do report it.
            tracing::warn!(
                filename,
                "autosave thumbnail could not be written: {error:#}"
            );
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_payload(save_directory: &str, filename: &str) -> Result<GameSaveFile> {
    validate_generated_filename(filename)?;
    GameSaveFile::read_from(
        &Path::new(save_directory)
            .join(filename)
            .with_extension("json"),
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn payload_exists(save_directory: &str, filename: &str) -> Result<bool> {
    validate_generated_filename(filename)?;
    Ok(Path::new(save_directory)
        .join(filename)
        .with_extension("json")
        .exists())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_thumbnail(save_directory: &str, filename: &str) -> Result<Option<Thumbnail>> {
    validate_generated_filename(filename)?;
    let path = Path::new(save_directory).join(format!("{filename}_thumb.png"));
    if !path.exists() {
        return Ok(None);
    }
    Thumbnail::read_from(&path).map(Some)
}

#[cfg(not(target_arch = "wasm32"))]
fn remove_payload(save_directory: &str, filename: &str) -> Result<()> {
    validate_generated_filename(filename)?;
    for path in [
        Path::new(save_directory)
            .join(filename)
            .with_extension("json"),
        Path::new(save_directory).join(format!("{filename}_thumb.png")),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("removing {}", path.display()));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn garbage_collect_orphans(save_directory: &str, manifest: &AutosaveManifest) -> Result<()> {
    let referenced: BTreeSet<_> = manifest
        .saves
        .iter()
        .map(|save| save.filename.as_str())
        .collect();
    let directory = Path::new(save_directory);
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("enumerating autosave directory {}", directory.display())
            });
        }
    };
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "reading an entry from autosave directory {}",
                directory.display()
            )
        })?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading file type for {}", entry.path().display()))?;
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let orphan = generated_filename_from_storage_name(name)
            .is_some_and(|filename| !referenced.contains(filename));
        let interrupted_stage = name.starts_with(".robin-autosave-staging-");
        if orphan || interrupted_stage {
            std::fs::remove_file(entry.path()).with_context(|| {
                format!(
                    "removing orphan autosave artifact {}",
                    entry.path().display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn generated_filename_from_storage_name(name: &str) -> Option<&str> {
    let filename = name
        .strip_suffix("_thumb.png")
        .or_else(|| name.strip_suffix(".json"))?;
    crate::savegame::is_generated_autosave_filename(filename).then_some(filename)
}

fn browser_autosave_filename_from_key<'a>(namespace: &str, key: &'a str) -> Option<&'a str> {
    let payload_prefix = format!("{namespace}.payload.");
    let thumbnail_prefix = format!("{namespace}.thumbnail.");
    let filename = key
        .strip_prefix(&payload_prefix)
        .or_else(|| key.strip_prefix(&thumbnail_prefix))?;
    crate::savegame::is_generated_autosave_filename(filename).then_some(filename)
}

// Browser autosaves use compressed, checksummed localStorage records. The
// browser's storage API commits each key atomically, and the separate manifest
// is published only after the immutable payload key succeeds.
#[cfg(target_arch = "wasm32")]
const BROWSER_STORAGE_PREFIX: &str = "robinhood.autosave.v1";

#[derive(Debug, Serialize, Deserialize)]
struct BrowserBlob {
    version: u32,
    sha256: String,
    compressed_base64: String,
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage> {
    web_sys::window()
        .context("browser window is unavailable")?
        .local_storage()
        .map_err(|error| anyhow::anyhow!("accessing localStorage failed: {error:?}"))?
        .context("browser localStorage is disabled")
}

#[cfg(target_arch = "wasm32")]
fn browser_namespace(save_directory: &str) -> String {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(save_directory.as_bytes());
    format!(
        "{BROWSER_STORAGE_PREFIX}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
    )
}

#[cfg(target_arch = "wasm32")]
fn browser_key(save_directory: &str, suffix: &str) -> String {
    format!("{}.{}", browser_namespace(save_directory), suffix)
}

fn encode_browser_blob<T: Serialize>(value: &T) -> Result<String> {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    let json = serde_json::to_vec(value).context("serializing browser autosave value")?;
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(&json), 3)
        .context("compressing browser autosave value")?;
    let blob = BrowserBlob {
        version: 1,
        sha256: hex::encode(Sha256::digest(&json)),
        compressed_base64: base64::engine::general_purpose::STANDARD.encode(compressed),
    };
    serde_json::to_string(&blob).context("serializing browser autosave envelope")
}

fn decode_browser_blob<T: for<'de> Deserialize<'de>>(encoded: &str) -> Result<T> {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    let blob: BrowserBlob =
        serde_json::from_str(encoded).context("parsing browser autosave envelope")?;
    if blob.version != 1 {
        bail!(
            "unsupported browser autosave envelope version {}",
            blob.version
        );
    }
    let compressed = base64::engine::general_purpose::STANDARD
        .decode(blob.compressed_base64)
        .context("decoding browser autosave base64")?;
    let json = zstd::stream::decode_all(std::io::Cursor::new(compressed))
        .context("decompressing browser autosave value")?;
    let actual = hex::encode(Sha256::digest(&json));
    if actual != blob.sha256 {
        bail!(
            "browser autosave checksum mismatch: expected {}, got {actual}",
            blob.sha256
        );
    }
    serde_json::from_slice(&json).context("parsing browser autosave value")
}

#[cfg(target_arch = "wasm32")]
fn load_manifest(save_directory: &str) -> Result<Option<AutosaveManifest>> {
    let storage = browser_storage()?;
    let key = browser_key(save_directory, "manifest");
    let Some(encoded) = storage
        .get_item(&key)
        .map_err(|error| anyhow::anyhow!("reading browser autosave manifest failed: {error:?}"))?
    else {
        return Ok(None);
    };
    let manifest: AutosaveManifest = decode_browser_blob(&encoded)?;
    manifest.validate()?;
    Ok(Some(manifest))
}

#[cfg(target_arch = "wasm32")]
fn persist_manifest(save_directory: &str, manifest: &AutosaveManifest) -> Result<()> {
    manifest.validate()?;
    let storage = browser_storage()?;
    let key = browser_key(save_directory, "manifest");
    let value = encode_browser_blob(manifest)?;
    storage
        .set_item(&key, &value)
        .map_err(|error| anyhow::anyhow!("publishing browser autosave manifest failed: {error:?}"))
}

#[cfg(target_arch = "wasm32")]
fn persist_payload(
    save_directory: &str,
    filename: &str,
    payload: &GameSaveFile,
    thumbnail: Option<&Thumbnail>,
) -> Result<()> {
    validate_generated_filename(filename)?;
    let storage = browser_storage()?;
    let payload_key = browser_key(save_directory, &format!("payload.{filename}"));
    let payload_value = encode_browser_blob(payload)?;
    storage
        .set_item(&payload_key, &payload_value)
        .map_err(|error| anyhow::anyhow!("writing browser autosave payload failed: {error:?}"))?;
    if let Some(thumbnail) = thumbnail {
        let thumbnail_key = browser_key(save_directory, &format!("thumbnail.{filename}"));
        match encode_browser_blob(thumbnail) {
            Ok(thumbnail_value) => {
                if let Err(error) = storage.set_item(&thumbnail_key, &thumbnail_value) {
                    // Keep the payload loadable when browser quota permits
                    // the game state but not its optional preview image.
                    tracing::warn!(
                        filename,
                        "browser autosave thumbnail could not be written: {error:?}"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    filename,
                    "browser autosave thumbnail could not be encoded: {error:#}"
                );
            }
        }
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_payload(save_directory: &str, filename: &str) -> Result<GameSaveFile> {
    validate_generated_filename(filename)?;
    let storage = browser_storage()?;
    let key = browser_key(save_directory, &format!("payload.{filename}"));
    let encoded = storage
        .get_item(&key)
        .map_err(|error| anyhow::anyhow!("reading browser autosave payload failed: {error:?}"))?
        .with_context(|| format!("browser autosave payload {filename:?} is missing"))?;
    decode_browser_blob(&encoded)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn payload_exists(save_directory: &str, filename: &str) -> Result<bool> {
    validate_generated_filename(filename)?;
    let storage = browser_storage()?;
    storage
        .get_item(&browser_key(save_directory, &format!("payload.{filename}")))
        .map(|value| value.is_some())
        .map_err(|error| anyhow::anyhow!("checking browser autosave payload failed: {error:?}"))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_thumbnail(save_directory: &str, filename: &str) -> Result<Option<Thumbnail>> {
    validate_generated_filename(filename)?;
    let storage = browser_storage()?;
    let key = browser_key(save_directory, &format!("thumbnail.{filename}"));
    let Some(encoded) = storage
        .get_item(&key)
        .map_err(|error| anyhow::anyhow!("reading browser autosave thumbnail failed: {error:?}"))?
    else {
        return Ok(None);
    };
    decode_browser_blob(&encoded).map(Some)
}

#[cfg(target_arch = "wasm32")]
fn remove_payload(save_directory: &str, filename: &str) -> Result<()> {
    validate_generated_filename(filename)?;
    let storage = browser_storage()?;
    for suffix in [
        format!("payload.{filename}"),
        format!("thumbnail.{filename}"),
    ] {
        storage
            .remove_item(&browser_key(save_directory, &suffix))
            .map_err(|error| anyhow::anyhow!("removing browser autosave failed: {error:?}"))?;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn garbage_collect_orphans(save_directory: &str, manifest: &AutosaveManifest) -> Result<()> {
    let referenced: BTreeSet<_> = manifest
        .saves
        .iter()
        .map(|save| save.filename.as_str())
        .collect();
    let storage = browser_storage()?;
    let namespace = browser_namespace(save_directory);
    let mut remove = Vec::new();
    for index in 0..storage.length().map_err(|error| {
        anyhow::anyhow!("enumerating browser autosave storage failed: {error:?}")
    })? {
        let Some(key) = storage.key(index).map_err(|error| {
            anyhow::anyhow!("reading browser autosave storage key failed: {error:?}")
        })?
        else {
            continue;
        };
        if let Some(filename) = browser_autosave_filename_from_key(&namespace, &key)
            && !referenced.contains(filename)
        {
            remove.push(key);
        }
    }
    for key in remove {
        storage.remove_item(&key).map_err(|error| {
            anyhow::anyhow!("removing orphan browser autosave {key:?} failed: {error:?}")
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept_schedule(
        schedule: &mut AutosaveSchedule,
        mission_id: u32,
        frame: u64,
        backgrounded: bool,
        terminal: bool,
    ) -> Option<AutosaveReason> {
        let reason = schedule.preview(mission_id, frame, backgrounded, terminal);
        if let Some(reason) = reason {
            schedule.commit(mission_id, frame, reason);
        }
        reason
    }

    #[test]
    fn schedule_uses_mission_boundaries_and_five_active_minutes() {
        let mut schedule = AutosaveSchedule::default();
        assert_eq!(
            accept_schedule(&mut schedule, 17, 0, false, false),
            Some(AutosaveReason::MissionTransition)
        );
        assert_eq!(
            accept_schedule(
                &mut schedule,
                17,
                AUTOSAVE_INTERVAL_FRAMES - 1,
                false,
                false
            ),
            None
        );
        assert_eq!(
            accept_schedule(&mut schedule, 17, AUTOSAVE_INTERVAL_FRAMES, false, false),
            Some(AutosaveReason::Periodic)
        );
        assert_eq!(
            accept_schedule(&mut schedule, 17, AUTOSAVE_INTERVAL_FRAMES, true, false),
            Some(AutosaveReason::Backgrounded)
        );
        assert_eq!(
            accept_schedule(&mut schedule, 17, AUTOSAVE_INTERVAL_FRAMES, true, false),
            None,
            "repeated background notifications while paused must coalesce"
        );
        assert_eq!(
            accept_schedule(&mut schedule, 17, AUTOSAVE_INTERVAL_FRAMES, false, true),
            Some(AutosaveReason::MissionTransition)
        );
        assert_eq!(
            accept_schedule(&mut schedule, 17, AUTOSAVE_INTERVAL_FRAMES, false, true),
            None
        );
        assert_eq!(
            accept_schedule(&mut schedule, 22, 0, false, false),
            Some(AutosaveReason::MissionTransition)
        );
    }

    #[test]
    fn session_policy_excludes_disabled_multiplayer_replay_and_headless_runs() {
        assert!(session_allows_autosave(true, false, false, false));
        assert!(!session_allows_autosave(false, false, false, false));
        assert!(!session_allows_autosave(true, true, false, false));
        assert!(!session_allows_autosave(true, false, true, false));
        assert!(!session_allows_autosave(true, false, false, true));
    }

    #[test]
    fn schedule_changes_only_after_an_accepted_snapshot() {
        let mut schedule = AutosaveSchedule::default();
        assert_eq!(
            schedule.preview(17, 0, false, false),
            Some(AutosaveReason::MissionTransition)
        );
        assert_eq!(
            schedule.preview(17, 0, false, false),
            Some(AutosaveReason::MissionTransition),
            "an interrupted capture must remain due"
        );
        schedule.commit(17, 0, AutosaveReason::MissionTransition);
        assert_eq!(schedule.preview(17, 0, false, false), None);
    }

    #[test]
    fn transient_snapshot_unavailability_does_not_consume_a_lifecycle_request() {
        let mut coordinator = AutosaveCoordinator::default();
        assert_eq!(coordinator.plan(true, false, 17, 0, true, false), None);
        assert_eq!(
            coordinator.plan(true, true, 17, 0, true, false),
            Some(AutosaveReason::MissionTransition)
        );
    }

    #[test]
    fn staged_manifest_evicts_oldest_generation_and_retains_three() {
        let mut manager = SaveGameManager::new("unused".into());
        for ordinal in 0..3 {
            let mut save = SaveGame::new(format!("Autosave_100_{ordinal:04}"), "Mission".into(), 1);
            save.timestamp = (100 + ordinal).to_string();
            manager.saves.push(save);
        }
        let mut newest = SaveGame::new("Autosave_200_0000".into(), "Mission".into(), 1);
        newest.timestamp = "200".into();
        let existing = AutosaveManifest {
            version: AUTOSAVE_MANIFEST_VERSION,
            saves: manager.saves.clone(),
        };
        let (manifest, evicted) = staged_manifest(existing, newest);
        assert_eq!(manifest.saves.len(), AUTOSAVE_SLOT_COUNT);
        assert_eq!(evicted, vec!["Autosave_100_0000"]);
        assert_eq!(manifest.saves.last().unwrap().filename, "Autosave_200_0000");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_manifest_commit_is_round_trippable() {
        let directory = tempfile::tempdir().unwrap();
        let mut save = SaveGame::new("Autosave_1_0000".into(), "Mission".into(), 1);
        save.timestamp = "1".into();
        let manifest = AutosaveManifest {
            version: AUTOSAVE_MANIFEST_VERSION,
            saves: vec![save],
        };
        persist_manifest(directory.path().to_str().unwrap(), &manifest).unwrap();
        assert_eq!(
            load_manifest(directory.path().to_str().unwrap()).unwrap(),
            Some(manifest)
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn shutdown_drains_accepted_jobs_and_cleans_the_rotated_payload() {
        use crate::game::Game;
        use robin_engine::campaign::Campaign;

        let directory = tempfile::tempdir().unwrap();
        let save_directory = directory.path().to_string_lossy().into_owned();
        let mut assets = robin_engine::engine::LevelAssets::new();
        let engine = Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets).unwrap();
        let host = Host::scratch(800.0, 600.0);
        let game = Game::default();
        let mut coordinator = AutosaveCoordinator::default();
        for ordinal in 0..4 {
            let filename = format!("Autosave_1_{ordinal:04}");
            let payload = GameSaveFile::capture(&engine, &host, 1, "Mission".into());
            let mut metadata = SaveGame::new(filename.clone(), "Mission".into(), 1);
            metadata.version = payload.header.version;
            metadata.timestamp = payload.header.timestamp_unix.to_string();
            coordinator
                .command_tx
                .send(Some(AutosaveJob {
                    save_directory: save_directory.clone(),
                    filename,
                    payload,
                    metadata,
                    thumbnail: None,
                    manifest_seed: AutosaveManifest::default(),
                    reason: AutosaveReason::MissionTransition,
                }))
                .unwrap();
        }

        let mut manager = SaveGameManager::new(save_directory.clone());
        let results = coordinator.shutdown_and_poll(&mut manager);
        assert_eq!(results.len(), 4);
        assert_eq!(manager.saves.len(), AUTOSAVE_SLOT_COUNT);
        assert!(!payload_exists(&save_directory, "Autosave_1_0000").unwrap());
        for ordinal in 1..4 {
            assert!(payload_exists(&save_directory, &format!("Autosave_1_{ordinal:04}")).unwrap());
        }
    }

    #[test]
    fn browser_blob_codec_preserves_u64_unicode_and_rejects_corruption() {
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct PortableFixture {
            timestamp: u64,
            label: String,
        }
        let fixture = PortableFixture {
            timestamp: u64::MAX,
            label: "Forêt de Sherwood 🏹".to_owned(),
        };
        let encoded = encode_browser_blob(&fixture).unwrap();
        let decoded: PortableFixture = decode_browser_blob(&encoded).unwrap();
        assert_eq!(decoded, fixture);

        let mut envelope: BrowserBlob = serde_json::from_str(&encoded).unwrap();
        let replacement = if envelope.sha256.starts_with('0') {
            "1"
        } else {
            "0"
        };
        envelope.sha256.replace_range(0..1, replacement);
        let corrupted = serde_json::to_string(&envelope).unwrap();
        let error = decode_browser_blob::<PortableFixture>(&corrupted).unwrap_err();
        assert!(error.to_string().contains("checksum mismatch"));
    }

    #[test]
    fn browser_orphan_scanner_accepts_only_exact_namespaced_generation_keys() {
        let namespace = "robinhood.autosave.v1.profile";
        assert_eq!(
            browser_autosave_filename_from_key(
                namespace,
                "robinhood.autosave.v1.profile.payload.Autosave_1_0000"
            ),
            Some("Autosave_1_0000")
        );
        assert_eq!(
            browser_autosave_filename_from_key(
                namespace,
                "robinhood.autosave.v1.profile.thumbnail.Autosave_1_0000"
            ),
            Some("Autosave_1_0000")
        );
        for hostile in [
            "robinhood.autosave.v1.other.payload.Autosave_1_0000",
            "robinhood.autosave.v1.profile.payload.Autosave_1_0000.extra",
            "robinhood.autosave.v1.profile.payload.Autosave_1_../../Continue",
            "robinhood.autosave.v1.profile.manifest",
        ] {
            assert_eq!(
                browser_autosave_filename_from_key(namespace, hostile),
                None,
                "{hostile}"
            );
        }
    }

    #[test]
    fn commit_never_publishes_before_payload_and_never_cleans_before_manifest() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let existing = AutosaveManifest::default();
        let metadata = SaveGame::new("Autosave_1_0000".into(), "Mission".into(), 1);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let payload_calls = calls.clone();
        let manifest_calls = calls.clone();
        let cleanup_calls = calls.clone();
        commit_generation(
            existing,
            metadata,
            move || {
                payload_calls.borrow_mut().push("payload");
                Ok(())
            },
            move |_| {
                manifest_calls.borrow_mut().push("manifest");
                bail!("simulated publication interruption")
            },
            move |_| {
                cleanup_calls.borrow_mut().push("cleanup");
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(&*calls.borrow(), &["payload", "manifest"]);
    }

    #[test]
    fn failed_payload_never_attempts_manifest_publication() {
        let calls = std::cell::RefCell::new(Vec::new());
        commit_generation(
            AutosaveManifest::default(),
            SaveGame::new("Autosave_1_0000".into(), "Mission".into(), 1),
            || {
                calls.borrow_mut().push("payload");
                bail!("simulated payload interruption")
            },
            |_| {
                calls.borrow_mut().push("manifest");
                Ok(())
            },
            |_| {
                calls.borrow_mut().push("cleanup");
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(&*calls.borrow(), &["payload"]);
    }

    #[test]
    fn published_rotation_survives_cleanup_failure_and_retains_exactly_three() {
        let mut saves = Vec::new();
        for ordinal in 0..3 {
            saves.push(SaveGame::new(
                format!("Autosave_1_{ordinal:04}"),
                "Mission".into(),
                1,
            ));
        }
        let calls = std::cell::RefCell::new(Vec::new());
        let manifest = commit_generation(
            AutosaveManifest {
                version: AUTOSAVE_MANIFEST_VERSION,
                saves,
            },
            SaveGame::new("Autosave_2_0000".into(), "Mission".into(), 1),
            || {
                calls.borrow_mut().push("payload");
                Ok(())
            },
            |_| {
                calls.borrow_mut().push("manifest");
                Ok(())
            },
            |filename| {
                calls.borrow_mut().push("cleanup");
                assert_eq!(filename, "Autosave_1_0000");
                bail!("simulated cleanup interruption")
            },
        )
        .unwrap();
        assert_eq!(manifest.saves.len(), AUTOSAVE_SLOT_COUNT);
        assert_eq!(&*calls.borrow(), &["payload", "manifest", "cleanup"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recovery_removes_only_unpublished_autosave_artifacts() {
        let directory = tempfile::tempdir().unwrap();
        let save_directory = directory.path().to_str().unwrap();
        persist_manifest(save_directory, &AutosaveManifest::default()).unwrap();
        for name in [
            "Autosave_1_0000.json",
            "Autosave_1_0000_thumb.png",
            ".robin-autosave-staging-interrupted",
            "Autosave_notes.json",
            ".robin-atomic-staging-manual",
        ] {
            std::fs::write(directory.path().join(name), b"hostile interruption").unwrap();
        }

        let mut manager = SaveGameManager::new(save_directory.to_owned());
        load_into_manager(&mut manager).unwrap();
        for removed in [
            "Autosave_1_0000.json",
            "Autosave_1_0000_thumb.png",
            ".robin-autosave-staging-interrupted",
        ] {
            assert!(!directory.path().join(removed).exists(), "{removed}");
        }
        for retained in ["Autosave_notes.json", ".robin-atomic-staging-manual"] {
            assert!(directory.path().join(retained).exists(), "{retained}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn missing_published_payload_fails_closed_and_hides_stale_index_row() {
        let directory = tempfile::tempdir().unwrap();
        let save_directory = directory.path().to_str().unwrap();
        let mut autosave = SaveGame::new("Autosave_1_0000".into(), "Mission".into(), 1);
        autosave.timestamp = "1".into();
        persist_manifest(
            save_directory,
            &AutosaveManifest {
                version: AUTOSAVE_MANIFEST_VERSION,
                saves: vec![autosave.clone()],
            },
        )
        .unwrap();
        let manual = SaveGame::new("Savegame_001".into(), "Manual".into(), 1);
        let mut manager = SaveGameManager::new(save_directory.to_owned());
        manager.saves = vec![manual.clone(), autosave];

        let error = load_into_manager(&mut manager).unwrap_err();
        assert!(error.to_string().contains("validating published autosave"));
        assert_eq!(manager.saves, vec![manual]);
    }
}

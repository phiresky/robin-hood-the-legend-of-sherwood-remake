//! Full game save payload — captures a snapshot of Engine + Campaign state.
//!
//! Logical fields are serialized through serde.  The format is JSON today
//! for debuggability; switching to a compact binary format (e.g. bitcode)
//! is a future option once the set of serialized fields stabilizes.  A
//! 4-byte "RHSG" magic plus a format version are stored in the header.
//! Readers validate that header before deserializing the version-specific
//! payload.
//!
//! ## What gets serialized
//!
//! - Camera, mission win/loss, frame counter, engine locks, speed
//! - Shield protection, cheat flags, script globals
//! - Entities (PCs, NPCs, animals, mobile elements, animations, FX)
//! - Quick-select groups, selected PCs, fighter/soldier indices
//! - AI global state, messenger queue, short briefings
//! - FastFindGrid, pathfinder state, minimap, ground marks, titbits
//! - Sequence manager, shadow polygon, sound state
//! - Mission stats
//! - Campaign state (missions, gang, values, ARES, reservists, etc.)
//!
//! Not serialized (transient or re-derivable):
//! - DebugFlags, InputState, WeatherState
//! - Rendering surface handles, DrawManager, FrameHolder, SpriteScriptor
//! - Console and immutable mission-script program/native attachments (restored
//!   from `LevelAssets` after load)
//! - FailedPathRequest list (just a 100-frame grace timer)
//! - script_*_count / script_location_positions (recomputed from level data)
//! - hiking_paths, profile_manager (Arc'd immutable data reloaded at level init)

use crate::host::Host;
#[cfg(test)]
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, LevelAssets, SnapshotRestoreError};
use std::fs;
use std::path::{Path, PathBuf};
use web_time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::game::GamePersistentState;
use crate::sound::SoundManager;
use sha2::{Digest, Sha256};

/// Stable identity of the complete runtime payload captured by a save.
///
/// This intentionally excludes [`SaveHeader`] metadata such as timestamps and
/// display text. Two payloads compare equal only when their engine, sound, and
/// host-owned persistent game state serialize identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ReplaySaveIdentity([u8; 32]);

/// Complete in-mission state needed to reproduce the effect of loading a save.
///
/// Replay playback keeps these snapshots in memory at save-marker boundaries.
/// Restoring only `Engine` is insufficient: normal save loading also restores
/// sound and persistent `Game` flags and runs engine/host post-load fixups.
#[derive(Clone)]
pub(crate) struct GameRuntimeSnapshot {
    engine: Engine,
    sound: SoundManager,
    game_persistent: GamePersistentState,
}

impl GameRuntimeSnapshot {
    pub(crate) fn capture(engine: &Engine, host: &Host, game: &crate::game::Game) -> Self {
        let mut game_persistent = game.persistent.clone();
        game_persistent.draw_hidden = host.input.draw_hidden;
        Self {
            engine: engine.clone(),
            sound: host.audio.sound.clone(),
            game_persistent,
        }
    }

    pub(crate) fn replay_identity(&self) -> Result<ReplaySaveIdentity> {
        replay_save_identity(&self.engine, &self.sound, &self.game_persistent)
    }

    pub(crate) fn apply_to_with_game(
        self,
        engine: &mut Engine,
        host: &mut Host,
        game: &mut crate::game::Game,
        assets: &LevelAssets,
    ) -> std::result::Result<(), SnapshotRestoreError> {
        let draw_hidden = self.game_persistent.draw_hidden;
        *engine = Engine::restore_from_snapshot(&mut host.engine_display, self.engine, assets)?;
        host.audio.sound = self.sound;
        host.audio.sound.after_load(&engine.sound_sim().sources);
        host.post_load_reset();
        game.persistent = self.game_persistent;
        host.input.draw_hidden = draw_hidden;
        Ok(())
    }
}

fn replay_save_identity(
    engine: &Engine,
    sound: &SoundManager,
    game_persistent: &GamePersistentState,
) -> Result<ReplaySaveIdentity> {
    replay_identity_digest(&(engine, sound, game_persistent))
}

fn replay_identity_digest<T: Serialize + ?Sized>(value: &T) -> Result<ReplaySaveIdentity> {
    // Engine state contains maps keyed by domain types. Reuse the same
    // JSON-compatible serde conversion as the engine-dump endpoints so those
    // keys receive their stable string representation before JSON encoding.
    let value = crate::json_value::to_json_value(value)
        .context("converting replay save identity to string-keyed JSON")?;
    let bytes = serde_json::to_vec(&value).context("serializing replay save identity")?;
    Ok(ReplaySaveIdentity(Sha256::digest(bytes).into()))
}

// ─── Thumbnail ───────────────────────────────────────────────────────

/// Default thumbnail dimensions.  Downsampled to a small fixed size to
/// keep the sibling file tiny.
pub const THUMB_WIDTH: u16 = 480;
pub const THUMB_HEIGHT: u16 = 360;

/// A small RGB565 thumbnail of the last rendered frame.
///
/// Written to a sibling PNG file (`<name>_thumb.png`) next to the save payload by
/// [`SaveGameManager::thumb_path`] / [`Thumbnail::write_to`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thumbnail {
    pub width: u16,
    pub height: u16,
    /// Row-major RGB565 pixels, length = `width × height`.
    pub pixels: Vec<u16>,
}

impl Thumbnail {
    /// Build a thumbnail from a raw RGB565 pixel buffer.
    pub fn from_pixels(width: u16, height: u16, pixels: Vec<u16>) -> Option<Self> {
        if pixels.len() != width as usize * height as usize {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels,
        })
    }

    /// Build a thumbnail by nearest-neighbour downsampling an RGBA8 frame.
    pub fn from_rgba_downscaled(
        src_width: u32,
        src_height: u32,
        rgba: &[u8],
        width: u16,
        height: u16,
    ) -> Result<Self> {
        let expected = src_width as usize * src_height as usize * 4;
        if rgba.len() != expected {
            bail!(
                "thumbnail source RGBA length mismatch: expected {}, got {}",
                expected,
                rgba.len()
            );
        }
        if src_width == 0 || src_height == 0 || width == 0 || height == 0 {
            bail!(
                "thumbnail dimensions must be non-zero: source={}x{}, target={}x{}",
                src_width,
                src_height,
                width,
                height
            );
        }

        let target_w = width as usize;
        let target_h = height as usize;
        let src_w = src_width as usize;
        let src_h = src_height as usize;
        let mut pixels = Vec::with_capacity(target_w * target_h);
        for ty in 0..target_h {
            let sy = ty * src_h / target_h;
            for tx in 0..target_w {
                let sx = tx * src_w / target_w;
                let off = (sy * src_w + sx) * 4;
                pixels.push(rgb888_to_rgb565(rgba[off], rgba[off + 1], rgba[off + 2]));
            }
        }

        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Write the thumbnail to `path` as a normal 8-bit RGB PNG file.
    pub fn write_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating thumbnail directory {}", parent.display()))?;
        }

        let file = fs::File::create(path)
            .with_context(|| format!("creating thumbnail {}", path.display()))?;
        let writer = std::io::BufWriter::new(file);
        let mut encoder = png::Encoder::new(writer, self.width as u32, self.height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .with_context(|| format!("writing thumbnail PNG header {}", path.display()))?;
        writer
            .write_image_data(&self.rgb888_pixels())
            .with_context(|| format!("writing thumbnail PNG data {}", path.display()))
    }

    /// Read a thumbnail written by [`write_to`](Self::write_to).
    pub fn read_from(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("reading thumbnail {}", path.display()))?;
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder
            .read_info()
            .with_context(|| format!("decoding thumbnail PNG header {}", path.display()))?;
        let mut buf = vec![
            0;
            reader.output_buffer_size().ok_or_else(|| anyhow::anyhow!(
                "unknown thumbnail PNG output size for {}",
                path.display()
            ))?
        ];
        let info = reader
            .next_frame(&mut buf)
            .with_context(|| format!("decoding thumbnail PNG frame {}", path.display()))?;
        if info.bit_depth != png::BitDepth::Eight {
            bail!(
                "unsupported thumbnail PNG bit depth {:?} for {}",
                info.bit_depth,
                path.display()
            );
        }
        let data = &buf[..info.buffer_size()];
        let expected_pixels = info.width as usize * info.height as usize;
        let mut pixels = Vec::with_capacity(expected_pixels);
        match info.color_type {
            png::ColorType::Rgb => {
                for chunk in data.as_chunks::<3>().0 {
                    pixels.push(rgb888_to_rgb565(chunk[0], chunk[1], chunk[2]));
                }
            }
            png::ColorType::Rgba => {
                for chunk in data.as_chunks::<4>().0 {
                    pixels.push(rgb888_to_rgb565(chunk[0], chunk[1], chunk[2]));
                }
            }
            other => {
                bail!(
                    "unsupported thumbnail PNG color type {:?} for {}",
                    other,
                    path.display()
                );
            }
        }
        if pixels.len() != expected_pixels {
            bail!(
                "thumbnail PNG pixel count mismatch for {}: expected {}, got {}",
                path.display(),
                expected_pixels,
                pixels.len()
            );
        }
        Ok(Self {
            width: info.width.try_into().with_context(|| {
                format!("thumbnail PNG width exceeds u16 for {}", path.display())
            })?,
            height: info.height.try_into().with_context(|| {
                format!("thumbnail PNG height exceeds u16 for {}", path.display())
            })?,
            pixels,
        })
    }

    fn rgb888_pixels(&self) -> Vec<u8> {
        let mut rgb = Vec::with_capacity(self.pixels.len() * 3);
        for &pixel in &self.pixels {
            rgb.extend_from_slice(&rgb565_to_rgb888(pixel));
        }
        rgb
    }
}

/// Truncating 565→888 expansion — shared with the renderer so PNG
/// thumbnails show the same colours as the live frame they capture.
fn rgb565_to_rgb888(pixel: u16) -> [u8; 3] {
    let (r, g, b) = crate::renderer::rgb565_to_rgb8(pixel);
    [r, g, b]
}

fn rgb888_to_rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | (b as u16 >> 3)
}

// ─── Header ──────────────────────────────────────────────────────────

/// Magic bytes at the start of every save file.
pub const SAVE_MAGIC: &str = "RHSG";

/// Current save format version.
///
/// Bumped on every incompatible change to the serialized fields.
/// The counter starts from 1.
///
/// ## History
/// - **v1**: initial Rust format. `ElementData.sprite` was skipped; the
///   embedded `PositionInterface` + sprite animation state did not persist.
/// - **v2** (2026-04-20, PI-into-Sprite refactor): `ElementData.sprite` is
///   now fully serialized. The saved `Sprite` carries its `PositionInterface`
///   (position / direction / layer / sector / material) plus the animation
///   state (`current_row`, `current_frame`, `frame_count`, `last_action`,
///   …). Arc-shared script caches re-hydrate from the sprite cache on load.
/// - **v3** (2026-04-29, engine-state cleanup): small sprite/titbit runtime
///   values that still live inside engine-owned structs now serialize instead
///   of resetting through `#[serde(skip)]`: sprite water-titbit cadence,
///   sprite bbox/center, and titbit blink/dotted-line counters.
/// - **v4** (2026-04-29, engine-state cleanup): door after-patch lock bits
///   serialize with the active lock bits so patch swap/revert behavior
///   survives save/load.
/// - **v5** (2026-04-29, engine-state cleanup): AI door/building caches
///   serialize, including live building occupant lists and soldier-register
///   mappings.
/// - **v6** (2026-04-29, engine-state cleanup): NPC patrol route IDs
///   serialize with AI controller state so alert-route switches survive
///   save/load.
/// - **v7** (2026-04-29, engine-state cleanup): NPC actor-script
///   FilterAIEvent override metadata serializes with the bound AI
///   controller.
/// - **v8** (2026-04-29, engine-state cleanup): NPC initial guard-post
///   position and facing direction serialize with AI controller state.
/// - **v9** (2026-04-29, engine-state cleanup): NPC focus-sync gate
///   state serializes so explicit focus clears are not undone after load.
/// - **v10** (2026-04-29, engine-state cleanup): AI think recursion
///   depth serializes with controller state instead of hiding behind
///   `#[serde(skip)]`.
/// - **v11** (2026-04-29, engine-state cleanup): pending NPC MYTALK
///   callback flags and instant music-change latches serialize with AI
///   controller state.
/// - **v12** (2026-04-29, engine-state cleanup): NPC AI frame/building
///   context caches and current max-visibility cache serialize with the
///   controller instead of resetting through skipped fields.
/// - **v13** (2026-04-29, engine-state cleanup): first batch of AI
///   pending work queues serializes with controller state: patrol
///   direction broadcasts, order intents, queued stimuli, cross-NPC
///   actions, and self-stimuli.
/// - **v14** (2026-04-29, engine-state cleanup): AI pending engine
///   mutation requests for halt/deactivate/swordfight/detectable updates
///   serialize with controller state.
/// - **v15** (2026-04-29, engine-state cleanup): AI pending target/focus
///   requests serialize with controller state.
/// - **v16** (2026-04-29, engine-state cleanup): AI pending state-change,
///   view recovery, detectable-object recovery, and guarded-PC requests
///   serialize with controller state.
/// - **v17** (2026-04-30, engine-state cleanup): AI pending sequence,
///   posture, waypoint-script, panic, and script-seek requests serialize
///   with controller state.
/// - **v18** (2026-04-30, engine-state cleanup): VM/native pending nested
///   script calls serialize instead of being silently dropped.
/// - **v19** (2026-04-30, engine-state cleanup): Tick side-effect queues
///   serialize/hash if they ever leak into an engine snapshot.
/// - **v20** (2026-04-30, engine-state cleanup): AI entity-view and
///   sight-obstacle dispatch caches serialize/hash with global AI state.
/// - **v21** (2026-04-30, engine-state cleanup): Script managers serialize
///   their immutable decoded program instead of relying on skipped reattach.
/// - **v22** (2026-04-30, engine-state cleanup): Script native hosts
///   serialize their profile-manager attachment with host state.
/// - **v23** (2026-04-30, engine-state cleanup): AI controllers no longer
///   cache per-NPC hiking-path Arcs; path data is threaded through AI context
///   and script host static data.
/// - **v24** (2026-04-30, engine-state cleanup): enemy AI pending archery
///   release requests and sword-strike cooldowns are now serialized as
///   simulation state.
/// - **v25** (2026-04-30, engine-state cleanup): enemy AI level-load profile
///   and combat caches serialize with the owning AI state.
/// - **v26** (2026-04-30, engine-state cleanup): in-flight actor sweep,
///   jump, rider-charge, push-followup, and roll side-effect state serializes
///   with actors.
/// - **v27** (2026-04-30, engine-state cleanup): PC quick-action sequences
///   and hero speech suppression state serialize with PC state.
/// - **v28** (2026-04-30, engine-state cleanup): remaining element-owned
///   spatial, combat-display, shield, alert, and patch attachment caches
///   serialize with their owner structs.
/// - **v29** (2026-04-30, engine-state cleanup): campaign pre-mission
///   snapshots serialize with campaign state so mission restart survives
///   save/load.
/// - **v30** (2026-04-30, engine-state cleanup): patch level-static
///   references serialize with patch state instead of being skipped.
/// - **v31** (2026-04-30, engine-state cleanup): sequence-manager pending
///   immediate actions, condolations, halt latch, and actor progress index
///   serialize with sequence state.
/// - **v32** (2026-04-30, engine-state cleanup): position-interface sprite
///   center offset serializes with position state.
/// - **v33** (2026-04-30, engine-state cleanup): sprite script and
///   conversion tables serialize with sprite state instead of relying on
///   skipped runtime reattachment.
/// - **v34** (2026-04-30, engine-state cleanup): door geometry, links,
///   jump metadata, patch binding, and action hints serialize with door
///   state.
/// - **v35** (2026-04-30, engine-state cleanup): sector geometry, level
///   references, material data, script metadata, archery points, and shadow
///   metrics serialize with sector state.
/// - **v36** (2026-04-30, engine-state cleanup): path graph static data
///   serializes through its Arc instead of being reattached after load.
/// - **v37** (2026-04-30, engine-state cleanup): fast-find level grid and
///   shadow data serialize with grid state; per-query visited and detection
///   scratch no longer lives on the grid.
/// - **v38** (2026-04-30, engine-state cleanup): pathfinder A* state no
///   longer has hidden skipped fields.
/// - **v39** (2026-04-30, engine-state cleanup): script VM native host is
///   passed as execution context instead of living on serialized VM state.
/// - **v40** (2026-04-30, engine-state cleanup): patch, sprite, and
///   position-interface state no longer accepts missing fields by default.
/// - **v41** (2026-04-30, engine-state cleanup): mission, campaign, order,
///   marker, titbit, and PC metadata no longer accepts missing snapshot
///   fields by default.
/// - **v42** (2026-04-30, engine-state cleanup): engine-inner pending
///   queues, macro state, freeze state, and script post-init flags no
///   longer accept missing snapshot fields by default.
/// - **v43** (2026-04-30, engine-state cleanup): sequence manager lookup,
///   pending immediate action, condolation, and halt state no longer
///   accept missing snapshot fields by default.
/// - **v44** (2026-04-30, engine-state cleanup): element-owned runtime
///   state no longer accepts missing snapshot fields by default.
/// - **v45** (2026-04-30, engine-state cleanup): AI profile caches,
///   tactical state, and pending AI side-effect flags no longer accept
///   missing snapshot fields by default.
/// - **v46** (2026-07-19, nested engine snapshot): `EngineInner` serializes
///   its nine current state owners instead of the historical flat field list.
/// - **v47** (2026-07-19, script effects): mission scripts serialize the
///   canonical `script_effects` owner with typed presentation, external, and
///   simulation-barrier domains.
/// - **v48** (2026-07-19, ordered effects and simulation lifecycle): typed
///   effects serialize in one emission-ordered stream, sequence continuation
///   state is explicit, persistent game state is required, and complete
///   `SimConfig` plus mission construction/restart RNG checkpoints serialize.
/// - **v49** (2026-07-19, snapshot-input ownership): shared-camera transition
///   inputs that affect a later tick are required snapshot fields.
/// - **v50** (2026-07-21, Strangle owner initialization): active ability state
///   records whether first-owner Strangle setup has completed.
/// - **v51** (2026-07-22, Execute owner identity): actor state records the
///   selected Execute order identity and one-shot initialization latch matching
///   Original `mulLastOrderID` / `mbNewOrder` semantics.
/// - **v52** (2026-07-22, specialized AI continuations): AI state records
///   result-bearing cross-NPC callback continuations, alert scan progress and
///   the final report-before-formation barrier.
/// - **v53** (2026-08-26, automatic quick-action queue): player state records
///   the active automatic queue and its serialized commands.
/// - **v54** (2026-08-26, resolved quick-action state): records shield danger
///   geometry, independent automatic queues, resolved group-move routes, and
///   resolved DropAle routes.
/// - **v55** (2026-08-26, exact movement-route ownership): every stored quick
///   action move requires its resolved per-PC destination-sector identity, and
///   sequence point Seeks require explicit live-versus-Original route
///   provenance. Obsolete Rust saves are rejected instead of re-running
///   spatial placement or silently re-enabling reconstructed gate search.
/// - **v56** (2026-08-28, human actor control): changes authoritative player
///   control state and its native snapshot layout.
/// - **v57** (2026-08-29, full-fidelity campaign history): requires the native
///   append-only attempt schema and exact practice-return snapshot. Earlier
///   Rust save layouts are rejected rather than migrated.
/// - **v58** (2026-08-30, authoritative Sherwood trading): records the
///   deterministic trading rule, exact sale commands, receipts, campaign
///   ransom, and production-item inventory state.
pub const SAVE_FORMAT_VERSION: u32 = 58;

/// Save file header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveHeader {
    /// Magic identifier — always `"RHSG"`.
    pub magic: String,
    /// Save format version.
    pub version: u32,
    /// Mission ID this save belongs to.  Used to refuse loading a save
    /// for a different level.
    pub mission_id: u32,
    /// Unix epoch seconds at save time.
    pub timestamp_unix: u64,
    /// Human-readable label chosen by the player (empty for auto saves).
    pub display_text: String,
}

impl SaveHeader {
    pub fn new(mission_id: u32, display_text: String) -> Self {
        let timestamp_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            magic: SAVE_MAGIC.to_string(),
            version: SAVE_FORMAT_VERSION,
            mission_id,
            timestamp_unix,
            display_text,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.magic != SAVE_MAGIC {
            bail!(
                "invalid save file magic: expected {SAVE_MAGIC:?}, got {:?}",
                self.magic
            );
        }
        if self.version != SAVE_FORMAT_VERSION {
            bail!(
                "unsupported save file version: expected {SAVE_FORMAT_VERSION}, got {}",
                self.version
            );
        }
        if self.mission_id == 0 {
            bail!("invalid save mission ID: zero is not a valid mission");
        }
        Ok(())
    }
}

// ─── Full save file ──────────────────────────────────────────────────

/// A complete game save file.
///
/// Logical layout: header, then engine state (which owns the campaign),
/// plus the host-owned `SoundManager` (volumes, muted state) that also
/// round-trips across saves.  The thumbnail image is handled separately
/// by `SaveGameManager`.
#[derive(Clone, Serialize, Deserialize)]
pub struct GameSaveFile {
    pub header: SaveHeader,
    /// Full engine snapshot (via the serde-transparent `Engine` wrapper).
    /// Includes campaign, entities, RNG, script heaps, and every other
    /// non-`#[serde(skip)]` field on `EngineInner`.  Static level data
    /// (level grid, sight obstacles, script bytecode) is skipped and
    /// carried across from the live engine by [`Engine::restore`].
    pub engine: Engine,
    /// Host-side sound manager state. Split from the engine because
    /// `SoundManager` lives in robin_rs (drives Kira), while
    /// the sim-state portion of sound is inside `EngineInner::sound_sim`.
    pub sound: SoundManager,
    /// Host-side persistent Game flags (campaign-map display state,
    /// widget-enable booleans, men-to-blazon mode).
    pub game_persistent: GamePersistentState,
}

impl GameSaveFile {
    /// Test-only convenience for snapshots that do not exercise host-owned
    /// persistent game flags. Production saves must use
    /// [`capture_with_game`](Self::capture_with_game) so a valid-looking v49
    /// payload can never silently substitute default persistent state.
    #[cfg(test)]
    pub fn capture(engine: &Engine, host: &Host, mission_id: u32, display_text: String) -> Self {
        Self {
            header: SaveHeader::new(mission_id, display_text),
            engine: engine.clone(),
            sound: host.audio.sound.clone(),
            game_persistent: GamePersistentState::default(),
        }
    }

    /// Build a production save while snapshotting the required host-side
    /// `GamePersistentState`.
    pub fn capture_with_game(
        engine: &Engine,
        host: &Host,
        game: &crate::game::Game,
        mission_id: u32,
        display_text: String,
    ) -> Self {
        let snapshot = GameRuntimeSnapshot::capture(engine, host, game);
        Self {
            header: SaveHeader::new(mission_id, display_text),
            engine: snapshot.engine,
            sound: snapshot.sound,
            game_persistent: snapshot.game_persistent,
        }
    }

    /// Identity of the serialized runtime payload, excluding header metadata.
    pub(crate) fn replay_identity(&self) -> Result<ReplaySaveIdentity> {
        replay_save_identity(&self.engine, &self.sound, &self.game_persistent)
    }

    /// Apply a save file to the engine, replacing it wholesale.
    ///
    /// The caller is responsible for checking that the engine has
    /// already been initialized for the matching mission ID (level
    /// geometry loaded).  The engine-side half of post-load resync
    /// lives in [`Engine::try_restore`](robin_engine::engine::Engine::try_restore)
    /// (asset attachment + transient reset); the host-side half lives in
    /// [`Host::post_load_reset`].
    ///
    /// Does **not** touch a live `Game` — use
    /// [`apply_to_with_game`](Self::apply_to_with_game) when a mutable
    /// `Game` is in scope so the persistent flags (`campaign_map_*`,
    /// widget enables, men-to-blazon mode) can be restored.
    pub fn apply_to(
        self,
        engine: &mut Engine,
        host: &mut Host,
        assets: &LevelAssets,
    ) -> std::result::Result<(), SnapshotRestoreError> {
        *engine = Engine::restore_from_snapshot(&mut host.engine_display, self.engine, assets)?;
        host.audio.sound = self.sound;
        // Re-arm the sound engine and prime the next hourglass to
        // (re)load music + resolve pendings.
        host.audio.sound.after_load(&engine.sound_sim().sources);
        host.post_load_reset();
        Ok(())
    }

    /// Apply a save file to the engine *and* restore the host-side
    /// `GamePersistentState` (men-to-blazon conversion, campaign-map
    /// display state, and widget-enable bools).
    pub fn apply_to_with_game(
        self,
        engine: &mut Engine,
        host: &mut Host,
        game: &mut crate::game::Game,
        assets: &LevelAssets,
    ) -> std::result::Result<(), SnapshotRestoreError> {
        let snapshot = GameRuntimeSnapshot {
            engine: self.engine,
            sound: self.sound,
            game_persistent: self.game_persistent,
        };
        snapshot.apply_to_with_game(engine, host, game, assets)
    }

    /// Write the save file to disk as JSON.
    pub fn write_to(&self, path: &Path) -> Result<()> {
        self.header.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating save directory {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("serializing save file")?;
        fs::write(path, json).with_context(|| format!("writing save file {}", path.display()))
    }

    /// Read a save file from disk.
    pub fn read_from(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path)
            .with_context(|| format!("reading save file {}", path.display()))?;
        let document: serde_json::Value = serde_json::from_str(&json)
            .with_context(|| format!("parsing save file {}", path.display()))?;
        let header: SaveHeader = serde_json::from_value(
            document
                .get("header")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("save file has no header"))?,
        )
        .with_context(|| format!("parsing save header {}", path.display()))?;
        header.validate()?;
        let save: Self = serde_json::from_value(document)
            .with_context(|| format!("parsing save payload {}", path.display()))?;
        save.engine
            .campaign()
            .validate_history_schema()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("validating campaign history {}", path.display()))?;
        Ok(save)
    }
}

// ─── Save directory resolution ───────────────────────────────────────

/// Well-known filenames for special save slots:
///
/// - `CONTINUE`   — auto-save after success
/// - `QUICK`      — F5 quick save
/// - `EX_QUICK`   — previous quick save
/// - `RESTART`    — pre-restart snapshot
/// - `SHERWOOD`   — Sherwood map checkpoint
pub mod special_slots {
    pub const CONTINUE: &str = "Continue";
    pub const QUICK: &str = "QuickSave";
    pub const EX_QUICK: &str = "ExQuickSave";
    pub const RESTART: &str = "Restart";
    pub const SHERWOOD: &str = "Sherwood";
}

/// Resolve the per-OS save-game *root* directory. This is the folder that
/// holds `profiles.json`, `keyconfigs.json`, and one `Profile_NNN/`
/// subdirectory per player profile.
///
/// Priority (first hit wins):
///   1. `ROBINHOOD_SAVE_DIR` environment variable (for tests / portable installs)
///   2. OS data dir via `dirs::data_dir()` — e.g. `~/.local/share/robin_hood/saves`
///      on Linux, `%APPDATA%\robin_hood\saves` on Windows,
///      `~/Library/Application Support/robin_hood/saves` on macOS
///   3. Fallback: `./Data/Savegame/default` (for installations without
///      a per-user profile)
///
/// The returned directory is *not* created automatically; the caller
/// creates it on first write via `fs::create_dir_all`.
pub fn default_save_directory() -> PathBuf {
    if let Ok(override_dir) = std::env::var("ROBINHOOD_SAVE_DIR") {
        return PathBuf::from(override_dir);
    }
    #[cfg(feature = "native-fs")]
    if let Some(data_dir) = dirs::data_dir() {
        return data_dir.join("robin_hood").join("saves");
    }
    PathBuf::from("Data/Savegame/default")
}

/// Name of the per-profile save subdirectory: `Profile_%03lu` against
/// the profile's stable id.
pub fn profile_save_subdirectory(profile_id: u32) -> String {
    format!("Profile_{profile_id:03}")
}

/// Full save directory for a specific profile — `<root>/Profile_NNN`.
pub fn save_directory_for_profile(profile_id: u32) -> PathBuf {
    default_save_directory().join(profile_save_subdirectory(profile_id))
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn save_format_version_includes_authoritative_sherwood_trading() {
        assert_eq!(SAVE_FORMAT_VERSION, 58);
    }

    fn fresh_engine() -> (Engine, engine_api::LevelAssets) {
        use robin_engine::campaign::Campaign;
        let mut assets = engine_api::LevelAssets::new();
        let engine = Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets)
            .expect("new_for_test");
        (engine, assets)
    }

    #[test]
    fn header_validate_ok() {
        let header = SaveHeader::new(42, "My Save".into());
        assert_eq!(header.magic, SAVE_MAGIC);
        assert_eq!(header.version, SAVE_FORMAT_VERSION);
        assert_eq!(header.mission_id, 42);
        assert_eq!(header.display_text, "My Save");
        header.validate().unwrap();
    }

    #[test]
    fn replay_identity_stringifies_structured_map_keys() {
        let value = std::collections::BTreeMap::from([((1_u32, 2_u32), 3_u32)]);

        let first = replay_identity_digest(&value).expect("structured-key identity");
        let second = replay_identity_digest(&value).expect("stable structured-key identity");

        assert_eq!(first, second);
    }

    #[test]
    fn header_rejects_bad_magic() {
        let mut header = SaveHeader::new(0, String::new());
        header.magic = "XXXX".into();
        assert!(header.validate().is_err());
    }

    #[test]
    fn header_rejects_bad_version() {
        let mut header = SaveHeader::new(0, String::new());
        header.version = SAVE_FORMAT_VERSION + 999;
        assert!(header.validate().is_err());
    }

    #[test]
    fn header_rejects_zero_mission_id() {
        let header = SaveHeader::new(0, String::new());
        assert_eq!(
            header.validate().unwrap_err().to_string(),
            "invalid save mission ID: zero is not a valid mission"
        );
    }

    #[test]
    fn save_round_trip_via_json() {
        // Seed scalar engine fields via `test_set_*` helpers (the only
        // back door into `EngineInner` from outside robin_engine), then
        // capture → JSON → decode → apply, and check the fields survived.
        let (mut engine, _assets) = fresh_engine();
        engine.test_set_frame_counter(12345);
        engine.test_set_mission_flags(false, false, true);
        let host = Host::scratch(800.0, 600.0);

        let save = GameSaveFile::capture(&engine, &host, 7, "Test Save".into());

        let json = serde_json::to_string(&save).expect("serialize");
        let decoded: GameSaveFile = serde_json::from_str(&json).expect("deserialize");
        decoded.header.validate().unwrap();
        assert_eq!(decoded.header.mission_id, 7);
        assert_eq!(decoded.header.display_text, "Test Save");

        let (mut engine2, assets2) = fresh_engine();
        let mut host2 = Host::scratch(800.0, 600.0);
        decoded
            .apply_to(&mut engine2, &mut host2, &assets2)
            .expect("apply decoded save");
        engine2.test_assert_level_assets_attached(&assets2);
        assert_eq!(engine2.frame_counter(), 12345);
        assert!(engine2.mission().mission_won);
    }

    #[test]
    fn save_payload_round_trips_required_camera_transition_inputs() {
        let (mut engine, _assets) = fresh_engine();
        let expected = (
            true,
            true,
            robin_engine::coordinates::MapVec::new(3.0, -4.0),
            7,
            Some(robin_engine::coordinates::ScreenPoint::new(123.0, 456.0)),
        );
        engine.test_set_camera_transition_inputs(
            expected.0, expected.1, expected.2, expected.3, expected.4,
        );
        let host = Host::scratch(800.0, 600.0);

        let save = GameSaveFile::capture(&engine, &host, 7, "camera inputs".into());
        let json = serde_json::to_string(&save).expect("serialize current save");
        let decoded: GameSaveFile = serde_json::from_str(&json).expect("decode current save");

        assert_eq!(decoded.engine.test_camera_transition_inputs(), expected);
    }

    #[test]
    fn save_requires_game_persistent_state() {
        let (engine, _assets) = fresh_engine();
        let host = Host::scratch(800.0, 600.0);
        let save = GameSaveFile::capture(&engine, &host, 7, "required".into());
        let mut value = serde_json::to_value(save).unwrap();
        value.as_object_mut().unwrap().remove("game_persistent");

        assert!(serde_json::from_value::<GameSaveFile>(value).is_err());
    }

    #[test]
    fn current_v52_requires_every_persistent_game_flag() {
        let (engine, _assets) = fresh_engine();
        let host = Host::scratch(800.0, 600.0);
        let required_fields = [
            "campaign_map_active",
            "campaign_map_displayed",
            "men_to_blazon_conversion",
            "start_mission_enabled",
            "quit_mission_enabled",
            "start_mission_disabled_temp",
            "quit_mission_disabled_temp",
            "draw_hidden",
        ];

        for field in required_fields {
            let save = GameSaveFile::capture(&engine, &host, 7, "required".into());
            let mut value = serde_json::to_value(save).unwrap();
            value["game_persistent"]
                .as_object_mut()
                .expect("serialized persistent game state")
                .remove(field);
            assert!(
                serde_json::from_value::<GameSaveFile>(value).is_err(),
                "current v52 payload missing {field} must be rejected"
            );
        }
    }

    #[test]
    fn direct_launch_save_retains_exact_construction_and_restart_checkpoint() {
        use robin_engine::campaign::Campaign;
        use robin_engine::engine::SimConfig;
        use robin_engine::player_profile::DifficultyLevel;

        let construction_config = SimConfig {
            difficulty: DifficultyLevel::Hard,
            amount_of_speaking: 9,
            golden_eye: true,
            ..Default::default()
        };

        let campaign = crate::game_session::establish_mission_restart_boundary(
            Campaign::default(),
            0x3333_4444,
            construction_config,
        );
        let mut assets = engine_api::LevelAssets::new();
        let engine = Engine::new_for_test_with_simulation(
            800.0,
            600.0,
            campaign,
            &mut assets,
            0x3333_4444,
            construction_config,
        )
        .unwrap();
        let host = Host::scratch(800.0, 600.0);
        let encoded = serde_json::to_string(&GameSaveFile::capture(
            &engine,
            &host,
            0,
            "checkpoint".into(),
        ))
        .unwrap();
        let loaded: GameSaveFile = serde_json::from_str(&encoded).unwrap();

        assert_eq!(
            loaded.engine.mission_start_simulation(),
            (0x3333_4444, construction_config)
        );
        assert_eq!(
            loaded.engine.campaign().restart_simulation_checkpoint(),
            (0x3333_4444, construction_config)
        );
        assert!(loaded.engine.campaign().pre_mission_was_preselected);
        let mut restarted_campaign = loaded.engine.campaign().clone();
        assert!(restarted_campaign.restore_snapshot());
        assert_eq!(
            restarted_campaign.restart_simulation_checkpoint(),
            (0x3333_4444, construction_config)
        );
    }

    #[test]
    fn write_and_read_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_save.json");

        let (mut engine, _assets) = fresh_engine();
        let host = Host::scratch(800.0, 600.0);
        engine.test_set_frame_counter(999);
        let save = GameSaveFile::capture(&engine, &host, 1, "Disk Save".into());
        save.write_to(&path).unwrap();

        let loaded = GameSaveFile::read_from(&path).unwrap();
        assert_eq!(loaded.header.mission_id, 1);
        assert_eq!(loaded.engine.frame_counter(), 999);
    }

    #[test]
    fn read_rejects_v46_before_deserializing_previous_script_effects_payload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("old_save.json");
        let old_save = serde_json::json!({
            "header": {
                "magic": SAVE_MAGIC,
                "version": 46,
                "mission_id": 1,
                "timestamp_unix": 0,
                "display_text": "Old Save"
            },
            "engine": {
                "mission": {}
            }
        });
        fs::write(&path, serde_json::to_vec(&old_save).unwrap()).unwrap();

        let error = GameSaveFile::read_from(&path)
            .err()
            .expect("v46 saves must be rejected");
        let message = format!("{error:#}");
        assert_eq!(
            message,
            format!("unsupported save file version: expected {SAVE_FORMAT_VERSION}, got 46")
        );
    }

    #[test]
    fn read_rejects_previous_rust_campaign_schema_at_header() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("previous_rust_save.json");
        let old_save = serde_json::json!({
            "header": {
                "magic": SAVE_MAGIC,
                "version": 57,
                "mission_id": 1,
                "timestamp_unix": 0,
                "display_text": "Previous Rust Save"
            },
            "engine": {}
        });
        fs::write(&path, serde_json::to_vec(&old_save).unwrap()).unwrap();

        let error = GameSaveFile::read_from(&path)
            .err()
            .expect("previous Rust campaign schema must be rejected");
        assert_eq!(
            format!("{error:#}"),
            format!("unsupported save file version: expected {SAVE_FORMAT_VERSION}, got 57")
        );
    }

    #[test]
    fn profile_save_subdirectory_formats_with_zero_padding() {
        assert_eq!(profile_save_subdirectory(0), "Profile_000");
        assert_eq!(profile_save_subdirectory(7), "Profile_007");
        assert_eq!(profile_save_subdirectory(99), "Profile_099");
        assert_eq!(profile_save_subdirectory(1000), "Profile_1000");
    }

    #[test]
    fn default_save_directory_respects_env_override() {
        // Use a unique env var to avoid clashes with other parallel tests.
        let dir = tempdir().unwrap();
        // Safety: `ROBINHOOD_SAVE_DIR` is only read/written by this test, and no
        // other test in the crate touches std::env. That keeps the set_var call
        // free of concurrent getenv readers from sibling tests.
        unsafe { std::env::set_var("ROBINHOOD_SAVE_DIR", dir.path()) };
        let resolved = default_save_directory();
        assert_eq!(resolved, dir.path());
        unsafe { std::env::remove_var("ROBINHOOD_SAVE_DIR") };
    }

    #[test]
    fn thumbnail_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("thumb.png");
        let pixels: Vec<u16> = (0..(THUMB_WIDTH as u32 * THUMB_HEIGHT as u32))
            .map(|i| (i & 0xFFFF) as u16)
            .collect();
        let thumb = Thumbnail::from_pixels(THUMB_WIDTH, THUMB_HEIGHT, pixels.clone()).unwrap();
        thumb.write_to(&path).unwrap();
        let loaded = Thumbnail::read_from(&path).unwrap();
        assert_eq!(loaded.width, THUMB_WIDTH);
        assert_eq!(loaded.height, THUMB_HEIGHT);
        assert_eq!(loaded.pixels, pixels);
    }

    #[test]
    fn thumbnail_rejects_invalid_png() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad_thumb.png");
        fs::write(&path, b"not a png").unwrap();
        assert!(Thumbnail::read_from(&path).is_err());
    }

    #[test]
    fn thumbnail_from_pixels_length_check() {
        assert!(Thumbnail::from_pixels(4, 4, vec![0; 15]).is_none());
        assert!(Thumbnail::from_pixels(4, 4, vec![0; 16]).is_some());
        assert!(Thumbnail::from_pixels(4, 4, vec![0; 17]).is_none());
    }

    #[test]
    fn thumbnail_from_rgba_downscaled_samples_source_frame() {
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let thumb = Thumbnail::from_rgba_downscaled(2, 2, &rgba, 2, 2).unwrap();
        assert_eq!(thumb.width, 2);
        assert_eq!(thumb.height, 2);
        assert_eq!(thumb.pixels, vec![0xF800, 0x07E0, 0x001F, 0xFFFF]);
    }

    #[test]
    fn apply_clears_host_transient_state() {
        // Host-side post-load resync: clear input, invalidate cached
        // surfaces, reset per-frame host scratch.  Engine-side transient
        // clearing is covered by tests in the engine crate.
        let (engine, _assets) = fresh_engine();
        let host = Host::scratch(800.0, 600.0);

        let save = GameSaveFile::capture(&engine, &host, 0, String::new());

        let (mut engine3, assets3) = fresh_engine();
        let mut host2 = Host::scratch(800.0, 600.0);
        host2.input.multi_selection_active = true;
        host2.input.left_mouse_down = true;
        host2.valid_trajectory = true;

        save.apply_to(&mut engine3, &mut host2, &assets3)
            .expect("apply save");

        assert!(!host2.input.multi_selection_active);
        assert!(!host2.input.left_mouse_down);
        assert!(host2.input.focused_entity_id.is_none());
        assert!(!host2.valid_trajectory);
    }

    #[test]
    fn failed_apply_preserves_live_engine_and_host_state() {
        let (mut saved_engine, _saved_assets) = fresh_engine();
        saved_engine.test_set_frame_counter(123);
        let saved_host = Host::scratch(800.0, 600.0);
        let save = GameSaveFile::capture(&saved_engine, &saved_host, 0, String::new());

        let (mut live_engine, mut assets) = fresh_engine();
        live_engine.test_set_frame_counter(999);
        assets.entities.mobile_element_count = 1;
        let mut live_host = Host::scratch(800.0, 600.0);
        live_host.input.left_mouse_down = true;

        let error = save
            .apply_to(&mut live_engine, &mut live_host, &assets)
            .unwrap_err();
        assert!(matches!(
            error,
            SnapshotRestoreError::WorldInvariantViolation { .. }
        ));
        assert_eq!(live_engine.frame_counter(), 999);
        assert!(live_host.input.left_mouse_down);
    }
}

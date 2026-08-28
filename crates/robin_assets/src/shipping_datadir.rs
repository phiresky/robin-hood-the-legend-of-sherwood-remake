//! Shipping datadir manifest and lazily loaded mission payloads.
//!
//! Produced by the `convert_datadir --format shipping` binary and loaded
//! at engine startup (see [`try_load`]). When a shipping datadir is
//! present, individual subsystem loaders (`ProfileManager::load_all_legacy_cpf`,
//! `FrameHolder::initialize_sprite_bank`, `ResourceManager::attach_resource_file`,
//! etc.) consult it instead of reading legacy files off disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::frame_holder::FrameDictionary;
use crate::res_descr::LevelDescriptors;
use crate::resource_manager::{EncodedPicture, ResourceManager};
use crate::scb::ScbFile;
use robin_engine::level_data::LoadedLevel;
use robin_engine::profiles::ProfileManager;
use robin_engine::sprite_script::SpriteInfo;

/// Top-level shipping payload.
///
/// Keys mirror the on-disk relative path under `Data/` so loaders can find
/// things under the same names they use for legacy I/O (e.g.
/// `"Interface/DEFAULT.RES"`, `"Levels/Dem_Lei_MP.rhm"`).
#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingDatadir {
    pub profiles: Option<ProfileManager>,
    pub res_files: std::collections::BTreeMap<String, ResourceManager>,
    pub pak_files: std::collections::BTreeMap<String, Vec<EncodedPicture>>,
    pub red_files: std::collections::BTreeMap<String, LevelDescriptors>,
    /// Keyed by mission base name (no extension), e.g. `"Dem_Lei_MP"`.
    pub levels: std::collections::BTreeMap<String, LoadedLevel>,
    pub scripts: std::collections::BTreeMap<String, ScbFile>,
    /// Keyed by the full relative path `Characters/<name>.rhs`.
    pub rhs_files: std::collections::BTreeMap<String, RhsData>,
    /// Packed sprite pool. See [`ShippingSpriteBank`].
    pub sprite_bank: Option<ShippingSpriteBank>,
    /// Terrain bitmaps and other not-yet-parsed binary blobs, keyed by
    /// relative path (e.g. `Levels/Day/leicester.map`).
    pub raw: std::collections::BTreeMap<String, Vec<u8>>,
    /// Source-authoritative durations for boot audio stored in `raw`.
    pub audio_durations_ms: BTreeMap<String, u32>,
    /// Independently compressed payload to fetch before starting each mission.
    pub missions: BTreeMap<String, ShippingMissionRef>,
    /// Content-addressed RHS payloads required when a character profile can
    /// participate in the selected mission. Keys are stable CPF character
    /// profile indices; values include the character's physical RHS variants
    /// and the object/projectile RHS files enabled by its actions.
    pub character_rhs_files: BTreeMap<u32, Vec<String>>,
    /// Content-addressed localized voice payloads for each CPF character
    /// profile. Runtime party/reinforcement selection uses the same profile
    /// closure as `character_rhs_files`, avoiding every PC voice in every
    /// mission reference.
    pub character_audio_files: BTreeMap<u32, Vec<String>>,
    /// Exclamation profile id corresponding to each CPF character profile.
    pub character_exclamation_ids: BTreeMap<u32, u32>,
    /// Authored soldier/civilian/required/rescue exclamation ids for each
    /// mission. Dynamic party ids are unioned at the mission-load boundary.
    pub mission_exclamation_ids: BTreeMap<String, Vec<u32>>,
    /// Conservative RHS closure used only when constructing a mission around
    /// an already-decoded saved world. Saved entities may contain object types
    /// that are neither authored by the destination mission nor implied by its
    /// current party, so save launches must not silently omit their masters.
    pub saved_world_rhs_files: Vec<String>,
    /// Runtime-only source directory containing `datadir.bin` and its payloads.
    #[serde(skip)]
    #[bitcode(skip)]
    source_dir: Option<PathBuf>,
    /// Runtime-only HTTP base used by the browser build.
    #[serde(skip)]
    #[bitcode(skip)]
    remote_base_url: Option<String>,
    /// Runtime shared-byte view of boot `raw`. Installation moves into this
    /// bundle when the manifest has a unique owner, avoiding a second copy.
    #[serde(skip)]
    #[bitcode(skip)]
    boot_raw_bundle: OnceLock<Arc<robin_util::asset_fs::Bundle>>,
    /// Payloads already installed for this process. Kept out of the manifest.
    #[serde(skip)]
    #[bitcode(skip)]
    loaded_missions: RwLock<BTreeMap<String, Arc<ShippingMission>>>,
    #[serde(skip)]
    #[bitcode(skip)]
    active_mission: RwLock<Option<String>>,
    /// Exact static + dynamic exclamation closure for the active mission.
    #[serde(skip)]
    #[bitcode(skip)]
    active_exclamation_ids: RwLock<BTreeSet<u32>>,
}

impl Default for ShippingDatadir {
    fn default() -> Self {
        Self {
            profiles: None,
            res_files: BTreeMap::new(),
            pak_files: BTreeMap::new(),
            red_files: BTreeMap::new(),
            levels: BTreeMap::new(),
            scripts: BTreeMap::new(),
            rhs_files: BTreeMap::new(),
            sprite_bank: None,
            raw: BTreeMap::new(),
            audio_durations_ms: BTreeMap::new(),
            missions: BTreeMap::new(),
            character_rhs_files: BTreeMap::new(),
            character_audio_files: BTreeMap::new(),
            character_exclamation_ids: BTreeMap::new(),
            mission_exclamation_ids: BTreeMap::new(),
            saved_world_rhs_files: Vec::new(),
            source_dir: None,
            remote_base_url: None,
            boot_raw_bundle: OnceLock::new(),
            loaded_missions: RwLock::new(BTreeMap::new()),
            active_mission: RwLock::new(None),
            active_exclamation_ids: RwLock::new(BTreeSet::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingMissionRef {
    /// Paths relative to the directory containing `datadir.bin`. Shared RHS
    /// payloads can be named by several missions without being stored twice.
    pub files: Vec<String>,
}

/// All data whose lifetime starts when one mission is selected.
#[derive(Default, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingMission {
    pub levels: BTreeMap<String, LoadedLevel>,
    pub scripts: BTreeMap<String, ScbFile>,
    pub rhs_files: BTreeMap<String, RhsData>,
    pub sprite_bank: Option<ShippingSpriteBank>,
    pub raw: BTreeMap<String, Vec<u8>>,
    /// Exact durations from the source assets, keyed like `raw`.
    ///
    /// Web shipping may transcode WAV/Vorbis to Opus. Simulation timing must
    /// continue to use the authoritative source duration rather than codec
    /// delay, resampling, or a browser decoder's rounded duration.
    pub audio_durations_ms: BTreeMap<String, u32>,
    /// Runtime shared-byte view of `raw`. Installation moves the decoded
    /// vectors here so the VFS and mission payload share the same allocation.
    #[serde(skip)]
    #[bitcode(skip)]
    raw_bundle: OnceLock<Arc<robin_util::asset_fs::Bundle>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct RhsData {
    pub signature: u32,
    pub profiles: Vec<(String, SpriteInfo)>,
}

/// Shipping-ready sprite bank. Unlike the runtime [`crate::frame_holder::FrameHolder`],
/// this carries every sprite's packed pixel data inline (the runtime
/// version marks `packed_data` `#[serde(skip)]` so savegames stay small).
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingSpriteBank {
    pub signature: u32,
    pub dictionaries: Vec<FrameDictionary>,
    /// Total number of slots in the original bank. The runtime expands the
    /// sparse entries below into this many slots once, after all mission
    /// chunks have been combined.
    pub sprite_count: u32,
    /// Sorted `(bank id, sprite)` entries. Mission RHS chunks normally use a
    /// tiny fraction of the global bank, so storing a dense `Vec<Option<_>>`
    /// here used hundreds of MiB of transient wasm heap while decoding.
    pub sprites: Vec<(u32, ShippingSprite)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingSprite {
    pub width: u16,
    pub height: u16,
    pub dictionary_index: u16,
    /// Packed pixel data (RLE or dictionary-indexed).
    pub packed_data: Arc<Vec<u16>>,
}

// ---------------------------------------------------------------------------
//  I/O
// ---------------------------------------------------------------------------

impl ShippingDatadir {
    /// Parse a shipping datadir blob: zstd decompress + native bitcode decode.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let compressed =
            robin_util::asset_fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let mut datadir = Self::from_compressed_bytes(&compressed)
            .with_context(|| format!("decode {}", path.display()))?;
        datadir.source_dir = path.parent().map(Path::to_path_buf);
        Ok(datadir)
    }

    /// Load through an explicit VFS instance.
    pub fn load_from_vfs(vfs: &robin_util::asset_fs::AssetVfs, path: &Path) -> Result<Self> {
        let compressed = vfs
            .read(path)
            .with_context(|| format!("read {}", path.display()))?;
        let mut datadir = Self::from_compressed_bytes(&compressed)
            .with_context(|| format!("decode {}", path.display()))?;
        datadir.source_dir = path.parent().map(Path::to_path_buf);
        Ok(datadir)
    }

    /// Parse a shipping datadir blob already in memory.  Used by the
    /// wasm-bindgen bootstrap, which fetches `datadir.bin` from JS,
    /// hands the bytes to Rust, and decodes here — bypassing the
    /// `asset_fs::read` path (which is bundle-only on wasm and the
    /// bundle isn't installed yet at this point).
    pub fn from_compressed_bytes(compressed: &[u8]) -> Result<Self> {
        // Streaming decoder with `windowLogMax=30` (1 GiB virtual) —
        // the cap zstd permits on 32-bit builds like wasm32. Shipping
        // blobs destined for wasm must be compressed with
        // `window_log <= 30` (the desktop encoder uses 31, which zstd
        // rejects on 32-bit targets — see `zstd_max_compress`).
        let mut decoder =
            zstd::stream::read::Decoder::new(compressed).context("zstd decoder init")?;
        decoder
            .window_log_max(30)
            .context("zstd window_log_max=30")?;
        let mut blob = Vec::with_capacity(compressed.len() * 4);
        std::io::Read::read_to_end(&mut decoder, &mut blob).context("zstd decompress")?;
        let dd = decode_native(&blob)?;
        tracing::info!(
            "loaded shipping datadir ({} → {} bytes)",
            compressed.len(),
            blob.len()
        );
        Ok(dd)
    }

    pub fn set_remote_base_url(&mut self, url: String) {
        self.remote_base_url = Some(url.trim_end_matches('/').to_owned());
    }

    pub fn remote_base_url(&self) -> Option<&str> {
        self.remote_base_url.as_deref()
    }

    pub fn mission_ref(&self, mission: &str) -> Option<&ShippingMissionRef> {
        self.missions.get(mission)
    }

    pub fn has_mission(&self, mission: &str) -> bool {
        self.missions.contains_key(mission) || self.levels.contains_key(mission)
    }

    pub fn source_file_path(&self, relative: &str) -> Result<PathBuf> {
        let source_dir = self.source_dir.as_ref().ok_or_else(|| {
            anyhow!("shipping manifest has no native source directory for {relative}")
        })?;
        Ok(source_dir.join(relative))
    }

    pub fn is_mission_loaded(&self, mission: &str) -> bool {
        self.loaded_missions
            .read()
            .expect("shipping mission lock poisoned")
            .contains_key(mission)
    }

    pub fn install_mission(&self, mission: &str, mut payload: ShippingMission) -> Result<()> {
        if !payload.levels.contains_key(mission) {
            return Err(anyhow!(
                "shipping payload for {mission} does not contain its level"
            ));
        }
        if let (Some(base), Some(bank)) = (self.sprite_bank.as_ref(), payload.sprite_bank.as_ref())
            && (base.signature != bank.signature || base.sprite_count != bank.sprite_count)
        {
            return Err(anyhow!(
                "shipping mission {mission} sprite bank is incompatible with boot dictionaries"
            ));
        }
        let raw = std::mem::take(&mut payload.raw)
            .into_iter()
            .map(|(path, bytes)| (path, bytes.into()))
            .collect();
        payload
            .raw_bundle
            .set(Arc::new(raw))
            .map_err(|_| anyhow!("shipping mission {mission} raw bundle was already installed"))?;
        let mut loaded = self
            .loaded_missions
            .write()
            .expect("shipping mission lock poisoned");
        loaded.clear();
        loaded.insert(mission.to_owned(), Arc::new(payload));
        drop(loaded);
        *self
            .active_mission
            .write()
            .expect("shipping active mission lock poisoned") = None;
        self.activate_mission(mission)?;
        Ok(())
    }

    pub fn install_mission_parts(
        &self,
        mission: &str,
        parts: impl IntoIterator<Item = ShippingMission>,
    ) -> Result<()> {
        let mut merged = ShippingMission::default();
        for part in parts {
            merged.merge_from(part)?;
        }
        self.install_mission(mission, merged)
    }

    /// Synchronous native-file equivalent of the runtime's asynchronous
    /// mission loader. Developer tools use this when they open a converted
    /// datadir directly rather than entering the game session boundary.
    pub fn load_mission_from_source(&self, mission: &str) -> Result<()> {
        if self.is_mission_loaded(mission) {
            return self.activate_mission(mission);
        }
        let reference = self
            .mission_ref(mission)
            .ok_or_else(|| anyhow!("shipping datadir does not contain mission {mission}"))?;
        let mut merged = ShippingMission::default();
        for file in &reference.files {
            let path = self.source_file_path(file)?;
            let compressed = robin_util::asset_fs::read(&path)
                .with_context(|| format!("read {}", path.display()))?;
            merged.merge_part(
                decode_mission_compressed(&compressed)
                    .with_context(|| format!("decode {}", path.display()))?,
            )?;
        }
        self.install_mission_parts(mission, std::iter::once(merged))
    }

    pub fn activate_mission(&self, mission: &str) -> Result<()> {
        if self.active_mission_name().as_deref() == Some(mission) {
            return Ok(());
        }
        let payload = self
            .loaded_mission(mission)
            .ok_or_else(|| anyhow!("shipping mission {mission} has not been loaded"))?;
        let raw = payload
            .raw_bundle
            .get()
            .cloned()
            .ok_or_else(|| anyhow!("shipping mission {mission} has no installed raw bundle"))?;
        let raw_files = raw.len();
        let rhs_files = payload.rhs_files.len();
        robin_util::asset_fs::global()
            .replace_active_bundle(raw.clone())
            .context("mount shipping mission assets")?;
        if let Some(first_path) = raw.keys().next()
            && !robin_util::asset_fs::global()
                .try_exists(first_path)
                .with_context(|| format!("probe mounted shipping asset {first_path}"))?
        {
            return Err(anyhow!(
                "shipping mission {mission} mounted {raw_files} raw assets, but {first_path} is not visible"
            ));
        }
        robin_engine::sprite_script::replace_shipping_rhs(
            payload
                .rhs_files
                .iter()
                .map(|(path, rhs)| (path.as_str(), rhs.signature, rhs.profiles.as_slice())),
        );
        *self
            .active_mission
            .write()
            .expect("shipping active mission lock poisoned") = Some(mission.to_owned());
        tracing::info!(mission, raw_files, rhs_files, "activated shipping mission");
        Ok(())
    }

    pub fn loaded_mission(&self, mission: &str) -> Option<Arc<ShippingMission>> {
        self.loaded_missions
            .read()
            .expect("shipping mission lock poisoned")
            .get(mission)
            .cloned()
    }

    pub fn loaded_mission_count(&self) -> usize {
        self.loaded_missions
            .read()
            .expect("shipping mission lock poisoned")
            .len()
    }

    pub fn loaded_level(&self, mission: &str) -> Option<LoadedLevel> {
        self.loaded_mission(mission)
            .and_then(|payload| payload.levels.get(mission).cloned())
            .or_else(|| self.levels.get(mission).cloned())
    }

    pub fn mission_scripts(&self, mission: &str) -> BTreeMap<String, ScbFile> {
        self.loaded_mission(mission)
            .map(|payload| payload.scripts.clone())
            .unwrap_or_else(|| self.scripts.clone())
    }

    pub fn with_active_sprite_bank<R>(
        &self,
        use_bank: impl FnOnce(&ShippingSpriteBank, &[FrameDictionary]) -> R,
    ) -> Option<R> {
        let active = self
            .active_mission
            .read()
            .expect("shipping active mission lock poisoned")
            .clone();
        let loaded = active
            .as_deref()
            .and_then(|mission| self.loaded_mission(mission));
        let bank = loaded
            .as_ref()
            .and_then(|mission| mission.sprite_bank.as_ref())
            .or(self.sprite_bank.as_ref())?;
        let dictionaries = if bank.dictionaries.is_empty() {
            &self.sprite_bank.as_ref()?.dictionaries
        } else {
            &bank.dictionaries
        };
        Some(use_bank(bank, dictionaries))
    }

    pub fn active_mission_name(&self) -> Option<String> {
        self.active_mission
            .read()
            .expect("shipping active mission lock poisoned")
            .clone()
    }

    /// Publish the exact speech-profile closure selected at the asynchronous
    /// mission boundary. Process-wide audio caches use this instead of
    /// scanning every CPF actor and warning for intentionally unmounted data.
    pub fn set_active_exclamation_ids(&self, ids: BTreeSet<u32>) {
        *self
            .active_exclamation_ids
            .write()
            .expect("shipping active exclamation lock poisoned") = ids;
    }

    pub fn active_exclamation_ids(&self) -> Vec<u32> {
        self.active_exclamation_ids
            .read()
            .expect("shipping active exclamation lock poisoned")
            .iter()
            .copied()
            .collect()
    }

    /// Return the source-authoritative duration for boot or active-mission
    /// audio. Web artifacts use `.opus` keys even though legacy metadata asks
    /// for `.wav` or `.ogg`, so resolution includes that target extension.
    pub fn active_audio_duration_ms(&self, path: &Path) -> Option<u32> {
        self.active_audio_metadata(path)
            .map(|(_, duration)| duration)
    }

    /// Return encoded byte size and source duration without copying the VFS
    /// asset. The wasm sound cache only needs this bookkeeping because Web
    /// Audio owns both decoding and PCM playback storage.
    pub fn active_audio_metadata(&self, path: &Path) -> Option<(u32, u32)> {
        let key = robin_util::asset_fs::bundle_key(path);
        let opus = Path::new(&key)
            .with_extension("opus")
            .to_string_lossy()
            .replace('\\', "/");
        let mission = self
            .active_mission_name()
            .and_then(|mission| self.loaded_mission(&mission))
            .and_then(|payload| {
                let duration = payload
                    .audio_durations_ms
                    .get(&key)
                    .or_else(|| payload.audio_durations_ms.get(&opus))
                    .copied()?;
                let bytes = payload.raw_bundle.get()?.get(&key).or_else(|| {
                    payload
                        .raw_bundle
                        .get()
                        .and_then(|bundle| bundle.get(&opus))
                })?;
                Some((u32::try_from(bytes.len()).ok()?, duration))
            });
        mission.or_else(|| {
            let duration = self
                .audio_durations_ms
                .get(&key)
                .or_else(|| self.audio_durations_ms.get(&opus))
                .copied()?;
            let bytes = self.raw_asset(&key).or_else(|| self.raw_asset(&opus))?;
            Some((u32::try_from(bytes.len()).ok()?, duration))
        })
    }

    /// Borrow one boot asset whether installation has moved it into the VFS
    /// shared-byte bundle or this manifest is still in converter/tool form.
    pub fn raw_asset(&self, key: &str) -> Option<&[u8]> {
        self.raw.get(key).map(Vec::as_slice).or_else(|| {
            self.boot_raw_bundle
                .get()
                .and_then(|bundle| bundle.get(key))
                .map(|bytes| bytes.as_ref())
        })
    }
}

impl ShippingMission {
    /// Borrow an installed raw asset without copying its encoded bytes.
    pub fn raw_asset(&self, key: &str) -> Option<&[u8]> {
        self.raw.get(key).map(Vec::as_slice).or_else(|| {
            self.raw_bundle
                .get()
                .and_then(|bundle| bundle.get(key))
                .map(|bytes| bytes.as_ref())
        })
    }

    /// Move-merge one independently decoded dependency into this payload.
    /// Loaders use this incrementally so compressed/decoded part shells can be
    /// released as soon as each bounded fetch completes.
    pub fn merge_part(&mut self, source: Self) -> Result<()> {
        self.merge_from(source)
    }

    fn merge_from(&mut self, mut source: Self) -> Result<()> {
        merge_unique_owned(&mut self.levels, source.levels, "level")?;
        merge_unique_owned(&mut self.scripts, source.scripts, "script")?;
        merge_unique_owned(&mut self.rhs_files, source.rhs_files, "RHS")?;
        merge_unique_owned(&mut self.raw, source.raw, "raw asset")?;
        merge_unique_owned(
            &mut self.audio_durations_ms,
            source.audio_durations_ms,
            "audio duration",
        )?;
        let Some(mut source_bank) = source.sprite_bank.take() else {
            return Ok(());
        };
        let bank = self.sprite_bank.get_or_insert_with(|| ShippingSpriteBank {
            signature: source_bank.signature,
            dictionaries: std::mem::take(&mut source_bank.dictionaries),
            sprite_count: source_bank.sprite_count,
            sprites: Vec::new(),
        });
        if bank.signature != source_bank.signature || bank.sprite_count != source_bank.sprite_count
        {
            return Err(anyhow!("shipping sprite-bank parts are incompatible"));
        }
        if bank.dictionaries.is_empty() {
            bank.dictionaries = std::mem::take(&mut source_bank.dictionaries);
        } else if !source_bank.dictionaries.is_empty()
            && bitcode::encode(&bank.dictionaries) != bitcode::encode(&source_bank.dictionaries)
        {
            return Err(anyhow!("shipping sprite-bank dictionaries conflict"));
        }
        for (index, sprite) in source_bank.sprites {
            if index >= bank.sprite_count {
                return Err(anyhow!(
                    "shipping sprite-bank part contains out-of-range sprite {index} (bank has {} slots)",
                    bank.sprite_count
                ));
            }
            match bank
                .sprites
                .binary_search_by_key(&index, |(index, _)| *index)
            {
                Ok(position) => {
                    if bitcode::encode(&bank.sprites[position].1) != bitcode::encode(&sprite) {
                        return Err(anyhow!(
                            "shipping sprite-bank parts conflict at sprite {index}"
                        ));
                    }
                }
                Err(position) => bank.sprites.insert(position, (index, sprite)),
            }
        }
        Ok(())
    }
}

fn merge_unique_owned<K, V>(dst: &mut BTreeMap<K, V>, src: BTreeMap<K, V>, kind: &str) -> Result<()>
where
    K: Ord + std::fmt::Debug,
{
    for (key, value) in src {
        if dst.contains_key(&key) {
            return Err(anyhow!("duplicate shipping {kind} key {key:?}"));
        }
        dst.insert(key, value);
    }
    Ok(())
}

const SHIPPING_DATADIR_MAGIC: [u8; 8] = *b"RHDDNAT6";
const SHIPPING_MISSION_MAGIC: [u8; 8] = *b"RHMISN03";
pub const SHIPPING_DATADIR_VERSION: u32 = 6;
pub const SHIPPING_MISSION_VERSION: u32 = 3;

/// Encode the versioned native-bitcode payload stored inside `datadir.bin`.
pub fn encode_native(datadir: &ShippingDatadir) -> Vec<u8> {
    let payload = bitcode::encode(datadir);
    let mut encoded = Vec::with_capacity(12 + payload.len());
    encoded.extend_from_slice(&SHIPPING_DATADIR_MAGIC);
    encoded.extend_from_slice(&SHIPPING_DATADIR_VERSION.to_le_bytes());
    encoded.extend_from_slice(&payload);
    encoded
}

fn decode_native(encoded: &[u8]) -> Result<ShippingDatadir> {
    let Some((header, payload)) = encoded.split_at_checked(12) else {
        return Err(anyhow!(
            "shipping datadir is shorter than its native header"
        ));
    };
    if header[..8] != SHIPPING_DATADIR_MAGIC {
        return Err(anyhow!(
            "shipping datadir is not native format version {SHIPPING_DATADIR_VERSION}; regenerate datadir.bin"
        ));
    }
    let version = u32::from_le_bytes(header[8..12].try_into().expect("fixed header length"));
    if version != SHIPPING_DATADIR_VERSION {
        return Err(anyhow!(
            "unsupported shipping datadir version {version}; expected {SHIPPING_DATADIR_VERSION}"
        ));
    }
    bitcode::decode(payload).map_err(|error| anyhow!("native bitcode decode: {error:?}"))
}

pub fn encode_mission_native(mission: &ShippingMission) -> Vec<u8> {
    let payload = bitcode::encode(mission);
    let mut encoded = Vec::with_capacity(12 + payload.len());
    encoded.extend_from_slice(&SHIPPING_MISSION_MAGIC);
    encoded.extend_from_slice(&SHIPPING_MISSION_VERSION.to_le_bytes());
    encoded.extend_from_slice(&payload);
    encoded
}

pub fn decode_mission_compressed(compressed: &[u8]) -> Result<ShippingMission> {
    let blob = zstd_decompress(compressed)?;
    let Some((header, payload)) = blob.split_at_checked(12) else {
        return Err(anyhow!(
            "shipping mission payload is shorter than its header"
        ));
    };
    if header[..8] != SHIPPING_MISSION_MAGIC {
        return Err(anyhow!("shipping mission payload has invalid magic"));
    }
    let version = u32::from_le_bytes(header[8..12].try_into().expect("fixed header length"));
    if version != SHIPPING_MISSION_VERSION {
        return Err(anyhow!(
            "unsupported shipping mission version {version}; expected {SHIPPING_MISSION_VERSION}"
        ));
    }
    bitcode::decode(payload).map_err(|error| anyhow!("native bitcode mission decode: {error:?}"))
}

fn zstd_decompress(compressed: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = zstd::stream::read::Decoder::new(compressed).context("zstd decoder init")?;
    decoder
        .window_log_max(30)
        .context("zstd window_log_max=30")?;
    let mut blob = Vec::with_capacity(compressed.len() * 4);
    std::io::Read::read_to_end(&mut decoder, &mut blob).context("zstd decompress")?;
    Ok(blob)
}

/// zstd level 22 with adaptive windows capped at the native 31-bit maximum.
pub fn zstd_max_compress(bytes: &[u8]) -> Result<Vec<u8>> {
    zstd_compress_with_window(bytes, 31)
}

/// zstd level 22 with an adaptive `windowLog` capped by the caller (10..=31).
/// Pledging the input size lets zstd advertise only the window this frame can
/// actually use. Split RHS chunks consequently require at most about 16 MiB
/// instead of claiming a 1 GiB wasm decoder window, with effectively neutral
/// compressed size.
pub fn zstd_compress_with_window(bytes: &[u8], max_window_log: u32) -> Result<Vec<u8>> {
    use zstd::stream::raw::CParameter;
    use zstd::stream::write::Encoder;
    if !(10..=31).contains(&max_window_log) {
        return Err(anyhow!(
            "zstd maximum window_log must be in 10..=31, got {max_window_log}"
        ));
    }
    let content_window_log = usize::BITS - bytes.len().saturating_sub(1).leading_zeros();
    let window_log = content_window_log.clamp(10, max_window_log);
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, 22).context("zstd encoder")?;
    enc.set_pledged_src_size(Some(bytes.len() as u64))
        .context("zstd pledged source size")?;
    enc.set_parameter(CParameter::WindowLog(window_log))
        .with_context(|| format!("zstd window_log={window_log}"))?;
    enc.set_parameter(CParameter::EnableLongDistanceMatching(true))
        .context("zstd long=1")?;
    std::io::Write::write_all(&mut enc, bytes).context("zstd write")?;
    enc.finish().context("zstd finish")?;
    Ok(out)
}

/// Convenience: look for `<data_dir>/datadir.bin`. Returns `Ok(None)` if
/// the file isn't present (legacy datadir), `Ok(Some(_))` on success.
pub fn try_load(data_dir: &Path) -> Result<Option<ShippingDatadir>> {
    let path = data_dir.join("datadir.bin");
    match robin_util::asset_fs::read(&path) {
        Ok(compressed) => {
            let mut datadir = ShippingDatadir::from_compressed_bytes(&compressed)
                .with_context(|| format!("decode {}", path.display()))?;
            datadir.source_dir = Some(data_dir.to_path_buf());
            Ok(Some(datadir))
        }
        Err(robin_util::asset_fs::AssetError::NotFound(_)) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

/// Instance form of [`try_load`]. Existence and open failures stay distinct:
/// only a genuine not-found result selects the legacy loose-file path.
pub fn try_load_from(
    vfs: &robin_util::asset_fs::AssetVfs,
    data_dir: &Path,
) -> Result<Option<ShippingDatadir>> {
    let path = data_dir.join("datadir.bin");
    match vfs.read(&path) {
        Ok(compressed) => {
            let mut datadir = ShippingDatadir::from_compressed_bytes(&compressed)
                .with_context(|| format!("decode {}", path.display()))?;
            datadir.source_dir = Some(data_dir.to_path_buf());
            Ok(Some(datadir))
        }
        Err(robin_util::asset_fs::AssetError::NotFound(_)) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

// ---------------------------------------------------------------------------
//  Process-global accessor
// ---------------------------------------------------------------------------

/// A parsed shipping payload and the VFS it was mounted into.
///
/// Keeping these together prevents startup from publishing parsed data while
/// silently failing to publish its raw-file mount (or vice versa).
#[derive(Debug)]
pub struct ShippingAssets {
    datadir: Arc<ShippingDatadir>,
    vfs: Arc<robin_util::asset_fs::AssetVfs>,
}

impl ShippingAssets {
    pub fn install(
        mut datadir: Arc<ShippingDatadir>,
        vfs: Arc<robin_util::asset_fs::AssetVfs>,
    ) -> Result<Self> {
        let raw: robin_util::asset_fs::Bundle = if let Some(unique) = Arc::get_mut(&mut datadir) {
            std::mem::take(&mut unique.raw)
                .into_iter()
                .map(|(path, bytes)| (path, bytes.into()))
                .collect()
        } else {
            datadir
                .raw
                .iter()
                .map(|(path, bytes)| (path.clone(), bytes.clone().into()))
                .collect()
        };
        let raw = Arc::new(raw);
        datadir
            .boot_raw_bundle
            .set(raw.clone())
            .map_err(|_| anyhow!("shipping boot raw bundle was already installed"))?;
        vfs.mount_bundle_first(raw)
            .context("mount shipping raw asset bundle")?;
        Ok(Self { datadir, vfs })
    }

    pub fn datadir(&self) -> &Arc<ShippingDatadir> {
        &self.datadir
    }

    pub fn vfs(&self) -> &Arc<robin_util::asset_fs::AssetVfs> {
        &self.vfs
    }
}

static GLOBAL: OnceLock<Arc<ShippingAssets>> = OnceLock::new();

/// Install a shipping datadir as the process-wide instance so lower-level
/// loaders can consult it for pre-parsed data. Installation and VFS mount
/// failures are returned to the startup boundary.
pub fn install_global(dd: Arc<ShippingDatadir>) -> Result<Arc<ShippingDatadir>> {
    if GLOBAL.get().is_some() {
        return Err(anyhow!("shipping datadir already installed"));
    }
    let installed = Arc::new(ShippingAssets::install(
        dd,
        robin_util::asset_fs::global().clone(),
    )?);
    GLOBAL
        .set(installed)
        .map_err(|_| anyhow!("shipping datadir concurrently installed"))?;
    Ok(global()
        .expect("shipping global was set immediately above")
        .clone())
}

/// Access the installed shipping datadir, if any.
pub fn global() -> Option<&'static Arc<ShippingDatadir>> {
    GLOBAL.get().map(|installed| installed.datadir())
}

/// Access the co-owned runtime shipping/VFS installation.
pub fn global_assets() -> Option<&'static Arc<ShippingAssets>> {
    GLOBAL.get()
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_util::asset_fs::{AssetVfs, Bundle};

    #[test]
    fn native_shipping_format_roundtrips_and_rejects_legacy_payloads() {
        let mut datadir = ShippingDatadir::default();
        datadir.raw.insert("test.bin".into(), vec![1, 2, 3]);
        datadir
            .audio_durations_ms
            .insert("musics/menu.opus".into(), 9_876);
        datadir.missions.insert(
            "MissionOne".into(),
            ShippingMissionRef {
                files: vec!["missions/mission-one.rhmission.zst".into()],
            },
        );
        datadir
            .character_rhs_files
            .insert(7, vec!["rhs/character-seven.rhmission.zst".into()]);
        datadir
            .character_audio_files
            .insert(7, vec!["audio/character-seven.rhmission.zst".into()]);
        datadir.character_exclamation_ids.insert(7, 0x5043_5248);
        datadir
            .mission_exclamation_ids
            .insert("MissionOne".into(), vec![0x534F_4C44]);
        datadir.saved_world_rhs_files = vec!["rhs/saved-objects.rhmission.zst".into()];

        let encoded = encode_native(&datadir);
        assert_eq!(&encoded[..8], b"RHDDNAT6");
        assert_eq!(&encoded[..8], &SHIPPING_DATADIR_MAGIC);
        let decoded = decode_native(&encoded).expect("decode native shipping datadir");
        assert_eq!(decoded.raw.get("test.bin"), Some(&vec![1, 2, 3]));
        assert_eq!(
            decoded.audio_durations_ms.get("musics/menu.opus"),
            Some(&9_876)
        );
        assert_eq!(
            decoded.mission_ref("MissionOne").unwrap().files,
            vec!["missions/mission-one.rhmission.zst"]
        );
        assert_eq!(
            decoded.character_rhs_files.get(&7).unwrap(),
            &["rhs/character-seven.rhmission.zst"]
        );
        assert_eq!(
            decoded.character_audio_files.get(&7).unwrap(),
            &["audio/character-seven.rhmission.zst"]
        );
        assert_eq!(
            decoded.character_exclamation_ids.get(&7),
            Some(&0x5043_5248)
        );
        assert_eq!(
            decoded.mission_exclamation_ids.get("MissionOne").unwrap(),
            &[0x534F_4C44]
        );
        assert_eq!(
            decoded.saved_world_rhs_files,
            ["rhs/saved-objects.rhmission.zst"]
        );

        let legacy_unversioned = bitcode::encode(&datadir);
        let error = decode_native(&legacy_unversioned).unwrap_err();
        assert!(error.to_string().contains("regenerate datadir.bin"));
    }

    #[test]
    fn mission_payload_roundtrips_independently() {
        let mut mission = ShippingMission::default();
        mission
            .raw
            .insert("levels/day/map.min".into(), vec![9, 8, 7]);
        mission
            .audio_durations_ms
            .insert("sounds/arrow.opus".into(), 1_234);
        let encoded = encode_mission_native(&mission);
        assert_eq!(&encoded[..8], b"RHMISN03");
        let compressed = zstd_compress_with_window(&encoded, 30).unwrap();
        let decoded = decode_mission_compressed(&compressed).unwrap();
        assert_eq!(decoded.raw.get("levels/day/map.min"), Some(&vec![9, 8, 7]));
        assert_eq!(
            decoded.audio_durations_ms.get("sounds/arrow.opus"),
            Some(&1_234)
        );
    }

    #[test]
    fn mission_parts_merge_disjoint_sprite_slots() {
        let sprite = |value| ShippingSprite {
            width: 1,
            height: 1,
            dictionary_index: 0,
            packed_data: Arc::new(vec![value]),
        };
        let bank = |sprites| ShippingSpriteBank {
            signature: 42,
            dictionaries: Vec::new(),
            sprite_count: 2,
            sprites,
        };
        let mut merged = ShippingMission {
            sprite_bank: Some(bank(Vec::new())),
            ..ShippingMission::default()
        };
        merged
            .merge_from(ShippingMission {
                sprite_bank: Some(bank(vec![(0, sprite(10))])),
                ..ShippingMission::default()
            })
            .unwrap();
        merged
            .merge_from(ShippingMission {
                sprite_bank: Some(bank(vec![(1, sprite(20))])),
                ..ShippingMission::default()
            })
            .unwrap();

        let sprites = &merged.sprite_bank.unwrap().sprites;
        assert_eq!(sprites[0].1.packed_data.as_slice(), &[10]);
        assert_eq!(sprites[1].1.packed_data.as_slice(), &[20]);
    }

    #[test]
    fn shipping_installation_owns_vfs_and_has_first_priority() {
        let vfs = Arc::new(AssetVfs::new());
        let mut loose = Bundle::new();
        loose.insert("shared.dat".to_string(), b"loose".to_vec().into());
        vfs.mount_bundle(Arc::new(loose)).unwrap();

        let mut datadir = ShippingDatadir::default();
        datadir
            .raw
            .insert("shared.dat".to_string(), b"shipping".to_vec());
        datadir
            .raw
            .insert("sounds/menu.opus".to_string(), vec![1, 2, 3, 4]);
        datadir
            .audio_durations_ms
            .insert("sounds/menu.opus".to_string(), 250);
        let installed = ShippingAssets::install(Arc::new(datadir), vfs.clone()).unwrap();

        assert!(Arc::ptr_eq(installed.vfs(), &vfs));
        assert!(installed.datadir().raw.is_empty());
        assert_eq!(
            installed.datadir().raw_asset("shared.dat"),
            Some(&b"shipping"[..])
        );
        assert_eq!(
            installed
                .datadir()
                .active_audio_metadata(Path::new("Data/Sounds/Menu.wav")),
            Some((4, 250))
        );
        assert_eq!(installed.vfs().read("shared.dat").unwrap(), b"shipping");
    }

    #[test]
    fn shipping_installation_propagates_invalid_bundle_path() {
        let vfs = Arc::new(AssetVfs::new());
        let mut datadir = ShippingDatadir::default();
        datadir
            .raw
            .insert("../escape.dat".to_string(), b"bad".to_vec());

        let error = ShippingAssets::install(Arc::new(datadir), vfs).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("mount shipping raw asset bundle")
        );
    }
}

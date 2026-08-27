//! Shipping datadir manifest and lazily loaded mission payloads.
//!
//! Produced by the `convert_datadir --format shipping` binary and loaded
//! at engine startup (see [`try_load`]). When a shipping datadir is
//! present, individual subsystem loaders (`ProfileManager::load_all_legacy_cpf`,
//! `FrameHolder::initialize_sprite_bank`, `ResourceManager::attach_resource_file`,
//! etc.) consult it instead of reading legacy files off disk.

use std::collections::BTreeMap;
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
    /// Independently compressed payload to fetch before starting each mission.
    pub missions: BTreeMap<String, ShippingMissionRef>,
    /// Runtime-only source directory containing `datadir.bin` and its payloads.
    #[serde(skip)]
    #[bitcode(skip)]
    source_dir: Option<PathBuf>,
    /// Runtime-only HTTP base used by the browser build.
    #[serde(skip)]
    #[bitcode(skip)]
    remote_base_url: Option<String>,
    /// Payloads already installed for this process. Kept out of the manifest.
    #[serde(skip)]
    #[bitcode(skip)]
    loaded_missions: RwLock<BTreeMap<String, Arc<ShippingMission>>>,
    #[serde(skip)]
    #[bitcode(skip)]
    loaded_files: RwLock<BTreeMap<String, Arc<ShippingMission>>>,
    #[serde(skip)]
    #[bitcode(skip)]
    active_mission: RwLock<Option<String>>,
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
            missions: BTreeMap::new(),
            source_dir: None,
            remote_base_url: None,
            loaded_missions: RwLock::new(BTreeMap::new()),
            loaded_files: RwLock::new(BTreeMap::new()),
            active_mission: RwLock::new(None),
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
    /// One slot per bank id. `None` for sprites no `.rhs` referenced.
    pub sprites: Vec<Option<ShippingSprite>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingSprite {
    pub width: u16,
    pub height: u16,
    pub dictionary_index: u16,
    /// Packed pixel data (RLE or dictionary-indexed).
    pub packed_data: Vec<u16>,
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

    pub fn install_mission(&self, mission: &str, payload: ShippingMission) -> Result<()> {
        if !payload.levels.contains_key(mission) {
            return Err(anyhow!(
                "shipping payload for {mission} does not contain its level"
            ));
        }
        self.loaded_missions
            .write()
            .expect("shipping mission lock poisoned")
            .insert(mission.to_owned(), Arc::new(payload));
        self.activate_mission(mission)?;
        Ok(())
    }

    pub fn cache_file(&self, file: &str, payload: ShippingMission) -> Arc<ShippingMission> {
        let payload = Arc::new(payload);
        self.loaded_files
            .write()
            .expect("shipping file lock poisoned")
            .insert(file.to_owned(), payload.clone());
        payload
    }

    pub fn cached_file(&self, file: &str) -> Option<Arc<ShippingMission>> {
        self.loaded_files
            .read()
            .expect("shipping file lock poisoned")
            .get(file)
            .cloned()
    }

    pub fn install_mission_parts(
        &self,
        mission: &str,
        parts: impl IntoIterator<Item = Arc<ShippingMission>>,
    ) -> Result<()> {
        let mut merged = ShippingMission {
            sprite_bank: self.sprite_bank.clone(),
            ..ShippingMission::default()
        };
        for part in parts {
            merged.merge_from(&part)?;
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
        let mut parts = Vec::with_capacity(reference.files.len());
        for file in &reference.files {
            let part = if let Some(cached) = self.cached_file(file) {
                cached
            } else {
                let path = self.source_file_path(file)?;
                let compressed = robin_util::asset_fs::read(&path)
                    .with_context(|| format!("read {}", path.display()))?;
                let payload = decode_mission_compressed(&compressed)
                    .with_context(|| format!("decode {}", path.display()))?;
                self.cache_file(file, payload)
            };
            parts.push(part);
        }
        self.install_mission_parts(mission, parts)
    }

    pub fn activate_mission(&self, mission: &str) -> Result<()> {
        if self.active_mission_name().as_deref() == Some(mission) {
            return Ok(());
        }
        let payload = self
            .loaded_mission(mission)
            .ok_or_else(|| anyhow!("shipping mission {mission} has not been loaded"))?;
        robin_util::asset_fs::global()
            .mount_bundle_first(Arc::new(payload.raw.clone()))
            .context("mount shipping mission assets")?;
        *self
            .active_mission
            .write()
            .expect("shipping active mission lock poisoned") = Some(mission.to_owned());
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

    pub fn mission_sprite_bank(&self, mission: &str) -> Option<ShippingSpriteBank> {
        self.loaded_mission(mission)
            .and_then(|payload| payload.sprite_bank.clone())
            .or_else(|| self.sprite_bank.clone())
    }

    pub fn active_sprite_bank(&self) -> Option<ShippingSpriteBank> {
        let active = self
            .active_mission
            .read()
            .expect("shipping active mission lock poisoned")
            .clone();
        active
            .as_deref()
            .and_then(|mission| self.mission_sprite_bank(mission))
            .or_else(|| self.sprite_bank.clone())
    }

    pub fn active_mission_name(&self) -> Option<String> {
        self.active_mission
            .read()
            .expect("shipping active mission lock poisoned")
            .clone()
    }

    pub fn mission_raw(&self, mission: &str, key: &str) -> Option<Vec<u8>> {
        self.loaded_mission(mission)
            .and_then(|payload| payload.raw.get(key).cloned())
            .or_else(|| self.raw.get(key).cloned())
    }
}

impl ShippingMission {
    fn merge_from(&mut self, source: &Self) -> Result<()> {
        merge_unique(&mut self.levels, &source.levels, "level")?;
        merge_unique(&mut self.scripts, &source.scripts, "script")?;
        merge_unique(&mut self.rhs_files, &source.rhs_files, "RHS")?;
        merge_unique(&mut self.raw, &source.raw, "raw asset")?;
        let Some(source_bank) = source.sprite_bank.as_ref() else {
            return Ok(());
        };
        let bank = self.sprite_bank.get_or_insert_with(|| ShippingSpriteBank {
            signature: source_bank.signature,
            dictionaries: source_bank.dictionaries.clone(),
            sprites: vec![None; source_bank.sprites.len()],
        });
        if bank.signature != source_bank.signature
            || bank.sprites.len() != source_bank.sprites.len()
        {
            return Err(anyhow!("shipping sprite-bank parts are incompatible"));
        }
        if bank.dictionaries.is_empty() {
            bank.dictionaries = source_bank.dictionaries.clone();
        } else if !source_bank.dictionaries.is_empty()
            && bitcode::encode(&bank.dictionaries) != bitcode::encode(&source_bank.dictionaries)
        {
            return Err(anyhow!("shipping sprite-bank dictionaries conflict"));
        }
        for (index, sprite) in source_bank.sprites.iter().enumerate() {
            let Some(sprite) = sprite else { continue };
            if let Some(existing) = bank.sprites[index].as_ref()
                && bitcode::encode(existing) != bitcode::encode(sprite)
            {
                return Err(anyhow!(
                    "shipping sprite-bank parts conflict at sprite {index}"
                ));
            }
            bank.sprites[index] = Some(sprite.clone());
        }
        Ok(())
    }
}

fn merge_unique<K, V>(dst: &mut BTreeMap<K, V>, src: &BTreeMap<K, V>, kind: &str) -> Result<()>
where
    K: Ord + Clone + std::fmt::Debug,
    V: Clone,
{
    for (key, value) in src {
        if dst.contains_key(key) {
            return Err(anyhow!("duplicate shipping {kind} key {key:?}"));
        }
        dst.insert(key.clone(), value.clone());
    }
    Ok(())
}

const SHIPPING_DATADIR_MAGIC: [u8; 8] = *b"RHDDNAT4";
const SHIPPING_MISSION_MAGIC: [u8; 8] = *b"RHMISN01";
pub const SHIPPING_DATADIR_VERSION: u32 = 4;
pub const SHIPPING_MISSION_VERSION: u32 = 1;

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

/// zstd level 22 with a 31-bit long-range window. Matches the converter.
pub fn zstd_max_compress(bytes: &[u8]) -> Result<Vec<u8>> {
    zstd_compress_with_window(bytes, 31)
}

/// zstd level 22 with a caller-chosen `windowLog` (must be 10..=31). Use
/// 31 for native builds; 30 is the ceiling for 32-bit zstd builds (wasm32).
pub fn zstd_compress_with_window(bytes: &[u8], window_log: u32) -> Result<Vec<u8>> {
    use zstd::stream::raw::CParameter;
    use zstd::stream::write::Encoder;
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, 22).context("zstd encoder")?;
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
        datadir: Arc<ShippingDatadir>,
        vfs: Arc<robin_util::asset_fs::AssetVfs>,
    ) -> Result<Self> {
        vfs.mount_bundle_first(Arc::new(datadir.raw.clone()))
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
pub fn install_global(dd: Arc<ShippingDatadir>) -> Result<()> {
    if GLOBAL.get().is_some() {
        return Err(anyhow!("shipping datadir already installed"));
    }
    let installed = Arc::new(ShippingAssets::install(
        dd,
        robin_util::asset_fs::global().clone(),
    )?);
    GLOBAL
        .set(installed)
        .map_err(|_| anyhow!("shipping datadir concurrently installed"))
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
        datadir.missions.insert(
            "MissionOne".into(),
            ShippingMissionRef {
                files: vec!["missions/mission-one.rhmission.zst".into()],
            },
        );

        let encoded = encode_native(&datadir);
        assert_eq!(&encoded[..8], &SHIPPING_DATADIR_MAGIC);
        let decoded = decode_native(&encoded).expect("decode native shipping datadir");
        assert_eq!(decoded.raw.get("test.bin"), Some(&vec![1, 2, 3]));
        assert_eq!(
            decoded.mission_ref("MissionOne").unwrap().files,
            vec!["missions/mission-one.rhmission.zst"]
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
        let encoded = encode_mission_native(&mission);
        let compressed = zstd_compress_with_window(&encoded, 30).unwrap();
        let decoded = decode_mission_compressed(&compressed).unwrap();
        assert_eq!(decoded.raw.get("levels/day/map.min"), Some(&vec![9, 8, 7]));
    }

    #[test]
    fn mission_parts_merge_disjoint_sprite_slots() {
        let sprite = |value| ShippingSprite {
            width: 1,
            height: 1,
            dictionary_index: 0,
            packed_data: vec![value],
        };
        let bank = |sprites| ShippingSpriteBank {
            signature: 42,
            dictionaries: Vec::new(),
            sprites,
        };
        let mut merged = ShippingMission {
            sprite_bank: Some(bank(vec![None, None])),
            ..ShippingMission::default()
        };
        merged
            .merge_from(&ShippingMission {
                sprite_bank: Some(bank(vec![Some(sprite(10)), None])),
                ..ShippingMission::default()
            })
            .unwrap();
        merged
            .merge_from(&ShippingMission {
                sprite_bank: Some(bank(vec![None, Some(sprite(20))])),
                ..ShippingMission::default()
            })
            .unwrap();

        let sprites = &merged.sprite_bank.unwrap().sprites;
        assert_eq!(sprites[0].as_ref().unwrap().packed_data, vec![10]);
        assert_eq!(sprites[1].as_ref().unwrap().packed_data, vec![20]);
    }

    #[test]
    fn shipping_installation_owns_vfs_and_has_first_priority() {
        let vfs = Arc::new(AssetVfs::new());
        let mut loose = Bundle::new();
        loose.insert("shared.dat".to_string(), b"loose".to_vec());
        vfs.mount_bundle(Arc::new(loose)).unwrap();

        let mut datadir = ShippingDatadir::default();
        datadir
            .raw
            .insert("shared.dat".to_string(), b"shipping".to_vec());
        let installed = ShippingAssets::install(Arc::new(datadir), vfs.clone()).unwrap();

        assert!(Arc::ptr_eq(installed.vfs(), &vfs));
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

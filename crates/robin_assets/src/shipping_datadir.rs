//! Shipping datadir — a single bitcode+zstd blob with every parsed
//! subsystem the engine needs at boot.
//!
//! Produced by the `convert_datadir --format shipping` binary and loaded
//! at engine startup (see [`try_load_global`]). When a shipping datadir is
//! present, individual subsystem loaders (`ProfileManager::load_all_legacy_cpf`,
//! `FrameHolder::initialize_sprite_bank`, `ResourceManager::attach_resource_file`,
//! etc.) consult it instead of reading legacy files off disk.

use std::path::Path;

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
#[derive(Default, Debug, Serialize, Deserialize)]
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RhsData {
    pub signature: u32,
    pub profiles: Vec<(String, SpriteInfo)>,
}

/// Shipping-ready sprite bank. Unlike the runtime [`crate::frame_holder::FrameHolder`],
/// this carries every sprite's packed pixel data inline (the runtime
/// version marks `packed_data` `#[serde(skip)]` so savegames stay small).
#[derive(Debug, Serialize, Deserialize)]
pub struct ShippingSpriteBank {
    pub signature: u32,
    pub dictionaries: Vec<FrameDictionary>,
    /// One slot per bank id. `None` for sprites no `.rhs` referenced.
    pub sprites: Vec<Option<ShippingSprite>>,
}

#[derive(Debug, Serialize, Deserialize)]
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
    /// Parse a shipping datadir blob: zstd decompress + bitcode deserialize.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let compressed =
            robin_util::asset_fs::read(path).with_context(|| format!("read {}", path.display()))?;
        Self::from_compressed_bytes(&compressed)
            .with_context(|| format!("decode {}", path.display()))
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
        let dd: ShippingDatadir =
            bitcode::deserialize(&blob).map_err(|e| anyhow!("bitcode decode: {e:?}"))?;
        tracing::info!(
            "loaded shipping datadir ({} → {} bytes)",
            compressed.len(),
            blob.len()
        );
        Ok(dd)
    }
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
    if !robin_util::asset_fs::exists(&path) {
        return Ok(None);
    }
    ShippingDatadir::load_from_file(&path).map(Some)
}

// ---------------------------------------------------------------------------
//  Process-global accessor
// ---------------------------------------------------------------------------

use std::sync::{Arc, OnceLock};

static GLOBAL: OnceLock<Arc<ShippingDatadir>> = OnceLock::new();

/// Install a shipping datadir as the process-wide instance so lower-level
/// loaders can consult it for pre-parsed data.  Returns `Err` with the
/// passed `Arc` if a datadir is already installed.
pub fn install_global(dd: Arc<ShippingDatadir>) -> std::result::Result<(), Arc<ShippingDatadir>> {
    GLOBAL.set(dd)
}

/// Access the installed shipping datadir, if any.
pub fn global() -> Option<&'static Arc<ShippingDatadir>> {
    GLOBAL.get()
}

//! Offline migration of a trusted v15 datadir.bin to v16 engine picture metadata.
//! Usage: migrate_picture_metadata OLD_DATADIR_BIN NEW_DATADIR_BIN
//! Mission/audio/image payload files are unchanged; copy the directory separately.
//! The legacy wire adapter exists only in this tool, not the runtime loader.
use anyhow::{Context, Result, ensure};
use robin_assets::picture::Picture;
use robin_assets::res_descr::LevelDescriptors;
use robin_assets::resource_manager::{
    EncodedPicture, MouseEntry, PictureOpacityMetadata, ResourceManager,
};
use robin_assets::scb::ScbFile;
use robin_assets::shipping_datadir::{self, *};
use robin_engine::level_data::LoadedLevel;
use robin_engine::profiles::ProfileManager;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::Path;

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct OldFileEntry {
    file_path: String,
    file_offset: u64,
    resource_type: [u8; 4],
}

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct OldResource {
    pictures: HashMap<i32, Vec<Option<Picture>>>,
    encoded_pictures: HashMap<i32, Vec<Option<EncodedPicture>>>,
    mouse_entries: HashMap<i32, MouseEntry>,
    strings: HashMap<i32, Vec<String>>,
    waves: HashMap<i32, Vec<String>>,
    references: HashMap<i32, u32>,
    file_entries: HashMap<i32, OldFileEntry>,
    recovery_disabled: bool,
}

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct NewResource {
    pictures: HashMap<i32, Vec<Option<Picture>>>,
    encoded_pictures: HashMap<i32, Vec<Option<EncodedPicture>>>,
    mouse_entries: HashMap<i32, MouseEntry>,
    strings: HashMap<i32, Vec<String>>,
    waves: HashMap<i32, Vec<String>>,
    picture_opacity: HashMap<i32, Vec<Option<PictureOpacityMetadata>>>,
    references: HashMap<i32, u32>,
    file_entries: HashMap<i32, OldFileEntry>,
    recovery_disabled: bool,
}

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct OldLocale {
    source_lcid: Option<String>,
    aliases: BTreeSet<String>,
    profiles: Option<ProfileManager>,
    res_files: BTreeMap<String, OldResource>,
    pak_files: BTreeMap<String, Vec<EncodedPicture>>,
    red_files: BTreeMap<String, LevelDescriptors>,
    raw: BTreeMap<String, Vec<u8>>,
}

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct OldDatadir {
    profiles: Option<ProfileManager>,
    res_files: std::collections::BTreeMap<String, OldResource>,
    pak_files: std::collections::BTreeMap<String, Vec<EncodedPicture>>,
    red_files: std::collections::BTreeMap<String, LevelDescriptors>,
    levels: std::collections::BTreeMap<String, LoadedLevel>,
    scripts: std::collections::BTreeMap<String, ScbFile>,
    rhs_files: std::collections::BTreeMap<String, RhsData>,
    sprite_bank: Option<ShippingSpriteBank>,
    raw: std::collections::BTreeMap<String, Vec<u8>>,
    audio_durations_ms: BTreeMap<String, u32>,
    audio_assets: BTreeMap<String, ShippingAudioAsset>,
    missions: BTreeMap<String, ShippingMissionRef>,
    character_rhs_files: BTreeMap<u32, Vec<String>>,
    character_audio_files: BTreeMap<u32, Vec<String>>,
    character_exclamation_ids: BTreeMap<u32, u32>,
    mission_exclamation_ids: BTreeMap<String, Vec<u32>>,
    saved_world_rhs_files: Vec<String>,
    locales: BTreeMap<String, OldLocale>,
}

impl OldResource {
    fn upgrade(self) -> Result<ResourceManager> {
        let Self {
            pictures,
            encoded_pictures,
            mouse_entries,
            strings,
            waves,
            references,
            file_entries,
            recovery_disabled,
        } = self;
        let wire = NewResource {
            pictures,
            encoded_pictures,
            mouse_entries,
            strings,
            waves,
            references,
            file_entries,
            recovery_disabled,
            picture_opacity: HashMap::new(),
        };
        let mut manager: ResourceManager = bitcode::decode(&bitcode::encode(&wire))?;
        manager.prepare_engine_picture_metadata()?;
        Ok(manager)
    }
}

fn resources(old: BTreeMap<String, OldResource>) -> Result<BTreeMap<String, ResourceManager>> {
    old.into_iter()
        .map(|(name, resource)| {
            let upgraded = resource
                .upgrade()
                .with_context(|| format!("resource archive {name}"))?;
            Ok((name, upgraded))
        })
        .collect()
}

impl OldLocale {
    fn upgrade(self) -> Result<ShippingLocale> {
        Ok(ShippingLocale {
            source_lcid: self.source_lcid,
            aliases: self.aliases,
            profiles: self.profiles,
            res_files: resources(self.res_files)?,
            pak_files: self.pak_files,
            red_files: self.red_files,
            raw: self.raw,
        })
    }
}

impl OldDatadir {
    fn upgrade(self) -> Result<ShippingDatadir> {
        Ok(ShippingDatadir::from_payload(ShippingDatadirPayload {
            profiles: self.profiles,
            res_files: resources(self.res_files)?,
            pak_files: self.pak_files,
            red_files: self.red_files,
            levels: self.levels,
            scripts: self.scripts,
            rhs_files: self.rhs_files,
            sprite_bank: self.sprite_bank,
            raw: self.raw,
            audio_durations_ms: self.audio_durations_ms,
            audio_assets: self.audio_assets,
            missions: self.missions,
            character_rhs_files: self.character_rhs_files,
            character_audio_files: self.character_audio_files,
            character_exclamation_ids: self.character_exclamation_ids,
            mission_exclamation_ids: self.mission_exclamation_ids,
            saved_world_rhs_files: self.saved_world_rhs_files,
            locales: self
                .locales
                .into_iter()
                .map(|(name, locale)| Ok((name, locale.upgrade()?)))
                .collect::<Result<_>>()?,
        }))
    }
}

fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        arguments.len() == 2,
        "usage: migrate_picture_metadata OLD_DATADIR_BIN NEW_DATADIR_BIN"
    );
    let input = Path::new(&arguments[0]);
    let output = Path::new(&arguments[1]);
    ensure!(
        !output.exists(),
        "output must not already exist (the source remains untouched)"
    );
    let compressed = std::fs::read(input)?;
    let mut decoder = zstd::stream::read::Decoder::new(compressed.as_slice())?;
    decoder.window_log_max(30)?;
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() >= 12 && &bytes[..8] == b"RHDDNA15" && bytes[8..12] == 15u32.to_le_bytes(),
        "input must be a v15 datadir"
    );
    let old: OldDatadir = bitcode::decode(&bytes[12..]).context("decode v15 payload")?;
    let upgraded = old.upgrade()?;
    let encoded = shipping_datadir::encode_native(&upgraded);
    let compressed = shipping_datadir::zstd_max_compress(&encoded)?;
    let mut verified = ShippingDatadir::from_compressed_bytes(&compressed)?;
    {
        for manager in verified.res_files.values_mut() {
            for id in [
                robin_engine::resource_ids::RHID_GROUND_FOCUS,
                robin_engine::resource_ids::RHMAP_CORNER,
            ] {
                if manager.has_picture_resource(id) {
                    manager.get_picture_opacity_metadata(id)?;
                }
            }
        }
    }
    for locale in verified.locales.values_mut() {
        for manager in locale.res_files.values_mut() {
            for id in [
                robin_engine::resource_ids::RHID_GROUND_FOCUS,
                robin_engine::resource_ids::RHMAP_CORNER,
            ] {
                if manager.has_picture_resource(id) {
                    manager.get_picture_opacity_metadata(id)?;
                }
            }
        }
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, &compressed)?;
    println!(
        "migrated {} -> {}: {} bytes; source audio/JXL and mission files unchanged",
        input.display(),
        output.display(),
        compressed.len()
    );
    Ok(())
}

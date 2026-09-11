//! Generate the core timing authority from English loose Data/Sounds trees.
//!
//! cargo run -p robin_rs --example generate_audio_durations --
//!   assets/core-datadir /path/to/english/fullgame/DATA/Sounds /path/to/english/demo/DATA/Sounds
//! Earlier roots win when releases contain the same sample/group. Later roots add demo-only data.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, ensure};
use robin_assets::resource_manager::ResourceManager;
use robin_engine::audio_durations::{
    AUDIO_DURATIONS_PATH, AUDIO_DURATIONS_VERSION, AudioDurations, sample_key,
};
use robin_engine::sbfile::SbFileSystem;
use robin_util::asset_fs::AssetVfs;

fn inventory(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            paths.extend(inventory(&path)?);
        } else if entry.file_type()?.is_file() {
            paths.push(path);
        } else {
            return Err(anyhow!(
                "unsupported node in English audio tree: {}",
                path.display()
            ));
        }
    }
    paths.sort();
    Ok(paths)
}

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    core: PathBuf,
    #[arg(required = true)]
    roots: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let Args { core, roots } = <Args as clap::Parser>::parse();
    let mut table = AudioDurations {
        version: AUDIO_DURATIONS_VERSION,
        locale: "en-US".into(),
        samples_ms: BTreeMap::new(),
        speech_groups: BTreeMap::new(),
    };
    for root in roots {
        let root = root.canonicalize()?;
        let paths = inventory(&root)?;
        for path in &paths {
            let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            if !ext.eq_ignore_ascii_case("wav") && !ext.eq_ignore_ascii_case("ogg") {
                continue;
            }
            let name = sample_key(
                path.strip_prefix(&root)?
                    .to_str()
                    .context("non-UTF8 audio path")?,
            )
            .map_err(anyhow::Error::msg)?;
            if table.samples_ms.contains_key(&name) {
                continue;
            }
            let bytes = std::fs::read(path)?;
            let duration = robin_rs::audio_backend::wav_duration_ms(&bytes)
                .with_context(|| format!("cannot derive English duration: {}", path.display()))?;
            table.samples_ms.insert(name, duration);
        }
        if !paths.iter().any(|path| {
            path.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("actors.res"))
        }) {
            eprintln!(
                "{}: accumulated {} samples (voice-only root)",
                root.display(),
                table.samples_ms.len()
            );
            continue;
        }
        let files = Arc::new(SbFileSystem::new(Arc::new(AssetVfs::new())));
        ensure!(
            files.set_primary_path(root.to_str().context("non-UTF8 sound root")?) == 0,
            "cannot mount English sound root"
        );
        let mut resources = ResourceManager::with_files(files);
        resources.attach_resource_file("Exclamations/actors.res")?;
        for path in &paths {
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("");
            if !stem.to_ascii_lowercase().starts_with("actor")
                || !path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("dat"))
            {
                continue;
            }
            let suffix = &stem[5..];
            ensure!(
                suffix.len() == 4,
                "invalid actor filename: {}",
                path.display()
            );
            let profile = u32::from_le_bytes(suffix.as_bytes().try_into()?);
            let (resource, groups) = robin_engine::sound_cache::parse_exclamation_file(
                &std::fs::read(path)?,
                profile & 0xffff_0000,
            )
            .map_err(anyhow::Error::msg)?;
            for (id, variants) in groups {
                if table.speech_groups.contains_key(&id) {
                    continue;
                }
                let names = variants
                    .into_iter()
                    .map(|variant| {
                        let sample = resources.get_sample(resource as i32, variant as usize)?;
                        sample_key(&format!("Exclamations/{sample}")).map_err(anyhow::Error::msg)
                    })
                    .collect::<Result<Vec<_>>>()?;
                table.speech_groups.insert(id, names);
            }
        }
        eprintln!(
            "{}: accumulated {} samples, {} speech groups",
            root.display(),
            table.samples_ms.len(),
            table.speech_groups.len()
        );
    }
    let mut bytes = serde_json::to_vec_pretty(&table)?;
    bytes.push(b'\n');
    AudioDurations::from_json(&bytes).map_err(anyhow::Error::msg)?;
    let destination = core.join(AUDIO_DURATIONS_PATH);
    std::fs::create_dir_all(destination.parent().unwrap())?;
    std::fs::write(&destination, &bytes)?;
    // Keep the required core inventory reviewable and consistent with the generated file.
    let manifest_path = core.join(robin_rs::core_overlay::CORE_OVERLAY_MANIFEST_PATH);
    let mut manifest: robin_rs::core_overlay::CoreOverlayManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    use sha2::{Digest, Sha256};
    manifest
        .files
        .retain(|entry| entry.path != AUDIO_DURATIONS_PATH);
    manifest
        .files
        .push(robin_rs::core_overlay::CoreOverlayFile {
            path: AUDIO_DURATIONS_PATH.into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(&bytes)),
        });
    manifest.files.sort_by(|a, b| a.path.cmp(&b.path));
    std::fs::write(
        manifest_path,
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    eprintln!("wrote {}", destination.display());
    Ok(())
}

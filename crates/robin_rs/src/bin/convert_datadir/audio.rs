//! Audio transformation, catalog construction and grouped packaging.
use super::*;

pub(super) fn insert_shipping_raw(
    payload: &mut ShippingMission,
    relative: &str,
    path: &Path,
) -> Result<()> {
    let relative = relative.replace('\\', "/").to_ascii_lowercase();
    let bytes = fs::read(path).with_context(|| format!("read audio {}", path.display()))?;
    if let Some(previous) = payload.raw.get(&relative) {
        if previous != &bytes {
            bail!("conflicting shipping audio sources for {relative}");
        }
    } else {
        payload.raw.insert(relative, bytes);
    }
    Ok(())
}

pub(super) fn is_common_audio_member(
    relative: &str,
    mission_dialogue_keys: &BTreeSet<String>,
) -> bool {
    !relative.starts_with("menu/")
        && !relative.starts_with("exclamations/")
        && !is_sound_source_audio(relative)
        && !mission_dialogue_keys.contains(&format!("sounds/{relative}"))
}

pub(super) fn is_sound_source_audio(relative: &str) -> bool {
    let Some(name) = relative.strip_prefix("snd_").and_then(|name| {
        name.strip_suffix(".wav")
            .or_else(|| name.strip_suffix(".ogg"))
    }) else {
        return false;
    };
    name.len() >= 3 && name.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Debug, Clone, Copy)]
pub(super) enum AudioKind {
    Voice,
    Effect,
    Music,
}

impl AudioKind {
    pub(super) fn bitrate_kbps(self) -> u32 {
        match self {
            Self::Voice => 24,
            Self::Effect => 48,
            // 64 -> 48 kbit/s together with switching music to the lossless
            // remaster sources (see `music_lossless_source`): encoding from
            // a clean master at 48k beats encoding the shipped lossy WAVs
            // at 64k, and drops ~25% of the music bytes.
            Self::Music => 48,
        }
    }

    pub(super) fn opus_application(self) -> &'static str {
        match self {
            Self::Voice => "voip",
            Self::Effect | Self::Music => "audio",
        }
    }
}

/// Logical bundle groups recorded during catalog construction, keyed by the
/// content-addressed asset file. A file referenced from several groups lands
/// in the "shared" bundle (see `bundle_grouped_audio`).
type AudioAssetGroups = std::collections::BTreeMap<String, std::collections::BTreeSet<String>>;

static AUDIO_ASSET_GROUPS: std::sync::LazyLock<std::sync::Mutex<AudioAssetGroups>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(AudioAssetGroups::new()));

// Converter-only provenance: remember the exact source used for each catalog
// entry, rather than inferring it from a WAV/OGG alias during boot cleanup.
static AUDIO_ASSET_SOURCES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<(PathBuf, String), PathBuf>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(super) fn catalog_source_bytes(assets_dir: &Path, relative: &str) -> Result<Option<Vec<u8>>> {
    let source = AUDIO_ASSET_SOURCES
        .lock()
        .expect("audio source map poisoned")
        .get(&(
            assets_dir.to_owned(),
            standalone_audio_logical_key(relative),
        ))
        .cloned();
    source
        .map(|path| {
            fs::read(&path).with_context(|| format!("read catalog source {}", path.display()))
        })
        .transpose()
}

pub(super) fn insert_shipping_audio(
    payload: &mut ShippingMission,
    catalog: &mut std::collections::BTreeMap<String, ShippingAudioAsset>,
    assets_dir: &Path,
    group: &str,
    relative: &str,
    path: &Path,
    kind: AudioKind,
    format: AudioFormat,
) -> Result<()> {
    let is_audio = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("wav") || extension.eq_ignore_ascii_case("ogg")
        });
    if !is_audio {
        return insert_shipping_raw(payload, relative, path);
    }

    let source = fs::read(path).with_context(|| format!("read audio {}", path.display()))?;
    let duration_ms = robin_rs::audio_backend::wav_duration_ms(&source).ok_or_else(|| {
        anyhow!(
            "cannot derive authoritative audio duration for {}",
            path.display()
        )
    })?;
    match format {
        AudioFormat::Source => {
            let relative = relative.replace('\\', "/").to_ascii_lowercase();
            if let Some(previous) = payload.raw.get(&relative) {
                if previous != &source {
                    bail!("conflicting shipping audio sources for {relative}");
                }
            } else {
                payload.raw.insert(relative.clone(), source);
            }
            insert_audio_duration(&mut payload.audio_durations_ms, relative, duration_ms)
        }
        AudioFormat::Opus => {
            let logical = standalone_audio_logical_key(relative);
            if let Some(existing) = catalog.get(&logical) {
                if existing.duration_ms != duration_ms {
                    bail!(
                        "conflicting source durations for standalone shipping audio {logical}: {} vs {duration_ms}",
                        existing.duration_ms
                    );
                }
            } else {
                // Music encodes from the lossless remaster drop when one
                // exists; the catalog duration above stays derived from the
                // GAME source, so deterministic timing tables are unchanged.
                let encode_source = if matches!(kind, AudioKind::Music) {
                    music_lossless_source(path)
                } else {
                    None
                };
                let bytes =
                    transcode_audio_to_opus(encode_source.as_deref().unwrap_or(path), kind)?;
                insert_standalone_audio(catalog, assets_dir, group, relative, &bytes, duration_ms)?;
                AUDIO_ASSET_SOURCES
                    .lock()
                    .expect("audio source map poisoned")
                    .insert((assets_dir.to_owned(), logical.clone()), path.to_owned());
            }
            // Opus bytes live only in the standalone catalog, but each boot
            // or mission payload retains this tiny exact-membership index.
            // Runtime warmup uses it to avoid decoding the whole catalog.
            insert_audio_duration(&mut payload.audio_durations_ms, logical, duration_ms)
        }
    }
}

/// Resolve the higher-quality lossless master for a music track, when the
/// optional `datadirs/music-rhmods-lossless` drop is present. Its
/// `mapping.json` maps game file names (under `DATA/Musics`) to remaster
/// file names; entries mapped to `null` have no clean source and fall back
/// to the game WAV. Matching is by file name, case-insensitive.
pub(super) fn music_lossless_source(game_path: &Path) -> Option<PathBuf> {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    static MAPPING: OnceLock<BTreeMap<String, PathBuf>> = OnceLock::new();
    let mapping = MAPPING.get_or_init(|| {
        let root = Path::new("datadirs/music-rhmods-lossless");
        let mapping_path = root.join("mapping.json");
        let json: serde_json::Value = match fs::read_to_string(&mapping_path)
            .map_err(anyhow::Error::from)
            .and_then(|text| serde_json::from_str(&text).map_err(anyhow::Error::from))
        {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!(
                    "no lossless music mapping at {} ({e:#}); music encodes from game sources",
                    mapping_path.display()
                );
                return BTreeMap::new();
            }
        };
        let lossless_root = json
            .get("lossless_root")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut resolved = BTreeMap::new();
        let Some(pairs) = json.get("game_to_lossless").and_then(|v| v.as_object()) else {
            tracing::warn!("lossless music mapping has no game_to_lossless object");
            return resolved;
        };
        for (game_name, lossless_name) in pairs {
            let Some(lossless_name) = lossless_name.as_str() else {
                continue; // null: no clean source for this track
            };
            // The drop has been seen both with and without the
            // `lossless_root` subdirectory; accept either layout.
            let candidates = [
                root.join(lossless_root).join(lossless_name),
                root.join(lossless_name),
            ];
            match candidates.into_iter().find(|p| p.is_file()) {
                Some(path) => {
                    resolved.insert(game_name.to_ascii_lowercase(), path);
                }
                None => tracing::warn!(
                    "lossless music mapping names missing file {lossless_name} for {game_name}"
                ),
            }
        }
        tracing::info!(
            tracks = resolved.len(),
            "music will encode from lossless remaster sources"
        );
        resolved
    });
    let name = game_path.file_name()?.to_str()?.to_ascii_lowercase();
    mapping.get(&name).cloned()
}

fn reader_matches_bytes(mut input: impl std::io::Read, expected: &[u8]) -> std::io::Result<bool> {
    let mut buffer = [0_u8; 8192];
    for chunk in expected.chunks(buffer.len()) {
        match input.read_exact(&mut buffer[..chunk.len()]) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(error),
        }
        if &buffer[..chunk.len()] != chunk {
            return Ok(false);
        }
    }
    match input.read_exact(&mut buffer[..1]) {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(true),
        Err(error) => Err(error),
    }
}

pub(super) fn insert_standalone_audio(
    catalog: &mut std::collections::BTreeMap<String, ShippingAudioAsset>,
    assets_dir: &Path,
    group: &str,
    relative: &str,
    bytes: &[u8],
    duration_ms: u32,
) -> Result<()> {
    let encoded_size =
        u32::try_from(bytes.len()).context("standalone Opus asset exceeds u32 byte length")?;
    let logical = standalone_audio_logical_key(relative);
    let filename = standalone_audio_filename(bytes);
    let asset = ShippingAudioAsset {
        file: format!("audio/assets/{filename}"),
        encoded_size,
        duration_ms,
        bundle_offset: None,
    };
    let entry = catalog.entry(logical);
    if let std::collections::btree_map::Entry::Occupied(existing) = &entry {
        if existing.get() != &asset {
            bail!(
                "conflicting standalone shipping audio for {}: {:?} vs {asset:?}",
                existing.key(),
                existing.get()
            );
        }
    }
    let output = assets_dir.join(&filename);
    match fs::symlink_metadata(&output) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "existing audio asset is not a regular file: {}",
                output.display()
            );
            let existing = fs::File::open(&output)
                .with_context(|| format!("open existing audio asset {}", output.display()))?;
            if !reader_matches_bytes(existing, bytes)
                .with_context(|| format!("read existing audio asset {}", output.display()))?
            {
                bail!("content-addressed audio collision at {}", output.display());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            publication::publish_bytes(&output, bytes)
                .with_context(|| format!("write audio asset {}", output.display()))?;
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("stat existing audio asset {}", output.display()));
        }
    }
    AUDIO_ASSET_GROUPS
        .lock()
        .expect("audio group recorder poisoned")
        .entry(asset.file.clone())
        .or_default()
        .insert(group.to_owned());
    if let std::collections::btree_map::Entry::Vacant(entry) = entry {
        entry.insert(asset);
    }
    Ok(())
}

pub(super) fn standalone_audio_logical_key(relative: &str) -> String {
    Path::new(&robin_util::asset_fs::bundle_key(Path::new(relative)))
        .with_extension("opus")
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn insert_audio_duration(
    durations: &mut std::collections::BTreeMap<String, u32>,
    relative: String,
    duration_ms: u32,
) -> Result<()> {
    match durations.entry(relative) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(duration_ms);
        }
        std::collections::btree_map::Entry::Occupied(entry) if *entry.get() == duration_ms => {}
        std::collections::btree_map::Entry::Occupied(entry) => {
            bail!(
                "conflicting source durations for shipping audio {}: {} vs {duration_ms}",
                entry.key(),
                entry.get()
            );
        }
    }
    Ok(())
}

/// Assets larger than this stay standalone files (music, long ambience):
/// they are few, individually worth an HTTP request, and bundling them
/// would force multi-MB downloads for one sound.
const AUDIO_BUNDLE_MAX_MEMBER: u32 = 262_144;

fn append_bundle_member(
    input: impl std::io::Read,
    expected: u32,
    bundle: &mut Vec<u8>,
) -> Result<()> {
    use std::io::Read as _;
    let start = bundle.len();
    let result = input.take(u64::from(expected) + 1).read_to_end(bundle);
    match result {
        Ok(read) if read == expected as usize => Ok(()),
        Ok(read) => {
            bundle.truncate(start);
            bail!("bundle member read {read} bytes but was cataloged as {expected}");
        }
        Err(error) => {
            bundle.truncate(start);
            Err(error).context("read bundle member")
        }
    }
}

/// Concatenate small catalog assets into one file per logical group
/// (recorded in [`AUDIO_ASSET_GROUPS`] during catalog construction; a file
/// referenced by several groups moves to the "shared" bundle). Rewrites the
/// catalog entries to (bundle file, offset) and deletes the standalone
/// files, so the browser fetches one request per group instead of ~2,000
/// tiny ones. Deterministic: members concatenate in content-hash order and
/// the bundle name is content-addressed.
pub(super) fn bundle_grouped_audio(
    dd: &mut robin_assets::shipping_datadir::ShippingDatadir,
    data_out: &Path,
) -> Result<()> {
    let groups_by_file = std::mem::take(
        &mut *AUDIO_ASSET_GROUPS
            .lock()
            .expect("audio group recorder poisoned"),
    );
    bundle_recorded_audio(dd, data_out, groups_by_file)
}

fn bundle_recorded_audio(
    dd: &mut robin_assets::shipping_datadir::ShippingDatadir,
    data_out: &Path,
    groups_by_file: AudioAssetGroups,
) -> Result<()> {
    use sha2::{Digest as _, Sha256};
    use std::collections::BTreeMap;
    // file -> (logical keys referencing it, encoded size)
    let mut file_refs = BTreeMap::<String, (Vec<String>, u32)>::new();
    for (logical, asset) in &dd.audio_assets {
        if asset.bundle_offset.is_some() {
            bail!("audio asset {logical} is already bundled; bundling must run once");
        }
        let entry = file_refs
            .entry(asset.file.clone())
            .or_insert_with(|| (Vec::new(), asset.encoded_size));
        if entry.1 != asset.encoded_size {
            bail!("conflicting encoded sizes recorded for {}", asset.file);
        }
        entry.0.push(logical.clone());
    }
    let mut members_by_group = BTreeMap::<&str, Vec<&str>>::new();
    for (file, (_, size)) in &file_refs {
        if *size >= AUDIO_BUNDLE_MAX_MEMBER {
            continue;
        }
        let groups = groups_by_file
            .get(file)
            .with_context(|| format!("catalog file {file} was never recorded in a bundle group"))?;
        let group = match groups.len() {
            0 => bail!("catalog file {file} was never recorded in a bundle group"),
            1 => groups.first().expect("len checked").as_str(),
            _ => "shared",
        };
        members_by_group
            .entry(group)
            .or_default()
            .push(file.as_str());
    }
    if members_by_group.is_empty() {
        return Ok(());
    }
    let bundles_dir = data_out.join("audio/bundles");
    fs::create_dir_all(&bundles_dir)?;
    let (mut bundled_files, mut bundle_count, mut bundled_bytes) = (0usize, 0usize, 0u64);
    let mut prepared = Vec::with_capacity(members_by_group.len());
    for (group, members) in members_by_group {
        // BTreeMap iteration already sorted members by content-hash name.
        let mut bytes = Vec::new();
        let mut offsets = Vec::with_capacity(members.len());
        for &file in &members {
            let input = fs::File::open(data_out.join(file))
                .with_context(|| format!("open bundle member {file}"))?;
            offsets.push(u32::try_from(bytes.len()).context("audio bundle exceeds u32")?);
            append_bundle_member(input, file_refs[file].1, &mut bytes)
                .with_context(|| format!("bundle member {file}"))?;
        }
        let digest = Sha256::digest(&bytes);
        let hash = hex::encode(&digest[..6]);
        let bundle_rel = format!("audio/bundles/{}-{hash}.bin", shipping_file_stem(group));
        publication::publish_bytes(&data_out.join(&bundle_rel), &bytes)
            .with_context(|| format!("write {bundle_rel}"))?;
        bundle_count += 1;
        bundled_bytes += bytes.len() as u64;
        prepared.push((bundle_rel, members, offsets));
    }
    // All inputs and bundle outputs must succeed before changing catalog
    // references or deleting any standalone source. Retain only metadata
    // between phases, not every bundle's encoded bytes.
    for (bundle_rel, members, offsets) in prepared {
        for (&file, offset) in members.iter().zip(offsets) {
            for logical in &file_refs[file].0 {
                let asset = dd
                    .audio_assets
                    .get_mut(logical)
                    .expect("logical key came from the catalog");
                asset.file = bundle_rel.clone();
                asset.bundle_offset = Some(offset);
            }
            // TODO: coordinate cleanup with final boot-index/manifest
            // publication; per-file atomic writes are not a directory transaction.
            fs::remove_file(data_out.join(file))
                .with_context(|| format!("remove bundled standalone {file}"))?;
            bundled_files += 1;
        }
    }
    tracing::info!(
        bundles = bundle_count,
        bundled_files,
        bundled_bytes,
        standalone_left = file_refs.len() - bundled_files,
        "grouped small audio assets into logical bundles"
    );
    Ok(())
}

pub(super) fn standalone_audio_filename(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(bytes);
    let hash = hex::encode(digest);
    format!("{hash}.opus")
}

/// Encode through FFmpeg's mature libopus integration, then remux the packets
/// with a fixed Ogg stream serial and vendor packet. FFmpeg randomizes Ogg
/// serials, which would otherwise make content-addressed shipping chunks and
/// `--resume` nondeterministic even when the encoded Opus packets are equal.
pub(super) fn transcode_audio_to_opus(source_path: &Path, kind: AudioKind) -> Result<Vec<u8>> {
    use std::io::Cursor;

    let bitrate = format!("{}k", kind.bitrate_kbps());
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(source_path)
        .args([
            "-map_metadata",
            "-1",
            "-vn",
            "-c:a",
            "libopus",
            "-b:a",
            &bitrate,
            "-vbr",
            "on",
            "-compression_level",
            "10",
            "-frame_duration",
            "20",
            "-application",
            kind.opus_application(),
            "-f",
            "ogg",
            "pipe:1",
        ])
        .output()
        .context("run ffmpeg with libopus support (is ffmpeg installed?)")?;
    if !output.status.success() {
        bail!(
            "ffmpeg Opus encode failed for {} ({}): {}",
            source_path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut reader = ogg::PacketReader::new(Cursor::new(output.stdout));
    let mut packets = Vec::new();
    while let Some(packet) = reader
        .read_packet()
        .with_context(|| format!("parse ffmpeg Ogg output for {}", source_path.display()))?
    {
        packets.push(packet);
    }
    if packets
        .first()
        .is_none_or(|packet| !packet.data.starts_with(b"OpusHead"))
    {
        bail!(
            "ffmpeg produced non-Opus Ogg output for {}",
            source_path.display()
        );
    }
    if packets.len() < 3 {
        bail!(
            "ffmpeg produced incomplete Opus stream for {}",
            source_path.display()
        );
    }
    packets[1].data = deterministic_opus_tags();

    let mut remuxed = Cursor::new(Vec::new());
    {
        use ogg::writing::{PacketWriteEndInfo, PacketWriter};
        let mut writer = PacketWriter::new(&mut remuxed);
        for packet in packets {
            let absgp = packet.absgp_page();
            let end = if packet.last_in_stream() {
                PacketWriteEndInfo::EndStream
            } else if packet.last_in_page() {
                PacketWriteEndInfo::EndPage
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            writer
                .write_packet(packet.data, 0x5248_4f50, end, absgp)
                .with_context(|| {
                    format!("write deterministic Ogg for {}", source_path.display())
                })?;
        }
    }
    Ok(remuxed.into_inner())
}

pub(super) fn deterministic_opus_tags() -> Vec<u8> {
    const VENDOR: &[u8] = b"robinhood-web-shipping";
    let mut tags = Vec::with_capacity(16 + VENDOR.len());
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    tags.extend_from_slice(VENDOR);
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags
}

pub(super) fn write_shipping_dependency(
    output_dir: &Path,
    label: &str,
    payload: &ShippingMission,
    window_log: u32,
    resume: bool,
) -> Result<Option<String>> {
    // Opus payloads deliberately keep their bytes in the browser-owned
    // catalog, so their exact boot/mission membership consists solely of
    // duration keys. Treat that metadata as real dependency content.
    if payload.raw.is_empty() && payload.audio_durations_ms.is_empty() {
        return Ok(None);
    }
    let (filename, compressed) =
        prepare_shipping_payload(output_dir, label, payload, window_log, resume)?;
    let compressed_len = write_prepared_shipping_payload(output_dir, &filename, compressed)?;
    tracing::info!(
        label,
        files = payload.raw.len(),
        audio_members = payload.audio_durations_ms.len(),
        bytes = compressed_len,
        "wrote shipping audio dependency"
    );
    Ok(Some(format!("audio/{filename}")))
}

#[cfg(test)]
mod boot_trim_tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct FailedRead;
    impl std::io::Read for FailedRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected read failure"))
        }
    }

    #[test]
    fn streamed_audio_comparison_propagates_read_errors() {
        use std::io::Read as _;
        assert!(reader_matches_bytes(FailedRead, &[]).is_err());
        assert!(reader_matches_bytes((&b"a"[..]).chain(FailedRead), b"ab").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn standalone_audio_reuse_refuses_symlink_targets() {
        let directory = tempfile::tempdir().unwrap();
        let target = tempfile::NamedTempFile::new().unwrap();
        let bytes = b"symlink audio fixture";
        fs::write(target.path(), bytes).unwrap();
        let path = directory.path().join(standalone_audio_filename(bytes));
        std::os::unix::fs::symlink(target.path(), &path).unwrap();
        let mut catalog = std::collections::BTreeMap::new();
        let error = insert_standalone_audio(
            &mut catalog,
            directory.path(),
            "test",
            "test.wav",
            bytes,
            100,
        )
        .unwrap_err();
        assert!(error.to_string().contains("not a regular file"));
        assert!(catalog.is_empty());
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(target.path()).unwrap(), bytes);
    }

    #[test]
    fn streamed_audio_comparison_checks_content_length_and_trailing_data() {
        for length in [0, 1, 8191, 8192, 8193, 200_000] {
            let bytes: Vec<_> = (0..length).map(|i| (i % 251) as u8).collect();
            assert!(reader_matches_bytes(bytes.as_slice(), &bytes).unwrap());
            let mut longer = bytes.clone();
            longer.push(0);
            assert!(!reader_matches_bytes(longer.as_slice(), &bytes).unwrap());
            assert!(!reader_matches_bytes(bytes.as_slice(), &longer).unwrap());
            if length != 0 {
                let mut different = bytes.clone();
                different[length / 2] ^= 1;
                assert!(!reader_matches_bytes(different.as_slice(), &bytes).unwrap());
            }
        }
        assert!(!reader_matches_bytes(std::io::repeat(0), &[0; 8193]).unwrap());
    }

    #[test]
    fn conflicting_catalog_insert_has_no_file_or_group_side_effects() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalog = std::collections::BTreeMap::new();
        let original_bytes = b"catalog conflict regression original";
        insert_standalone_audio(
            &mut catalog,
            directory.path(),
            "original",
            "effect.wav",
            original_bytes,
            100,
        )
        .unwrap();
        let before = catalog.clone();
        let rejected_bytes = b"catalog conflict regression rejected";
        assert!(
            insert_standalone_audio(
                &mut catalog,
                directory.path(),
                "rejected-content",
                "effect.wav",
                rejected_bytes,
                100
            )
            .is_err()
        );
        assert!(
            !directory
                .path()
                .join(standalone_audio_filename(rejected_bytes))
                .exists()
        );
        assert!(
            insert_standalone_audio(
                &mut catalog,
                directory.path(),
                "rejected-duration",
                "effect.wav",
                original_bytes,
                200
            )
            .is_err()
        );
        assert_eq!(catalog, before);
        let groups = AUDIO_ASSET_GROUPS.lock().unwrap();
        assert!(!groups.contains_key(&format!(
            "audio/assets/{}",
            standalone_audio_filename(rejected_bytes)
        )));
        assert!(
            !groups[&format!("audio/assets/{}", standalone_audio_filename(original_bytes))]
                .contains("rejected-duration")
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn empty_group_records_do_not_bypass_catalog_validation() {
        use robin_assets::shipping_datadir::ShippingAudioAsset;
        let directory = tempfile::tempdir().unwrap();
        let mut dd = robin_assets::shipping_datadir::ShippingDatadir::default();
        bundle_recorded_audio(&mut dd, directory.path(), AudioAssetGroups::new()).unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        dd.audio_assets.insert(
            "effect".into(),
            ShippingAudioAsset {
                file: "audio/assets/effect.opus".into(),
                encoded_size: 1,
                duration_ms: 100,
                bundle_offset: None,
            },
        );
        let original = dd.audio_assets.clone();
        let error =
            bundle_recorded_audio(&mut dd, directory.path(), AudioAssetGroups::new()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("never recorded in a bundle group")
        );
        assert_eq!(dd.audio_assets, original);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);

        // Large standalone entries do not need bundle groups, but still
        // participate in the once-only catalog validation.
        dd.audio_assets.get_mut("effect").unwrap().encoded_size = AUDIO_BUNDLE_MAX_MEMBER;
        bundle_recorded_audio(&mut dd, directory.path(), AudioAssetGroups::new()).unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        dd.audio_assets.get_mut("effect").unwrap().bundle_offset = Some(0);
        assert!(
            bundle_recorded_audio(&mut dd, directory.path(), AudioAssetGroups::new())
                .unwrap_err()
                .to_string()
                .contains("already bundled")
        );
    }

    #[test]
    fn later_bundle_input_failure_keeps_original_catalog_and_sources() {
        use robin_assets::shipping_datadir::ShippingAudioAsset;
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("audio/assets")).unwrap();
        let mut dd = robin_assets::shipping_datadir::ShippingDatadir::default();
        let mut groups = AudioAssetGroups::new();
        for (name, bytes) in [("a", &b"a"[..]), ("z", &b"too long"[..])] {
            let file = format!("audio/assets/{name}.opus");
            fs::write(directory.path().join(&file), bytes).unwrap();
            dd.audio_assets.insert(
                name.to_owned(),
                ShippingAudioAsset {
                    file: file.clone(),
                    encoded_size: 1,
                    duration_ms: 123,
                    bundle_offset: None,
                },
            );
            groups.insert(file, [name.to_owned()].into());
        }
        let original = dd.audio_assets.clone();
        let error = bundle_recorded_audio(&mut dd, directory.path(), groups).unwrap_err();
        assert!(format!("{error:#}").contains("cataloged as 1"));
        assert_eq!(dd.audio_assets, original);
        assert_eq!(
            fs::read(directory.path().join("audio/assets/a.opus")).unwrap(),
            b"a"
        );
        assert_eq!(
            fs::read(directory.path().join("audio/assets/z.opus")).unwrap(),
            b"too long"
        );
        // An already completed output is harmless and can be overwritten on
        // retry; the old catalog still references its intact standalone files.
        assert_eq!(
            fs::read_dir(directory.path().join("audio/bundles"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn recorded_audio_bundles_preserve_bytes_aliases_and_large_files() {
        use robin_assets::shipping_datadir::ShippingAudioAsset;
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("audio/assets")).unwrap();
        let inputs = std::collections::BTreeMap::from([
            ("a", vec![1]),
            ("b", vec![2, 3]),
            ("shared", vec![4, 5, 6]),
            ("large", vec![7; AUDIO_BUNDLE_MAX_MEMBER as usize]),
        ]);
        let mut dd = robin_assets::shipping_datadir::ShippingDatadir::default();
        let mut groups = AudioAssetGroups::new();
        for (name, bytes) in &inputs {
            let file = format!("audio/assets/{name}.opus");
            fs::write(directory.path().join(&file), bytes).unwrap();
            dd.audio_assets.insert(
                (*name).to_owned(),
                ShippingAudioAsset {
                    file: file.clone(),
                    encoded_size: bytes.len() as u32,
                    duration_ms: 123,
                    bundle_offset: None,
                },
            );
            groups.insert(
                file,
                if *name == "shared" {
                    ["voice", "effects"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                } else {
                    ["voice"].into_iter().map(str::to_owned).collect()
                },
            );
        }
        let alias = dd.audio_assets["a"].clone();
        dd.audio_assets.insert("alias".into(), alias);
        fs::write(directory.path().join("notes.txt"), b"keep").unwrap();
        bundle_recorded_audio(&mut dd, directory.path(), groups).unwrap();
        for (name, expected) in &inputs {
            let asset = &dd.audio_assets[*name];
            let bytes = fs::read(directory.path().join(&asset.file)).unwrap();
            let start = asset.bundle_offset.unwrap_or(0) as usize;
            assert_eq!(&bytes[start..start + asset.encoded_size as usize], expected);
            assert_eq!(asset.duration_ms, 123);
            assert_eq!(
                directory
                    .path()
                    .join(format!("audio/assets/{name}.opus"))
                    .exists(),
                *name == "large"
            );
        }
        assert_eq!(dd.audio_assets["a"], dd.audio_assets["alias"]);
        assert_eq!(dd.audio_assets["a"].file, dd.audio_assets["b"].file);
        assert_eq!(dd.audio_assets["a"].bundle_offset, Some(0));
        assert_eq!(dd.audio_assets["b"].bundle_offset, Some(1));
        assert!(
            dd.audio_assets["shared"]
                .file
                .starts_with("audio/bundles/shared-")
        );
        assert_eq!(dd.audio_assets["large"].bundle_offset, None);
        assert_eq!(
            fs::read_dir(directory.path().join("audio/bundles"))
                .unwrap()
                .count(),
            2
        );
        assert_eq!(
            fs::read(directory.path().join("notes.txt")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn bundle_member_append_is_bounded_and_preserves_prefix_on_failure() {
        let mut bundle = b"prefix".to_vec();
        append_bundle_member(&b"abc"[..], 3, &mut bundle).unwrap();
        assert_eq!(bundle, b"prefixabc");
        append_bundle_member(&b""[..], 0, &mut bundle).unwrap();
        assert!(append_bundle_member(&b"ab"[..], 3, &mut bundle).is_err());
        assert_eq!(bundle, b"prefixabc");
        // An unbounded source must fail after expected + 1 bytes, not
        // attempt to read the whole stream.
        assert!(append_bundle_member(std::io::repeat(0), 3, &mut bundle).is_err());
        assert_eq!(bundle, b"prefixabc");
        assert!(append_bundle_member(&b"x"[..], 0, &mut bundle).is_err());
        assert_eq!(bundle, b"prefixabc");
    }

    #[test]
    fn bundle_member_append_rolls_back_partial_io_failure() {
        use std::io::Read as _;
        let mut bundle = b"prefix".to_vec();
        let error =
            append_bundle_member((&b"abc"[..]).chain(FailedRead), 4, &mut bundle).unwrap_err();
        assert!(format!("{error:#}").contains("injected read failure"));
        assert_eq!(bundle, b"prefix");
    }

    #[test]
    fn standalone_audio_name_retains_its_full_lowercase_digest() {
        assert_eq!(
            standalone_audio_filename(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad.opus"
        );
    }

    #[test]
    fn boot_trim_uses_actual_catalog_source_when_aliases_collide() {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&1636u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&1600u32.to_le_bytes());
        wav.resize(1644, 0);
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.wav");
        let second = temp.path().join("second.wav");
        fs::write(&first, &wav).unwrap();
        let mut translated = wav.clone();
        translated[44] = 17;
        fs::write(&second, &translated).unwrap();
        let assets = temp.path().join("audio/assets");
        fs::create_dir_all(&assets).unwrap();
        let mut datadir = ShippingDatadir::default();
        for path in [&first, &second] {
            insert_shipping_audio(
                &mut ShippingMission::default(),
                &mut datadir.audio_assets,
                &assets,
                "test",
                "Sounds/Voice.wav",
                path,
                AudioKind::Voice,
                AudioFormat::Opus,
            )
            .unwrap();
        }
        for (name, bytes) in [("en-US", wav.clone()), ("de-DE", translated.clone())] {
            datadir.locales.insert(
                name.into(),
                ShippingLocale {
                    raw: [("sounds/voice.wav".into(), bytes)].into_iter().collect(),
                    ..Default::default()
                },
            );
        }
        assert_eq!(
            catalog_source_bytes(&assets, "sounds/voice.wav")
                .unwrap()
                .unwrap(),
            wav
        );
        let report =
            robin_assets::shipping_boot_trim::trim_browser_locale_audio(&mut datadir, |key| {
                catalog_source_bytes(&assets, key)
            })
            .unwrap();
        assert_eq!(report.removed_files, 1);
        assert_eq!(datadir.locales["de-DE"].raw["sounds/voice.wav"], translated);
        assert!(datadir.locales["en-US"].raw.is_empty());
    }
}

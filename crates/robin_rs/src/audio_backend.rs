//! Native Kira and browser-native Web Audio backends.
//!
//! Implements [`AudioBackend`](crate::sound::AudioBackend) on top of
//! [`kira`]. SFX go through a pool of `StaticSoundData` handles played
//! through one shared track per "channel slot" so the channel-id surface
//! the rest of the game expects (`play_sound` returns an `i32` channel
//! number) keeps working.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use robin_assets::shipping_datadir::ShippingDatadir;
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
use robin_engine::sbfile::SbFile;
use robin_engine::sbfile::SbFileSystem;

#[cfg(any(not(feature = "audio"), not(target_arch = "wasm32")))]
use crate::sound::AudioBackend;
use robin_engine::sound_cache::SampleLoader;

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
mod native;
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
mod resolver;
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
mod sample_cache;
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
pub use native::KiraAudioBackend;

// ─── Stub backend (audio feature disabled) ──────────────────────────
//
// Wasm/no-audio builds get the same type with a no-op impl so callers
// don't need per-cfg plumbing.

#[cfg(all(feature = "audio", target_arch = "wasm32"))]
pub use crate::web_audio_backend::{
    AudioWarmProgress, KiraAudioBackend, clear_mission, preload_active_mission,
    preload_active_mission_in_background, preload_boot, preload_boot_catalog, replace_mission,
};

impl KiraAudioBackend {
    /// Playback belongs to the application's explicit content authority.
    pub fn new_for_application(
        application: &crate::host::ApplicationContext,
        sound_dir: impl Into<PathBuf>,
        num_channels: u32,
    ) -> Result<Self, String> {
        #[cfg(all(feature = "audio", target_arch = "wasm32"))]
        {
            Self::new_with_session(sound_dir, num_channels, application.browser_audio()?)
        }
        #[cfg(not(all(feature = "audio", target_arch = "wasm32")))]
        {
            Self::new_with_files(
                sound_dir,
                num_channels,
                application.preparation_files()?.clone(),
            )
        }
    }
}

#[cfg(not(feature = "audio"))]
mod null;
#[cfg(not(feature = "audio"))]
pub use null::NullAudioBackend as KiraAudioBackend;

// ─── WAV / OGG duration utilities ───
//
// `sound_cache::SampleLoader` consumers want `(bytes, size, duration_ms)`
// to drive the hourglass-expiry pipeline. These pure-bytes parsers don't
// touch the audio backend.

pub fn wav_duration_ms(data: &[u8]) -> Option<u32> {
    if data.len() < 4 {
        return None;
    }
    if &data[0..4] == b"OggS" {
        return ogg_duration_ms(data);
    }
    if data.len() < 44 {
        return None;
    }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }

    let mut offset = 12usize;
    let mut byte_rate: u32 = 0;
    let mut data_size: u32 = 0;

    while data.len().saturating_sub(offset) >= 8 {
        let chunk_id = &data[offset..offset + 4];
        let chunk_size = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().ok()?);

        let chunk_end = offset.checked_add(8)?.checked_add(chunk_size as usize)?;
        // Duration may be read from a header-only data chunk, but metadata
        // and unknown chunks must be present before traversing past them.
        if chunk_id != b"data" && chunk_end > data.len() {
            return None;
        }
        if chunk_id == b"fmt " {
            if chunk_size < 12 {
                return None;
            }
            byte_rate = u32::from_le_bytes(data[offset + 16..offset + 20].try_into().ok()?);
        } else if chunk_id == b"data" {
            data_size = chunk_size;
        }

        offset = chunk_end;
        if !offset.is_multiple_of(2) {
            offset = offset.checked_add(1)?;
        }
    }

    let duration_ms = u64::from(data_size)
        .checked_mul(1000)?
        .checked_div(u64::from(byte_rate))?;
    u32::try_from(duration_ms).ok()
}

pub fn ogg_duration_ms(data: &[u8]) -> Option<u32> {
    if data.len() < 28 || &data[0..4] != b"OggS" {
        return None;
    }
    let page_segments = *data.get(26)? as usize;
    let header_end = 27 + page_segments;
    let body = data.get(header_end..)?;
    if body.len() < 16 || body[0] != 0x01 || &body[1..7] != b"vorbis" {
        return None;
    }
    let sample_rate = u32::from_le_bytes(body[12..16].try_into().ok()?);
    if sample_rate == 0 {
        return None;
    }

    let mut last_granule: u64 = 0;
    let mut i = 0usize;
    while i + 27 <= data.len() {
        if &data[i..i + 4] == b"OggS" {
            let gp = u64::from_le_bytes(data[i + 6..i + 14].try_into().ok()?);
            if gp != u64::MAX {
                last_granule = gp;
            }
            let segs = data[i + 26] as usize;
            if i + 27 + segs > data.len() {
                break;
            }
            let body_len: usize = data[i + 27..i + 27 + segs]
                .iter()
                .map(|&s| s as usize)
                .sum();
            i += 27 + segs + body_len;
        } else {
            i += 1;
        }
    }

    let duration_ms = last_granule
        .checked_mul(1000)?
        .checked_div(sample_rate as u64)?;
    u32::try_from(duration_ms).ok()
}

/// A sample located by [`locate_sample`]: either authoritative metadata
/// from the active shipping datadir (wasm — the encoded bytes stay with
/// Web Audio) or the encoded bytes themselves.
enum LocatedSample {
    /// Only constructed on wasm, where the shipping datadir carries audio
    /// metadata instead of encoded bytes.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Metadata {
        size: u32,
        duration_ms: u32,
    },
    Bytes {
        data: Vec<u8>,
        source_path: PathBuf,
    },
}

/// Authored sound paths try the sound directory first, then Exclamations.
/// Absolute paths never receive a speech-directory fallback.
fn sample_base_paths(base_dir: &Path, file_name: &str) -> (PathBuf, Option<PathBuf>) {
    let normalised = file_name.replace('\\', "/");
    let absolute = Path::new(&normalised).is_absolute();
    let path = if absolute {
        PathBuf::from(&normalised)
    } else {
        base_dir.join(&normalised)
    };
    let speech_path = (!absolute).then(|| base_dir.join("Exclamations").join(&normalised));
    (path, speech_path)
}

/// Try each authored path before its converted Opus sibling.
fn with_opus_fallback(
    (primary, speech): (PathBuf, Option<PathBuf>),
) -> impl Iterator<Item = PathBuf> {
    std::iter::once(primary).chain(speech).flat_map(|path| {
        let opus = path.with_extension("opus");
        [path, opus]
    })
}

/// Resolve a sample name against the loader's candidate paths and read it.
///
/// Candidate order for the explicitly owned playback loader.
fn locate_sample(
    base_dir: &Path,
    file_name: &str,
    files: &SbFileSystem,
    _shipping: Option<&ShippingDatadir>,
) -> Option<LocatedSample> {
    let candidates = sample_base_paths(base_dir, file_name);
    #[cfg(target_arch = "wasm32")]
    if let Some((size, duration_ms)) = _shipping.and_then(|shipping| {
        std::iter::once(&candidates.0)
            .chain(candidates.1.as_ref())
            .find_map(|path| shipping.active_audio_metadata(path))
    }) {
        // Web Audio already owns the decoded buffer. SoundCache needs only
        // authoritative bookkeeping, not another encoded-byte copy.
        return Some(LocatedSample::Metadata { size, duration_ms });
    }
    let (data, source_path) = with_opus_fallback(candidates).find_map(|candidate| {
        files
            .read_all(&candidate.to_string_lossy())
            .ok()
            .map(|data| (data, candidate))
    })?;
    Some(LocatedSample::Bytes { data, source_path })
}

/// Build a sample loader using only the supplied preparation authority.
/// The shipping handle must belong to the same prepared installation/selection
/// as `files`; neither reader nor metadata is resolved from process globals.
/// The caller must not reselect that shipping handle while the loader is live;
/// independently prepared applications use independent shipping handles.
pub fn create_sample_loader_with_files(
    base_dir: PathBuf,
    files: Arc<SbFileSystem>,
    shipping: Option<Arc<ShippingDatadir>>,
) -> Box<SampleLoader> {
    Box::new(move |file_name: &str| {
        tracing::trace!(file_name, "SampleLoader: enter");
        match locate_sample(&base_dir, file_name, &files, shipping.as_deref())? {
            LocatedSample::Metadata { size, duration_ms } => Some((Vec::new(), size, duration_ms)),
            LocatedSample::Bytes { data, source_path } => {
                let size = data.len() as u32;
                let duration_ms = shipping
                    .as_deref()
                    .and_then(|shipping| shipping.active_audio_duration_ms(&source_path))
                    .or_else(|| wav_duration_ms(&data))
                    .or_else(|| {
                        tracing::warn!(path = %source_path.display(), "audio duration unavailable");
                        None
                    })?;
                Some((data, size, duration_ms))
            }
        }
    })
}

#[cfg(test)]
mod tests {
    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    use super::native::*;
    use super::*;
    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    use kira::sound::static_sound::StaticSoundData;
    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    use std::io::Cursor;

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    fn legacy_vorbis_repair_preserves_packets_and_granules() {
        let packets = [
            b"\x01vorbis identification".as_slice(),
            LEGACY_VORBIS_COMMENT,
            b"\x05vorbis setup",
            b"audio packet one",
            b"audio packet two",
        ];
        let mut writer = ogg::PacketWriter::new(Vec::new());
        for (index, packet) in packets.iter().enumerate() {
            let end = if index == packets.len() - 1 {
                ogg::PacketWriteEndInfo::EndStream
            } else {
                ogg::PacketWriteEndInfo::EndPage
            };
            writer
                .write_packet(*packet, 123, end, index as u64 * 1024)
                .unwrap();
        }
        let original = writer.into_inner();
        let repaired = repair_legacy_vorbis_comment(original.clone()).unwrap();
        assert_ne!(repaired.as_ref(), original);
        // PacketReader validates CRCs in the rewritten container as well.
        let mut reader = ogg::PacketReader::new(Cursor::new(&repaired));
        for (index, expected) in packets.iter().enumerate() {
            let packet = reader.read_packet().unwrap().unwrap();
            assert_eq!(packet.stream_serial(), 123);
            assert_eq!(packet.absgp_page(), index as u64 * 1024);
            assert_eq!(packet.last_in_stream(), index == packets.len() - 1);
            if index == 1 {
                assert_eq!(&packet.data[..47], &expected[..47]); // vendor/count
                assert_eq!(&packet.data[47..51], &38u32.to_le_bytes());
                assert_eq!(
                    &packet.data[51..],
                    b"ENCODER=Sonic Foundry OggVorbis Beta 3\x01"
                );
            } else {
                assert_eq!(&packet.data, expected);
            }
        }
        assert!(reader.read_packet().unwrap().is_none());
        assert_eq!(
            repair_legacy_vorbis_comment(repaired.clone())
                .unwrap()
                .as_ref(),
            repaired.as_ref()
        );
    }

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    fn unknown_vorbis_metadata_is_not_rewritten() {
        let mut unknown = LEGACY_VORBIS_COMMENT.to_vec();
        unknown[51] = b'X';
        let mut writer = ogg::PacketWriter::new(Vec::new());
        writer
            .write_packet(unknown, 123, ogg::PacketWriteEndInfo::EndStream, 0)
            .unwrap();
        let original = writer.into_inner();
        assert_eq!(
            repair_legacy_vorbis_comment(original.clone())
                .unwrap()
                .as_ref(),
            original
        );
        let wav = b"RIFF non-Ogg input".to_vec();
        assert_eq!(
            repair_legacy_vorbis_comment(wav.clone()).unwrap().as_ref(),
            wav
        );
    }

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    #[ignore = "requires ROBINHOOD_DATA_DIR pointing to original fullgame data"]
    fn legacy_vorbis_repair_preserves_decoded_original_music() {
        let root = std::env::var("ROBINHOOD_DATA_DIR").expect("set ROBINHOOD_DATA_DIR");
        for name in ["Menu", "Cast_Fight"] {
            let path = ["DATA/Musics", "Data/Musics"]
                .into_iter()
                .flat_map(|directory| {
                    ["wav", "ogg"].map(|extension| {
                        Path::new(&root)
                            .join(directory)
                            .join(format!("{name}.{extension}"))
                    })
                })
                .find(|path| path.is_file())
                .unwrap_or_else(|| panic!("original music fixture {name} is missing under {root}"));
            let bytes = std::fs::read(&path).expect("read original music");
            let repaired = repair_legacy_vorbis_comment(bytes.clone()).unwrap();
            assert_ne!(
                repaired.as_ref(),
                bytes,
                "fixture must contain legacy comment"
            );
            let original = StaticSoundData::from_cursor(Cursor::new(bytes)).unwrap();
            let normalized = StaticSoundData::from_cursor(Cursor::new(repaired)).unwrap();
            assert_eq!(normalized.sample_rate, original.sample_rate);
            assert_eq!(normalized.frames.len(), original.frames.len());
            for (left, right) in original.frames.iter().zip(normalized.frames.iter()) {
                assert_eq!(left.left.to_bits(), right.left.to_bits());
                assert_eq!(left.right.to_bits(), right.right.to_bits());
            }
        }
    }

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    fn audio_preparation_seeks_and_loops_without_a_device() {
        let sample = StaticSoundData {
            sample_rate: 4,
            frames: vec![kira::Frame::ZERO; 8].into(),
            settings: Default::default(),
            slice: None,
        };
        let prepared = prepare_sample(sample, 0.25, true, 255);
        assert_eq!(
            prepared.settings.start_position,
            kira::sound::PlaybackPosition::Seconds(0.5)
        );
        assert!(prepared.settings.loop_region.is_some());
    }

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    fn music_path_falls_back_from_wav_to_ogg() {
        let temp = tempfile::tempdir().unwrap();
        let ogg = temp.path().join("Lincoln_D.ogg");
        std::fs::write(&ogg, []).unwrap();

        let wav = temp.path().join("Lincoln_D.wav");
        let files = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            resolver::resolve_music(&files, wav.to_str().unwrap()).unwrap(),
            ogg
        );
    }

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    fn audio_cursor_reuses_shared_bytes_and_survives_asset_replacement() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let path = Path::new("Data/Sounds/shared-cursor.wav");
        let bytes = one_second_wav();
        assets.install_preloaded_asset(path, bytes.clone()).unwrap();
        let files = SbFileSystem::new(assets.clone());
        let shared = files.read_shared(path.to_str().unwrap()).unwrap();
        let cursor = read_audio_cursor(&files, path).unwrap();
        assert_eq!(cursor.get_ref().as_ptr(), shared.as_ptr());
        assets
            .install_preloaded_asset(path, vec![0; bytes.len()])
            .unwrap();
        drop(shared);
        drop(files);
        drop(assets);
        assert_eq!(cursor.get_ref().as_ref(), bytes);
        assert!(StaticSoundData::from_cursor(cursor).is_ok());
    }

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    fn native_playback_decoders_use_explicit_vfs_without_a_device() {
        for rate in [44_100_u32, 22_050] {
            let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
            let mut bytes = one_second_wav();
            bytes[24..28].copy_from_slice(&rate.to_le_bytes());
            bytes[28..32].copy_from_slice(&(rate * 4).to_le_bytes());
            assets
                .install_preloaded_asset("Data/Sounds/Exclamations/reader.wav", bytes.clone())
                .unwrap();
            assets
                .install_preloaded_asset("Data/Music/reader.ogg", bytes)
                .unwrap();
            let files = SbFileSystem::new(assets);
            let sample =
                resolver::resolve_sample(Path::new("Data/Sounds"), "reader.wav", &files).unwrap();
            let data = load_static_sound(&files, &sample).unwrap();
            assert_eq!(data.sample_rate, rate);
            let music = resolver::resolve_music(&files, "Data/Music/reader.wav").unwrap();
            assert_eq!(music, Path::new("Data/Music/reader.ogg"));
            assert!(load_streaming_sound(&files, &music).is_ok());
            let old_key = sample_cache_key(&files, &sample);
            files.set_locale_paths(Some("other-locale"), None);
            assert_ne!(sample_cache_key(&files, &sample), old_key);
        }
    }

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    fn native_playback_decoders_do_not_reopen_forbidden_absolute_paths() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("sample.wav");
        std::fs::write(&path, one_second_wav()).unwrap();
        let files = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        files.lock_ranked_verifier_primary_path(root.path());
        assert!(load_static_sound(&files, Path::new("sample.wav")).is_ok());
        assert!(load_streaming_sound(&files, Path::new("sample.wav")).is_ok());
        assert!(load_static_sound(&files, &path).is_err());
        assert!(load_streaming_sound(&files, &path).is_err());
        assert!(load_static_sound(&files, Path::new("../sample.wav")).is_err());
        assert!(load_streaming_sound(&files, Path::new("../sample.wav")).is_err());
    }

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    fn audio_cache_identity_changes_with_mounts_but_frozen_readers_stay_pinned() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let path = Path::new("Data/Sounds/replaced.wav");
        assets
            .install_preloaded_asset(path, one_second_wav())
            .unwrap();
        let files = SbFileSystem::new(assets.clone());
        let frozen = files.snapshot();
        let old_key = sample_cache_key(&files, path);
        let mut replacement = one_second_wav();
        replacement[24..28].copy_from_slice(&22_050u32.to_le_bytes());
        replacement[28..32].copy_from_slice(&88_200u32.to_le_bytes());
        assets.install_preloaded_asset(path, replacement).unwrap();
        assert_ne!(sample_cache_key(&files, path), old_key);
        assert_eq!(sample_cache_key(&frozen, path), old_key);
        assert_eq!(load_static_sound(&files, path).unwrap().sample_rate, 22_050);
        assert_eq!(
            load_static_sound(&frozen, path).unwrap().sample_rate,
            44_100
        );
    }

    fn one_second_wav() -> Vec<u8> {
        let sample_rate: u32 = 44_100;
        let channels: u16 = 2;
        let bits_per_sample: u16 = 16;
        let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
        let block_align = channels * bits_per_sample / 8;
        let data_size: u32 = byte_rate;

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.resize(wav.len() + data_size as usize, 0);
        wav
    }

    #[test]
    fn wav_duration_basic() {
        assert_eq!(wav_duration_ms(&one_second_wav()), Some(1000));
    }

    fn wav_with_duration_multiplier(multiplier: u32) -> Vec<u8> {
        let mut bytes = one_second_wav();
        let rate = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) / multiplier;
        bytes[28..32].copy_from_slice(&rate.to_le_bytes());
        bytes
    }

    #[test]
    fn explicit_audio_readers_keep_same_named_samples_independent() {
        let first_assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let second_assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        for (assets, multiplier) in [(&first_assets, 1), (&second_assets, 2)] {
            assets
                .install_preloaded_asset(
                    "Data/Sounds/isolated-audio.wav",
                    wav_with_duration_multiplier(multiplier),
                )
                .unwrap();
        }
        let first = Arc::new(SbFileSystem::new(first_assets.clone()).snapshot());
        let second = Arc::new(SbFileSystem::new(second_assets).snapshot());
        first_assets
            .install_preloaded_asset("Data/Sounds/isolated-audio.wav", b"poisoned".to_vec())
            .unwrap();
        let base = Path::new("Data/Sounds");
        for (files, expected) in [(first, 1000), (second, 2000)] {
            let loader = create_sample_loader_with_files(base.to_owned(), files.clone(), None);
            let (bytes, size, duration) = loader("isolated-audio.wav").unwrap();
            assert_eq!(duration, expected);
            assert_eq!(size as usize, bytes.len());
        }
    }

    #[test]
    fn explicit_audio_candidates_preserve_extension_and_exclamation_order() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        for (name, multiplier) in [
            ("Data/Sounds/order.wav", 1),
            ("Data/Sounds/order.opus", 2),
            ("Data/Sounds/Exclamations/order.wav", 4),
            ("Data/Sounds/fallback.opus", 2),
            ("Data/Sounds/Exclamations/fallback.wav", 4),
            ("Data/Sounds/Exclamations/voice.wav", 4),
        ] {
            assets
                .install_preloaded_asset(name, wav_with_duration_multiplier(multiplier))
                .unwrap();
        }
        let files = Arc::new(SbFileSystem::new(assets).snapshot());
        let base = Path::new("Data/Sounds");
        let loader = create_sample_loader_with_files(base.to_owned(), files.clone(), None);
        for (name, expected) in [
            ("order.wav", 1000),
            ("fallback.wav", 2000),
            ("voice.wav", 4000),
        ] {
            assert_eq!(loader(name).unwrap().2, expected);
        }
    }

    #[test]
    fn explicit_shipping_metadata_precedes_encoded_duration() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        assets
            .install_preloaded_asset("Data/Sounds/metadata.wav", one_second_wav())
            .unwrap();
        let files = Arc::new(SbFileSystem::new(assets).snapshot());
        let mut shipping = ShippingDatadir::default();
        shipping.audio_assets.insert(
            "sounds/metadata.opus".into(),
            robin_assets::shipping_datadir::ShippingAudioAsset {
                file: "audio/metadata.opus".into(),
                encoded_size: 123,
                duration_ms: 2345,
                bundle_offset: None,
            },
        );
        let shipping = Arc::new(shipping);
        let base = Path::new("Data/Sounds");
        let loader =
            create_sample_loader_with_files(base.to_owned(), files.clone(), Some(shipping.clone()));
        let (bytes, size, duration) = loader("metadata.wav").unwrap();
        assert_eq!(duration, 2345);
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert_eq!(bytes, one_second_wav());
            assert_eq!(size as usize, bytes.len());
        }
        #[cfg(target_arch = "wasm32")]
        {
            assert!(bytes.is_empty());
            assert_eq!(size, 123);
            let empty = Arc::new(SbFileSystem::new(Arc::new(
                robin_util::asset_fs::AssetVfs::new(),
            )));
            let metadata_only =
                create_sample_loader_with_files(base.to_owned(), empty, Some(shipping));
            assert_eq!(metadata_only("metadata.wav"), Some((Vec::new(), 123, 2345)));
        }
    }

    #[test]
    fn explicit_audio_reader_does_not_bypass_ranked_path_confinement() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("sample.wav");
        std::fs::write(&path, one_second_wav()).unwrap();
        let files = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        files.lock_ranked_verifier_primary_path(root.path());
        let loader = create_sample_loader_with_files(PathBuf::new(), files.clone(), None);
        assert_eq!(loader("sample.wav").unwrap().2, 1000);
        assert!(loader(path.to_str().unwrap()).is_none());
        assert!(loader("../sample.wav").is_none());
    }

    #[test]
    fn sample_lookup_paths_preserve_authored_and_converted_precedence() {
        let base = Path::new("Data/Sounds");
        let expected = [
            "Data/Sounds/Expressions/voice.wav",
            "Data/Sounds/Expressions/voice.opus",
            "Data/Sounds/Exclamations/Expressions/voice.wav",
            "Data/Sounds/Exclamations/Expressions/voice.opus",
        ]
        .map(PathBuf::from);
        assert_eq!(
            with_opus_fallback(sample_base_paths(base, "Expressions\\\\voice.wav"))
                .collect::<Vec<_>>(),
            expected
        );
        let absolute = std::env::current_dir().unwrap().join("voice.wav");
        assert_eq!(
            with_opus_fallback(sample_base_paths(base, absolute.to_str().unwrap()))
                .collect::<Vec<_>>(),
            [absolute.clone(), absolute.with_extension("opus")]
        );

        for first_available in 0..expected.len() {
            let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
            for (index, path) in expected.iter().enumerate().skip(first_available) {
                assets
                    .install_preloaded_asset(path.to_str().unwrap(), vec![index as u8])
                    .unwrap();
            }
            let files = SbFileSystem::new(assets);
            let Some(LocatedSample::Bytes { data, source_path }) =
                locate_sample(base, "Expressions\\\\voice.wav", &files, None)
            else {
                panic!("an installed candidate must resolve to bytes");
            };
            assert_eq!(source_path, expected[first_available]);
            assert_eq!(data, [first_available as u8]);
            #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
            assert_eq!(
                resolver::resolve_sample(base, "Expressions\\\\voice.wav", &files).unwrap(),
                source_path
            );
        }
    }

    #[test]
    fn wav_duration_invalid() {
        assert_eq!(wav_duration_ms(b"not a wav"), None);
        assert_eq!(wav_duration_ms(&[]), None);
    }

    #[test]
    fn ogg_duration_rejects_overflowing_granule_timestamps() {
        let mut ogg = vec![0u8; 44];
        ogg[..4].copy_from_slice(b"OggS");
        ogg[26] = 1;
        ogg[27] = 16;
        ogg[28] = 1;
        ogg[29..35].copy_from_slice(b"vorbis");
        ogg[40..44].copy_from_slice(&48_000u32.to_le_bytes());
        for (granule, expected) in [
            (48_000u64, Some(1000)),
            (u64::MAX - 1, None),
            (u64::MAX, Some(0)), // unset granules do not advance the timestamp
        ] {
            ogg[6..14].copy_from_slice(&granule.to_le_bytes());
            assert_eq!(ogg_duration_ms(&ogg), expected);
            assert_eq!(wav_duration_ms(&ogg), expected);
        }
    }

    #[test]
    fn wav_duration_rejects_unrepresentable_chunk_cursor_without_panicking() {
        let mut wav = one_second_wav();
        wav[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(wav_duration_ms(&wav), None);
        for size in [0u32, 4, 11] {
            wav[16..20].copy_from_slice(&size.to_le_bytes());
            assert_eq!(wav_duration_ms(&wav), None);
        }
        wav[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        // Unknown oversized chunks cannot fabricate an audio byte rate.
        wav[12..16].copy_from_slice(b"JUNK");
        assert_eq!(wav_duration_ms(&wav), None);
    }

    #[test]
    fn audio_unknown_duration_is_absent_from_playback() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        assets
            .install_preloaded_asset("invalid.wav", b"not an audio file".to_vec())
            .unwrap();
        let files = Arc::new(SbFileSystem::new(assets));
        assert!(
            create_sample_loader_with_files(PathBuf::new(), files, None)("invalid.wav").is_none()
        );
    }

    #[test]
    fn wav_duration_handles_samples_larger_than_u32_milliseconds_product() {
        let byte_rate = 88_200u32;
        let data_size = 10_000_000u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&44_100u32.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());

        assert_eq!(wav_duration_ms(&wav), Some(113_378));
    }
}

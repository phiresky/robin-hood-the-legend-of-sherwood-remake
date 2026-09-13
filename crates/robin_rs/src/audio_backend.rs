//! Audio playback backend selection plus backend-independent sample probing.
//!
//! Exactly one playback backend exists per build configuration and is
//! exported as [`PlatformAudioBackend`]:
//!
//! | configuration                                 | backend             | module                     |
//! |-----------------------------------------------|---------------------|----------------------------|
//! | `all(feature = "audio", not(wasm32))`         | `KiraAudioBackend`  | `audio_backend/native.rs`  |
//! | `all(feature = "audio", wasm32)`              | `WebAudioBackend`   | `crate::web_audio_backend` |
//! | `not(feature = "audio")`                      | `NullAudioBackend`  | `audio_backend/null.rs`    |
//!
//! `NullAudioBackend` is uninhabited: construction fails, so callers keep
//! `Option<PlatformAudioBackend>` = `None` and no backend value is ever
//! queried in audio-disabled builds.
//!
//! The sample loader and the WAV/OGG duration probes below are shared by all
//! configurations (the sound cache needs durations even without playback).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use robin_assets::shipping_datadir::ShippingDatadir;
use robin_engine::sbfile::SbFileSystem;
use robin_engine::sound_cache::SampleLoader;

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
mod native;
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
pub use native::{KiraAudioBackend, PlatformAudioBackend};

#[cfg(all(feature = "audio", target_arch = "wasm32"))]
pub use crate::web_audio_backend::{
    AudioWarmProgress, PlatformAudioBackend, WebAudioBackend, clear_mission,
    preload_active_mission, preload_active_mission_in_background, preload_boot,
    preload_boot_catalog, replace_mission,
};

#[cfg(not(feature = "audio"))]
mod null;
#[cfg(not(feature = "audio"))]
pub use null::{NullAudioBackend, PlatformAudioBackend};

// ─── WAV / OGG duration probes ───
//
// `sound_cache::SampleLoader` consumers want `(bytes, size, duration_ms)`
// to drive the hourglass-expiry pipeline. These pure-bytes parsers don't
// touch the audio backend.

/// Why a sample's duration could not be derived from its encoded bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioProbeError {
    #[error("{len}-byte input is too short for a {format} header")]
    TooShort { format: &'static str, len: usize },
    #[error("input is neither a RIFF/WAVE nor an Ogg container")]
    UnknownContainer,
    #[error("WAV chunk {chunk:?} at offset {offset} overruns the input")]
    ChunkOverrun { chunk: [u8; 4], offset: usize },
    #[error("WAV fmt chunk is {size} bytes; at least 12 are required")]
    FmtChunkTooSmall { size: u32 },
    #[error("WAV has no non-zero byte rate (missing fmt chunk)")]
    MissingByteRate,
    #[error("first Ogg page is not a Vorbis identification header")]
    NotVorbis,
    #[error("Vorbis identification header declares a zero sample rate")]
    ZeroSampleRate,
    #[error("duration does not fit in u32 milliseconds")]
    DurationOverflow,
}

/// Little-endian `u32` from a slice the caller has bounds-checked to 4 bytes.
fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("caller slices exactly 4 bytes"))
}

/// Duration of a RIFF/WAVE sample; Ogg input is forwarded to [`ogg_duration_ms`].
pub fn wav_duration_ms(data: &[u8]) -> Result<u32, AudioProbeError> {
    if data.len() < 4 {
        return Err(AudioProbeError::TooShort {
            format: "audio",
            len: data.len(),
        });
    }
    if &data[0..4] == b"OggS" {
        return ogg_duration_ms(data);
    }
    if data.len() < 44 {
        return Err(AudioProbeError::TooShort {
            format: "WAV",
            len: data.len(),
        });
    }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(AudioProbeError::UnknownContainer);
    }

    let mut offset = 12usize;
    let mut byte_rate: u32 = 0;
    let mut data_size: u32 = 0;

    while data.len().saturating_sub(offset) >= 8 {
        let chunk: [u8; 4] = data[offset..offset + 4]
            .try_into()
            .expect("loop condition guarantees an 8-byte chunk header");
        let chunk_size = le_u32(&data[offset + 4..offset + 8]);
        let overrun = AudioProbeError::ChunkOverrun { chunk, offset };

        let chunk_end = offset
            .checked_add(8)
            .and_then(|header_end| header_end.checked_add(chunk_size as usize))
            .ok_or(overrun.clone())?;
        // Duration may be read from a header-only data chunk, but metadata
        // and unknown chunks must be present before traversing past them.
        if &chunk != b"data" && chunk_end > data.len() {
            return Err(overrun);
        }
        if &chunk == b"fmt " {
            if chunk_size < 12 {
                return Err(AudioProbeError::FmtChunkTooSmall { size: chunk_size });
            }
            byte_rate = le_u32(&data[offset + 16..offset + 20]);
        } else if &chunk == b"data" {
            data_size = chunk_size;
        }

        offset = chunk_end;
        if !offset.is_multiple_of(2) {
            offset = offset.checked_add(1).ok_or(overrun)?;
        }
    }

    if byte_rate == 0 {
        return Err(AudioProbeError::MissingByteRate);
    }
    // u32 * 1000 always fits in u64.
    let duration_ms = u64::from(data_size) * 1000 / u64::from(byte_rate);
    u32::try_from(duration_ms).map_err(|_| AudioProbeError::DurationOverflow)
}

/// Duration of an Ogg Vorbis sample from its last set granule position.
pub fn ogg_duration_ms(data: &[u8]) -> Result<u32, AudioProbeError> {
    if data.len() < 28 {
        return Err(AudioProbeError::TooShort {
            format: "Ogg",
            len: data.len(),
        });
    }
    if &data[0..4] != b"OggS" {
        return Err(AudioProbeError::UnknownContainer);
    }
    let page_segments = data[26] as usize;
    let header_end = 27 + page_segments;
    let body = data
        .get(header_end..)
        .filter(|body| body.len() >= 16)
        .ok_or(AudioProbeError::TooShort {
            format: "Vorbis identification",
            len: data.len(),
        })?;
    if body[0] != 0x01 || &body[1..7] != b"vorbis" {
        return Err(AudioProbeError::NotVorbis);
    }
    let sample_rate = le_u32(&body[12..16]);
    if sample_rate == 0 {
        return Err(AudioProbeError::ZeroSampleRate);
    }

    let mut last_granule: u64 = 0;
    let mut i = 0usize;
    while i + 27 <= data.len() {
        if &data[i..i + 4] == b"OggS" {
            let gp = u64::from_le_bytes(
                data[i + 6..i + 14]
                    .try_into()
                    .expect("loop condition guarantees a 27-byte page header"),
            );
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
        .checked_mul(1000)
        .ok_or(AudioProbeError::DurationOverflow)?
        / u64::from(sample_rate);
    u32::try_from(duration_ms).map_err(|_| AudioProbeError::DurationOverflow)
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
    // TODO: the wasm metadata short-circuit is a shipping-datadir concern, not
    // a backend one; it could move behind a ShippingDatadir method if a native
    // metadata-only path ever appears.
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
                let shipped_duration = shipping
                    .as_deref()
                    .and_then(|shipping| shipping.active_audio_duration_ms(&source_path));
                let duration_ms = match shipped_duration {
                    Some(duration_ms) => duration_ms,
                    None => match wav_duration_ms(&data) {
                        Ok(duration_ms) => duration_ms,
                        Err(error) => {
                            // Same fallback as before: an undeterminable
                            // duration makes the sample unavailable.
                            tracing::warn!(
                                path = %source_path.display(),
                                %error,
                                "audio duration unavailable"
                            );
                            return None;
                        }
                    },
                };
                Some((data, size, duration_ms))
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn one_second_wav() -> Vec<u8> {
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
        assert_eq!(wav_duration_ms(&one_second_wav()), Ok(1000));
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
        files
            .lock_ranked_verifier_primary_path(root.path())
            .expect("confine fixture sample loader");
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
        }
    }

    #[test]
    fn wav_duration_invalid() {
        assert_eq!(
            wav_duration_ms(b"not a wav"),
            Err(AudioProbeError::TooShort {
                format: "WAV",
                len: 9
            })
        );
        assert_eq!(
            wav_duration_ms(&[]),
            Err(AudioProbeError::TooShort {
                format: "audio",
                len: 0
            })
        );
        let mut not_riff = one_second_wav();
        not_riff[0..4].copy_from_slice(b"RIFX");
        assert_eq!(
            wav_duration_ms(&not_riff),
            Err(AudioProbeError::UnknownContainer)
        );
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
            (48_000u64, Ok(1000)),
            (u64::MAX - 1, Err(AudioProbeError::DurationOverflow)),
            (u64::MAX, Ok(0)), // unset granules do not advance the timestamp
        ] {
            ogg[6..14].copy_from_slice(&granule.to_le_bytes());
            assert_eq!(ogg_duration_ms(&ogg), expected);
            assert_eq!(wav_duration_ms(&ogg), expected);
        }
        ogg[29] = b'X';
        assert_eq!(ogg_duration_ms(&ogg), Err(AudioProbeError::NotVorbis));
        ogg[29] = b'v';
        ogg[40..44].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(ogg_duration_ms(&ogg), Err(AudioProbeError::ZeroSampleRate));
    }

    #[test]
    fn wav_duration_rejects_unrepresentable_chunk_cursor_without_panicking() {
        let mut wav = one_second_wav();
        wav[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            wav_duration_ms(&wav),
            Err(AudioProbeError::ChunkOverrun { chunk, offset: 12 }) if &chunk == b"fmt "
        ));
        for size in [0u32, 4, 11] {
            wav[16..20].copy_from_slice(&size.to_le_bytes());
            assert_eq!(
                wav_duration_ms(&wav),
                Err(AudioProbeError::FmtChunkTooSmall { size })
            );
        }
        wav[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        // Unknown oversized chunks cannot fabricate an audio byte rate.
        wav[12..16].copy_from_slice(b"JUNK");
        assert!(matches!(
            wav_duration_ms(&wav),
            Err(AudioProbeError::ChunkOverrun { chunk, offset: 12 }) if &chunk == b"JUNK"
        ));
    }

    #[test]
    fn wav_without_fmt_chunk_reports_missing_byte_rate() {
        let mut wav = one_second_wav();
        // Rename fmt to an unknown (but fully present) chunk.
        wav[12..16].copy_from_slice(b"LIST");
        assert_eq!(wav_duration_ms(&wav), Err(AudioProbeError::MissingByteRate));
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

        assert_eq!(wav_duration_ms(&wav), Ok(113_378));
    }
}

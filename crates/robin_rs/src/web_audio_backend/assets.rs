//! Browser asset fetch/decode/cache owner. Voice lifecycle stays in the parent.
use super::BrowserAudioSession;
use crate::audio_bundle_cache::AudioBundleCache;
use crate::web_audio_state::should_cache_decoded;
use futures::{
    FutureExt as _,
    future::{AbortHandle, Abortable, LocalBoxFuture, Shared},
};
use robin_assets::shipping_datadir::RemoteAudioAsset;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::JsFuture;
use web_sys::AudioBuffer;

const MAX_DECODED_PCM_BYTES: u64 = 96 * 1024 * 1024;

struct CachedBuffer {
    buffer: AudioBuffer,
    bytes: u64,
    last_used: u64,
}

/// Content-addressed buffers survive mission transitions; active voices own
/// their buffer references independently of LRU retention.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct AudioAssets {
    #[serde(skip)]
    buffers: HashMap<String, CachedBuffer>,
    #[serde(skip)]
    cached_bytes: u64,
    #[serde(skip)]
    cache_clock: u64,
    #[serde(skip)]
    bundles: AudioBundleCache<js_sys::ArrayBuffer>,
    #[serde(skip)]
    encoded_loads: HashMap<String, EncodedFuture>,
    // Any joined bundle consumer may request retention of the shared result.
    #[serde(skip)]
    retain_encoded: HashSet<String>,
    #[serde(skip)]
    decode_loads: HashMap<String, DecodeFuture>,
    #[serde(skip)]
    cancellations: HashMap<String, AbortHandle>,
}

impl Drop for AudioAssets {
    fn drop(&mut self) {
        for handle in self.cancellations.values() {
            handle.abort();
        }
    }
}

fn cached_buffer(session: &BrowserAudioSession, key: &str) -> Result<Option<AudioBuffer>, String> {
    session.with_audio(|audio| {
        audio.assets.cache_clock = audio.assets.cache_clock.wrapping_add(1);
        let clock = audio.assets.cache_clock;
        audio.assets.buffers.get_mut(key).map(|cached| {
            cached.last_used = clock;
            cached.buffer.clone()
        })
    })
}

fn cache_buffer(
    session: &BrowserAudioSession,
    key: String,
    buffer: AudioBuffer,
) -> Result<AudioBuffer, String> {
    session.with_audio(|audio| {
        if audio.assets.buffers.contains_key(&key) {
            audio.assets.cache_clock = audio.assets.cache_clock.wrapping_add(1);
            let clock = audio.assets.cache_clock;
            let existing = audio
                .assets
                .buffers
                .get_mut(&key)
                .expect("content-keyed buffer checked above");
            existing.last_used = clock;
            return existing.buffer.clone();
        }
        let bytes = u64::from(buffer.length())
            .saturating_mul(u64::from(buffer.number_of_channels()))
            .saturating_mul(std::mem::size_of::<f32>() as u64);
        if !should_cache_decoded(bytes, MAX_DECODED_PCM_BYTES) {
            tracing::debug!(
                key,
                pcm_bytes = bytes,
                budget_bytes = MAX_DECODED_PCM_BYTES,
                "decoded browser audio exceeds the shared PCM budget; leaving it voice-owned"
            );
            return buffer;
        }
        audio.assets.cache_clock = audio.assets.cache_clock.wrapping_add(1);
        audio.assets.cached_bytes = audio.assets.cached_bytes.saturating_add(bytes);
        audio.assets.buffers.insert(
            key.clone(),
            CachedBuffer {
                buffer: buffer.clone(),
                bytes,
                last_used: audio.assets.cache_clock,
            },
        );
        while audio.assets.cached_bytes > MAX_DECODED_PCM_BYTES && audio.assets.buffers.len() > 1 {
            let Some(victim) = audio
                .assets
                .buffers
                .iter()
                .filter(|(candidate, _)| candidate.as_str() != key)
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(candidate, _)| candidate.clone())
            else {
                break;
            };
            if let Some(removed) = audio.assets.buffers.remove(&victim) {
                audio.assets.cached_bytes = audio.assets.cached_bytes.saturating_sub(removed.bytes);
                tracing::debug!(
                    key = victim,
                    pcm_bytes = removed.bytes,
                    "evicted decoded browser audio under PCM budget"
                );
            }
        }
        buffer
    })
}

type EncodedFuture = Shared<LocalBoxFuture<'static, Result<js_sys::ArrayBuffer, String>>>;

async fn fetch_array_buffer(
    files: &robin_engine::sbfile::SbFileSystem,
    url: &str,
) -> Result<js_sys::ArrayBuffer, String> {
    if let Some(preloaded) = url.strip_prefix("robin-preloaded://") {
        let (_, relative) = preloaded
            .split_once('/')
            .ok_or_else(|| format!("preloaded audio URL has no contained relative path: {url}"))?;
        let bytes = files
            .asset_vfs()
            .read(relative)
            .map_err(|error| format!("read preloaded audio {relative}: {error}"))?;
        return Ok(js_sys::Uint8Array::from(bytes.as_slice()).buffer());
    }
    let window = web_sys::window().ok_or("fetch audio: no window")?;
    let response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|error| format!("fetch {url}: {error:?}"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| format!("fetch {url}: result is not a Response"))?;
    if !response.ok() {
        return Err(format!("fetch {url}: HTTP {}", response.status()));
    }
    JsFuture::from(
        response
            .array_buffer()
            .map_err(|error| format!("fetch {url}: arrayBuffer: {error:?}"))?,
    )
    .await
    .map_err(|error| format!("fetch {url}: read body: {error:?}"))?
    .dyn_into::<js_sys::ArrayBuffer>()
    .map_err(|_| format!("fetch {url}: body is not an ArrayBuffer"))
}

pub(super) fn request_encoded(
    session: &BrowserAudioSession,
    url: &str,
    retain_bundle: bool,
) -> Result<EncodedFuture, String> {
    if retain_bundle
        && let Some(bytes) = session.with_audio(|audio| audio.assets.bundles.get(url).cloned())?
    {
        return Ok(futures::future::ready(Ok(bytes)).boxed_local().shared());
    }
    if let Some(load) = session.with_audio(|audio| {
        if retain_bundle {
            audio.assets.retain_encoded.insert(url.to_owned());
        }
        audio.assets.encoded_loads.get(url).cloned()
    })? {
        return Ok(load);
    }
    let owner = session.downgrade()?;
    let files = session.with_audio(|audio| audio.files.clone())?;
    let url = url.to_owned();
    let future_url = url.clone();
    let cancellation_key = format!("encoded:{url}");
    let future_cancellation_key = cancellation_key.clone();
    let (abort, registration) = AbortHandle::new_pair();
    let future = async move {
        let result = Abortable::new(fetch_array_buffer(&files, &future_url), registration)
            .await
            .unwrap_or_else(|_| Err("browser audio fetch cancelled with session".into()));
        let owner = owner
            .upgrade()
            .ok_or("browser audio session retired during fetch")?;
        let mut audio = owner.borrow_mut();
        if audio.retired {
            return Err("browser audio session retired during fetch".into());
        }
        if audio.assets.retain_encoded.remove(&future_url)
            && let Ok(bytes) = &result
        {
            audio.assets.bundles.insert(
                future_url.clone(),
                u64::from(bytes.byte_length()),
                bytes.clone(),
            );
        }
        audio.assets.encoded_loads.remove(&future_url);
        audio.assets.cancellations.remove(&future_cancellation_key);
        result
    }
    .boxed_local()
    .shared();
    session.with_audio(|audio| {
        audio.assets.encoded_loads.insert(url, future.clone());
        audio.assets.cancellations.insert(cancellation_key, abort);
    })?;
    // Keep the shared operation driven even if a blocking warm plan is
    // cancelled after another item fails. Later playback can still join the
    // exact request, and the future always gets to remove its map entry.
    let driver = future.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = driver.await;
    });
    Ok(future)
}

async fn asset_encoded_bytes(
    encoded: EncodedFuture,
    asset: &RemoteAudioAsset,
) -> Result<js_sys::ArrayBuffer, String> {
    let encoded = encoded.await?;
    match asset.bundle_offset {
        None => Ok(encoded),
        Some(offset) => {
            let end = offset
                .checked_add(asset.encoded_size)
                .ok_or_else(|| format!("bundle slice overflow in {}", asset.url))?;
            if end > encoded.byte_length() {
                return Err(format!(
                    "bundle {} is {} bytes; asset wants {offset}..{end}",
                    asset.url,
                    encoded.byte_length()
                ));
            }
            Ok(encoded.slice_with_end(offset, end))
        }
    }
}

async fn fetch_and_decode(
    context: web_sys::AudioContext,
    encoded: EncodedFuture,
    asset: &RemoteAudioAsset,
) -> Result<AudioBuffer, String> {
    let encoded = asset_encoded_bytes(encoded, asset).await?;
    let promise = context
        .decode_audio_data(&encoded)
        .map_err(|error| format!("decode {}: {error:?}", asset.url))?;
    JsFuture::from(promise)
        .await
        .map_err(|error| format!("decode {}: {error:?}", asset.url))?
        .dyn_into::<AudioBuffer>()
        .map_err(|_| format!("decode {}: result is not AudioBuffer", asset.url))
}

pub(super) fn buffer_key(asset: &RemoteAudioAsset) -> String {
    match asset.bundle_offset {
        None => asset.url.clone(),
        Some(offset) => format!("{}#{offset}", asset.url),
    }
}

type DecodeFuture = Shared<LocalBoxFuture<'static, Result<AudioBuffer, String>>>;

pub(super) enum DecodedRequest {
    Ready(AudioBuffer),
    Pending(DecodeFuture),
}

pub(super) fn request_decoded(
    session: &BrowserAudioSession,
    asset: RemoteAudioAsset,
) -> Result<DecodedRequest, String> {
    let key = buffer_key(&asset);
    if let Some(buffer) = cached_buffer(session, &key)? {
        return Ok(DecodedRequest::Ready(buffer));
    }
    if let Some(load) = session.with_audio(|audio| audio.assets.decode_loads.get(&key).cloned())? {
        return Ok(DecodedRequest::Pending(load));
    }
    let future_key = key.clone();
    let owner = session.downgrade()?;
    let context = session.with_audio(|audio| audio.context.clone())?;
    let encoded = request_encoded(session, &asset.url, asset.bundle_offset.is_some())?;
    let cancellation_key = format!("decoded:{key}");
    let future_cancellation_key = cancellation_key.clone();
    let (abort, registration) = AbortHandle::new_pair();
    let future = async move {
        let decoded = Abortable::new(fetch_and_decode(context, encoded, &asset), registration)
            .await
            .unwrap_or_else(|_| Err("browser audio decode cancelled with session".into()));
        let inner = owner
            .upgrade()
            .ok_or("browser audio session retired during decode")?;
        let session = BrowserAudioSession { inner: Some(inner) };
        let result = match decoded {
            Ok(buffer) => cache_buffer(&session, future_key.clone(), buffer),
            Err(error) => Err(error),
        };
        session.with_audio(|audio| {
            audio.assets.decode_loads.remove(&future_key);
            audio.assets.cancellations.remove(&future_cancellation_key);
        })?;
        result
    }
    .boxed_local()
    .shared();
    session.with_audio(|audio| {
        audio.assets.decode_loads.insert(key, future.clone());
        audio.assets.cancellations.insert(cancellation_key, abort);
    })?;
    let driver = future.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = driver.await;
    });
    Ok(DecodedRequest::Pending(future))
}

pub(super) fn resolve_asset(
    session: &BrowserAudioSession,
    path: &str,
) -> Result<RemoteAudioAsset, String> {
    let datadir = session.with_audio(|audio| audio.catalog.clone())?;
    datadir
        .remote_audio_asset(Path::new(path))
        .ok_or_else(|| format!("browser audio catalog has no entry for {path}"))
}

#[cfg(test)]
mod browser_ownership_tests {
    use super::*;
    use robin_assets::shipping_datadir::{ShippingAudioAsset, ShippingDatadir};
    use robin_engine::sbfile::SbFileSystem;
    use std::sync::Arc;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn wav(sample: i16) -> Vec<u8> {
        let pcm: Vec<u8> = (0..800).flat_map(|_| sample.to_le_bytes()).collect();
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + pcm.len() as u32).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(16000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((pcm.len() as u32).to_le_bytes());
        bytes.extend(pcm);
        bytes
    }

    fn session(bytes: Vec<u8>) -> BrowserAudioSession {
        let vfs = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let size = bytes.len() as u32;
        vfs.install_preloaded_asset("audio/tone.wav", bytes)
            .unwrap();
        let mut catalog = ShippingDatadir::default();
        catalog.set_remote_base_url("robin-preloaded://same-catalog".into());
        catalog.audio_assets.insert(
            "sounds/tone.opus".into(),
            ShippingAudioAsset {
                file: "audio/tone.wav".into(),
                encoded_size: size,
                duration_ms: 100,
                bundle_offset: Some(0),
            },
        );
        BrowserAudioSession::new(Arc::new(SbFileSystem::new(vfs)), Arc::new(catalog)).unwrap()
    }

    async fn decode(session: &BrowserAudioSession) -> AudioBuffer {
        match request_decoded(
            session,
            resolve_asset(session, "Data/Sounds/tone.wav").unwrap(),
        )
        .unwrap()
        {
            DecodedRequest::Ready(buffer) => buffer,
            DecodedRequest::Pending(load) => load.await.unwrap(),
        }
    }

    #[wasm_bindgen_test]
    async fn encoded_budget_eviction_preserves_shared_results_and_oversized_decode() {
        let session = session(wav(8192));
        let asset = resolve_asset(&session, "Data/Sounds/tone.wav").unwrap();
        session
            .with_audio(|audio| {
                audio.assets.bundles = AudioBundleCache::new(u64::from(asset.encoded_size));
            })
            .unwrap();
        let first = request_encoded(&session, &asset.url, false).unwrap();
        let joined = request_encoded(&session, &asset.url, true).unwrap();
        let bytes = first.await.unwrap();
        session
            .with_audio(|audio| {
                assert!(audio.assets.bundles.get(&asset.url).is_some());
                audio.assets.bundles.insert(
                    "replacement".into(),
                    u64::from(bytes.byte_length()),
                    bytes.clone(),
                );
                assert!(audio.assets.bundles.get(&asset.url).is_none());
            })
            .unwrap();
        assert_eq!(joined.await.unwrap().byte_length(), bytes.byte_length());
        assert_eq!(js_sys::Uint8Array::new(&bytes).to_vec(), wav(8192));
        // Cache retention is optional even when the caller requests a bundle.
        // A tiny budget must not prevent fetching, slicing, or decoding it.
        session
            .with_audio(|audio| audio.assets.bundles = AudioBundleCache::new(1))
            .unwrap();
        assert!(decode(&session).await.get_channel_data(0).unwrap()[100] > 0.2);
        assert!(
            session
                .with_audio(|audio| audio.assets.bundles.is_empty()
                    && audio.assets.encoded_loads.is_empty()
                    && audio.assets.decode_loads.is_empty())
                .unwrap()
        );
    }

    #[wasm_bindgen_test]
    async fn same_url_catalogs_have_isolated_preloaded_bytes_and_decoded_buffers() {
        let first = session(wav(8192));
        let second = session(wav(-8192));
        let first_asset = resolve_asset(&first, "Data/Sounds/tone.wav").unwrap();
        let second_asset = resolve_asset(&second, "Data/Sounds/tone.wav").unwrap();
        assert_eq!(first_asset.url, second_asset.url);
        let a = decode(&first).await;
        let b = decode(&second).await;
        assert!(a.get_channel_data(0).unwrap()[100] > 0.2);
        assert!(b.get_channel_data(0).unwrap()[100] < -0.2);
        assert!(matches!(
            request_decoded(&first, first_asset).unwrap(),
            DecodedRequest::Ready(_)
        ));
        first.retire();
        assert!(request_decoded(&first, second_asset.clone()).is_err());
        assert!(matches!(
            request_decoded(&second, second_asset).unwrap(),
            DecodedRequest::Ready(_)
        ));
    }

    #[wasm_bindgen_test]
    async fn retirement_cancels_pending_loads_without_populating_replacement_catalog() {
        let old = session(wav(8192));
        let asset = resolve_asset(&old, "Data/Sounds/tone.wav").unwrap();
        let encoded = request_encoded(&old, &asset.url, true).unwrap();
        let DecodedRequest::Pending(decoded) = request_decoded(&old, asset.clone()).unwrap() else {
            panic!("fresh catalog unexpectedly cached");
        };
        old.retire();
        let replacement = session(wav(-8192));
        assert!(encoded.await.is_err());
        assert!(decoded.await.is_err());
        assert!(
            replacement
                .with_audio(|audio| audio.assets.buffers.is_empty()
                    && audio.assets.bundles.is_empty()
                    && audio.assets.encoded_loads.is_empty()
                    && audio.assets.decode_loads.is_empty())
                .unwrap()
        );
        assert!(decode(&replacement).await.get_channel_data(0).unwrap()[100] < -0.2);
    }

    #[wasm_bindgen_test]
    async fn failures_remove_pending_entries_and_dropping_owner_cancels_driver() {
        let bad = session(vec![1, 2, 3]);
        let missing = request_encoded(
            &bad,
            "robin-preloaded://same-catalog/audio/missing.wav",
            true,
        )
        .unwrap();
        assert!(missing.await.is_err());
        assert!(
            bad.with_audio(|audio| audio.assets.encoded_loads.is_empty()
                && audio.assets.retain_encoded.is_empty()
                && audio.assets.cancellations.is_empty())
                .unwrap()
        );
        let asset = resolve_asset(&bad, "Data/Sounds/tone.wav").unwrap();
        let DecodedRequest::Pending(load) = request_decoded(&bad, asset).unwrap() else {
            panic!("invalid audio unexpectedly cached");
        };
        assert!(load.await.is_err());
        assert!(
            bad.with_audio(|audio| audio.assets.decode_loads.is_empty()
                && audio.assets.encoded_loads.is_empty()
                && audio.assets.retain_encoded.is_empty()
                && audio.assets.cancellations.is_empty())
                .unwrap()
        );
        let old = session(wav(8192));
        let weak = old.downgrade().unwrap();
        let load =
            request_encoded(&old, "robin-preloaded://same-catalog/audio/tone.wav", true).unwrap();
        drop(old);
        assert!(weak.upgrade().is_none());
        assert!(load.await.is_err());
    }
}

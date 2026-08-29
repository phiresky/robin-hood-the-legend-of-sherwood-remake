//! Browser-native playback; encoded audio and decoded PCM remain browser-owned.

use crate::sound::AudioBackend;
use crate::web_audio_state::{
    CompletionDecision, ContentDedup, PendingPlayback, PlaybackGeneration, PlaybackKind,
    ProgressCounter, RequestIds, WarmPriority, completion_decision, should_cache_decoded,
    should_decode_during_warmup, warm_priority,
};
use futures::{FutureExt as _, StreamExt as _, future::LocalBoxFuture, future::Shared};
use robin_assets::shipping_datadir::RemoteAudioAsset;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
};
use wasm_bindgen::{JsCast as _, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState, GainNode, StereoPannerNode,
};

const MAX_DECODED_PCM_BYTES: u64 = 96 * 1024 * 1024;
const AUDIO_IO_CONCURRENCY: usize = 3;

struct CachedBuffer {
    buffer: AudioBuffer,
    bytes: u64,
    last_used: u64,
}

struct BrowserAudio {
    context: AudioContext,
    /// Content-keyed shared PCM. Mission transitions retain this cache; an
    /// LRU budget, rather than an arbitrary lifecycle boundary, limits it.
    buffers: HashMap<String, CachedBuffer>,
    cached_bytes: u64,
    cache_clock: u64,
    generation: PlaybackGeneration,
    backends: Vec<Weak<RefCell<BackendState>>>,
}

thread_local! {
    static AUDIO: RefCell<Option<BrowserAudio>> = const { RefCell::new(None) };
}

fn with_audio<R>(f: impl FnOnce(&mut BrowserAudio) -> R) -> Result<R, String> {
    AUDIO.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let context =
                AudioContext::new().map_err(|error| format!("create AudioContext: {error:?}"))?;
            install_autoplay_unlock(&context)?;
            *slot = Some(BrowserAudio {
                context,
                buffers: HashMap::new(),
                cached_bytes: 0,
                cache_clock: 0,
                generation: PlaybackGeneration::default(),
                backends: Vec::new(),
            });
        }
        Ok(f(slot.as_mut().expect("browser audio initialized above")))
    })
}

fn cached_buffer(key: &str) -> Result<Option<AudioBuffer>, String> {
    with_audio(|audio| {
        audio.cache_clock = audio.cache_clock.wrapping_add(1);
        let clock = audio.cache_clock;
        audio.buffers.get_mut(key).map(|cached| {
            cached.last_used = clock;
            cached.buffer.clone()
        })
    })
}

fn cache_buffer(key: String, buffer: AudioBuffer) -> Result<AudioBuffer, String> {
    with_audio(|audio| {
        if audio.buffers.contains_key(&key) {
            audio.cache_clock = audio.cache_clock.wrapping_add(1);
            let clock = audio.cache_clock;
            let existing = audio
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
        audio.cache_clock = audio.cache_clock.wrapping_add(1);
        audio.cached_bytes = audio.cached_bytes.saturating_add(bytes);
        audio.buffers.insert(
            key.clone(),
            CachedBuffer {
                buffer: buffer.clone(),
                bytes,
                last_used: audio.cache_clock,
            },
        );
        while audio.cached_bytes > MAX_DECODED_PCM_BYTES && audio.buffers.len() > 1 {
            let Some(victim) = audio
                .buffers
                .iter()
                .filter(|(candidate, _)| candidate.as_str() != key)
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(candidate, _)| candidate.clone())
            else {
                break;
            };
            if let Some(removed) = audio.buffers.remove(&victim) {
                audio.cached_bytes = audio.cached_bytes.saturating_sub(removed.bytes);
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

thread_local! {
    /// Encoded bytes of downloaded logical bundles. Bundles are only a few
    /// MiB and let every member decode without another request. Standalone
    /// music/ambience bytes are left to the browser HTTP cache.
    static BUNDLE_CACHE: RefCell<HashMap<String, js_sys::ArrayBuffer>> =
        RefCell::new(HashMap::new());
}

type EncodedFuture = Shared<LocalBoxFuture<'static, Result<js_sys::ArrayBuffer, String>>>;

thread_local! {
    /// In-flight URL fetches shared by warmup, lazy playback and background
    /// prefetch. This closes the previous race where each path could issue a
    /// duplicate fetch for the same logical bundle.
    static ENCODED_LOADS: RefCell<HashMap<String, EncodedFuture>> =
        RefCell::new(HashMap::new());
}

async fn fetch_array_buffer(url: &str) -> Result<js_sys::ArrayBuffer, String> {
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

fn request_encoded(url: &str, retain_bundle: bool) -> EncodedFuture {
    if retain_bundle
        && let Some(bytes) = BUNDLE_CACHE.with(|cache| cache.borrow().get(url).cloned())
    {
        return futures::future::ready(Ok(bytes)).boxed_local().shared();
    }
    if let Some(load) = ENCODED_LOADS.with(|loads| loads.borrow().get(url).cloned()) {
        return load;
    }
    let url = url.to_owned();
    let future_url = url.clone();
    let future = async move {
        let result = fetch_array_buffer(&future_url).await;
        if retain_bundle && let Ok(bytes) = &result {
            BUNDLE_CACHE.with(|cache| {
                cache
                    .borrow_mut()
                    .entry(future_url.clone())
                    .or_insert_with(|| bytes.clone());
            });
        }
        ENCODED_LOADS.with(|loads| {
            loads.borrow_mut().remove(&future_url);
        });
        result
    }
    .boxed_local()
    .shared();
    ENCODED_LOADS.with(|loads| {
        loads.borrow_mut().insert(url, future.clone());
    });
    // Keep the shared operation driven even if a blocking warm plan is
    // cancelled after another item fails. Later playback can still join the
    // exact request, and the future always gets to remove its map entry.
    let driver = future.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = driver.await;
    });
    future
}

async fn asset_encoded_bytes(asset: &RemoteAudioAsset) -> Result<js_sys::ArrayBuffer, String> {
    let encoded = request_encoded(&asset.url, asset.bundle_offset.is_some()).await?;
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

async fn fetch_and_decode(asset: &RemoteAudioAsset) -> Result<AudioBuffer, String> {
    let context = with_audio(|audio| audio.context.clone())?;
    let encoded = asset_encoded_bytes(asset).await?;
    let promise = context
        .decode_audio_data(&encoded)
        .map_err(|error| format!("decode {}: {error:?}", asset.url))?;
    JsFuture::from(promise)
        .await
        .map_err(|error| format!("decode {}: {error:?}", asset.url))?
        .dyn_into::<AudioBuffer>()
        .map_err(|_| format!("decode {}: result is not AudioBuffer", asset.url))
}

fn buffer_key(asset: &RemoteAudioAsset) -> String {
    match asset.bundle_offset {
        None => asset.url.clone(),
        Some(offset) => format!("{}#{offset}", asset.url),
    }
}

type DecodeFuture = Shared<LocalBoxFuture<'static, Result<AudioBuffer, String>>>;

thread_local! {
    static DECODE_LOADS: RefCell<HashMap<String, DecodeFuture>> =
        RefCell::new(HashMap::new());
}

enum DecodedRequest {
    Ready(AudioBuffer),
    Pending(DecodeFuture),
}

fn request_decoded(asset: RemoteAudioAsset) -> Result<DecodedRequest, String> {
    let key = buffer_key(&asset);
    if let Some(buffer) = cached_buffer(&key)? {
        return Ok(DecodedRequest::Ready(buffer));
    }
    if let Some(load) = DECODE_LOADS.with(|loads| loads.borrow().get(&key).cloned()) {
        return Ok(DecodedRequest::Pending(load));
    }
    let future_key = key.clone();
    let future = async move {
        let result = match fetch_and_decode(&asset).await {
            Ok(buffer) => cache_buffer(future_key.clone(), buffer),
            Err(error) => Err(error),
        };
        DECODE_LOADS.with(|loads| {
            loads.borrow_mut().remove(&future_key);
        });
        result
    }
    .boxed_local()
    .shared();
    DECODE_LOADS.with(|loads| {
        loads.borrow_mut().insert(key, future.clone());
    });
    let driver = future.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = driver.await;
    });
    Ok(DecodedRequest::Pending(future))
}

fn resolve_asset(path: &str) -> Result<RemoteAudioAsset, String> {
    let datadir = robin_assets::shipping_datadir::global()
        .ok_or_else(|| format!("browser audio catalog unavailable while resolving {path}"))?;
    datadir
        .remote_audio_asset(Path::new(path))
        .ok_or_else(|| format!("browser audio catalog has no entry for {path}"))
}

fn now_ms() -> u64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now().max(0.0) as u64)
        .unwrap_or_else(|| js_sys::Date::now().max(0.0) as u64)
}

fn request_context_resume(context: &AudioContext, reason: &'static str) {
    if context.state() == AudioContextState::Running {
        return;
    }
    match context.resume() {
        Ok(promise) => wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = JsFuture::from(promise).await {
                tracing::debug!(?error, reason, "AudioContext resume was rejected");
            }
        }),
        Err(error) => tracing::debug!(?error, reason, "AudioContext resume call failed"),
    }
}

thread_local! {
    static AUTOPLAY_UNLOCK_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

fn install_autoplay_unlock(context: &AudioContext) -> Result<(), String> {
    if AUTOPLAY_UNLOCK_INSTALLED.with(|installed| installed.get()) {
        return Ok(());
    }
    let window = web_sys::window().ok_or("install audio autoplay unlock: no window")?;
    for event_name in ["pointerdown", "touchstart", "keydown"] {
        let context = context.clone();
        let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            request_context_resume(&context, "user gesture");
        });
        window
            .add_event_listener_with_callback(event_name, callback.as_ref().unchecked_ref())
            .map_err(|error| format!("install {event_name} audio unlock: {error:?}"))?;
        // The listeners intentionally live for the page lifetime. Browsers
        // may suspend an AudioContext again after backgrounding the tab.
        callback.forget();
    }
    AUTOPLAY_UNLOCK_INSTALLED.with(|installed| installed.set(true));
    Ok(())
}

#[derive(Clone)]
enum WarmWork {
    Encoded { url: String, retain_bundle: bool },
    Decoded(RemoteAudioAsset),
}

#[derive(Clone)]
struct WarmItem {
    label: String,
    priority: WarmPriority,
    work: WarmWork,
}

fn build_warm_plan(keys: Vec<String>, boot: bool) -> Result<Vec<WarmItem>, String> {
    let mut plan = Vec::new();
    let mut encoded_urls = ContentDedup::default();
    let mut decoded_keys = ContentDedup::default();
    let mut decoded_urls = HashSet::new();
    for path in keys {
        let asset = resolve_asset(&path)?;
        let priority = warm_priority(&path, &asset.url);
        if encoded_urls.claim(asset.url.clone()) {
            plan.push(WarmItem {
                label: asset.url.clone(),
                priority,
                work: WarmWork::Encoded {
                    url: asset.url.clone(),
                    retain_bundle: asset.bundle_offset.is_some(),
                },
            });
        }
        let key = buffer_key(&asset);
        if should_decode_during_warmup(priority, boot) && decoded_keys.claim(key) {
            decoded_urls.insert(asset.url.clone());
            plan.push(WarmItem {
                label: path,
                priority,
                work: WarmWork::Decoded(asset),
            });
        }
    }
    // A decode already includes its encoded fetch. Remove the separate URL
    // step even when a common member encountered earlier happened to share
    // that bundle, avoiding redundant HTTP-cache reads and inflated progress.
    plan.retain(|item| {
        !matches!(
            &item.work,
            WarmWork::Encoded { url, .. } if decoded_urls.contains(url)
        )
    });
    plan.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.label.cmp(&right.label))
    });
    Ok(plan)
}

pub struct AudioWarmProgress<'a> {
    pub completed: usize,
    pub total: usize,
    pub file: Option<&'a str>,
}

async fn run_warm_plan<F>(plan: Vec<WarmItem>, mut progress: F) -> Result<(), String>
where
    F: FnMut(AudioWarmProgress<'_>),
{
    let mut progress_counter = ProgressCounter::new(plan.len());
    progress(AudioWarmProgress {
        completed: 0,
        total: progress_counter.total(),
        file: None,
    });
    // Present the blocking mission-audio phase before its first network or
    // decode step; otherwise the loading screen would remain on "data ready"
    // until an arbitrarily slow first asset completed.
    crate::window::yield_to_runtime().await;
    let mut work = futures::stream::iter(plan.into_iter().map(|item| async move {
        let result = match item.work {
            WarmWork::Encoded { url, retain_bundle } => {
                request_encoded(&url, retain_bundle).await.map(|_| ())
            }
            WarmWork::Decoded(asset) => match request_decoded(asset) {
                Ok(DecodedRequest::Ready(_)) => Ok(()),
                Ok(DecodedRequest::Pending(load)) => load.await.map(|_| ()),
                Err(error) => Err(error),
            },
        };
        (item.label, result)
    }))
    .buffer_unordered(AUDIO_IO_CONCURRENCY);
    while let Some((label, result)) = work.next().await {
        result.map_err(|error| format!("warm browser audio {label}: {error}"))?;
        progress(AudioWarmProgress {
            completed: progress_counter.advance(),
            total: progress_counter.total(),
            file: Some(&label),
        });
        crate::window::yield_to_runtime().await;
    }
    Ok(())
}

/// Decode the deliberately small menu set before the application creates its
/// first audio backend. This is the browser boot boundary, not a whole-catalog
/// PCM preload.
pub async fn preload_boot_catalog() -> Result<(), String> {
    let datadir = robin_assets::shipping_datadir::global()
        .ok_or("preload browser boot audio: shipping catalog unavailable")?;
    let plan = build_warm_plan(datadir.boot_audio_keys(), true)?;
    if plan.is_empty() && datadir.audio_catalog_keys().next().is_some() {
        tracing::warn!(
            "browser audio catalog has no boot membership index; regenerate the shipping datadir"
        );
    }
    tracing::info!(items = plan.len(), "warming browser boot/menu audio");
    run_warm_plan(plan, |_| {}).await
}

/// Preload active-mission critical audio while its loading screen is visible.
/// Common short SFX are fetched as their compact logical bundle but remain
/// encoded; dialogue, actor voices, music and long standalone ambience decode.
pub async fn preload_active_mission<F>(progress: F) -> Result<(), String>
where
    F: FnMut(AudioWarmProgress<'_>),
{
    let datadir = robin_assets::shipping_datadir::global()
        .ok_or("preload active mission audio: shipping catalog unavailable")?;
    let mission = datadir
        .active_mission_name()
        .ok_or("preload active mission audio: no active mission")?;
    let plan = build_warm_plan(datadir.active_audio_keys(), false)?;
    if plan.is_empty() && datadir.audio_catalog_keys().next().is_some() {
        tracing::warn!(
            mission,
            "browser audio catalog has no active-mission membership index; regenerate the shipping datadir"
        );
    }
    let active_urls = warm_plan_urls(&plan);
    exclude_catalog_prefetch(&active_urls);
    tracing::info!(
        mission,
        items = plan.len(),
        "warming active mission browser audio"
    );
    run_warm_plan(plan, progress).await?;
    // Only after the active set is ready may low-priority future-mission
    // traffic use the audio workers.
    start_catalog_prefetch(&active_urls);
    Ok(())
}

/// Compatibility entry point for old host-preload callers. Encoded bytes no
/// longer cross into wasm, but the requested catalog entry is genuinely
/// fetched and decoded.
pub async fn preload_boot(path: &str, _bytes: &[u8]) -> Result<(), String> {
    match request_decoded(resolve_asset(path)?)? {
        DecodedRequest::Ready(_) => Ok(()),
        DecodedRequest::Pending(load) => load.await.map(|_| ()),
    }
}

/// Advance the playback generation without discarding content-addressed PCM.
/// Every registered backend cancels old voices and reservations exactly; an
/// in-flight decode may still populate the shared cache, but its stale request
/// id/generation can never start playback in the new mission.
pub fn clear_mission() -> Result<(), String> {
    let (generation, backends) = with_audio(|audio| {
        let generation = audio.generation.advance();
        audio.backends.retain(|backend| backend.strong_count() != 0);
        (generation, audio.backends.clone())
    })?;
    for backend in backends {
        if let Some(backend) = backend.upgrade() {
            backend.borrow_mut().advance_generation(generation);
        }
    }
    Ok(())
}

/// Compatibility adapter for the former embedded mission-audio path.
pub async fn replace_mission(entries: Vec<(String, &[u8])>) -> Result<(), String> {
    clear_mission()?;
    let keys = entries.into_iter().map(|(path, _)| path).collect();
    run_warm_plan(build_warm_plan(keys, false)?, |_| {}).await
}

thread_local! {
    static PREFETCH_STARTED: Cell<bool> = const { Cell::new(false) };
    static PREFETCH_QUEUE: RefCell<VecDeque<(String, bool)>> =
        const { RefCell::new(VecDeque::new()) };
}

/// Low-priority encoded-only catalog prefetch. It starts after the first
/// active mission's blocking critical warmup, never as a side effect of the
/// first requested sound.
fn start_catalog_prefetch(exclude: &HashSet<String>) {
    if PREFETCH_STARTED.with(|started| started.replace(true)) {
        return;
    }
    let Some(datadir) = robin_assets::shipping_datadir::global() else {
        return;
    };
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for key in datadir.audio_catalog_keys() {
        let Some(asset) = datadir.remote_audio_asset(Path::new(key)) else {
            continue;
        };
        if !exclude.contains(&asset.url) && seen.insert(asset.url.clone()) {
            ordered.push((asset.url, asset.bundle_offset.is_some()));
        }
    }
    tracing::info!(
        files = ordered.len(),
        workers = AUDIO_IO_CONCURRENCY,
        "background encoded-audio prefetch started"
    );
    PREFETCH_QUEUE.with(|queue| queue.borrow_mut().extend(ordered));
    for _ in 0..AUDIO_IO_CONCURRENCY {
        wasm_bindgen_futures::spawn_local(async {
            loop {
                let Some((url, retain_bundle)) =
                    PREFETCH_QUEUE.with(|queue| queue.borrow_mut().pop_front())
                else {
                    break;
                };
                if let Err(error) = request_encoded(&url, retain_bundle).await {
                    tracing::debug!(
                        url,
                        error,
                        "background audio prefetch miss; lazy playback may retry"
                    );
                }
            }
        });
    }
}

fn warm_plan_urls(plan: &[WarmItem]) -> HashSet<String> {
    plan.iter()
        .map(|item| match &item.work {
            WarmWork::Encoded { url, .. } => url.clone(),
            WarmWork::Decoded(asset) => asset.url.clone(),
        })
        .collect()
}

/// Active warmup owns these URLs at mission priority. Remove not-yet-started
/// background duplicates; already in-flight requests are joined by the
/// content-keyed shared futures.
fn exclude_catalog_prefetch(urls: &HashSet<String>) {
    if urls.is_empty() {
        return;
    }
    PREFETCH_QUEUE.with(|queue| {
        queue
            .borrow_mut()
            .retain(|(url, _)| !urls.contains(url.as_str()));
    });
}

struct Voice {
    source: AudioBufferSourceNode,
    gain: GainNode,
    panner: StereoPannerNode,
    buffer: AudioBuffer,
    looping: bool,
    paused: bool,
    offset: f64,
    started_at: f64,
    volume: f32,
    pan: f32,
}

impl Voice {
    fn position(&self, now: f64) -> f64 {
        let duration = self.buffer.duration();
        let position = if self.paused {
            self.offset
        } else {
            self.offset + (now - self.started_at).max(0.0)
        };
        if self.looping && duration > 0.0 {
            position.rem_euclid(duration)
        } else {
            position
        }
    }

    fn playing(&self, now: f64) -> bool {
        self.paused || self.looping || self.position(now) < self.buffer.duration()
    }

    fn stop(&self) {
        let _ = self.source.stop();
    }
}

fn make_voice(
    context: &AudioContext,
    buffer: AudioBuffer,
    looping: bool,
    offset: f64,
    volume: f32,
    pan: f32,
) -> Result<Voice, String> {
    let source =
        AudioBufferSourceNode::new(context).map_err(|error| format!("create source: {error:?}"))?;
    let gain = GainNode::new(context).map_err(|error| format!("create gain: {error:?}"))?;
    let panner =
        StereoPannerNode::new(context).map_err(|error| format!("create panner: {error:?}"))?;
    source.set_buffer(Some(&buffer));
    source.set_loop(looping);
    gain.gain().set_value(volume);
    panner.pan().set_value(pan);
    source
        .connect_with_audio_node(&gain)
        .and_then(|_| gain.connect_with_audio_node(&panner))
        .and_then(|_| panner.connect_with_audio_node(&context.destination()))
        .map_err(|error| format!("connect audio graph: {error:?}"))?;
    let duration = buffer.duration();
    let offset = if looping && duration > 0.0 {
        offset.rem_euclid(duration)
    } else {
        offset.clamp(0.0, duration)
    };
    source
        .start_with_when_and_grain_offset(0.0, offset)
        .map_err(|error| format!("start source: {error:?}"))?;
    Ok(Voice {
        source,
        gain,
        panner,
        buffer,
        looping,
        paused: false,
        offset,
        started_at: context.current_time(),
        volume,
        pan,
    })
}

struct PendingChannel {
    request: PendingPlayback,
    decoded: Option<AudioBuffer>,
}

struct PlayingChannel {
    voice: Voice,
    generation: u64,
}

enum ChannelSlot {
    Empty,
    /// A jingle completed or failed. Keep its channel reserved until the
    /// sound engine observes `is_channel_playing == false` and calls
    /// `free_jingle`, so an unrelated SFX cannot reuse the same public handle
    /// in between those two operations.
    FinishedJingle,
    Pending(PendingChannel),
    Playing(PlayingChannel),
}

struct PendingMusic {
    request: PendingPlayback,
    decoded: Option<AudioBuffer>,
}

struct PlayingMusic {
    voice: Voice,
    generation: u64,
}

enum MusicSlot {
    Empty,
    Pending(PendingMusic),
    Playing(PlayingMusic),
}

struct BackendState {
    context: AudioContext,
    channels: Vec<ChannelSlot>,
    music: MusicSlot,
    music_finished_event: bool,
    music_volume: u16,
    jingle_channel: Option<usize>,
    generation: u64,
    request_ids: RequestIds,
}

impl BackendState {
    fn reap_channels(&mut self) {
        let current_time = self.context.current_time();
        let now_ms = now_ms();
        for (index, slot) in self.channels.iter_mut().enumerate() {
            let replacement = match std::mem::replace(slot, ChannelSlot::Empty) {
                ChannelSlot::Pending(pending) if pending.request.is_expired(now_ms) => {
                    tracing::warn!(
                        path = pending.request.path,
                        kind = ?pending.request.kind,
                        "expired cold browser audio request before decode completed"
                    );
                    ChannelSlot::Empty
                }
                ChannelSlot::Playing(playing) if !playing.voice.playing(current_time) => {
                    ChannelSlot::Empty
                }
                other => other,
            };
            *slot = if matches!(&replacement, ChannelSlot::Empty)
                && self.jingle_channel == Some(index)
            {
                ChannelSlot::FinishedJingle
            } else {
                replacement
            };
        }
        let music_ended = matches!(
            &self.music,
            MusicSlot::Playing(playing) if !playing.voice.playing(current_time)
        );
        if music_ended {
            self.music = MusicSlot::Empty;
            self.music_finished_event = true;
        }
    }

    fn free_channel(&mut self) -> Option<usize> {
        self.reap_channels();
        self.channels
            .iter()
            .position(|slot| matches!(slot, ChannelSlot::Empty))
    }

    fn reserve_channel(
        &mut self,
        kind: PlaybackKind,
        path: &str,
        looping: bool,
        fraction: f32,
        pan: f32,
    ) -> Option<(usize, u64, u64)> {
        let index = self.free_channel()?;
        let id = self.request_ids.next();
        let generation = self.generation;
        self.channels[index] = ChannelSlot::Pending(PendingChannel {
            request: PendingPlayback::new(
                id,
                generation,
                kind,
                path.to_owned(),
                looping,
                fraction,
                pan,
                1.0,
                now_ms(),
            ),
            decoded: None,
        });
        Some((index, id, generation))
    }

    fn finish_channel_load(
        &mut self,
        index: usize,
        id: u64,
        generation: u64,
        result: Result<AudioBuffer, String>,
    ) -> bool {
        let Some(slot) = self.channels.get_mut(index) else {
            return false;
        };
        let slot = std::mem::replace(slot, ChannelSlot::Empty);
        let ChannelSlot::Pending(mut pending) = slot else {
            self.channels[index] = slot;
            return false;
        };
        match completion_decision(&pending.request, id, generation, now_ms(), result.is_ok()) {
            CompletionDecision::IgnoreStale => {
                self.channels[index] = ChannelSlot::Pending(pending);
                return false;
            }
            CompletionDecision::Expire => {
                tracing::warn!(
                    path = pending.request.path,
                    kind = ?pending.request.kind,
                    "expired cold browser audio request before decode completed"
                );
                if self.jingle_channel == Some(index) {
                    self.channels[index] = ChannelSlot::FinishedJingle;
                }
                return false;
            }
            CompletionDecision::Fail => {
                let Err(error) = result else {
                    unreachable!("failed completion decision requires an error")
                };
                tracing::warn!(
                    path = pending.request.path,
                    kind = ?pending.request.kind,
                    error,
                    "cold browser audio request failed"
                );
                if self.jingle_channel == Some(index) {
                    self.channels[index] = ChannelSlot::FinishedJingle;
                }
                return false;
            }
            CompletionDecision::Start => {}
        }
        let buffer = result.expect("successful completion decision requires a buffer");
        if pending.request.paused {
            pending.decoded = Some(buffer);
            self.channels[index] = ChannelSlot::Pending(pending);
            return true;
        }
        self.channels[index] = ChannelSlot::Pending(pending);
        self.start_pending_channel(index, buffer)
    }

    fn start_pending_channel(&mut self, index: usize, buffer: AudioBuffer) -> bool {
        let slot = std::mem::replace(&mut self.channels[index], ChannelSlot::Empty);
        let ChannelSlot::Pending(pending) = slot else {
            self.channels[index] = slot;
            return false;
        };
        if pending.request.is_expired(now_ms()) {
            tracing::warn!(
                path = pending.request.path,
                kind = ?pending.request.kind,
                "expired cold browser audio request before playback started"
            );
            if self.jingle_channel == Some(index) {
                self.channels[index] = ChannelSlot::FinishedJingle;
            }
            return false;
        }
        let offset = buffer.duration() * f64::from(pending.request.fraction);
        match make_voice(
            &self.context,
            buffer,
            pending.request.looping,
            offset,
            pending.request.volume,
            pending.request.pan,
        ) {
            Ok(voice) => {
                tracing::debug!(
                    path = pending.request.path,
                    kind = ?pending.request.kind,
                    channel = index,
                    "cold browser audio request started"
                );
                self.channels[index] = ChannelSlot::Playing(PlayingChannel {
                    voice,
                    generation: pending.request.generation,
                });
                true
            }
            Err(error) => {
                tracing::warn!(
                    path = pending.request.path,
                    kind = ?pending.request.kind,
                    error,
                    "Web Audio play failed"
                );
                if self.jingle_channel == Some(index) {
                    self.channels[index] = ChannelSlot::FinishedJingle;
                }
                false
            }
        }
    }

    fn halt_channel(&mut self, index: usize) {
        let Some(slot) = self.channels.get_mut(index) else {
            return;
        };
        if let ChannelSlot::Playing(playing) = slot {
            playing.voice.stop();
        }
        *slot = ChannelSlot::Empty;
        if self.jingle_channel == Some(index) {
            self.jingle_channel = None;
        }
    }

    fn pause_channel(&mut self, index: usize) {
        let Some(slot) = self.channels.get_mut(index) else {
            return;
        };
        match slot {
            ChannelSlot::Pending(pending) => pending.request.paused = true,
            ChannelSlot::Playing(playing) => Self::pause_voice(&self.context, &mut playing.voice),
            ChannelSlot::Empty | ChannelSlot::FinishedJingle => {}
        }
    }

    fn resume_channel(&mut self, index: usize) {
        let context = self.context.clone();
        let mut ready = None;
        let mut resume_failed = false;
        let Some(slot) = self.channels.get_mut(index) else {
            return;
        };
        match slot {
            ChannelSlot::Pending(pending) => {
                pending.request.paused = false;
                ready = pending.decoded.take();
            }
            ChannelSlot::Playing(playing) => {
                resume_failed = !Self::resume_voice(&context, &mut playing.voice)
            }
            ChannelSlot::Empty | ChannelSlot::FinishedJingle => {}
        }
        if resume_failed {
            self.channels[index] = if self.jingle_channel == Some(index) {
                ChannelSlot::FinishedJingle
            } else {
                ChannelSlot::Empty
            };
        }
        if let Some(buffer) = ready {
            let _ = self.start_pending_channel(index, buffer);
        }
    }

    fn pause_voice(context: &AudioContext, voice: &mut Voice) {
        if !voice.paused {
            voice.offset = voice.position(context.current_time());
            voice.stop();
            voice.paused = true;
        }
    }

    fn resume_voice(context: &AudioContext, voice: &mut Voice) -> bool {
        if !voice.paused {
            return true;
        }
        match make_voice(
            context,
            voice.buffer.clone(),
            voice.looping,
            voice.offset,
            voice.volume,
            voice.pan,
        ) {
            Ok(replacement) => {
                *voice = replacement;
                true
            }
            Err(error) => {
                tracing::warn!(error, "Web Audio resume failed");
                false
            }
        }
    }

    fn pause_music_slot(&mut self) {
        match &mut self.music {
            MusicSlot::Pending(pending) => pending.request.paused = true,
            MusicSlot::Playing(playing) => Self::pause_voice(&self.context, &mut playing.voice),
            MusicSlot::Empty => {}
        }
    }

    fn resume_music_slot(&mut self) {
        let context = self.context.clone();
        let mut ready = None;
        let mut resume_failed = false;
        match &mut self.music {
            MusicSlot::Pending(pending) => {
                pending.request.paused = false;
                ready = pending.decoded.take();
            }
            MusicSlot::Playing(playing) => {
                resume_failed = !Self::resume_voice(&context, &mut playing.voice)
            }
            MusicSlot::Empty => {}
        }
        if resume_failed {
            self.music = MusicSlot::Empty;
            self.music_finished_event = true;
        }
        if let Some(buffer) = ready {
            let _ = self.start_pending_music(buffer);
        }
    }

    fn reserve_music(&mut self, path: &str, looping: bool) -> (u64, u64) {
        self.halt_music();
        let id = self.request_ids.next();
        let generation = self.generation;
        self.music = MusicSlot::Pending(PendingMusic {
            request: PendingPlayback::new(
                id,
                generation,
                PlaybackKind::Music,
                path.to_owned(),
                looping,
                0.0,
                0.0,
                (self.music_volume as f32 / 128.0).clamp(0.0, 1.0),
                now_ms(),
            ),
            decoded: None,
        });
        (id, generation)
    }

    fn finish_music_load(
        &mut self,
        id: u64,
        generation: u64,
        result: Result<AudioBuffer, String>,
    ) -> bool {
        let slot = std::mem::replace(&mut self.music, MusicSlot::Empty);
        let MusicSlot::Pending(mut pending) = slot else {
            self.music = slot;
            return false;
        };
        if !pending.request.belongs_to(id, generation) {
            self.music = MusicSlot::Pending(pending);
            return false;
        }
        let buffer = match result {
            Ok(buffer) => buffer,
            Err(error) => {
                tracing::warn!(
                    path = pending.request.path,
                    error,
                    "cold browser music/dialogue request failed"
                );
                self.music = MusicSlot::Empty;
                self.music_finished_event = true;
                return false;
            }
        };
        if pending.request.paused {
            pending.decoded = Some(buffer);
            self.music = MusicSlot::Pending(pending);
            return true;
        }
        self.music = MusicSlot::Pending(pending);
        self.start_pending_music(buffer)
    }

    fn start_pending_music(&mut self, buffer: AudioBuffer) -> bool {
        let slot = std::mem::replace(&mut self.music, MusicSlot::Empty);
        let MusicSlot::Pending(pending) = slot else {
            self.music = slot;
            return false;
        };
        match make_voice(
            &self.context,
            buffer,
            pending.request.looping,
            0.0,
            pending.request.volume,
            0.0,
        ) {
            Ok(voice) => {
                tracing::debug!(
                    path = pending.request.path,
                    "cold browser music/dialogue started"
                );
                self.music = MusicSlot::Playing(PlayingMusic {
                    voice,
                    generation: pending.request.generation,
                });
                true
            }
            Err(error) => {
                tracing::warn!(path = pending.request.path, error, "Web Audio music failed");
                self.music_finished_event = true;
                false
            }
        }
    }

    fn halt_music(&mut self) {
        if let MusicSlot::Playing(playing) = &self.music {
            playing.voice.stop();
        }
        self.music = MusicSlot::Empty;
        self.music_finished_event = false;
    }

    fn advance_generation(&mut self, generation: u64) {
        if self.generation == generation {
            return;
        }
        let mut cancelled = 0usize;
        for slot in &mut self.channels {
            match slot {
                ChannelSlot::Pending(pending) if pending.request.generation != generation => {
                    cancelled += 1;
                    *slot = ChannelSlot::Empty;
                }
                ChannelSlot::Playing(playing) if playing.generation != generation => {
                    playing.voice.stop();
                    cancelled += 1;
                    *slot = ChannelSlot::Empty;
                }
                ChannelSlot::FinishedJingle => *slot = ChannelSlot::Empty,
                _ => {}
            }
        }
        let music = std::mem::replace(&mut self.music, MusicSlot::Empty);
        self.music = match music {
            MusicSlot::Pending(pending) if pending.request.generation != generation => {
                cancelled += 1;
                MusicSlot::Empty
            }
            MusicSlot::Playing(playing) if playing.generation != generation => {
                playing.voice.stop();
                cancelled += 1;
                MusicSlot::Empty
            }
            current => current,
        };
        self.jingle_channel = None;
        self.music_finished_event = false;
        self.generation = generation;
        tracing::debug!(
            generation,
            cancelled,
            "advanced browser audio mission generation"
        );
    }

    fn stop_all(&mut self) {
        for slot in &self.channels {
            if let ChannelSlot::Playing(playing) = slot {
                playing.voice.stop();
            }
        }
        if let MusicSlot::Playing(playing) = &self.music {
            playing.voice.stop();
        }
        self.channels.fill_with(|| ChannelSlot::Empty);
        self.music = MusicSlot::Empty;
    }
}

pub struct KiraAudioBackend {
    state: Rc<RefCell<BackendState>>,
    start: web_time::Instant,
}

impl KiraAudioBackend {
    pub fn new(_sound_dir: impl Into<PathBuf>, num_channels: u32) -> Result<Self, String> {
        let (context, generation) =
            with_audio(|audio| (audio.context.clone(), audio.generation.current()))?;
        let state = Rc::new(RefCell::new(BackendState {
            context,
            channels: (0..num_channels).map(|_| ChannelSlot::Empty).collect(),
            music: MusicSlot::Empty,
            music_finished_event: false,
            music_volume: 128,
            jingle_channel: None,
            generation,
            request_ids: RequestIds::default(),
        }));
        with_audio(|audio| audio.backends.push(Rc::downgrade(&state)))?;
        Ok(Self {
            state,
            start: web_time::Instant::now(),
        })
    }

    fn play_at(
        &mut self,
        path: &str,
        looping: bool,
        fraction: f32,
        pan: f32,
        kind: PlaybackKind,
    ) -> Option<i32> {
        let asset = resolve_asset(path)
            .map_err(|error| tracing::warn!(path, error, "browser audio request rejected"))
            .ok()?;
        let context = self.state.borrow().context.clone();
        request_context_resume(&context, "play request");
        let (index, id, generation) = self
            .state
            .borrow_mut()
            .reserve_channel(kind, path, looping, fraction, pan)
            .or_else(|| {
                tracing::warn!(path, kind = ?kind, "browser audio channel queue is full");
                None
            })?;
        match request_decoded(asset) {
            Ok(DecodedRequest::Ready(buffer)) => {
                if !self
                    .state
                    .borrow_mut()
                    .finish_channel_load(index, id, generation, Ok(buffer))
                {
                    return None;
                }
            }
            Ok(DecodedRequest::Pending(load)) => {
                let state = Rc::downgrade(&self.state);
                wasm_bindgen_futures::spawn_local(async move {
                    let result = load.await;
                    if let Some(state) = state.upgrade() {
                        state
                            .borrow_mut()
                            .finish_channel_load(index, id, generation, result);
                    }
                });
            }
            Err(error) => {
                tracing::warn!(path, error, "browser audio decode request failed");
                self.state.borrow_mut().halt_channel(index);
                return None;
            }
        }
        Some(index as i32)
    }
}

impl AudioBackend for KiraAudioBackend {
    fn play_sound(&mut self, path: &str, looping: bool) -> Option<i32> {
        self.play_at(
            path,
            looping,
            0.0,
            0.0,
            PlaybackKind::for_sound(path, looping),
        )
    }

    fn play_sound_at(&mut self, path: &str, looping: bool, position: f32) -> Option<i32> {
        self.play_at(
            path,
            looping,
            position,
            0.0,
            PlaybackKind::for_sound(path, looping),
        )
    }

    fn halt_channel(&mut self, channel: i32) {
        if let Ok(index) = usize::try_from(channel) {
            self.state.borrow_mut().halt_channel(index);
        }
    }

    fn set_channel_volume(&mut self, channel: i32, volume: u16) {
        let Ok(index) = usize::try_from(channel) else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let Some(slot) = state.channels.get_mut(index) else {
            return;
        };
        let volume = (volume as f32 / 255.0).clamp(0.0, 1.0);
        match slot {
            ChannelSlot::Pending(pending) => pending.request.volume = volume,
            ChannelSlot::Playing(playing) => {
                playing.voice.volume = volume;
                playing.voice.gain.gain().set_value(volume);
            }
            ChannelSlot::Empty | ChannelSlot::FinishedJingle => {}
        }
    }

    fn is_channel_playing(&self, channel: i32) -> bool {
        let Ok(index) = usize::try_from(channel) else {
            return false;
        };
        let mut state = self.state.borrow_mut();
        state.reap_channels();
        state
            .channels
            .get(index)
            .is_some_and(|slot| matches!(slot, ChannelSlot::Pending(_) | ChannelSlot::Playing(_)))
    }

    fn pause_channels(&mut self, channel: i32) {
        let mut state = self.state.borrow_mut();
        if channel < 0 {
            for index in 0..state.channels.len() {
                state.pause_channel(index);
            }
            state.pause_music_slot();
        } else if let Ok(index) = usize::try_from(channel) {
            state.pause_channel(index);
        }
    }

    fn resume_channels(&mut self, channel: i32) {
        let context = self.state.borrow().context.clone();
        request_context_resume(&context, "resume channels");
        let mut state = self.state.borrow_mut();
        if channel < 0 {
            for index in 0..state.channels.len() {
                state.resume_channel(index);
            }
            state.resume_music_slot();
        } else if let Ok(index) = usize::try_from(channel) {
            state.resume_channel(index);
        }
    }

    fn play_music(&mut self, path: &str, looping: bool) -> bool {
        let asset = match resolve_asset(path) {
            Ok(asset) => asset,
            Err(error) => {
                tracing::warn!(path, error, "browser music/dialogue request rejected");
                return false;
            }
        };
        let context = self.state.borrow().context.clone();
        request_context_resume(&context, "music/dialogue play request");
        let (id, generation) = self.state.borrow_mut().reserve_music(path, looping);
        match request_decoded(asset) {
            Ok(DecodedRequest::Ready(buffer)) => {
                self.state
                    .borrow_mut()
                    .finish_music_load(id, generation, Ok(buffer))
            }
            Ok(DecodedRequest::Pending(load)) => {
                let state = Rc::downgrade(&self.state);
                wasm_bindgen_futures::spawn_local(async move {
                    let result = load.await;
                    if let Some(state) = state.upgrade() {
                        state.borrow_mut().finish_music_load(id, generation, result);
                    }
                });
                true
            }
            Err(error) => {
                tracing::warn!(path, error, "browser music/dialogue decode request failed");
                self.state.borrow_mut().halt_music();
                false
            }
        }
    }

    fn halt_music(&mut self) {
        self.state.borrow_mut().halt_music();
    }

    fn pause_music(&mut self) {
        self.state.borrow_mut().pause_music_slot();
    }

    fn resume_music(&mut self) {
        let context = self.state.borrow().context.clone();
        request_context_resume(&context, "resume music/dialogue");
        self.state.borrow_mut().resume_music_slot();
    }

    fn set_music_volume(&mut self, volume: u16) {
        let mut state = self.state.borrow_mut();
        state.music_volume = volume;
        let volume = (volume as f32 / 128.0).clamp(0.0, 1.0);
        match &mut state.music {
            MusicSlot::Pending(pending) => pending.request.volume = volume,
            MusicSlot::Playing(playing) => {
                playing.voice.volume = volume;
                playing.voice.gain.gain().set_value(volume);
            }
            MusicSlot::Empty => {}
        }
    }

    fn get_music_volume(&self) -> u16 {
        self.state.borrow().music_volume
    }

    fn take_music_finished(&mut self) -> bool {
        let mut state = self.state.borrow_mut();
        state.reap_channels();
        std::mem::take(&mut state.music_finished_event)
    }

    fn play_jingle(&mut self, path: &str) -> Option<i32> {
        let previous = self.state.borrow().jingle_channel;
        if let Some(previous) = previous {
            self.state.borrow_mut().halt_channel(previous);
        }
        let channel = self.play_at(path, false, 0.0, 0.0, PlaybackKind::Jingle)?;
        self.state.borrow_mut().jingle_channel = usize::try_from(channel).ok();
        Some(channel)
    }

    fn free_jingle(&mut self) {
        let channel = self.state.borrow_mut().jingle_channel.take();
        if let Some(channel) = channel {
            self.state.borrow_mut().halt_channel(channel);
        }
    }

    fn get_ticks(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn num_channels(&self) -> u32 {
        self.state.borrow().channels.len() as u32
    }

    fn can_3d_sound(&self) -> bool {
        true
    }

    fn play_sound_3d(
        &mut self,
        path: &str,
        looping: bool,
        position: f32,
        world_pos: [f32; 3],
    ) -> Option<i32> {
        self.play_at(
            path,
            looping,
            position,
            world_pos[0],
            PlaybackKind::for_sound(path, looping),
        )
    }

    fn set_channel_position_3d(&mut self, channel: i32, world_pos: [f32; 3]) {
        let Ok(index) = usize::try_from(channel) else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let Some(slot) = state.channels.get_mut(index) else {
            return;
        };
        let pan = world_pos[0].clamp(-1.0, 1.0);
        match slot {
            ChannelSlot::Pending(pending) => pending.request.pan = pan,
            ChannelSlot::Playing(playing) => {
                playing.voice.pan = pan;
                playing.voice.panner.pan().set_value(pan);
            }
            ChannelSlot::Empty | ChannelSlot::FinishedJingle => {}
        }
    }
}

impl Drop for KiraAudioBackend {
    fn drop(&mut self) {
        self.state.borrow_mut().stop_all();
    }
}

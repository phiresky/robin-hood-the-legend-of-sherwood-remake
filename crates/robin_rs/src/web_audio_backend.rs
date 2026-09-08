//! Browser-native playback; encoded audio and decoded PCM remain browser-owned.

use crate::sound::AudioBackend;
use crate::web_audio_state::{
    CompletionDecision, ContentDedup, PendingPlayback, PlaybackGeneration, PlaybackKind,
    ProgressCounter, RequestIds, WarmPriority, completion_decision, should_decode_during_warmup,
    warm_priority,
};
use futures::StreamExt as _;
use robin_assets::shipping_datadir::RemoteAudioAsset;
use robin_assets::shipping_datadir::ShippingDatadir;
use robin_engine::sbfile::SbFileSystem;
use std::sync::Arc;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::PathBuf,
    rc::{Rc, Weak},
};
use wasm_bindgen::{JsCast as _, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState, GainNode, StereoPannerNode,
};

mod assets;
use assets::{
    AudioAssets, DecodedRequest, buffer_key, request_decoded, request_encoded, resolve_asset,
};

const AUDIO_IO_CONCURRENCY: usize = 3;
struct BrowserAudio {
    mission_warmup: Option<futures::future::AbortHandle>,
    retired: bool,
    context: AudioContext,
    files: Arc<SbFileSystem>,
    catalog: Arc<ShippingDatadir>,
    assets: AudioAssets,
    generation: PlaybackGeneration,
    backends: Vec<Weak<RefCell<BackendState>>>,
}

/// Application-lifetime content authority. Deserialization cannot recreate a
/// browser device or grant access to a catalog; the application skips this handle.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct BrowserAudioSession {
    #[serde(skip)]
    inner: Option<Rc<RefCell<BrowserAudio>>>,
}

impl std::fmt::Debug for BrowserAudioSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserAudioSession")
            .field("available", &self.inner.is_some())
            .finish()
    }
}

impl BrowserAudioSession {
    pub fn new(files: Arc<SbFileSystem>, catalog: Arc<ShippingDatadir>) -> Result<Self, String> {
        let context = platform_context()?;
        Ok(Self {
            inner: Some(Rc::new(RefCell::new(BrowserAudio {
                mission_warmup: None,
                retired: false,
                context,
                files,
                catalog,
                assets: AudioAssets::default(),
                generation: PlaybackGeneration::default(),
                backends: Vec::new(),
            }))),
        })
    }

    fn with_audio<R>(&self, f: impl FnOnce(&mut BrowserAudio) -> R) -> Result<R, String> {
        let inner = self
            .inner
            .as_ref()
            .ok_or("browser audio session authority unavailable")?;
        let mut audio = inner.borrow_mut();
        if audio.retired {
            return Err("browser audio session retired".into());
        }
        Ok(f(&mut audio))
    }

    /// Permanently release this catalog's voices, reservations and caches.
    /// Existing handles and late async completions cannot reactivate it.
    pub fn retire(&self) {
        let Some(inner) = self.inner.as_ref() else {
            return;
        };
        let mut audio = inner.borrow_mut();
        audio.retired = true;
        if let Some(task) = audio.mission_warmup.take() {
            task.abort();
        }
        for backend in audio.backends.drain(..) {
            if let Some(backend) = backend.upgrade() {
                backend.borrow_mut().stop_all();
            }
        }
        audio.assets = AudioAssets::default();
    }

    fn downgrade(&self) -> Result<Weak<RefCell<BrowserAudio>>, String> {
        self.inner
            .as_ref()
            .map(Rc::downgrade)
            .ok_or_else(|| "browser audio session authority unavailable".into())
    }
}

impl Drop for BrowserAudio {
    fn drop(&mut self) {
        for backend in &self.backends {
            if let Some(backend) = backend.upgrade() {
                backend.borrow_mut().stop_all();
            }
        }
    }
}

// Only the device and its autoplay listener are page-owned. No content,
// catalog, pending request, backend, or decoded buffer lives in this slot.
thread_local! { static DEVICE: RefCell<Option<AudioContext>> = const { RefCell::new(None) }; }
fn platform_context() -> Result<AudioContext, String> {
    DEVICE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(context) = slot.as_ref() {
            return Ok(context.clone());
        }
        let context =
            AudioContext::new().map_err(|error| format!("create AudioContext: {error:?}"))?;
        install_autoplay_unlock(&context)?;
        *slot = Some(context.clone());
        Ok(context)
    })
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

fn build_warm_plan(
    session: &BrowserAudioSession,
    keys: Vec<String>,
    boot: bool,
) -> Result<Vec<WarmItem>, String> {
    let mut plan = Vec::new();
    let mut encoded_urls = ContentDedup::default();
    let mut decoded_keys = ContentDedup::default();
    let mut decoded_urls = HashSet::new();
    for path in keys {
        let asset = resolve_asset(session, &path)?;
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

async fn run_warm_plan<F>(
    session: &BrowserAudioSession,
    plan: Vec<WarmItem>,
    mut progress: F,
) -> Result<(), String>
where
    F: FnMut(AudioWarmProgress<'_>),
{
    let started = web_time::Instant::now();
    let mut yield_ms = 0.0;
    let mut progress_ms = 0.0;
    let mut progress_counter = ProgressCounter::new(plan.len());
    let progress_started = web_time::Instant::now();
    progress(AudioWarmProgress {
        completed: 0,
        total: progress_counter.total(),
        file: None,
    });
    // Let a blocking caller present its progress before the first network
    // or decode step. Background warmup uses a no-op progress observer and
    // cooperatively yields to the engine here instead.
    progress_ms += progress_started.elapsed().as_secs_f64() * 1000.0;
    let yield_started = web_time::Instant::now();
    crate::window::yield_to_runtime().await;
    yield_ms += yield_started.elapsed().as_secs_f64() * 1000.0;
    let mut work = futures::stream::iter(plan.into_iter().map(|item| async move {
        let result = match item.work {
            WarmWork::Encoded { url, retain_bundle } => {
                match request_encoded(session, &url, retain_bundle) {
                    Ok(load) => load.await.map(|_| ()),
                    Err(error) => Err(error),
                }
            }
            WarmWork::Decoded(asset) => match request_decoded(session, asset) {
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
        let progress_started = web_time::Instant::now();
        progress(AudioWarmProgress {
            completed: progress_counter.advance(),
            total: progress_counter.total(),
            file: Some(&label),
        });
        progress_ms += progress_started.elapsed().as_secs_f64() * 1000.0;
        let yield_started = web_time::Instant::now();
        crate::window::yield_to_runtime().await;
        yield_ms += yield_started.elapsed().as_secs_f64() * 1000.0;
    }
    tracing::info!(
        items = progress_counter.total(),
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        progress_ms,
        yield_ms,
        "startup timing: audio warmup"
    );
    Ok(())
}

/// Decode the deliberately small menu set before the application creates its
/// first audio backend. This is the browser boot boundary, not a whole-catalog
/// PCM preload.
pub async fn preload_boot_catalog(session: &BrowserAudioSession) -> Result<(), String> {
    let datadir = session.with_audio(|audio| audio.catalog.clone())?;
    let plan = build_warm_plan(session, datadir.boot_audio_keys(), true)?;
    if plan.is_empty() && !datadir.audio_assets.is_empty() {
        tracing::warn!(
            "browser audio catalog has no boot membership index; regenerate the shipping datadir"
        );
    }
    tracing::info!(items = plan.len(), "warming browser boot/menu audio");
    run_warm_plan(session, plan, |_| {}).await
}

/// Build the active mission audio warmup plan from shipping metadata.
/// Common short SFX are fetched as their compact logical bundle but remain
/// encoded; dialogue, actor voices, music and long standalone ambience decode.
fn active_mission_warm_plan(
    session: &BrowserAudioSession,
) -> Result<(String, Vec<WarmItem>), String> {
    let datadir = session.with_audio(|audio| audio.catalog.clone())?;
    let mission = datadir
        .active_mission_name()
        .ok_or("preload active mission audio: no active mission")?;
    let plan = build_warm_plan(session, datadir.active_audio_keys(), false)?;
    if plan.is_empty() && !datadir.audio_assets.is_empty() {
        tracing::warn!(
            mission,
            "browser audio catalog has no active-mission membership index; regenerate the shipping datadir"
        );
    }
    // Deliberately scope warmup to the ACTIVE mission. Fetching the whole
    // catalog would pull every other mission's dialogue and unrelated actor
    // voice banks; anything outside this exact set remains lazy on playback.
    tracing::info!(
        mission,
        items = plan.len(),
        "warming active mission browser audio"
    );
    Ok((mission, plan))
}

pub async fn preload_active_mission<F>(
    session: &BrowserAudioSession,
    progress: F,
) -> Result<(), String>
where
    F: FnMut(AudioWarmProgress<'_>),
{
    let (_, plan) = active_mission_warm_plan(session)?;
    run_warm_plan(session, plan, progress).await
}

/// Begin presentation-only audio work without blocking engine construction.
/// Engine duration tables use shipping metadata. A cold playback request
/// joins the in-flight decode and retains its existing cancellation policy.
pub fn preload_active_mission_in_background(session: &BrowserAudioSession) -> Result<(), String> {
    let (mission, plan) = active_mission_warm_plan(session)?;
    let (abort, registration) = futures::future::AbortHandle::new_pair();
    session.with_audio(|audio| {
        if let Some(previous) = audio.mission_warmup.replace(abort) {
            previous.abort();
        }
    })?;
    let session = session.clone();
    tracing::info!(mission, "background mission audio warmup started");
    wasm_bindgen_futures::spawn_local(async move {
        match futures::future::Abortable::new(run_warm_plan(&session, plan, |_| {}), registration)
            .await
        {
            Ok(Ok(())) => tracing::info!(mission, "background mission audio warmup complete"),
            Ok(Err(error)) => tracing::warn!(
                mission,
                error,
                "background mission audio warmup failed; playback will retry on demand"
            ),
            Err(_) => tracing::debug!(mission, "background mission audio warmup cancelled"),
        }
    });
    Ok(())
}

/// Compatibility entry point for old host-preload callers. Encoded bytes no
/// longer cross into wasm, but the requested catalog entry is genuinely
/// fetched and decoded.
pub async fn preload_boot(session: &BrowserAudioSession, path: &str) -> Result<(), String> {
    match request_decoded(session, resolve_asset(session, path)?)? {
        DecodedRequest::Ready(_) => Ok(()),
        DecodedRequest::Pending(load) => load.await.map(|_| ()),
    }
}

/// Advance the playback generation without discarding content-addressed PCM.
/// Every registered backend cancels old voices and reservations exactly; an
/// in-flight decode may still populate the shared cache, but its stale request
/// id/generation can never start playback in the new mission.
pub fn clear_mission(session: &BrowserAudioSession) -> Result<(), String> {
    let (generation, backends) = session.with_audio(|audio| {
        if let Some(task) = audio.mission_warmup.take() {
            task.abort();
        }
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
pub async fn replace_mission(
    session: &BrowserAudioSession,
    keys: Vec<String>,
) -> Result<(), String> {
    clear_mission(session)?;
    run_warm_plan(session, build_warm_plan(session, keys, false)?, |_| {}).await
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
        volume: f32,
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
                volume,
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
                crate::sound::music_gain(self.music_volume),
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
    session: BrowserAudioSession,
    state: Rc<RefCell<BackendState>>,
    start: web_time::Instant,
}

impl KiraAudioBackend {
    pub fn new_with_session(
        _sound_dir: impl Into<PathBuf>,
        num_channels: u32,
        session: BrowserAudioSession,
    ) -> Result<Self, String> {
        let (context, generation) =
            session.with_audio(|audio| (audio.context.clone(), audio.generation.current()))?;
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
        session.with_audio(|audio| audio.backends.push(Rc::downgrade(&state)))?;
        Ok(Self {
            session,
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
        self.try_play_at(path, looping, fraction, pan, kind, 255)
            .map_err(|error| tracing::warn!(path, %error, "browser audio request rejected"))
            .ok()
    }

    fn try_play_at(
        &mut self,
        path: &str,
        looping: bool,
        fraction: f32,
        pan: f32,
        kind: PlaybackKind,
        volume: u16,
    ) -> Result<i32, crate::sound::PlaybackError> {
        use crate::sound::PlaybackError;
        let asset = resolve_asset(&self.session, path).map_err(PlaybackError::Asset)?;
        let context = self.state.borrow().context.clone();
        request_context_resume(&context, "play request");
        let (index, id, generation) = self
            .state
            .borrow_mut()
            .reserve_channel(
                kind,
                path,
                looping,
                fraction,
                pan,
                crate::sound::channel_gain(volume),
            )
            .ok_or(PlaybackError::Capacity)?;
        match request_decoded(&self.session, asset) {
            Ok(DecodedRequest::Ready(buffer)) => {
                if !self
                    .state
                    .borrow_mut()
                    .finish_channel_load(index, id, generation, Ok(buffer))
                {
                    return Err(PlaybackError::Backend(
                        "failed to start decoded audio".into(),
                    ));
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
                return Err(PlaybackError::Backend(error));
            }
        }
        Ok(index as i32)
    }
}

impl AudioBackend for KiraAudioBackend {
    fn try_play_request(
        &mut self,
        request: crate::sound::PlaybackRequest<'_>,
    ) -> Result<i32, crate::sound::PlaybackError> {
        let kind = PlaybackKind::for_category(request.category, request.looping);
        let pan = request.spatial_position.map_or(0.0, |position| position[0]);
        self.try_play_at(
            request.asset,
            request.looping,
            request.fraction,
            pan,
            kind,
            request.volume,
        )
    }
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
        let volume = crate::sound::channel_gain(volume);
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
        let asset = match resolve_asset(&self.session, path) {
            Ok(asset) => asset,
            Err(error) => {
                tracing::warn!(path, error, "browser music/dialogue request rejected");
                return false;
            }
        };
        let context = self.state.borrow().context.clone();
        request_context_resume(&context, "music/dialogue play request");
        let (id, generation) = self.state.borrow_mut().reserve_music(path, looping);
        match request_decoded(&self.session, asset) {
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
        let volume = crate::sound::music_gain(volume);
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

#[cfg(test)]
mod browser_lifecycle_tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn session() -> BrowserAudioSession {
        BrowserAudioSession::new(
            Arc::new(SbFileSystem::new(Arc::new(
                robin_util::asset_fs::AssetVfs::new(),
            ))),
            Arc::new(ShippingDatadir::default()),
        )
        .unwrap()
    }

    #[wasm_bindgen_test]
    fn mission_transition_rejects_paused_completion_without_affecting_other_session() {
        let session = session();
        let other = BrowserAudioSession::new(
            Arc::new(SbFileSystem::new(Arc::new(
                robin_util::asset_fs::AssetVfs::new(),
            ))),
            Arc::new(ShippingDatadir::default()),
        )
        .unwrap();
        let backend = KiraAudioBackend::new_with_session("", 2, session.clone()).unwrap();
        let other_backend = KiraAudioBackend::new_with_session("", 2, other.clone()).unwrap();
        let (index, id, generation) = backend
            .state
            .borrow_mut()
            .reserve_channel(PlaybackKind::Voice, "voice.wav", false, 0.0, 0.0, 1.0)
            .unwrap();
        backend.state.borrow_mut().pause_channel(index);
        let (other_index, _, _) = other_backend
            .state
            .borrow_mut()
            .reserve_channel(PlaybackKind::Voice, "voice.wav", false, 0.0, 0.0, 1.0)
            .unwrap();
        let buffer = session
            .with_audio(|audio| audio.context.create_buffer(1, 800, 8000.0).unwrap())
            .unwrap();
        clear_mission(&session).unwrap();
        assert!(
            !backend
                .state
                .borrow_mut()
                .finish_channel_load(index, id, generation, Ok(buffer))
        );
        backend.state.borrow_mut().resume_channel(index);
        assert!(matches!(
            backend.state.borrow().channels[index],
            ChannelSlot::Empty
        ));
        assert!(matches!(
            other_backend.state.borrow().channels[other_index],
            ChannelSlot::Pending(_)
        ));
        session.retire();
        assert!(session.with_audio(|_| ()).is_err());
        assert!(other.with_audio(|_| ()).is_ok());
        other.retire();
        assert!(matches!(
            other_backend.state.borrow().channels[other_index],
            ChannelSlot::Empty
        ));
    }

    #[wasm_bindgen_test]
    fn deserialization_does_not_recreate_browser_authority() {
        let original = session();
        fn local_factory<F: crate::window::GameFactoryThreadBound>(_: F) {}
        let local_handle = original.clone();
        local_factory(move || drop(local_handle));
        let serialized = serde_json::to_string(&original).unwrap();
        let decoded: BrowserAudioSession = serde_json::from_str(&serialized).unwrap();
        assert!(decoded.with_audio(|_| ()).is_err());
        assert!(original.with_audio(|_| ()).is_ok());
    }
}

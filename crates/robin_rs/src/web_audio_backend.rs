//! Browser-native playback; encoded audio and decoded PCM remain browser-owned.

use crate::sound::AudioBackend;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AudioBuffer, AudioBufferSourceNode, AudioContext, GainNode, StereoPannerNode};

struct BrowserAudio {
    context: AudioContext,
    /// Decoded buffers are keyed by content URL, not by the legacy path the
    /// game used to request them. This makes aliases share one decode too.
    buffers: HashMap<String, AudioBuffer>,
    loading: HashSet<String>,
    failed: HashSet<String>,
    generation: u64,
}

thread_local! {
    static AUDIO: RefCell<Option<BrowserAudio>> = const { RefCell::new(None) };
}

fn with_audio<R>(f: impl FnOnce(&mut BrowserAudio) -> R) -> Result<R, String> {
    AUDIO.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(BrowserAudio {
                context: AudioContext::new().map_err(|e| format!("create AudioContext: {e:?}"))?,
                buffers: HashMap::new(),
                loading: HashSet::new(),
                failed: HashSet::new(),
                generation: 0,
            });
        }
        Ok(f(slot.as_mut().expect("initialized above")))
    })
}

thread_local! {
    /// Encoded bytes of already-downloaded logical audio bundles, keyed by
    /// bundle URL. Bundles are content-addressed and mission-independent,
    /// and total only a few MB encoded, so they are kept for the page's
    /// lifetime; every member sound decodes from a slice without another
    /// network round trip.
    static BUNDLE_CACHE: RefCell<HashMap<String, js_sys::ArrayBuffer>> =
        RefCell::new(HashMap::new());
}

/// Fetch a URL to a raw ArrayBuffer.
async fn fetch_array_buffer(url: &str) -> Result<js_sys::ArrayBuffer, String> {
    let window = web_sys::window().ok_or("no window")?;
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

/// Bytes of the bundle at `url`, downloading and caching on first use.
async fn bundle_bytes(url: &str) -> Result<js_sys::ArrayBuffer, String> {
    if let Some(bytes) = BUNDLE_CACHE.with(|cache| cache.borrow().get(url).cloned()) {
        return Ok(bytes);
    }
    let bytes = fetch_array_buffer(url).await?;
    // A concurrent fetch of the same bundle may have raced us; either
    // ArrayBuffer holds identical content-addressed bytes.
    BUNDLE_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(url.to_owned())
            .or_insert_with(|| bytes.clone())
            .clone()
    });
    Ok(bytes)
}

/// Resolve one catalog asset to its own encoded ArrayBuffer: a plain fetch
/// for standalone files, a cached-bundle slice for bundled ones.
async fn asset_encoded_bytes(
    asset: &robin_assets::shipping_datadir::RemoteAudioAsset,
) -> Result<js_sys::ArrayBuffer, String> {
    match asset.bundle_offset {
        None => fetch_array_buffer(&asset.url).await,
        Some(offset) => {
            let bundle = bundle_bytes(&asset.url).await?;
            let end = offset
                .checked_add(asset.encoded_size)
                .ok_or_else(|| format!("bundle slice overflow in {}", asset.url))?;
            if end > bundle.byte_length() {
                return Err(format!(
                    "bundle {} is {} bytes; asset wants {offset}..{end}",
                    asset.url,
                    bundle.byte_length()
                ));
            }
            Ok(bundle.slice_with_end(offset, end))
        }
    }
}

async fn fetch_and_decode(
    asset: &robin_assets::shipping_datadir::RemoteAudioAsset,
) -> Result<AudioBuffer, String> {
    let url = &asset.url;
    let context = with_audio(|audio| audio.context.clone())?;
    let encoded = asset_encoded_bytes(asset).await?;
    let promise = context
        .decode_audio_data(&encoded)
        .map_err(|e| format!("decode {url}: {e:?}"))?;
    JsFuture::from(promise)
        .await
        .map_err(|e| format!("decode {url}: {e:?}"))?
        .dyn_into::<AudioBuffer>()
        .map_err(|_| format!("decode {url}: result is not AudioBuffer"))
}

/// Cache key for a decoded buffer: bundled assets share a URL, so the slice
/// offset keeps them distinct while aliases of the same slice still share.
fn buffer_key(asset: &robin_assets::shipping_datadir::RemoteAudioAsset) -> String {
    match asset.bundle_offset {
        None => asset.url.clone(),
        Some(offset) => format!("{}#{offset}", asset.url),
    }
}

/// How many background prefetch requests run at once. Low on purpose: the
/// point is warming the HTTP cache during play, not competing with gameplay
/// traffic or the audio the player triggers right now.
const PREFETCH_CONCURRENCY: usize = 3;

thread_local! {
    static PREFETCH_STARTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PREFETCH_QUEUE: RefCell<std::collections::VecDeque<PrefetchItem>> =
        const { RefCell::new(std::collections::VecDeque::new()) };
}

/// One prefetch work item.
enum PrefetchItem {
    /// A logical-group bundle: downloaded into [`BUNDLE_CACHE`], so every
    /// member sound later decodes without a network round trip.
    Bundle(String),
    /// A standalone asset (music/ambience): fetched and dropped, which
    /// commits it to the browser HTTP cache without paying for decoded PCM.
    Standalone(String),
}

/// One-shot background prefetch of the audio catalog.
///
/// Audio is otherwise fetched lazily at first playback; this starts from
/// the first time any audio is requested — in practice the mission music
/// right after the blocking load finishes — and pulls, in order: the
/// ACTIVE mission's bundles and standalone tracks first, then every
/// remaining catalog group (for later missions and the menu). Nothing is
/// eagerly decoded: PCM for the whole catalog would cost hundreds of MB.
/// The existing `loading`/`buffers` dedup keeps real loads authoritative.
fn maybe_start_catalog_prefetch() {
    if PREFETCH_STARTED.with(|started| started.replace(true)) {
        return;
    }
    let Some(dd) = robin_assets::shipping_datadir::global() else {
        return;
    };
    let mission_first: Vec<String> = dd.active_audio_keys();
    let mut ordered = Vec::<PrefetchItem>::new();
    let mut seen = HashSet::<String>::new();
    let mut total_bytes: u64 = 0;
    let mut push_key = |key: &str, ordered: &mut Vec<PrefetchItem>, seen: &mut HashSet<String>| {
        let Some(asset) = dd.remote_audio_asset(Path::new(key)) else {
            return;
        };
        if !seen.insert(asset.url.clone()) {
            return;
        }
        total_bytes += u64::from(asset.encoded_size);
        ordered.push(match asset.bundle_offset {
            Some(_) => PrefetchItem::Bundle(asset.url),
            None => PrefetchItem::Standalone(asset.url),
        });
    };
    for key in &mission_first {
        push_key(key, &mut ordered, &mut seen);
    }
    let catalog_keys: Vec<String> = dd.audio_catalog_keys().map(str::to_owned).collect();
    for key in &catalog_keys {
        push_key(key, &mut ordered, &mut seen);
    }
    drop(push_key);
    if ordered.is_empty() {
        return;
    }
    tracing::info!(
        files = ordered.len(),
        total_bytes,
        mission_first = mission_first.len(),
        workers = PREFETCH_CONCURRENCY,
        "background audio prefetch started"
    );
    PREFETCH_QUEUE.with(|queue| queue.borrow_mut().extend(ordered));
    for _ in 0..PREFETCH_CONCURRENCY {
        wasm_bindgen_futures::spawn_local(async {
            loop {
                let Some(item) = PREFETCH_QUEUE.with(|queue| queue.borrow_mut().pop_front()) else {
                    break;
                };
                let result = match &item {
                    PrefetchItem::Bundle(url) => bundle_bytes(url).await.map(|_| ()),
                    PrefetchItem::Standalone(url) => fetch_array_buffer(url).await.map(|_| ()),
                };
                if let Err(error) = result {
                    tracing::debug!(error, "audio prefetch miss (will retry lazily)");
                }
            }
        });
    }
}

fn request_buffer(path: &str) -> Option<AudioBuffer> {
    maybe_start_catalog_prefetch();
    let asset = robin_assets::shipping_datadir::global()?.remote_audio_asset(Path::new(path))?;
    let url = buffer_key(&asset);
    let (buffer, generation, should_load) = with_audio(|audio| {
        if let Some(buffer) = audio.buffers.get(&url) {
            return (Some(buffer.clone()), audio.generation, false);
        }
        if audio.loading.contains(&url) || audio.failed.contains(&url) {
            return (None, audio.generation, false);
        }
        audio.loading.insert(url.clone());
        (None, audio.generation, true)
    })
    .ok()?;
    if buffer.is_some() {
        return buffer;
    }
    if !should_load {
        return None;
    }

    let requested_path = path.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        let result = fetch_and_decode(&asset).await;
        let _ = with_audio(|audio| {
            // A mission transition invalidates in-flight work. In particular,
            // an old decode must not overwrite the same logical alias in the
            // newly active catalog.
            if audio.generation != generation {
                return;
            }
            audio.loading.remove(&url);
            match result {
                Ok(buffer) => {
                    tracing::debug!(path = requested_path, %url, "browser audio ready");
                    audio.buffers.insert(url, buffer);
                }
                Err(error) => {
                    audio.failed.insert(url.clone());
                    tracing::warn!(path = requested_path, %url, error, "browser audio load failed");
                }
            }
        });
    });
    None
}

/// Legacy adapter while callers stop passing embedded audio bytes. Nothing is
/// decoded eagerly: cache misses are fetched from the standalone catalog.
pub async fn preload_boot(_path: &str, _bytes: &[u8]) -> Result<(), String> {
    Ok(())
}

pub fn clear_mission() -> Result<(), String> {
    with_audio(|audio| {
        audio.generation = audio.generation.wrapping_add(1);
        audio.buffers.clear();
        audio.loading.clear();
        audio.failed.clear();
    })
}

/// Legacy adapter while mission payload callers stop passing embedded bytes.
pub async fn replace_mission(_entries: Vec<(String, &[u8])>) -> Result<(), String> {
    clear_mission()
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
        AudioBufferSourceNode::new(context).map_err(|e| format!("create source: {e:?}"))?;
    let gain = GainNode::new(context).map_err(|e| format!("create gain: {e:?}"))?;
    let panner = StereoPannerNode::new(context).map_err(|e| format!("create panner: {e:?}"))?;
    source.set_buffer(Some(&buffer));
    source.set_loop(looping);
    gain.gain().set_value(volume);
    panner.pan().set_value(pan);
    source
        .connect_with_audio_node(&gain)
        .and_then(|_| gain.connect_with_audio_node(&panner))
        .and_then(|_| panner.connect_with_audio_node(&context.destination()))
        .map_err(|e| format!("connect audio graph: {e:?}"))?;
    let duration = buffer.duration();
    let offset = if looping && duration > 0.0 {
        offset.rem_euclid(duration)
    } else {
        offset.clamp(0.0, duration)
    };
    source
        .start_with_when_and_grain_offset(0.0, offset)
        .map_err(|e| format!("start source: {e:?}"))?;
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

pub struct KiraAudioBackend {
    context: AudioContext,
    channels: Vec<Option<Voice>>,
    music: Option<Voice>,
    /// `SoundEngine` asks for a music track only once. Keep that request while
    /// its standalone file is fetched so `take_music_finished`, which is
    /// polled every tick, can start it as soon as decoding completes.
    pending_music: Option<(String, bool)>,
    was_music_playing: bool,
    music_volume: u16,
    jingle_channel: Option<usize>,
    start: web_time::Instant,
}

impl KiraAudioBackend {
    pub fn new(_sound_dir: impl Into<PathBuf>, num_channels: u32) -> Result<Self, String> {
        Ok(Self {
            context: with_audio(|audio| audio.context.clone())?,
            channels: (0..num_channels).map(|_| None).collect(),
            music: None,
            pending_music: None,
            was_music_playing: false,
            music_volume: 128,
            jingle_channel: None,
            start: web_time::Instant::now(),
        })
    }
    fn buffer(&self, path: &str) -> Option<AudioBuffer> {
        request_buffer(path)
    }
    fn buffer_failed(&self, path: &str) -> bool {
        let Some(asset) = robin_assets::shipping_datadir::global()
            .and_then(|shipping| shipping.remote_audio_asset(Path::new(path)))
        else {
            return true;
        };
        with_audio(|audio| audio.failed.contains(&asset.url)).unwrap_or(true)
    }
    fn free_channel(&self) -> Option<usize> {
        let now = self.context.current_time();
        self.channels
            .iter()
            .position(|voice| voice.as_ref().is_none_or(|voice| !voice.playing(now)))
    }
    fn play_at(&mut self, path: &str, looping: bool, fraction: f32, pan: f32) -> Option<i32> {
        let buffer = self.buffer(path)?;
        let index = self.free_channel()?;
        if let Some(old) = self.channels[index].take() {
            old.stop();
        }
        let offset = buffer.duration() * f64::from(fraction.clamp(0.0, 0.999));
        let voice = make_voice(&self.context, buffer, looping, offset, 1.0, pan)
            .map_err(|error| tracing::warn!(path, error, "Web Audio play failed"))
            .ok()?;
        let _ = self.context.resume();
        self.channels[index] = Some(voice);
        Some(index as i32)
    }
    fn pause_voice(context: &AudioContext, voice: &mut Voice) {
        if !voice.paused {
            voice.offset = voice.position(context.current_time());
            voice.stop();
            voice.paused = true;
        }
    }
    fn resume_voice(context: &AudioContext, voice: &mut Voice) {
        if voice.paused {
            match make_voice(
                context,
                voice.buffer.clone(),
                voice.looping,
                voice.offset,
                voice.volume,
                voice.pan,
            ) {
                Ok(replacement) => *voice = replacement,
                Err(error) => tracing::warn!(error, "Web Audio resume failed"),
            }
            let _ = context.resume();
        }
    }
    fn start_music_buffer(&mut self, path: &str, buffer: AudioBuffer, looping: bool) -> bool {
        if let Some(old) = self.music.take() {
            old.stop();
        }
        let volume = (self.music_volume as f32 / 128.0).clamp(0.0, 1.0);
        match make_voice(&self.context, buffer, looping, 0.0, volume, 0.0) {
            Ok(voice) => {
                let _ = self.context.resume();
                self.music = Some(voice);
                self.was_music_playing = true;
                true
            }
            Err(error) => {
                tracing::warn!(path, error, "Web Audio music failed");
                false
            }
        }
    }
}

impl AudioBackend for KiraAudioBackend {
    fn play_sound(&mut self, path: &str, looping: bool) -> Option<i32> {
        self.play_at(path, looping, 0.0, 0.0)
    }
    fn play_sound_at(&mut self, path: &str, looping: bool, position: f32) -> Option<i32> {
        self.play_at(path, looping, position, 0.0)
    }
    fn halt_channel(&mut self, channel: i32) {
        if let Ok(index) = usize::try_from(channel)
            && let Some(slot) = self.channels.get_mut(index)
            && let Some(voice) = slot.take()
        {
            voice.stop();
        }
    }
    fn set_channel_volume(&mut self, channel: i32, volume: u16) {
        if let Ok(index) = usize::try_from(channel)
            && let Some(Some(voice)) = self.channels.get_mut(index)
        {
            voice.volume = (volume as f32 / 255.0).clamp(0.0, 1.0);
            voice.gain.gain().set_value(voice.volume);
        }
    }
    fn is_channel_playing(&self, channel: i32) -> bool {
        usize::try_from(channel)
            .ok()
            .and_then(|i| self.channels.get(i))
            .and_then(Option::as_ref)
            .is_some_and(|v| v.playing(self.context.current_time()))
    }
    fn pause_channels(&mut self, channel: i32) {
        if channel < 0 {
            for voice in self.channels.iter_mut().flatten() {
                Self::pause_voice(&self.context, voice);
            }
            if let Some(music) = &mut self.music {
                Self::pause_voice(&self.context, music);
            }
        } else if let Some(Some(voice)) = self.channels.get_mut(channel as usize) {
            Self::pause_voice(&self.context, voice);
        }
    }
    fn resume_channels(&mut self, channel: i32) {
        if channel < 0 {
            for voice in self.channels.iter_mut().flatten() {
                Self::resume_voice(&self.context, voice);
            }
            if let Some(music) = &mut self.music {
                Self::resume_voice(&self.context, music);
            }
        } else if let Some(Some(voice)) = self.channels.get_mut(channel as usize) {
            Self::resume_voice(&self.context, voice);
        }
    }
    fn play_music(&mut self, path: &str, looping: bool) -> bool {
        let Some(buffer) = self.buffer(path) else {
            if self.buffer_failed(path) {
                return false;
            }
            self.pending_music = Some((path.to_owned(), looping));
            self.was_music_playing = true;
            return true;
        };
        self.pending_music = None;
        self.start_music_buffer(path, buffer, looping)
    }
    fn halt_music(&mut self) {
        self.pending_music = None;
        if let Some(music) = self.music.take() {
            music.stop();
        }
        self.was_music_playing = false;
    }
    fn pause_music(&mut self) {
        if let Some(music) = &mut self.music {
            Self::pause_voice(&self.context, music);
        }
    }
    fn resume_music(&mut self) {
        if let Some(music) = &mut self.music {
            Self::resume_voice(&self.context, music);
        }
    }
    fn set_music_volume(&mut self, volume: u16) {
        self.music_volume = volume;
        if let Some(music) = &mut self.music {
            music.volume = (volume as f32 / 128.0).clamp(0.0, 1.0);
            music.gain.gain().set_value(music.volume);
        }
    }
    fn get_music_volume(&self) -> u16 {
        self.music_volume
    }
    fn take_music_finished(&mut self) -> bool {
        if let Some((path, looping)) = self.pending_music.clone() {
            if let Some(buffer) = self.buffer(&path) {
                self.pending_music = None;
                if !self.start_music_buffer(&path, buffer, looping) {
                    self.was_music_playing = false;
                    return true;
                }
            } else if self.buffer_failed(&path) {
                self.pending_music = None;
                if let Some(old) = self.music.take() {
                    old.stop();
                }
                self.was_music_playing = false;
                return true;
            }
            return false;
        }
        let playing = self
            .music
            .as_ref()
            .is_some_and(|v| v.playing(self.context.current_time()));
        if self.was_music_playing && !playing {
            self.was_music_playing = false;
            self.music = None;
            true
        } else {
            false
        }
    }
    fn play_jingle(&mut self, path: &str) -> Option<i32> {
        let channel = self.play_sound(path, false)?;
        self.jingle_channel = Some(channel as usize);
        Some(channel)
    }
    fn free_jingle(&mut self) {
        if let Some(channel) = self.jingle_channel.take() {
            self.halt_channel(channel as i32);
        }
    }
    fn get_ticks(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }
    fn num_channels(&self) -> u32 {
        self.channels.len() as u32
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
        self.play_at(path, looping, position, world_pos[0].clamp(-1.0, 1.0))
    }
    fn set_channel_position_3d(&mut self, channel: i32, world_pos: [f32; 3]) {
        if let Ok(index) = usize::try_from(channel)
            && let Some(Some(voice)) = self.channels.get_mut(index)
        {
            voice.pan = world_pos[0].clamp(-1.0, 1.0);
            voice.panner.pan().set_value(voice.pan);
        }
    }
}

impl Drop for KiraAudioBackend {
    fn drop(&mut self) {
        for voice in self.channels.iter().flatten() {
            voice.stop();
        }
        if let Some(music) = &self.music {
            music.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    // Browser behavior is covered by the wasm smoke test. Legacy-path alias
    // resolution now belongs to ShippingDatadir::remote_audio_asset and is
    // tested alongside the manifest rather than duplicated here.
}

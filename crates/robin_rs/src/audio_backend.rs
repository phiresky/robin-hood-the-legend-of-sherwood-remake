//! Native Kira and browser-native Web Audio backends.
//!
//! Implements [`AudioBackend`](crate::sound::AudioBackend) on top of
//! [`kira`]. SFX go through a pool of `StaticSoundData` handles played
//! through one shared track per "channel slot" so the channel-id surface
//! the rest of the game expects (`play_sound` returns an `i32` channel
//! number) keeps working.

use std::path::{Path, PathBuf};

use crate::sound::AudioBackend;
use robin_engine::sound_cache::SampleLoader;

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
use kira::{
    AudioManager, AudioManagerSettings, DefaultBackend, Tween,
    listener::ListenerHandle,
    sound::{
        FromFileError,
        static_sound::{StaticSoundData, StaticSoundHandle},
    },
    track::{SpatialTrackBuilder, SpatialTrackHandle},
};
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
use std::collections::HashMap;

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
type MusicHandle = StreamingSoundHandle<FromFileError>;

/// Kira-backed audio backend.
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
pub struct KiraAudioBackend {
    manager: Option<AudioManager>,
    sound_dir: PathBuf,
    /// Cached `StaticSoundData` (decoded audio) keyed by file path.
    /// kira's `StaticSoundData` is cheap to clone — clones share the
    /// underlying sample buffer.
    sample_cache: HashMap<String, StaticSoundData>,
    /// Per-channel handle tracking. `channel_idx -> currently-playing handle`.
    /// `None` means the slot is free.
    channels: Vec<Option<StaticSoundHandle>>,
    /// Music slot — independent from SFX channels.
    music_handle: Option<MusicHandle>,
    /// `was_music_playing` — tracked so `take_music_finished` can edge-detect.
    was_music_playing: bool,
    /// Channel slot the active jingle occupies.
    jingle_channel: Option<usize>,
    num_channels: u32,
    /// Process-uptime origin for `get_ticks`.
    start: web_time::Instant,
    /// Spatial-audio listener at the world origin facing forward.
    /// `SoundGeometry::get_3d_playing_params` already returns
    /// listener-relative unit-ish vectors, so we never move it.
    listener: ListenerHandle,
    /// One spatial sub-track per channel slot, lazily created on the
    /// first 3D play for that slot and reused thereafter. `None` until
    /// the slot has been used in 3D mode.
    spatial_tracks: Vec<Option<SpatialTrackHandle>>,
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
impl KiraAudioBackend {
    /// Construct a new audio backend.
    pub fn new(sound_dir: impl Into<PathBuf>, num_channels: u32) -> Result<Self, String> {
        let reused: Option<AudioManager> = None;

        let reused_manager = reused.is_some();
        let mut manager = match reused {
            Some(manager) => manager,
            None => AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
                .map_err(|e| format!("kira AudioManager init failed: {e}"))?,
        };
        let listener = manager
            .add_listener(
                mint::Vector3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                mint::Quaternion {
                    v: mint::Vector3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    s: 1.0,
                },
            )
            .map_err(|e| format!("kira add_listener failed: {e}"))?;
        let channels = (0..num_channels).map(|_| None).collect();
        let spatial_tracks = (0..num_channels).map(|_| None).collect();
        tracing::info!(
            reused_manager,
            "kira audio initialised: {num_channels} channel slots"
        );
        Ok(Self {
            manager: Some(manager),
            sound_dir: sound_dir.into(),
            sample_cache: HashMap::new(),
            channels,
            music_handle: None,
            was_music_playing: false,
            jingle_channel: None,
            num_channels,
            start: web_time::Instant::now(),
            listener,
            spatial_tracks,
        })
    }

    fn resolve_path(&self, file_name: &str) -> PathBuf {
        let normalised = file_name.replace('\\', "/");
        let absolute = Path::new(&normalised).is_absolute();
        let path = if absolute {
            PathBuf::from(&normalised)
        } else {
            self.sound_dir.join(&normalised)
        };
        let candidates = if absolute {
            vec![path.clone()]
        } else {
            // actors.res stores speech paths relative to its Exclamations
            // directory (for example `Expressions/X_SD_...wav`), whereas
            // ordinary FX/source paths are relative to Data/Sounds.
            vec![
                path.clone(),
                self.sound_dir.join("Exclamations").join(&normalised),
            ]
        };
        for candidate in candidates {
            if candidate.exists() {
                return candidate;
            }
            if let Some(p) = robin_engine::sbfile::resolve_case_insensitive(&candidate)
                && p.exists()
            {
                return p;
            }
            if let Some(full) = candidate.to_str()
                && let Some(p) = robin_engine::sbfile::resolve_data_path(full)
            {
                return p;
            }
        }
        path
    }

    fn load_sample(&mut self, file_name: &str) -> Option<StaticSoundData> {
        if let Some(s) = self.sample_cache.get(file_name) {
            return Some(s.clone());
        }
        let path = self.resolve_path(file_name);
        match load_static_sound(&path) {
            Ok(data) => {
                self.sample_cache
                    .insert(file_name.to_string(), data.clone());
                Some(data)
            }
            Err(e) => {
                tracing::warn!("kira: failed to load '{}': {}", path.display(), e);
                None
            }
        }
    }

    fn find_free_channel(&self) -> Option<usize> {
        self.channels
            .iter()
            .position(|c| c.as_ref().is_none_or(is_handle_done))
    }

    /// Get-or-create the spatial sub-track for a channel slot, set its
    /// position, and return a mutable handle. Returns `None` if track
    /// allocation fails.
    fn ensure_spatial_track(
        &mut self,
        idx: usize,
        pos: [f32; 3],
    ) -> Option<&mut SpatialTrackHandle> {
        let mint_pos = mint::Vector3 {
            x: pos[0],
            y: pos[1],
            z: pos[2],
        };
        if self.spatial_tracks[idx].is_none() {
            let listener_id = self.listener.id();
            let track = self
                .manager
                .as_mut()
                .expect("live Kira backend lost its AudioManager")
                .add_spatial_sub_track(
                    listener_id,
                    mint_pos,
                    // Distance attenuation is already baked into our
                    // per-sound volume by `SoundGeometry`, so we
                    // disable kira's built-in falloff and let the
                    // spatial track only contribute panning.
                    SpatialTrackBuilder::new().attenuation_function(None),
                )
                .ok()?;
            self.spatial_tracks[idx] = Some(track);
        } else {
            self.spatial_tracks[idx]
                .as_mut()
                .unwrap()
                .set_position(mint_pos, Tween::default());
        }
        self.spatial_tracks[idx].as_mut()
    }

    fn resolve_music_path(path: &str) -> PathBuf {
        fn resolve_one(path: &str) -> Option<PathBuf> {
            let raw = PathBuf::from(path);
            if raw.exists() {
                return Some(raw);
            }
            if let Some(p) = robin_engine::sbfile::resolve_case_insensitive(&raw)
                && p.is_file()
            {
                return Some(p);
            }
            robin_engine::sbfile::resolve_data_path(path)
        }

        if let Some(resolved) = resolve_one(path) {
            return resolved;
        }

        let raw = PathBuf::from(path);
        if raw
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
        {
            let ogg_path = raw.with_extension("ogg");
            if let Some(ogg_path) = ogg_path.to_str()
                && let Some(resolved) = resolve_one(ogg_path)
            {
                return resolved;
            }
        } else if raw
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ogg"))
        {
            let wav_path = raw.with_extension("wav");
            if let Some(wav_path) = wav_path.to_str()
                && let Some(resolved) = resolve_one(wav_path)
            {
                return resolved;
            }
        }

        raw
    }
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn load_static_sound(path: &Path) -> Result<StaticSoundData, FromFileError> {
    StaticSoundData::from_file(path)
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn is_handle_done(h: &StaticSoundHandle) -> bool {
    matches!(h.state(), kira::sound::PlaybackState::Stopped)
}

/// 0.0–1.0 linear amplitude → decibels (kira's native volume unit).
/// Below ~ -60 dB we clamp to silence to avoid `-inf` from log10(0).
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn amplitude_to_decibels(amp: f32) -> kira::Decibels {
    if amp <= 0.0 {
        kira::Decibels::SILENCE
    } else {
        kira::Decibels::from(20.0 * amp.log10())
    }
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
impl AudioBackend for KiraAudioBackend {
    fn play_sound(&mut self, file_name: &str, looping: bool) -> Option<i32> {
        let data = self.load_sample(file_name)?;
        let data = if looping {
            // kira's StaticSoundData has a `.loop_region(..)` builder that
            // sets the loop on the resulting handle.
            data.loop_region(..)
        } else {
            data
        };
        let idx = self.find_free_channel()?;
        match self
            .manager
            .as_mut()
            .expect("live Kira backend lost its AudioManager")
            .play(data)
        {
            Ok(handle) => {
                self.channels[idx] = Some(handle);
                Some(idx as i32)
            }
            Err(e) => {
                tracing::debug!("kira play '{file_name}': {e}");
                None
            }
        }
    }

    fn play_sound_at(&mut self, file_name: &str, looping: bool, _position: f32) -> Option<i32> {
        // kira's StaticSoundData supports start_position via builder; the
        // single ambient-loop caller passes 0.0 in practice, so we just
        // proxy through to play_sound.
        self.play_sound(file_name, looping)
    }

    fn halt_channel(&mut self, channel: i32) {
        if let Some(slot) = self.channels.get_mut(channel as usize)
            && let Some(h) = slot
        {
            h.stop(Tween::default());
            *slot = None;
        }
    }

    fn set_channel_volume(&mut self, channel: i32, volume: u16) {
        let v = (volume as f32 / 255.0).clamp(0.0, 1.0);
        if let Some(Some(h)) = self.channels.get_mut(channel as usize) {
            h.set_volume(amplitude_to_decibels(v), Tween::default());
        }
    }

    fn is_channel_playing(&self, channel: i32) -> bool {
        self.channels
            .get(channel as usize)
            .and_then(|s| s.as_ref())
            .is_some_and(|h| !is_handle_done(h))
    }

    fn pause_channels(&mut self, channel: i32) {
        if channel < 0 {
            for h in self.channels.iter_mut().flatten() {
                h.pause(Tween::default());
            }
            if let Some(h) = &mut self.music_handle {
                h.pause(Tween::default());
            }
        } else if let Some(Some(h)) = self.channels.get_mut(channel as usize) {
            h.pause(Tween::default());
        }
    }

    fn resume_channels(&mut self, channel: i32) {
        if channel < 0 {
            for h in self.channels.iter_mut().flatten() {
                h.resume(Tween::default());
            }
            if let Some(h) = &mut self.music_handle {
                h.resume(Tween::default());
            }
        } else if let Some(Some(h)) = self.channels.get_mut(channel as usize) {
            h.resume(Tween::default());
        }
    }

    fn play_music(&mut self, path: &str, looping: bool) -> bool {
        let full_path = KiraAudioBackend::resolve_music_path(path);
        let data = match StreamingSoundData::from_file(&full_path) {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!(
                    "kira: load streaming music '{}': {}",
                    full_path.display(),
                    e
                );
                return false;
            }
        };
        let data = if looping { data.loop_region(..) } else { data };
        if let Some(h) = &mut self.music_handle {
            h.stop(Tween::default());
        }
        match self
            .manager
            .as_mut()
            .expect("live Kira backend lost its AudioManager")
            .play(data)
        {
            Ok(handle) => {
                self.music_handle = Some(handle);
                self.was_music_playing = true;
                true
            }
            Err(e) => {
                tracing::warn!("kira: play music '{}': {}", path, e);
                false
            }
        }
    }

    fn halt_music(&mut self) {
        if let Some(h) = &mut self.music_handle {
            h.stop(Tween::default());
        }
        self.music_handle = None;
        self.was_music_playing = false;
    }

    fn pause_music(&mut self) {
        if let Some(h) = &mut self.music_handle {
            h.pause(Tween::default());
        }
    }

    fn resume_music(&mut self) {
        if let Some(h) = &mut self.music_handle {
            h.resume(Tween::default());
        }
    }

    fn set_music_volume(&mut self, volume: u16) {
        let v = (volume as f32 / 128.0).clamp(0.0, 1.0);
        if let Some(h) = &mut self.music_handle {
            h.set_volume(amplitude_to_decibels(v), Tween::default());
        }
    }

    fn get_music_volume(&self) -> u16 {
        // kira doesn't expose current volume on a handle directly — we
        // don't track it locally. Return a neutral default; the only
        // caller is the volume-restore path on level transitions which
        // re-applies the persisted setting via `set_music_volume`.
        128
    }

    fn take_music_finished(&mut self) -> bool {
        let playing = self
            .music_handle
            .as_ref()
            .is_some_and(|h| !matches!(h.state(), kira::sound::PlaybackState::Stopped));
        if self.was_music_playing && !playing {
            self.was_music_playing = false;
            self.music_handle = None;
            return true;
        }
        false
    }

    fn play_jingle(&mut self, path: &str) -> Option<i32> {
        let full_path = self.resolve_path(path);
        let data = match load_static_sound(&full_path) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("kira: load jingle '{}': {}", full_path.display(), e);
                return None;
            }
        };
        let idx = self.find_free_channel()?;
        match self
            .manager
            .as_mut()
            .expect("live Kira backend lost its AudioManager")
            .play(data)
        {
            Ok(handle) => {
                self.channels[idx] = Some(handle);
                self.jingle_channel = Some(idx);
                Some(idx as i32)
            }
            Err(e) => {
                tracing::warn!("kira: play jingle '{}': {}", full_path.display(), e);
                None
            }
        }
    }

    fn free_jingle(&mut self) {
        if let Some(idx) = self.jingle_channel.take()
            && let Some(slot) = self.channels.get_mut(idx)
            && let Some(h) = slot
        {
            h.stop(Tween::default());
            *slot = None;
        }
    }

    fn get_ticks(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn num_channels(&self) -> u32 {
        self.num_channels
    }

    fn can_3d_sound(&self) -> bool {
        true
    }

    fn play_sound_3d(
        &mut self,
        file_name: &str,
        looping: bool,
        _sample_pos: f32,
        world_pos: [f32; 3],
    ) -> Option<i32> {
        // Ignore `_sample_pos` to mirror the 2D `play_sound_at` —
        // kira's start-position builder is wired only when a real
        // caller needs it.
        let data = self.load_sample(file_name)?;
        let data = if looping { data.loop_region(..) } else { data };
        let idx = self.find_free_channel()?;
        let track = self.ensure_spatial_track(idx, world_pos)?;
        match track.play(data) {
            Ok(handle) => {
                self.channels[idx] = Some(handle);
                Some(idx as i32)
            }
            Err(e) => {
                tracing::debug!("kira spatial play '{file_name}': {e}");
                None
            }
        }
    }

    fn set_channel_position_3d(&mut self, channel: i32, world_pos: [f32; 3]) {
        let Ok(idx) = usize::try_from(channel) else {
            return;
        };
        if idx >= self.spatial_tracks.len() {
            return;
        }
        if let Some(track) = self.spatial_tracks[idx].as_mut() {
            track.set_position(
                mint::Vector3 {
                    x: world_pos[0],
                    y: world_pos[1],
                    z: world_pos[2],
                },
                Tween::default(),
            );
        }
    }
}

// ─── Stub backend (audio feature disabled) ──────────────────────────
//
// Wasm/no-audio builds get the same type with a no-op impl so callers
// don't need per-cfg plumbing.

#[cfg(all(feature = "audio", target_arch = "wasm32"))]
pub use crate::web_audio_backend::{KiraAudioBackend, preload_boot, replace_mission};

#[cfg(not(feature = "audio"))]
pub struct KiraAudioBackend;

#[cfg(not(feature = "audio"))]
impl KiraAudioBackend {
    pub fn new(_sound_dir: impl Into<PathBuf>, _num_channels: u32) -> Result<Self, String> {
        Err("audio feature disabled in this build".to_string())
    }
}

#[cfg(not(feature = "audio"))]
impl AudioBackend for KiraAudioBackend {
    fn play_sound(&mut self, _file_name: &str, _looping: bool) -> Option<i32> {
        None
    }
    fn play_sound_at(&mut self, _file_name: &str, _looping: bool, _position: f32) -> Option<i32> {
        None
    }
    fn halt_channel(&mut self, _channel: i32) {}
    fn set_channel_volume(&mut self, _channel: i32, _volume: u16) {}
    fn is_channel_playing(&self, _channel: i32) -> bool {
        false
    }
    fn pause_channels(&mut self, _channel: i32) {}
    fn resume_channels(&mut self, _channel: i32) {}
    fn play_music(&mut self, _path: &str, _looping: bool) -> bool {
        false
    }
    fn halt_music(&mut self) {}
    fn pause_music(&mut self) {}
    fn resume_music(&mut self) {}
    fn set_music_volume(&mut self, _volume: u16) {}
    fn get_music_volume(&self) -> u16 {
        0
    }
    fn take_music_finished(&mut self) -> bool {
        false
    }
    fn play_jingle(&mut self, _path: &str) -> Option<i32> {
        None
    }
    fn free_jingle(&mut self) {}
    fn get_ticks(&self) -> u32 {
        0
    }
    fn num_channels(&self) -> u32 {
        0
    }
}

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

    while offset + 8 <= data.len() {
        let chunk_id = &data[offset..offset + 4];
        let chunk_size = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().ok()?);

        if chunk_id == b"fmt " && offset + 20 <= data.len() {
            byte_rate = u32::from_le_bytes(data[offset + 16..offset + 20].try_into().ok()?);
        } else if chunk_id == b"data" {
            data_size = chunk_size;
        }

        offset += 8 + chunk_size as usize;
        if !offset.is_multiple_of(2) {
            offset += 1;
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

    let duration_ms = (last_granule * 1000).checked_div(sample_rate as u64)?;
    u32::try_from(duration_ms).ok()
}

pub fn create_sample_loader(base_dir: PathBuf) -> Box<SampleLoader> {
    Box::new(move |file_name: &str| {
        tracing::trace!(file_name, "SampleLoader: enter");
        let normalised = file_name.replace('\\', "/");
        let absolute = Path::new(&normalised).is_absolute();
        let path = if absolute {
            PathBuf::from(&normalised)
        } else {
            base_dir.join(&normalised)
        };
        let candidates = if absolute {
            vec![path]
        } else {
            vec![path, base_dir.join("Exclamations").join(&normalised)]
        };
        #[cfg(target_arch = "wasm32")]
        if let Some((size, duration_ms)) =
            robin_assets::shipping_datadir::global().and_then(|shipping| {
                candidates
                    .iter()
                    .find_map(|path| shipping.active_audio_metadata(path))
            })
        {
            // Web Audio already owns the decoded buffer. SoundCache needs only
            // authoritative bookkeeping, not another encoded-byte copy.
            return Some((Vec::new(), size, duration_ms));
        }
        let candidates = candidates.into_iter().flat_map(|candidate| {
            let opus = candidate.with_extension("opus");
            [candidate, opus]
        });
        let (data, source_path) = candidates.into_iter().find_map(|candidate| {
            if let Ok(data) = robin_util::asset_fs::read(&candidate) {
                return Some((data, candidate));
            }
            let resolved =
                robin_engine::sbfile::resolve_case_insensitive(&candidate).or_else(|| {
                    candidate
                        .to_str()
                        .and_then(robin_engine::sbfile::resolve_data_path)
                })?;
            robin_util::asset_fs::read(&resolved)
                .ok()
                .map(|data| (data, resolved))
        })?;
        let size = data.len() as u32;
        let duration_ms = robin_assets::shipping_datadir::global()
            .and_then(|shipping| shipping.active_audio_duration_ms(&source_path))
            .or_else(|| wav_duration_ms(&data))
            .unwrap_or(0);
        Some((data, size, duration_ms))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    fn music_path_falls_back_from_wav_to_ogg() {
        let temp = tempfile::tempdir().unwrap();
        let ogg = temp.path().join("Lincoln_D.ogg");
        std::fs::write(&ogg, []).unwrap();

        let wav = temp.path().join("Lincoln_D.wav");
        assert_eq!(
            KiraAudioBackend::resolve_music_path(wav.to_str().unwrap()),
            ogg
        );
    }

    #[test]
    fn wav_duration_basic() {
        let sample_rate: u32 = 44100;
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

        assert_eq!(wav_duration_ms(&wav), Some(1000));
    }

    #[test]
    fn wav_duration_invalid() {
        assert_eq!(wav_duration_ms(b"not a wav"), None);
        assert_eq!(wav_duration_ms(&[]), None);
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

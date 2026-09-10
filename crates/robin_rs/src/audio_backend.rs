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
mod resolver;
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
mod sample_cache;
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
use std::io::Cursor;

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
type MusicHandle = StreamingSoundHandle<FromFileError>;

/// Kira-backed audio backend.
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
pub struct KiraAudioBackend {
    manager: Option<AudioManager>,
    sound_dir: PathBuf,
    files: Arc<SbFileSystem>,
    /// Cached `StaticSoundData` (decoded audio) keyed by file path.
    /// kira's `StaticSoundData` is cheap to clone — clones share the
    /// underlying sample buffer.
    sample_cache: sample_cache::SampleCache,
    /// Per-channel handle tracking. `channel_idx -> currently-playing handle`.
    /// `None` means the slot is free.
    channels: Vec<Option<StaticSoundHandle>>,
    /// Music slot — independent from SFX channels.
    music_handle: Option<MusicHandle>,
    music_volume: u16,
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
    /// Legacy tool constructor capturing the current process reader. Production
    /// applications must use [`Self::new_with_files`].
    pub fn new(sound_dir: impl Into<PathBuf>, num_channels: u32) -> Result<Self, String> {
        Self::new_with_files(
            sound_dir,
            num_channels,
            Arc::new(SbFile::snapshot_legacy_file_system()),
        )
    }

    /// Bind playback to the same file authority as application preparation.
    pub fn new_with_files(
        sound_dir: impl Into<PathBuf>,
        num_channels: u32,
        files: Arc<SbFileSystem>,
    ) -> Result<Self, String> {
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
            files,
            // Match the browser's conservative retained-PCM policy. Active
            // voices may exceed this cache-only budget; logs measure residency.
            sample_cache: sample_cache::SampleCache::new(96 * 1024 * 1024),
            channels,
            music_handle: None,
            music_volume: 128,
            was_music_playing: false,
            jingle_channel: None,
            num_channels,
            start: web_time::Instant::now(),
            listener,
            spatial_tracks,
        })
    }

    fn resolve_path(&self, file_name: &str) -> Result<PathBuf, String> {
        resolver::resolve_sample(&self.sound_dir, file_name, &self.files)
    }

    fn load_sample(
        &mut self,
        file_name: &str,
    ) -> Result<StaticSoundData, crate::sound::PlaybackError> {
        let path = self
            .resolve_path(file_name)
            .map_err(crate::sound::PlaybackError::Asset)?;
        // Logical names stay identical across locale/VFS switches. Include
        // explicit authority generations rather than reopening a resolved
        // absolute path (which could bypass confined-reader policy).
        let cache_key = sample_cache_key(&self.files, &path);
        if let Some(s) = self.sample_cache.get(&cache_key) {
            return Ok(s);
        }
        let started = web_time::Instant::now();
        match load_static_sound(&self.files, &path) {
            Ok(data) => {
                tracing::debug!(path = %path.display(), decode_ms = started.elapsed().as_secs_f64() * 1000.0, decoded_bytes = std::mem::size_of_val(data.frames.as_ref()), "native cold audio load");
                self.sample_cache.insert(cache_key, data.clone());
                Ok(data)
            }
            Err(e) => Err(crate::sound::PlaybackError::Asset(format!(
                "{}: {e}",
                path.display()
            ))),
        }
    }

    fn find_free_channel(&self) -> Option<usize> {
        let channel = self
            .channels
            .iter()
            .position(|c| c.as_ref().is_none_or(is_handle_done));
        if channel.is_none() {
            tracing::debug!(
                capacity = self.num_channels,
                "kira playback rejected: channel capacity exhausted"
            );
        }
        channel
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
                .map_err(
                    |error| tracing::warn!(idx, %error, "kira spatial track allocation failed"),
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

    fn resolve_music_path(files: &SbFileSystem, path: &str) -> Result<PathBuf, String> {
        let resolve_one = |path: &str| {
            files
                .try_exists(path)
                .map(|exists| exists.then(|| PathBuf::from(path)))
                .map_err(|status| format!("music lookup failed for {path}: {status}"))
        };

        if let Some(resolved) = resolve_one(path)? {
            return Ok(resolved);
        }

        let raw = PathBuf::from(path);
        if raw
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
        {
            let ogg_path = raw.with_extension("ogg");
            if let Some(ogg_path) = ogg_path.to_str()
                && let Some(resolved) = resolve_one(ogg_path)?
            {
                return Ok(resolved);
            }
        } else if raw
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ogg"))
        {
            let wav_path = raw.with_extension("wav");
            if let Some(wav_path) = wav_path.to_str()
                && let Some(resolved) = resolve_one(wav_path)?
            {
                return Ok(resolved);
            }
        }

        let opus = raw.with_extension("opus");
        if let Some(resolved) = resolve_one(&opus.to_string_lossy())? {
            return Ok(resolved);
        }
        Err(format!("music asset not found: {}", raw.display()))
    }
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn sample_cache_key(files: &SbFileSystem, path: &Path) -> String {
    format!(
        "{:?}:{}:{}",
        files.locale_paths(),
        files.selection_snapshot().generation,
        path.to_string_lossy().replace('\\', "/")
    )
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn load_static_sound(files: &SbFileSystem, path: &Path) -> Result<StaticSoundData, String> {
    let bytes = files
        .read_all(&path.to_string_lossy())
        .map_err(|status| format!("audio reader failed for {}: {status}", path.display()))?;
    let bytes = repair_legacy_vorbis_comment(bytes)?;
    StaticSoundData::from_cursor(Cursor::new(bytes)).map_err(|error| error.to_string())
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn load_streaming_sound(
    files: &SbFileSystem,
    path: &Path,
) -> Result<StreamingSoundData<FromFileError>, String> {
    let bytes = files
        .read_all(&path.to_string_lossy())
        .map_err(|status| format!("audio reader failed for {}: {status}", path.display()))?;
    let bytes = repair_legacy_vorbis_comment(bytes)?;
    StreamingSoundData::from_cursor(Cursor::new(bytes)).map_err(|error| error.to_string())
}

// The original Sonic Foundry encoder wrote this bare encoder name as its sole
// user comment. libvorbis (original-code/mixer/music_ogg.c) accepted it, whereas
// Symphonia reports the missing KEY=VALUE separator on every music restart.
// Match the complete known packet, including vendor and framing byte: unknown
// malformed metadata must still reach the decoder's normal diagnostics.
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
const LEGACY_VORBIS_COMMENT: &[u8] = b"\x03vorbis\x20\0\0\0Xiphophorus libVorbis I 20010813\x01\0\0\0\x1e\0\0\0Sonic Foundry OggVorbis Beta 3\x01";

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn repair_legacy_vorbis_comment(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if !bytes.starts_with(b"OggS")
        || !bytes
            .windows(LEGACY_VORBIS_COMMENT.len())
            .any(|window| window == LEGACY_VORBIS_COMMENT)
    {
        return Ok(bytes);
    }
    let mut reader = ogg::PacketReader::new(Cursor::new(&bytes));
    let mut writer = ogg::PacketWriter::new(Vec::with_capacity(bytes.len() + 8));
    let mut repaired = false;
    while let Some(mut packet) = reader
        .read_packet()
        .map_err(|error| format!("reading legacy Vorbis metadata: {error}"))?
    {
        let end = if packet.last_in_stream() {
            ogg::PacketWriteEndInfo::EndStream
        } else if packet.last_in_page() {
            ogg::PacketWriteEndInfo::EndPage
        } else {
            ogg::PacketWriteEndInfo::NormalPacket
        };
        if packet.data == LEGACY_VORBIS_COMMENT {
            // Keep the vendor and encoder value intact; add the standard key
            // and update this comment's length. Ogg handles page CRCs/lacing.
            let comment_length_offset = 7 + 4 + 32 + 4;
            packet.data[comment_length_offset..comment_length_offset + 4]
                .copy_from_slice(&38u32.to_le_bytes());
            packet.data.splice(
                comment_length_offset + 4..comment_length_offset + 4,
                b"ENCODER=".iter().copied(),
            );
            repaired = true;
        }
        let serial = packet.stream_serial();
        let granule = packet.absgp_page();
        writer
            .write_packet(packet.data, serial, end, granule)
            .map_err(|error| format!("writing legacy Vorbis metadata: {error}"))?;
    }
    if repaired {
        tracing::debug!("normalized legacy Sonic Foundry Vorbis encoder comment");
        Ok(writer.into_inner())
    } else {
        Ok(bytes)
    }
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn is_handle_done(h: &StaticSoundHandle) -> bool {
    matches!(h.state(), kira::sound::PlaybackState::Stopped)
}

/// 0.0–1.0 linear amplitude → decibels (kira's native volume unit).
/// Zero amplitude uses Kira's silence value to avoid `-inf` from log10(0).
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn amplitude_to_decibels(amp: f32) -> kira::Decibels {
    if amp <= 0.0 {
        kira::Decibels::SILENCE
    } else {
        kira::Decibels::from(20.0 * amp.log10())
    }
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn prepare_sample(
    data: StaticSoundData,
    fraction: f32,
    looping: bool,
    volume: u16,
) -> StaticSoundData {
    let data = data
        .start_position(
            data.duration().as_secs_f64() * f64::from(crate::sound::playback_fraction(fraction)),
        )
        .volume(amplitude_to_decibels(crate::sound::channel_gain(volume)));
    if looping { data.loop_region(..) } else { data }
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
impl AudioBackend for KiraAudioBackend {
    fn play_sound(&mut self, file_name: &str, looping: bool) -> Option<i32> {
        self.play_sound_at(file_name, looping, 0.0)
    }

    fn play_sound_at(&mut self, file_name: &str, looping: bool, position: f32) -> Option<i32> {
        self.play_request(crate::sound::PlaybackRequest {
            asset: file_name,
            category: crate::sound::PlaybackCategory::Effect,
            looping,
            fraction: position,
            volume: 255,
            spatial_position: None,
        })
    }

    fn try_play_request(
        &mut self,
        request: crate::sound::PlaybackRequest<'_>,
    ) -> Result<i32, crate::sound::PlaybackError> {
        use crate::sound::PlaybackError;
        let idx = self.find_free_channel().ok_or(PlaybackError::Capacity)?;
        let data = self.load_sample(request.asset)?;
        let data = prepare_sample(data, request.fraction, request.looping, request.volume);
        let handle = if let Some(position) = request.spatial_position {
            self.ensure_spatial_track(idx, position)
                .ok_or_else(|| PlaybackError::Backend("spatial track allocation failed".into()))?
                .play(data)
                .map_err(|error| PlaybackError::Backend(error.to_string()))?
        } else {
            self.manager
                .as_mut()
                .expect("live Kira backend lost its AudioManager")
                .play(data)
                .map_err(|error| PlaybackError::Backend(error.to_string()))?
        };
        // A naturally finished jingle may have left its index available for
        // reuse. Its later free operation must not stop this unrelated voice.
        if self.jingle_channel == Some(idx) {
            self.jingle_channel = None;
        }
        self.channels[idx] = Some(handle);
        Ok(idx as i32)
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
        let v = crate::sound::channel_gain(volume);
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
        let full_path = match KiraAudioBackend::resolve_music_path(&self.files, path) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!("kira: resolve music '{path}': {error}");
                return false;
            }
        };
        // Keep streaming decode, but obtain encoded bytes through the reader:
        // a direct OS reopen would discard locale/VFS/confinement authority.
        let data = match load_streaming_sound(&self.files, &full_path) {
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
        let data = data.volume(amplitude_to_decibels(crate::sound::music_gain(
            self.music_volume,
        )));
        self.halt_music();
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
        self.music_volume = volume;
        let v = crate::sound::music_gain(volume);
        if let Some(h) = &mut self.music_handle {
            h.set_volume(amplitude_to_decibels(v), Tween::default());
        }
    }

    fn get_music_volume(&self) -> u16 {
        self.music_volume
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
        let full_path = match self.resolve_path(path) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!("kira: resolve jingle '{path}': {error}");
                return None;
            }
        };
        let data = match load_static_sound(&self.files, &full_path) {
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
        sample_pos: f32,
        world_pos: [f32; 3],
    ) -> Option<i32> {
        self.play_request(crate::sound::PlaybackRequest {
            asset: file_name,
            category: crate::sound::PlaybackCategory::Effect,
            looping,
            fraction: sample_pos,
            volume: 255,
            spatial_position: Some(world_pos),
        })
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
pub struct KiraAudioBackend;

#[cfg(not(feature = "audio"))]
impl KiraAudioBackend {
    pub fn new_with_files(
        _sound_dir: impl Into<PathBuf>,
        _num_channels: u32,
        _files: Arc<SbFileSystem>,
    ) -> Result<Self, String> {
        Err("audio feature disabled in this build".to_string())
    }

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
    use super::*;

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
        assert_ne!(repaired, original);
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
            repair_legacy_vorbis_comment(repaired.clone()).unwrap(),
            repaired
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
            repair_legacy_vorbis_comment(original.clone()).unwrap(),
            original
        );
        let wav = b"RIFF non-Ogg input".to_vec();
        assert_eq!(repair_legacy_vorbis_comment(wav.clone()).unwrap(), wav);
    }

    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    #[test]
    #[ignore = "requires ROBINHOOD_DATA_DIR pointing to original fullgame data"]
    fn legacy_vorbis_repair_preserves_decoded_original_music() {
        let root = std::env::var("ROBINHOOD_DATA_DIR").expect("set ROBINHOOD_DATA_DIR");
        for name in ["Menu.wav", "Cast_Fight.wav"] {
            let path = Path::new(&root).join("DATA/Musics").join(name);
            let bytes = std::fs::read(&path).expect("read original music");
            let repaired = repair_legacy_vorbis_comment(bytes.clone()).unwrap();
            assert_ne!(repaired, bytes, "fixture must contain legacy comment");
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
            KiraAudioBackend::resolve_music_path(&files, wav.to_str().unwrap()).unwrap(),
            ogg
        );
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
            let music =
                KiraAudioBackend::resolve_music_path(&files, "Data/Music/reader.wav").unwrap();
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

//! Native Kira implementation.
use super::*;

use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::{
    AudioManager, AudioManagerSettings, DefaultBackend, Tween,
    listener::ListenerHandle,
    sound::{
        FromFileError,
        static_sound::{StaticSoundData, StaticSoundHandle},
    },
    track::{SpatialTrackBuilder, SpatialTrackHandle},
};
use robin_util::asset_fs::AssetBytes;
use std::io::Cursor;

type MusicHandle = StreamingSoundHandle<FromFileError>;

/// Kira-backed audio backend.
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
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| format!("kira AudioManager init failed: {e}"))?;
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
        tracing::info!("kira audio initialised: {num_channels} channel slots");
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

    /// Get-or-create the spatial sub-track for a channel slot and update its
    /// position. Allocation failures retain the backend error for the caller.
    fn ensure_spatial_track(
        &mut self,
        idx: usize,
        pos: [f32; 3],
    ) -> Result<&mut SpatialTrackHandle, crate::sound::PlaybackError> {
        let mint_pos = mint::Vector3 {
            x: pos[0],
            y: pos[1],
            z: pos[2],
        };
        let slot = &mut self.spatial_tracks[idx];
        match slot {
            Some(track) => {
                track.set_position(mint_pos, Tween::default());
                Ok(track)
            }
            None => {
                let track = self
                    .manager
                    .as_mut()
                    .expect("live Kira backend lost its AudioManager")
                    .add_spatial_sub_track(
                        self.listener.id(),
                        mint_pos,
                        // SoundGeometry already supplies distance attenuation;
                        // the spatial track contributes panning only.
                        SpatialTrackBuilder::new().attenuation_function(None),
                    )
                    .map_err(|error| {
                        tracing::warn!(idx, %error, "kira spatial track allocation failed");
                        crate::sound::PlaybackError::Backend(format!(
                            "spatial track allocation failed: {error}"
                        ))
                    })?;
                Ok(slot.insert(track))
            }
        }
    }
}

pub(super) fn sample_cache_key(files: &SbFileSystem, path: &Path) -> String {
    format!(
        "{:?}:{}:{}",
        files.locale_paths(),
        files.asset_vfs().selection_generation(),
        path.to_string_lossy().replace('\\', "/")
    )
}

pub(super) fn load_static_sound(
    files: &SbFileSystem,
    path: &Path,
) -> Result<StaticSoundData, String> {
    StaticSoundData::from_cursor(read_audio_cursor(files, path)?).map_err(|error| error.to_string())
}

pub(super) fn load_streaming_sound(
    files: &SbFileSystem,
    path: &Path,
) -> Result<StreamingSoundData<FromFileError>, String> {
    StreamingSoundData::from_cursor(read_audio_cursor(files, path)?)
        .map_err(|error| error.to_string())
}

/// Both decoder modes use the supplied reader and the same legacy metadata repair.
pub(super) fn read_audio_cursor(
    files: &SbFileSystem,
    path: &Path,
) -> Result<Cursor<AssetBytes>, String> {
    let bytes = files
        .read_shared(&path.to_string_lossy())
        .map_err(|status| format!("audio reader failed for {}: {status}", path.display()))?;
    repair_legacy_vorbis_comment(bytes).map(Cursor::new)
}

// The original Sonic Foundry encoder wrote this bare encoder name as its sole
// user comment. libvorbis (original-code/mixer/music_ogg.c) accepted it, whereas
// Symphonia reports the missing KEY=VALUE separator on every music restart.
// Match the complete known packet, including vendor and framing byte: unknown
// malformed metadata must still reach the decoder's normal diagnostics.
pub(super) const LEGACY_VORBIS_COMMENT: &[u8] = b"\x03vorbis\x20\0\0\0Xiphophorus libVorbis I 20010813\x01\0\0\0\x1e\0\0\0Sonic Foundry OggVorbis Beta 3\x01";

pub(super) fn repair_legacy_vorbis_comment(
    bytes: impl Into<AssetBytes>,
) -> Result<AssetBytes, String> {
    let bytes = bytes.into();
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
        Ok(writer.into_inner().into())
    } else {
        Ok(bytes)
    }
}

pub(super) fn is_handle_done(h: &StaticSoundHandle) -> bool {
    matches!(h.state(), kira::sound::PlaybackState::Stopped)
}

/// 0.0–1.0 linear amplitude → decibels (kira's native volume unit).
/// Zero amplitude uses Kira's silence value to avoid `-inf` from log10(0).
pub(super) fn amplitude_to_decibels(amp: f32) -> kira::Decibels {
    if amp <= 0.0 {
        kira::Decibels::SILENCE
    } else {
        kira::Decibels::from(20.0 * amp.log10())
    }
}

pub(super) fn prepare_sample(
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
            self.ensure_spatial_track(idx, position)?
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
        let full_path = match resolver::resolve_music(&self.files, path) {
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

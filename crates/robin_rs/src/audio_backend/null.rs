//! Audio-disabled builds (`not(feature = "audio")`): no playback backend exists.
//!
//! `NullAudioBackend` is uninhabited. Construction always fails, so callers
//! hold `Option<PlatformAudioBackend>` = `None` and never call into a backend.
//! The [`AudioBackend`] impl below exists only so the type satisfies the same
//! signatures as the real backends; every method is statically unreachable
//! (`match *self {}`), so no method can fabricate ticks, channel counts or
//! "music played" answers. Sound bookkeeping in no-audio builds therefore has
//! exactly the provenance it had before: no backend call is ever made.
use super::*;
use crate::sound::AudioBackend;

pub enum NullAudioBackend {}

/// The backend type for this build configuration.
pub type PlatformAudioBackend = NullAudioBackend;

impl NullAudioBackend {
    /// Resolves the application's preparation files exactly like the real
    /// native backend, then reports that playback is disabled.
    pub fn new_for_application(
        application: &crate::host::ApplicationContext,
        sound_dir: impl Into<PathBuf>,
        num_channels: u32,
    ) -> Result<Self, String> {
        Self::new_with_files(
            sound_dir,
            num_channels,
            application.preparation_files()?.clone(),
        )
    }

    pub fn new_with_files(
        _sound_dir: impl Into<PathBuf>,
        _num_channels: u32,
        _files: Arc<SbFileSystem>,
    ) -> Result<Self, String> {
        Err("audio feature disabled in this build".to_string())
    }
}

impl AudioBackend for NullAudioBackend {
    fn play_sound(&mut self, _file_name: &str, _looping: bool) -> Option<i32> {
        match *self {}
    }
    fn play_sound_at(&mut self, _file_name: &str, _looping: bool, _position: f32) -> Option<i32> {
        match *self {}
    }
    fn halt_channel(&mut self, _channel: i32) {
        match *self {}
    }
    fn set_channel_volume(&mut self, _channel: i32, _volume: u16) {
        match *self {}
    }
    fn is_channel_playing(&self, _channel: i32) -> bool {
        match *self {}
    }
    fn pause_channels(&mut self, _channel: i32) {
        match *self {}
    }
    fn resume_channels(&mut self, _channel: i32) {
        match *self {}
    }
    fn play_music(&mut self, _path: &str, _looping: bool) -> bool {
        match *self {}
    }
    fn halt_music(&mut self) {
        match *self {}
    }
    fn pause_music(&mut self) {
        match *self {}
    }
    fn resume_music(&mut self) {
        match *self {}
    }
    fn set_music_volume(&mut self, _volume: u16) {
        match *self {}
    }
    fn get_music_volume(&self) -> u16 {
        match *self {}
    }
    fn take_music_finished(&mut self) -> bool {
        match *self {}
    }
    fn play_jingle(&mut self, _path: &str) -> Option<i32> {
        match *self {}
    }
    fn free_jingle(&mut self) {
        match *self {}
    }
    fn get_ticks(&self) -> u32 {
        match *self {}
    }
    fn num_channels(&self) -> u32 {
        match *self {}
    }
}

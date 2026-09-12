//! Explicit audio-disabled backend; construction reports disabled playback.
use super::*;

pub struct NullAudioBackend;

impl NullAudioBackend {
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

impl AudioBackend for NullAudioBackend {
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

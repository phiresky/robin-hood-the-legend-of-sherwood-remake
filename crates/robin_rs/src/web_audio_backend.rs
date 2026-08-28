//! Browser-native playback; decoded PCM remains browser-owned.

use crate::sound::AudioBackend;
use futures::{StreamExt as _, TryStreamExt as _};
use js_sys::Uint8Array;
use std::{cell::RefCell, collections::HashMap, path::PathBuf};
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AudioBuffer, AudioBufferSourceNode, AudioContext, GainNode, StereoPannerNode};

struct BrowserAudio {
    context: AudioContext,
    boot: HashMap<String, AudioBuffer>,
    mission: HashMap<String, AudioBuffer>,
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
                boot: HashMap::new(),
                mission: HashMap::new(),
            });
        }
        Ok(f(slot.as_mut().expect("initialized above")))
    })
}

fn aliases(path: &str) -> Vec<String> {
    let path = path
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_ascii_lowercase();
    let stem = path.rfind('.').map_or(path.as_str(), |dot| &path[..dot]);
    let mut bases = vec![stem.to_owned()];
    let data_relative = stem
        .rfind("/data/")
        .map(|offset| &stem[offset + "/data/".len()..])
        .or_else(|| stem.strip_prefix("data/"));
    if let Some(relative) = data_relative {
        bases.push(relative.to_owned());
    }
    let sound_relative_base = data_relative.unwrap_or(stem);
    for prefix in ["sounds/exclamations/", "sounds/"] {
        if let Some(relative) = sound_relative_base.strip_prefix(prefix) {
            bases.push(relative.to_owned());
        }
    }
    let mut result = Vec::new();
    for base in bases {
        for extension in ["opus", "wav", "ogg"] {
            result.push(format!("{base}.{extension}"));
        }
    }
    result.sort();
    result.dedup();
    result
}

fn insert(cache: &mut HashMap<String, AudioBuffer>, path: &str, buffer: &AudioBuffer) {
    for alias in aliases(path) {
        cache.insert(alias, buffer.clone());
    }
}

fn lookup(audio: &BrowserAudio, path: &str) -> Option<AudioBuffer> {
    aliases(path).into_iter().find_map(|key| {
        audio
            .mission
            .get(&key)
            .or_else(|| audio.boot.get(&key))
            .cloned()
    })
}

async fn decode(path: &str, bytes: &[u8]) -> Result<AudioBuffer, String> {
    let context = with_audio(|audio| audio.context.clone())?;
    // Copy only the encoded representation into JS; decodeAudioData owns PCM.
    let encoded = Uint8Array::new_with_length(bytes.len() as u32);
    encoded.copy_from(bytes);
    let promise = context
        .decode_audio_data(&encoded.buffer())
        .map_err(|e| format!("decode {path}: {e:?}"))?;
    JsFuture::from(promise)
        .await
        .map_err(|e| format!("decode {path}: {e:?}"))?
        .dyn_into::<AudioBuffer>()
        .map_err(|_| format!("decode {path}: result is not AudioBuffer"))
}

pub async fn preload_boot(path: &str, bytes: &[u8]) -> Result<(), String> {
    let buffer = decode(path, bytes).await?;
    with_audio(|audio| insert(&mut audio.boot, path, &buffer))
}

pub async fn replace_mission(entries: Vec<(String, &[u8])>) -> Result<(), String> {
    const DECODE_CONCURRENCY: usize = 8;
    let decoded = futures::stream::iter(entries.into_iter().map(|(path, bytes)| async move {
        decode(&path, bytes).await.map(|buffer| (path, buffer))
    }))
    .buffer_unordered(DECODE_CONCURRENCY)
    .try_collect::<Vec<_>>()
    .await?;
    let mut replacement = HashMap::new();
    for (path, buffer) in decoded {
        insert(&mut replacement, &path, &buffer);
    }
    with_audio(|audio| audio.mission = replacement)
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
            was_music_playing: false,
            music_volume: 128,
            jingle_channel: None,
            start: web_time::Instant::now(),
        })
    }
    fn buffer(&self, path: &str) -> Option<AudioBuffer> {
        let found = with_audio(|audio| lookup(audio, path)).ok().flatten();
        if found.is_none() {
            tracing::warn!(path, "Web Audio sample was not predecoded");
        }
        found
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
            return false;
        };
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
    fn halt_music(&mut self) {
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
    use super::aliases;
    #[test]
    fn opus_aliases_legacy_names_and_relative_speech() {
        let got = aliases("Data/Sounds/Exclamations/Expressions/Line.OPUS");
        assert!(got.contains(&"data/sounds/exclamations/expressions/line.wav".into()));
        assert!(got.contains(&"expressions/line.ogg".into()));
        assert!(got.contains(&"expressions/line.opus".into()));
    }

    #[test]
    fn shipped_sound_resolves_bare_legacy_request() {
        let inserted = aliases("Sounds/foo.opus");
        assert!(aliases("foo.wav").iter().any(|key| inserted.contains(key)));
    }

    #[test]
    fn shipped_music_resolves_absolute_legacy_request() {
        let inserted = aliases("Musics/Menu.opus");
        assert!(
            aliases("/install/Data/Musics/Menu.wav")
                .iter()
                .any(|key| inserted.contains(key))
        );
    }
}

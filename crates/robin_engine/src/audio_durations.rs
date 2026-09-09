//! Required, engine-owned English audio timing. Playback never reads these lengths.
use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::{
    LevelAudioAssets, SpeechTimingCatalog, SpeechTimingGroup, SpeechTimingVariant,
};
use crate::profiles::{CivilianType, ProfileManager};
use crate::sbfile::SbFileSystem;
use crate::sound::ExclamationGroup;
use crate::sound_cache::SoundCache;

pub const AUDIO_DURATIONS_PATH: &str = "Data/AudioDurations.json";
pub const AUDIO_DURATIONS_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioDurations {
    pub version: u32,
    pub locale: String,
    /// Lowercase paths relative to Data/Sounds, with original .wav/.ogg extensions.
    pub samples_ms: BTreeMap<String, u32>,
    /// English actor group IDs and ordered sample identities, independent of local actors.res.
    pub speech_groups: BTreeMap<u32, Vec<String>>,
}

impl AudioDurations {
    pub fn load(files: &SbFileSystem) -> Result<Self, String> {
        let bytes = files.read_all(AUDIO_DURATIONS_PATH).map_err(|status| {
            format!(
                "required core audio timing file {AUDIO_DURATIONS_PATH} is unavailable: {status}"
            )
        })?;
        Self::from_json(&bytes)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let table: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid {AUDIO_DURATIONS_PATH}: {error}"))?;
        if table.version != AUDIO_DURATIONS_VERSION || table.locale != "en-US" {
            return Err(format!(
                "unsupported core audio timing version/locale: {}/{}",
                table.version, table.locale
            ));
        }
        if table.samples_ms.is_empty() || table.speech_groups.is_empty() {
            return Err("core audio timing must contain samples and English speech groups".into());
        }
        for name in table.samples_ms.keys() {
            if sample_key(name)? != *name {
                return Err(format!("noncanonical audio timing sample identity: {name}"));
            }
        }
        for names in table.speech_groups.values() {
            for name in names {
                table.duration_ms(name)?;
            }
        }
        Ok(table)
    }

    pub fn duration_ms(&self, name: &str) -> Result<u32, String> {
        let key = sample_key(name)?;
        self.samples_ms
            .get(&key)
            .copied()
            .ok_or_else(|| format!("required English audio duration missing for {name} ({key})"))
    }

    pub fn duration_frames(&self, name: &str) -> Result<u32, String> {
        self.duration_ms(name)
            .map(|ms| (ms.saturating_add(39) / 40).max(1))
    }

    /// Publish the same mission timing closure in interactive and isolated verifier loads.
    pub fn populate(
        &self,
        audio: &mut LevelAudioAssets,
        profiles: &ProfileManager,
    ) -> Result<(), String> {
        let mut catalog = SpeechTimingCatalog::default();
        for &profile in &audio.required_exclamation_ids {
            let prefix = profile & 0xffff_0000;
            if !self
                .speech_groups
                .keys()
                .any(|id| id & 0xffff_0000 == prefix)
            {
                return Err(format!(
                    "English audio timing has no speech profile {profile:#010x}"
                ));
            }
            for (&id, names) in self.speech_groups.range(prefix..=prefix | 0xffff) {
                let variants = names
                    .iter()
                    .map(|name| {
                        Ok(SpeechTimingVariant {
                            sample_identity: name.clone(),
                            duration_frames: Some(self.duration_frames(name)?),
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                catalog
                    .groups
                    .insert(id, SpeechTimingGroup { gaps: 0, variants });
            }
        }
        let mut exclamations = BTreeMap::new();
        let mut add = |profile: u32, kinds: &[ExclamationGroup]| {
            for (&id, group) in &catalog.groups {
                if id & 0xffff_0000 != profile & 0xffff_0000 {
                    continue;
                }
                if let Some(duration) = group
                    .variants
                    .iter()
                    .filter_map(|v| v.duration_frames)
                    .max()
                {
                    for &kind in kinds {
                        exclamations.insert((kind, profile, id as u16), duration);
                    }
                }
            }
        };
        for profile in &profiles.characters {
            add(profile.exclamation_id, &[ExclamationGroup::Pc]);
        }
        for profile in &profiles.soldiers {
            add(
                profile.exclamation_id,
                &[ExclamationGroup::Civilian, ExclamationGroup::Soldier],
            );
            if profile.vip {
                add(profile.exclamation_id, &[ExclamationGroup::Vip]);
            }
        }
        for profile in &profiles.civilians {
            add(profile.exclamation_id, &[ExclamationGroup::Civilian]);
            if profile.civilian_type == CivilianType::Vip {
                add(profile.exclamation_id, &[ExclamationGroup::Vip]);
            }
        }
        let mut cache = SoundCache::new();
        cache.initialize_sound_source_cache(&audio.sound_source_required_ids);
        let sources = cache
            .source_cache
            .entries
            .iter()
            .map(|(&id, entry)| Ok((id, self.duration_frames(&entry.file_name)?)))
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        audio
            .publish_timing(Arc::new(exclamations), Arc::new(catalog), Arc::new(sources))
            .map_err(|error| error.to_string())
    }
}

pub fn sample_key(name: &str) -> Result<String, String> {
    let normalized = name.replace('\\', "/").to_ascii_lowercase();
    let relative = normalized
        .strip_prefix("data/sounds/")
        .unwrap_or(&normalized);
    if relative.is_empty()
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
    {
        return Err(format!("invalid audio timing sample identity: {name}"));
    }
    Ok(relative.to_owned())
}

/// Explicit variants keep their authored English duration; random speech uses
/// the longest English variant, independent of presentation RNG and locale.
pub fn speech_duration_frames(
    catalog: &SpeechTimingCatalog,
    id: u32,
    variant: i32,
) -> Result<u32, String> {
    let group = catalog
        .groups
        .get(&id)
        .ok_or_else(|| format!("English speech group missing: {id:#010x}"))?;
    let duration = match variant {
        -1 => group
            .variants
            .iter()
            .filter_map(|entry| entry.duration_frames)
            .max(),
        0.. => group
            .variants
            .get(variant as usize)
            .and_then(|entry| entry.duration_frames),
        _ => return Err(format!("invalid speech variant: {variant}")),
    };
    duration.ok_or_else(|| format!("English speech duration missing: {id:#010x} variant {variant}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localized_audio_and_missing_english_recordings_do_not_change_simulation_timing() {
        use robin_util::asset_fs::AssetVfs;
        let bytes = br#"{"version":1,"locale":"en-US","samples_ms":{"exclamations/a.wav":1001,"exclamations/b.wav":2000,"snd_001.wav":120},"speech_groups":{"65537":["exclamations/a.wav","exclamations/b.wav"]}}"#;
        let mut outputs = Vec::new();
        for local_audio in [vec![0; 1], vec![0; 5000], Vec::new()] {
            let vfs = Arc::new(AssetVfs::new());
            vfs.install_preloaded_asset(AUDIO_DURATIONS_PATH, bytes.to_vec())
                .unwrap();
            // Deliberately invalid local media and no English pack: timing
            // preparation must never open or decode these presentation bytes.
            vfs.install_preloaded_asset("Data/Sounds/Exclamations/a.wav", local_audio)
                .unwrap();
            let files = SbFileSystem::new(vfs);
            let table = AudioDurations::load(&files).unwrap();
            let mut audio = LevelAudioAssets::default();
            audio.required_exclamation_ids.insert(0x10000);
            audio.sound_source_required_ids.insert(1);
            table.populate(&mut audio, &ProfileManager::new()).unwrap();
            assert_eq!(audio.source_durations()[&1], 3);
            assert_eq!(
                speech_duration_frames(audio.speech_timing_catalog(), 65537, 0).unwrap(),
                26
            );
            assert_eq!(
                speech_duration_frames(audio.speech_timing_catalog(), 65537, -1).unwrap(),
                50
            );
            assert!(speech_duration_frames(audio.speech_timing_catalog(), 65537, 2).is_err());
            audio.validate_ranked_timing().unwrap();
            outputs.push(serde_json::to_vec(audio.speech_timing_catalog()).unwrap());
        }
        assert!(outputs.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn required_table_and_entries_fail_closed() {
        let files = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert!(
            AudioDurations::load(&files)
                .unwrap_err()
                .contains("required core audio")
        );
        let bytes = br#"{"version":1,"locale":"en-US","samples_ms":{"exclamations/a.wav":1001},"speech_groups":{"1":["exclamations/a.wav"]}}"#;
        let table = AudioDurations::from_json(bytes).unwrap();
        assert_eq!(
            table
                .duration_frames("Data\\Sounds\\Exclamations\\A.wav")
                .unwrap(),
            26
        );
        assert!(table.duration_ms("missing.wav").is_err());
        assert!(table.duration_ms("../a.wav").is_err());
        let mut invalid = table.clone();
        invalid.version += 1;
        assert!(AudioDurations::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
        invalid = table;
        invalid.samples_ms.clear();
        assert!(AudioDurations::from_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
}

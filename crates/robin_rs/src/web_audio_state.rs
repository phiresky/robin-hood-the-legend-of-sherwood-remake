//! Platform-neutral bookkeeping for browser audio requests.
//!
//! Keeping lifecycle decisions here makes the async Web Audio path testable
//! without a browser. The browser owner still holds the actual JS nodes and
//! decoded buffers.

use std::collections::HashSet;

const TRANSIENT_EFFECT_LIFETIME_MS: u64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlaybackKind {
    Effect,
    Voice,
    Loop,
    Jingle,
    Music,
}

impl PlaybackKind {
    pub(crate) fn for_sound(path: &str, looping: bool) -> Self {
        if looping {
            return Self::Loop;
        }
        let path = path.replace('\\', "/").to_ascii_lowercase();
        if path.contains("/exclamations/") || path.contains("/dialog") || path.contains("/speech") {
            Self::Voice
        } else {
            Self::Effect
        }
    }

    fn lifetime_ms(self) -> Option<u64> {
        match self {
            // A click/combat cue is misleading if it arrives long after the
            // action. Voice, loops, jingles and music remain pending until an
            // explicit halt, failure, backend drop, or mission transition.
            Self::Effect => Some(TRANSIENT_EFFECT_LIFETIME_MS),
            Self::Voice | Self::Loop | Self::Jingle | Self::Music => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingPlayback {
    pub(crate) id: u64,
    pub(crate) generation: u64,
    pub(crate) kind: PlaybackKind,
    pub(crate) path: String,
    pub(crate) looping: bool,
    pub(crate) fraction: f32,
    pub(crate) pan: f32,
    pub(crate) volume: f32,
    pub(crate) paused: bool,
    deadline_ms: Option<u64>,
}

impl PendingPlayback {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u64,
        generation: u64,
        kind: PlaybackKind,
        path: String,
        looping: bool,
        fraction: f32,
        pan: f32,
        volume: f32,
        now_ms: u64,
    ) -> Self {
        Self {
            id,
            generation,
            kind,
            path,
            looping,
            fraction: fraction.clamp(0.0, 0.999),
            pan: pan.clamp(-1.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
            paused: false,
            deadline_ms: kind
                .lifetime_ms()
                .map(|lifetime| now_ms.saturating_add(lifetime)),
        }
    }

    pub(crate) fn is_expired(&self, now_ms: u64) -> bool {
        self.deadline_ms.is_some_and(|deadline| now_ms >= deadline)
    }

    pub(crate) fn belongs_to(&self, id: u64, generation: u64) -> bool {
        self.id == id && self.generation == generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionDecision {
    Start,
    IgnoreStale,
    Expire,
    Fail,
}

pub(crate) fn completion_decision(
    pending: &PendingPlayback,
    id: u64,
    generation: u64,
    now_ms: u64,
    succeeded: bool,
) -> CompletionDecision {
    if !pending.belongs_to(id, generation) {
        CompletionDecision::IgnoreStale
    } else if pending.is_expired(now_ms) {
        CompletionDecision::Expire
    } else if succeeded {
        CompletionDecision::Start
    } else {
        CompletionDecision::Fail
    }
}

#[derive(Debug, Default)]
pub(crate) struct RequestIds(u64);

impl RequestIds {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1).max(1);
        self.0
    }
}

#[derive(Debug, Default)]
pub(crate) struct PlaybackGeneration(u64);

impl PlaybackGeneration {
    pub(crate) fn current(&self) -> u64 {
        self.0
    }

    pub(crate) fn advance(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
}

#[derive(Debug, Default)]
pub(crate) struct ContentDedup(HashSet<String>);

impl ContentDedup {
    /// Returns true only for the owner that must start the work. Later aliases
    /// join the existing content-keyed request/cache entry.
    pub(crate) fn claim(&mut self, key: String) -> bool {
        self.0.insert(key)
    }
}

#[derive(Debug)]
pub(crate) struct ProgressCounter {
    total: usize,
    completed: usize,
}

impl ProgressCounter {
    pub(crate) fn new(total: usize) -> Self {
        Self {
            total,
            completed: 0,
        }
    }

    pub(crate) fn total(&self) -> usize {
        self.total
    }

    pub(crate) fn completed(&self) -> usize {
        self.completed
    }

    pub(crate) fn advance(&mut self) -> usize {
        assert!(
            self.completed < self.total,
            "audio warmup progress advanced past its exact plan"
        );
        self.completed += 1;
        self.completed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum WarmPriority {
    Menu,
    Dialogue,
    Voice,
    Music,
    Ambience,
    Common,
}

pub(crate) fn warm_priority(path: &str, content_url: &str) -> WarmPriority {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    let content_url = content_url.to_ascii_lowercase();
    if path.contains("/menu/") || path.starts_with("musics/menu.") {
        WarmPriority::Menu
    } else if path.starts_with("text/")
        || content_url.contains("/dialogue-")
        || path.contains("/dialog")
        || path.contains("/speech")
    {
        WarmPriority::Dialogue
    } else if content_url.contains("/voice-") || path.contains("/exclamations/") {
        WarmPriority::Voice
    } else if path.starts_with("musics/") {
        WarmPriority::Music
    } else if path
        .rsplit('/')
        .next()
        .is_some_and(|name| name.starts_with("snd_"))
        || content_url.contains("/ambience-")
    {
        // Mission sound-source ids are indexed exactly by the converter.
        // Long members stay standalone and therefore lose their group name,
        // so the logical `snd_NNN` name is the authoritative classification.
        WarmPriority::Ambience
    } else {
        WarmPriority::Common
    }
}

pub(crate) fn should_decode_during_warmup(priority: WarmPriority, boot: bool) -> bool {
    boot || matches!(
        priority,
        WarmPriority::Dialogue | WarmPriority::Voice | WarmPriority::Music | WarmPriority::Ambience
    )
}

pub(crate) fn should_cache_decoded(decoded_bytes: u64, budget_bytes: u64) -> bool {
    decoded_bytes <= budget_bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_effect_preserves_playback_semantics_and_has_documented_expiry() {
        let pending = PendingPlayback::new(
            7,
            3,
            PlaybackKind::for_sound("Data/Sounds/hit.wav", false),
            "Data/Sounds/hit.wav".into(),
            false,
            0.25,
            0.75,
            0.4,
            1_000,
        );
        assert_eq!(pending.kind, PlaybackKind::Effect);
        assert_eq!(pending.fraction, 0.25);
        assert_eq!(pending.pan, 0.75);
        assert_eq!(pending.volume, 0.4);
        assert!(!pending.is_expired(10_999));
        assert!(pending.is_expired(11_000));
    }

    #[test]
    fn voice_jingle_music_and_loop_wait_for_explicit_lifecycle_events() {
        for (kind, path, looping) in [
            (PlaybackKind::Voice, "Data/Sounds/Exclamations/x.wav", false),
            (PlaybackKind::Jingle, "Data/Sounds/jingle_01.wav", false),
            (PlaybackKind::Music, "Data/Musics/mission.wav", true),
            (PlaybackKind::Loop, "Data/Sounds/snd_042.wav", true),
        ] {
            let pending = PendingPlayback::new(1, 9, kind, path.into(), looping, 0.0, 0.0, 1.0, 0);
            assert!(!pending.is_expired(u64::MAX));
        }
    }

    #[test]
    fn completion_requires_exact_request_generation_and_surfaces_failure() {
        let pending = PendingPlayback::new(
            12,
            5,
            PlaybackKind::Effect,
            "effect.wav".into(),
            false,
            0.0,
            0.0,
            1.0,
            0,
        );
        assert!(pending.belongs_to(12, 5));
        assert!(!pending.belongs_to(11, 5));
        assert!(!pending.belongs_to(12, 6));
        assert_eq!(
            completion_decision(&pending, 12, 5, 1, true),
            CompletionDecision::Start
        );
        assert_eq!(
            completion_decision(&pending, 12, 6, 1, true),
            CompletionDecision::IgnoreStale
        );
        assert_eq!(
            completion_decision(&pending, 12, 5, 1, false),
            CompletionDecision::Fail
        );
    }

    #[test]
    fn request_ids_do_not_return_plausible_zero_handles() {
        let mut ids = RequestIds::default();
        assert_eq!(ids.next(), 1);
        assert_eq!(ids.next(), 2);
    }

    #[test]
    fn generation_transition_invalidates_exact_old_completion() {
        let mut generations = PlaybackGeneration::default();
        let pending = PendingPlayback::new(
            1,
            generations.current(),
            PlaybackKind::Voice,
            "voice.wav".into(),
            false,
            0.0,
            0.0,
            1.0,
            0,
        );

        let next = generations.advance();
        assert_eq!(next, 1);
        assert_eq!(
            completion_decision(&pending, pending.id, next, 1, true),
            CompletionDecision::IgnoreStale
        );
    }

    #[test]
    fn content_loads_deduplicate_aliases_and_progress_is_exact() {
        let mut dedup = ContentDedup::default();
        assert!(dedup.claim("bundle#40".into()));
        assert!(!dedup.claim("bundle#40".into()));
        assert!(dedup.claim("bundle#80".into()));

        let mut progress = ProgressCounter::new(2);
        assert_eq!(progress.total(), 2);
        assert_eq!(progress.completed(), 0);
        assert_eq!(progress.advance(), 1);
        assert_eq!(progress.advance(), 2);
    }

    #[test]
    fn warmup_priority_and_pcm_policy_are_bounded() {
        let cases = [
            (
                "Sounds/Menu/click.opus",
                "audio/bundles/menu-a.bin",
                false,
                WarmPriority::Menu,
            ),
            (
                "Text/mission_line_001.opus",
                "audio/assets/large-standalone.opus",
                false,
                WarmPriority::Dialogue,
            ),
            (
                "Sounds/Exclamations/a.opus",
                "audio/bundles/shared-a.bin",
                false,
                WarmPriority::Voice,
            ),
            (
                "Musics/fight.opus",
                "audio/assets/hash.opus",
                true,
                WarmPriority::Music,
            ),
            (
                "Sounds/snd_099.opus",
                "audio/assets/hash.opus",
                true,
                WarmPriority::Ambience,
            ),
            (
                "Sounds/hit.opus",
                "audio/assets/large-common.opus",
                true,
                WarmPriority::Common,
            ),
        ];
        for (path, url, _standalone, expected) in cases {
            assert_eq!(warm_priority(path, url), expected);
        }
        assert!(should_decode_during_warmup(WarmPriority::Menu, true));
        assert!(should_decode_during_warmup(WarmPriority::Voice, false));
        assert!(!should_decode_during_warmup(WarmPriority::Common, false));
        assert!(should_cache_decoded(96, 96));
        assert!(!should_cache_decoded(97, 96));
    }
}

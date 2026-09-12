use super::*;

/// Host-only pending ownership; never overload mixer indices with lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum PendingChannel {
    Inaudible,
    Queued,
    Assigned(i32),
    Finished,
}

impl PendingChannel {
    pub(super) fn assigned(self) -> Option<i32> {
        match self {
            Self::Assigned(channel) => Some(channel),
            Self::Inaudible | Self::Queued | Self::Finished => None,
        }
    }
}

// ─── Channel tracking ───────────────────────────────────────────────

/// Identifies which cache sub-system a channel's sound comes from,
/// so we can update `playing` counts when the channel stops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum CacheKey {
    FxIndex(usize),
    CombatFx(u32),
    Source(u32),
    SpeechIndex(usize),
    Menu(u32),
}

/// Per-channel bookkeeping.
///
/// `actor_id` is the actor identifier used by [`SoundManager::stop_exclamation`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ChannelInfo {
    pub(super) sound_type: SoundType,
    pub(super) cache_key: Option<CacheKey>,
    /// For [`SoundType::Exclamation`]: actor identifier for stop lookups.
    pub(super) actor_id: Option<u32>,
}

impl Default for ChannelInfo {
    fn default() -> Self {
        Self {
            sound_type: SoundType::None,
            cache_key: None,
            actor_id: None,
        }
    }
}

// ─── Pending sound ──────────────────────────────────────────────────

/// A sound playing or waiting to play, tracked over time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PendingSoundInfo {
    pub(super) settings: SoundSettings,
    /// Host playback ownership, independent of logical sound lifetime.
    pub(super) channel: PendingChannel,
    /// Timestamp (ms) when the sound started.
    pub(super) start_time_ms: u32,
    /// Duration of the sample (ms). 0 = not yet determined.
    pub(super) length_ms: u32,
    /// For exclamations: actor identifier (for AI callback on finish).
    pub(super) actor_id: Option<u32>,
    /// For source sounds: index into the source manager.
    pub(super) source_index: Option<usize>,
    /// Speech variant for exclamation cache lookups.
    pub(super) speech_variant: Option<u32>,
}

/// A short FX queued to be played in the current frame's [`SoundManager::hourglass`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FxToPlay {
    pub(super) settings: SoundSettings,
    pub(super) params: PlayingParameters,
}

// ─── Cache entry info (extracted to avoid borrow conflicts) ─────────

/// Extracted cache entry metadata, owned (no borrow on the cache).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CacheEntryInfo {
    pub(super) file_name: String,
    pub(super) sample_length_ms: u32,
    pub(super) loop_sample: bool,
    pub(super) cache_key: Option<CacheKey>,
}

impl SoundManager {
    // ── Hourglass (main update) ──────────────────────────────────────

    /// Main per-frame sound update. Call once per game tick.
    ///
    /// Updates timers, plays pending sounds, manages music transitions,
    /// and updates cache TTLs.
    pub fn hourglass(
        &mut self,
        backend: &mut dyn AudioBackend,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        alert_status: AlertStatus,
        sources: &SoundSourceManager,
        pending_play_delayed_sources: &mut Vec<usize>,
    ) -> Vec<ResolvedHostExclamation> {
        if !self.persisted.active {
            return Vec::new();
        }
        if !self.runtime.pending_sounds.is_empty() {
            tracing::trace!(
                pending = self.runtime.pending_sounds.len(),
                "sound hourglass: processing pending"
            );
        }
        // Speech completion is simulation-side, but its schedule begins only
        // after Pass 1 below resolves the concrete sample and reports its
        // decoded duration to the engine. Mixer completion never mutates sim.

        // Play deferred jingle (queued from script commands)
        if let Some(jingle) = self.runtime.pending_jingle.take() {
            self.play_jingle(jingle, backend);
        }

        // Check for music/dialog finished callbacks
        if backend.take_music_finished() {
            if self.runtime.has_dialog {
                self.runtime.dialog_finished = true;
                self.runtime.stop_dialog = true;
            } else if self.runtime.has_mission_music {
                self.on_music_finished(alert_status);
            }
        }

        // Check jingle channel finished
        if self
            .runtime
            .jingle_channel
            .is_some_and(|channel| !backend.is_channel_playing(channel))
        {
            self.runtime.jingle_channel = None;
            self.runtime.stop_jingle = true;
        }

        // ── Start delayed sound sources the engine flagged ─────
        // Engine ticks the timer down inside `perform_hourglass`,
        // emits `SoundCommand::PlayDelayedSource(idx)` when it hits
        // zero, and immediately re-rolls the timer using `sim_rng`.
        // We just drain the queue and start playback. (Source timer
        // reset used to live here, driven by audio-backend playback
        // completion + a host RNG, which broke rollback determinism.)
        for idx in pending_play_delayed_sources.drain(..) {
            if !self.is_source_pending(idx)
                && sources.get(idx).is_some_and(|s| s.is_effectively_active())
            {
                self.start_sound_source_pending(idx, sources);
            }
        }

        // ── Update channel playing state ──
        self.update_all_channels_info(backend);

        // ── Handle deferred cleanup ──
        if self.runtime.stop_jingle {
            backend.free_jingle();
            self.runtime.stop_jingle = false;
        }
        if self.runtime.stop_dialog {
            backend.halt_music();
            self.runtime.has_dialog = false;
            self.runtime.stop_dialog = false;
        }

        // ── Decay music mode weights ──
        self.persisted.quiet_mode_weight = self.persisted.quiet_mode_weight.saturating_sub(1);
        self.persisted.alert_mode_weight = self.persisted.alert_mode_weight.saturating_sub(1);
        self.persisted.fight_mode_weight = self.persisted.fight_mode_weight.saturating_sub(1);

        // ── Music loop selection ──
        if self.persisted.load_music {
            let mut mode = MusicMode::Quiet;
            let mut weight = self.persisted.quiet_mode_weight;
            if self.persisted.alert_mode_weight > weight {
                mode = MusicMode::Alert;
                weight = self.persisted.alert_mode_weight;
            }
            if self.persisted.fight_mode_weight > weight {
                mode = MusicMode::Fight;
            }
            self.persisted.music_mode = mode;

            // Choose a random loop index
            let pool_size = match mode {
                MusicMode::Quiet => self.persisted.sound_cache.quiet_music_pool.len(),
                MusicMode::Alert => self.persisted.sound_cache.alert_music_pool.len(),
                MusicMode::Fight => self.persisted.sound_cache.fight_music_pool.len(),
            };
            if pool_size > 0 {
                self.persisted.loop_index = rng(pool_size as u32) as i16;
            }

            self.persisted.load_music = false;
            self.persisted.start_music = true;
        }

        if self.persisted.start_music {
            if let Some(name) = self.select_music_loop() {
                let path = format!("{}/{}.wav", self.persisted.music_directory, name);
                if backend.play_music(&path, false) {
                    self.runtime.has_mission_music = true;
                    // play_music replaces any previously playing stream
                    // (e.g., the menu music carried over from the loading
                    // screen). Clear the flag now that the new mission
                    // track has taken over.
                    self.runtime.has_menu_music = false;
                }
            }
            self.persisted.start_music = false;
            self.runtime.update_music = true;
        }

        if self.runtime.update_music {
            backend.set_music_volume(self.persisted.geometry_engine.get_volume_for_music(true) / 2);
            self.runtime.update_music = false;
        }

        // ── Play queued short FX ──
        let fx_list = std::mem::take(&mut self.runtime.fx_to_play);
        for fx in &fx_list {
            self.play_sound_now(&fx.settings, &fx.params, backend, loader, rng, sources);
        }

        // ── Process pending sounds ──
        let resolved_exclamations = self.process_pending_sounds(backend, loader, rng, sources);

        // ── Update cache TTLs ──
        self.persisted.sound_cache.update_cache_state();
        resolved_exclamations
    }

    /// Process all pending sounds: expire finished ones, update params, play new.
    pub(super) fn process_pending_sounds(
        &mut self,
        backend: &mut dyn AudioBackend,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        sources: &SoundSourceManager,
    ) -> Vec<ResolvedHostExclamation> {
        let now = backend.get_ticks();
        let mut resolved_exclamations = Vec::new();

        // ── Pass 1: initialize lengths and remove finished sounds ──
        self.runtime.pending_sounds.retain_mut(|pending| {
            if pending.length_ms == 0 {
                pending.start_time_ms = now;
                let entry = Self::get_entry_info(
                    &mut self.persisted.sound_cache,
                    &pending.settings,
                    pending.speech_variant,
                    false,
                    loader,
                    rng,
                    sources,
                );
                let length = entry.as_ref().map_or(0, |info| info.sample_length_ms);
                pending.length_ms = length;
                if pending.settings.sound_type == SoundType::Exclamation {
                    if let Some(actor_id) = pending.actor_id {
                        resolved_exclamations.push(ResolvedHostExclamation {
                            actor_id,
                            identifier: pending.settings.identifier,
                            exclamation_id: pending.settings.identifier as u16,
                            length_ms: length,
                        });
                    }
                }
            }

            let elapsed = if pending.length_ms == 0 {
                0 // Unavailable samples expire immediately.
            } else if pending.settings.sound_type == SoundType::Source
                && pending
                    .source_index
                    .and_then(|idx| sources.get(idx))
                    .is_some_and(|source| source.source_kind == SoundSourceKind::Looped)
            {
                0 // Looping sources never expire.
            } else {
                time_elapsed(pending.start_time_ms, now)
            };
            let finished = elapsed >= pending.length_ms;
            if finished && pending.settings.sound_type == SoundType::Exclamation {
                tracing::trace!(
                    actor_id = ?pending.actor_id,
                    identifier = pending.settings.identifier,
                    length_ms = pending.length_ms,
                    "exclamation expired in Pass 1 (length_ms=0 means sample missing)"
                );
            }
            !finished
        });

        // Handle finished sounds. Channel cleanup stays host-side
        // (audio-backend state, not in the rollback hash); the kind-specific
        // sim transition for finished `Source`-type sounds (Single
        // `active = false`, Volatile `sources.delete`) now fires from
        // the sim-side drain in `Engine::perform_hourglass` using
        // `SoundSimState::playing_sources` scheduled at activation
        // time. Exclamation finish callbacks are likewise sim-side
        // from `playing_exclamations`.
        // We erase pending sounds on logical completion, but do not
        // halt their mixer channels here. The channel either ends
        // naturally or is stopped by explicit stop/deactivate paths.

        // ── Pass 2: update params if listen point changed ──
        if self.runtime.update_pending_sounds {
            for i in 0..self.runtime.pending_sounds.len() {
                if self.runtime.pending_sounds[i].channel == PendingChannel::Finished {
                    continue;
                }
                let settings = &self.runtime.pending_sounds[i].settings;
                let low_priority = settings.sound_type == SoundType::Fx
                    && self
                        .persisted
                        .sound_cache
                        .is_material_fx(settings.identifier);

                if let Some(mut params) = self
                    .persisted
                    .geometry_engine
                    .get_logical_playing_params(settings, low_priority)
                {
                    let channel = self.runtime.pending_sounds[i].channel;
                    match channel {
                        PendingChannel::Inaudible => {
                            self.runtime.pending_sounds[i].channel = PendingChannel::Queued;
                        }
                        PendingChannel::Queued | PendingChannel::Finished => {}
                        PendingChannel::Assigned(ch) => {
                            if self.persisted.use_3d_sound {
                                SoundGeometry::get_3d_playing_params(&mut params);
                                backend.set_channel_position_3d(ch, params.position_3d);
                            } else {
                                SoundGeometry::get_2d_playing_params(&mut params);
                            }
                            backend.set_channel_volume(ch, params.volume_2d);
                        }
                    }
                } else {
                    let channel = self.runtime.pending_sounds[i].channel;
                    if let Some(channel) = channel.assigned() {
                        backend.halt_channel(channel);
                    }
                    self.runtime.pending_sounds[i].channel = PendingChannel::Inaudible;
                }
            }
            self.runtime.update_pending_sounds = false;
        }

        // ── Pass 3: play queued sounds ──
        let now = backend.get_ticks();
        for i in 0..self.runtime.pending_sounds.len() {
            if self.runtime.pending_sounds[i].channel != PendingChannel::Queued {
                continue;
            }

            let settings = &self.runtime.pending_sounds[i].settings;
            let sound_type = settings.sound_type;
            let identifier = settings.identifier;
            let low_priority = sound_type == SoundType::Fx
                && self.persisted.sound_cache.is_material_fx(identifier);

            let Some(params) = self
                .persisted
                .geometry_engine
                .get_logical_playing_params(settings, low_priority)
            else {
                if sound_type == SoundType::Exclamation {
                    tracing::trace!(
                        actor_id = ?self.runtime.pending_sounds[i].actor_id,
                        identifier = identifier,
                        "exclamation skipped: no logical playing params"
                    );
                }
                self.runtime.pending_sounds[i].channel = PendingChannel::Inaudible;
                continue;
            };

            let speech_variant = self.runtime.pending_sounds[i].speech_variant;
            let entry_info = Self::get_entry_info(
                &mut self.persisted.sound_cache,
                settings,
                speech_variant,
                true,
                loader,
                rng,
                sources,
            );
            let Some(info) = entry_info else {
                if sound_type == SoundType::Exclamation {
                    tracing::trace!(
                        actor_id = ?self.runtime.pending_sounds[i].actor_id,
                        identifier = identifier,
                        "exclamation skipped: no entry_info (sample file missing?)"
                    );
                }
                continue;
            };

            // Compute play position
            let start = self.runtime.pending_sounds[i].start_time_ms;
            let length = self.runtime.pending_sounds[i].length_ms;
            let elapsed = time_elapsed(start, now);
            let mut position = if length > 0 {
                elapsed as f32 / length as f32
            } else {
                0.0
            };

            // Handle looping
            if self.runtime.pending_sounds[i].settings.sound_type == SoundType::Source
                && let Some(idx) = self.runtime.pending_sounds[i].source_index
                && sources
                    .get(idx)
                    .is_some_and(|s| s.source_kind == SoundSourceKind::Looped)
            {
                position -= position.floor();
            }
            position = position.clamp(0.0, 0.999);

            let mut hw_params = params;
            if self.persisted.use_3d_sound {
                SoundGeometry::get_3d_playing_params(&mut hw_params);
            } else {
                SoundGeometry::get_2d_playing_params(&mut hw_params);
            }

            let play_result = backend.play_request(PlaybackRequest {
                asset: &info.file_name,
                category: playback_category(sound_type),
                looping: info.loop_sample,
                fraction: position,
                volume: hw_params.volume_2d,
                spatial_position: self.persisted.use_3d_sound.then_some(hw_params.position_3d),
            });

            if let Some(channel) = play_result {
                let actor_id = self.runtime.pending_sounds[i].actor_id;

                self.update_channel_info(channel, sound_type, info.cache_key, actor_id);

                self.runtime.pending_sounds[i].channel = PendingChannel::Assigned(channel);
                if sound_type == SoundType::Exclamation {
                    tracing::trace!(
                        actor_id = ?actor_id,
                        file = info.file_name.as_str(),
                        channel,
                        volume_2d = hw_params.volume_2d,
                        "exclamation playing"
                    );
                }
            } else if sound_type == SoundType::Exclamation {
                tracing::trace!(
                    actor_id = ?self.runtime.pending_sounds[i].actor_id,
                    file = info.file_name.as_str(),
                    "exclamation skipped: backend.play_sound_at returned None"
                );
            }
        }
        resolved_exclamations
    }

    /// Add a sound source to the pending sounds list.
    pub(super) fn start_sound_source_pending(
        &mut self,
        source_index: usize,
        sources: &SoundSourceManager,
    ) {
        let src = match sources.get(source_index) {
            Some(s) => s,
            None => return,
        };

        let settings = SoundSettings {
            sound_type: SoundType::Source,
            position: src.shape.first().copied().unwrap_or(MapPoint::ZERO),
            identifier: src.id,
            source: SoundSettingsSource::SoundSource {
                info: src.to_source_info(),
                speech_variant: -1,
            },
        };

        self.runtime.pending_sounds.push(PendingSoundInfo {
            settings,
            channel: PendingChannel::Queued,
            start_time_ms: 0,
            length_ms: 0,
            actor_id: None,
            source_index: Some(source_index),
            speech_variant: None,
        });
    }

    /// Check if a sound source is already in the pending list.
    pub(super) fn is_source_pending(&self, source_index: usize) -> bool {
        self.runtime.pending_sounds.iter().any(|p| {
            p.settings.sound_type == SoundType::Source && p.source_index == Some(source_index)
        })
    }

    /// Play a sound immediately using pre-computed playing params.
    pub(super) fn play_sound_now(
        &mut self,
        settings: &SoundSettings,
        params: &PlayingParameters,
        backend: &mut dyn AudioBackend,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        sources: &SoundSourceManager,
    ) {
        let info = match Self::get_entry_info(
            &mut self.persisted.sound_cache,
            settings,
            None,
            true,
            loader,
            rng,
            sources,
        ) {
            Some(i) => i,
            None => return,
        };

        let mut hw_params = params.clone();
        if self.persisted.use_3d_sound {
            SoundGeometry::get_3d_playing_params(&mut hw_params);
        } else {
            SoundGeometry::get_2d_playing_params(&mut hw_params);
        }
        let play_result = backend.play_request(PlaybackRequest {
            asset: &info.file_name,
            category: playback_category(settings.sound_type),
            looping: info.loop_sample,
            fraction: 0.0,
            volume: hw_params.volume_2d,
            spatial_position: self.persisted.use_3d_sound.then_some(hw_params.position_3d),
        });

        let Some(channel) = play_result else {
            return;
        };

        self.update_channel_info(channel, settings.sound_type, info.cache_key, None);
    }

    /// Stop a channel and clear its bookkeeping.
    pub(super) fn stop_channel(&mut self, channel: i32, backend: &mut dyn AudioBackend) {
        if channel >= 0 {
            backend.halt_channel(channel);
            self.update_channel_info(channel, SoundType::None, None, None);
        }
    }

    /// Update channel bookkeeping (playing counts, etc.).
    pub(super) fn update_channel_info(
        &mut self,
        channel: i32,
        sound_type: SoundType,
        cache_key: Option<CacheKey>,
        actor_id: Option<u32>,
    ) {
        let idx = channel as usize;
        if idx >= self.runtime.channel_info.len() {
            return;
        }

        if self.runtime.jingle_channel == Some(channel) && sound_type != SoundType::Jingle {
            self.runtime.jingle_channel = None;
            self.runtime.stop_jingle = true;
        }
        // A mixer slot may be reused before a logical duration expires (for
        // example, localized speech is shorter than canonical sim timing).
        // The old pending sound must no longer move, stop, or resume that slot.
        for pending in &mut self.runtime.pending_sounds {
            if pending.channel == PendingChannel::Assigned(channel) {
                pending.channel = PendingChannel::Finished;
            }
        }

        // Decrement playing count on the old entry
        if let Some(key) = self.runtime.channel_info[idx].cache_key.clone() {
            self.adjust_cache_playing(&key, false);
        }

        self.runtime.channel_info[idx] = ChannelInfo {
            sound_type,
            cache_key: cache_key.clone(),
            actor_id,
        };

        // Increment playing count on the new entry
        if let Some(ref key) = cache_key {
            self.adjust_cache_playing(key, true);
        }
    }

    /// Update all channels: clear info for channels that stopped playing.
    pub(super) fn update_all_channels_info(&mut self, backend: &dyn AudioBackend) {
        for i in 0..self.persisted.num_channels as usize {
            if i < self.runtime.channel_info.len()
                && self.runtime.channel_info[i].sound_type != SoundType::None
                && !backend.is_channel_playing(i as i32)
            {
                if let Some(key) = self.runtime.channel_info[i].cache_key.clone() {
                    self.adjust_cache_playing(&key, false);
                }
                self.runtime.channel_info[i] = ChannelInfo::default();
            }
        }
    }

    /// Increment or decrement the playing count on a cache entry.
    pub(super) fn adjust_cache_playing(&mut self, key: &CacheKey, increment: bool) {
        let delta = |entry: &mut robin_engine::sound_cache::SoundCacheEntry| {
            if increment {
                entry.playing += 1;
            } else {
                entry.playing = entry.playing.saturating_sub(1);
            }
        };

        match key {
            CacheKey::FxIndex(idx) => {
                if let Some(entry) = self.persisted.sound_cache.fx_cache.entries.get_mut(*idx) {
                    delta(entry);
                }
            }
            CacheKey::CombatFx(id) => {
                if let Some(entry) = self
                    .persisted
                    .sound_cache
                    .combat_fx_cache
                    .entries
                    .get_mut(id)
                {
                    delta(entry);
                }
            }
            CacheKey::Source(id) => {
                if let Some(entry) = self.persisted.sound_cache.source_cache.entries.get_mut(id) {
                    delta(entry);
                }
            }
            CacheKey::SpeechIndex(idx) => {
                if let Some(entry) = self
                    .persisted
                    .sound_cache
                    .speech_cache
                    .entries
                    .get_mut(*idx)
                {
                    delta(entry);
                }
            }
            CacheKey::Menu(id) => {
                if let Some(entry) = self.persisted.sound_cache.menu_cache.entries.get_mut(id) {
                    delta(entry);
                }
            }
        }
    }

    // ── Cache entry info extraction ──────────────────────────────────

    /// Extract cache entry info (file name, length, etc.) without holding a
    /// borrow on the cache. Calls the appropriate cache getter internally.
    pub(super) fn get_entry_info(
        cache: &mut SoundCache,
        settings: &SoundSettings,
        speech_variant: Option<u32>,
        sample_present: bool,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        sources: &SoundSourceManager,
    ) -> Option<CacheEntryInfo> {
        let (entry, cache_key): (&SoundCacheEntry, CacheKey) = match settings.sound_type {
            SoundType::Source => {
                let looping = sources
                    .find_by_sample_id(settings.identifier)
                    .and_then(|idx| sources.get(idx))
                    .is_some_and(|s| s.source_kind == SoundSourceKind::Looped);

                let entry = cache.get_source_sample(
                    sample_present,
                    settings.identifier,
                    looping,
                    loader,
                )?;
                (entry, CacheKey::Source(settings.identifier))
            }
            SoundType::Fx | SoundType::MenuFx => {
                let material = match &settings.source {
                    SoundSettingsSource::Position { material: m } => material_from_u8(*m),
                    _ => None,
                };
                let idx = cache.get_fx_sample(
                    sample_present,
                    settings.identifier,
                    material,
                    loader,
                    rng,
                )?;
                let entry = &cache.fx_cache.entries[idx];
                (entry, CacheKey::FxIndex(idx))
            }
            SoundType::CombatFx => {
                let entry =
                    cache.get_combat_fx_sample(sample_present, settings.identifier, loader)?;
                (entry, CacheKey::CombatFx(settings.identifier))
            }
            SoundType::Exclamation => {
                let idx = cache.get_exclamation_sample(
                    sample_present,
                    settings.identifier,
                    speech_variant,
                    loader,
                    rng,
                )?;
                let entry = &cache.speech_cache.entries[idx];
                (entry, CacheKey::SpeechIndex(idx))
            }
            _ => return None,
        };
        if sample_present && !entry.is_loaded() {
            return None;
        }
        Some(CacheEntryInfo {
            file_name: entry.file_name.clone(),
            sample_length_ms: entry.sample_length_ms,
            loop_sample: entry.loop_sample,
            cache_key: Some(cache_key),
        })
    }
}

use super::*;

impl SoundManager {
    // ── Exclamation management ───────────────────────────────────────

    /// Play an exclamation (character speech).
    pub fn play_exclamation(
        &mut self,
        group: ExclamationGroup,
        profile_id: u32,
        exclamation_id: u16,
        variant: i32,
        position: MapPoint,
        actor_id: Option<u32>,
    ) {
        tracing::trace!(
            ?group,
            profile_id,
            exclamation_id,
            variant,
            ?actor_id,
            active = self.persisted.active,
            "play_exclamation"
        );
        if !self.persisted.active {
            return;
        }

        let pt = if group == ExclamationGroup::Pc {
            let mut lp = self.persisted.geometry_engine.listen_point();
            if self.persisted.use_3d_sound {
                lp.x -= 20.0;
                lp.y -= 20.0;
            }
            lp
        } else {
            position
        };

        let excl_id = (profile_id & 0xFFFF_0000) | exclamation_id as u32;
        let speech_variant = if variant == EXCLAMATION_VARIANT_NONE {
            None
        } else {
            Some(variant as u32)
        };

        let settings = SoundSettings {
            sound_type: SoundType::Exclamation,
            position: pt,
            identifier: excl_id,
            source: SoundSettingsSource::Position { material: 0 },
        };

        self.runtime.pending_sounds.push(PendingSoundInfo {
            settings,
            channel: PendingChannel::Queued,
            start_time_ms: 0,
            length_ms: 0,
            actor_id,
            source_index: None,
            speech_variant,
        });
    }

    /// Stop the currently playing exclamation channel without dropping pending speech.
    pub fn stop_exclamation_channel_only(&mut self, actor_id: u32, backend: &mut dyn AudioBackend) {
        if !self.persisted.active {
            return;
        }

        for i in 0..self.persisted.num_channels as usize {
            if self.runtime.channel_info.get(i).is_some_and(|c| {
                c.sound_type == SoundType::Exclamation && c.actor_id == Some(actor_id)
            }) {
                self.stop_channel(i as i32, backend);
            }
        }
    }

    /// Drop queued exclamations for an actor without touching the currently playing channel.
    pub fn drop_pending_exclamations(&mut self, actor_id: u32) {
        self.runtime.pending_sounds.retain(|p| {
            !(p.settings.sound_type == SoundType::Exclamation && p.actor_id == Some(actor_id))
        });
    }

    /// Stop the actor's audible exclamation and drop its queued speech.
    pub fn stop_exclamation(&mut self, actor_id: u32, backend: &mut dyn AudioBackend) {
        self.stop_exclamation_channel_only(actor_id, backend);
        self.drop_pending_exclamations(actor_id);
    }
}

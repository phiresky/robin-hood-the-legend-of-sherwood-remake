//! Deterministic timed-mission and runtime-ambience clocks.
//!
//! The Original only selects ambience during level loading. Rust-authored
//! missions may add an active-play clock and ordered ambience cues; all state
//! in this module is serialized and hashed with `MissionState`.

use super::{
    Ambiance, AmbienceRuntimeCue, AmbienceScheduleRuntime, AmbienceTransitionRuntime, EngineInner,
    LevelAssets, MissionCountdownStatus, MissionRuntimeFeatures, TimedMissionRuntime,
};
use crate::level_data::LoadedLevel;

pub(crate) const ACTIVE_TICKS_PER_SECOND: u32 = 25;

fn ticks(seconds: u32) -> u32 {
    seconds
        .checked_mul(ACTIVE_TICKS_PER_SECOND)
        .expect("validated mission seconds overflowed while compiling runtime features")
}

pub(crate) fn ambiance_color(ambiance: Ambiance) -> u16 {
    let (r, g, b) = ambiance.night_color_rgb();
    robin_util::color::rgb565(r, g, b)
}

/// Integer RGB565 interpolation keeps crossfades bit-identical on every peer.
fn blend_rgb565(from: u16, to: u16, elapsed: u32, duration: u32) -> u16 {
    if duration == 0 || elapsed >= duration {
        return to;
    }
    let blend = |a: u32, b: u32| -> u16 {
        (((a * (duration - elapsed)) + (b * elapsed) + duration / 2) / duration) as u16
    };
    let r = blend(u32::from((from >> 11) & 0x1f), u32::from((to >> 11) & 0x1f));
    let g = blend(u32::from((from >> 5) & 0x3f), u32::from((to >> 5) & 0x3f));
    let b = blend(u32::from(from & 0x1f), u32::from(to & 0x1f));
    (r << 11) | (g << 5) | b
}

impl EngineInner {
    /// Compile editable authoring data before ambience-sensitive assets load.
    pub(super) fn initialize_mission_runtime_features(&mut self, loaded: &LoadedLevel) {
        let initial = Ambiance::from_raw(loaded.mission.header.ambiance);
        let timed_mission =
            loaded
                .mission
                .timed_mission
                .as_ref()
                .map(|definition| TimedMissionRuntime {
                    limit_ticks: ticks(definition.limit_seconds),
                    warning_ticks: ticks(definition.warning_seconds),
                    countdown_mode: definition.countdown,
                    elapsed_ticks: 0,
                    expired: false,
                });
        let cues: Vec<_> = loaded
            .mission
            .ambience_schedule
            .iter()
            .map(|cue| AmbienceRuntimeCue {
                at_tick: ticks(cue.at_seconds),
                ambiance: cue.ambiance,
                transition_ticks: ticks(cue.transition_seconds),
            })
            .collect();
        let ambience_schedule = (!cues.is_empty()).then_some(AmbienceScheduleRuntime {
            initial_ambiance: initial,
            current_ambiance: initial,
            elapsed_ticks: 0,
            cues,
            next_cue: 0,
            transition: None,
        });

        self.mission_domain.state.runtime_features = MissionRuntimeFeatures {
            active_elapsed_ticks: 0,
            timed_mission,
            ambience_schedule,
        };
        self.world.weather.ambiance = initial;
        self.world.weather.night_color = ambiance_color(initial);
        self.ai.standard_view_polygon_radius = initial.default_view_polygon_radius();

        // A cue at zero defines the level's effective load ambience. Apply it
        // before backgrounds, light sectors, sound sources and sprites load.
        self.apply_due_ambience_cues(None);
        self.update_ambience_crossfade();
    }

    fn update_ambience_crossfade(&mut self) {
        let Some(schedule) = self
            .mission_domain
            .state
            .runtime_features
            .ambience_schedule
            .as_mut()
        else {
            return;
        };
        let Some(transition) = schedule.transition.as_ref() else {
            return;
        };
        let elapsed = schedule
            .elapsed_ticks
            .saturating_sub(transition.started_at_tick);
        self.world.weather.night_color = blend_rgb565(
            transition.from_color,
            transition.to_color,
            elapsed,
            transition.duration_ticks,
        );
        if elapsed >= transition.duration_ticks {
            schedule.transition = None;
        }
    }

    fn apply_due_ambience_cues(&mut self, assets: Option<&LevelAssets>) {
        loop {
            let cue = {
                let Some(schedule) = self
                    .mission_domain
                    .state
                    .runtime_features
                    .ambience_schedule
                    .as_ref()
                else {
                    return;
                };
                let Some(cue) = schedule.cues.get(schedule.next_cue as usize) else {
                    return;
                };
                if cue.at_tick > schedule.elapsed_ticks {
                    return;
                }
                cue.clone()
            };

            let from_color = self.world.weather.night_color;
            let to_color = ambiance_color(cue.ambiance);
            let schedule = self
                .mission_domain
                .state
                .runtime_features
                .ambience_schedule
                .as_mut()
                .expect("ambience schedule disappeared while applying its cue");
            schedule.next_cue += 1;
            schedule.current_ambiance = cue.ambiance;
            schedule.transition = (cue.transition_ticks > 0).then_some(AmbienceTransitionRuntime {
                started_at_tick: schedule.elapsed_ticks,
                duration_ticks: cue.transition_ticks,
                from_color,
                to_color,
            });

            self.world.weather.ambiance = cue.ambiance;
            self.world.weather.night_color = if cue.transition_ticks == 0 {
                to_color
            } else {
                from_color
            };
            self.ai.standard_view_polygon_radius = cue.ambiance.default_view_polygon_radius();

            if let Some(assets) = assets {
                let ambiance_mask = cue.ambiance.to_bitmask();
                for &(sector, mask) in assets.ambience_shadow_sectors.iter() {
                    self.world
                        .fast_grid_mut()
                        .set_sector_active(sector.get(), mask & ambiance_mask != 0);
                }
                for index in 0..self.feedback.sound_sim.sources.num_sources() {
                    if let Some(source) = self.feedback.sound_sim.sources.get_mut(index) {
                        source.ambience_enabled = source.ambiences & ambiance_mask != 0;
                    }
                }
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::RefreshAmbienceSources);
            }

            tracing::info!(
                ambiance = ?cue.ambiance,
                at_tick = cue.at_tick,
                transition_ticks = cue.transition_ticks,
                "applied authored runtime ambience cue"
            );
        }
    }

    /// Advance extensions for one completed interactive simulation tick.
    /// Returns true exactly once when an enabled time limit expires.
    pub(super) fn tick_mission_runtime_features(&mut self, assets: &LevelAssets) -> bool {
        if self.mission_domain.state.mission_won {
            return false;
        }

        self.mission_domain
            .state
            .runtime_features
            .active_elapsed_ticks = self
            .mission_domain
            .state
            .runtime_features
            .active_elapsed_ticks
            .saturating_add(1);

        let mut newly_expired = false;
        if self.control.sim_config.enable_timed_missions
            && let Some(timer) = self
                .mission_domain
                .state
                .runtime_features
                .timed_mission
                .as_mut()
            && !timer.expired
        {
            timer.elapsed_ticks = timer.elapsed_ticks.saturating_add(1);
            if timer.elapsed_ticks >= timer.limit_ticks {
                timer.elapsed_ticks = timer.limit_ticks;
                timer.expired = true;
                newly_expired = true;
            }
        }

        if self.control.sim_config.enable_dynamic_ambience
            && let Some(schedule) = self
                .mission_domain
                .state
                .runtime_features
                .ambience_schedule
                .as_mut()
        {
            schedule.elapsed_ticks = schedule.elapsed_ticks.saturating_add(1);
            self.apply_due_ambience_cues(Some(assets));
            self.update_ambience_crossfade();
        }

        newly_expired
    }

    pub fn mission_countdown(&self) -> Option<MissionCountdownStatus> {
        let timer = self
            .mission_domain
            .state
            .runtime_features
            .timed_mission
            .as_ref()?;
        Some(MissionCountdownStatus {
            remaining_ticks: timer.limit_ticks.saturating_sub(timer.elapsed_ticks),
            limit_ticks: timer.limit_ticks,
            warning_ticks: timer.warning_ticks,
            mode: timer.countdown_mode,
            expired: timer.expired,
        })
    }

    pub fn initial_mission_ambiance(&self) -> Ambiance {
        self.mission_domain
            .state
            .runtime_features
            .ambience_schedule
            .as_ref()
            .map_or(self.world.weather.ambiance, |schedule| {
                schedule.initial_ambiance
            })
    }

    pub fn initial_mission_night_color(&self) -> u16 {
        ambiance_color(self.initial_mission_ambiance())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_blend_has_exact_endpoints() {
        let day = ambiance_color(Ambiance::Day);
        let night = ambiance_color(Ambiance::Night);
        assert_eq!(blend_rgb565(day, night, 0, 25), day);
        assert_eq!(blend_rgb565(day, night, 25, 25), night);
        assert_ne!(blend_rgb565(day, night, 12, 25), day);
        assert_ne!(blend_rgb565(day, night, 12, 25), night);
    }
}

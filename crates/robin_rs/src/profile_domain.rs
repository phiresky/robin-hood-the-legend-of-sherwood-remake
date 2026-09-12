//! Profile transition ownership. Persistence policies remain operation-specific.
//! Lock order is profiles, keys, trust, then the caller's derived simulation state.

use crate::application::{profile_derived_state, profile_publication_visible};
use crate::key_config_store::{KeyConfigStore, ProfileKeyConfig};
use crate::spellforge_trust::SpellforgeTrustStore;
use robin_engine::{engine as engine_api, player_profile::PlayerProfileManager};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Serialize)]
pub(crate) struct ProfileDomain {
    #[serde(skip)]
    pub(crate) profile_store: crate::player_profile_store::PlayerProfileStore,
    pub(crate) player_profiles: Mutex<PlayerProfileManager>,
    pub(crate) key_configs: Mutex<KeyConfigStore>,
    pub(crate) spellforge_trust: Mutex<SpellforgeTrustStore>,
}

impl<'de> Deserialize<'de> for ProfileDomain {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "profile services require application composition",
        ))
    }
}

impl ProfileDomain {
    /// Replace the auto-created first-launch placeholder and its parallel key
    /// configuration while both context service locks are held. The returned
    /// id is the final active profile id that save/session construction must
    /// use. `None` keeps the placeholder but still finalizes first launch.
    pub(crate) fn complete_first_launch_profile(
        &self,
        sim_config: &Mutex<engine_api::SimConfig>,
        replacement: Option<(String, robin_engine::player_profile::DifficultyLevel)>,
        screen_dims: (u32, u32),
    ) -> Result<u32, String> {
        let services = self;
        let profile_id = {
            // Keep this lock order (profiles, keys, trust, then simulation)
            // consistent for the operation that updates these services as one domain
            // transition. No guard escapes this synchronous method.
            let mut profiles_guard = services
                .player_profiles
                .lock()
                .map_err(|_| "ApplicationContext player-profile lock poisoned".to_string())?;
            let mut key_configs_guard = services
                .key_configs
                .lock()
                .map_err(|_| "ApplicationContext key-config lock poisoned".to_string())?;
            let mut spellforge_trust = services
                .spellforge_trust
                .lock()
                .map_err(|_| "ApplicationContext Spellforge-trust lock poisoned".to_string())?;
            let mut state = sim_config
                .lock()
                .map_err(|_| "ApplicationContext sim-config lock poisoned".to_string())?;

            if !profiles_guard.default_profiles {
                return Err("first-launch profile transition was already completed".to_string());
            }
            let profiles_before = profiles_guard.clone();
            let key_configs_before = key_configs_guard.clone();
            let mut staged_profiles = profiles_guard.clone();
            let mut staged_keys = key_configs_guard.clone();
            let profiles = &mut staged_profiles;
            let key_configs = &mut staged_keys;
            let mut removed_profile = None;

            if let Some((name, difficulty)) = replacement {
                if profiles.profiles.len() != 1 || profiles.active_index != Some(0) {
                    return Err(format!(
                        "first-launch replacement requires one active placeholder, found {} profiles with active index {:?}",
                        profiles.profiles.len(),
                        profiles.active_index,
                    ));
                }
                let placeholder_id = profiles.profiles[0].id;
                // Revoke authority before destroying or replacing the
                // profile.  If durable trust persistence is unavailable,
                // leave the complete profile/key domain untouched rather
                // than creating an orphaned approval for a deleted identity.
                spellforge_trust
                    .remove_profile(placeholder_id)
                    .map_err(|error| {
                        format!(
                            "failed to remove first-launch placeholder Spellforge trust: {error}"
                        )
                    })?;
                profiles.default_profiles = false;
                removed_profile = Some(placeholder_id);
                profiles.delete_profile(0);
                let index =
                    profiles.create_profile_with_screen_dims(name, difficulty, Some(screen_dims));
                profiles.set_active(index);
                let profile_id = profiles.profiles[index].id;

                key_configs.configs.remove(&placeholder_id);
                key_configs
                    .configs
                    .insert(profile_id, ProfileKeyConfig::fresh());
            } else {
                profiles.default_profiles = false;
            }

            let active = profiles.get_active().ok_or_else(|| {
                "first-launch transition did not leave an active profile".to_string()
            })?;
            let profile_id = active.id;
            let next = profile_derived_state(profiles, &state)?;
            if let Some(id) = removed_profile {
                if let Err(error) = services.profile_store.quarantine_profile_saves(id) {
                    let restored = services.profile_store.restore_profile_saves(id);
                    return Err(format!(
                        "quarantine placeholder saves: {error}; restore={restored:?}"
                    ));
                }
            }

            let persistence = services
                .profile_store
                .save(profiles)
                .map_err(|error| format!("persist player profile: {error}"))
                .and_then(|()| {
                    key_configs
                        .save()
                        .map_err(|error| format!("persist key configuration: {error}"))
                });
            if let Err(error) = persistence {
                let profile_rollback = services.profile_store.save(&profiles_before);
                let key_rollback = key_configs_before.save();
                let save_rollback = if profile_rollback.is_ok() {
                    removed_profile.map(|id| services.profile_store.restore_profile_saves(id))
                } else {
                    None // Retain quarantine until startup reads the actual archive.
                };
                return Err(format!(
                    "failed to complete durable first-launch profile transition: {error}; rollback profile={profile_rollback:?}, keys={key_rollback:?}, saves={save_rollback:?}"
                ));
            }
            // Durable profile/key writes above retain their explicit best-effort
            // rollback contract; trust revocation stays fail-closed. Renamed saves
            // remain recoverable and startup uses profiles.json to restore them.
            *profiles_guard = staged_profiles;
            *key_configs_guard = staged_keys;
            *state = next;
            profile_id
        };

        Ok(profile_id)
    }

    /// Profile metadata is the commit point. Save quarantine is reversible
    /// until that publication; stale key bindings are harmless cleanup afterward.
    pub(crate) fn delete_player_profile(
        &self,
        sim_config: &Mutex<engine_api::SimConfig>,
        index: usize,
    ) -> Result<bool, String> {
        let services = self;
        // Same lock order as first-launch replacement.
        let mut profiles = services
            .player_profiles
            .lock()
            .map_err(|_| "ApplicationContext player-profile lock poisoned")?;
        let mut keys = services
            .key_configs
            .lock()
            .map_err(|_| "ApplicationContext key-config lock poisoned")?;
        let mut trust = services
            .spellforge_trust
            .lock()
            .map_err(|_| "ApplicationContext Spellforge-trust lock poisoned")?;
        let mut state = sim_config
            .lock()
            .map_err(|_| "ApplicationContext sim-config lock poisoned")?;
        let Some(profile) = profiles.profiles.get(index) else {
            return Ok(false);
        };
        if profiles.profiles.len() == 1 {
            tracing::warn!("Refusing to delete the final player profile");
            return Ok(false);
        }
        let id = profile.id;
        let mut staged = profiles.clone();
        staged.delete_profile(index);
        staged.set_active(0);
        let next = profile_derived_state(&staged, &state)?;
        services
            .profile_store
            .restore_profile_saves(id)
            .map_err(|error| format!("recover interrupted player deletion: {error}"))?;
        // Revocation deliberately fails closed and is never rolled back.
        trust.remove_profile(id)?;
        if let Err(error) = services.profile_store.quarantine_profile_saves(id) {
            let restored = services.profile_store.restore_profile_saves(id);
            return Err(format!(
                "quarantine player saves: {error}; restore={restored:?}"
            ));
        }
        if let Err(error) = services.profile_store.save(&staged) {
            if !profile_publication_visible(&error) {
                let restored = services.profile_store.restore_profile_saves(id);
                return Err(format!(
                    "persist player deletion: {error}; restore={restored:?}"
                ));
            }
            // Replacement happened: reverting only memory/saves would contradict
            // the visible archive. Keep quarantine and report durability uncertainty.
            tracing::error!("Player deletion published but durability is unconfirmed: {error}");
        }
        *profiles = staged;
        *state = next;
        keys.configs.remove(&id);
        if let Err(error) = keys.save() {
            tracing::warn!("Player deleted; obsolete key configuration cleanup failed: {error}");
        }
        Ok(true)
    }
}

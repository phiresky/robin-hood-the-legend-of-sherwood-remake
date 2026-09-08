//! Host-owned profile transactions. Clock receipts are process-local, while
//! history promotion deduplicates against persisted attempt identities.
use super::*;

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct ProfileClockCredits {
    // TODO(profile-time): these receipts are intentionally process-local.
    // Durable exactly-once time accounting across application restarts needs
    // a versioned profile/save ledger, not an inferred mission timestamp.
    receipts: Vec<ClockReceipt>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ClockReceipt {
    profile_id: u32,
    mission_id: u32,
    run_id: Option<u64>,
    preceding_attempt: Option<robin_engine::campaign_history::MissionAttemptKey>,
    seconds: u32,
}

impl ProfileClockCredits {
    fn credit(
        &mut self,
        profile_id: u32,
        mission_id: u32,
        campaign: &Campaign,
        seconds: u32,
    ) -> u32 {
        let run_id = campaign.history_run_id();
        let preceding_attempt = campaign.latest_mission_attempt_key();
        if let Some(receipt) = self.receipts.iter_mut().find(|receipt| {
            receipt.profile_id == profile_id
                && receipt.mission_id == mission_id
                && receipt.run_id == run_id
                && receipt.preceding_attempt == preceding_attempt
        }) {
            let credit = seconds.saturating_sub(receipt.seconds);
            // Loading/rewinding an earlier clock must not credit the same
            // interval twice. A new terminal attempt has a distinct identity.
            receipt.seconds = receipt.seconds.max(seconds);
            credit
        } else {
            self.receipts.push(ClockReceipt {
                profile_id,
                mission_id,
                run_id,
                preceding_attempt,
                seconds,
            });
            seconds
        }
    }
}

pub(super) fn synchronize_metrics(
    application: &ApplicationContext,
    credits: &mut ProfileClockCredits,
    campaign: &Campaign,
    profiles: &ProfileManager,
    seconds: u32,
) {
    let result = application
        .with_player_profiles_mut(|manager| {
            let profile = manager
                .get_active_mut()
                .expect("profile synchronization requires an active profile");
            let credit = credits.credit(
                profile.id,
                current_mission_id(campaign, profiles),
                campaign,
                seconds,
            );
            robin_engine::player_profile::synchronize_with_campaign(
                profile, campaign, profiles, credit,
            );
            // Always retry persistence, even if this invocation added no time.
            // The receipt tracks the in-memory application, not disk success.
            application.persist_player_profiles(manager)
        })
        .unwrap_or_else(|error| {
            panic!("profile synchronization lost its ApplicationContext: {error}")
        });
    if let Err(error) = result {
        tracing::error!(
            "Profile metrics persistence failed; retained in memory and retried on the next profile save/synchronization: {error}"
        );
    }
}

impl RustCallbacks {
    /// Call only after the terminal engine command and eligibility attestation.
    /// Repeating promotion refreshes attestations without duplicating history;
    /// persistence must run even when promotion reports no new attempts.
    pub(crate) fn promote_terminal_profile(
        application: &ApplicationContext,
        campaign: &Campaign,
        profiles: &ProfileManager,
    ) {
        application.with_player_profiles_mut(|manager| {
            manager.get_active_mut().expect("terminal promotion requires an active profile")
                .promote_campaign_history(campaign, profiles)
                .unwrap_or_else(|error| panic!("campaign-history promotion failed: {error}"));
            if let Err(error) = application.persist_player_profiles(manager) {
                #[cfg(not(target_arch = "wasm32"))]
                panic!("failed to persist campaign history: {error}");
                #[cfg(target_arch = "wasm32")]
                tracing::warn!("Failed to persist campaign history in browser storage; keeping it in memory for this session: {error}");
            }
        }).unwrap_or_else(|error| panic!("campaign profile synchronization failed: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_sync_and_load_back_only_credit_new_clock_intervals() {
        let campaign = Campaign::default();
        let mut credits = ProfileClockCredits::default();
        assert_eq!(credits.credit(1, 17, &campaign, 60), 60);
        assert_eq!(credits.credit(1, 17, &campaign, 60), 0);
        assert_eq!(credits.credit(1, 17, &campaign, 20), 0);
        assert_eq!(credits.credit(1, 17, &campaign, 70), 10);
        assert_eq!(credits.credit(2, 17, &campaign, 70), 70);
        assert_eq!(credits.credit(1, 18, &campaign, 70), 70);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn failed_persistence_retry_writes_applied_metrics_without_recrediting() {
        let directory = tempfile::tempdir().unwrap();
        let (callbacks, _host, engine, _assets, _game, profiles) =
            super::super::operation_outcome_tests::diagnostic_callback_fixture(directory.path());
        let application = &callbacks.application_context;
        let campaign = engine.campaign();
        let mut credits = ProfileClockCredits::default();
        // A directory at the exact file target deterministically rejects the
        // atomic rename, without touching permissions or any player data.
        let target = directory.path().join("profiles.json");
        std::fs::create_dir(&target).unwrap();
        synchronize_metrics(application, &mut credits, campaign, &profiles, 60);
        assert_eq!(
            application
                .with_player_profiles_mut(|manager| manager.get_active_mut().unwrap().play_time)
                .unwrap(),
            60
        );
        std::fs::remove_dir(&target).unwrap();
        synchronize_metrics(application, &mut credits, campaign, &profiles, 60);
        let stored = crate::player_profile_store::PlayerProfileStore::for_directory(
            directory.path().to_str().unwrap(),
        )
        .load()
        .unwrap();
        assert_eq!(stored.profiles[0].play_time, 60);
        synchronize_metrics(application, &mut credits, campaign, &profiles, 70);
        let stored = crate::player_profile_store::PlayerProfileStore::for_directory(
            directory.path().to_str().unwrap(),
        )
        .load()
        .unwrap();
        assert_eq!(stored.profiles[0].play_time, 70);

        let mut completed = campaign.clone();
        completed.record_mission_attempt(
            0,
            robin_engine::campaign_history::MissionAttemptOutcome::Won,
            Some(100),
            Some(0xbeef),
            70,
            engine_api::SimConfig::default(),
            &robin_engine::mission_stat::MissionStat::default(),
            None,
        );
        RustCallbacks::promote_terminal_profile(application, &completed, &profiles);
        RustCallbacks::promote_terminal_profile(application, &completed, &profiles);
        let stored = crate::player_profile_store::PlayerProfileStore::for_directory(
            directory.path().to_str().unwrap(),
        )
        .load()
        .unwrap();
        assert_eq!(
            stored.profiles[0].play_time, 70,
            "history promotion does not credit time"
        );
        assert_eq!(stored.profiles[0].lifetime_campaign_totals().attempts, 1);
        assert_eq!(
            credits.credit(stored.profiles[0].id, 17, &completed, 20),
            20,
            "a subsequent attempt gets its own clock"
        );
    }
}

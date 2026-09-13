//! Multiplayer session gate state owned by the timeline: ordered admission,
//! the host frame clock with its exactly-once hash publication, and the last
//! rollback telemetry sample.
//!
//! None of this participates in deterministic engine state or the wire format.
//! Transitions that must also discard network reconciliation samples stay on
//! `TimelineRuntime`, which sequences both owners.
use super::MultiplayerAdmission;
use super::timing::MultiplayerTiming;
use crate::game_session::multiplayer::MultiplayerRollbackTelemetry;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(in crate::game_session) struct MultiplayerSync {
    pub(super) admission: MultiplayerAdmission,
    pub(super) timing: MultiplayerTiming,
    /// Process-side diagnostics from the most recent network rollback.
    #[serde(skip)]
    last_rollback: Option<MultiplayerRollbackTelemetry>,
}

impl MultiplayerSync {
    pub(super) fn new(wait_for_multiplayer_start: bool, local_is_host: bool) -> Self {
        Self {
            admission: match (wait_for_multiplayer_start, local_is_host) {
                (false, _) => MultiplayerAdmission::NotRequired,
                (true, true) => MultiplayerAdmission::HostWaitingForBegin,
                (true, false) => MultiplayerAdmission::PeerWaitingForSnapshot,
            },
            timing: MultiplayerTiming::default(),
            last_rollback: None,
        }
    }

    pub(in crate::game_session) fn admission(&self) -> MultiplayerAdmission {
        self.admission
    }

    pub(in crate::game_session) fn timing(&self) -> &MultiplayerTiming {
        &self.timing
    }

    pub(in crate::game_session) fn timing_mut(&mut self) -> &mut MultiplayerTiming {
        &mut self.timing
    }

    pub(in crate::game_session) fn last_rollback(&self) -> Option<&MultiplayerRollbackTelemetry> {
        self.last_rollback.as_ref()
    }

    pub(in crate::game_session) fn record_rollback(
        &mut self,
        rollback: MultiplayerRollbackTelemetry,
    ) {
        self.last_rollback = Some(rollback);
    }

    /// Advance the wall-clock release gate and report whether simulation must
    /// remain held for multiplayer admission.
    pub(in crate::game_session) fn admission_paused(&mut self, now_epoch_ms: u64) -> bool {
        if let MultiplayerAdmission::WaitingForStart {
            frame,
            start_epoch_ms,
        } = self.admission
            && now_epoch_ms >= start_epoch_ms
        {
            self.admission = MultiplayerAdmission::Running;
            tracing::info!(frame, "multiplayer: synchronized start gate opened");
        }
        !matches!(
            self.admission,
            MultiplayerAdmission::NotRequired | MultiplayerAdmission::Running
        )
    }

    /// `local_frame` is the timeline cursor at receipt; it only feeds the
    /// diagnostic deadline delta, never the accepted schedule itself.
    pub(in crate::game_session) fn accept_host_frame_schedule(
        &mut self,
        frame: u32,
        delay_ms: u32,
        local_frame: u32,
    ) {
        let now_ms = crate::window::process_uptime_ms();
        if !self.timing.accept_schedule(frame, delay_ms, now_ms) {
            tracing::trace!(
                clock_frame = frame,
                current_sample_frame = self.timing.schedule_frame(),
                "multiplayer: ignored stale host frame schedule"
            );
            return;
        }
        tracing::info!(
            host_clock_frame = frame,
            ms_until_next_frame = delay_ms,
            local_frame_at_receive = local_frame,
            deadline_delta_ms_for_local_frame = self
                .timing
                .deadline_ms(local_frame)
                .expect("schedule just installed")
                - i64::from(now_ms),
            "multiplayer: received host frame schedule"
        );
    }

    /// Both graphical and headless drivers publish through this consuming
    /// transition. A failed send is not retried: NetChannels latches worker
    /// failure for the next ingress poll, just as before this extraction.
    pub(in crate::game_session) fn publish_timing(
        &mut self,
        transport: &crate::host::HostTransport,
        clock_frame: u32,
        remaining_sleep_ms: u32,
    ) {
        let Some(net) = transport.net() else { return };
        if transport.local_seat() != robin_engine::player_command::PlayerId::HOST {
            return;
        }
        let Some(sample) = self.timing.take_publication() else {
            return;
        };
        net.publish_frame(clock_frame);
        tracing::info!(
            hash_frame = sample.frame,
            clock_frame,
            remaining_sleep_ms,
            "multiplayer: host sending state hash timing sample"
        );
        if let Err(error) =
            net.send_state_hash(sample.frame, sample.hash, clock_frame, remaining_sleep_ms)
        {
            tracing::error!(%error, "multiplayer state hash publication failed");
        }
    }
}

robin_util::deny_deserialize!(
    MultiplayerSync,
    "multiplayer session gates are live timeline authority, not a saved game"
);

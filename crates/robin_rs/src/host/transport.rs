//! Mission transport authority and snapshot lifecycle.
use super::*;

/// Mission-scoped transport authority. Admission installs the channel owner and
/// construction metadata together; later events can only advance named states.
///
/// ```compile_fail,E0616
/// let mut transport = robin_rs::host::HostTransport::default();
/// transport.net = None;
/// ```
///
/// ```compile_fail,E0616
/// let mut transport = robin_rs::host::HostTransport::default();
/// transport.local_seat = robin_engine::player_command::PlayerId::HOST;
/// ```
#[derive(Default)]
pub struct HostTransport {
    local_seat: engine_player_command::PlayerId,
    net: Option<crate::multiplayer::NetChannels>,
    mission_seed: Option<u64>,
    mission_sim_config: Option<engine_api::SimConfig>,
    speech_timing_locale: Option<String>,
    mission_id: Option<String>,
    synchronization: TransportSynchronization,
    /// Verified full-mod bytes, VFS overlays, and cache lease for a
    /// host-distributed mission. Field order makes the network runtime stop
    /// before this mount is dropped with the enclosing transport.
    #[cfg(feature = "multiplayer")]
    distributed_mod: Option<crate::distributed_mod_admission::AdmittedDistributedMod>,
    /// Delayed Sherwood command boundary belongs to this transport lifetime.
    pending_campaign_exit: Option<crate::main_entry::PendingMultiplayerCampaignExit>,
}

/// Prepared transitions always hold simulation. Consuming their committed
/// payload keeps that hold until the replacement mission releases BeginSim.
#[derive(Default)]
enum TransportSynchronization {
    #[default]
    Running,
    AwaitingSnapshot,
    Prepared(PendingSnapshotTransition),
}

pub struct PendingSnapshotTransition {
    id: robin_engine::multiplayer::SnapshotTransitionId,
    payload: PendingSnapshotTransitionPayload,
    committed: bool,
}

impl PendingSnapshotTransition {
    pub(crate) fn new(
        id: robin_engine::multiplayer::SnapshotTransitionId,
        payload: PendingSnapshotTransitionPayload,
    ) -> Self {
        Self {
            id,
            payload,
            committed: false,
        }
    }
    /// Called by the authenticated transport event drain; the prepared payload
    /// and instance remain private and cannot be swapped after this admission.
    pub(crate) fn commit_authenticated(
        &mut self,
        id: robin_engine::multiplayer::SnapshotTransitionId,
    ) -> Result<(), String> {
        if self.id != id {
            return Err("snapshot transition commit does not match prepared payload".into());
        }
        if self.committed {
            return Err("snapshot transition was already committed".into());
        }
        self.committed = true;
        Ok(())
    }
}

pub enum PendingSnapshotTransitionPayload {
    Save {
        load: SnapshotSave,
    },
    CampaignExit {
        exit_code: robin_engine::game_operation::GameCode,
        /// Clients retain the exact decoded host engine so their campaign is
        /// identical before all participants enter the next mission. The
        /// host already owns that engine and therefore stores `None`.
        engine: Option<Box<robin_engine::engine::Engine>>,
    },
}

pub enum SnapshotSave {
    Local(crate::main_entry::PreparedLoad),
    Remote(Box<crate::save_file::GameSaveFile>),
}

/// Only the transport's committed take can create this process-local token.
pub(crate) struct CommittedSnapshotTransition(PendingSnapshotTransition);

impl serde::Serialize for CommittedSnapshotTransition {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Diagnostic serialization deliberately carries no payload or authority.
        serializer.serialize_unit_struct("CommittedSnapshotTransition")
    }
}

robin_util::deny_deserialize!(
    CommittedSnapshotTransition,
    "committed transition cannot be deserialized"
);

impl CommittedSnapshotTransition {
    pub(crate) fn id(&self) -> robin_engine::multiplayer::SnapshotTransitionId {
        self.0.id
    }
    pub(crate) fn is_save(&self) -> bool {
        matches!(
            self.0.payload,
            PendingSnapshotTransitionPayload::Save { .. }
        )
    }
    pub(crate) fn into_payload(self) -> PendingSnapshotTransitionPayload {
        self.0.payload
    }
}

impl HostTransport {
    pub fn local_seat(&self) -> engine_player_command::PlayerId {
        self.local_seat
    }
    pub fn net(&self) -> Option<&crate::multiplayer::NetChannels> {
        self.net.as_ref()
    }
    pub fn mission_seed(&self) -> Option<u64> {
        self.mission_seed
    }
    pub fn mission_sim_config(&self) -> Option<engine_api::SimConfig> {
        self.mission_sim_config
    }
    pub fn speech_timing_locale(&self) -> Option<&str> {
        self.speech_timing_locale.as_deref()
    }
    pub fn mission_id(&self) -> Option<&str> {
        self.mission_id.as_deref()
    }
    pub fn reconnecting(&self) -> bool {
        !matches!(self.synchronization, TransportSynchronization::Running)
    }
    pub fn has_snapshot_transition(&self) -> bool {
        matches!(self.synchronization, TransportSynchronization::Prepared(_))
    }

    /// Install one fully validated session before engine construction. Metadata
    /// and identity cannot be partially replaced by a subsequent Welcome.
    #[cfg(any(feature = "multiplayer", test))]
    pub(crate) fn install_session(
        &mut self,
        net: crate::multiplayer::NetChannels,
        seat: engine_player_command::PlayerId,
        mission_id: String,
        seed: u64,
        config: engine_api::SimConfig,
        speech_locale: Option<String>,
    ) {
        assert!(
            self.net.is_none(),
            "cannot overwrite a live multiplayer session"
        );
        assert!(
            !self.has_snapshot_transition(),
            "cannot install a session during a snapshot transition"
        );
        self.local_seat = seat;
        self.mission_id = Some(mission_id);
        self.mission_seed = Some(seed);
        self.mission_sim_config = Some(config);
        self.speech_timing_locale = speech_locale;
        self.net = Some(net);
        self.synchronization = TransportSynchronization::Running;
    }

    pub(crate) fn confirm_local_seat(&self, seat: engine_player_command::PlayerId) {
        assert_eq!(
            self.local_seat, seat,
            "assigned seat changed after session admission"
        );
    }

    /// Hold simulation without losing the admitted seat, campaign, or runtime.
    pub(crate) fn await_authoritative_snapshot(&mut self) {
        if !self.has_snapshot_transition() {
            self.synchronization = TransportSynchronization::AwaitingSnapshot;
        }
    }
    pub(crate) fn begin_simulation(&mut self) {
        assert!(
            !self.has_snapshot_transition(),
            "ordinary snapshot cannot complete a prepared transition"
        );
        self.synchronization = TransportSynchronization::Running;
    }
    pub(crate) fn prepare_snapshot_transition(&mut self, pending: PendingSnapshotTransition) {
        assert!(
            !self.has_snapshot_transition(),
            "snapshot transition already prepared"
        );
        self.synchronization = TransportSynchronization::Prepared(pending);
    }
    pub(crate) fn commit_snapshot_transition(
        &mut self,
        id: robin_engine::multiplayer::SnapshotTransitionId,
    ) -> Result<(), String> {
        match &mut self.synchronization {
            TransportSynchronization::Prepared(pending) => pending.commit_authenticated(id),
            _ => Err("snapshot transition commit has no prepared payload".into()),
        }
    }
    #[cfg(feature = "multiplayer")]
    pub(crate) fn retain_distributed_mod(
        &mut self,
        admitted: crate::distributed_mod_admission::AdmittedDistributedMod,
    ) {
        assert!(
            self.distributed_mod.is_none(),
            "distributed mission content already admitted"
        );
        self.distributed_mod = Some(admitted);
    }
    pub(crate) fn defer_campaign_exit(
        &mut self,
        pending: crate::main_entry::PendingMultiplayerCampaignExit,
    ) {
        assert!(
            self.pending_campaign_exit.is_none(),
            "campaign exit already pending"
        );
        self.pending_campaign_exit = Some(pending);
    }
    pub(crate) fn pending_campaign_exit(
        &self,
    ) -> Option<&crate::main_entry::PendingMultiplayerCampaignExit> {
        self.pending_campaign_exit.as_ref()
    }
    pub(crate) fn take_campaign_exit_at(
        &mut self,
        frame: u32,
    ) -> Option<crate::main_entry::PendingMultiplayerCampaignExit> {
        if self
            .pending_campaign_exit
            .as_ref()
            .is_some_and(|pending| frame >= pending.not_before_frame)
        {
            self.pending_campaign_exit.take()
        } else {
            None
        }
    }
    pub(crate) fn preserve_session_for_next_mission(&mut self) {
        if let Some(net) = self.net.as_mut() {
            net.preserve_session_for_next_mission();
        }
    }
    #[cfg(test)]
    pub(crate) fn test_session(
        net: crate::multiplayer::NetChannels,
        seat: engine_player_command::PlayerId,
    ) -> Self {
        Self {
            net: Some(net),
            local_seat: seat,
            ..Self::default()
        }
    }
    #[cfg(test)]
    pub(crate) fn test_local_seat(&mut self, seat: engine_player_command::PlayerId) {
        self.local_seat = seat;
    }
    #[cfg(test)]
    pub(crate) fn test_drop_channels(&mut self) {
        self.net = None;
    }

    pub fn authoritative_transition_actions_enabled(&self) -> bool {
        !self.reconnecting() && self.local_seat == robin_engine::player_command::PlayerId::HOST
    }

    pub(crate) fn take_committed_snapshot_transition(
        &mut self,
    ) -> Option<CommittedSnapshotTransition> {
        if !matches!(&self.synchronization, TransportSynchronization::Prepared(pending) if pending.committed)
        {
            return None;
        }
        let TransportSynchronization::Prepared(pending) = std::mem::replace(
            &mut self.synchronization,
            TransportSynchronization::AwaitingSnapshot,
        ) else {
            unreachable!("checked committed preparation")
        };
        Some(CommittedSnapshotTransition(pending))
    }
}

#[cfg(test)]
mod transport_lifecycle_tests {
    use super::*;
    use robin_engine::multiplayer::{MultiplayerSessionId, SnapshotTransitionId};
    use robin_engine::player_command::PlayerId;

    fn installed(seat: PlayerId) -> HostTransport {
        let (channels, _incoming, _outgoing, _, _) = crate::multiplayer::NetChannels::new();
        let mut transport = HostTransport::default();
        transport.install_session(
            channels,
            seat,
            "leicester".into(),
            42,
            engine_api::SimConfig::default(),
            Some("en".into()),
        );
        transport
    }

    fn transition_id() -> SnapshotTransitionId {
        SnapshotTransitionId {
            session_id: MultiplayerSessionId([8; 32]),
            sequence: 1,
        }
    }

    fn prepare(transport: &mut HostTransport) {
        transport.prepare_snapshot_transition(PendingSnapshotTransition::new(
            transition_id(),
            PendingSnapshotTransitionPayload::CampaignExit {
                exit_code: robin_engine::game_operation::GameCode::LevelInterrupted,
                engine: None,
            },
        ));
    }

    #[test]
    fn reconnect_retains_admitted_identity_metadata_and_channels() {
        let mut transport = installed(PlayerId(2));
        let channel_address = transport.net().unwrap() as *const _;
        transport.await_authoritative_snapshot();
        assert!(transport.reconnecting());
        assert!(!transport.authoritative_transition_actions_enabled());
        assert_eq!(transport.local_seat(), PlayerId(2));
        assert_eq!(transport.mission_id(), Some("leicester"));
        assert_eq!(transport.mission_seed(), Some(42));
        assert_eq!(
            transport.mission_sim_config(),
            Some(engine_api::SimConfig::default())
        );
        assert_eq!(transport.speech_timing_locale(), Some("en"));
        assert_eq!(transport.net().unwrap() as *const _, channel_address);
        transport.confirm_local_seat(PlayerId(2));
        transport.begin_simulation();
        assert!(!transport.reconnecting());
        assert!(
            !transport.authoritative_transition_actions_enabled(),
            "a resumed peer never becomes the host"
        );
    }

    #[test]
    fn committed_payload_is_consumed_once_and_keeps_simulation_held() {
        let mut transport = installed(PlayerId::HOST);
        prepare(&mut transport);
        assert!(transport.take_committed_snapshot_transition().is_none());
        assert!(
            transport
                .commit_snapshot_transition(SnapshotTransitionId {
                    sequence: 2,
                    ..transition_id()
                })
                .is_err()
        );
        transport.await_authoritative_snapshot();
        assert!(
            transport.has_snapshot_transition(),
            "disconnect cannot discard an authenticated preparation"
        );
        transport
            .commit_snapshot_transition(transition_id())
            .unwrap();
        assert!(
            transport
                .commit_snapshot_transition(transition_id())
                .is_err()
        );
        assert_eq!(
            transport.take_committed_snapshot_transition().unwrap().id(),
            transition_id()
        );
        assert!(transport.take_committed_snapshot_transition().is_none());
        assert!(!transport.has_snapshot_transition());
        assert!(transport.reconnecting());
        assert!(!transport.authoritative_transition_actions_enabled());
        transport.begin_simulation();
        assert!(transport.authoritative_transition_actions_enabled());
    }

    #[test]
    #[should_panic(expected = "ordinary snapshot cannot complete a prepared transition")]
    fn ordinary_barrier_cannot_discard_prepared_payload() {
        let mut transport = installed(PlayerId::HOST);
        prepare(&mut transport);
        transport.begin_simulation();
    }

    #[test]
    #[should_panic(expected = "assigned seat changed after session admission")]
    fn late_assignment_cannot_rewrite_admitted_seat() {
        installed(PlayerId(2)).confirm_local_seat(PlayerId::HOST);
    }

    #[test]
    fn losing_test_channels_does_not_promote_a_waiting_peer() {
        let mut transport = installed(PlayerId(2));
        transport.await_authoritative_snapshot();
        transport.test_drop_channels();
        assert_eq!(transport.local_seat(), PlayerId(2));
        assert!(!transport.authoritative_transition_actions_enabled());
        transport.begin_simulation();
        assert!(
            !transport.authoritative_transition_actions_enabled(),
            "missing channels cannot turn a former client into a single-player host"
        );
    }
}

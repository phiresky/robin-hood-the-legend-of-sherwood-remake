//! Ordered presentation effects and deferred audio.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeferredAudioRequest {
    PlayDelayedSource(usize),
    ResumeAllSources,
    ActivateSource(usize),
    RefreshAmbienceSources,
    StopExclamation(u32),
    StopExclamationChannel(u32),
}

#[derive(Default)]
pub struct HostAudio {
    pub sound: SoundManager,
    pub deferred: Vec<DeferredAudioRequest>,
}

pub use robin_engine::engine::HostSignal;

/// Live presentation facts required to admit a Sherwood trading-panel request.
///
/// These checks intentionally mirror the authoritative sale-command ordering:
/// host ownership, feature rule, then mission location. The engine repeats the
/// same checks when a sale reaches the deterministic command frame, so a stale
/// or forged presentation request cannot mutate campaign state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SherwoodTradingAccess {
    pub(crate) local_is_host: bool,
    pub(crate) enabled: bool,
    pub(crate) in_sherwood: bool,
}

impl SherwoodTradingAccess {
    pub(crate) fn validate(self) -> Result<(), robin_engine::trading::TradeRejectReason> {
        use robin_engine::trading::TradeRejectReason;
        if !self.local_is_host {
            return Err(TradeRejectReason::HostOnly);
        }
        if !self.enabled {
            return Err(TradeRejectReason::TradingDisabled);
        }
        if !self.in_sherwood {
            return Err(TradeRejectReason::NotInSherwood);
        }
        Ok(())
    }
}

/// Host-session state surrounding the shared deferred requests.
#[derive(Default)]
pub struct HostEffectBatches {
    requests: robin_engine::engine::HostEffects,
    next_trade_request_id: u64,
    modal_admission: Option<robin_engine::multiplayer::ModalEffectAdmission>,
    remote_story_session: bool,
}

impl std::ops::Deref for HostEffectBatches {
    type Target = robin_engine::engine::HostEffects;
    fn deref(&self) -> &Self::Target {
        &self.requests
    }
}

impl std::ops::DerefMut for HostEffectBatches {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.requests
    }
}

impl HostEffectBatches {
    pub(crate) fn bind_modal_session(
        &mut self,
        admission: robin_engine::multiplayer::ModalEffectAdmission,
    ) {
        admission
            .admit(&self.requests.modals)
            .expect("pending startup story admission");
        self.modal_admission = Some(admission);
        self.remote_story_session = false;
    }

    pub(crate) fn bind_remote_modal_session(&mut self) {
        self.remote_story_session = true;
        self.modal_admission = None;
        self.requests
            .modals
            .retain(|kind| !robin_engine::multiplayer::is_shared_story_modal(kind));
    }

    pub fn append(&mut self, mut incoming: robin_engine::engine::HostEffects) {
        if self.remote_story_session {
            incoming
                .modals
                .retain(|kind| !robin_engine::multiplayer::is_shared_story_modal(kind));
        }
        let previous = self.requests.modals.len();
        self.requests.append(incoming);
        if let Some(admission) = &self.modal_admission {
            admission
                .admit(&self.requests.modals[previous..])
                .expect("story effect admission");
        }
    }

    fn extend_story_requests(
        &mut self,
        kinds: impl IntoIterator<Item = robin_engine::player_command::ModalKind>,
    ) {
        if self.remote_story_session {
            return;
        }
        let previous = self.requests.modals.len();
        self.requests.modals.extend(kinds);
        if let Some(admission) = &self.modal_admission {
            admission
                .admit(&self.requests.modals[previous..])
                .expect("story request admission");
        }
    }

    pub fn extend_dialogues(&mut self, ids: impl IntoIterator<Item = i32>) {
        self.extend_story_requests(
            ids.into_iter()
                .map(|dialog_id| robin_engine::player_command::ModalKind::Dialog { dialog_id }),
        );
    }

    pub fn extend_popup_texts(&mut self, ids: impl IntoIterator<Item = i32>) {
        self.extend_story_requests(
            ids.into_iter()
                .map(|text_id| robin_engine::player_command::ModalKind::PopupText { text_id }),
        );
    }

    pub fn extend_debriefings(
        &mut self,
        ids: impl IntoIterator<Item = robin_engine::player_command::DebriefingTextId>,
    ) {
        self.extend_story_requests(
            ids.into_iter()
                .map(|text_id| robin_engine::player_command::ModalKind::Debriefing { text_id }),
        );
    }

    pub fn request_sherwood_report(&mut self) {
        if !self.requests.has_sherwood_report() {
            self.extend_story_requests([robin_engine::player_command::ModalKind::SherwoodReport]);
        }
    }

    pub fn clear(&mut self) {
        if let Some(admission) = &self.modal_admission {
            admission
                .discard_pending()
                .expect("clear pending story effects");
        }
        self.requests.clear();
    }

    /// Allocate a process-session correlation id for one authoritative sale.
    /// This counter deliberately survives panel close/reopen and effect-queue
    /// clears so a delayed network receipt cannot alias a newer request.
    pub(crate) fn allocate_trade_request_id(&mut self) -> u64 {
        self.next_trade_request_id = self
            .next_trade_request_id
            .checked_add(1)
            .expect("Sherwood trade request id exhausted");
        self.next_trade_request_id
    }

    /// Queue a player-facing trading-panel request only after all live access
    /// checks pass. This is the single producer used by keyboard and menu UI.
    pub(crate) fn request_sherwood_trading(
        &mut self,
        access: SherwoodTradingAccess,
    ) -> Result<(), robin_engine::trading::TradeRejectReason> {
        access.validate()?;
        self.request_signal(HostSignal::SherwoodTrading);
        Ok(())
    }

    /// Consume and revalidate a queued request immediately before modal
    /// construction. A settings/location/seat transition between input and
    /// presentation therefore fails closed, while an empty queue is ordinary.
    pub(crate) fn take_sherwood_trading(
        &mut self,
        access: SherwoodTradingAccess,
    ) -> Result<bool, robin_engine::trading::TradeRejectReason> {
        if !self.take_signal(HostSignal::SherwoodTrading) {
            return Ok(false);
        }
        access.validate()?;
        Ok(true)
    }
}

#[cfg(test)]
mod modal_tests {
    use super::*;
    use robin_engine::multiplayer::{MultiplayerSessionId, NetChannels};
    use robin_engine::player_command::ModalKind;

    #[test]
    fn story_requests_preserve_other_effects_and_reserve_only_retained_reports() {
        let (net, _input, _output, _, _) = NetChannels::new();
        net.install_session_id(MultiplayerSessionId([9; 32]))
            .unwrap();
        let mut effects = HostEffectBatches::default();
        effects.bind_modal_session(net.modal_effect_admission());
        effects.skip_render = true;
        effects.request_sherwood_report();
        let mut additional = robin_engine::engine::HostEffects::default();
        additional.request_sherwood_report();
        additional.skip_render = true;
        effects.append(additional);
        effects.extend_popup_texts([7]);
        assert!(effects.skip_render);
        assert_eq!(
            effects.modals,
            [
                ModalKind::SherwoodReport,
                ModalKind::PopupText { text_id: 7 }
            ]
        );
        let report = net.open_modal_instance(&ModalKind::SherwoodReport).unwrap();
        net.complete_modal_instance(&ModalKind::SherwoodReport, report)
            .unwrap();
        assert!(net.open_modal_instance(&ModalKind::SherwoodReport).is_err());
        effects.clear();
        assert!(
            net.open_modal_instance(&ModalKind::PopupText { text_id: 7 })
                .is_err()
        );
    }

    #[test]
    fn remote_story_requests_require_session_admission() {
        let mut effects = HostEffectBatches::default();
        effects.extend_dialogues([1]);
        effects.bind_remote_modal_session();
        effects.extend_dialogues([2]);
        effects.request_sherwood_report();
        assert!(effects.modals.is_empty());
        // The authoritative ingress explicitly projects its admitted instance.
        effects.modals.push(ModalKind::Dialog { dialog_id: 3 });
        assert_eq!(effects.dialogue_count(), 1);
    }
}

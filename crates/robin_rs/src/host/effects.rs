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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostModalRequest {
    Dialogue(i32),
    PopupText(i32),
    Debriefing(engine_player_command::DebriefingTextId),
    SherwoodReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSignal {
    ShowConsole,
    SilentWinWidgetSwap,
    MissionStateNotice,
    MissionStatePopup,
    ResetInput,
    PromoteFpsCheat,
    SherwoodTrading,
}

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

/// Ordered, typed work emitted at the post-tick boundary. Variant-specific
/// drains preserve the existing host phase priority and simulation timing.
#[derive(Default)]
pub struct HostEffectBatches {
    modals: Vec<HostModalRequest>,
    signals: Vec<HostSignal>,
    trade_receipts: Vec<robin_engine::trading::TradeReceipt>,
    next_trade_request_id: u64,
    pub background_blits: Vec<PendingBgBlit>,
}

impl HostEffectBatches {
    pub fn pending_modal_kinds(&self) -> Vec<engine_player_command::ModalKind> {
        self.modals
            .iter()
            .map(|request| match *request {
                HostModalRequest::Dialogue(dialog_id) => {
                    engine_player_command::ModalKind::Dialog { dialog_id }
                }
                HostModalRequest::PopupText(text_id) => {
                    engine_player_command::ModalKind::PopupText { text_id }
                }
                HostModalRequest::Debriefing(text_id) => {
                    engine_player_command::ModalKind::Debriefing { text_id }
                }
                HostModalRequest::SherwoodReport => {
                    engine_player_command::ModalKind::SherwoodReport
                }
            })
            .collect()
    }

    pub fn extend_dialogues(&mut self, ids: impl IntoIterator<Item = i32>) {
        self.modals
            .extend(ids.into_iter().map(HostModalRequest::Dialogue));
    }

    pub fn extend_popup_texts(&mut self, ids: impl IntoIterator<Item = i32>) {
        self.modals
            .extend(ids.into_iter().map(HostModalRequest::PopupText));
    }

    pub fn extend_debriefings(
        &mut self,
        ids: impl IntoIterator<Item = engine_player_command::DebriefingTextId>,
    ) {
        self.modals
            .extend(ids.into_iter().map(HostModalRequest::Debriefing));
    }

    pub fn request_sherwood_report(&mut self) {
        if !self.has_sherwood_report() {
            self.modals.push(HostModalRequest::SherwoodReport);
        }
    }

    pub fn has_sherwood_report(&self) -> bool {
        self.modals.contains(&HostModalRequest::SherwoodReport)
    }

    pub fn take_sherwood_report(&mut self) -> bool {
        let Some(index) = self
            .modals
            .iter()
            .position(|request| *request == HostModalRequest::SherwoodReport)
        else {
            return false;
        };
        self.modals.remove(index);
        true
    }

    pub fn extend_trade_receipts(
        &mut self,
        receipts: impl IntoIterator<Item = robin_engine::trading::TradeReceipt>,
    ) {
        self.trade_receipts.extend(receipts);
    }

    pub fn take_trade_receipts(&mut self) -> Vec<robin_engine::trading::TradeReceipt> {
        std::mem::take(&mut self.trade_receipts)
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

    pub fn dialogue_count(&self) -> usize {
        self.modals
            .iter()
            .filter(|request| matches!(request, HostModalRequest::Dialogue(_)))
            .count()
    }

    pub fn popup_text_count(&self) -> usize {
        self.modals
            .iter()
            .filter(|request| matches!(request, HostModalRequest::PopupText(_)))
            .count()
    }

    pub fn debriefing_count(&self) -> usize {
        self.modals
            .iter()
            .filter(|request| matches!(request, HostModalRequest::Debriefing(_)))
            .count()
    }

    pub fn take_dialogues(&mut self) -> Vec<i32> {
        take_modal_payloads(&mut self.modals, |request| match request {
            HostModalRequest::Dialogue(id) => Some(id),
            _ => None,
        })
    }

    pub fn take_popup_texts(&mut self) -> Vec<i32> {
        take_modal_payloads(&mut self.modals, |request| match request {
            HostModalRequest::PopupText(id) => Some(id),
            _ => None,
        })
    }

    pub fn take_debriefings(&mut self) -> Vec<engine_player_command::DebriefingTextId> {
        take_modal_payloads(&mut self.modals, |request| match request {
            HostModalRequest::Debriefing(id) => Some(id),
            _ => None,
        })
    }

    pub fn request_signal(&mut self, signal: HostSignal) {
        if !self.signals.contains(&signal) {
            self.signals.push(signal);
        }
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

    pub fn has_signal(&self, signal: HostSignal) -> bool {
        self.signals.contains(&signal)
    }

    pub fn take_signal(&mut self, signal: HostSignal) -> bool {
        let Some(index) = self.signals.iter().position(|queued| *queued == signal) else {
            return false;
        };
        self.signals.remove(index);
        true
    }

    pub fn clear(&mut self) {
        self.modals.clear();
        self.signals.clear();
        self.trade_receipts.clear();
        self.background_blits.clear();
    }
}

fn take_modal_payloads<T>(
    requests: &mut Vec<HostModalRequest>,
    take: impl Fn(HostModalRequest) -> Option<T>,
) -> Vec<T> {
    let mut payloads = Vec::new();
    requests.retain(|request| {
        if let Some(payload) = take(*request) {
            payloads.push(payload);
            false
        } else {
            true
        }
    });
    payloads
}

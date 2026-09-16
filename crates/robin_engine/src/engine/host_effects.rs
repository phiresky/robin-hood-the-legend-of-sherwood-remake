//! Deferred host requests shared by simulation producers and presentation consumers.
use super::PendingBgBlit;
use crate::player_command::{self as engine_player_command, ModalKind};

#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
    Copy,
    PartialEq,
    Eq,
)]
pub enum HostSignal {
    ShowConsole,
    SilentWinWidgetSwap,
    MissionStateNotice,
    MissionStatePopup,
    ResetInput,
    PromoteFpsCheat,
    SherwoodTrading,
}

/// Ordered, typed work emitted at the post-tick boundary. Variant-specific
/// drains preserve the existing host phase priority and simulation timing.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
    Default,
)]
pub struct HostEffects {
    modals: Vec<ModalKind>,
    signals: Vec<HostSignal>,
    pub trade_receipts: Vec<crate::trading::TradeReceipt>,
    pub background_blits: Vec<PendingBgBlit>,
}

impl HostEffects {
    pub fn pending_modal_kinds(&self) -> Vec<engine_player_command::ModalKind> {
        self.modals.clone()
    }

    pub fn extend_dialogues(&mut self, ids: impl IntoIterator<Item = i32>) {
        self.modals.extend(
            ids.into_iter()
                .map(|dialog_id| ModalKind::Dialog { dialog_id }),
        );
    }

    pub fn extend_popup_texts(&mut self, ids: impl IntoIterator<Item = i32>) {
        self.modals.extend(
            ids.into_iter()
                .map(|text_id| ModalKind::PopupText { text_id }),
        );
    }

    pub fn extend_debriefings(
        &mut self,
        ids: impl IntoIterator<Item = engine_player_command::DebriefingTextId>,
    ) {
        self.modals.extend(
            ids.into_iter()
                .map(|text_id| ModalKind::Debriefing { text_id }),
        );
    }

    pub fn request_sherwood_report(&mut self) {
        if !self.has_sherwood_report() {
            self.modals.push(ModalKind::SherwoodReport);
        }
    }

    pub fn has_sherwood_report(&self) -> bool {
        self.modals.contains(&ModalKind::SherwoodReport)
    }

    pub fn take_sherwood_report(&mut self) -> bool {
        let Some(index) = self
            .modals
            .iter()
            .position(|request| *request == ModalKind::SherwoodReport)
        else {
            return false;
        };
        self.modals.remove(index);
        true
    }

    pub fn extend_trade_receipts(
        &mut self,
        receipts: impl IntoIterator<Item = crate::trading::TradeReceipt>,
    ) {
        self.trade_receipts.extend(receipts);
    }

    pub fn take_trade_receipts(&mut self) -> Vec<crate::trading::TradeReceipt> {
        std::mem::take(&mut self.trade_receipts)
    }

    pub fn dialogue_count(&self) -> usize {
        self.modals
            .iter()
            .filter(|request| matches!(request, ModalKind::Dialog { .. }))
            .count()
    }

    pub fn popup_text_count(&self) -> usize {
        self.modals
            .iter()
            .filter(|request| matches!(request, ModalKind::PopupText { .. }))
            .count()
    }

    pub fn debriefing_count(&self) -> usize {
        self.modals
            .iter()
            .filter(|request| matches!(request, ModalKind::Debriefing { .. }))
            .count()
    }

    /// Drain one presentation phase without repacking its requests.
    /// Requests in other phases remain pending; production order is retained
    /// within each phase. Modal consumers must not replace this with FIFO.
    pub fn take_modals(&mut self, phase: HostModalPhase) -> Vec<ModalKind> {
        self.modals
            .extract_if(.., |request| phase.contains(request))
            .collect()
    }

    pub fn request_signal(&mut self, signal: HostSignal) {
        if !self.signals.contains(&signal) {
            self.signals.push(signal);
        }
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

    /// Transfer deterministic output without translating its vocabulary. The host
    /// consumes audio/input first, modals by their explicit priority, and decals
    /// only at the next render pass. Replayed output is discarded before here.
    pub fn append(&mut self, mut incoming: Self) {
        if self.has_sherwood_report() {
            incoming
                .modals
                .retain(|request| *request != ModalKind::SherwoodReport);
        }
        self.modals.append(&mut incoming.modals);
        for signal in incoming.signals {
            self.request_signal(signal);
        }
        self.trade_receipts.append(&mut incoming.trade_receipts);
        self.background_blits.append(&mut incoming.background_blits);
    }

    pub fn clear(&mut self) {
        self.modals.clear();
        self.signals.clear();
        self.trade_receipts.clear();
        self.background_blits.clear();
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum HostModalPhase {
    Dialogue,
    Popup,
    Debriefing,
}

impl HostModalPhase {
    fn contains(self, request: &ModalKind) -> bool {
        matches!(
            (self, request),
            (Self::Dialogue, ModalKind::Dialog { .. })
                | (Self::Popup, ModalKind::PopupText { .. })
                | (Self::Debriefing, ModalKind::Debriefing { .. })
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admitted_ticks_preserve_modal_order_but_drain_only_the_requested_phase() {
        let mut pending = HostEffects::default();
        pending.extend_popup_texts([3]);
        pending.request_sherwood_report();
        pending.request_signal(HostSignal::ResetInput);
        let mut tick = HostEffects::default();
        tick.extend_dialogues([7]);
        tick.extend_popup_texts([4]);
        tick.extend_dialogues([8]);
        tick.request_sherwood_report();
        tick.request_signal(HostSignal::ResetInput);
        pending.append(tick);

        assert_eq!(
            pending.pending_modal_kinds(),
            vec![
                ModalKind::PopupText { text_id: 3 },
                ModalKind::SherwoodReport,
                ModalKind::Dialog { dialog_id: 7 },
                ModalKind::PopupText { text_id: 4 },
                ModalKind::Dialog { dialog_id: 8 },
            ]
        );
        assert_eq!(
            pending.take_modals(HostModalPhase::Dialogue),
            vec![
                ModalKind::Dialog { dialog_id: 7 },
                ModalKind::Dialog { dialog_id: 8 },
            ]
        );
        assert_eq!(
            pending.take_modals(HostModalPhase::Popup),
            vec![
                ModalKind::PopupText { text_id: 3 },
                ModalKind::PopupText { text_id: 4 }
            ]
        );
        assert!(pending.take_sherwood_report());
        assert!(!pending.take_sherwood_report());
        assert!(pending.take_signal(HostSignal::ResetInput));
        assert!(!pending.take_signal(HostSignal::ResetInput));
    }
}

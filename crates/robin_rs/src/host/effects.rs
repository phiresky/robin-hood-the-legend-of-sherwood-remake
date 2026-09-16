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

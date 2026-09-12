//! Cancellation ends eligibility, not already-admitted simulation work.
use super::Reply;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Queued,
    Admitted,
    Completed,
    Cancelled,
    Expired,
}

/// A single reply owner; serialization is observation, never live authority.
#[derive(Serialize)]
pub struct Responder {
    #[serde(skip)]
    router: Option<std::sync::Weak<Mutex<super::RequestRouter>>>,
    #[serde(skip)]
    tx: async_channel::Sender<Reply>,
    phase: Arc<Mutex<Phase>>,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(skip)]
    deadline: Option<std::time::Instant>,
}

#[derive(Serialize)]
pub(super) struct ReplyWait {
    #[serde(skip)]
    rx: async_channel::Receiver<Reply>,
    phase: Arc<Mutex<Phase>>,
}

robin_util::deny_deserialize!(Responder, "live RPC responder cannot be deserialized");
robin_util::deny_deserialize!(ReplyWait, "live RPC reply wait cannot be deserialized");

impl Responder {
    /// Read-only work may release its resources after the reply consumer leaves,
    /// even after admission. This does not cancel admitted simulation commands.
    pub(super) fn consumer_gone(&self) -> bool {
        self.tx.is_closed()
    }

    pub(super) fn channel() -> (Self, ReplyWait) {
        let (tx, rx) = async_channel::bounded(1);
        let phase = Arc::new(Mutex::new(Phase::Queued));
        (
            Self {
                router: None,
                tx,
                phase: phase.clone(),
                #[cfg(not(target_arch = "wasm32"))]
                deadline: None,
            },
            ReplyWait { rx, phase },
        )
    }

    pub(super) fn with_router(mut self, router: &super::Queue) -> Self {
        self.router = Some(Arc::downgrade(router));
        self
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn cancellation_observer(&self) -> Box<dyn Fn() -> bool> {
        let phase = self.phase.clone();
        Box::new(move || *phase.lock().expect("RPC lifetime poisoned") == Phase::Cancelled)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn with_deadline(mut self, deadline: std::time::Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    fn refresh(&self, phase: &mut Phase) {
        if *phase != Phase::Queued {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            *phase = Phase::Expired;
            return;
        }
        if self.tx.is_closed() {
            *phase = Phase::Cancelled;
        }
    }

    pub(super) fn eligible(&self) -> bool {
        let mut phase = self.phase.lock().expect("RPC lifetime poisoned");
        self.refresh(&mut phase);
        *phase == Phase::Queued
    }

    /// Linearization point before work leaves cancellable ingress ownership.
    pub(super) fn admit(&self) -> bool {
        // Retirement and admission linearize on the router lock. Keep the
        // guard until the phase transition, in router -> lifetime lock order.
        let router = self.router.as_ref().and_then(std::sync::Weak::upgrade);
        let route = router
            .as_ref()
            .map(|router| router.lock().expect("RPC router poisoned"));
        let mut phase = self.phase.lock().expect("RPC lifetime poisoned");
        if self.router.is_some() && route.as_ref().is_none_or(|route| route.is_retired()) {
            if *phase == Phase::Queued {
                *phase = Phase::Cancelled;
            }
            return false;
        }
        self.refresh(&mut phase);
        if *phase != Phase::Queued {
            return false;
        }
        *phase = Phase::Admitted;
        true
    }

    pub fn send(self, reply: Reply) {
        let mut phase = self.phase.lock().expect("RPC lifetime poisoned");
        if matches!(*phase, Phase::Cancelled | Phase::Expired) {
            return;
        }
        *phase = Phase::Completed;
        if let Err(error) = self.tx.try_send(reply) {
            tracing::debug!("script RPC: response consumer gone: {error}");
        }
    }
}

impl ReplyWait {
    pub(super) async fn recv(&self) -> Result<Reply, async_channel::RecvError> {
        self.rx.recv().await
    }

    #[cfg(test)]
    pub(super) fn try_recv(&self) -> Result<Reply, async_channel::TryRecvError> {
        self.rx.try_recv()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn expire(&self) {
        let mut phase = self.phase.lock().expect("RPC lifetime poisoned");
        if *phase == Phase::Queued {
            *phase = Phase::Expired;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn is_expired(&self) -> bool {
        *self.phase.lock().expect("RPC lifetime poisoned") == Phase::Expired
    }
}

impl Drop for ReplyWait {
    fn drop(&mut self) {
        let mut phase = self.phase.lock().expect("RPC lifetime poisoned");
        if *phase == Phase::Queued {
            *phase = Phase::Cancelled;
        }
        self.rx.close();
    }
}

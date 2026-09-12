//! Shared worker ownership, timing and cancellation; dispatch policy stays with each codec.
use anyhow::{Result, anyhow};
use futures_channel::oneshot;
use futures_util::{StreamExt, stream::FuturesUnordered};

type Completion<C, O> = (C, Result<O>, Option<[f64; 4]>);

/// Transient worker state, never serialized.
pub(super) struct DecodeJobs<C, O> {
    jobs: FuturesUnordered<oneshot::Receiver<Completion<C, O>>>,
}

impl<C, O> Default for DecodeJobs<C, O> {
    fn default() -> Self {
        Self {
            jobs: FuturesUnordered::new(),
        }
    }
}

impl<C: Send + 'static, O: Send + 'static> DecodeJobs<C, O> {
    pub(super) fn spawn(
        &mut self,
        chunk: C,
        ready: Option<f64>,
        decode: impl FnOnce(&C) -> Result<O> + Send + 'static,
    ) {
        let (sender, receiver) = oneshot::channel();
        let enqueued = ready.map(|_| js_sys::Date::now());
        rayon::spawn(move || {
            // Date.now shares an epoch across browser workers.
            let started = ready.map(|_| js_sys::Date::now());
            let output = decode(&chunk);
            let timing = ready
                .zip(enqueued)
                .zip(started)
                .map(|((ready, enqueued), started)| {
                    [ready, enqueued, started, js_sys::Date::now()]
                });
            // A dropped receiver means its dispatcher has already stopped.
            let _ = sender.send((chunk, output, timing));
        });
        self.jobs.push(receiver);
    }

    /// Cancel-safe: an unconsumed result remains in the queue.
    pub(super) async fn next(&mut self, label: &str) -> Result<Option<Completion<C, O>>> {
        self.jobs
            .next()
            .await
            .map(|result| result.map_err(|_| anyhow!("{label} decode worker dropped its result")))
            .transpose()
    }

    pub(super) fn len(&self) -> usize {
        self.jobs.len()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

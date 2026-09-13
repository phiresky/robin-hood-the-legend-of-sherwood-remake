//! Browser autosave writer. Everything runs on the page thread: urgent
//! lifecycle writes publish synchronously, periodic writes drain on the next
//! microtask, and deferred-thumbnail captures publish when their GPU readback
//! resolves, ordered by [`AutosavePublicationOrder`].

use super::{
    AutosaveCompletion, AutosaveCoordinator, AutosaveJob, AutosavePublicationOrder, AutosaveReason,
    AutosaveRequest, write_job,
};
use crate::save_file::Thumbnail;
use crate::savegame::SaveGameManager;
use anyhow::{Context, Result, bail};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

#[derive(Default)]
pub(super) struct AutosaveWriter {
    publication_order: Rc<RefCell<AutosavePublicationOrder>>,
    completions: Rc<RefCell<VecDeque<AutosaveCompletion>>>,
    pending_jobs: Rc<RefCell<VecDeque<AutosaveJob>>>,
    writer_running: Rc<Cell<bool>>,
}

impl AutosaveWriter {
    /// Publish urgent jobs now (returning their error to the caller); queue
    /// periodic jobs for the next microtask.
    pub(super) fn submit(&self, job: AutosaveJob) -> Result<()> {
        let reason = job.reason;
        if reason != AutosaveReason::Periodic {
            // A lifecycle callback may be the page's last executable
            // turn before it is frozen or discarded. Publish urgent
            // payload+manifest bytes now and return the error to the
            // caller instead of merely queueing a failed completion.
            let completion = write_job(&job)
                .with_context(|| format!("publishing urgent {reason:?} browser autosave"))?;
            self.publication_order
                .borrow_mut()
                .published(job.capture_sequence);
            self.completions.borrow_mut().push_back(completion);
            self.writer_running.set(false);
        } else {
            self.pending_jobs.borrow_mut().push_back(job);
            if !self.writer_running.replace(true) {
                let completions = self.completions.clone();
                let pending_jobs = self.pending_jobs.clone();
                let writer_running = self.writer_running.clone();
                let publication_order = self.publication_order.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    // No timer precedes persistence. Periodic writes move
                    // to the next microtask only.
                    drain_browser_jobs(&pending_jobs, &completions, &publication_order);
                    writer_running.set(false);
                });
            }
        }
        Ok(())
    }

    pub(super) fn take_completions(&self) -> Vec<AutosaveCompletion> {
        std::mem::take(&mut *self.completions.borrow_mut()).into()
    }

    /// Publish still-queued periodic jobs and retire the session so a pending
    /// initial thumbnail cannot resurrect a checkpoint afterwards.
    pub(super) fn shutdown(&mut self) {
        if !self.pending_jobs.borrow().is_empty() {
            drain_browser_jobs(
                &self.pending_jobs,
                &self.completions,
                &self.publication_order,
            );
            self.writer_running.set(false);
        }
        self.publication_order.borrow_mut().retired = true;
    }
}

fn drain_browser_jobs(
    pending_jobs: &Rc<RefCell<VecDeque<AutosaveJob>>>,
    completions: &Rc<RefCell<VecDeque<AutosaveCompletion>>>,
    publication_order: &Rc<RefCell<AutosavePublicationOrder>>,
) {
    while let Some(job) = pending_jobs.borrow_mut().pop_front() {
        if !publication_order.borrow().permits(job.capture_sequence) {
            tracing::info!(
                filename = job.filename,
                "Queued autosave superseded by a newer checkpoint or retired session"
            );
            continue;
        }
        let completion = match write_job(&job) {
            Ok(completion) => {
                publication_order
                    .borrow_mut()
                    .published(job.capture_sequence);
                completion
            }
            Err(error) => AutosaveCompletion::failed(&job, error),
        };
        completions.borrow_mut().push_back(completion);
    }
}

impl AutosaveCoordinator {
    /// Snapshot mission-entry state now, but finish its already-submitted GPU
    /// thumbnail independently of the live frame. Real exits/background events
    /// still use enqueue's synchronous publication path.
    pub(crate) fn enqueue_initial_with_thumbnail(
        &mut self,
        manager: &SaveGameManager,
        request: AutosaveRequest<'_>,
        thumbnail: std::pin::Pin<Box<dyn std::future::Future<Output = Option<Thumbnail>>>>,
    ) -> Result<()> {
        let mission_id = request.mission_id;
        let planned = self.planned.context("missing initial autosave plan")?;
        if planned.frame != 0
            || planned.reason != AutosaveReason::MissionTransition
            || self.schedule.mission_id == Some(mission_id)
        {
            bail!("deferred thumbnail requires the initial mission-entry autosave");
        }
        let started = web_time::Instant::now();
        let (mut job, planned) =
            self.prepare_job(manager, request, None, AutosaveReason::MissionTransition)?;
        tracing::debug!(
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "initial autosave: immutable payload capture"
        );
        let completions = self.writer.completions.clone();
        let publication_order = self.writer.publication_order.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let thumbnail = thumbnail.await;
            if !publication_order.borrow().permits(job.capture_sequence) {
                tracing::info!(
                    filename = job.filename,
                    "Initial autosave superseded by a newer checkpoint or retired session"
                );
                return;
            }
            // write_job is synchronous on the browser thread, including both
            // payload and manifest publication. There is no await/reentrant
            // callback between this ordering check and advancing the watermark.
            // Thumbnail capture already logs its failure. Preserve the valid
            // recovery payload even when the optional preview is unavailable,
            // just as the awaited autosave path does.
            job.thumbnail = thumbnail;
            let completion = match write_job(&job) {
                Ok(completion) => {
                    publication_order
                        .borrow_mut()
                        .published(job.capture_sequence);
                    completion
                }
                Err(error) => AutosaveCompletion::failed(&job, error),
            };
            completions.borrow_mut().push_back(completion);
        });
        self.schedule
            .commit(planned.mission_id, planned.frame, planned.reason);
        self.planned = None;
        Ok(())
    }
}

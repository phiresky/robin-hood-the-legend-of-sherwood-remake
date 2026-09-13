//! Native autosave writer: one dedicated thread persists accepted jobs in
//! submission order, off the game thread.

use super::{AutosaveCompletion, AutosaveJob, write_job};
use anyhow::{Context, Result};

pub(super) struct AutosaveWriter {
    command_tx: std::sync::mpsc::Sender<Option<AutosaveJob>>,
    completion_rx: std::sync::mpsc::Receiver<AutosaveCompletion>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
}

impl Default for AutosaveWriter {
    fn default() -> Self {
        let (command_tx, command_rx) = std::sync::mpsc::channel::<Option<AutosaveJob>>();
        let (completion_tx, completion_rx) = std::sync::mpsc::channel::<AutosaveCompletion>();
        let writer_thread = std::thread::Builder::new()
            .name("autosave-writer".to_owned())
            .spawn(move || {
                while let Ok(Some(job)) = command_rx.recv() {
                    let completion = match write_job(&job) {
                        Ok(completion) => completion,
                        Err(error) => AutosaveCompletion::failed(&job, error),
                    };
                    if completion_tx.send(completion).is_err() {
                        break;
                    }
                }
            })
            .expect("failed to spawn the autosave writer thread");
        Self {
            command_tx,
            completion_rx,
            writer_thread: Some(writer_thread),
        }
    }
}

impl AutosaveWriter {
    /// Queue an accepted job; its completion surfaces through
    /// [`Self::take_completions`].
    pub(super) fn submit(&self, job: AutosaveJob) -> Result<()> {
        self.command_tx
            .send(Some(job))
            .context("autosave writer thread is unavailable")
    }

    pub(super) fn take_completions(&self) -> Vec<AutosaveCompletion> {
        self.completion_rx.try_iter().collect()
    }

    /// Finish every accepted write and join the writer thread.
    pub(super) fn shutdown(&mut self) {
        if self.writer_thread.is_none() {
            return;
        }
        if self.command_tx.send(None).is_err() {
            tracing::error!("autosave writer command channel closed before shutdown");
        }
        if let Some(handle) = self.writer_thread.take()
            && handle.join().is_err()
        {
            tracing::error!("autosave writer thread panicked during shutdown");
        }
    }
}

impl Drop for AutosaveWriter {
    fn drop(&mut self) {
        self.shutdown();
    }
}

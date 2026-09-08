//! One bounded physical save operation. The save-store owner joins completion
//! before publishing metadata or retiring its directory authority.

use crate::savegame::{SaveGame, SlotName};
use anyhow::{Context, Result};

/// Runtime authority: a running thread cannot be cloned or deserialized.
#[derive(Debug, Default)]
pub(crate) struct SaveOperationOwner {
    pending: Option<(SlotName, std::thread::JoinHandle<Result<SaveGame>>)>,
}

impl SaveOperationOwner {
    pub(crate) fn pending_name(&self) -> Option<&SlotName> {
        self.pending.as_ref().map(|(name, _)| name)
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|(_, worker)| worker.is_finished())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start(
        &mut self,
        name: SlotName,
        work: impl FnOnce() -> Result<SaveGame> + Send + 'static,
    ) -> Result<()> {
        anyhow::ensure!(
            self.pending.is_none(),
            "previous save must complete before another can start"
        );
        let worker = std::thread::Builder::new()
            .name("owned-save".into())
            .spawn(work)
            .context("starting owned save worker")?;
        self.pending = Some((name, worker));
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> Result<Option<(SlotName, SaveGame)>> {
        let Some((name, worker)) = self.pending.take() else {
            return Ok(None);
        };
        let metadata = worker
            .join()
            .map_err(|_| anyhow::anyhow!("save worker panicked"))??;
        Ok(Some((name, metadata)))
    }
}

impl Drop for SaveOperationOwner {
    fn drop(&mut self) {
        if self.pending.is_some() {
            tracing::error!("save operation owner dropped without explicit completion");
            if let Err(error) = self.finish() {
                tracing::error!("joining retired save worker failed: {error:#}");
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn delayed_operation_is_bounded_and_joined_before_retirement() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (retired_tx, retired_rx) = mpsc::channel();
        let mut owner = SaveOperationOwner::default();
        owner
            .start(SlotName::new("Continue").unwrap(), move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(SaveGame::new("Continue".into(), "Complete".into(), 1))
            })
            .unwrap();
        started_rx.recv().unwrap();
        assert!(!owner.is_finished());
        assert!(
            owner
                .start(SlotName::new("Restart").unwrap(), || unreachable!())
                .is_err()
        );
        let retirement = std::thread::spawn(move || {
            let completed = owner.finish().unwrap().unwrap();
            retired_tx.send(completed).unwrap();
        });
        assert!(matches!(
            retired_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        release_tx.send(()).unwrap();
        assert_eq!(retired_rx.recv().unwrap().0.as_str(), "Continue");
        retirement.join().unwrap();
    }

    #[test]
    fn physical_failure_is_returned_not_reported_as_completion() {
        let mut owner = SaveOperationOwner::default();
        owner
            .start(SlotName::new("Continue").unwrap(), || {
                anyhow::bail!("injected write failure")
            })
            .unwrap();
        assert!(
            owner
                .finish()
                .unwrap_err()
                .to_string()
                .contains("injected write failure")
        );
        assert!(owner.pending_name().is_none());
    }
}

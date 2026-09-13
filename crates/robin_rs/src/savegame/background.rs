//! Background publication completion, sticky operation errors, and the
//! owned-save recovery receipt.
use super::*;

impl SaveGameManager {
    /// Polling never blocks on serialization. Errors remain sticky until the
    /// caller explicitly reopens the store, so failed publication cannot be
    /// mistaken for an idle successful writer on the next frame.
    pub fn poll_background(&mut self) -> Result<bool> {
        if self.operation_error.is_some() {
            if self.operation_error_reported {
                return Ok(false);
            }
            self.operation_error_reported = true;
            return self.check_operation_error().map(|()| false);
        }
        self.check_operation_error()?;
        if !self.operations.is_finished() {
            return Ok(false);
        }
        if let Err(error) = self.finish_background() {
            self.operation_error_reported = true;
            return Err(error);
        }
        Ok(true)
    }

    pub fn finish_background(&mut self) -> Result<()> {
        self.check_operation_error()?;
        let completion = self.operations.finish();
        let result = (|| {
            let Some((name, metadata)) = completion? else {
                return Ok(());
            };
            let index = self
                .find_by_filename(name.as_str())
                .context("completed save lost its owned slot")?;
            self.finish_publication(index, metadata)
        })();
        if let Err(error) = &result {
            self.operation_error = Some(format!(
                "owned save publication failed: {error:#}; reopen the save store before further operations"
            ));
        }
        result
    }

    /// Nonblocking completion fence for bootstrap. Unlike notification polling,
    /// this reports a failed publication every time: consuming an error banner
    /// must never turn a failed Restart save into a completed frame-zero save.
    pub(crate) fn try_finish_background(&mut self) -> Result<SaveWriteStatus> {
        self.check_operation_error()?;
        if self.operations.pending_name().is_some() && !self.operations.is_finished() {
            return Ok(SaveWriteStatus::Queued);
        }
        self.finish_background()?;
        Ok(SaveWriteStatus::Completed)
    }

    pub(super) fn check_operation_error(&self) -> Result<()> {
        if let Some(error) = &self.operation_error {
            anyhow::bail!("{error}");
        }
        Ok(())
    }

    pub(super) fn owned_recovery_path(&self) -> PathBuf {
        Path::new(&self.save_directory).join("owned-save-recovery.json")
    }

    pub(super) fn retire_owned_receipt(&self) -> Result<()> {
        match std::fs::remove_file(self.owned_recovery_path()) {
            Ok(()) => sync_save_directory(&self.save_directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("retire owned save recovery receipt"),
        }
    }

    pub(super) fn reconcile_owned_save(&mut self) -> Result<()> {
        if let Some(slot) =
            recovery::owned_candidate(&self.save_directory, &self.owned_recovery_path())?
        {
            self.catalog.upsert(slot, SlotState::Published)?;
            self.publish_index().map_err(anyhow::Error::msg)?;
        }
        self.retire_owned_receipt()
    }
}

impl Drop for SaveGameManager {
    fn drop(&mut self) {
        if self.operations.pending_name().is_some() {
            tracing::warn!(
                "save manager retired without explicit finish; joining outstanding publication"
            );
            if let Err(error) = self.finish_background() {
                tracing::error!("retired save publication failed: {error:#}");
            }
        }
    }
}

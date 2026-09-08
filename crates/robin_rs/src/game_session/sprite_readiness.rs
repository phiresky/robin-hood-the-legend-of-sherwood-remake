//! Host-only barriers for the opt-in first-frame sprite streaming experiment.
use robin_assets::late_sprites::{self, Readiness};
use robin_engine::engine::Engine;

fn completed(readiness: Readiness) -> Result<bool, String> {
    match readiness {
        Readiness::Ready => Ok(true),
        Readiness::Pending => Ok(false),
        Readiness::Failed(error) => Err(format!("sprite streaming failed: {error}")),
        Readiness::Superseded => Err("sprite streaming mission epoch was superseded".into()),
    }
}

pub(super) async fn wait_for_render_sprites(engine: &Engine) -> Result<(), String> {
    let Some(epoch) = late_sprites::experimental_epoch() else {
        return Ok(());
    };
    let ids = crate::game_render::required_render_sprite_ids(engine);
    let mut pending_since = None;
    let result = loop {
        match completed(late_sprites::readiness(epoch, &ids)) {
            Ok(true) => break Ok(()),
            Err(error) => break Err(error),
            Ok(false) => {
                pending_since.get_or_insert_with(|| {
                    tracing::debug!(epoch, required_ids = ?ids, "Sprite render preflight pending");
                    web_time::Instant::now()
                });
                crate::window::yield_to_runtime().await;
                crate::window::sleep_ms(10).await;
            }
        }
    };
    if let Some(started) = pending_since {
        tracing::info!(
            epoch,
            required_ids_count = ids.len(),
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            ready = result.is_ok(),
            "Sprite render preflight wait finished"
        );
    }
    result
}

/// Reclassification changes opacity/shadow interpretation, so all worker
/// publication must finish before runtime ambiance rebinding starts.
pub(super) async fn wait_for_all_sprites() -> Result<(), String> {
    let Some(epoch) = late_sprites::experimental_epoch() else {
        return Ok(());
    };
    let mut pending_since = None;
    let result = loop {
        match completed(late_sprites::all_readiness(epoch)) {
            Ok(true) => break Ok(()),
            Err(error) => break Err(error),
            Ok(false) => {
                pending_since.get_or_insert_with(web_time::Instant::now);
                crate::window::yield_to_runtime().await;
                crate::window::sleep_ms(10).await;
            }
        }
    };
    if let Some(started) = pending_since {
        tracing::info!(
            epoch,
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            ready = result.is_ok(),
            "All sprite publication wait finished"
        );
    }
    result
}

pub(super) fn assert_render_sprites_ready(engine: &Engine) {
    if let Some(epoch) = late_sprites::experimental_epoch() {
        let ids = crate::game_render::required_render_sprite_ids(engine);
        assert!(
            completed(late_sprites::readiness(epoch, &ids))
                .unwrap_or_else(|error| panic!("{error}")),
            "render_frame entered before required sprite pixels were resident"
        );
    }
}

pub(super) fn assert_all_sprites_ready() {
    if let Some(epoch) = late_sprites::experimental_epoch() {
        assert!(
            completed(late_sprites::all_readiness(epoch)).unwrap_or_else(|error| panic!("{error}")),
            "ambiance rebinding entered before deferred sprite publication finished"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_errors_never_turn_into_waits_or_success() {
        assert_eq!(completed(Readiness::Ready), Ok(true));
        assert_eq!(completed(Readiness::Pending), Ok(false));
        assert!(
            completed(Readiness::Failed("download failed".into()))
                .unwrap_err()
                .contains("download failed")
        );
        assert!(
            completed(Readiness::Superseded)
                .unwrap_err()
                .contains("superseded")
        );
    }
}

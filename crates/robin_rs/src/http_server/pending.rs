//! Deferred requests handed to the mission loop: screenshots and timeline steps.

use super::*;

// ──────────────────────────────────────────────────────────────────
// Screenshot pipeline
// ──────────────────────────────────────────────────────────────────

/// A screenshot request waiting for the next rendered frame.
///
/// The caller (the main loop) is expected to:
/// 1. Clone the live `DevState` and feed the per-request
///    [`ScreenshotFlags`] through [`apply_screenshot_flags`].
/// 2. Render a throwaway frame with that dev clone into the offscreen
///    target.
/// 3. Submit readback (`Renderer::begin_capture_frame_rgba`) and retain the
///    future and this responder in the mission's bounded capture queue.
/// 4. Consume this struct via [`PendingScreenshot::respond`], handing
///    over the pixels so the request replies with `image/png`.
/// Submission clears recorded commands; completion never borrows the live
/// renderer. Ending the mission retires outstanding replies and readbacks.
pub struct PendingScreenshot {
    pub(super) response_tx: Responder,
    pub(super) request: ScreenshotRequest,
}

impl PendingScreenshot {
    /// Full screenshot options shared by viewport and full-map captures.
    pub fn request(&self) -> &ScreenshotRequest {
        &self.request
    }

    /// Encode the captured RGBA frame as PNG (applying the request's
    /// optional crop + resize) and send the reply to the HTTP client.
    /// Consumes `self` — callers get one shot.
    pub fn respond(self, src_w: u32, src_h: u32, rgba: &[u8]) {
        let reply = encode_png(src_w, src_h, rgba, &self.request);
        self.response_tx.send(reply);
    }

    /// Reply with an error string instead of a PNG (e.g. when pixel
    /// readback failed).  Consumes `self`.
    pub fn respond_err(self, error: RpcError) {
        self.response_tx.send(Err(error));
    }
}

// ──────────────────────────────────────────────────────────────────
// Step-forward / step-back pipeline
// ──────────────────────────────────────────────────────────────────

/// A step-forward / step-back request waiting for the main loop to
/// drive the engine.  The main loop is expected to
/// [`SessionIngress::take_pending_steps`] once per frame and, for each request, either:
///
/// - run `n` full frame-equivalent ticks (`Forward`), or
/// - rewind `n` frames through the rewind buffer (`Back`),
///
/// then reply via [`PendingStep::respond_ok`] /
/// [`PendingStep::respond_err`].  Refuse to run when the game has
/// modal state queued (dialog / briefing / scroll) — advancing the
/// sim while a modal is pending would skip past the modal.
pub struct PendingStep {
    pub(super) response_tx: Responder,
    pub kind: StepKind,
}

impl PendingStep {
    pub fn respond_ok(self, body: serde_json::Value) {
        self.response_tx.send(Ok(ReplyBody::Json(body)));
    }

    pub fn respond_err(self, error: RpcError) {
        self.response_tx.send(Err(error));
    }
}

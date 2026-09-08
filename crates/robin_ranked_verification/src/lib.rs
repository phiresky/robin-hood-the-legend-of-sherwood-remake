//! Dependency-minimal deterministic ranked replay preparation.
//!
//! This crate is the shared authority used by the interactive client and the
//! isolated production verifier. It deliberately owns no renderer, window,
//! audio device, video decoder, gamepad, or multiplayer transport.

mod mission_loading;
mod profile_loading;
pub mod ranked_verifier;
pub mod replay_campaign_validation;

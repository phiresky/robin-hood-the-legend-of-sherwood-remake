//! Hostile-input boundary for ranked replay verification.
//!
//! The binary is intentionally a one-job worker. Public submission and result
//! types live in `robin_run_protocol`; this crate owns only private deployment
//! configuration, content-mount verification, bounded replay admission, and
//! the renderer/audio/network-free re-simulation adapter.

pub mod content_manifest;
pub mod job_config;
pub mod request_auth;
pub mod result_projection;
pub mod worker;
pub mod worker_process;

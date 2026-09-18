//! Hostile-input boundary for ranked replay verification.
//!
//! The binary is a one-job worker launched inside the leaderboard worker's
//! bwrap/prlimit sandbox. It reads a trusted `VerifierJobV2`, the hostile
//! compact replay and the read-only raw game content, resimulates the replay
//! and writes one `VerifierOutputV2`.

pub mod ranked_verification;
pub mod result_projection;
pub mod worker;
pub mod worker_process;

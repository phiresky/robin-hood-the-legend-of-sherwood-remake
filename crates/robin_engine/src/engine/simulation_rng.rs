//! Focused engine simulation rng ownership and behavior.
use super::*;

// ─── Deterministic simulation RNG ─────────────────────────────────────────

/// Engine-owned capability for deterministic gameplay randomness.
///
/// This always owns the one `fastrand::Rng`. Gameplay receives an explicit
/// [`crate::sim_rng::SimulationContext`] handle tied to this allocation;
/// cloning an engine snapshot deep-copies the current stream state rather than
/// sharing it with the live engine.
///
/// The original game seeds the one
/// process-wide C RNG, and gameplay consumers call that shared `rand()`
/// stream. Rust keeps ownership explicit so replay/save snapshots can carry
/// the exact corresponding state.
pub(crate) struct SimulationRng {
    state: Arc<Mutex<fastrand::Rng>>,
    original_replay: Option<Arc<Mutex<crate::sim_rng::OriginalRngReplay>>>,
}

impl Clone for SimulationRng {
    fn clone(&self) -> Self {
        Self {
            state: Arc::new(Mutex::new(
                self.state
                    .lock()
                    .expect("simulation RNG mutex poisoned")
                    .clone(),
            )),
            original_replay: self.original_replay.as_ref().map(|replay| {
                Arc::new(Mutex::new(
                    replay
                        .lock()
                        .expect("original RNG replay mutex poisoned")
                        .clone(),
                ))
            }),
        }
    }
}

impl SimulationRng {
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn with_seed(seed: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(fastrand::Rng::with_seed(seed))),
            original_replay: None,
        }
    }

    pub(crate) fn with_original_replay(draws: Vec<u32>) -> Self {
        let mut rng = Self::with_seed(0);
        rng.original_replay = Some(Arc::new(Mutex::new(
            crate::sim_rng::OriginalRngReplay::new(draws),
        )));
        rng
    }

    pub(crate) fn context(
        &self,
        config: crate::engine::SimConfig,
    ) -> crate::sim_rng::SimulationContext {
        crate::sim_rng::SimulationContext::new(
            Arc::clone(&self.state),
            self.original_replay.as_ref().map(Arc::clone),
            config,
        )
    }

    pub(crate) fn seed(&self) -> u64 {
        self.state
            .lock()
            .expect("simulation RNG mutex poisoned")
            .get_seed()
    }

    pub(crate) fn persisted_seed(&self) -> Result<u64, String> {
        if self.original_replay.is_some() {
            return Err("original RNG parity replay cannot be serialized".into());
        }
        Ok(self.seed())
    }

    #[allow(clippy::disallowed_methods)]
    pub(crate) fn reseed(&mut self, seed: u64) {
        *self.state.lock().expect("simulation RNG mutex poisoned") = fastrand::Rng::with_seed(seed);
        self.original_replay = None;
    }

    pub(crate) fn append_original_replay(&mut self, draws: Vec<u32>) {
        self.original_replay
            .as_ref()
            .expect("original RNG replay is not active")
            .lock()
            .expect("original RNG replay mutex poisoned")
            .append(draws);
    }

    pub(crate) fn replace_original_replay(&mut self, draws: Vec<u32>) {
        self.original_replay = Some(Arc::new(Mutex::new(
            crate::sim_rng::OriginalRngReplay::new(draws),
        )));
    }

    pub(crate) fn original_replay_cursor(&self) -> Option<usize> {
        self.original_replay.as_ref().map(|replay| {
            replay
                .lock()
                .expect("original RNG replay mutex poisoned")
                .cursor()
        })
    }

    pub(crate) fn original_replay_sites(
        &self,
        range: std::ops::Range<usize>,
    ) -> Option<Vec<crate::sim_rng::RngSite>> {
        self.original_replay.as_ref().map(|replay| {
            replay
                .lock()
                .expect("original RNG replay mutex poisoned")
                .sites(range)
        })
    }

    pub(crate) fn original_replay_diagnostics(
        &self,
        range: std::ops::Range<usize>,
    ) -> Option<crate::sim_rng::OriginalRngDiagnostics> {
        self.original_replay.as_ref().map(|replay| {
            replay
                .lock()
                .expect("original RNG replay mutex poisoned")
                .diagnostics(range)
        })
    }

    /// Clone the normal PRNG state while deliberately omitting the
    /// non-serializable Original parity draw stream.
    ///
    /// This is only for diagnostic snapshots whose surrounding record carries
    /// the parity cursor separately. It must not be used for rollback, saves,
    /// or replay adoption, because those need the live draw capability.
    pub(crate) fn clone_without_original_replay(&self) -> Self {
        Self {
            state: Arc::new(Mutex::new(
                self.state
                    .lock()
                    .expect("simulation RNG mutex poisoned")
                    .clone(),
            )),
            original_replay: None,
        }
    }
}

impl Serialize for SimulationRng {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.persisted_seed()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SimulationRng {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u64::deserialize(deserializer).map(Self::with_seed)
    }
}

impl crate::bitcode_adapters::NativeBitcode for SimulationRng {
    type Wire = u64;

    fn to_wire(&self) -> Self::Wire {
        assert!(
            self.original_replay.is_none(),
            "original RNG parity replay cannot be encoded in an Engine snapshot"
        );
        self.seed()
    }

    fn from_wire(seed: Self::Wire) -> Self {
        Self::with_seed(seed)
    }
}

crate::bitcode_adapters::impl_native_bitcode!(SimulationRng);

impl robin_util::state_hash::StateHash for SimulationRng {
    fn state_hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        robin_util::state_hash::StateHash::state_hash(
            &*self.state.lock().expect("simulation RNG mutex poisoned"),
            hasher,
        );
        if let Some(replay) = &self.original_replay {
            replay
                .lock()
                .expect("original RNG replay mutex poisoned")
                .state_hash(hasher);
        }
    }
}

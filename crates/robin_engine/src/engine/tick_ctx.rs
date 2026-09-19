//! Immutable per-tick inputs threaded through engine code as one parameter.

use super::LevelAssets;
use crate::sim_rng::SimulationContext;

/// The simulation context and level assets of the tick being executed.
///
/// Both references are shared, so the context is `Copy` and is passed by
/// value. It never owns or recreates the simulation context: the RNG state
/// behind `sim` is the one the caller threaded in.
#[derive(Clone, Copy)]
pub(crate) struct TickCtx<'a> {
    pub(crate) sim: &'a SimulationContext,
    pub(crate) assets: &'a LevelAssets,
}

impl<'a> TickCtx<'a> {
    pub(crate) fn new(sim: &'a SimulationContext, assets: &'a LevelAssets) -> Self {
        Self { sim, assets }
    }
}

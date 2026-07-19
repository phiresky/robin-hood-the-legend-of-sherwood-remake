//! Domain routing for synchronous native calls.
//!
//! The declarative native registry remains the sole owner of IDs and
//! signatures. This module only routes an already-decoded native to a
//! cohesive implementation module.

use super::*;

mod actors;
mod ai;
mod campaign;
mod script_core;
mod sequences;
mod world;

pub(super) fn call_immediate(
    context: &mut NativeContext<'_, '_>,
    index: u32,
    stack: &mut NativeStack,
) -> i32 {
    let Ok(native) = NativeFn::try_from(index) else {
        // We cannot drain the stack because an unknown ID has no signature.
        // A malformed SCB calling outside the declarative registry is already
        // invalid, but retaining the zero result matches the prior adapter.
        tracing::error!("Unknown native function index {index}");
        return 0;
    };

    context.dispatch_script_core(native, stack)
}

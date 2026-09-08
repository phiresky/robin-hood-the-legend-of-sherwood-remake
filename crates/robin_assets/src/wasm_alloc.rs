//! Spin-locked talc as the global allocator on every wasm build.
//!
//! std's default wasm allocator (dlmalloc) guards the whole heap with one
//! futex lock. The worker-pool sprite decode is allocation heavy, and under
//! that lock decode throughput peaks around 4 workers and then inverts
//! (measured, H01 in headless Chrome: serial 11.2 s, 4 workers 6.6 s,
//! 12 workers 12.2 s). With [`talc`]'s much shorter spinlocked critical
//! section the inversion disappears (8/12 workers ~7.5 s) and even the
//! single-threaded decode gets ~9% faster (11.2 -> 10.2 s), so the swap is
//! unconditional for wasm rather than tied to `wasm-threads`.
//!
//! `TalcLock` is the crate's multithread-capable form (the single-threaded
//! `TalcSyncCell` wasm helpers panic on atomics builds); on a non-atomics
//! build the spinlock is uncontended by construction.

use talc::sync::TalcLock;
use talc::wasm::{WasmBinning, WasmGrowAndClaim};

// `WasmGrowAndClaim` rather than `WasmGrowAndExtend`: the extending source
// caches the previous heap end as a `NonNull` and is therefore `!Send`,
// which a multithreaded `static` allocator must be. Claiming fresh heaps
// costs a little fragmentation and saves nothing else.
#[global_allocator]
static TALC_ALLOCATOR: TalcLock<spinning_top::RawSpinlock, WasmGrowAndClaim, WasmBinning> =
    TalcLock::new(WasmGrowAndClaim);

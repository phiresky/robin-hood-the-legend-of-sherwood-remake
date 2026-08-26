//! Native-bitcode codec for complete engine snapshots.
//!
//! The engine model is finite, but its generated native-bitcode encoder and
//! decoder are large, deeply nested values. Constructing them through
//! `bitcode::encode` / `bitcode::decode` can exhaust an ordinary native thread
//! stack before any game data is visited. Keep that implementation detail out
//! of callers and perform the rare network snapshot operation on a bounded,
//! explicitly sized stack.

#[cfg(not(target_arch = "wasm32"))]
const CODEC_STACK_BYTES: usize = 32 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
fn with_codec_stack<T: Send>(operation: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("engine-snapshot-codec".into())
            .stack_size(CODEC_STACK_BYTES)
            .spawn_scoped(scope, operation)
            .expect("spawn native engine snapshot codec worker");
        match worker.join() {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })
}

/// Encode a large snapshot using native bitcode.
pub fn encode<T: bitcode::Encode + Sync + ?Sized>(value: &T) -> Vec<u8> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        return with_codec_stack(|| bitcode::encode(value));
    }

    // Browser builds do not have native scoped threads. Multiplayer engine
    // snapshots are currently a native transport path.
    // TODO(wasm-multiplayer): use a worker-backed codec before enabling full
    // engine snapshot exchange in browsers.
    #[cfg(target_arch = "wasm32")]
    bitcode::encode(value)
}

/// Decode a large native-bitcode snapshot.
pub fn decode<T: bitcode::DecodeOwned + Send>(bytes: &[u8]) -> Result<T, bitcode::Error> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        return with_codec_stack(|| bitcode::decode(bytes));
    }

    #[cfg(target_arch = "wasm32")]
    bitcode::decode(bytes)
}

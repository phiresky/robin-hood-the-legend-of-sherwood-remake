//! Dedicated native compact-replay validator; stdout is only the bounded reply.
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if std::env::args_os().len() != 1 {
        std::process::exit(64);
    }
    std::process::exit(robin_replay_format::native_admission::run_native_admission_worker());
}

#[cfg(target_arch = "wasm32")]
fn main() {
    panic!("native replay admission requires a native target");
}

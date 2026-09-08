fn main() {
    // Limits precede even argv/path and config parsing: hostile bitcode can
    // request an allocation large enough to abort rather than unwind.
    if let Err(error) = robin_replay_verifier::worker_process::apply_bootstrap_resource_limits(
        robin_replay_verifier::worker_process::BootstrapResourceLimits::default(),
    ) {
        eprintln!("install verifier process limits: {error}");
        std::process::exit(70);
    }
    let paths = match robin_replay_verifier::worker_process::WorkerPaths::parse_flags(
        std::env::args_os().skip(1),
    ) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("invalid verifier invocation: {error}");
            std::process::exit(64);
        }
    };

    if let Err(error) = robin_replay_verifier::worker::run_one_job(&paths) {
        eprintln!("verifier worker failed: {error}");
        std::process::exit(74);
    }
}

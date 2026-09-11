//! Command-line admission for the single public runner entry point.
use super::{
    Deserialize, DumpOptions, NativeStoragePolicy, PathBuf, Serialize, TRACE_NATIVE_BLOCK_RECORDS,
    TRACE_NATIVE_WINDOW_LOG, TraceEntityId, TraceEntityKind,
};

pub(super) struct Options {
    #[cfg(not(feature = "client"))]
    pub(super) core_datadir: PathBuf,
    pub(super) inspect_capabilities: bool,
    pub(super) scan_all: bool,
    pub(super) no_auto_dump: bool,
    pub(super) visual: bool,
    pub(super) trace_path: PathBuf,
    pub(super) dump: Option<DumpOptions>,
    pub(super) http_server: Option<u16>,
    #[cfg(feature = "client")]
    pub(super) start_paused: bool,
    pub(super) frame_zero_screenshot_dir: Option<PathBuf>,
    pub(super) bench_encodings: bool,
    pub(super) convert: bool,
    pub(super) reblock: bool,
    pub(super) reblock_policy: NativeStoragePolicy,
    pub(super) validate_native: bool,
}

#[derive(clap::Parser, Serialize, Deserialize)]
#[command(about = "Replay or inspect an Original parity trace")]
pub(super) struct CliOptions {
    /// CPU replay core data root (default: invocation-relative assets/core-datadir).
    /// Not supported by the client-feature runner, which uses client asset startup.
    #[arg(long, conflicts_with = "mode")]
    pub(super) core_datadir: Option<PathBuf>,
    #[arg(long, group = "mode")]
    pub(super) inspect_capabilities: bool,
    #[arg(long)]
    pub(super) scan_all: bool,
    #[arg(long)]
    pub(super) no_auto_dump: bool,
    #[arg(long)]
    pub(super) visual: bool,
    pub(super) trace_path: PathBuf,
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    pub(super) http_server: Option<u16>,
    #[arg(long, requires = "http_server")]
    pub(super) start_paused: bool,
    #[arg(long)]
    pub(super) frame_zero_screenshot_dir: Option<PathBuf>,
    #[arg(long, requires = "frame_zero_screenshot_dir")]
    pub(super) frame_zero_screenshot_only: bool,
    #[arg(long, group = "mode")]
    pub(super) bench_encodings: bool,
    #[arg(long, group = "mode")]
    pub(super) convert: bool,
    #[arg(long, group = "mode")]
    pub(super) reblock: bool,
    #[arg(long, requires = "reblock")]
    pub(super) reblock_records: Option<usize>,
    #[arg(long, requires = "reblock")]
    pub(super) reblock_window_log: Option<u32>,
    #[arg(long, group = "mode")]
    pub(super) validate_native: bool,
    #[arg(long = "dump-jsonl")]
    pub(super) dump_path: Option<PathBuf>,
    #[arg(long, default_value_t = 0)]
    pub(super) dump_from: u64,
    #[arg(long, default_value_t = u64::MAX)]
    pub(super) dump_through: u64,
    #[arg(long = "dump-entity", value_parser = parse_dump_entity)]
    pub(super) dump_entities: Vec<TraceEntityId>,
}

pub(super) fn parse_options() -> Options {
    let CliOptions {
        core_datadir,
        inspect_capabilities,
        scan_all,
        no_auto_dump,
        visual,
        trace_path,
        http_server,
        start_paused,
        frame_zero_screenshot_dir,
        frame_zero_screenshot_only,
        bench_encodings,
        convert,
        reblock,
        reblock_records,
        reblock_window_log,
        validate_native,
        dump_path,
        dump_from,
        dump_through,
        dump_entities,
    } = <CliOptions as clap::Parser>::parse();
    #[cfg(feature = "client")]
    assert!(
        core_datadir.is_none(),
        "--core-datadir is only supported by the CPU runner; client builds use client asset startup"
    );
    #[cfg(not(feature = "client"))]
    let core_datadir = {
        let path = core_datadir.unwrap_or_else(|| PathBuf::from("assets/core-datadir"));
        std::path::absolute(&path)
            .unwrap_or_else(|error| panic!("resolve --core-datadir {}: {error}", path.display()))
    };
    let reblock_policy_requested = reblock_records.is_some() || reblock_window_log.is_some();
    let reblock_records = reblock_records.unwrap_or(TRACE_NATIVE_BLOCK_RECORDS);
    let reblock_window_log = reblock_window_log.unwrap_or(TRACE_NATIVE_WINDOW_LOG);
    assert!(
        dump_from <= dump_through,
        "--dump-from exceeds --dump-through"
    );
    assert!(
        dump_path.is_some()
            || (dump_from == 0 && dump_through == u64::MAX && dump_entities.is_empty()),
        "--dump-from, --dump-through, and --dump-entity require --dump-jsonl"
    );
    assert!(
        !start_paused || http_server.is_some(),
        "--start-paused requires --http-server"
    );
    assert!(
        !frame_zero_screenshot_only || frame_zero_screenshot_dir.is_some(),
        "--frame-zero-screenshot-only requires --frame-zero-screenshot-dir"
    );
    assert!(
        usize::from(bench_encodings)
            + usize::from(inspect_capabilities)
            + usize::from(convert)
            + usize::from(reblock)
            + usize::from(validate_native)
            <= 1,
        "--inspect-capabilities, --bench-encodings, --convert, --reblock, and --validate-native are mutually exclusive"
    );
    assert!(
        reblock || !reblock_policy_requested,
        "--reblock-records and --reblock-window-log require --reblock"
    );
    let reblock_policy = NativeStoragePolicy::new(reblock_records, reblock_window_log);
    Options {
        #[cfg(not(feature = "client"))]
        core_datadir,
        inspect_capabilities,
        scan_all,
        no_auto_dump,
        visual,
        trace_path,
        http_server,
        #[cfg(feature = "client")]
        start_paused,
        frame_zero_screenshot_dir,
        bench_encodings,
        convert,
        reblock,
        reblock_policy,
        validate_native,
        dump: dump_path.map(|path| DumpOptions {
            path,
            from_frame: dump_from,
            through_frame: dump_through,
            entities: dump_entities,
        }),
    }
}

pub(super) fn parse_dump_entity(value: &str) -> Result<TraceEntityId, String> {
    let (kind, index) = value
        .split_once(':')
        .ok_or_else(|| format!("--dump-entity must be KIND:INDEX, got {value:?}"))?;
    let kind = match kind {
        "pc" => TraceEntityKind::Pc,
        "soldier" => TraceEntityKind::Soldier,
        "civilian" => TraceEntityKind::Civilian,
        "fx" => TraceEntityKind::Fx,
        "target" => TraceEntityKind::Target,
        "bonus" => TraceEntityKind::Bonus,
        "scroll" => TraceEntityKind::Scroll,
        "projectile" => TraceEntityKind::Projectile,
        "net" => TraceEntityKind::Net,
        _ => return Err(format!("unknown --dump-entity kind {kind:?}")),
    };
    Ok(TraceEntityId {
        kind,
        index: index
            .parse()
            .map_err(|_| format!("invalid --dump-entity index in {value:?}"))?,
    })
}

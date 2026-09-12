//! Trace codec and lossless-conversion boundary.
//!
//! `trace_model` and `native_model` own the frozen current wire layout.
//! Older native generations require offline migration before admission.
use super::*;

pub(super) fn trace_content_sha256(trace_path: &Path) -> TraceStorageResult<String> {
    let mut source = File::open(trace_path).map_err(|error| {
        format!(
            "open parity trace for fingerprint {}: {error}",
            trace_path.display()
        )
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer).map_err(|error| {
            format!(
                "read parity trace for fingerprint {}: {error}",
                trace_path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

pub(super) fn trace_source_fingerprint(trace_path: &Path) -> TraceStorageResult<String> {
    let metadata = std::fs::metadata(trace_path)
        .map_err(|error| format!("stat parity trace {}: {error}", trace_path.display()))?;
    let modified = metadata
        .modified()
        .storage_context("parity trace modification time is unavailable")?
        .duration_since(std::time::UNIX_EPOCH)
        .storage_context("parity trace modification time predates Unix epoch")?
        .as_nanos();
    let content_sha256 = trace_content_sha256(trace_path)?;
    Ok(format!(
        "native-parity-v{TRACE_NATIVE_VERSION}:length={}:modified={modified}:sha256={content_sha256}",
        metadata.len()
    ))
}

pub(super) fn absolute_trace_path(trace_path: &Path) -> TraceStorageResult<PathBuf> {
    Ok(absolute_trace_path_from(
        trace_path,
        &std::env::current_dir()
            .storage_context("read current directory for relative parity trace")?,
    ))
}

pub(super) fn absolute_trace_path_from(trace_path: &Path, current_dir: &Path) -> PathBuf {
    if trace_path.is_absolute() {
        return trace_path.to_owned();
    }
    current_dir.join(trace_path)
}

/// Canonicalize the logical trace path even when only its native artifact
/// still exists on disk (a converted recording is deleted, but its
/// `.jsonl.zst` path remains the trace's identity).
pub(super) fn canonicalize_trace_identity(trace_path: &Path) -> TraceStorageResult<PathBuf> {
    if trace_path.exists() {
        return Ok(trace_path
            .canonicalize()
            .map_err(|error| format!("canonicalize {}: {error}", trace_path.display()))?);
    }
    let file_name = trace_path.file_name().ok_or_else(|| {
        format!(
            "parity trace path {} has no file name",
            trace_path.display()
        )
    })?;
    let parent = match trace_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    Ok(parent
        .canonicalize()
        .map_err(|error| {
            format!(
                "canonicalize parity trace directory {}: {error}",
                parent.display()
            )
        })?
        .join(file_name))
}

pub(super) fn native_binary_trace_path(trace_path: &std::path::Path) -> PathBuf {
    let mut native_name = trace_path.as_os_str().to_owned();
    // A plain `.jsonl` capture converts to the same artifact name its
    // compressed spelling would have produced: the `.jsonl.zst` path is the
    // stable trace identity in ledgers, sweep status keys, and completion
    // markers, so skipping the interim zstd recording must not change it.
    if trace_path.as_os_str().to_string_lossy().ends_with(".jsonl") {
        native_name.push(".zst");
    }
    native_name.push(TRACE_NATIVE_SUFFIX);
    PathBuf::from(native_name)
}

pub(super) fn open_jsonl_trace(
    trace_path: &std::path::Path,
) -> TraceStorageResult<Box<dyn BufRead>> {
    const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

    let mut file = File::open(trace_path)
        .map_err(|error| format!("open parity trace {}: {error}", trace_path.display()))?;
    let mut magic = [0_u8; ZSTD_MAGIC.len()];
    let magic_len = file
        .read(&mut magic)
        .map_err(|error| format!("read parity trace magic {}: {error}", trace_path.display()))?;
    file.rewind().map_err(|error| {
        format!(
            "rewind parity trace after reading magic {}: {error}",
            trace_path.display()
        )
    })?;

    Ok(if magic_len == ZSTD_MAGIC.len() && magic == ZSTD_MAGIC {
        let mut decoder = zstd::stream::read::Decoder::new(file).map_err(|error| {
            format!(
                "start parity trace decompression {}: {error}",
                trace_path.display()
            )
        })?;
        decoder
            .window_log_max(TRACE_ZSTD_WINDOW_LOG_MAX)
            .map_err(|error| {
                format!(
                    "configure parity trace decompression {}: {error}",
                    trace_path.display()
                )
            })?;
        Box::new(BufReader::new(decoder))
    } else {
        Box::new(BufReader::new(file))
    })
}

/// Normalize a trace JSON tree for the cache round-trip audit. Two declared,
/// information-preserving differences between the raw JSONL and the typed
/// representation are erased on BOTH sides so everything else must match
/// exactly:
///
/// * `{"bits": N, "value": F}` float objects lose the redundant decimal
///   rendering `value`; `bits` alone is authoritative.
/// * `null` object entries are removed: serde cannot distinguish an absent
///   optional field from an explicit `null` once re-serialized (both are
///   `None`), and the two fields where the original game's missing value is meaningful keep
///   the distinction in their typed `Option<Option<_>>` form. Array elements
///   are never removed.
/// * Empty array/object entries are removed after their children normalize:
///   additive legacy collections parse from an absent key but re-serialize as
///   empty, so the typed compatibility schema deliberately identifies the two.
pub(super) fn normalize_trace_json_for_roundtrip(value: &mut serde_json::Value) {
    normalize_trace_json_for_roundtrip_inner(value);
}

pub(super) fn normalize_trace_json_for_roundtrip_inner(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if map.len() == 2
                && map.get("bits").is_some_and(serde_json::Value::is_u64)
                && map.get("value").is_some_and(|value| {
                    // The parity format's float state renders non-finite floats as
                    // strings; `bits` alone carries the information.
                    value.is_number()
                        || matches!(value.as_str(), Some("nan" | "infinity" | "-infinity"))
                })
            {
                map.remove("value");
            }
            for child in map.values_mut() {
                normalize_trace_json_for_roundtrip_inner(child);
            }
            map.retain(|key, child| match child {
                serde_json::Value::Null => false,
                serde_json::Value::Array(items) => !items.is_empty(),
                serde_json::Value::Object(entries) => !entries.is_empty(),
                // Early schema-16 elements omitted this additive observation.
                // Its compatibility default is false and is excluded from
                // logical comparison for that recorder generation.
                serde_json::Value::Bool(false)
                    if matches!(
                        key.as_str(),
                        "increment_map_valid"
                            | "passing_door_directly"
                            | "script_locked"
                            | "locked"
                            | "was_busy"
                            | "very_busy"
                            | "macro_timer_running"
                            | "macro_in_progress"
                    ) =>
                {
                    false
                }
                serde_json::Value::Number(number)
                    if number.as_u64() == Some(0)
                        && matches!(
                            key.as_str(),
                            "locks" | "macro_timer_ring" | "macro_remaining"
                        ) =>
                {
                    false
                }
                _ => true,
            });
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_trace_json_for_roundtrip_inner(item);
            }
        }
        _ => {}
    }
}

/// First path where the two normalized JSON trees disagree, or `None` when
/// they match. Paths make cache-build failures actionable on multi-hundred-KB
/// trace lines.
pub(super) fn first_json_difference(
    path: &str,
    original: &serde_json::Value,
    reserialized: &serde_json::Value,
) -> Option<String> {
    use serde_json::Value;
    match (original, reserialized) {
        (Value::Object(original), Value::Object(reserialized)) => {
            for (key, original_child) in original {
                let Some(reserialized_child) = reserialized.get(key) else {
                    return Some(format!(
                        "{path}.{key} is dropped by the typed representation (recorded {original_child})"
                    ));
                };
                if let Some(difference) = first_json_difference(
                    &format!("{path}.{key}"),
                    original_child,
                    reserialized_child,
                ) {
                    return Some(difference);
                }
            }
            reserialized
                .keys()
                .find(|key| !original.contains_key(*key))
                .map(|key| format!("{path}.{key} is invented by the typed representation"))
        }
        (Value::Array(original), Value::Array(reserialized)) => {
            if original.len() != reserialized.len() {
                return Some(format!(
                    "{path} has {} recorded elements but {} typed elements",
                    original.len(),
                    reserialized.len()
                ));
            }
            original.iter().zip(reserialized).enumerate().find_map(
                |(index, (original_child, reserialized_child))| {
                    first_json_difference(
                        &format!("{path}[{index}]"),
                        original_child,
                        reserialized_child,
                    )
                },
            )
        }
        _ if original == reserialized => None,
        _ => Some(format!("{path}: recorded {original} became {reserialized}")),
    }
}

/// Reject unless the typed record re-serializes to the JSON it was parsed
/// from, modulo [`normalize_trace_json_for_roundtrip`]. Running this on every
/// line during cache conversion is what lets the binary cache stand in for
/// the recording: a field the typed schema silently drops or reshapes fails
/// the build instead of becoming data loss.
///
/// Building JSON trees for both sides of every frame is expensive, so
/// [`ensure_native_binary_trace`] audits frame lines on a worker pool
/// ([`spawn_roundtrip_audit_workers`]) while the writer thread streams
/// records into the cache; a failed audit aborts conversion before the
/// temporary cache file is published.
pub(super) fn verify_trace_line_roundtrip<T: Serialize>(
    record: &T,
    line: &str,
    line_number: usize,
) -> TraceStorageResult<()> {
    let mut original: serde_json::Value = serde_json::from_str(line).map_err(|error| {
        format!("reparse trace line {line_number} for the round-trip audit: {error}")
    })?;
    let mut reserialized = serde_json::to_value(record).map_err(|error| {
        format!("reserialize trace line {line_number} for the round-trip audit: {error}")
    })?;
    normalize_trace_json_for_roundtrip(&mut original);
    normalize_trace_json_for_roundtrip(&mut reserialized);
    Ok(
        if let Some(difference) = first_json_difference("$", &original, &reserialized) {
            return Err(format!(
                "trace line {line_number} does not survive the typed cache round trip: {difference}"
            ));
        },
    )
}

/// Re-parse and round-trip-audit one trace line (any line after the header
/// and RNG prefix: frames and the rng_suffix terminator).
pub(super) fn audit_trace_line(line: &str, line_number: usize) -> TraceStorageResult<()> {
    Ok(if let Some(frame) = parse_trace_frame(line, line_number)? {
        verify_trace_line_roundtrip(&frame, line, line_number)?;
    } else {
        let suffix: TraceRngOnly = serde_json::from_str(line).map_err(|error| {
            format!(
                "reparse RNG suffix on trace line {line_number} for the round-trip audit: {error}"
            )
        })?;
        verify_trace_line_roundtrip(&suffix, line, line_number)?;
    })
}

/// Fan trace lines out to audit workers. Drop the sender and join every worker
/// before publishing the converted trace, including when the writer fails.
pub(super) fn spawn_roundtrip_audit_workers<'scope, 'env>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
) -> (
    std::sync::mpsc::SyncSender<(usize, String)>,
    Vec<std::thread::ScopedJoinHandle<'scope, TraceStorageResult<()>>>,
) {
    // Bounded so a fast reader cannot buffer a whole multi-GB trace.
    let (sender, receiver) = std::sync::mpsc::sync_channel::<(usize, String)>(64);
    let receiver = std::sync::Arc::new(std::sync::Mutex::new(receiver));
    let workers = std::thread::available_parallelism()
        .map(|threads| threads.get().saturating_sub(1).clamp(1, 8))
        .unwrap_or(1);
    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let receiver = std::sync::Arc::clone(&receiver);
        handles.push(scope.spawn(move || {
            loop {
                let received = receiver
                    .lock()
                    .storage_context("lock round-trip audit channel")?
                    .recv();
                match received {
                    Ok((line_number, line)) => audit_trace_line(&line, line_number)?,
                    Err(_) => return Ok(()),
                }
            }
        }));
    }
    (sender, handles)
}

pub(super) fn join_roundtrip_audit_workers(
    workers: Vec<std::thread::ScopedJoinHandle<'_, TraceStorageResult<()>>>,
) -> TraceStorageResult<()> {
    let mut result = Ok(());
    for worker in workers {
        let outcome = worker
            .join()
            .map_err(|_| "parity round-trip audit worker panicked".to_owned())
            .and_then(|result| result);
        if result.is_ok() {
            result = outcome;
        }
    }
    result
}

/// Validate a standalone native trace, including the legacy version whose
/// JSONL source may intentionally have been deleted.
pub(super) fn validate_standalone_native_trace(native_path: &Path) -> TraceStorageResult<()> {
    let footer = read_binary_trace_footer(native_path).map_err(|error| format!(
            "native parity trace {} has a corrupt or missing fixed footer: {error};              its JSONL source is gone, so restore or migrate the native file",
            native_path.display()
        ))?;
    let header = read_binary_trace_header(native_path)?;
    validate_binary_trace_footer(&footer).map_err(|error| {
        format!(
            "native parity trace {} has an unsupported fixed footer: {error}",
            native_path.display()
        )
    })?;
    storage_ensure!(
        (footer.version) == (header.version),
        "native parity trace {} has header version {} but footer version {}",
        native_path.display(),
        header.version,
        footer.version
    );

    // A conversion may have crashed after unlinking the JSONL source.  In
    // that state the native file is authoritative, so accepting it based on
    // only its header/footer would conceal a corrupt compressed block.  Read
    // the complete record stream before reporting an already-converted
    // source as successful.
    let mut reader = BinaryTraceReader::open(native_path)?;
    let decoded_header = reader.read_header()?;
    let mut timeline = TraceTimeline::new(decoded_header.trace.initial_frame);
    Ok(loop {
        match reader.read_record()? {
            BinaryTraceRecord::Frame(frame) => timeline
                .observe(frame.frame_before, frame.frame_after)
                .map_err(|error| {
                    format!(
                        "standalone native parity trace {} breaks the frame timeline: {error}",
                        native_path.display()
                    )
                })?,
            BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => {
                storage_ensure!(
                    rng_suffix.is_some(),
                    "standalone native parity trace {} lost its RNG suffix",
                    native_path.display()
                );
                let final_frame = final_frame.ok_or_else(|| {
                    format!(
                        "standalone native parity trace {} lost its final frame",
                        native_path.display()
                    )
                })?;
                let frame_count = frame_count.ok_or_else(|| {
                    format!(
                        "standalone native parity trace {} lost its frame count",
                        native_path.display()
                    )
                })?;
                timeline
                    .validate_terminator(frame_count, final_frame).map_err(|error| format!(
                            "standalone native parity trace {} terminator disagrees with its frames: {error}",
                            native_path.display()
                        ))?;
                reader
                    .validate_terminator(frame_count, final_frame).map_err(|error| format!(
                            "standalone native parity trace {} disagrees with its fixed footer: {error}",
                            native_path.display()
                        ))?;
                break;
            }
        }
    })
}

/// `--convert`: turn a JSONL recording into its native parity trace and
/// delete the recording once losslessness is assured. Safety gates, in
/// order:
///
/// 1. Conversion itself round-trip audits every line (see
///    [`verify_trace_line_roundtrip`]) — an unfaithful typed representation
///    aborts before the native file is published.
/// 2. The published native file is independently re-read from disk: every
///    block must decode, the frame timeline must be contiguous, and the
///    terminator must agree with the fixed footer.
/// 3. The decoded frame count must match the recording's own line count.
///
/// Only then is the JSONL deleted, along with obsolete `.parity-cache-v*`
/// derivations of it. Completion markers (`*.complete`) are left in place —
/// they carry the trace's capture provenance and its identity persists.
pub(super) fn convert_recording_to_native(trace_path: &Path) -> TraceStorageResult<()> {
    // `Path::parent()` is `Some("")` for a bare relative file name. Resolve
    // the input before publishing so cleanup always has a real directory and
    // `--convert replay.jsonl.zst` behaves like `--convert ./replay.jsonl.zst`.
    let trace_path = absolute_trace_path(trace_path)?;
    let display = trace_path.display();
    storage_ensure!(
        !trace_path
            .as_os_str()
            .to_string_lossy()
            .ends_with(TRACE_NATIVE_SUFFIX),
        "{display} already is a native parity trace"
    );
    let native_path = native_binary_trace_path(&trace_path);
    let quarantine_path = conversion_quarantine_path(&trace_path);
    // State classification and recovery are protected by the same stable
    // lock inode as generation and deletion.
    let _generation_lock = lock_native_trace_generation(&native_path)?;
    reject_conversion_symlink(&trace_path)?;
    reject_conversion_symlink(&quarantine_path)?;
    reject_conversion_symlink(&native_path)?;

    match (trace_path.exists(), quarantine_path.exists()) {
        (true, true) => {
            return Err(format!(
                "conversion conflict: producer recreated {display} while pending quarantine {} exists; preserve both",
                quarantine_path.display()
            ));
        }
        (false, true) => {
            storage_ensure!(
                native_path.is_file(),
                "pending conversion quarantine {} has no native counterpart {}",
                quarantine_path.display(),
                native_path.display()
            );
            let fingerprint = trace_source_fingerprint(&quarantine_path)?;
            let verified =
                verify_converted_native_trace(&quarantine_path, &native_path, fingerprint)?;
            let removed_derived =
                finish_verified_conversion(&trace_path, &quarantine_path, &verified)?;
            eprintln!(
                "recovered conversion of {display} into {} ({} frames); deleted the pending source and {removed_derived} obsolete derived files",
                native_path.display(),
                verified.decoded_frames
            );
            return Ok(());
        }
        (false, false) => {
            storage_ensure!(
                native_path.is_file(),
                "recording {display} does not exist and has no native counterpart {}",
                native_path.display()
            );
            validate_standalone_native_trace(&native_path)?;
            eprintln!(
                "recording {display} was already converted into {}",
                native_path.display()
            );
            return Ok(());
        }
        (true, false) => {}
    }
    storage_ensure!(trace_path.is_file(), "recording {display} is not a file");
    let source_bytes = std::fs::metadata(&trace_path)
        .storage_context("stat recording before conversion")?
        .len();
    let source_fingerprint = trace_source_fingerprint(&trace_path)?;
    let native_path =
        ensure_native_binary_trace_locked(&trace_path, &native_path, source_fingerprint.clone())?;

    // Independent re-read of the published artifact. Keep the proof token in
    // the type flow so the destructive cleanup below cannot move ahead of the
    // complete native readback accidentally.
    let verified = verify_converted_native_trace(&trace_path, &native_path, source_fingerprint)?;
    move_verified_recording_to_quarantine(&trace_path, &quarantine_path, &verified)
        .map_err(|error| format!("refuse to quarantine converted recording: {error}"))?;
    let removed_derived = finish_verified_conversion(
        &trace_path,
        &quarantine_path,
        &VerifiedNativeReadback {
            source_path: quarantine_path.clone(),
            ..verified.clone()
        },
    )?;

    let native_bytes = std::fs::metadata(&native_path)
        .storage_context("stat native parity trace after conversion")?
        .len();
    eprintln!(
        "converted {display} ({:.2} MiB) into {} ({:.2} MiB, {} frames);          deleted the recording and {removed_derived} obsolete derived files",
        source_bytes as f64 / (1024.0 * 1024.0),
        native_path.display(),
        native_bytes as f64 / (1024.0 * 1024.0),
        verified.decoded_frames,
    );

    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct VerifiedNativeReadback {
    pub(super) decoded_frames: u64,
    pub(super) source_path: PathBuf,
    pub(super) source_fingerprint: String,
}

pub(super) fn verify_converted_native_trace(
    trace_path: &Path,
    native_path: &Path,
    source_fingerprint: String,
) -> TraceStorageResult<VerifiedNativeReadback> {
    let mut source_lines = open_jsonl_trace(trace_path)?.lines();
    let source_header_line = source_lines
        .next()
        .storage_context("recording lost its header during native readback")?
        .storage_context("read recording header during native readback")?;
    let source_trace: TraceHeader = serde_json::from_str(&source_header_line)
        .storage_context("reparse recording header during native readback")?;
    let source_prefix_line = source_lines
        .next()
        .storage_context("recording lost its RNG prefix during native readback")?
        .storage_context("read recording RNG prefix during native readback")?;
    let source_prefix: TraceRngPrefix = serde_json::from_str(&source_prefix_line)
        .storage_context("reparse recording RNG prefix during native readback")?;
    let expected_header = BinaryTraceHeaderV68 {
        version: TRACE_NATIVE_VERSION,
        source_fingerprint: source_fingerprint.clone(),
        trace: source_trace,
        rng_prefix: source_prefix,
    };

    let mut reader = BinaryTraceReader::open(native_path)?;
    let header = reader.read_header()?;
    storage_ensure!(
        (header.version) == (TRACE_NATIVE_VERSION),
        "native parity trace {} decodes with the wrong version",
        native_path.display()
    );
    storage_ensure!(
        (bitcode::encode(&header)) == (bitcode::encode(&expected_header)),
        "native parity trace {} header differs semantically from its recording",
        native_path.display()
    );
    let mut timeline = TraceTimeline::new(header.trace.initial_frame);
    let mut decoded_frames = 0_u64;
    loop {
        match reader.read_record()? {
            BinaryTraceRecord::Frame(frame) => {
                let line_number = decoded_frames + 3;
                let source_line = source_lines
                    .next()
                    .ok_or_else(|| {
                        format!("recording ended before native frame on line {line_number}")
                    })?
                    .map_err(|error| {
                        format!("read recording frame on line {line_number}: {error}")
                    })?;
                let source_frame = parse_trace_frame(&source_line, line_number as usize)?
                    .ok_or_else(|| {
                        format!(
                            "recording has its terminator before native frame on line {line_number}"
                        )
                    })?;
                storage_ensure!(
                    (bitcode::encode(&frame)) == (bitcode::encode(&source_frame)),
                    "native parity frame on line {line_number} differs semantically from its recording"
                );
                timeline
                    .observe(frame.frame_before, frame.frame_after)
                    .map_err(|error| {
                        format!(
                            "native parity trace {} breaks the frame timeline: {error}",
                            native_path.display()
                        )
                    })?;
                decoded_frames += 1;
            }
            BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => {
                let line_number = decoded_frames + 3;
                let source_line = source_lines
                    .next()
                    .ok_or_else(|| {
                        format!("recording ended before native terminator on line {line_number}")
                    })?
                    .map_err(|error| {
                        format!("read recording terminator on line {line_number}: {error}")
                    })?;
                let source_end: TraceRngOnly =
                    serde_json::from_str(&source_line).map_err(|error| {
                        format!("parse recording terminator on line {line_number}: {error}")
                    })?;
                let final_frame =
                    final_frame.storage_context("native End record lost its final frame")?;
                let frame_count =
                    frame_count.storage_context("native End record lost its frame count")?;
                let rng_suffix =
                    rng_suffix.storage_context("native End record lost its RNG suffix")?;
                storage_ensure!(
                    (bitcode::encode(&rng_suffix)) == (bitcode::encode(&source_end.draws)),
                    "native RNG suffix differs semantically from its recording"
                );
                storage_ensure!(
                    (final_frame) == (source_end.final_frame),
                    "native trace invariant failed: assert_eq"
                );
                storage_ensure!(
                    (frame_count) == (source_end.frame_count),
                    "native trace invariant failed: assert_eq"
                );
                storage_ensure!(
                    (frame_count) == (decoded_frames),
                    "native trace invariant failed: assert_eq"
                );
                timeline
                    .validate_terminator(frame_count, final_frame)
                    .map_err(|error| {
                        format!(
                            "native parity trace {} terminator disagrees with its frames: {error}",
                            native_path.display()
                        )
                    })?;
                reader
                    .validate_terminator(frame_count, final_frame)
                    .map_err(|error| {
                        format!(
                            "native parity trace {} disagrees with its fixed footer: {error}",
                            native_path.display()
                        )
                    })?;
                break;
            }
        }
    }
    storage_ensure!(
        source_lines.next().is_none(),
        "recording {} has data after its native terminator",
        trace_path.display()
    );

    storage_ensure!(
        (header.source_fingerprint) == (source_fingerprint),
        "native parity trace {} was not built from the source fingerprint held by this conversion",
        native_path.display()
    );

    Ok(VerifiedNativeReadback {
        decoded_frames,
        source_path: trace_path.to_owned(),
        source_fingerprint,
    })
}

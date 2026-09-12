//! Native trace storage, conversion transactions, reblocking and recovery.
use super::*;

/// Storage and trace-format failures retain their operation/path context until
/// the command boundary. Conversion callers must not publish on an error.
pub(super) type TraceStorageResult<T> = Result<T, String>;

pub(super) trait StorageContext<T> {
    fn storage_context(self, context: &str) -> TraceStorageResult<T>;
}

impl<T, E: std::fmt::Display> StorageContext<T> for Result<T, E> {
    fn storage_context(self, context: &str) -> TraceStorageResult<T> {
        self.map_err(|error| format!("{context}: {error}"))
    }
}

impl<T> StorageContext<T> for Option<T> {
    fn storage_context(self, context: &str) -> TraceStorageResult<T> {
        self.ok_or_else(|| context.to_owned())
    }
}

macro_rules! storage_ensure {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            return Err(format!($($message)+));
        }
    };
}
pub(super) use storage_ensure;

pub(super) fn conversion_quarantine_path(trace_path: &Path) -> PathBuf {
    let mut path = trace_path.as_os_str().to_owned();
    path.push(TRACE_CONVERSION_QUARANTINE_SUFFIX);
    PathBuf::from(path)
}

pub(super) fn reject_conversion_symlink(path: &Path) -> TraceStorageResult<()> {
    Ok(match std::fs::symlink_metadata(path) {
        Ok(_) => storage_ensure!(
            !conversion_path_is_symlink(path),
            "conversion paths must not be symbolic links: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "inspect conversion path {}: {error}",
                path.display()
            ));
        }
    })
}

pub(super) fn conversion_path_is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

pub(super) fn move_verified_recording_to_quarantine(
    trace_path: &Path,
    quarantine_path: &Path,
    verified: &VerifiedNativeReadback,
) -> Result<(), String> {
    if trace_path != verified.source_path {
        return Err(format!(
            "verified source path {} changed to {} before quarantine",
            verified.source_path.display(),
            trace_path.display()
        ));
    }
    let parent = trace_path
        .parent()
        .storage_context("absolute converted recording always has a parent directory")?;
    if quarantine_path.exists() {
        return Err(format!(
            "deterministic conversion quarantine {} already exists",
            quarantine_path.display()
        ));
    }
    std::fs::rename(trace_path, quarantine_path).map_err(|error| {
        format!(
            "atomically quarantine converted recording {} as {}: {error}",
            trace_path.display(),
            quarantine_path.display()
        )
    })?;
    sync_directory(parent, "converted recording quarantine")?;

    let quarantined_fingerprint = trace_source_fingerprint(quarantine_path)?;
    if quarantined_fingerprint == verified.source_fingerprint {
        return Ok(());
    }

    let restoration = match restore_quarantined_recording_no_replace(quarantine_path, trace_path) {
        Ok(()) => {
            std::fs::remove_file(quarantine_path)
                .map_err(|error| format!("remove restored recording quarantine link: {error}"))?;
            sync_directory(parent, "mismatched recording restoration")?;
            "restored it without overwriting another producer".to_owned()
        }
        Err(error) => {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "canonical pathname was recreated; preserved mismatched content at {}",
                    quarantine_path.display()
                )
            } else {
                return Err(format!(
                    "recording changed and atomic no-replace restoration {} -> {} failed: {error}; preserved mismatched content at {}",
                    quarantine_path.display(),
                    trace_path.display(),
                    quarantine_path.display()
                ));
            }
        }
    };
    Err(format!(
        "recording {} changed after native readback (expected {:?}, found {:?}); {restoration}",
        trace_path.display(),
        verified.source_fingerprint,
        quarantined_fingerprint
    ))
}

/// Atomically restore a quarantined source without replacing a producer that
/// recreated the canonical pathname. Both paths share a filesystem, so a
/// hard link provides the required no-replace operation.
pub(super) fn restore_quarantined_recording_no_replace(
    quarantine_path: &Path,
    trace_path: &Path,
) -> std::io::Result<()> {
    std::fs::hard_link(quarantine_path, trace_path)
}

pub(super) fn finish_verified_conversion(
    trace_path: &Path,
    quarantine_path: &Path,
    verified: &VerifiedNativeReadback,
) -> TraceStorageResult<usize> {
    storage_ensure!(
        (verified.source_path) == (quarantine_path),
        "native trace invariant failed: assert_eq"
    );
    storage_ensure!(
        (trace_source_fingerprint(quarantine_path)?) == (verified.source_fingerprint),
        "pending conversion source changed after native readback"
    );
    storage_ensure!(
        !trace_path.exists(),
        "conversion conflict: producer recreated {} while its verified quarantine still exists",
        trace_path.display()
    );
    let parent = trace_path
        .parent()
        .storage_context("absolute converted recording always has a parent directory")?;
    commit_verified_conversion_files(trace_path, quarantine_path, || {})
        .map_err(|error| format!("conversion committed with a producer conflict: {error}"))?;
    let mut removed_derived = 0_usize;
    if let Some(name) = trace_path.file_name() {
        let obsolete_prefix = format!("{}.parity-cache-v", name.to_string_lossy());
        let entries = match std::fs::read_dir(parent) {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("warning: could not list obsolete conversion derivations: {error}");
                return Ok(0);
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    eprintln!("warning: could not read obsolete derivation entry: {error}");
                    continue;
                }
            };
            if is_obsolete_native_derivation(&entry.file_name().to_string_lossy(), &obsolete_prefix)
            {
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => removed_derived += 1,
                    Err(error) => eprintln!(
                        "warning: could not delete obsolete derivation {}: {error}",
                        entry.path().display()
                    ),
                }
            }
        }
        if removed_derived > 0
            && let Err(error) = File::open(parent).and_then(|directory| directory.sync_all())
        {
            eprintln!(
                "warning: could not sync {} after obsolete converted recording cleanup: {error}",
                parent.display()
            );
        }
    }
    Ok(removed_derived)
}

pub(super) fn commit_verified_conversion_files(
    trace_path: &Path,
    quarantine_path: &Path,
    before_quarantine_unlink: impl FnOnce(),
) -> Result<(), String> {
    let parent = trace_path
        .parent()
        .storage_context("absolute converted recording always has a parent directory")?;
    if trace_path.exists() {
        return Err(format!(
            "producer recreated canonical recording {} before quarantine commit",
            trace_path.display()
        ));
    }
    before_quarantine_unlink();
    std::fs::remove_file(quarantine_path).map_err(|error| {
        format!(
            "delete quarantined converted recording {}: {error}",
            quarantine_path.display()
        )
    })?;
    sync_directory(parent, "converted recording transaction commit")?;
    if trace_path.exists() {
        return Err(format!(
            "producer recreated canonical recording {} during quarantine commit; new recording was preserved",
            trace_path.display()
        ));
    }
    Ok(())
}

pub(super) fn is_obsolete_native_derivation(file_name: &str, prefix: &str) -> bool {
    let Some(suffix) = file_name.strip_prefix(prefix) else {
        return false;
    };
    let digit_count = suffix
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    digit_count > 0
        && (digit_count == suffix.len() || suffix.as_bytes().get(digit_count) == Some(&b'.'))
}

pub(super) fn native_trace_generation_lock_path(native_path: &Path) -> PathBuf {
    let mut lock_name = native_path.as_os_str().to_owned();
    lock_name.push(".lock");
    PathBuf::from(lock_name)
}

pub(super) fn lock_native_trace_generation(native_path: &Path) -> TraceStorageResult<File> {
    let lock_path = native_trace_generation_lock_path(native_path);
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| {
            format!(
                "open native parity trace lock {}: {error}",
                lock_path.display()
            )
        })?;
    lock_file.lock_exclusive().map_err(|error| {
        format!(
            "lock native parity trace generation {}: {error}",
            lock_path.display()
        )
    })?;
    Ok(lock_file)
}

pub(super) fn sync_directory(directory: &Path, operation: &str) -> TraceStorageResult<()> {
    File::open(directory)
        .map_err(|error| {
            format!(
                "open directory {} after {operation}: {error}",
                directory.display()
            )
        })?
        .sync_all()
        .map_err(|error| {
            format!(
                "sync directory {} after {operation}: {error}",
                directory.display()
            )
        })?;

    Ok(())
}

pub(super) fn native_reblock_source_path(native_path: &Path) -> PathBuf {
    let mut path = native_path.as_os_str().to_owned();
    path.push(TRACE_REBLOCK_SOURCE_SUFFIX);
    PathBuf::from(path)
}

pub(super) fn native_reblock_binding_path(native_path: &Path) -> PathBuf {
    let mut path = native_path.as_os_str().to_owned();
    path.push(TRACE_REBLOCK_BINDING_SUFFIX);
    PathBuf::from(path)
}

pub(super) fn native_reblock_temporary_prefix(
    native_path: &Path,
    binding: bool,
) -> TraceStorageResult<String> {
    let canonical_path = native_reblock_canonical_path(native_path)?;
    #[cfg(unix)]
    let path_bytes = canonical_path.as_os_str().as_bytes();
    #[cfg(not(unix))]
    let path_text = canonical_path.to_string_lossy();
    #[cfg(not(unix))]
    let path_bytes = path_text.as_bytes();
    let path_digest = sha256_hex(path_bytes);
    let kind = if binding { "binding-v67" } else { "v67" };
    Ok(format!(".parity-reblock-{kind}-{path_digest}-"))
}

pub(super) fn cleanup_native_reblock_orphans(native_path: &Path) -> TraceStorageResult<()> {
    let parent = native_path
        .parent()
        .storage_context("absolute native trace always has a parent directory")?;
    let prefixes = [
        native_reblock_temporary_prefix(native_path, false)?,
        native_reblock_temporary_prefix(native_path, true)?,
    ];
    let mut removed = 0_usize;
    for entry in std::fs::read_dir(parent).map_err(|error| {
        format!(
            "list native reblock temporary directory {}: {error}",
            parent.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "read native reblock temporary entry in {}: {error}",
                parent.display()
            )
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !prefixes.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
            format!(
                "inspect native reblock temporary {}: {error}",
                entry.path().display()
            )
        })?;
        if !metadata.file_type().is_file() {
            eprintln!(
                "warning: preserving non-regular native reblock temporary {}",
                entry.path().display()
            );
            continue;
        }
        std::fs::remove_file(entry.path()).map_err(|error| {
            format!(
                "remove orphaned native reblock temporary {}: {error}",
                entry.path().display()
            )
        })?;
        removed += 1;
    }
    Ok(if removed > 0 {
        sync_directory(parent, "native reblock orphan cleanup")?;
        eprintln!(
            "removed {removed} orphaned temporary file(s) for {}",
            native_path.display()
        );
    })
}

pub(super) fn native_reblock_canonical_path(native_path: &Path) -> TraceStorageResult<PathBuf> {
    let parent = native_path
        .parent()
        .storage_context("absolute native trace always has a parent directory")?
        .canonicalize()
        .map_err(|error| {
            format!(
                "canonicalize native trace parent {}: {error}",
                native_path.parent().unwrap().display()
            )
        })?;
    Ok(parent.join(
        native_path
            .file_name()
            .storage_context("native trace path always has a file name")?,
    ))
}

pub(super) fn native_reblock_file_identity(
    path: &Path,
) -> TraceStorageResult<(u64, String, u64, u64)> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("stat native reblock file {}: {error}", path.display()))?;
    #[cfg(unix)]
    let (device, inode) = (metadata.dev(), metadata.ino());
    #[cfg(not(unix))]
    let (device, inode) = (0, 0);
    Ok((metadata.len(), trace_content_sha256(path)?, device, inode))
}

pub(super) fn native_reblock_semantic_identity(
    path: &Path,
) -> TraceStorageResult<(u64, u64, String)> {
    Ok(native_reblock_semantic_identity_with_version_policy(
        path, true,
    )?)
}

pub(super) fn native_reblock_semantic_identity_with_version_policy(
    path: &Path,
    normalize_container_version: bool,
) -> TraceStorageResult<(u64, u64, String)> {
    let footer = read_binary_trace_footer(path)
        .map_err(|error| format!("read native reblock footer {}: {error}", path.display()))?;
    validate_binary_trace_footer(&footer)
        .map_err(|error| format!("validate native reblock footer {}: {error}", path.display()))?;
    let (frame_count, digest) =
        digest_and_validate_native_trace_with_version_policy(path, normalize_container_version)?;
    storage_ensure!(
        (frame_count) == (footer.frame_count),
        "native trace invariant failed: assert_eq"
    );
    Ok((frame_count, footer.final_frame, sha256_hex(&digest)))
}

pub(super) fn create_native_reblock_binding(
    native_path: &Path,
) -> TraceStorageResult<NativeReblockBinding> {
    let (source_bytes, source_content_sha256, source_device, source_inode) =
        native_reblock_file_identity(native_path)?;
    let (frame_count, final_frame, source_semantic_sha256) =
        native_reblock_semantic_identity(native_path)?;
    Ok(NativeReblockBinding {
        version: TRACE_NATIVE_VERSION,
        canonical_path: native_reblock_canonical_path(native_path)?,
        source_content_sha256,
        source_bytes,
        source_semantic_sha256,
        frame_count,
        final_frame,
        #[cfg(unix)]
        source_device,
        #[cfg(unix)]
        source_inode,
    })
}

pub(super) fn write_native_reblock_binding(
    path: &Path,
    binding: &NativeReblockBinding,
) -> TraceStorageResult<()> {
    let parent = path
        .parent()
        .storage_context("absolute native reblock binding always has a parent")?;
    let mut temporary = tempfile::Builder::new()
        .prefix(&native_reblock_temporary_prefix(
            Path::new(&binding.canonical_path),
            true,
        )?)
        .tempfile_in(parent)
        .map_err(|error| {
            format!(
                "create temporary native reblock binding beside {}: {error}",
                path.display()
            )
        })?;
    serde_json::to_writer(&mut temporary, binding)
        .map_err(|error| format!("write native reblock binding: {error}"))?;
    temporary
        .write_all(b"\n")
        .map_err(|error| format!("terminate native reblock binding: {error}"))?;
    temporary
        .flush()
        .map_err(|error| format!("flush native reblock binding: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("sync native reblock binding: {error}"))?;
    temporary.persist(path).map_err(|error| {
        format!(
            "atomically publish native reblock binding {}: {}",
            path.display(),
            error.error
        )
    })?;
    sync_directory(parent, "native reblock binding publication")?;

    Ok(())
}

pub(super) fn read_native_reblock_binding(
    path: &Path,
    native_path: &Path,
) -> TraceStorageResult<NativeReblockBinding> {
    reject_conversion_symlink(path)?;
    let file = File::open(path)
        .map_err(|error| format!("open native reblock binding {}: {error}", path.display()))?;
    let binding: NativeReblockBinding = serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("read native reblock binding {}: {error}", path.display()))?;
    storage_ensure!(
        matches!(binding.version, TRACE_NATIVE_VERSION),
        "native reblock binding {} has unsupported version {}",
        path.display(),
        binding.version
    );
    storage_ensure!(
        (binding.canonical_path) == (native_reblock_canonical_path(native_path)?),
        "native reblock binding {} belongs to another canonical path",
        path.display()
    );
    Ok(binding)
}

pub(super) fn validate_native_reblock_source_file_identity(
    source_path: &Path,
    binding: &NativeReblockBinding,
) -> Result<(), String> {
    let (bytes, content_sha256, device, inode) = native_reblock_file_identity(source_path)?;
    if !native_reblock_file_identity_matches(binding, bytes, &content_sha256, device, inode) {
        return Err(format!(
            "native reblock recovery {} content or inode is not the bound source",
            source_path.display()
        ));
    }
    Ok(())
}

pub(super) fn native_reblock_file_identity_matches(
    binding: &NativeReblockBinding,
    bytes: u64,
    content_sha256: &str,
    device: u64,
    inode: u64,
) -> bool {
    if bytes != binding.source_bytes || content_sha256 != binding.source_content_sha256 {
        return false;
    }
    #[cfg(unix)]
    {
        device == binding.source_device && inode == binding.source_inode
    }
    #[cfg(not(unix))]
    {
        let _ = (device, inode);
        true
    }
}

pub(super) fn validate_native_reblock_semantics(
    path: &Path,
    binding: &NativeReblockBinding,
    label: &str,
) -> TraceStorageResult<()> {
    let (frame_count, final_frame, semantic_sha256) =
        native_reblock_semantic_identity_with_version_policy(
            path,
            binding.version == TRACE_NATIVE_VERSION,
        )?;
    storage_ensure!(
        (frame_count) == (binding.frame_count),
        "{label} frame count changed"
    );
    storage_ensure!(
        (final_frame) == (binding.final_frame),
        "{label} final frame changed"
    );
    storage_ensure!(
        (semantic_sha256) == (binding.source_semantic_sha256),
        "{label} is not semantically identical to the bound source"
    );

    Ok(())
}

pub(super) fn refresh_native_reblock_binding(
    binding_path: &Path,
    source_path: &Path,
    mut binding: NativeReblockBinding,
) -> TraceStorageResult<NativeReblockBinding> {
    validate_native_reblock_source_file_identity(source_path, &binding)
        .map_err(|error| format!("{error}"))?;
    let (frame_count, final_frame, semantic_sha256) =
        native_reblock_semantic_identity(source_path)?;
    storage_ensure!(
        (frame_count) == (binding.frame_count),
        "native trace invariant failed: assert_eq"
    );
    storage_ensure!(
        (final_frame) == (binding.final_frame),
        "native trace invariant failed: assert_eq"
    );
    if binding.version == TRACE_NATIVE_VERSION && binding.source_semantic_sha256 == semantic_sha256
    {
        return Ok(binding);
    }
    binding.version = TRACE_NATIVE_VERSION;
    binding.source_semantic_sha256 = semantic_sha256;
    write_native_reblock_binding(binding_path, &binding)?;
    eprintln!(
        "refreshed native reblock binding {} after exact source authentication",
        binding_path.display()
    );
    Ok(binding)
}

pub(super) enum NativeReblockPreparation {
    Ready(NativeReblockBinding),
    AlreadyCommitted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NativeReblockRecoveryState {
    Fresh,
    AuthenticatedPair,
    BindingOnly,
}

pub(super) fn classify_native_reblock_recovery_state(
    native_exists: bool,
    source_exists: bool,
    binding_exists: bool,
) -> Result<NativeReblockRecoveryState, String> {
    match (native_exists, source_exists, binding_exists) {
        (true, false, false) => Ok(NativeReblockRecoveryState::Fresh),
        (_, true, true) => Ok(NativeReblockRecoveryState::AuthenticatedPair),
        (true, false, true) => Ok(NativeReblockRecoveryState::BindingOnly),
        (_, true, false) => {
            Err("native reblock recovery source has no authenticated binding".to_owned())
        }
        (false, false, false) => {
            Err("native trace is missing and has no authenticated recovery pair".to_owned())
        }
        (false, false, true) => Err(
            "native reblock binding exists without canonical trace or recovery source".to_owned(),
        ),
    }
}

pub(super) fn prepare_native_reblock_source(
    native_path: &Path,
    source_path: &Path,
    binding_path: &Path,
) -> TraceStorageResult<NativeReblockPreparation> {
    let source_exists = source_path.exists();
    let binding_exists = binding_path.exists();
    let recovery_state = classify_native_reblock_recovery_state(
        native_path.is_file(),
        source_exists,
        binding_exists,
    ).map_err(|error| format!(
            "{error}: canonical={} source={} binding={}; preserve every existing path for manual audit",
            native_path.display(),
            source_path.display(),
            binding_path.display()
        ))?;

    if recovery_state == NativeReblockRecoveryState::Fresh {
        let binding = create_native_reblock_binding(native_path)?;
        write_native_reblock_binding(binding_path, &binding)?;
        std::fs::hard_link(native_path, source_path).map_err(|error| {
            format!(
                "create authenticated reblock source {} for {}: {error}",
                source_path.display(),
                native_path.display()
            )
        })?;
        sync_directory(
            native_path.parent().unwrap(),
            "native parity trace reblock source publication",
        )?;
        validate_native_reblock_source_file_identity(source_path, &binding)
            .map_err(|error| format!("{error}"))?;
        return Ok(NativeReblockPreparation::Ready(binding));
    }

    let binding = read_native_reblock_binding(binding_path, native_path)?;
    if recovery_state == NativeReblockRecoveryState::BindingOnly {
        let (bytes, content_sha256, device, inode) = native_reblock_file_identity(native_path)?;
        if native_reblock_file_identity_matches(&binding, bytes, &content_sha256, device, inode) {
            std::fs::hard_link(native_path, source_path).map_err(|error| {
                format!(
                    "resume authenticated reblock source publication {}: {error}",
                    source_path.display()
                )
            })?;
            sync_directory(
                native_path.parent().unwrap(),
                "resumed native parity trace reblock source publication",
            )?;
            let binding = refresh_native_reblock_binding(binding_path, source_path, binding)?;
            return Ok(NativeReblockPreparation::Ready(binding));
        }

        // The source link is removed before its binding during commit. A
        // semantically identical but byte-distinct canonical file proves the
        // atomic publication completed and only binding cleanup remains.
        validate_native_reblock_semantics(native_path, &binding, "published canonical trace")?;
        std::fs::remove_file(binding_path).map_err(|error| {
            format!(
                "finish committed native reblock binding cleanup {}: {error}",
                binding_path.display()
            )
        })?;
        sync_directory(
            native_path.parent().unwrap(),
            "committed native reblock binding cleanup",
        )?;
        return Ok(NativeReblockPreparation::AlreadyCommitted);
    }

    let binding = refresh_native_reblock_binding(binding_path, source_path, binding)?;
    if native_path.exists() {
        validate_native_reblock_semantics(native_path, &binding, "canonical trace")?;
    }
    eprintln!(
        "resuming authenticated reblock of {} from {}",
        native_path.display(),
        source_path.display()
    );
    Ok(NativeReblockPreparation::Ready(binding))
}

pub(super) fn requested_native_trace_path(trace_path: &Path) -> TraceStorageResult<PathBuf> {
    let requested = absolute_trace_path(trace_path)?;
    Ok(
        if requested
            .as_os_str()
            .to_string_lossy()
            .ends_with(TRACE_NATIVE_SUFFIX)
        {
            requested
        } else {
            native_binary_trace_path(&requested)
        },
    )
}

pub(super) fn update_native_semantic_digest<T: bitcode::Encode + ?Sized>(
    digest: &mut Sha256,
    value: &T,
) {
    let encoded = bitcode::encode(value);
    digest.update((encoded.len() as u64).to_le_bytes());
    digest.update(encoded);
}

/// Rewrite an authoritative version-66, version-67, or version-68 native trace
/// into bounded bitcode blocks and a bounded-window zstd frame. The semantic
/// record stream, frame count, and final frame are unchanged; legacy inputs
/// migrate to the current header/footer version through their frozen decoders.
///
/// A hard-link recovery source is synced before the atomic replacement. If
/// the process crashes at any later point, rerunning `--reblock` reads that
/// original inode and retries; the link is removed only after a complete
/// semantic-digest and timeline readback of the replacement.
pub(super) fn reblock_native_trace(
    trace_path: &Path,
    policy: NativeStoragePolicy,
) -> TraceStorageResult<()> {
    let native_path = requested_native_trace_path(trace_path)?;
    let display = native_path.display();
    let parent = native_path
        .parent()
        .storage_context("absolute native trace always has a parent directory")?;
    let source_path = native_reblock_source_path(&native_path);
    let binding_path = native_reblock_binding_path(&native_path);
    let _generation_lock = lock_native_trace_generation(&native_path)?;
    reject_conversion_symlink(&native_path)?;
    reject_conversion_symlink(&source_path)?;
    reject_conversion_symlink(&binding_path)?;
    cleanup_native_reblock_orphans(&native_path)?;
    let binding = match prepare_native_reblock_source(&native_path, &source_path, &binding_path)? {
        NativeReblockPreparation::Ready(binding) => binding,
        NativeReblockPreparation::AlreadyCommitted => {
            eprintln!("native trace reblock was already committed for {display}");
            return Ok(());
        }
    };
    let source_bytes = binding.source_bytes;
    let started = Instant::now();
    let mut source = BinaryTraceReader::open(&source_path)?;
    let source_footer = source.footer;
    validate_binary_trace_footer(&source_footer).map_err(|error| {
        format!(
            "native parity trace reblock source {} has an invalid footer: {error}",
            source_path.display()
        )
    })?;
    let mut header = source.read_header()?;
    storage_ensure!(
        (header.version) == (source_footer.version),
        "native parity trace reblock source header/footer versions differ"
    );
    // The reader has already projected every supported legacy layout into the
    // current in-memory representation. Reblocking is therefore also the
    // native-format migration boundary: always emit the current header/footer
    // version, and compare semantic digests after the same normalization.
    header.version = TRACE_NATIVE_VERSION;
    let output_footer = BinaryTraceFooter {
        version: TRACE_NATIVE_VERSION,
        ..source_footer
    };
    let mut source_digest = Sha256::new();
    update_native_semantic_digest(&mut source_digest, &header);
    let mut timeline = TraceTimeline::new(header.trace.initial_frame);
    let mut frame_count = 0_u64;

    let mut temporary = tempfile::Builder::new()
        .prefix(&native_reblock_temporary_prefix(&native_path, false)?)
        .tempfile_in(parent)
        .map_err(|error| {
            format!("create temporary reblocked native trace beside {display}: {error}")
        })?;
    {
        let mut encoder = zstd::stream::write::Encoder::new(
            BufWriter::new(temporary.as_file_mut()),
            TRACE_NATIVE_ZSTD_LEVEL,
        )
        .map_err(|error| format!("start native trace reblock compression: {error}"))?;
        configure_cache_compression(&mut encoder, None, policy.window_log)?;
        write_binary_record(&mut encoder, &header, "reblocked native trace header")?;
        let mut block = Vec::with_capacity(policy.block_records);
        loop {
            let record = source.read_record()?;
            update_native_semantic_digest(&mut source_digest, &record);
            let terminal = match &record {
                BinaryTraceRecord::Frame(frame) => {
                    timeline
                        .observe(frame.frame_before, frame.frame_after)
                        .map_err(|error| {
                            format!("native trace reblock source timeline is invalid: {error}")
                        })?;
                    frame_count += 1;
                    None
                }
                BinaryTraceRecord::End {
                    rng_suffix,
                    final_frame,
                    frame_count: terminal_frame_count,
                } => {
                    storage_ensure!(
                        rng_suffix.is_some(),
                        "native trace reblock source lost RNG suffix"
                    );
                    Some((
                        terminal_frame_count.storage_context(
                            "native trace reblock source lost terminal frame count",
                        )?,
                        final_frame
                            .storage_context("native trace reblock source lost final frame")?,
                    ))
                }
            };
            block.push(record);
            if block.len() == policy.block_records || terminal.is_some() {
                write_binary_record(
                    &mut encoder,
                    block.as_slice(),
                    "reblocked native trace frame block",
                )?;
                block.clear();
            }
            if let Some((terminal_frame_count, final_frame)) = terminal {
                storage_ensure!(
                    (terminal_frame_count) == (frame_count),
                    "native trace invariant failed: assert_eq"
                );
                timeline
                    .validate_terminator(terminal_frame_count, final_frame)
                    .map_err(|error| {
                        format!("native trace reblock terminator is invalid: {error}")
                    })?;
                source
                    .validate_terminator(terminal_frame_count, final_frame)
                    .map_err(|error| {
                        format!("native trace reblock source footer is invalid: {error}")
                    })?;
                break;
            }
        }
        let mut writer = encoder
            .finish()
            .map_err(|error| format!("finish native trace reblock compression: {error}"))?;
        write_binary_trace_footer(&mut writer, output_footer)
            .map_err(|error| format!("write reblocked native trace footer: {error}"))?;
        writer
            .flush()
            .map_err(|error| format!("flush reblocked native trace: {error}"))?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("sync reblocked native trace: {error}"))?;
    let expected_digest = source_digest.finalize();
    storage_ensure!(
        (sha256_hex(&expected_digest)) == (binding.source_semantic_sha256),
        "native trace reblock source changed after its authenticated binding was published"
    );
    let (decoded_frames, actual_digest) = digest_and_validate_native_trace(temporary.path())?;
    storage_ensure!(
        (decoded_frames) == (frame_count),
        "native trace invariant failed: assert_eq"
    );
    storage_ensure!(
        (actual_digest) == (expected_digest),
        "temporary reblocked native trace for {display} differs semantically from its recovery source"
    );
    temporary.persist(&native_path).map_err(|error| {
        format!(
            "atomically publish reblocked native trace {display}: {}",
            error.error
        )
    })?;
    sync_directory(parent, "reblocked native trace publication")?;
    std::fs::remove_file(&source_path).map_err(|error| {
        format!(
            "remove verified native trace reblock source {}: {error}",
            source_path.display()
        )
    })?;
    std::fs::remove_file(&binding_path).map_err(|error| {
        format!(
            "remove verified native trace reblock binding {}: {error}",
            binding_path.display()
        )
    })?;
    sync_directory(parent, "verified native trace reblock commit")?;
    let output_bytes = std::fs::metadata(&native_path)
        .storage_context("stat reblocked native parity trace")?
        .len();
    eprintln!(
        "reblocked {display}: {frame_count} frames, {:.2} MiB -> {:.2} MiB in {:.1}s ({} records/block, {} MiB zstd window)",
        source_bytes as f64 / (1024.0 * 1024.0),
        output_bytes as f64 / (1024.0 * 1024.0),
        started.elapsed().as_secs_f64(),
        policy.block_records,
        1_u64 << (policy.window_log - 20),
    );

    Ok(())
}

/// Read and semantically validate a native trace without running the replay.
/// This is intentionally read-only so a migration operator can compare old
/// and reblocked artifacts by digest, elapsed time, and peak process RSS.
pub(super) fn validate_native_trace(trace_path: &Path) -> TraceStorageResult<()> {
    let native_path = requested_native_trace_path(trace_path)?;
    let started = Instant::now();
    let (frame_count, digest) = digest_and_validate_native_trace(&native_path)?;
    eprintln!(
        "validated {}: {frame_count} frames, semantic_sha256={}, {:.1}s",
        native_path.display(),
        sha256_hex(&digest),
        started.elapsed().as_secs_f64(),
    );

    Ok(())
}

pub(super) fn digest_and_validate_native_trace(
    path: &Path,
) -> TraceStorageResult<(u64, sha2::digest::Output<Sha256>)> {
    Ok(digest_and_validate_native_trace_with_version_policy(
        path, true,
    )?)
}

pub(super) fn digest_and_validate_native_trace_with_version_policy(
    path: &Path,
    normalize_container_version: bool,
) -> TraceStorageResult<(u64, sha2::digest::Output<Sha256>)> {
    let mut reader = BinaryTraceReader::open(path)?;
    validate_binary_trace_footer(&reader.footer)
        .map_err(|error| format!("reblocked native trace footer is invalid: {error}"))?;
    let mut header = reader.read_header()?;
    storage_ensure!(
        (header.version) == (reader.footer.version),
        "native parity trace header/footer versions differ"
    );
    // Container versions describe encoding layouts, not replay semantics.
    // Readers project supported legacy layouts into the current types, so
    // normalize the header before hashing to compare migrations faithfully.
    if normalize_container_version {
        header.version = TRACE_NATIVE_VERSION;
    }
    let mut digest = Sha256::new();
    update_native_semantic_digest(&mut digest, &header);
    let mut timeline = TraceTimeline::new(header.trace.initial_frame);
    let mut decoded_frames = 0_u64;
    loop {
        let record = reader.read_record()?;
        update_native_semantic_digest(&mut digest, &record);
        match record {
            BinaryTraceRecord::Frame(frame) => {
                timeline
                    .observe(frame.frame_before, frame.frame_after)
                    .map_err(|error| format!("reblocked native trace timeline: {error}"))?;
                decoded_frames += 1;
            }
            BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => {
                storage_ensure!(
                    rng_suffix.is_some(),
                    "reblocked native trace lost RNG suffix"
                );
                let frame_count =
                    frame_count.storage_context("reblocked native trace lost frame count")?;
                let final_frame =
                    final_frame.storage_context("reblocked native trace lost final frame")?;
                storage_ensure!(
                    (frame_count) == (decoded_frames),
                    "native trace invariant failed: assert_eq"
                );
                timeline
                    .validate_terminator(frame_count, final_frame)
                    .map_err(|error| format!("reblocked native trace terminator: {error}"))?;
                reader
                    .validate_terminator(frame_count, final_frame)
                    .map_err(|error| format!("reblocked native trace footer: {error}"))?;
                return Ok((decoded_frames, digest.finalize()));
            }
        }
    }
}

pub(super) fn ensure_native_binary_trace(
    trace_path: &std::path::Path,
) -> TraceStorageResult<PathBuf> {
    if trace_path
        .as_os_str()
        .to_string_lossy()
        .ends_with(TRACE_NATIVE_SUFFIX)
    {
        // The native trace itself was passed; it is the artifact, not a
        // derivation of one.
        validate_standalone_native_trace(trace_path)?;
        return Ok(trace_path.to_owned());
    }
    let native_path = native_binary_trace_path(trace_path);
    if !trace_path.exists() {
        storage_ensure!(
            native_path.is_file(),
            "parity trace {} does not exist and has no native counterpart {}",
            trace_path.display(),
            native_path.display()
        );
        // The recording was converted and deleted; the native trace is the
        // authoritative replacement.
        validate_standalone_native_trace(&native_path)?;
        return Ok(native_path);
    }
    let _generation_lock = lock_native_trace_generation(&native_path)?;
    let fingerprint = trace_source_fingerprint(trace_path)?;
    Ok(ensure_native_binary_trace_locked(
        trace_path,
        &native_path,
        fingerprint,
    )?)
}

pub(super) fn ensure_native_binary_trace_locked(
    trace_path: &Path,
    native_path: &Path,
    fingerprint: String,
) -> TraceStorageResult<PathBuf> {
    match try_read_binary_trace_header(native_path) {
        Ok(header)
            if header.version == TRACE_NATIVE_VERSION
                && header.source_fingerprint == fingerprint =>
        {
            let footer = read_binary_trace_footer(native_path).map_err(|error| format!(
                    "native parity trace {} has a corrupt or missing fixed footer: {error}; remove the derived native file and retry (its JSONL source is still present)",
                    native_path.display()
                ))?;
            validate_binary_trace_footer(&footer).map_err(|error| format!(
                    "native parity trace {} has an invalid fixed footer: {error}; remove the derived native file and retry (its JSONL source is still present)",
                    native_path.display()
                ))?;
            eprintln!("loaded native parity trace {}", native_path.display());
            return Ok(native_path.to_owned());
        }
        Ok(header) => eprintln!(
            "rebuilding stale native parity trace {} (version {}, fingerprint {:?})",
            native_path.display(),
            header.version,
            header.source_fingerprint
        ),
        Err(error) if native_path.exists() => eprintln!(
            "rebuilding unreadable derived native parity trace {} from its JSONL source: {error}",
            native_path.display()
        ),
        Err(_) => eprintln!(
            "building native parity trace {} with bitcode + zstd level {TRACE_NATIVE_ZSTD_LEVEL}",
            native_path.display()
        ),
    }

    let mut lines = open_jsonl_trace(trace_path)?.lines();
    let header_line = lines
        .next()
        .storage_context("parity trace has no header")?
        .storage_context("read parity trace header")?;
    let trace: TraceHeader =
        serde_json::from_str(&header_line).storage_context("parse parity trace header")?;
    verify_trace_line_roundtrip(&trace, &header_line, 1)?;
    validate_trace_header(&trace);
    let rng_prefix_line = lines
        .next()
        .storage_context("parity trace has no RNG prefix")?
        .storage_context("read parity RNG prefix")?;
    let rng_prefix: TraceRngPrefix =
        serde_json::from_str(&rng_prefix_line).storage_context("parse parity RNG prefix")?;
    verify_trace_line_roundtrip(&rng_prefix, &rng_prefix_line, 2)?;
    storage_ensure!(
        (rng_prefix.r#type) == ("rng_prefix"),
        "invalid RNG prefix record type"
    );
    rng_prefix.draws.validate();
    let header = BinaryTraceHeaderV68 {
        version: TRACE_NATIVE_VERSION,
        source_fingerprint: fingerprint,
        trace,
        rng_prefix,
    };

    let parent = match native_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "create temporary native parity trace beside {}: {error}",
            native_path.display()
        )
    })?;
    let started = std::time::Instant::now();
    let mut frame_count = 0_u64;
    let mut trace_timeline = TraceTimeline::new(header.trace.initial_frame);
    // The closure returns the terminal frame purely so the scope's value
    // documents a completed conversion; the footer consumed it inside.
    let _final_frame = std::thread::scope(|scope| {
        let (audit_sender, audit_workers) = spawn_roundtrip_audit_workers(scope);
        let write_result = (|| {
            let mut encoder = zstd::stream::write::Encoder::new(
                BufWriter::new(temporary.as_file_mut()),
                TRACE_NATIVE_ZSTD_LEVEL,
            )
            .map_err(|error| format!("start native parity trace compression: {error}"))?;
            configure_cache_compression(
                &mut encoder,
                expected_native_stream_bytes(trace_path),
                TRACE_NATIVE_WINDOW_LOG,
            )?;
            write_binary_record(&mut encoder, &header, "native parity trace header")?;

            let mut records = lines.enumerate();
            let mut terminal_metadata = None;
            let mut block: Vec<BinaryTraceRecord> = Vec::with_capacity(TRACE_NATIVE_BLOCK_RECORDS);
            while let Some((record_index, line)) = records.next() {
                let line_number = record_index + 3;
                let line = line.map_err(|error| {
                    format!("read parity trace record on line {line_number}: {error}")
                })?;
                audit_sender
                    .send((line_number, line.clone()))
                    .storage_context("parity round-trip audit workers stopped early")?;
                if let Some(frame) = parse_trace_frame(&line, line_number) {
                    validate_trace_frame_with_legacy_additive_omissions(
                        header.trace.schema,
                        &frame,
                        header.trace.initial_npc_transients.is_none(),
                    );
                    trace_timeline
                        .observe(frame.frame_before, frame.frame_after)
                        .map_err(|error| {
                            format!("invalid parity frame timeline on line {line_number}: {error}")
                        })?;
                    block.push(BinaryTraceRecord::Frame(frame));
                    if block.len() >= TRACE_NATIVE_BLOCK_RECORDS {
                        write_binary_record(
                            &mut encoder,
                            block.as_slice(),
                            "parity trace frame block",
                        )?;
                        block.clear();
                    }
                    frame_count += 1;
                    if frame_count.is_multiple_of(500) {
                        eprintln!("cached {frame_count} parity frames");
                    }
                } else {
                    let suffix: TraceRngOnly = serde_json::from_str(&line).map_err(|error| {
                        format!("parse RNG suffix on trace line {line_number}: {error}")
                    })?;
                    storage_ensure!(
                        (suffix.record_type) == ("rng_suffix"),
                        "invalid parity terminator record type on line {line_number}"
                    );
                    suffix.draws.validate();
                    trace_timeline
                        .validate_terminator(suffix.frame_count, suffix.final_frame)
                        .map_err(|error| {
                            format!(
                                "invalid parity terminator timeline on line {line_number}: {error}"
                            )
                        })?;
                    block.push(BinaryTraceRecord::End {
                        rng_suffix: Some(suffix.draws),
                        final_frame: Some(suffix.final_frame),
                        frame_count: Some(suffix.frame_count),
                    });
                    terminal_metadata = Some((suffix.frame_count, suffix.final_frame));
                    if let Some((trailing_index, trailing)) = records.next() {
                        let trailing_line = trailing_index + 3;
                        trailing.map_err(|error| {
                            format!("read trailing parity trace line {trailing_line}: {error}")
                        })?;
                        return Err(format!(
                            "parity trace has a record after its rng_suffix terminator on line {trailing_line}"
                        ));
                    }
                    break;
                }
            }
            let (terminal_frame_count, terminal_final_frame) = terminal_metadata.ok_or_else(|| format!("parity trace ended without an rng_suffix terminator; refusing to publish the native trace"))?;
            storage_ensure!(
                (terminal_frame_count) == (frame_count),
                "native trace invariant failed: assert_eq"
            );
            let final_frame = terminal_final_frame;
            write_binary_record(
                &mut encoder,
                block.as_slice(),
                "native parity trace final block",
            )?;
            let mut writer = encoder
                .finish()
                .map_err(|error| format!("finish native parity trace compression: {error}"))?;
            write_binary_trace_footer(
                &mut writer,
                BinaryTraceFooter {
                    version: TRACE_NATIVE_VERSION,
                    frame_count,
                    final_frame,
                },
            )
            .map_err(|error| format!("write native parity trace fixed footer: {error}"))?;
            writer
                .flush()
                .map_err(|error| format!("flush native parity trace: {error}"))?;
            Ok(final_frame)
        })();
        drop(audit_sender);
        // Join all workers even if encoding failed. An audit error must never
        // be lost behind a disconnected-channel error or publish a temp file.
        join_roundtrip_audit_workers(audit_workers)?;
        write_result
    })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("sync native parity trace: {error}"))?;
    temporary.persist(native_path).map_err(|error| {
        format!(
            "persist native parity trace {}: {}",
            native_path.display(),
            error.error
        )
    })?;
    sync_directory(parent, "native parity trace publication")?;
    let compressed_bytes = std::fs::metadata(native_path)
        .storage_context("stat completed native parity trace")?
        .len();
    eprintln!(
        "cached {frame_count} frames in {} ({:.1} MiB, {:.1}s)",
        native_path.display(),
        compressed_bytes as f64 / (1024.0 * 1024.0),
        started.elapsed().as_secs_f64()
    );
    Ok(native_path.to_owned())
}

impl BinaryTraceReader {
    pub(super) fn open(path: &std::path::Path) -> TraceStorageResult<Self> {
        let footer = read_binary_trace_footer(path).map_err(|error| {
            format!(
                "read native parity trace fixed footer {}: {error}",
                path.display()
            )
        })?;
        let file = File::open(path)
            .map_err(|error| format!("open native parity trace {}: {error}", path.display()))?;
        let compressed_len = file
            .metadata()
            .storage_context("stat native parity trace before decompression")?
            .len()
            .checked_sub(TRACE_NATIVE_FOOTER_LEN)
            .storage_context("validated native parity trace is shorter than its footer")?;
        // The fixed footer is outside the zstd stream so it can be checked
        // without decoding a potentially ABI-incompatible file. Bound zstd
        // to the compressed bytes or it treats the footer as another frame.
        let mut decoder =
            zstd::stream::read::Decoder::new(file.take(compressed_len)).map_err(|error| {
                format!(
                    "start native parity trace decompression {}: {error}",
                    path.display()
                )
            })?;
        decoder
            .window_log_max(TRACE_ZSTD_WINDOW_LOG_MAX)
            .map_err(|error| {
                format!(
                    "configure native parity trace decompression {}: {error}",
                    path.display()
                )
            })?;
        Ok(Self {
            path: path.to_owned(),
            reader: Box::new(decoder),
            footer,
            pending: VecDeque::new(),
        })
    }

    pub(super) fn read_header(&mut self) -> TraceStorageResult<BinaryTraceHeaderV68> {
        Ok(
            read_binary_trace_header_record(&mut self.reader, self.footer.version).map_err(
                |error| {
                    format!(
                        "read native parity trace header {}: {error}",
                        self.path.display()
                    )
                },
            )?,
        )
    }

    pub(super) fn read_record(&mut self) -> TraceStorageResult<BinaryTraceRecord> {
        if let Some(record) = self.pending.pop_front() {
            return Ok(record);
        }
        let block = read_binary_trace_block_record(&mut self.reader, self.footer.version).map_err(
            |error| {
                format!(
                    "read native parity trace block {}: {error}",
                    self.path.display()
                )
            },
        )?;
        self.pending.extend(block);
        Ok(self.pending.pop_front().ok_or_else(|| {
            format!(
                "native parity trace {} contains an empty record block",
                self.path.display()
            )
        })?)
    }

    pub(super) fn validate_terminator(
        &mut self,
        frame_count: u64,
        final_frame: u64,
    ) -> Result<(), String> {
        if self.footer.frame_count != frame_count || self.footer.final_frame != final_frame {
            return Err(format!(
                "decoded terminator says frame_count={frame_count} final_frame={final_frame}, fixed footer says frame_count={} final_frame={}",
                self.footer.frame_count, self.footer.final_frame
            ));
        }
        if !self.pending.is_empty() {
            return Err(
                "decoded native trace contains records after its first End record in the same block"
                    .to_owned(),
            );
        }
        let mut trailing = [0_u8; 1];
        match self.reader.read(&mut trailing) {
            Ok(0) => Ok(()),
            Ok(_) => {
                Err("decoded native trace contains data after its first End record".to_owned())
            }
            Err(error) => Err(format!(
                "decompress cache after its first End record: {error}"
            )),
        }
    }
}

pub(super) fn write_binary_trace_footer(
    writer: &mut impl Write,
    footer: BinaryTraceFooter,
) -> std::io::Result<()> {
    writer.write_all(&TRACE_NATIVE_FOOTER_MAGIC)?;
    writer.write_all(&footer.version.to_le_bytes())?;
    writer.write_all(&footer.frame_count.to_le_bytes())?;
    writer.write_all(&footer.final_frame.to_le_bytes())?;
    Ok(())
}

pub(super) fn read_binary_trace_footer(path: &Path) -> Result<BinaryTraceFooter, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    if length < TRACE_NATIVE_FOOTER_LEN {
        return Err(format!(
            "file is {length} bytes, shorter than the {TRACE_NATIVE_FOOTER_LEN}-byte footer"
        ));
    }
    file.seek(SeekFrom::End(
        -i64::try_from(TRACE_NATIVE_FOOTER_LEN).unwrap(),
    ))
    .map_err(|error| error.to_string())?;
    let mut magic = [0_u8; TRACE_NATIVE_FOOTER_MAGIC.len()];
    file.read_exact(&mut magic)
        .map_err(|error| error.to_string())?;
    if magic != TRACE_NATIVE_FOOTER_MAGIC {
        return Err(format!("footer magic is {magic:?}"));
    }
    let mut version = [0_u8; 4];
    let mut frame_count = [0_u8; 8];
    let mut final_frame = [0_u8; 8];
    file.read_exact(&mut version)
        .map_err(|error| error.to_string())?;
    file.read_exact(&mut frame_count)
        .map_err(|error| error.to_string())?;
    file.read_exact(&mut final_frame)
        .map_err(|error| error.to_string())?;
    Ok(BinaryTraceFooter {
        version: u32::from_le_bytes(version),
        frame_count: u64::from_le_bytes(frame_count),
        final_frame: u64::from_le_bytes(final_frame),
    })
}

pub(super) fn validate_binary_trace_footer(footer: &BinaryTraceFooter) -> Result<(), String> {
    validate_native_version(footer.version)
}

fn validate_native_version(version: u32) -> Result<(), String> {
    if version != TRACE_NATIVE_VERSION {
        return Err(format!(
            "native parity trace version {version} is unsupported; expected {TRACE_NATIVE_VERSION}; migrate this authoritative artifact offline with its original runner"
        ));
    }
    Ok(())
}

pub(super) fn try_read_binary_trace_header(
    path: &std::path::Path,
) -> Result<BinaryTraceHeaderV68, String> {
    let footer = read_binary_trace_footer(path)?;
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut decoder = zstd::stream::read::Decoder::new(file).map_err(|error| error.to_string())?;
    decoder
        .window_log_max(TRACE_ZSTD_WINDOW_LOG_MAX)
        .map_err(|error| error.to_string())?;
    read_binary_trace_header_record(&mut decoder, footer.version)
}

/// A compression window large enough for short encoded streams and otherwise
/// capped at [`TRACE_NATIVE_WINDOW_LOG`]. A wider zstd frame makes every
/// replay decoder retain that history even though parity consumes records
/// strictly once and in order. The bounded window is therefore part of the
/// replay lane's memory contract, not merely an encoder tuning parameter.
#[cfg(test)]
pub(super) fn native_stream_window_log(expected_bytes: Option<u64>) -> u32 {
    native_stream_window_log_capped(expected_bytes, TRACE_NATIVE_WINDOW_LOG)
}

pub(super) fn native_stream_window_log_capped(
    expected_bytes: Option<u64>,
    maximum_window_log: u32,
) -> u32 {
    assert!(
        (TRACE_NATIVE_MIN_WINDOW_LOG..=TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG)
            .contains(&maximum_window_log),
        "native trace window cap is outside the supported writer range"
    );
    let Some(bytes) = expected_bytes else {
        return maximum_window_log;
    };
    // ceil(log2(bytes)): the smallest window that still spans the estimate.
    let spanning = u64::BITS
        - bytes
            .max(1)
            .checked_next_power_of_two()
            .unwrap_or(u64::MAX)
            .leading_zeros()
        - 1;
    // Below ~1 MiB the window stops being what limits matching, and a stream
    // that outgrows the estimate only loses long-range matches, never data.
    spanning.clamp(TRACE_NATIVE_MIN_WINDOW_LOG, maximum_window_log)
}

/// How much bitcode a recording is expected to encode into. Measured traces
/// pack to roughly a fifth of their JSONL; a quarter leaves room for one that
/// packs worse. `None` when the recording's uncompressed size is unknown, and
/// the window then falls back to the maximum.
pub(super) fn expected_native_stream_bytes(trace_path: &std::path::Path) -> Option<u64> {
    Some(recording_uncompressed_bytes(trace_path)? / 4)
}

/// The uncompressed byte count of a JSONL recording: the file length, or the
/// content size a `.zst` recording declares in its frame header.
pub(super) fn recording_uncompressed_bytes(trace_path: &std::path::Path) -> Option<u64> {
    let length = std::fs::metadata(trace_path).ok()?.len();
    if !trace_path.as_os_str().to_string_lossy().ends_with(".zst") {
        return Some(length);
    }
    // A zstd frame header is at most 18 bytes.
    let mut header = [0_u8; 18];
    let read = {
        let mut file = File::open(trace_path).ok()?;
        read_up_to(&mut file, &mut header).ok()?
    };
    // An unknown content size is reported as an error or as zero; either way
    // the caller falls back to the widest window rather than guessing small.
    match zstd::zstd_safe::get_frame_content_size(&header[..read]) {
        Ok(Some(size)) if size > 0 => Some(size),
        _ => None,
    }
}

/// Fill `buffer` as far as the reader allows, returning how much was read.
pub(super) fn read_up_to<R: Read>(reader: &mut R, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

pub(super) fn configure_cache_compression<W: Write>(
    encoder: &mut zstd::stream::write::Encoder<'_, W>,
    expected_bytes: Option<u64>,
    maximum_window_log: u32,
) -> TraceStorageResult<()> {
    if TRACE_NATIVE_LONG_DISTANCE_MATCHING {
        encoder
            .long_distance_matching(true)
            .map_err(|error| format!("enable cache long-distance matching: {error}"))?;
    }
    encoder
        .window_log(native_stream_window_log_capped(
            expected_bytes,
            maximum_window_log,
        ))
        .map_err(|error| format!("configure cache compression window: {error}"))?;

    Ok(())
}

pub(super) fn read_binary_trace_header(
    path: &std::path::Path,
) -> TraceStorageResult<BinaryTraceHeaderV68> {
    Ok(try_read_binary_trace_header(path).map_err(|error| {
        format!(
            "read native parity trace header {} after conversion: {error}",
            path.display()
        )
    })?)
}

pub(super) fn write_binary_record<T: bitcode::Encode + ?Sized>(
    writer: &mut impl Write,
    value: &T,
    label: &str,
) -> TraceStorageResult<()> {
    let encoded = bitcode::encode(value);
    let length =
        u64::try_from(encoded.len()).storage_context("binary record length exceeds u64")?;
    writer
        .write_all(&length.to_le_bytes())
        .map_err(|error| format!("write {label} length: {error}"))?;
    writer
        .write_all(&encoded)
        .map_err(|error| format!("write {label}: {error}"))?;

    Ok(())
}

#[cfg(test)]
pub(super) fn read_binary_record<T: bitcode::DecodeOwned>(
    reader: &mut dyn Read,
    label: &str,
) -> Result<T, String> {
    let encoded = read_binary_record_payload(reader, label)?;
    bitcode::decode(&encoded).map_err(|error| format!("decode {label}: {error}"))
}

pub(super) fn read_binary_record_payload(
    reader: &mut dyn Read,
    label: &str,
) -> Result<Vec<u8>, String> {
    const MAX_RECORD_BYTES: u64 = 1024 * 1024 * 1024;
    let mut length_bytes = [0_u8; 8];
    reader
        .read_exact(&mut length_bytes)
        .map_err(|error| format!("read {label} length: {error}"))?;
    let length = u64::from_le_bytes(length_bytes);
    if length > MAX_RECORD_BYTES {
        return Err(format!(
            "{label} length {length} exceeds {MAX_RECORD_BYTES}-byte safety limit"
        ));
    }
    let length = usize::try_from(length)
        .map_err(|_| format!("{label} length cannot be represented on this platform"))?;
    let mut encoded = vec![0_u8; length];
    reader
        .read_exact(&mut encoded)
        .map_err(|error| format!("read {label} payload: {error}"))?;
    Ok(encoded)
}

pub(super) fn read_binary_trace_header_record(
    reader: &mut dyn Read,
    version: u32,
) -> Result<BinaryTraceHeaderV68, String> {
    validate_native_version(version)?;
    read_binary_record(reader, "native parity trace header")
}

pub(super) fn read_binary_trace_block_record(
    reader: &mut dyn Read,
    version: u32,
) -> Result<Vec<BinaryTraceRecord>, String> {
    validate_native_version(version)?;
    read_binary_record(reader, "native parity trace block")
}

/// Storage experiment for the canonical trace layout: re-encode the cached
/// records of one trace with bitcode in several layouts and report raw and
/// zstd-compressed sizes. Each candidate is streamed independently so the
/// benchmark retains at most one block of decoded frames.
///
/// `PARITY_BENCH_ZSTD_LEVELS` (default `3,19`) and `PARITY_BENCH_BLOCKS`
/// (default `16,64,256`) tune the sweep.
pub(super) fn bench_trace_encodings(trace_path: &Path) -> TraceStorageResult<()> {
    #[derive(Default)]
    struct CountingWriter {
        bytes: usize,
    }

    impl Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes += bytes.len();
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    pub(super) fn env_list(name: &str, default: &str) -> TraceStorageResult<Vec<usize>> {
        std::env::var(name)
            .unwrap_or_else(|_| default.to_owned())
            .split(',')
            .map(|item| {
                item.trim()
                    .parse()
                    .map_err(|error| format!("parse {name} item {item:?}: {error}"))
            })
            .collect()
    }

    pub(super) fn measured_record<T: bitcode::Encode + ?Sized>(
        encoder: &mut zstd::stream::write::Encoder<'_, CountingWriter>,
        raw_bytes: &mut usize,
        value: &T,
    ) -> TraceStorageResult<()> {
        let encoded = bitcode::encode(value);
        let length = (encoded.len() as u64).to_le_bytes();
        *raw_bytes += length.len() + encoded.len();
        encoder
            .write_all(&length)
            .storage_context("write benchmark length")?;
        encoder
            .write_all(&encoded)
            .storage_context("write benchmark record")?;

        Ok(())
    }

    pub(super) fn measure_layout(
        native_path: &Path,
        records_per_block: usize,
        level: i32,
    ) -> TraceStorageResult<(u64, usize, usize, Duration)> {
        storage_ensure!(
            (records_per_block) != (0),
            "benchmark block size must be nonzero"
        );
        let started = Instant::now();
        let mut reader = BinaryTraceReader::open(native_path)?;
        let header = reader.read_header()?;
        let mut encoder = zstd::stream::write::Encoder::new(CountingWriter::default(), level)
            .map_err(|error| format!("start benchmark zstd level {level}: {error}"))?;
        // Real traces of this scale use the production 64 MiB window. Using
        // the same fixed bound also avoids a raw-size pre-pass per candidate.
        configure_cache_compression(&mut encoder, None, TRACE_NATIVE_WINDOW_LOG)?;
        let mut raw_bytes = 0_usize;
        measured_record(&mut encoder, &mut raw_bytes, &header)?;

        let mut frame_count = 0_u64;
        let mut block = Vec::with_capacity(records_per_block);
        loop {
            let record = reader.read_record()?;
            let is_end = matches!(record, BinaryTraceRecord::End { .. });
            if matches!(record, BinaryTraceRecord::Frame(_)) {
                frame_count += 1;
            }
            block.push(record);
            if block.len() == records_per_block || is_end {
                if records_per_block == 1 {
                    measured_record(&mut encoder, &mut raw_bytes, &block[0])?;
                } else {
                    measured_record(&mut encoder, &mut raw_bytes, block.as_slice())?;
                }
                block.clear();
            }
            if is_end {
                break;
            }
        }

        let compressed_bytes = encoder
            .finish()
            .map_err(|error| format!("finish benchmark zstd level {level}: {error}"))?
            .bytes;
        Ok((frame_count, raw_bytes, compressed_bytes, started.elapsed()))
    }

    let zstd_levels: Vec<i32> = env_list("PARITY_BENCH_ZSTD_LEVELS", "3,19")?
        .into_iter()
        .map(|level| i32::try_from(level).storage_context("zstd level fits i32"))
        .collect::<TraceStorageResult<_>>()?;
    let block_sizes = env_list("PARITY_BENCH_BLOCKS", "16,64,256")?;

    let native_path = ensure_native_binary_trace(trace_path)?;
    let source_bytes = std::fs::metadata(trace_path)
        .storage_context("stat source trace")?
        .len();
    let cache_bytes = std::fs::metadata(&native_path)
        .storage_context("stat trace cache")?
        .len();

    let mut results: Vec<(String, usize, Vec<(i32, usize, Duration)>)> = Vec::new();
    let mut frame_count = None;
    for (name, block) in std::iter::once(("bitcode per-record".to_owned(), 1_usize)).chain(
        block_sizes.iter().map(|&block| {
            let marker = if block == TRACE_NATIVE_BLOCK_RECORDS {
                " (current cache)"
            } else {
                ""
            };
            (format!("bitcode blocks of {block}{marker}"), block)
        }),
    ) {
        let mut raw_bytes = None;
        let mut compressed = Vec::new();
        for &level in &zstd_levels {
            let (measured_frames, measured_raw, size, time) =
                measure_layout(&native_path, block, level)?;
            storage_ensure!(
                (*frame_count.get_or_insert(measured_frames)) == (measured_frames),
                "native trace invariant failed: assert_eq"
            );
            storage_ensure!(
                (*raw_bytes.get_or_insert(measured_raw)) == (measured_raw),
                "native trace invariant failed: assert_eq"
            );
            compressed.push((level, size, time));
        }
        eprintln!("measured {name}");
        results.push((
            name,
            raw_bytes.storage_context("benchmark has a zstd level")?,
            compressed,
        ));
    }
    let frame_count = frame_count.storage_context("benchmark has a layout")?;
    let baseline_raw = results[0].1;

    let mib = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
    println!();
    println!("trace: {} ({} frames)", trace_path.display(), frame_count);
    println!(
        "source artifact: {:.2} MiB; existing native artifact: {:.2} MiB",
        mib(source_bytes as usize),
        mib(cache_bytes as usize)
    );
    println!();
    let mut head = format!("| {:<36} | {:>10} |", "encoding", "raw MiB");
    let mut sep = format!("|{:-<38}|{:->12}|", "", "");
    for level in &zstd_levels {
        head.push_str(&format!(
            " {:>13} | {:>8} |",
            format!("zstd-{level} MiB"),
            "zstd s"
        ));
        sep.push_str(&format!("{:->15}|{:->10}|", "", ""));
    }
    println!("{head}");
    println!("{sep}");
    for (name, raw, compressed) in &results {
        let mut line = format!(
            "| {:<36} | {:>5.2} {:>4.0}% |",
            name,
            mib(*raw),
            100.0 * *raw as f64 / baseline_raw as f64
        );
        for (index, (_, size, time)) in compressed.iter().enumerate() {
            let baseline = results[0].2[index].1;
            line.push_str(&format!(
                " {:>6.2} {:>5.0}% | {:>8.2} |",
                mib(*size),
                100.0 * *size as f64 / baseline as f64,
                time.as_secs_f64()
            ));
        }
        println!("{line}");
    }
    println!();
    println!(
        "percentages are relative to the authoritative per-record bitcode layout at the same zstd level; timings include native decode, bitcode encode, and zstd"
    );

    Ok(())
}

pub(super) fn read_all_rng_draws(native_path: &std::path::Path) -> TraceStorageResult<Vec<u32>> {
    let mut reader = BinaryTraceReader::open(native_path)?;
    let header = reader.read_header()?;
    let mut result = Vec::new();
    let mut original_index = 0_usize;
    let mut frame_count = 0_u64;
    let mut trace_timeline = TraceTimeline::new(header.trace.initial_frame);
    append_simulation_rng_draws(&mut result, &mut original_index, &header.rng_prefix.draws);
    loop {
        match reader.read_record()? {
            BinaryTraceRecord::Frame(frame) => {
                trace_timeline
                    .observe(frame.frame_before, frame.frame_after)
                    .map_err(|error| {
                        format!("invalid cached parity frame timeline during RNG pre-scan: {error}")
                    })?;
                append_simulation_rng_draws(&mut result, &mut original_index, &frame.rng_draws);
                frame_count += 1;
            }
            BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count: terminal_frame_count,
            } => {
                if let Some(batch) = rng_suffix {
                    append_simulation_rng_draws(&mut result, &mut original_index, &batch);
                }
                let terminal_frame_count = terminal_frame_count
                    .ok_or_else(|| format!("parity cache RNG pre-scan found incomplete End"))?;
                let final_frame = final_frame
                    .ok_or_else(|| format!("parity cache RNG pre-scan found incomplete End"))?;
                storage_ensure!(
                    (terminal_frame_count) == (frame_count),
                    "native trace invariant failed: assert_eq"
                );
                trace_timeline
                    .validate_terminator(terminal_frame_count, final_frame)
                    .map_err(|error| {
                        format!(
                            "invalid cached parity terminator timeline during RNG pre-scan: {error}"
                        )
                    })?;
                reader
                    .validate_terminator(terminal_frame_count, final_frame).map_err(|error| format!(
                            "native parity trace {} has an invalid terminal record during RNG pre-scan: {error}",
                            native_path.display()
                        ))?;
                break;
            }
        }
    }
    eprintln!(
        "loaded {} simulation RNG draws from {}",
        result.len(),
        native_path.display()
    );
    Ok(result)
}

pub(super) fn append_simulation_rng_draws(
    result: &mut Vec<u32>,
    original_index: &mut usize,
    batch: &TraceRngBatch,
) {
    assert_eq!(batch.first_index, *original_index, "RNG stream has a gap");
    batch.validate();
    *original_index += batch.values.len();
    result.extend(
        batch
            .values
            .iter()
            .copied()
            .zip(batch.domains.iter().copied())
            .filter_map(|(value, domain)| (domain == TraceRngDomain::Simulation).then_some(value)),
    );
}

pub(super) fn simulation_rng_draws(batch: &TraceRngBatch) -> Vec<u32> {
    batch
        .values
        .iter()
        .copied()
        .zip(batch.domains.iter().copied())
        .filter_map(|(value, domain)| (domain == TraceRngDomain::Simulation).then_some(value))
        .collect()
}

pub(super) fn should_preload_complete_rng_stream(
    start_state: TraceStartState,
    prefix_draw_count: usize,
    forced: bool,
) -> bool {
    forced || (start_state == TraceStartState::LoadedSave && prefix_draw_count == 0)
}

pub(super) fn difference_field(difference: &str) -> &str {
    let entity_field = difference
        .split_once(").")
        .and_then(|(_, tail)| tail.split_once(':'))
        .map(|(field, _)| field);
    entity_field
        .or_else(|| {
            difference
                .strip_prefix("frame.")
                .and_then(|tail| tail.split_once(':'))
                .map(|(field, _)| field)
        })
        .unwrap_or("other")
}

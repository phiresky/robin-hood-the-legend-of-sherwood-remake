//! Audio transformation, catalog construction and grouped packaging.
use super::*;

pub(super) fn insert_shipping_raw(
    payload: &mut ShippingMission,
    relative: &str,
    path: &Path,
) -> Result<()> {
    let relative = relative.replace('\\', "/").to_ascii_lowercase();
    let bytes = fs::read(path).with_context(|| format!("read audio {}", path.display()))?;
    if let Some(previous) = payload.raw.get(&relative) {
        if previous != &bytes {
            bail!("conflicting shipping audio sources for {relative}");
        }
    } else {
        payload.raw.insert(relative, bytes);
    }
    Ok(())
}

pub(super) fn is_common_audio_member(
    relative: &str,
    mission_dialogue_keys: &BTreeSet<String>,
) -> bool {
    !relative.starts_with("menu/")
        && !relative.starts_with("exclamations/")
        && !is_sound_source_audio(relative)
        && !mission_dialogue_keys.contains(&format!("sounds/{relative}"))
}

pub(super) fn is_sound_source_audio(relative: &str) -> bool {
    let Some(name) = relative.strip_prefix("snd_").and_then(|name| {
        name.strip_suffix(".wav")
            .or_else(|| name.strip_suffix(".ogg"))
    }) else {
        return false;
    };
    name.len() >= 3 && name.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Debug, Clone, Copy)]
pub(super) enum AudioKind {
    Voice,
    Effect,
    Music,
}

impl AudioKind {
    /// Target opusenc VBR bitrate. Everything else (signal type, bandwidth,
    /// SILK/CELT/hybrid mode) stays on the opusenc/libopus defaults: see
    /// `transcode_audio_to_opus` and docs/COMPRESSION.md (2026-09-14).
    pub(super) fn bitrate_kbps(self) -> u32 {
        match self {
            Self::Voice => 24,
            // 48 -> 40 kbit/s with the libopus 1.6.1 upgrade (2026-09-14).
            Self::Effect => 40,
            // 64 -> 48 kbit/s came with the lossless remaster sources (see
            // `music_lossless_source`); 48 -> 40 with libopus 1.6.1.
            Self::Music => 40,
        }
    }
}

/// The only libopus release browser Opus encodes may use. Encodes are
/// content-addressed, so a different libopus silently changes every audio
/// chunk hash (and quality); the converter refuses to run with anything else.
pub(super) const REQUIRED_LIBOPUS_VERSION: &str = "libopus 1.6.1";
/// What `opusenc --version` must report about the libopus it runs on.
const REQUIRED_OPUSENC_LIBOPUS: &str = "(using libopus 1.6.1)";

#[derive(Debug)]
pub(super) struct OpusToolchain {
    /// Canonical path of opus-tools' `opusenc` (built against libopusenc and
    /// the pinned libopus, see docs/COMPRESSION.md 2026-09-14).
    opusenc: PathBuf,
    /// Canonical path of the verified `libopus.so.0` that opusenc loads.
    library: PathBuf,
}

static OPUS_TOOLCHAIN: std::sync::OnceLock<OpusToolchain> = std::sync::OnceLock::new();

/// Verify and select the opusenc used by every subsequent Opus encode.
///
/// Runs `<opus_tools_dir>/bin/opusenc --version` and requires it to report
/// [`REQUIRED_OPUSENC_LIBOPUS`]; from glibc's loader trace takes the one
/// `libopus.so.0` that process initialized, loads that file in-process and
/// requires `opus_get_version_string()` to be [`REQUIRED_LIBOPUS_VERSION`].
/// Every later encode repeats the loader check against that exact file (see
/// [`run_verified_opusenc`]).
pub(super) fn configure_opus_toolchain(opus_tools_dir: &Path) -> Result<()> {
    let candidate = opus_tools_dir.join("bin").join("opusenc");
    let opusenc = fs::canonicalize(&candidate)
        .with_context(|| format!("resolve opusenc {}", candidate.display()))?;
    let (probe, loaded) = run_with_loader_trace(Command::new(&opusenc).arg("--version"))?;
    let version_line = String::from_utf8_lossy(&probe.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    anyhow::ensure!(
        probe.status.success() && version_line.contains(REQUIRED_OPUSENC_LIBOPUS),
        "{} --version reports {version_line:?} ({}); web Opus encodes require {REQUIRED_OPUSENC_LIBOPUS:?}",
        opusenc.display(),
        probe.status
    );
    let library = match loaded.iter().collect::<Vec<_>>().as_slice() {
        [single] => (*single).clone(),
        other => bail!(
            "{} must load exactly one libopus.so; loader trace shows {other:?} \
             (a static or setuid opusenc cannot be verified)",
            opusenc.display()
        ),
    };
    let version = libopus_version_string(&library)?;
    anyhow::ensure!(
        version == REQUIRED_LIBOPUS_VERSION,
        "{} (loaded by opusenc) reports {version:?}; web Opus encodes require {REQUIRED_LIBOPUS_VERSION:?}",
        library.display()
    );
    let sha256 = |path: &Path| -> Result<String> {
        use sha2::{Digest as _, Sha256};
        Ok(hex::encode(Sha256::digest(
            fs::read(path).with_context(|| format!("read {}", path.display()))?,
        )))
    };
    tracing::info!(
        opusenc = %opusenc.display(),
        opusenc_version = version_line,
        opusenc_sha256 = sha256(&opusenc)?,
        libopus = %library.display(),
        libopus_version = version,
        libopus_sha256 = sha256(&library)?,
        "verified opusenc and libopus for Opus encodes"
    );
    let toolchain = OpusToolchain { opusenc, library };
    match OPUS_TOOLCHAIN.get() {
        Some(existing)
            if existing.opusenc == toolchain.opusenc && existing.library == toolchain.library =>
        {
            Ok(())
        }
        Some(existing) => bail!(
            "Opus toolchain already configured as {existing:?}, refusing to switch to {toolchain:?}"
        ),
        None => {
            // A concurrent identical configuration is harmless.
            let _ = OPUS_TOOLCHAIN.set(toolchain);
            Ok(())
        }
    }
}

fn libopus_version_string(library: &Path) -> Result<String> {
    // SAFETY: loading libopus runs no initializers with preconditions, and
    // `opus_get_version_string` takes no arguments and returns a pointer to a
    // static NUL-terminated string that lives as long as the library.
    unsafe {
        let lib = libloading::Library::new(library)
            .with_context(|| format!("load {}", library.display()))?;
        let get: libloading::Symbol<unsafe extern "C" fn() -> *const std::ffi::c_char> = lib
            .get(b"opus_get_version_string\0")
            .with_context(|| format!("{} has no opus_get_version_string", library.display()))?;
        let pointer = get();
        anyhow::ensure!(!pointer.is_null(), "opus_get_version_string returned NULL");
        Ok(std::ffi::CStr::from_ptr(pointer)
            .to_str()
            .context("libopus version string is not UTF-8")?
            .to_owned())
    }
}

fn configured_opus_toolchain() -> Result<&'static OpusToolchain> {
    OPUS_TOOLCHAIN.get().with_context(|| {
        format!(
            "Opus encoding requires a verified opusenc on {REQUIRED_LIBOPUS_VERSION} \
             (pass --opus-tools-dir <dir containing bin/opusenc>)"
        )
    })
}

/// Run opusenc and prove from the loader trace that this very process
/// initialized exactly the verified `libopus.so.0`. An `LD_LIBRARY_PATH` or
/// `LD_PRELOAD` pointing at another libopus fails loudly instead of encoding.
fn run_verified_opusenc(
    toolchain: &OpusToolchain,
    command: &mut Command,
) -> Result<std::process::Output> {
    let (output, loaded) = run_with_loader_trace(command)?;
    anyhow::ensure!(
        loaded.len() == 1 && loaded.contains(&toolchain.library),
        "opusenc did not load the verified {REQUIRED_LIBOPUS_VERSION} at {}; loader trace shows {:?}",
        toolchain.library.display(),
        loaded
    );
    Ok(output)
}

/// Run a dynamically linked tool with glibc's loader trace (`LD_DEBUG=libs`,
/// written to a private `LD_DEBUG_OUTPUT` file so its stderr stays clean) and
/// return the canonical paths of every `libopus.so*` it initialized.
fn run_with_loader_trace(
    command: &mut Command,
) -> Result<(std::process::Output, BTreeSet<PathBuf>)> {
    let trace_dir = tempfile::tempdir().context("create loader trace directory")?;
    let output = command
        .env("LD_DEBUG", "libs")
        .env("LD_DEBUG_OUTPUT", trace_dir.path().join("ld"))
        .output()
        .with_context(|| format!("run {:?}", command.get_program()))?;
    let mut loaded = BTreeSet::new();
    for entry in fs::read_dir(trace_dir.path()).context("read loader trace directory")? {
        let path = entry?.path();
        let trace =
            fs::read(&path).with_context(|| format!("read loader trace {}", path.display()))?;
        for line in String::from_utf8_lossy(&trace).lines() {
            let Some((_, object)) = line.split_once("calling init: ") else {
                continue;
            };
            let object = Path::new(object.trim());
            if object
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("libopus.so"))
            {
                loaded.insert(
                    fs::canonicalize(object)
                        .with_context(|| format!("resolve loaded {}", object.display()))?,
                );
            }
        }
    }
    Ok((output, loaded))
}

/// Logical bundle groups recorded during catalog construction, keyed by the
/// content-addressed asset file. A file referenced from several groups lands
/// in the "shared" bundle (see `bundle_grouped_audio`).
type AudioAssetGroups = std::collections::BTreeMap<String, std::collections::BTreeSet<String>>;

static AUDIO_ASSET_GROUPS: std::sync::LazyLock<std::sync::Mutex<AudioAssetGroups>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(AudioAssetGroups::new()));

// Converter-only provenance: remember the exact source used for each catalog
// entry, rather than inferring it from a WAV/OGG alias during boot cleanup.
static AUDIO_ASSET_SOURCES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<(PathBuf, String), PathBuf>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(super) fn catalog_source_bytes(assets_dir: &Path, relative: &str) -> Result<Option<Vec<u8>>> {
    let source = AUDIO_ASSET_SOURCES
        .lock()
        .expect("audio source map poisoned")
        .get(&(
            assets_dir.to_owned(),
            standalone_audio_logical_key(relative),
        ))
        .cloned();
    source
        .map(|path| {
            fs::read(&path).with_context(|| format!("read catalog source {}", path.display()))
        })
        .transpose()
}

pub(super) fn insert_shipping_audio(
    payload: &mut ShippingMission,
    catalog: &mut std::collections::BTreeMap<String, ShippingAudioAsset>,
    assets_dir: &Path,
    group: &str,
    relative: &str,
    path: &Path,
    kind: AudioKind,
    format: AudioFormat,
) -> Result<()> {
    let is_audio = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("wav") || extension.eq_ignore_ascii_case("ogg")
        });
    if !is_audio {
        return insert_shipping_raw(payload, relative, path);
    }

    let source = fs::read(path).with_context(|| format!("read audio {}", path.display()))?;
    let duration_ms = robin_rs::audio_backend::wav_duration_ms(&source).with_context(|| {
        format!(
            "cannot derive authoritative audio duration for {}",
            path.display()
        )
    })?;
    match format {
        AudioFormat::Source => {
            let relative = relative.replace('\\', "/").to_ascii_lowercase();
            if let Some(previous) = payload.raw.get(&relative) {
                if previous != &source {
                    bail!("conflicting shipping audio sources for {relative}");
                }
            } else {
                payload.raw.insert(relative.clone(), source);
            }
            insert_audio_duration(&mut payload.audio_durations_ms, relative, duration_ms)
        }
        AudioFormat::Opus => {
            let logical = standalone_audio_logical_key(relative);
            if let Some(existing) = catalog.get(&logical) {
                if existing.duration_ms != duration_ms {
                    bail!(
                        "conflicting source durations for standalone shipping audio {logical}: {} vs {duration_ms}",
                        existing.duration_ms
                    );
                }
            } else {
                // Music encodes from the lossless remaster drop when one
                // exists; the catalog duration above stays derived from the
                // GAME source, so deterministic timing tables are unchanged.
                let encode_source = if matches!(kind, AudioKind::Music) {
                    music_lossless_source(path)?
                } else {
                    None
                };
                if let Some(choice) = &encode_source {
                    let lossless = &choice.path;
                    let lossless_ms = robin_rs::audio_backend::wav_duration_ms(
                        &fs::read(lossless)
                            .with_context(|| format!("read audio {}", lossless.display()))?,
                    )
                    .with_context(|| format!("duration of {}", lossless.display()))?;
                    // The catalog keeps the game duration, so a remaster of a
                    // different length (a wrong mapping, or an edit) would
                    // desynchronize timing: refuse it unless the mapping marks
                    // the pair as an intentional edit.
                    let tolerance_ms = remaster_duration_tolerance_ms(duration_ms);
                    let difference_ms = lossless_ms.abs_diff(duration_ms);
                    if difference_ms > tolerance_ms {
                        anyhow::ensure!(
                            choice.intentional_duration_edit,
                            "lossless remaster {} is {lossless_ms} ms but game music {} is {duration_ms} ms \
                             (tolerance {tolerance_ms} ms); fix the mapping or list the game file in \
                             intentional_duration_edits ({LOSSLESS_MUSIC_MAPPING_PATH})",
                            lossless.display(),
                            path.display()
                        );
                        tracing::info!(
                            game = %path.display(),
                            lossless = %lossless.display(),
                            game_ms = duration_ms,
                            lossless_ms,
                            "encoding intentionally edited remaster of different duration"
                        );
                    }
                }
                let encoded_from = encode_source
                    .as_ref()
                    .map_or(path, |choice| choice.path.as_path());
                let bytes = transcode_audio_to_opus(encoded_from, kind)?;
                tracing::debug!(
                    source = %encoded_from.display(),
                    ?kind,
                    bytes = bytes.len(),
                    "encoded opus asset"
                );
                insert_standalone_audio(catalog, assets_dir, group, relative, &bytes, duration_ms)?;
                AUDIO_ASSET_SOURCES
                    .lock()
                    .expect("audio source map poisoned")
                    .insert((assets_dir.to_owned(), logical.clone()), path.to_owned());
            }
            // Opus bytes live only in the standalone catalog, but each boot
            // or mission payload retains this tiny exact-membership index.
            // Runtime warmup uses it to avoid decoding the whole catalog.
            insert_audio_duration(&mut payload.audio_durations_ms, logical, duration_ms)
        }
    }
}

/// Tracked game-music -> lossless-remaster mapping. The lossless drop
/// (`--lossless-music-dir`) only supplies the remaster WAVs; its old flat
/// `mapping.json` is superseded (it keyed tracks by file name alone and so gave
/// the Demo the wrong menu and castle-fight remasters).
const LOSSLESS_MUSIC_MAPPING: &str = include_str!("lossless_music_mapping.json");
const LOSSLESS_MUSIC_MAPPING_PATH: &str =
    "crates/robin_rs/src/bin/convert_datadir/lossless_music_mapping.json";

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct LosslessMusicMapping {
    schema_version: u32,
    lossless_root: String,
    sources: std::collections::BTreeMap<String, LosslessMusicSource>,
    lossless_only: Vec<String>,
}

/// One game music release (Demo Leicester, full game, ...), selected by the
/// exact content of its Musics directory.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct LosslessMusicSource {
    description: String,
    game_root: String,
    identity: LosslessMusicIdentity,
    game_to_lossless: std::collections::BTreeMap<String, Option<String>>,
    correlation: std::collections::BTreeMap<String, Option<f64>>,
    intentional_duration_edits: Vec<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct LosslessMusicIdentity {
    /// sha256 of every `.wav`/`.ogg` file directly in `game_root`.
    music_sha256: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LosslessMusicChoice {
    pub(super) path: PathBuf,
    pub(super) intentional_duration_edit: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct LosslessMusicSelection {
    section: String,
    /// Lowercase game file name -> remaster (None: encode the game file).
    by_game_file: std::collections::BTreeMap<String, Option<LosslessMusicChoice>>,
}

/// `None` inside the lock: no lossless drop was given, every track encodes
/// from its game file.
static LOSSLESS_MUSIC: std::sync::OnceLock<Option<LosslessMusicSelection>> =
    std::sync::OnceLock::new();

/// A remaster may differ from its game track by max(0.25 s, 3%), capped at 2 s.
pub(super) fn remaster_duration_tolerance_ms(game_ms: u32) -> u32 {
    (game_ms / 100 * 3).clamp(250, 2_000)
}

fn parse_lossless_music_mapping() -> Result<LosslessMusicMapping> {
    let mapping: LosslessMusicMapping = serde_json::from_str(LOSSLESS_MUSIC_MAPPING)
        .with_context(|| format!("parse {LOSSLESS_MUSIC_MAPPING_PATH}"))?;
    anyhow::ensure!(
        mapping.schema_version == 2,
        "{LOSSLESS_MUSIC_MAPPING_PATH} has schema_version {}, expected 2",
        mapping.schema_version
    );
    for (name, source) in &mapping.sources {
        let identity: BTreeSet<_> = source.identity.music_sha256.keys().collect();
        anyhow::ensure!(
            identity == source.game_to_lossless.keys().collect::<BTreeSet<_>>()
                && identity == source.correlation.keys().collect::<BTreeSet<_>>(),
            "{LOSSLESS_MUSIC_MAPPING_PATH}: source {name} must list every identity file in \
             game_to_lossless and correlation"
        );
        for edit in &source.intentional_duration_edits {
            anyhow::ensure!(
                identity.contains(edit),
                "{LOSSLESS_MUSIC_MAPPING_PATH}: source {name} marks unknown file {edit} as an intentional edit"
            );
        }
    }
    Ok(mapping)
}

fn lowercase_identity(
    identity: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    identity
        .iter()
        .map(|(name, sha)| (name.to_ascii_lowercase(), sha.to_ascii_lowercase()))
        .collect()
}

/// sha256 of every `.wav`/`.ogg` file directly inside a game Musics directory.
fn music_directory_identity(
    musics_dir: &Path,
) -> Result<std::collections::BTreeMap<String, String>> {
    use sha2::{Digest as _, Sha256};
    let mut identity = std::collections::BTreeMap::new();
    for entry in fs::read_dir(musics_dir)
        .with_context(|| format!("read music directory {}", musics_dir.display()))?
    {
        let path = entry?.path();
        let is_music = path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("wav") || extension.eq_ignore_ascii_case("ogg")
                });
        if !is_music {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("non-UTF-8 music file name in {}", musics_dir.display()))?
            .to_owned();
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        identity.insert(name, hex::encode(Sha256::digest(bytes)));
    }
    Ok(identity)
}

/// Pick the one mapping source whose identity equals the game's music files.
fn select_lossless_music_source<'a>(
    mapping: &'a LosslessMusicMapping,
    identity: &std::collections::BTreeMap<String, String>,
) -> Result<(&'a String, &'a LosslessMusicSource)> {
    let wanted = lowercase_identity(identity);
    let matches: Vec<_> = mapping
        .sources
        .iter()
        .filter(|(_, source)| lowercase_identity(&source.identity.music_sha256) == wanted)
        .collect();
    match matches.as_slice() {
        [single] => Ok(*single),
        [] => bail!(
            "no source in {LOSSLESS_MUSIC_MAPPING_PATH} matches this datadir's music files; \
             verify the remaster correlations and add a section with identity.music_sha256 = {}",
            serde_json::to_string_pretty(identity)?
        ),
        _ => bail!(
            "several sources in {LOSSLESS_MUSIC_MAPPING_PATH} match this datadir's music files: {:?}",
            matches.iter().map(|(name, _)| name).collect::<Vec<_>>()
        ),
    }
}

/// Select the lossless remasters music encodes from.
///
/// `root` is the lossless drop (`--lossless-music-dir`, WAVs only); the mapping
/// is the tracked [`LOSSLESS_MUSIC_MAPPING`]. Its source section is chosen by
/// the sha256 of every file in the datadir's `musics_dir`, never by file name,
/// and a datadir no section describes is an error. Every source -> remaster
/// decision is logged.
///
/// The drop used to be looked up as `datadirs/music-rhmods-lossless` relative
/// to the current directory, so a conversion started from a worktree (which
/// has no `datadirs/`) silently encoded the game files instead: that is why the
/// v16 and v16r2 Demo generations differ in music bytes. It is now explicit.
pub(super) fn configure_lossless_music(root: Option<&Path>, musics_dir: &Path) -> Result<()> {
    let selection = match root {
        None => {
            tracing::warn!("no --lossless-music-dir given; music encodes from the game sources");
            None
        }
        Some(root) => {
            let mapping = parse_lossless_music_mapping()?;
            if root.join("mapping.json").exists() {
                tracing::info!(
                    ignored = %root.join("mapping.json").display(),
                    authoritative = LOSSLESS_MUSIC_MAPPING_PATH,
                    "ignoring superseded lossless drop mapping.json"
                );
            }
            let identity = music_directory_identity(musics_dir)?;
            let (section, source) = select_lossless_music_source(&mapping, &identity)?;
            tracing::info!(
                section,
                description = source.description,
                music_dir = %musics_dir.display(),
                "selected lossless music mapping source by music content"
            );
            let mut by_game_file = std::collections::BTreeMap::new();
            for (game_name, lossless_name) in &source.game_to_lossless {
                let choice = match lossless_name {
                    None => {
                        tracing::info!(
                            section,
                            game = game_name,
                            "music source has no remaster; encoding the game file"
                        );
                        None
                    }
                    Some(lossless_name) => {
                        // The drop has been seen both with and without the
                        // `lossless_root` subdirectory; accept either layout.
                        let path = [
                            root.join(&mapping.lossless_root).join(lossless_name),
                            root.join(lossless_name),
                        ]
                        .into_iter()
                        .find(|p| p.is_file())
                        .with_context(|| {
                            format!(
                                "lossless drop {} has no {lossless_name} (mapped from {section}/{game_name})",
                                root.display()
                            )
                        })?;
                        let intentional_duration_edit =
                            source.intentional_duration_edits.contains(game_name);
                        tracing::info!(
                            section,
                            game = game_name,
                            remaster = %path.display(),
                            correlation = source.correlation[game_name],
                            intentional_duration_edit,
                            "music source -> lossless remaster"
                        );
                        Some(LosslessMusicChoice {
                            path,
                            intentional_duration_edit,
                        })
                    }
                };
                by_game_file.insert(game_name.to_ascii_lowercase(), choice);
            }
            Some(LosslessMusicSelection {
                section: section.clone(),
                by_game_file,
            })
        }
    };
    if let Some(existing) = LOSSLESS_MUSIC.get() {
        anyhow::ensure!(
            *existing == selection,
            "lossless music sources already configured differently"
        );
    } else {
        let _ = LOSSLESS_MUSIC.set(selection);
    }
    Ok(())
}

/// Resolve the lossless master for a music track from the selected source.
pub(super) fn music_lossless_source(game_path: &Path) -> Result<Option<LosslessMusicChoice>> {
    let selection = LOSSLESS_MUSIC
        .get()
        .context("Opus music encoding requires configure_lossless_music to run first")?;
    let Some(selection) = selection else {
        return Ok(None);
    };
    let name = game_path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("music path has no UTF-8 file name: {}", game_path.display()))?
        .to_ascii_lowercase();
    selection.by_game_file.get(&name).cloned().with_context(|| {
        format!(
            "music file {} is not part of the identified lossless mapping source {}",
            game_path.display(),
            selection.section
        )
    })
}

#[cfg(test)]
mod lossless_music_tests {
    use super::*;

    #[test]
    fn tracked_mapping_is_consistent() {
        let mapping = parse_lossless_music_mapping().unwrap();
        let mapped: BTreeSet<_> = mapping
            .sources
            .values()
            .flat_map(|source| source.game_to_lossless.values().flatten())
            .collect();
        for only in &mapping.lossless_only {
            assert!(
                !mapped.contains(only),
                "{only} is both mapped and lossless_only"
            );
        }
        let identities: BTreeSet<_> = mapping
            .sources
            .values()
            .map(|source| lowercase_identity(&source.identity.music_sha256))
            .collect();
        assert_eq!(identities.len(), mapping.sources.len());
        // Regression: the Demo menu is the Leicester Day piece, and its
        // castle fight is the alternative mix, not the full game's.
        let demo = &mapping.sources["demo_leicester"];
        assert_eq!(
            demo.game_to_lossless["Menu.wav"].as_deref(),
            Some("Leicester_Day.wav")
        );
        assert_eq!(
            demo.game_to_lossless["Cast_Fight.wav"].as_deref(),
            Some("Castles_red - Alternative.wav")
        );
        let full = &mapping.sources["fullgame_linux"];
        assert_eq!(
            full.game_to_lossless["Menu.ogg"].as_deref(),
            Some("Menü-Soundtrack.wav")
        );
    }

    #[test]
    fn source_selection_uses_content_not_names() {
        let mapping = parse_lossless_music_mapping().unwrap();
        let demo = mapping.sources["demo_leicester"]
            .identity
            .music_sha256
            .clone();
        let upper: std::collections::BTreeMap<_, _> = demo
            .iter()
            .map(|(name, sha)| (name.to_ascii_uppercase(), sha.clone()))
            .collect();
        assert_eq!(
            select_lossless_music_source(&mapping, &upper).unwrap().0,
            "demo_leicester"
        );
        // Same names, one different file: no silent fallback to a name match.
        let mut changed = demo.clone();
        changed.insert("Menu.wav".into(), "0".repeat(64));
        let error = select_lossless_music_source(&mapping, &changed).unwrap_err();
        assert!(error.to_string().contains("no source"));
        let mut extra = demo;
        extra.insert("Bonus.wav".into(), "1".repeat(64));
        assert!(select_lossless_music_source(&mapping, &extra).is_err());
    }

    #[test]
    fn remaster_duration_tolerance_is_three_percent_clamped() {
        assert_eq!(remaster_duration_tolerance_ms(7_000), 250);
        assert_eq!(remaster_duration_tolerance_ms(50_000), 1_500);
        assert_eq!(remaster_duration_tolerance_ms(114_000), 2_000);
    }
}

fn reader_matches_bytes(mut input: impl std::io::Read, expected: &[u8]) -> std::io::Result<bool> {
    let mut buffer = [0_u8; 8192];
    for chunk in expected.chunks(buffer.len()) {
        match input.read_exact(&mut buffer[..chunk.len()]) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(error),
        }
        if &buffer[..chunk.len()] != chunk {
            return Ok(false);
        }
    }
    match input.read_exact(&mut buffer[..1]) {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(true),
        Err(error) => Err(error),
    }
}

pub(super) fn insert_standalone_audio(
    catalog: &mut std::collections::BTreeMap<String, ShippingAudioAsset>,
    assets_dir: &Path,
    group: &str,
    relative: &str,
    bytes: &[u8],
    duration_ms: u32,
) -> Result<()> {
    let encoded_size =
        u32::try_from(bytes.len()).context("standalone Opus asset exceeds u32 byte length")?;
    let logical = standalone_audio_logical_key(relative);
    let filename = standalone_audio_filename(bytes);
    let asset = ShippingAudioAsset {
        file: format!("audio/assets/{filename}"),
        encoded_size,
        duration_ms,
        bundle_offset: None,
    };
    let entry = catalog.entry(logical);
    if let std::collections::btree_map::Entry::Occupied(existing) = &entry {
        if existing.get() != &asset {
            bail!(
                "conflicting standalone shipping audio for {}: {:?} vs {asset:?}",
                existing.key(),
                existing.get()
            );
        }
    }
    let output = assets_dir.join(&filename);
    match fs::symlink_metadata(&output) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "existing audio asset is not a regular file: {}",
                output.display()
            );
            let existing = fs::File::open(&output)
                .with_context(|| format!("open existing audio asset {}", output.display()))?;
            if !reader_matches_bytes(existing, bytes)
                .with_context(|| format!("read existing audio asset {}", output.display()))?
            {
                bail!("content-addressed audio collision at {}", output.display());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            publication::publish_bytes(&output, bytes)
                .with_context(|| format!("write audio asset {}", output.display()))?;
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("stat existing audio asset {}", output.display()));
        }
    }
    AUDIO_ASSET_GROUPS
        .lock()
        .expect("audio group recorder poisoned")
        .entry(asset.file.clone())
        .or_default()
        .insert(group.to_owned());
    if let std::collections::btree_map::Entry::Vacant(entry) = entry {
        entry.insert(asset);
    }
    Ok(())
}

pub(super) fn standalone_audio_logical_key(relative: &str) -> String {
    Path::new(&robin_util::asset_fs::bundle_key(Path::new(relative)))
        .with_extension("opus")
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn insert_audio_duration(
    durations: &mut std::collections::BTreeMap<String, u32>,
    relative: String,
    duration_ms: u32,
) -> Result<()> {
    match durations.entry(relative) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(duration_ms);
        }
        std::collections::btree_map::Entry::Occupied(entry) if *entry.get() == duration_ms => {}
        std::collections::btree_map::Entry::Occupied(entry) => {
            bail!(
                "conflicting source durations for shipping audio {}: {} vs {duration_ms}",
                entry.key(),
                entry.get()
            );
        }
    }
    Ok(())
}

/// Assets larger than this stay standalone files (music, long ambience):
/// they are few, individually worth an HTTP request, and bundling them
/// would force multi-MB downloads for one sound.
const AUDIO_BUNDLE_MAX_MEMBER: u32 = 262_144;

fn append_bundle_member(
    input: impl std::io::Read,
    expected: u32,
    bundle: &mut Vec<u8>,
) -> Result<()> {
    use std::io::Read as _;
    let start = bundle.len();
    let result = input.take(u64::from(expected) + 1).read_to_end(bundle);
    match result {
        Ok(read) if read == expected as usize => Ok(()),
        Ok(read) => {
            bundle.truncate(start);
            bail!("bundle member read {read} bytes but was cataloged as {expected}");
        }
        Err(error) => {
            bundle.truncate(start);
            Err(error).context("read bundle member")
        }
    }
}

/// Concatenate small catalog assets into one file per logical group
/// (recorded in [`AUDIO_ASSET_GROUPS`] during catalog construction; a file
/// referenced by several groups moves to the "shared" bundle). Rewrites the
/// catalog entries to (bundle file, offset) and deletes the standalone
/// files, so the browser fetches one request per group instead of ~2,000
/// tiny ones. Deterministic: members concatenate in content-hash order and
/// the bundle name is content-addressed.
pub(super) fn bundle_grouped_audio(
    dd: &mut robin_assets::shipping_datadir::ShippingDatadir,
    data_out: &Path,
) -> Result<()> {
    let groups_by_file = std::mem::take(
        &mut *AUDIO_ASSET_GROUPS
            .lock()
            .expect("audio group recorder poisoned"),
    );
    bundle_recorded_audio(dd, data_out, groups_by_file)
}

fn bundle_recorded_audio(
    dd: &mut robin_assets::shipping_datadir::ShippingDatadir,
    data_out: &Path,
    groups_by_file: AudioAssetGroups,
) -> Result<()> {
    use sha2::{Digest as _, Sha256};
    use std::collections::BTreeMap;
    // file -> (logical keys referencing it, encoded size)
    let mut file_refs = BTreeMap::<String, (Vec<String>, u32)>::new();
    for (logical, asset) in &dd.audio_assets {
        if asset.bundle_offset.is_some() {
            bail!("audio asset {logical} is already bundled; bundling must run once");
        }
        let entry = file_refs
            .entry(asset.file.clone())
            .or_insert_with(|| (Vec::new(), asset.encoded_size));
        if entry.1 != asset.encoded_size {
            bail!("conflicting encoded sizes recorded for {}", asset.file);
        }
        entry.0.push(logical.clone());
    }
    let mut members_by_group = BTreeMap::<&str, Vec<&str>>::new();
    for (file, (_, size)) in &file_refs {
        if *size >= AUDIO_BUNDLE_MAX_MEMBER {
            continue;
        }
        let groups = groups_by_file
            .get(file)
            .with_context(|| format!("catalog file {file} was never recorded in a bundle group"))?;
        let group = match groups.len() {
            0 => bail!("catalog file {file} was never recorded in a bundle group"),
            1 => groups.first().expect("len checked").as_str(),
            _ => "shared",
        };
        members_by_group
            .entry(group)
            .or_default()
            .push(file.as_str());
    }
    if members_by_group.is_empty() {
        return Ok(());
    }
    let bundles_dir = data_out.join("audio/bundles");
    fs::create_dir_all(&bundles_dir)?;
    let (mut bundled_files, mut bundle_count, mut bundled_bytes) = (0usize, 0usize, 0u64);
    let mut prepared = Vec::with_capacity(members_by_group.len());
    for (group, members) in members_by_group {
        // BTreeMap iteration already sorted members by content-hash name.
        let mut bytes = Vec::new();
        let mut offsets = Vec::with_capacity(members.len());
        for &file in &members {
            let path = data_out.join(file);
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("stat bundle member {file}"))?;
            anyhow::ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "bundle member is not a regular file: {file}"
            );
            let input =
                fs::File::open(&path).with_context(|| format!("open bundle member {file}"))?;
            offsets.push(u32::try_from(bytes.len()).context("audio bundle exceeds u32")?);
            append_bundle_member(input, file_refs[file].1, &mut bytes)
                .with_context(|| format!("bundle member {file}"))?;
        }
        let digest = Sha256::digest(&bytes);
        let hash = hex::encode(&digest[..6]);
        let bundle_rel = format!("audio/bundles/{}-{hash}.bin", shipping_file_stem(group));
        publication::publish_bytes(&data_out.join(&bundle_rel), &bytes)
            .with_context(|| format!("write {bundle_rel}"))?;
        bundle_count += 1;
        bundled_bytes += bytes.len() as u64;
        prepared.push((bundle_rel, members, offsets));
    }
    // All inputs and bundle outputs must succeed before changing catalog
    // references or deleting any standalone source. Retain only metadata
    // between phases, not every bundle's encoded bytes.
    for (bundle_rel, members, offsets) in prepared {
        for (&file, offset) in members.iter().zip(offsets) {
            for logical in &file_refs[file].0 {
                let asset = dd
                    .audio_assets
                    .get_mut(logical)
                    .expect("logical key came from the catalog");
                asset.file = bundle_rel.clone();
                asset.bundle_offset = Some(offset);
            }
            // TODO: coordinate cleanup with final boot-index/manifest
            // publication; per-file atomic writes are not a directory transaction.
            fs::remove_file(data_out.join(file))
                .with_context(|| format!("remove bundled standalone {file}"))?;
            bundled_files += 1;
        }
    }
    tracing::info!(
        bundles = bundle_count,
        bundled_files,
        bundled_bytes,
        standalone_left = file_refs.len() - bundled_files,
        "grouped small audio assets into logical bundles"
    );
    Ok(())
}

pub(super) fn standalone_audio_filename(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(bytes);
    let hash = hex::encode(digest);
    format!("{hash}.opus")
}

/// Decode the source to PCM with FFmpeg (the Demo's `.wav` files are really
/// Ogg Vorbis), encode it with the verified opusenc (see
/// [`configure_opus_toolchain`]), then remux the packets with a fixed Ogg
/// stream serial and vendor packet. opusenc randomizes Ogg serials, which would
/// otherwise make content-addressed shipping chunks and `--resume`
/// nondeterministic even though the encoded Opus packets are identical.
///
/// opusenc runs with only `--bitrate`, `--vbr`, `--comp 10` and
/// `--framesize 20`: no `--speech`/`--music`, so the signal type is
/// `OPUS_AUTO`. libopusenc always creates the libopus encoder at 48 kHz with
/// `OPUS_APPLICATION_AUDIO` and resamples other input rates with its speex
/// resampler; bandwidth and SILK/CELT/hybrid selection stay automatic.
pub(super) fn transcode_audio_to_opus(source_path: &Path, kind: AudioKind) -> Result<Vec<u8>> {
    use std::io::Cursor;

    let toolchain = configured_opus_toolchain()?;
    let scratch = tempfile::tempdir().context("create PCM scratch directory")?;
    let pcm = scratch.path().join("source.wav");
    // 32-bit float keeps the decode exact for the 16-bit PCM sources and
    // avoids a requantization of the Vorbis ones before opusenc's resampler.
    let decoded = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(source_path)
        .args([
            "-map_metadata",
            "-1",
            "-vn",
            "-fflags",
            "+bitexact",
            "-c:a",
            "pcm_f32le",
            "-f",
            "wav",
        ])
        .arg(&pcm)
        .output()
        .context("run ffmpeg to decode audio (is ffmpeg installed?)")?;
    if !decoded.status.success() {
        bail!(
            "ffmpeg decode failed for {} ({}): {}",
            source_path.display(),
            decoded.status,
            String::from_utf8_lossy(&decoded.stderr).trim()
        );
    }
    let bitrate = kind.bitrate_kbps().to_string();
    let output = run_verified_opusenc(
        toolchain,
        Command::new(&toolchain.opusenc)
            .args([
                "--quiet",
                "--bitrate",
                &bitrate,
                "--vbr",
                "--comp",
                "10",
                "--framesize",
                "20",
                "--discard-comments",
            ])
            .arg(&pcm)
            .arg("-"),
    )?;
    if !output.status.success() {
        bail!(
            "opusenc encode failed for {} ({}): {}",
            source_path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut reader = ogg::PacketReader::new(Cursor::new(output.stdout));
    let mut packets = Vec::new();
    while let Some(packet) = reader
        .read_packet()
        .with_context(|| format!("parse opusenc Ogg output for {}", source_path.display()))?
    {
        packets.push(packet);
    }
    if packets
        .first()
        .is_none_or(|packet| !packet.data.starts_with(b"OpusHead"))
    {
        bail!(
            "opusenc produced non-Opus Ogg output for {}",
            source_path.display()
        );
    }
    if packets.len() < 3 {
        bail!(
            "opusenc produced incomplete Opus stream for {}",
            source_path.display()
        );
    }
    packets[1].data = deterministic_opus_tags();

    let mut remuxed = Cursor::new(Vec::new());
    {
        use ogg::writing::{PacketWriteEndInfo, PacketWriter};
        let mut writer = PacketWriter::new(&mut remuxed);
        for packet in packets {
            let absgp = packet.absgp_page();
            let end = if packet.last_in_stream() {
                PacketWriteEndInfo::EndStream
            } else if packet.last_in_page() {
                PacketWriteEndInfo::EndPage
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            writer
                .write_packet(packet.data, 0x5248_4f50, end, absgp)
                .with_context(|| {
                    format!("write deterministic Ogg for {}", source_path.display())
                })?;
        }
    }
    Ok(remuxed.into_inner())
}

pub(super) fn deterministic_opus_tags() -> Vec<u8> {
    const VENDOR: &[u8] = b"robinhood-web-shipping";
    let mut tags = Vec::with_capacity(16 + VENDOR.len());
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    tags.extend_from_slice(VENDOR);
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags
}

pub(super) fn write_shipping_dependency(
    output_dir: &Path,
    label: &str,
    payload: &ShippingMission,
    window_log: u32,
    resume: bool,
) -> Result<Option<String>> {
    // Opus payloads deliberately keep their bytes in the browser-owned
    // catalog, so their exact boot/mission membership consists solely of
    // duration keys. Treat that metadata as real dependency content.
    if payload.raw.is_empty() && payload.audio_durations_ms.is_empty() {
        return Ok(None);
    }
    let (filename, compressed) =
        prepare_shipping_payload(output_dir, label, payload, window_log, resume)?;
    let compressed_len = write_prepared_shipping_payload(output_dir, &filename, compressed)?;
    tracing::info!(
        label,
        files = payload.raw.len(),
        audio_members = payload.audio_durations_ms.len(),
        bytes = compressed_len,
        "wrote shipping audio dependency"
    );
    Ok(Some(format!("audio/{filename}")))
}

/// Tests that really encode need the pinned opus-tools: `ROBIN_OPUS_TOOLS_DIR`
/// (the same variable `scripts/build_web_shipping_datadir.sh` reads).
#[cfg(test)]
pub(super) fn configure_test_opus_toolchain() {
    let dir = std::env::var_os("ROBIN_OPUS_TOOLS_DIR").unwrap_or_else(|| {
        panic!(
            "set ROBIN_OPUS_TOOLS_DIR to the opus-tools prefix whose bin/opusenc uses {REQUIRED_LIBOPUS_VERSION}"
        )
    });
    configure_opus_toolchain(Path::new(&dir)).unwrap();
}

#[cfg(test)]
mod boot_trim_tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct FailedRead;
    impl std::io::Read for FailedRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected read failure"))
        }
    }

    #[test]
    fn streamed_audio_comparison_propagates_read_errors() {
        use std::io::Read as _;
        assert!(reader_matches_bytes(FailedRead, &[]).is_err());
        assert!(reader_matches_bytes((&b"a"[..]).chain(FailedRead), b"ab").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn standalone_audio_reuse_refuses_symlink_targets() {
        let directory = tempfile::tempdir().unwrap();
        let target = tempfile::NamedTempFile::new().unwrap();
        let bytes = b"symlink audio fixture";
        fs::write(target.path(), bytes).unwrap();
        let path = directory.path().join(standalone_audio_filename(bytes));
        std::os::unix::fs::symlink(target.path(), &path).unwrap();
        let mut catalog = std::collections::BTreeMap::new();
        let error = insert_standalone_audio(
            &mut catalog,
            directory.path(),
            "test",
            "test.wav",
            bytes,
            100,
        )
        .unwrap_err();
        assert!(error.to_string().contains("not a regular file"));
        assert!(catalog.is_empty());
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(target.path()).unwrap(), bytes);
    }

    #[test]
    fn streamed_audio_comparison_checks_content_length_and_trailing_data() {
        for length in [0, 1, 8191, 8192, 8193, 200_000] {
            let bytes: Vec<_> = (0..length).map(|i| (i % 251) as u8).collect();
            assert!(reader_matches_bytes(bytes.as_slice(), &bytes).unwrap());
            let mut longer = bytes.clone();
            longer.push(0);
            assert!(!reader_matches_bytes(longer.as_slice(), &bytes).unwrap());
            assert!(!reader_matches_bytes(bytes.as_slice(), &longer).unwrap());
            if length != 0 {
                let mut different = bytes.clone();
                different[length / 2] ^= 1;
                assert!(!reader_matches_bytes(different.as_slice(), &bytes).unwrap());
            }
        }
        assert!(!reader_matches_bytes(std::io::repeat(0), &[0; 8193]).unwrap());
    }

    #[test]
    fn conflicting_catalog_insert_has_no_file_or_group_side_effects() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalog = std::collections::BTreeMap::new();
        let original_bytes = b"catalog conflict regression original";
        insert_standalone_audio(
            &mut catalog,
            directory.path(),
            "original",
            "effect.wav",
            original_bytes,
            100,
        )
        .unwrap();
        let before = catalog.clone();
        let rejected_bytes = b"catalog conflict regression rejected";
        assert!(
            insert_standalone_audio(
                &mut catalog,
                directory.path(),
                "rejected-content",
                "effect.wav",
                rejected_bytes,
                100
            )
            .is_err()
        );
        assert!(
            !directory
                .path()
                .join(standalone_audio_filename(rejected_bytes))
                .exists()
        );
        assert!(
            insert_standalone_audio(
                &mut catalog,
                directory.path(),
                "rejected-duration",
                "effect.wav",
                original_bytes,
                200
            )
            .is_err()
        );
        assert_eq!(catalog, before);
        let groups = AUDIO_ASSET_GROUPS.lock().unwrap();
        assert!(!groups.contains_key(&format!(
            "audio/assets/{}",
            standalone_audio_filename(rejected_bytes)
        )));
        assert!(
            !groups[&format!("audio/assets/{}", standalone_audio_filename(original_bytes))]
                .contains("rejected-duration")
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn empty_group_records_do_not_bypass_catalog_validation() {
        use robin_assets::shipping_datadir::ShippingAudioAsset;
        let directory = tempfile::tempdir().unwrap();
        let mut dd = robin_assets::shipping_datadir::ShippingDatadir::default();
        bundle_recorded_audio(&mut dd, directory.path(), AudioAssetGroups::new()).unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        dd.audio_assets.insert(
            "effect".into(),
            ShippingAudioAsset {
                file: "audio/assets/effect.opus".into(),
                encoded_size: 1,
                duration_ms: 100,
                bundle_offset: None,
            },
        );
        let original = dd.audio_assets.clone();
        let error =
            bundle_recorded_audio(&mut dd, directory.path(), AudioAssetGroups::new()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("never recorded in a bundle group")
        );
        assert_eq!(dd.audio_assets, original);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);

        // Large standalone entries do not need bundle groups, but still
        // participate in the once-only catalog validation.
        dd.audio_assets.get_mut("effect").unwrap().encoded_size = AUDIO_BUNDLE_MAX_MEMBER;
        bundle_recorded_audio(&mut dd, directory.path(), AudioAssetGroups::new()).unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        dd.audio_assets.get_mut("effect").unwrap().bundle_offset = Some(0);
        assert!(
            bundle_recorded_audio(&mut dd, directory.path(), AudioAssetGroups::new())
                .unwrap_err()
                .to_string()
                .contains("already bundled")
        );
    }

    #[test]
    fn bundle_members_must_be_regular_files() {
        let directory = tempfile::tempdir().unwrap();
        let file = "audio/assets/effect.opus";
        let path = directory.path().join(file);
        fs::create_dir_all(&path).unwrap();
        let mut dd = robin_assets::shipping_datadir::ShippingDatadir::default();
        dd.audio_assets.insert(
            "effect".into(),
            ShippingAudioAsset {
                file: file.into(),
                encoded_size: 1,
                duration_ms: 123,
                bundle_offset: None,
            },
        );
        let original = dd.audio_assets.clone();
        let groups = AudioAssetGroups::from([(file.into(), BTreeSet::from(["effects".into()]))]);
        let error = bundle_recorded_audio(&mut dd, directory.path(), groups.clone()).unwrap_err();
        assert!(error.to_string().contains("not a regular file"));
        assert_eq!(dd.audio_assets, original);
        assert!(path.is_dir());

        #[cfg(unix)]
        {
            fs::remove_dir(&path).unwrap();
            let target = directory.path().join("original.opus");
            fs::write(&target, b"x").unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap();
            let error = bundle_recorded_audio(&mut dd, directory.path(), groups).unwrap_err();
            assert!(error.to_string().contains("not a regular file"));
            assert_eq!(dd.audio_assets, original);
            assert!(
                fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(fs::read(&target).unwrap(), b"x");
        }
        assert_eq!(
            fs::read_dir(directory.path().join("audio/bundles"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn later_bundle_input_failure_keeps_original_catalog_and_sources() {
        use robin_assets::shipping_datadir::ShippingAudioAsset;
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("audio/assets")).unwrap();
        let mut dd = robin_assets::shipping_datadir::ShippingDatadir::default();
        let mut groups = AudioAssetGroups::new();
        for (name, bytes) in [("a", &b"a"[..]), ("z", &b"too long"[..])] {
            let file = format!("audio/assets/{name}.opus");
            fs::write(directory.path().join(&file), bytes).unwrap();
            dd.audio_assets.insert(
                name.to_owned(),
                ShippingAudioAsset {
                    file: file.clone(),
                    encoded_size: 1,
                    duration_ms: 123,
                    bundle_offset: None,
                },
            );
            groups.insert(file, [name.to_owned()].into());
        }
        let original = dd.audio_assets.clone();
        let error = bundle_recorded_audio(&mut dd, directory.path(), groups).unwrap_err();
        assert!(format!("{error:#}").contains("cataloged as 1"));
        assert_eq!(dd.audio_assets, original);
        assert_eq!(
            fs::read(directory.path().join("audio/assets/a.opus")).unwrap(),
            b"a"
        );
        assert_eq!(
            fs::read(directory.path().join("audio/assets/z.opus")).unwrap(),
            b"too long"
        );
        // An already completed output is harmless and can be overwritten on
        // retry; the old catalog still references its intact standalone files.
        assert_eq!(
            fs::read_dir(directory.path().join("audio/bundles"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn recorded_audio_bundles_preserve_bytes_aliases_and_large_files() {
        use robin_assets::shipping_datadir::ShippingAudioAsset;
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("audio/assets")).unwrap();
        let inputs = std::collections::BTreeMap::from([
            ("a", vec![1]),
            ("b", vec![2, 3]),
            ("shared", vec![4, 5, 6]),
            ("large", vec![7; AUDIO_BUNDLE_MAX_MEMBER as usize]),
        ]);
        let mut dd = robin_assets::shipping_datadir::ShippingDatadir::default();
        let mut groups = AudioAssetGroups::new();
        for (name, bytes) in &inputs {
            let file = format!("audio/assets/{name}.opus");
            fs::write(directory.path().join(&file), bytes).unwrap();
            dd.audio_assets.insert(
                (*name).to_owned(),
                ShippingAudioAsset {
                    file: file.clone(),
                    encoded_size: bytes.len() as u32,
                    duration_ms: 123,
                    bundle_offset: None,
                },
            );
            groups.insert(
                file,
                if *name == "shared" {
                    ["voice", "effects"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                } else {
                    ["voice"].into_iter().map(str::to_owned).collect()
                },
            );
        }
        let alias = dd.audio_assets["a"].clone();
        dd.audio_assets.insert("alias".into(), alias);
        fs::write(directory.path().join("notes.txt"), b"keep").unwrap();
        bundle_recorded_audio(&mut dd, directory.path(), groups).unwrap();
        for (name, expected) in &inputs {
            let asset = &dd.audio_assets[*name];
            let bytes = fs::read(directory.path().join(&asset.file)).unwrap();
            let start = asset.bundle_offset.unwrap_or(0) as usize;
            assert_eq!(&bytes[start..start + asset.encoded_size as usize], expected);
            assert_eq!(asset.duration_ms, 123);
            assert_eq!(
                directory
                    .path()
                    .join(format!("audio/assets/{name}.opus"))
                    .exists(),
                *name == "large"
            );
        }
        assert_eq!(dd.audio_assets["a"], dd.audio_assets["alias"]);
        assert_eq!(dd.audio_assets["a"].file, dd.audio_assets["b"].file);
        assert_eq!(dd.audio_assets["a"].bundle_offset, Some(0));
        assert_eq!(dd.audio_assets["b"].bundle_offset, Some(1));
        assert!(
            dd.audio_assets["shared"]
                .file
                .starts_with("audio/bundles/shared-")
        );
        assert_eq!(dd.audio_assets["large"].bundle_offset, None);
        assert_eq!(
            fs::read_dir(directory.path().join("audio/bundles"))
                .unwrap()
                .count(),
            2
        );
        assert_eq!(
            fs::read(directory.path().join("notes.txt")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn bundle_member_append_is_bounded_and_preserves_prefix_on_failure() {
        let mut bundle = b"prefix".to_vec();
        append_bundle_member(&b"abc"[..], 3, &mut bundle).unwrap();
        assert_eq!(bundle, b"prefixabc");
        append_bundle_member(&b""[..], 0, &mut bundle).unwrap();
        assert!(append_bundle_member(&b"ab"[..], 3, &mut bundle).is_err());
        assert_eq!(bundle, b"prefixabc");
        // An unbounded source must fail after expected + 1 bytes, not
        // attempt to read the whole stream.
        assert!(append_bundle_member(std::io::repeat(0), 3, &mut bundle).is_err());
        assert_eq!(bundle, b"prefixabc");
        assert!(append_bundle_member(&b"x"[..], 0, &mut bundle).is_err());
        assert_eq!(bundle, b"prefixabc");
    }

    #[test]
    fn bundle_member_append_rolls_back_partial_io_failure() {
        use std::io::Read as _;
        let mut bundle = b"prefix".to_vec();
        let error =
            append_bundle_member((&b"abc"[..]).chain(FailedRead), 4, &mut bundle).unwrap_err();
        assert!(format!("{error:#}").contains("injected read failure"));
        assert_eq!(bundle, b"prefix");
    }

    #[test]
    fn standalone_audio_name_retains_its_full_lowercase_digest() {
        assert_eq!(
            standalone_audio_filename(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad.opus"
        );
    }

    #[test]
    #[ignore = "requires ffmpeg and ROBIN_OPUS_TOOLS_DIR (opusenc on libopus 1.6.1)"]
    fn boot_trim_uses_actual_catalog_source_when_aliases_collide() {
        configure_test_opus_toolchain();
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&1636u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&1600u32.to_le_bytes());
        wav.resize(1644, 0);
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.wav");
        let second = temp.path().join("second.wav");
        fs::write(&first, &wav).unwrap();
        let mut translated = wav.clone();
        translated[44] = 17;
        fs::write(&second, &translated).unwrap();
        let assets = temp.path().join("audio/assets");
        fs::create_dir_all(&assets).unwrap();
        let mut datadir = ShippingDatadir::default();
        for path in [&first, &second] {
            insert_shipping_audio(
                &mut ShippingMission::default(),
                &mut datadir.audio_assets,
                &assets,
                "test",
                "Sounds/Voice.wav",
                path,
                AudioKind::Voice,
                AudioFormat::Opus,
            )
            .unwrap();
        }
        for (name, bytes) in [("en-US", wav.clone()), ("de-DE", translated.clone())] {
            datadir.locales.insert(
                name.into(),
                ShippingLocale {
                    raw: [("sounds/voice.wav".into(), bytes)].into_iter().collect(),
                    ..Default::default()
                },
            );
        }
        assert_eq!(
            catalog_source_bytes(&assets, "sounds/voice.wav")
                .unwrap()
                .unwrap(),
            wav
        );
        let report =
            robin_assets::shipping_boot_trim::trim_browser_locale_audio(&mut datadir, |key| {
                catalog_source_bytes(&assets, key)
            })
            .unwrap();
        assert_eq!(report.removed_files, 1);
        assert_eq!(datadir.locales["de-DE"].raw["sounds/voice.wav"], translated);
        assert!(datadir.locales["en-US"].raw.is_empty());
    }
}

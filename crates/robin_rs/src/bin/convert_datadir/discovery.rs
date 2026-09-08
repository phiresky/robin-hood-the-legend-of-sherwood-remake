//! Source datadir discovery and locale/edition resolution.
use super::*;

/// Locate the game data directory, retaining original release casing rules.
pub(super) fn find_data_dir(input: &Path) -> Result<PathBuf> {
    for name in ["Data", "DATA", "data"] {
        let p = input.join(name);
        if p.is_dir() {
            return Ok(p);
        }
    }
    bail!(
        "no Data/ directory found inside {} (expected Data/, DATA/, or data/)",
        input.display()
    )
}

/// Detect the two official source layouts from their canonical mission roots.
///
/// Detection validates the operator's typed `--web-content-edition`; it does
/// not replace that attestation. Mixed trees fail instead of inheriting an
/// edition from probe order, and unknown/custom trees cannot be published as
/// official browser content by accident.
pub(super) fn detect_official_web_content_edition(
    data_in: &Path,
) -> Result<robin_rs::multiplayer::content_identity::WebContentEdition> {
    use robin_rs::multiplayer::content_identity::WebContentEdition;

    const DEMO_MISSION_MARKERS: &[&str] = &["Levels/Dem_Lei_MP.rhm", "Levels/Demo_Lin.rhm"];
    const FULL_MISSION_MARKERS: &[&str] = &["Levels/Sherwood.rhm"];

    let demo_markers = DEMO_MISSION_MARKERS
        .iter()
        .copied()
        .filter(|relative| resolve_data_file(data_in, relative).is_some())
        .collect::<Vec<_>>();
    let full_markers = FULL_MISSION_MARKERS
        .iter()
        .copied()
        .filter(|relative| resolve_data_file(data_in, relative).is_some())
        .collect::<Vec<_>>();

    match (demo_markers.is_empty(), full_markers.is_empty()) {
        (false, true) => Ok(WebContentEdition::Demo),
        (true, false) => Ok(WebContentEdition::Full),
        (false, false) => bail!(
            "official web content edition is ambiguous: Demo markers [{}] and Full markers [{}] coexist under {}",
            demo_markers.join(", "),
            full_markers.join(", "),
            data_in.display()
        ),
        (true, true) => bail!(
            "official web content edition is unrecognized under {}: expected one of [{}] for Demo or [{}] for Full",
            data_in.display(),
            DEMO_MISSION_MARKERS.join(", "),
            FULL_MISSION_MARKERS.join(", ")
        ),
    }
}

pub(super) fn validate_web_content_edition(
    data_in: &Path,
    declared: robin_rs::multiplayer::content_identity::WebContentEdition,
) -> Result<robin_rs::multiplayer::content_identity::WebContentEdition> {
    use robin_rs::multiplayer::content_identity::WebContentEdition;

    let detected = detect_official_web_content_edition(data_in)?;
    if declared != detected {
        let label = |edition| match edition {
            WebContentEdition::Demo => "demo",
            WebContentEdition::Full => "full",
        };
        bail!(
            "--web-content-edition {} does not match detected official {} source under {}",
            label(declared),
            label(detected),
            data_in.display()
        );
    }
    Ok(declared)
}

/// Windows LCID → BCP-47 / ISO locale string.  Used to rename the
/// localized subfolders in the hackable output so they're readable
/// (`1033` → `en-US`).  Unknown LCIDs fall through to the numeric name.
pub(super) fn lcid_to_iso(lcid: &str) -> &'static str {
    match lcid {
        "1028" => "zh-TW",
        "1029" => "cs-CZ",
        "1031" => "de-DE",
        "1033" => "en-US",
        "1036" => "fr-FR",
        "1040" => "it-IT",
        "1041" => "ja-JP",
        "1042" => "ko-KR",
        "1045" => "pl-PL",
        "1046" => "pt-BR",
        "1049" => "ru-RU",
        "1054" => "th-TH",
        "2047" => "und",
        "2052" => "zh-CN",
        "2070" => "pt-PT",
        "3082" => "es-ES",
        // Unknown — keep numeric so the conversion is never lossy.
        _ => Box::leak(lcid.to_string().into_boxed_str()),
    }
}

/// Resolve `<root>/<lcid>/Data` (case-insensitive on both components) to a
/// real directory if it exists.  Returns `None` otherwise.
pub(super) fn resolve_locale_data_dir(root: &Path, lcid: &str) -> Option<PathBuf> {
    let lcid_dir = resolve_case_insensitive(&root.join(lcid))?;
    if !lcid_dir.is_dir() {
        return None;
    }
    for name in ["Data", "DATA", "data"] {
        let p = lcid_dir.join(name);
        if p.is_dir() {
            return Some(p);
        }
        if let Some(resolved) = resolve_case_insensitive(&p)
            && resolved.is_dir()
        {
            return Some(resolved);
        }
    }
    None
}

pub(super) fn resolve_data_file(data_dir: &Path, relative: &str) -> Option<PathBuf> {
    let candidate = data_dir.join(relative);
    if candidate.is_file() {
        return Some(candidate);
    }
    resolve_case_insensitive(&candidate).filter(|path| path.is_file())
}

/// A locale alternate source dir + the ISO name used for its output subtree.
#[derive(Debug, Clone)]
pub(super) struct LocaleSource {
    pub(super) data_dir: PathBuf,
    pub(super) lcid: &'static str,
    pub(super) iso: &'static str,
}

/// Detect every locale data dir alongside `data_in`. English remains first to
/// preserve the legacy default-resolution order, but shipping conversion must
/// not collapse later installed packs into that first language.
pub(super) fn detect_locale_data_dirs(data_in: &Path) -> Vec<LocaleSource> {
    let Some(root) = data_in.parent() else {
        return Vec::new();
    };
    let mut sources = Vec::new();
    if let Some(d) = resolve_locale_data_dir(root, FALLBACK_LOCALE_FOLDER) {
        sources.push(LocaleSource {
            data_dir: d,
            lcid: FALLBACK_LOCALE_FOLDER,
            iso: lcid_to_iso(FALLBACK_LOCALE_FOLDER),
        });
    }
    for &folder in LANGUAGE_FOLDERS {
        if let Some(d) = resolve_locale_data_dir(root, folder) {
            sources.push(LocaleSource {
                data_dir: d,
                lcid: folder,
                iso: lcid_to_iso(folder),
            });
        }
    }
    sources
}

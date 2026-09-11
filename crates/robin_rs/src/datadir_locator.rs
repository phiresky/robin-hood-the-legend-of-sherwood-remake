//! Locate a game installation at startup when no datadir was given.
//!
//! Used by `setup_data_dir` when `ROBINHOOD_DATA_DIR` is unset: reuse the
//! remembered choice if there is one, otherwise auto-detect the usual
//! install locations of the CD, GOG, and Steam releases and confirm the
//! result with the player through native OS dialogs (with a folder picker
//! for manual selection). The confirmed choice is remembered next to the
//! saves and can be changed later from the Options menu.
//!
//! A directory counts as a game installation if and only if it contains
//! `Data/robinhood.bks` in any capitalization, or the converted replacement
//! `Data/datadir.bin`. A marker must be a file, not merely a directory with that name.

use std::path::{Path, PathBuf};

/// Keep an explicit user cancellation distinct from unavailable game data.
#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DataDirResolution {
    Selected(PathBuf),
    Unavailable,
    Cancelled,
}

impl DataDirResolution {
    fn detected(candidate: Option<PathBuf>) -> Self {
        candidate.map_or(Self::Unavailable, Self::Selected)
    }

    #[cfg(any(feature = "dialogs", test))]
    fn picked(chosen: Option<PathBuf>) -> Self {
        chosen.map_or(Self::Cancelled, Self::Selected)
    }
}

/// Where to buy the game; shown in the picker dialog.
pub const GOG_STORE_URL: &str = "https://www.gog.com/game/robin_hood_the_legend_of_sherwood";

/// Marker file identifying a correct datadir, looked up case-insensitively
/// inside the installation's `Data/` folder.
const MARKER_FILE: &str = "robinhood.bks";
const DATA_MARKERS: &[&str] = &[MARKER_FILE, "datadir.bin"];

/// Install folder names used by the known Windows distributions. These are
/// joined onto every searched root, so each spelling only needs to be
/// listed once.
const INSTALL_FOLDER_NAMES: &[&str] = &[
    // GOG offline installer / Galaxy, and the English CD (`%APPTITLE%` in
    // the Wise script, installed under `Wanadoo Edition\`).
    "Robin Hood - The Legend of Sherwood",
    // Steam `installdir`.
    "Robin Hood The Legend of Sherwood",
    // Localized CD `%APPTITLE%` values from the Wise installer script.
    "Robin Hood - La Légende de Sherwood",
    "Robin Hood - Die Legende von Sherwood",
    "Robin Hood - La Leggenda di Sherwood",
    "Robin de los Bosques - La Leyenda de Sherwood",
    // Shorthand some repacks/manual installs use.
    "Robin Hood",
];

fn directory_entries(dir: &Path) -> impl Iterator<Item = std::fs::DirEntry> + '_ {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!(path = %dir.display(), "Could not inspect game installation directory: {error}");
            None
        }
    };
    entries.into_iter().flatten().filter_map(move |entry| match entry {
        Ok(entry) => Some(entry),
        Err(error) => {
            tracing::warn!(path = %dir.display(), "Could not inspect game installation entry: {error}");
            None
        }
    })
}

/// Case-insensitive single-component lookup: the entry of `dir` whose name
/// matches `name` ignoring ASCII case.
fn entry_case_insensitive(dir: &Path, name: &str) -> Option<PathBuf> {
    directory_entries(dir).find_map(|entry| {
        entry
            .file_name()
            .to_str()?
            .eq_ignore_ascii_case(name)
            .then(|| entry.path())
    })
}

/// True when `dir` is a game installation root containing
/// `Data/robinhood.bks` (any capitalization of either component), or a
/// pre-converted shipping bundle (`Data/datadir.bin`), which replaces the
/// loose files and therefore has no `.bks`.
pub fn is_valid_install_dir(dir: &Path) -> bool {
    entry_case_insensitive(dir, "Data")
        .map(|data| {
            DATA_MARKERS.iter().any(|marker| {
                let Some(path) = entry_case_insensitive(&data, marker) else { return false; };
                match path.metadata() {
                    Ok(metadata) => metadata.is_file(),
                    Err(error) => {
                        tracing::warn!(path = %path.display(), "Could not inspect game data marker: {error}");
                        false
                    }
                }
            })
        })
        .unwrap_or(false)
}

/// Roots under which [`INSTALL_FOLDER_NAMES`] are searched, plus the store
/// layouts (`GOG Games/`, Galaxy's and Steam's library folders).
fn search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut push = |root: PathBuf| {
        if root.is_dir() && !roots.contains(&root) {
            roots.push(root);
        }
    };

    #[cfg(target_os = "windows")]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(dir) = std::env::var_os(var).map(PathBuf::from) {
                push(dir.join("GOG Galaxy").join("Games"));
                // Older GOG offline installers defaulted here.
                push(dir.join("GOG.com"));
                push(dir.join("Steam").join("steamapps").join("common"));
                // Original CD installer default (Wise `%MAINDIR%`).
                push(dir.join("Wanadoo Edition"));
                push(dir);
            }
        }
        let system_drive = std::env::var_os("SystemDrive")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:"));
        // `C:` alone is cwd-relative on Windows; re-anchor at the root.
        let drive_root = PathBuf::from(format!("{}\\", system_drive.display()));
        push(drive_root.join("GOG Games"));
        push(drive_root.join("Games"));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return roots;
        };
        // GOG via MojoSetup / Heroic / Lutris under Wine.
        push(home.join("GOG Games"));
        push(home.join("Games/Heroic"));
        push(home.join("Games"));
        push(home.join(".wine/drive_c/GOG Games"));
        push(home.join(".wine/drive_c/Program Files (x86)/GOG.com"));
        push(home.join(".wine/drive_c/Program Files (x86)/GOG Galaxy/Games"));
        push(home.join(".wine/drive_c/Program Files/Wanadoo Edition"));
        push(home.join(".wine/drive_c/Program Files (x86)/Wanadoo Edition"));
        // Steam library folders (native client layouts).
        push(home.join(".local/share/Steam/steamapps/common"));
        push(home.join(".steam/steam/steamapps/common"));
        #[cfg(target_os = "macos")]
        push(home.join("Library/Application Support/Steam/steamapps/common"));
    }

    roots
}

/// Probe the well-known install locations of the CD, GOG, and Steam
/// releases and return the first valid installation.
pub fn find_installed_datadir() -> Option<PathBuf> {
    for root in search_roots() {
        if let Some(candidate) = find_installation_in_root(&root) {
            tracing::info!("Found game installation: {}", candidate.display());
            return Some(candidate);
        }
    }
    None
}

fn find_installation_in_root(root: &Path) -> Option<PathBuf> {
    let mut candidates = vec![None; INSTALL_FOLDER_NAMES.len()];
    for entry in directory_entries(root) {
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        if let Some(index) = INSTALL_FOLDER_NAMES
            .iter()
            .position(|name| filename.eq_ignore_ascii_case(name))
            && candidates[index].is_none()
        {
            candidates[index] = Some(entry.path());
        }
    }
    // Preserve configured priority, not filesystem enumeration order. As with
    // single-name lookup, only the first case-insensitive match is considered.
    candidates
        .into_iter()
        .flatten()
        .find(|candidate| is_valid_install_dir(candidate))
}

/// Accept a picked folder as either the installation root or its `Data`
/// subfolder (players often select `Data` itself), returning the root.
#[cfg(any(feature = "dialogs", all(test, not(target_arch = "wasm32"))))]
fn normalize_selection(path: &Path) -> Option<PathBuf> {
    if is_valid_install_dir(path) {
        return Some(path.to_owned());
    }
    if !path.file_name()?.to_str()?.eq_ignore_ascii_case("Data") {
        return None;
    }
    path.parent()
        .filter(|parent| is_valid_install_dir(parent))
        .map(Path::to_owned)
}

/// Invalid selections stay in the picker flow and never reach startup.
#[cfg(any(feature = "dialogs", all(test, not(target_arch = "wasm32"))))]
fn pick_valid_folder(
    mut pick: impl FnMut() -> Option<PathBuf>,
    mut report_invalid: impl FnMut(&Path),
) -> Option<PathBuf> {
    loop {
        let picked = pick()?;
        if let Some(install_dir) = normalize_selection(&picked) {
            tracing::info!(
                "Player selected game installation: {}",
                install_dir.display()
            );
            return Some(install_dir);
        }
        tracing::warn!("Rejected game data folder: {}", picked.display());
        report_invalid(&picked);
    }
}

/// Whether a native dialog can appear at all. Prevents headless runs
/// (CI, batch tools without a datadir) from hanging on an invisible
/// prompt.
#[cfg(feature = "dialogs")]
fn display_available() -> bool {
    if cfg!(target_os = "linux") {
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
    } else {
        true
    }
}

// ─── Persisted choice ────────────────────────────────────────────

/// Location of the remembered-datadir config, next to the saves
/// (`~/.local/share/robin_hood/datadir.txt` on Linux). `None` when the
/// build has no OS-data-dir support.
fn config_path() -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    return dirs::data_dir().map(|dir| dir.join("robin_hood").join("datadir.txt"));
    #[cfg(target_arch = "wasm32")]
    None
}

/// Previously confirmed datadir, if it is still a valid installation.
pub fn load_saved_datadir() -> Option<PathBuf> {
    load_saved_datadir_from(&config_path()?)
}

fn load_saved_datadir_from(path: &Path) -> Option<PathBuf> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(path = %path.display(), "Could not read remembered game datadir: {error}");
            return None;
        }
    };
    let dir = PathBuf::from(content.trim());
    if dir.as_os_str().is_empty() {
        return None;
    }
    if is_valid_install_dir(&dir) {
        Some(dir)
    } else {
        tracing::warn!(
            "Saved game datadir {} no longer holds the game data; ignoring it",
            dir.display()
        );
        None
    }
}

/// Remember a confirmed datadir so the startup dialog only asks once.
pub fn save_datadir(dir: &Path) -> std::io::Result<()> {
    let path = config_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "remembering a game datadir requires an OS data directory",
        )
    })?;
    persist_datadir(&path, dir)?;
    tracing::info!(
        "Remembered game datadir {} in {}",
        dir.display(),
        path.display()
    );
    Ok(())
}

fn persist_datadir(path: &Path, dir: &Path) -> std::io::Result<()> {
    // The existing text format cannot round-trip non-UTF-8 names or newlines.
    // Reject these explicitly instead of remembering a different path.
    let text = dir
        .to_str()
        .filter(|value| {
            !value.is_empty() && value.trim() == *value && !value.contains(['\n', '\r'])
        })
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "game datadir cannot be represented in datadir.txt",
            )
        })?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::desktop_persistence::write_bytes(path, format!("{text}\n").as_bytes())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, text);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "native datadir preferences are unavailable in the browser",
        ))
    }
}

// ─── Dialogs ─────────────────────────────────────────────────────

#[cfg(feature = "dialogs")]
const STORE_RECOMMENDATION: &str = "I recommend to buy it on GOG if you do not have it:";

#[cfg(feature = "dialogs")]
fn store_recommendation() -> String {
    format!(
        "{STORE_RECOMMENDATION}\n{GOG_STORE_URL}\n\
         It is also available on Steam, but the distributers there do not care \
         about it - it breaks on modern Windows without tweaks."
    )
}

/// Confirmation dialog over an auto-detected installation: OK accepts it,
/// Cancel opens the folder picker instead.
#[cfg(feature = "dialogs")]
fn confirm_candidate(candidate: &Path) -> rfd::MessageDialogResult {
    pollster::block_on(
        rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title("Robin Hood — game data found")
            .set_description(format!(
                "This is an open-source engine for Robin Hood: The Legend of Sherwood; \
                 it uses the original game's data files.\n\n\
                 A game installation was found at:\n{}\n\n\
                 Click OK to use it (remembered for future launches), or Cancel to \
                 choose a different folder yourself.\n\n{}",
                candidate.display(),
                store_recommendation(),
            ))
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show(),
    )
}

/// Introduction dialog when nothing was auto-detected: OK opens the
/// folder picker, Cancel aborts.
#[cfg(feature = "dialogs")]
fn confirm_search() -> rfd::MessageDialogResult {
    pollster::block_on(
        rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title("Robin Hood — game data not found")
            .set_description(format!(
                "This is an open-source engine for Robin Hood: The Legend of Sherwood; \
                 it needs the original game's data files, and no installation was found.\n\n\
                 Click OK to select the folder where the game is installed \
                 (the one containing Data/{MARKER_FILE}).\n\n{}",
                store_recommendation(),
            ))
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show(),
    )
}

/// Folder-picker loop: pick, validate, re-prompt on an invalid choice.
/// Returns `None` when the player cancels the picker.
#[cfg(feature = "dialogs")]
fn pick_folder_loop() -> Option<PathBuf> {
    pick_valid_folder(
        || {
            pollster::block_on(
                rfd::AsyncFileDialog::new()
                    .set_title("Select the Robin Hood installation folder")
                    .pick_folder(),
            )
            .map(|folder| folder.path().to_path_buf())
        },
        |picked| {
            pollster::block_on(
                rfd::AsyncMessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("Not a Robin Hood installation")
                    .set_description(format!(
                        "{} does not contain the original game's data files.\n\n\
                         Select the folder where Robin Hood: The Legend of Sherwood \
                         is installed. The engine needs the original game's files to run.\n\n\
                         Click OK to choose again, or cancel the folder picker to stop.",
                        picked.display()
                    ))
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show(),
            );
        },
    )
}

/// Resolve the datadir when no explicit override or env var was given.
///
/// A previously confirmed choice is used silently, so the dialog only
/// appears once. Otherwise the best auto-detected candidate — working
/// directory, executable directory, then the well-known install
/// locations — is always shown in a confirmation dialog: OK accepts it,
/// Cancel opens the folder picker instead. The confirmed choice is
/// remembered for future launches. Headless runs use the candidate
/// without a dialog and without remembering it.
pub fn resolve_datadir(exe_dir: Option<&Path>) -> DataDirResolution {
    if let Some(saved) = load_saved_datadir() {
        tracing::info!("Using remembered game datadir: {}", saved.display());
        return DataDirResolution::Selected(saved);
    }

    let candidate = if is_valid_install_dir(Path::new(".")) {
        std::env::current_dir().ok()
    } else if let Some(exe_dir) = exe_dir.filter(|dir| is_valid_install_dir(dir)) {
        Some(exe_dir.to_owned())
    } else {
        find_installed_datadir()
    };

    #[cfg(not(feature = "dialogs"))]
    return DataDirResolution::detected(candidate);

    #[cfg(feature = "dialogs")]
    {
        if !display_available() {
            if candidate.is_none() {
                tracing::warn!("No display available; skipping the datadir picker dialog");
            }
            return DataDirResolution::detected(candidate);
        }

        let chosen = match candidate {
            Some(candidate) => {
                if confirm_candidate(&candidate) == rfd::MessageDialogResult::Ok {
                    Some(candidate)
                } else {
                    pick_folder_loop()
                }
            }
            None => {
                if confirm_search() == rfd::MessageDialogResult::Ok {
                    pick_folder_loop()
                } else {
                    None
                }
            }
        };
        if let Some(chosen) = &chosen {
            if let Err(error) = save_datadir(chosen) {
                tracing::warn!("Could not remember selected game datadir: {error}");
            }
        }
        DataDirResolution::picked(chosen)
    }
}

/// Options-menu entry point: pick a new game data folder with the native
/// picker, remember it, and tell the player it applies on the next
/// launch. Returns the new folder, or `None` when the player cancelled.
#[cfg(feature = "dialogs")]
pub fn change_datadir_interactive() -> Option<PathBuf> {
    if !display_available() {
        return None;
    }
    let chosen = pick_folder_loop()?;
    if let Err(error) = save_datadir(&chosen) {
        tracing::warn!("Could not remember selected game datadir: {error}");
        pollster::block_on(
            rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Game data folder could not be saved")
                .set_description(format!("Could not remember the selected folder:\n{error}"))
                .set_buttons(rfd::MessageButtons::Ok)
                .show(),
        );
        return None;
    }
    pollster::block_on(
        rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title("Game data folder saved")
            .set_description(format!(
                "The game data folder is now:\n{}\n\n\
                 The change takes effect the next time the game starts.",
                chosen.display()
            ))
            .set_buttons(rfd::MessageButtons::Ok)
            .show(),
    );
    Some(chosen)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #[test]
    fn root_scan_preserves_folder_priority_and_skips_invalid_installations() {
        let root = tempfile::tempdir().unwrap();
        let preferred = root
            .path()
            .join(super::INSTALL_FOLDER_NAMES[0].to_ascii_uppercase());
        let fallback = root.path().join(super::INSTALL_FOLDER_NAMES[1]);
        // Create in reverse preference order so creation order is not policy.
        for path in [&fallback, &preferred] {
            std::fs::create_dir_all(path.join("Data")).unwrap();
            std::fs::write(path.join("Data/datadir.bin"), b"marker").unwrap();
        }
        assert_eq!(
            super::find_installation_in_root(root.path()),
            Some(preferred.clone())
        );
        std::fs::remove_file(preferred.join("Data/datadir.bin")).unwrap();
        assert_eq!(
            super::find_installation_in_root(root.path()),
            Some(fallback)
        );
        assert_eq!(
            super::find_installation_in_root(&root.path().join("missing")),
            None
        );
    }

    use super::*;

    #[test]
    fn selection_uses_the_same_file_markers_for_roots_and_data_subfolders() {
        for marker in ["ROBINHOOD.BKS", "DATADIR.BIN"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let data = root.join("dAtA");
            std::fs::create_dir(&data).unwrap();
            assert!(!is_valid_install_dir(root));
            std::fs::write(data.join(marker), "fixture").unwrap();
            assert!(is_valid_install_dir(root));
            assert_eq!(normalize_selection(root).as_deref(), Some(root));
            assert_eq!(normalize_selection(&data).as_deref(), Some(root));
        }
    }

    #[test]
    fn marker_directories_and_unrelated_subfolders_are_not_installations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir_all(root.join("Data/robinhood.bks")).unwrap();
        std::fs::create_dir_all(root.join("Data/datadir.bin")).unwrap();
        assert!(!is_valid_install_dir(root));
        assert!(normalize_selection(&root.join("Data")).is_none());
        let unrelated = root.join("Other");
        std::fs::create_dir(&unrelated).unwrap();
        std::fs::write(unrelated.join(MARKER_FILE), "fixture").unwrap();
        assert!(normalize_selection(&unrelated).is_none());
    }

    #[test]
    fn remembered_directory_roundtrips_and_invalid_paths_preserve_the_previous_choice() {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("Unicode 雪");
        std::fs::create_dir_all(install.join("Data")).unwrap();
        std::fs::write(install.join("Data/datadir.bin"), "fixture").unwrap();
        let config = directory.path().join("preferences/datadir.txt");
        assert!(load_saved_datadir_from(&config).is_none());
        persist_datadir(&config, &install).unwrap();
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            format!("{}\n", install.display())
        );
        assert_eq!(load_saved_datadir_from(&config), Some(install));
        let before = std::fs::read(&config).unwrap();
        for invalid in ["", " leading", "trailing ", "line\nbreak", "line\rbreak"] {
            assert_eq!(
                persist_datadir(&config, Path::new(invalid))
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidInput
            );
            assert_eq!(std::fs::read(&config).unwrap(), before);
        }
        assert!(persist_datadir(&config.join("child"), Path::new("game")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_directory_is_not_silently_rewritten() {
        use std::os::unix::ffi::OsStrExt;
        let directory = tempfile::tempdir().unwrap();
        let invalid = Path::new(std::ffi::OsStr::from_bytes(b"/game/\xff"));
        assert_eq!(
            persist_datadir(&directory.path().join("datadir.txt"), invalid)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod picker_tests {
    use super::*;

    #[test]
    fn cancelling_is_distinct_from_no_automatic_candidate() {
        assert_eq!(
            DataDirResolution::picked(None),
            DataDirResolution::Cancelled
        );
        assert_eq!(
            DataDirResolution::detected(None),
            DataDirResolution::Unavailable
        );
    }

    #[test]
    fn invalid_selections_keep_asking_until_a_valid_installation() {
        let root = tempfile::tempdir().unwrap();
        let invalid = root.path().join("random");
        std::fs::create_dir(&invalid).unwrap();
        // A marker outside Data must not cause its invalid parent to be accepted.
        std::fs::write(invalid.join(MARKER_FILE), []).unwrap();
        let fake = root.path().join("fake");
        std::fs::create_dir_all(fake.join("Data").join(MARKER_FILE)).unwrap();
        let valid = root.path().join("game");
        std::fs::create_dir_all(valid.join("Data")).unwrap();
        std::fs::write(valid.join("Data").join(MARKER_FILE), []).unwrap();
        let mut choices = [
            invalid.clone(),
            fake.clone(),
            invalid.clone(),
            valid.clone(),
        ]
        .into_iter();
        let mut rejected = Vec::new();
        let selected = pick_valid_folder(
            || Some(choices.next().expect("must accept the valid installation")),
            |path| rejected.push(path.to_owned()),
        );
        assert_eq!(selected, Some(valid));
        assert_eq!(rejected, [invalid.clone(), fake, invalid]);
        assert!(choices.next().is_none());
    }

    #[test]
    fn cancellation_after_invalid_selection_returns_no_folder() {
        let root = tempfile::tempdir().unwrap();
        let mut choices = [Some(root.path().to_owned()), None].into_iter();
        let mut rejected = Vec::new();
        assert_eq!(
            pick_valid_folder(
                || choices.next().expect("must stop on cancellation"),
                |path| rejected.push(path.to_owned()),
            ),
            None
        );
        assert_eq!(rejected, [root.path()]);
    }

    #[test]
    fn accepts_original_and_converted_data_subfolders_case_insensitively() {
        for marker in ["ROBINHOOD.BKS", "DATADIR.BIN"] {
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("dAtA");
            std::fs::create_dir(&data).unwrap();
            std::fs::write(data.join(marker), []).unwrap();
            assert_eq!(
                normalize_selection(root.path()),
                Some(root.path().to_owned())
            );
            assert_eq!(normalize_selection(&data), Some(root.path().to_owned()));
        }
    }
}

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
//! `Data/robinhood.bks` in any capitalization — the sprite-bank index that
//! every release ships and nothing else plausibly provides.

use std::path::{Path, PathBuf};

/// Where to buy the game; shown in the picker dialog.
pub const GOG_STORE_URL: &str = "https://www.gog.com/game/robin_hood_the_legend_of_sherwood";

/// Marker file identifying a correct datadir, looked up case-insensitively
/// inside the installation's `Data/` folder.
const MARKER_FILE: &str = "robinhood.bks";

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

/// Case-insensitive single-component lookup: the entry of `dir` whose name
/// matches `name` ignoring ASCII case.
fn entry_case_insensitive(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.find_map(|entry| {
        let entry = entry.ok()?;
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
            entry_case_insensitive(&data, MARKER_FILE).is_some()
                || entry_case_insensitive(&data, "datadir.bin").is_some()
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
        for name in INSTALL_FOLDER_NAMES {
            let Some(candidate) = entry_case_insensitive(&root, name) else {
                continue;
            };
            if is_valid_install_dir(&candidate) {
                tracing::info!("Found game installation: {}", candidate.display());
                return Some(candidate);
            }
        }
    }
    None
}

/// Accept a picked folder as either the installation root or its `Data`
/// subfolder (players often select `Data` itself), returning the root.
fn normalize_selection(path: &Path) -> Option<PathBuf> {
    if is_valid_install_dir(path) {
        return Some(path.to_owned());
    }
    if entry_case_insensitive(path, MARKER_FILE).is_some() {
        return path.parent().map(Path::to_owned);
    }
    None
}

/// Whether a native dialog can appear at all. Prevents headless runs
/// (CI, batch tools without a datadir) from hanging on an invisible
/// prompt.
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
    #[cfg(feature = "native-fs")]
    return dirs::data_dir().map(|dir| dir.join("robin_hood").join("datadir.txt"));
    #[cfg(not(feature = "native-fs"))]
    None
}

/// Previously confirmed datadir, if it is still a valid installation.
pub fn load_saved_datadir() -> Option<PathBuf> {
    let content = std::fs::read_to_string(config_path()?).ok()?;
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
pub fn save_datadir(dir: &Path) {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, format!("{}\n", dir.display())) {
        Ok(()) => tracing::info!(
            "Remembered game datadir {} in {}",
            dir.display(),
            path.display()
        ),
        Err(error) => tracing::warn!(
            "Failed to remember game datadir in {}: {error}",
            path.display()
        ),
    }
}

// ─── Dialogs ─────────────────────────────────────────────────────

const STORE_RECOMMENDATION: &str = "I recommend to buy it on GOG if you do not have it:";

fn store_recommendation() -> String {
    format!(
        "{STORE_RECOMMENDATION}\n{GOG_STORE_URL}\n\
         It is also available on Steam, but the distributers there do not care \
         about it - it breaks on modern Windows without tweaks."
    )
}

/// Confirmation dialog over an auto-detected installation: OK accepts it,
/// Cancel opens the folder picker instead.
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
fn pick_folder_loop() -> Option<PathBuf> {
    loop {
        let folder = pollster::block_on(
            rfd::AsyncFileDialog::new()
                .set_title("Select the Robin Hood installation folder")
                .pick_folder(),
        )?;
        let picked = folder.path().to_path_buf();
        if let Some(install_dir) = normalize_selection(&picked) {
            tracing::info!(
                "Player selected game installation: {}",
                install_dir.display()
            );
            return Some(install_dir);
        }
        pollster::block_on(
            rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Not a Robin Hood installation")
                .set_description(format!(
                    "{} does not contain Data/{MARKER_FILE}.\n\
                     Please select the game's installation folder.",
                    picked.display()
                ))
                .set_buttons(rfd::MessageButtons::Ok)
                .show(),
        );
    }
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
pub fn resolve_datadir(exe_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(saved) = load_saved_datadir() {
        tracing::info!("Using remembered game datadir: {}", saved.display());
        return Some(saved);
    }

    let candidate = if is_valid_install_dir(Path::new(".")) {
        std::env::current_dir().ok()
    } else if let Some(exe_dir) = exe_dir.filter(|dir| is_valid_install_dir(dir)) {
        Some(exe_dir.to_owned())
    } else {
        find_installed_datadir()
    };

    if !display_available() {
        if candidate.is_none() {
            tracing::warn!("No display available; skipping the datadir picker dialog");
        }
        return candidate;
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
    }?;
    save_datadir(&chosen);
    Some(chosen)
}

/// Options-menu entry point: pick a new game data folder with the native
/// picker, remember it, and tell the player it applies on the next
/// launch. Returns the new folder, or `None` when the player cancelled.
pub fn change_datadir_interactive() -> Option<PathBuf> {
    if !display_available() {
        return None;
    }
    let chosen = pick_folder_loop()?;
    save_datadir(&chosen);
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

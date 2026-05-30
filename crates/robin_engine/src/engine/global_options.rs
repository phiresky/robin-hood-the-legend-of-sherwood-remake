//! Application-wide startup options (`GlobalOptions`).

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// ─── Global options ──────────────────────────────────────────────────

/// Application-wide startup options.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct GlobalOptions {
    pub major_version: u16,
    pub minor_version: u16,
    pub build_number: u16,
    pub release_name: String,

    // Directories
    pub save_directory: String,
    pub level_directory: String,
    pub sound_directory: String,
    pub music_directory: String,
    pub character_directory: String,
    pub animation_directory: String,
    pub configuration_directory: String,
    pub interface_directory: String,
    pub text_directory: String,
    pub cinematics_directory: String,

    // Runtime flags
    pub quit: bool,
    pub console: bool,
    pub sound_enabled: bool,
    pub check_sound_data: bool,
    pub patch_characters: bool,
    pub highlander2: bool,
    pub whatsup: bool,
    pub debug_surfaces: bool,
    pub ezekiel2517: bool,
    pub golden_eye: bool,
    pub script_enabled: bool,
    pub ignore_default_loose: bool,
    pub set_reg: bool,
    pub bypass_fog_sprites_crash: bool,
}

impl Default for GlobalOptions {
    fn default() -> Self {
        Self {
            major_version: 1,
            minor_version: 2,
            build_number: 0,
            release_name: String::new(),

            save_directory: "Data/Savegame".into(),
            level_directory: "Data/Levels".into(),
            sound_directory: "Data/Sounds".into(),
            music_directory: "Data/Musics".into(),
            character_directory: "Data/Characters".into(),
            animation_directory: "Data/Animations".into(),
            configuration_directory: "Data/Configuration".into(),
            interface_directory: "Data/Interface".into(),
            text_directory: "Data/Text".into(),
            cinematics_directory: "Data/Cinematics".into(),

            quit: false,
            console: true,
            sound_enabled: true,
            check_sound_data: false,
            patch_characters: false,
            highlander2: false,
            whatsup: false,
            debug_surfaces: false,
            ezekiel2517: false,
            golden_eye: false,
            script_enabled: true,
            ignore_default_loose: false,
            set_reg: false,
            bypass_fog_sprites_crash: false,
        }
    }
}

// ─── Global singleton ───────────────────────────────────────────────
//
// A process-wide store the menu layer reaches without having to thread
// `&GlobalOptions` through every UI call.  Populated by
// `main_entry::parse_cli` once the CLI has been walked.

static GLOBAL_OPTIONS: Mutex<Option<GlobalOptions>> = Mutex::new(None);

impl GlobalOptions {
    /// Install the process-wide `GlobalOptions`.  Usually called once
    /// from `main_entry::parse_cli` after argument parsing.
    pub fn set_global(opts: GlobalOptions) {
        *GLOBAL_OPTIONS.lock().unwrap() = Some(opts);
    }

    /// Acquire the process-wide `GlobalOptions`.  Returns `None` if
    /// `set_global` has not been called yet (tests, headless tooling).
    pub fn global() -> std::sync::MutexGuard<'static, Option<GlobalOptions>> {
        GLOBAL_OPTIONS.lock().unwrap()
    }
}

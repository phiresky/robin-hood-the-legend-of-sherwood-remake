//! Per-mission resource cache shared by every in-game menu.  Loads:
//!
//! - `Data/Interface/DEFAULT.RES` — menu button sprites, parchment and
//!   numbered window backgrounds, radio/toggle backgrounds, slider bar,
//!   portraits, checkmark, separator.
//! - `Data/Interface/Fonts/manager.cfg` + the referenced `.sbf` files —
//!   native bitmap fonts `MenuButtonEnabled`, `MenuButtonDisabled`,
//!   `MissionTitle`, `PopupScroll`, `Default`, `EditField`, `Debrief`,
//!   `ActiveShortBriefing`, `InactiveShortBriefing`, `ListDefault`,
//!   `ListFocused`, `ListSelected`.
//! - The menu text string table (id `1000507` for the campaign build,
//!   `1000040` / `1000034` for the demo variants) — loaded from
//!   whichever `.sxt` file already lives on disk.

use std::collections::HashMap;

use crate::main_entry::picture_to_surface;
use crate::native_font::{self, Font, NativeFont};
use crate::renderer::Renderer;
use crate::resource_ids;
use crate::resource_manager::ResourceManager;

// ═══════════════════════════════════════════════════════════════════
// Menu text string table
// ═══════════════════════════════════════════════════════════════════

/// Campaign menu text table ID.
pub const MENU_TEXT_TABLE_ID: i32 = 1000507;
/// Demo 1 menu text table ID.
pub const MENU_TEXT_TABLE_ID_DEMO: i32 = 1000040;
/// Demo 2 menu text table ID.
pub const MENU_TEXT_TABLE_ID_DEMO2: i32 = 1000034;

// ── Menu text IDs ───────────────────────────────────────────────────
//
// Only the subset consumed by the in-game menus is named here; the full
// table has 343 entries.  Kept as plain `usize` constants so lookups are
// simple `.get(id)` calls.

pub const MT_BTN_START_GAME: usize = 0;
pub const MT_BTN_SELECT_PLAYER: usize = 1;
pub const MT_BTN_SHOW_MOVIES: usize = 2;
pub const MT_BTN_SHOW_CREDITS: usize = 3;
pub const MT_BTN_SELECT: usize = 5;
pub const MT_BTN_NEW: usize = 6;
pub const MT_BTN_RENAME: usize = 7;
pub const MT_BTN_DELETE: usize = 8;
pub const MT_BTN_CONTINUE: usize = 9;
pub const MT_BTN_LOAD: usize = 10;
pub const MT_BTN_SAVE: usize = 11;
pub const MT_BTN_OPTIONS: usize = 12;
pub const MT_BTN_RESTART: usize = 13;
pub const MT_BTN_QUIT_GAME: usize = 14;
pub const MT_BTN_OK: usize = 15;
pub const MT_BTN_CANCEL: usize = 16;
pub const MT_BTN_BACK: usize = 17;
pub const MT_BTN_GRAPHICS: usize = 18;
pub const MT_BTN_SOUNDS: usize = 19;
pub const MT_BTN_SHORTCUTS: usize = 20;
pub const MT_BTN_DEFAULT_1: usize = 21;
pub const MT_BTN_DEFAULT_2: usize = 22;
pub const MT_BTN_USER_DEFINED: usize = 23;

pub const MT_TTL_MISSION_WON: usize = 24;
pub const MT_TTL_MISSION_LOST: usize = 25;
pub const MT_TTL_MISSION_ABORTED: usize = 26;
pub const MT_TTL_OPTIONS: usize = 27;
pub const MT_TTL_GRAPHICS: usize = 28;
pub const MT_TTL_SOUNDS: usize = 29;
pub const MT_TTL_NEW_PLAYER: usize = 30;

// Blazon-purchase modal:
//   239 — "Do you want to buy a blazon?" confirmation line.
//   240 — "Blazon price: %lu" format string.
pub const MT_MSG_BUY_BLAZON: usize = 239;
pub const MT_STR_BLAZON_PRICE: usize = 240;

pub const MT_MSG_REALLY_QUIT: usize = 31;
pub const MT_MSG_REALLY_DELETE_SAVEGAME: usize = 32;
pub const MT_MSG_REALLY_OVERWRITE_SAVEGAME: usize = 33;
pub const MT_MSG_REALLY_DELETE_PLAYER: usize = 254;
pub const MT_MSG_RETURN_TO_WINDOWS: usize = 263;

/// In-game banner displayed after a successful save / load.
pub const MT_MSG_GAME_SAVED: usize = 256;
pub const MT_MSG_GAME_LOADED: usize = 257;

/// Pseudo-mission (strategical) debriefing messages shown on the
/// campaign map after a pseudo-mission resolves.
pub const MT_MSG_STRATEGICAL_MISSION_LOST: usize = 259;
pub const MT_MSG_STRATEGICAL_MISSION_WON: usize = 260;

/// Cross-mission quicksave confirmation prompt fired when the
/// quicksave's mission ID differs from the running mission.
pub const MT_MSG_REALLY_LOAD_QUICKSAVE: usize = 261;

// Sherwood / mission confirm prompts.
/// First-time mission-won banner message — shown once after the player
/// reaches a guarded exit with no escort in flight.
pub const MT_MSG_LEAVE_MISSION_NOW: usize = 235;
pub const MT_MSG_REALLY_ABORT_MISSION: usize = 238;
pub const MT_MSG_REALLY_CONVERT_PEASANTS: usize = 241;
pub const MT_MSG_REALLY_START_MISSION: usize = 253;
pub const MT_MSG_REALLY_RETURN_TO_MAP: usize = 255;

// ── Main-menu profile info block ────────────────────────────────────
pub const MT_STR_DIFFICULTY_EASY: usize = 34;
pub const MT_STR_DIFFICULTY_MEDIUM: usize = 35;
pub const MT_STR_DIFFICULTY_HARD: usize = 36;
pub const MT_STR_ANONYMOUS: usize = 62;
pub const MT_STR_MONEY: usize = 63;
pub const MT_STR_CARNAGE_FACTOR: usize = 65;
pub const MT_STR_PROGRESSION: usize = 66;
pub const MT_STR_DIFFICULTY_LEVEL: usize = 67;

pub const MT_STR_PROCESSOR: usize = 38;
pub const MT_STR_MEMORY: usize = 39;
pub const MT_STR_MEGA_BYTES: usize = 40;
pub const MT_STR_MEGA_HERZS: usize = 41;
pub const MT_STR_RES: usize = 42;
pub const MT_STR_RES_LOW: usize = 43;
pub const MT_STR_RES_MEDIUM: usize = 44;
pub const MT_STR_RES_HIGH: usize = 45;
pub const MT_STR_SPECIAL_FX: usize = 46;
pub const MT_STR_ALPHA_VISION_FIELD: usize = 47;
pub const MT_STR_TRANSPARENT_SHADOWS: usize = 48;
pub const MT_STR_EFFECT_ANIMATIONS: usize = 49;
pub const MT_STR_BCKGND_ANIMATIONS: usize = 50;

pub const MT_STR_SOUND_STEREO: usize = 51;
pub const MT_STR_SOUND_EAX: usize = 52;
pub const MT_STR_SOUND_3D: usize = 53;
pub const MT_STR_SOUND_RES_HIGH: usize = 54;
pub const MT_STR_SOUND_RES_LOW: usize = 55;
pub const MT_STR_SOUND_VOL_FX: usize = 56;
pub const MT_STR_SOUND_VOL_DIALOGUE: usize = 57;
pub const MT_STR_SOUND_VOL_MUSIC: usize = 58;
pub const MT_STR_SOUND_VOL_COMMENT: usize = 59;
pub const MT_STR_SOUND_COMMENT_FREQUENCY: usize = 60;
pub const MT_STR_NAME: usize = 61;

pub const MT_STR_SHORT_BRIEFING_TITLE_PRIMARY: usize = 269;
pub const MT_STR_SHORT_BRIEFING_TITLE_SECONDARY: usize = 270;
pub const MT_STR_SHORT_BRIEFING_TITLE_SHERWOOD: usize = 271;

pub const MT_STR_SHORTCUT_00: usize = 151;

// Physical key → menu-text ids.  Used by the shortcuts rebind
// list to render localized key names (e.g. "Espacio" in ES rather than
// "Space").
pub const MT_STR_KEY_UP: usize = 179;
pub const MT_STR_KEY_DOWN: usize = 180;
pub const MT_STR_KEY_LEFT: usize = 181;
pub const MT_STR_KEY_RIGHT: usize = 182;
pub const MT_STR_KEY_SHIFT_LEFT: usize = 183;
pub const MT_STR_KEY_SHIFT_RIGHT: usize = 184;
pub const MT_STR_KEY_CAPS_LOCK: usize = 185;
pub const MT_STR_KEY_CTRL_LEFT: usize = 186;
pub const MT_STR_KEY_CTRL_RIGHT: usize = 187;
pub const MT_STR_KEY_SPACE: usize = 188;
pub const MT_STR_KEY_BACKSPACE: usize = 189;
pub const MT_STR_KEY_ESC: usize = 190;
pub const MT_STR_KEY_F1: usize = 191;
pub const MT_STR_KEY_F2: usize = 192;
pub const MT_STR_KEY_F3: usize = 193;
pub const MT_STR_KEY_F4: usize = 194;
pub const MT_STR_KEY_F5: usize = 195;
pub const MT_STR_KEY_F6: usize = 196;
pub const MT_STR_KEY_F7: usize = 197;
pub const MT_STR_KEY_F8: usize = 198;
pub const MT_STR_KEY_F9: usize = 199;
pub const MT_STR_KEY_F10: usize = 200;
pub const MT_STR_KEY_F11: usize = 201;
pub const MT_STR_KEY_F12: usize = 202;
pub const MT_STR_KEY_RETURN: usize = 203;
pub const MT_STR_KEY_NUM_LOCK: usize = 204;
pub const MT_STR_KEY_NUM_SLASH: usize = 205;
pub const MT_STR_KEY_NUM_STAR: usize = 206;
pub const MT_STR_KEY_NUM_DASH: usize = 207;
pub const MT_STR_KEY_NUM_CROSS: usize = 208;
pub const MT_STR_KEY_NUM_RETURN: usize = 209;
pub const MT_STR_KEY_NUM_7: usize = 210;
pub const MT_STR_KEY_NUM_8: usize = 211;
pub const MT_STR_KEY_NUM_9: usize = 212;
pub const MT_STR_KEY_NUM_4: usize = 213;
pub const MT_STR_KEY_NUM_5: usize = 214;
pub const MT_STR_KEY_NUM_6: usize = 215;
pub const MT_STR_KEY_NUM_1: usize = 216;
pub const MT_STR_KEY_NUM_2: usize = 217;
pub const MT_STR_KEY_NUM_3: usize = 218;
pub const MT_STR_KEY_NUM_0: usize = 219;
pub const MT_STR_KEY_NUM_SUP: usize = 220;
pub const MT_STR_KEY_INS: usize = 221;
pub const MT_STR_KEY_SUP: usize = 222;
pub const MT_STR_KEY_ALT: usize = 223;
pub const MT_STR_KEY_ALT_GR: usize = 224;
pub const MT_STR_KEY_TAB: usize = 225;
pub const MT_STR_KEY_PAGE_UP: usize = 226;
pub const MT_STR_KEY_PAGE_DOWN: usize = 227;
pub const MT_STR_KEY_HOME: usize = 228;
pub const MT_STR_KEY_END: usize = 229;
pub const MT_STR_KEY_PRINT: usize = 230;
pub const MT_STR_KEY_SCROLL_LOCK: usize = 231;
pub const MT_STR_KEY_PAUSE: usize = 232;
pub const MT_STR_KEY_NONE: usize = 233;
pub const MT_STR_KEY_RESERVED: usize = 234;

// Sherwood campaign-map / debriefing strings.
pub const MT_STR_SCORE: usize = 64;

// Blazon-bar slot tooltips:
//   320 — empty blazon tooltip ("to win")
//   321 — castle blazon tooltip ("to win inside attack mission")
//   322 — already-won blazon tooltip
pub const MT_INFOBULLE_BLAZON_TO_WIN: usize = 320;
pub const MT_INFOBULLE_BLAZON_TO_WIN_IN_ATTACK: usize = 321;
pub const MT_INFOBULLE_BLAZON_WON: usize = 322;

// Mission debriefing stat-panel strings:
//   S01 — hand-to-hand training (Sherwood)              (1x u)
//   S02 — bow training (Sherwood)                       (1x u)
//   S03 — healing (Sherwood)                            (1x u)
//   S04 — "(max reached)" suffix (Sherwood)             (0x)
//   S05 — "Sherwood report:" header                     (0x)
//   S06 — money collected by the player                 (1x u)
//   S07 — soldiers surviving / total                    (2x u)
//   S08 — total new members (peasants + PCs)            (1x u)
//   S09 — "<name> joined" suffix (printed after PC name)(0x)
//   S10 — peasants killed                               (1x u)
//   S11 — score added                                   (1x u)
//   S12 — Sherwood score line                           (1x u)
//   S13 — mission length (HH:MM:SS)                     (1x s)
//   S14 — Sherwood preserved lives                      (1x u)
//   S16 — Sherwood play time (HH:MM:SS)                 (1x s)
//   S17 — allied soldiers killed                        (1x u)
//   S18 — money from bonuses + soldiers, total / bonus / soldier (3x u)
//   C01 — "with" connective (Sherwood)                  (0x)
//   C02 — "led by %s" specialist suffix (Sherwood)      (1x s)
pub use robin_engine::sherwood_stat::{
    MT_STR_DB_C01, MT_STR_DB_C02, MT_STR_DB_S01, MT_STR_DB_S02, MT_STR_DB_S03, MT_STR_DB_S04,
    MT_STR_DB_S05, MT_STR_DB_S12, MT_STR_DB_S14, MT_STR_DB_S16,
};
pub const MT_STR_DB_S06: usize = 73;
pub const MT_STR_DB_S07: usize = 74;
pub const MT_STR_DB_S08: usize = 75;
pub const MT_STR_DB_S09: usize = 76;
pub const MT_STR_DB_S10: usize = 77;
pub const MT_STR_DB_S11: usize = 78;
pub const MT_STR_DB_S13: usize = 80;
pub const MT_STR_DB_S17: usize = 84;
pub const MT_STR_DB_S18: usize = 85;

pub const MT_STR_DB_BONUS_ARROW: usize = 89;
pub const MT_STR_DB_BONUS_APPLE: usize = 90;
pub const MT_STR_DB_BONUS_WASP_NEST: usize = 91;
pub const MT_STR_DB_BONUS_LAMB_LEGG: usize = 92;
pub const MT_STR_DB_BONUS_PLANTS: usize = 93;
pub const MT_STR_DB_BONUS_STONE: usize = 94;
pub const MT_STR_DB_BONUS_ALE: usize = 95;
pub const MT_STR_DB_BONUS_NET: usize = 96;
pub const MT_STR_DB_BONUS_PURSE: usize = 97;
pub const MT_STR_DB_MERRYMAN: usize = 98;
pub const MT_STR_DB_MERRYMEN: usize = 99;
pub const MT_STR_PRESERVED_LIFES: usize = 243;
pub const MT_STR_RANSOM: usize = 244;
pub const MT_STR_AMULETS: usize = 245;
pub const MT_STR_PLAYING_TIME: usize = 258;
pub const MT_WORD_NOTHING: usize = 329;

// Per-button tooltip menu-text ids.
pub const MT_INFOBULLE_BUTTON_PLAY_MISSION: usize = 323;
pub const MT_INFOBULLE_BUTTON_FARMERS_TO_BLAZON: usize = 325;
pub const MT_INFOBULLE_BUTTON_MONEY_TO_BLAZON: usize = 326;
pub const MT_INFOBULLE_BUTTON_MISSION_TO_BLAZON: usize = 327;
pub const MT_INFOBULLE_BUTTON_YES: usize = 330;
pub const MT_INFOBULLE_BUTTON_NO: usize = 331;
pub const MT_INFOBULLE_BUTTON_OK: usize = 332;
pub const MT_INFOBULLE_BUTTON_CANCEL: usize = 333;
pub const MT_INFOBULLE_BUTTON_RECOMMENCER: usize = 336;
pub const MT_INFOBULLE_BUTTON_DIALOG_CONTINUE: usize = 338;
pub const MT_INFOBULLE_BUTTON_DIALOG_ABANDON: usize = 339;

// Requirements-bar per-slot tooltip ids.  All three are static
// strings — one is wired to each slot type's icon at widget-creation
// time via `SetTooltipText`.
pub const MT_INFOBULLE_QG_NEEDED_PC: usize = 301;
pub const MT_INFOBULLE_QG_NEEDED_ACTION: usize = 302;
pub const MT_INFOBULLE_QG_OTHER_PC: usize = 303;

// Zoom button tooltip ids — attached to the zoom-up/zoom-down widgets.
pub const MT_INFOBULLE_ZOOMIN: usize = 289;
pub const MT_INFOBULLE_ZOOMOUT: usize = 290;

// Top-right HUD buttons tooltip ids — attached to the clock,
// quick-start icon, and sight (view-cone) widgets.
pub const MT_INFOBULLE_SAVEQA: usize = 285;
pub const MT_INFOBULLE_LAUNCHQA_ALL: usize = 287;
pub const MT_INFOBULLE_VIEWCONE: usize = 288;

// Stature (stand-up / crouch-down) arrow tooltip ids.
pub const MT_INFOBULLE_CROUCH: usize = 283;
pub const MT_INFOBULLE_STANDUP: usize = 284;

// PC action-button tooltip ids.  Fed to the three PC portrait action
// buttons by the per-action tooltip lookup.
pub const MT_INFOBULLE_ACTION_BOW: usize = 304;
pub const MT_INFOBULLE_ACTION_PURSE: usize = 305;
pub const MT_INFOBULLE_ACTION_NET: usize = 306;
pub const MT_INFOBULLE_ACTION_APPLE: usize = 307;
pub const MT_INFOBULLE_ACTION_STONE: usize = 308;
pub const MT_INFOBULLE_ACTION_FIST: usize = 309;
pub const MT_INFOBULLE_ACTION_STRANGLER: usize = 310;
pub const MT_INFOBULLE_ACTION_HERBS: usize = 311;
pub const MT_INFOBULLE_ACTION_GIGOT: usize = 312;
pub const MT_INFOBULLE_ACTION_BEER: usize = 313;
pub const MT_INFOBULLE_ACTION_SHIELD: usize = 314;
pub const MT_INFOBULLE_ACTION_WASP: usize = 315;
pub const MT_INFOBULLE_ACTION_SPY: usize = 316;
pub const MT_INFOBULLE_ACTION_COURTE_ECHELLE: usize = 317;
pub const MT_INFOBULLE_ACTION_SIMULER_MENDIANT: usize = 318;
pub const MT_INFOBULLE_ACTION_SIFFLER: usize = 319;

// Sherwood start/quit + in-mission finish/abandon tooltip ids.
// The start/quit widget tooltip update logic picks one of three
// `(start, quit)` pairs based on whether we're in Sherwood mode and
// whether men-to-blazon conversion is active.
pub const MT_INFOBULLE_MISSION_ABANDON: usize = 292;
pub const MT_INFOBULLE_MISSION_FINISH: usize = 293;
pub const MT_INFOBULLE_QG_BEGIN_MISSION: usize = 298;
pub const MT_INFOBULLE_QG_BACKTOMAP: usize = 300;

/// Menu text string table.
///
/// `strings[i]` is the wide string for the menu text id `i`.  Missing
/// entries return an English fallback taken from the original game's
/// resource file so the in-game menus stay readable even without the
/// localised text tables loaded.
#[derive(Default)]
pub struct MenuText {
    strings: Vec<String>,
    /// English fallbacks for the ids we actually use, indexed by id.
    fallbacks: HashMap<usize, &'static str>,
}

impl MenuText {
    /// Build a new table, trying the three known table ids in turn.
    ///
    /// The text tables usually live in `Data/Interface/Start.sxt` or
    /// `Data/Text/Level.res` depending on the build.  The caller supplies
    /// the [`ResourceManager`] that already has whichever file is
    /// available attached.
    pub fn load(res: &mut ResourceManager) -> Self {
        let tables = [
            MENU_TEXT_TABLE_ID,
            MENU_TEXT_TABLE_ID_DEMO,
            MENU_TEXT_TABLE_ID_DEMO2,
        ];

        let mut strings: Vec<String> = Vec::new();
        for &id in &tables {
            if let Ok(count) = res.get_string_count(id) {
                strings = (0..count)
                    .map(|i| {
                        res.get_string(id, i)
                            .map(str::to_string)
                            .unwrap_or_default()
                    })
                    .collect();
                tracing::info!(
                    "MenuText: loaded table {} with {} entries",
                    id,
                    strings.len()
                );
                break;
            }
        }
        if strings.is_empty() {
            tracing::warn!(
                "MenuText: none of the known text tables ({:?}) were found",
                tables
            );
        }

        Self {
            strings,
            fallbacks: default_fallbacks(),
        }
    }

    /// Build a table with only the English fallbacks loaded — useful
    /// for unit tests that don't want to spin up a `ResourceManager`.
    pub fn english_fallbacks_only() -> Self {
        Self {
            strings: Vec::new(),
            fallbacks: default_fallbacks(),
        }
    }

    /// Test helper: replace the loaded `strings` table.  Used by unit
    /// tests that need to inject a localised override (e.g. verifying
    /// that the rebind screen reads "Espacio" when the ES `.sxt` is
    /// loaded) without spinning up a `ResourceManager`.
    #[cfg(test)]
    pub fn replace_strings_for_test(&mut self, strings: Vec<String>) {
        self.strings = strings;
    }

    /// Look up a menu text entry by id.  Returns the English fallback
    /// when the resource table is missing or the entry is empty.
    pub fn get(&self, id: usize) -> String {
        if let Some(s) = self.strings.get(id)
            && !s.is_empty()
        {
            return s.clone();
        }
        self.fallbacks.get(&id).copied().unwrap_or("").to_string()
    }
}

fn default_fallbacks() -> HashMap<usize, &'static str> {
    // Hardcoded English strings match `1033/Data/Interface/Start.sxt` from
    // the international release.  Used when no `.sxt` file is available
    // so the Rust port is still usable on developer machines without the
    // localised text bundle.
    let mut m = HashMap::new();
    m.insert(MT_BTN_START_GAME, "Start Game");
    m.insert(MT_BTN_SELECT_PLAYER, "Select Player");
    m.insert(MT_BTN_SHOW_MOVIES, "Show Movies");
    m.insert(MT_BTN_SHOW_CREDITS, "Show Credits");
    m.insert(MT_BTN_SELECT, "Select");
    m.insert(MT_BTN_NEW, "New");
    m.insert(MT_BTN_RENAME, "Rename");
    m.insert(MT_BTN_DELETE, "Delete");
    m.insert(MT_BTN_CONTINUE, "Continue");
    m.insert(MT_BTN_LOAD, "Load");
    m.insert(MT_BTN_SAVE, "Save");
    m.insert(MT_BTN_OPTIONS, "Options");
    m.insert(MT_BTN_RESTART, "Restart");
    m.insert(MT_BTN_QUIT_GAME, "Quit Game");
    m.insert(MT_BTN_OK, "OK");
    m.insert(MT_BTN_CANCEL, "Cancel");
    m.insert(MT_BTN_BACK, "Back");
    m.insert(MT_BTN_GRAPHICS, "Graphics");
    m.insert(MT_BTN_SOUNDS, "Sounds");
    m.insert(MT_BTN_SHORTCUTS, "Shortcuts");
    m.insert(MT_BTN_DEFAULT_1, "Default 1");
    m.insert(MT_BTN_DEFAULT_2, "Default 2");
    m.insert(MT_BTN_USER_DEFINED, "User Defined");
    m.insert(MT_TTL_MISSION_WON, "Mission Won");
    m.insert(MT_TTL_MISSION_LOST, "Mission Lost");
    m.insert(MT_TTL_MISSION_ABORTED, "Mission Aborted");
    m.insert(MT_TTL_OPTIONS, "Options");
    m.insert(MT_TTL_GRAPHICS, "Graphics");
    m.insert(MT_TTL_SOUNDS, "Sounds");
    m.insert(MT_MSG_REALLY_QUIT, "Do you really want to quit the game?");
    m.insert(
        MT_MSG_STRATEGICAL_MISSION_WON,
        "You have won the strategical mission.",
    );
    m.insert(
        MT_MSG_STRATEGICAL_MISSION_LOST,
        "You have lost the strategical mission.",
    );
    m.insert(
        MT_MSG_REALLY_DELETE_SAVEGAME,
        "Do you really want to delete this saved game?",
    );
    m.insert(
        MT_MSG_REALLY_OVERWRITE_SAVEGAME,
        "Do you really want to overwrite this saved game?",
    );
    m.insert(
        MT_MSG_REALLY_DELETE_PLAYER,
        "Do you really want to delete this player?",
    );
    m.insert(
        MT_MSG_REALLY_ABORT_MISSION,
        "Do you really want to abort the mission?",
    );
    m.insert(
        MT_MSG_REALLY_CONVERT_PEASANTS,
        "Do you really want to convert your peasants into blazons?",
    );
    m.insert(
        MT_MSG_REALLY_START_MISSION,
        "Do you really want to start the mission?",
    );
    m.insert(
        MT_MSG_REALLY_RETURN_TO_MAP,
        "Do you really want to return to the campaign map?",
    );
    m.insert(
        MT_MSG_REALLY_LOAD_QUICKSAVE,
        "Do you really want to load this quicksave?",
    );
    m.insert(MT_MSG_RETURN_TO_WINDOWS, "Return to Windows?");
    m.insert(MT_MSG_GAME_SAVED, "Game saved.");
    m.insert(MT_MSG_GAME_LOADED, "Game loaded.");
    // Main-menu profile info block fallbacks.
    m.insert(MT_STR_DIFFICULTY_EASY, "Easy");
    m.insert(MT_STR_DIFFICULTY_MEDIUM, "Medium");
    m.insert(MT_STR_DIFFICULTY_HARD, "Hard");
    // `MT_STR_MONEY` in the stock table is a format string
    // ("Money: £%i") — the profile-info widget plugs the ransom into it
    // via `printf`-style substitution.  Keep the `%i` placeholder so
    // the call site substitutes the same way regardless of whether the
    // localized table loaded.
    m.insert(MT_STR_MONEY, "Money: £%i");
    m.insert(MT_STR_CARNAGE_FACTOR, "Spared lives");
    m.insert(MT_STR_PROGRESSION, "Progress");
    m.insert(MT_STR_DIFFICULTY_LEVEL, "Difficulty level");
    m.insert(MT_STR_PROCESSOR, "Processor");
    m.insert(MT_STR_MEMORY, "Memory");
    m.insert(MT_STR_MEGA_BYTES, "MB");
    m.insert(MT_STR_MEGA_HERZS, "MHz");
    m.insert(MT_STR_RES, "Resolution");
    m.insert(MT_STR_RES_LOW, "640 x 480");
    m.insert(MT_STR_RES_MEDIUM, "800 x 600");
    m.insert(MT_STR_RES_HIGH, "1024 x 768");
    m.insert(MT_STR_SPECIAL_FX, "Special Effects");
    m.insert(MT_STR_ALPHA_VISION_FIELD, "Alpha Vision Field");
    m.insert(MT_STR_TRANSPARENT_SHADOWS, "Transparent Shadows");
    m.insert(MT_STR_EFFECT_ANIMATIONS, "Effect Animations");
    m.insert(MT_STR_BCKGND_ANIMATIONS, "Background Animations");
    m.insert(MT_STR_SOUND_STEREO, "Stereo");
    m.insert(MT_STR_SOUND_EAX, "EAX 3D");
    m.insert(MT_STR_SOUND_3D, "3D");
    m.insert(MT_STR_SOUND_RES_HIGH, "High Resolution");
    m.insert(MT_STR_SOUND_RES_LOW, "Low Resolution");
    m.insert(MT_STR_SOUND_VOL_FX, "Sound Effects Volume");
    m.insert(MT_STR_SOUND_VOL_DIALOGUE, "Dialogue Volume");
    m.insert(MT_STR_SOUND_VOL_MUSIC, "Music Volume");
    m.insert(MT_STR_SOUND_VOL_COMMENT, "Comment Volume");
    m.insert(MT_STR_SOUND_COMMENT_FREQUENCY, "Comment Frequency");
    m.insert(MT_STR_SHORT_BRIEFING_TITLE_PRIMARY, "Primary objectives:");
    m.insert(
        MT_STR_SHORT_BRIEFING_TITLE_SECONDARY,
        "Secondary objectives:",
    );
    m.insert(MT_STR_SHORT_BRIEFING_TITLE_SHERWOOD, "Sherwood objectives:");
    // Sherwood debriefing + campaign-map status bar fallbacks.
    m.insert(MT_STR_SCORE, "Score");
    // Mission debriefing stat-panel format strings.  The real game text
    // table carries printf-style wide-char templates here (`%lu`/`%ls`);
    // fall back to `%u`/`%s` English when the resource table is missing
    // so the call sites can substitute the same way either way.
    m.insert(MT_STR_DB_S01, "%u trained in hand-to-hand combat");
    m.insert(MT_STR_DB_S02, "%u trained in archery");
    m.insert(MT_STR_DB_S03, "%u healed");
    m.insert(MT_STR_DB_S04, "(max reached)");
    m.insert(MT_STR_DB_S05, "Sherwood report:");
    m.insert(MT_STR_DB_S12, "Score: %u");
    m.insert(MT_STR_DB_S14, "Preserved lives: %u");
    m.insert(MT_STR_DB_S16, "Play time: %s");
    m.insert(MT_STR_DB_C01, "with");
    m.insert(MT_STR_DB_C02, "led by %s");
    m.insert(MT_STR_DB_S06, "You collected %u gold pieces.");
    m.insert(MT_STR_DB_S07, "%u of %u enemy soldiers still alive.");
    m.insert(MT_STR_DB_S08, "%u new gang members.");
    m.insert(MT_STR_DB_S09, "joined the gang.");
    m.insert(MT_STR_DB_S10, "%u peasants were killed.");
    m.insert(MT_STR_DB_S11, "Score: %u");
    m.insert(MT_STR_DB_S13, "Mission length: %s");
    m.insert(MT_STR_DB_S17, "%u allied soldiers were killed.");
    m.insert(
        MT_STR_DB_S18,
        "Found %u gold pieces (bonuses: %u, soldiers: %u).",
    );
    m.insert(MT_STR_DB_BONUS_ARROW, "arrows");
    m.insert(MT_STR_DB_BONUS_APPLE, "apples");
    m.insert(MT_STR_DB_BONUS_WASP_NEST, "wasp nests");
    m.insert(MT_STR_DB_BONUS_LAMB_LEGG, "lamb legs");
    m.insert(MT_STR_DB_BONUS_PLANTS, "plants");
    m.insert(MT_STR_DB_BONUS_STONE, "stones");
    m.insert(MT_STR_DB_BONUS_ALE, "ales");
    m.insert(MT_STR_DB_BONUS_NET, "nets");
    m.insert(MT_STR_DB_BONUS_PURSE, "purses");
    m.insert(MT_STR_DB_MERRYMAN, "merryman");
    m.insert(MT_STR_DB_MERRYMEN, "merrymen");
    m.insert(MT_STR_PRESERVED_LIFES, "Preserved lives");
    // `MT_STR_RANSOM` in the stock table is a format string
    // ("Ransom: %d") — the campaign-map status bar plugs the current
    // ransom amount into it via `printf`-style substitution.  Keep the
    // literal `%d` placeholder so call sites can substitute regardless
    // of whether the localized table loaded.
    m.insert(MT_STR_RANSOM, "Ransom: %d");
    m.insert(MT_STR_AMULETS, "Amulets: %d");
    // The blazon-purchase modal message is assembled as:
    //   STR_RANSOM + "\n" + STR_BLAZON_PRICE + "\n" + MSG_BUY_BLAZON
    // Keep the `%d` placeholder on the price format string so call
    // sites substitute with `replacen`.
    m.insert(MT_MSG_BUY_BLAZON, "Do you want to buy a blazon?");
    m.insert(MT_STR_BLAZON_PRICE, "Blazon price: %d");
    m.insert(MT_STR_PLAYING_TIME, "Play time");
    m.insert(MT_WORD_NOTHING, "nothing");
    m.insert(MT_INFOBULLE_BUTTON_YES, "Yes");
    m.insert(MT_INFOBULLE_BUTTON_NO, "No");
    m.insert(MT_INFOBULLE_BUTTON_OK, "OK");
    m.insert(MT_INFOBULLE_BUTTON_CANCEL, "Cancel");
    m.insert(MT_INFOBULLE_BUTTON_RECOMMENCER, "Restart mission");
    m.insert(MT_INFOBULLE_BUTTON_DIALOG_CONTINUE, "Continue");
    m.insert(MT_INFOBULLE_BUTTON_DIALOG_ABANDON, "Cancel");
    m.insert(MT_INFOBULLE_BUTTON_PLAY_MISSION, "Play mission");
    m.insert(
        MT_INFOBULLE_BUTTON_FARMERS_TO_BLAZON,
        "Trade merry men for blazons",
    );
    m.insert(
        MT_INFOBULLE_BUTTON_MONEY_TO_BLAZON,
        "Buy blazons with money",
    );
    m.insert(
        MT_INFOBULLE_BUTTON_MISSION_TO_BLAZON,
        "Play another mission for blazons",
    );
    m.insert(MT_INFOBULLE_QG_NEEDED_PC, "Required character");
    m.insert(MT_INFOBULLE_QG_NEEDED_ACTION, "Required action");
    m.insert(MT_INFOBULLE_QG_OTHER_PC, "Optional character");
    m.insert(MT_INFOBULLE_SAVEQA, "Record quick action");
    m.insert(MT_INFOBULLE_LAUNCHQA_ALL, "Launch all quick actions");
    m.insert(MT_INFOBULLE_VIEWCONE, "Show view cone");
    m.insert(MT_INFOBULLE_CROUCH, "Crouch down");
    m.insert(MT_INFOBULLE_STANDUP, "Stand up");
    m.insert(MT_INFOBULLE_ACTION_BOW, "Bow");
    m.insert(MT_INFOBULLE_ACTION_PURSE, "Purse");
    m.insert(MT_INFOBULLE_ACTION_NET, "Net");
    m.insert(MT_INFOBULLE_ACTION_APPLE, "Apple");
    m.insert(MT_INFOBULLE_ACTION_STONE, "Stone");
    m.insert(MT_INFOBULLE_ACTION_FIST, "Fist");
    m.insert(MT_INFOBULLE_ACTION_STRANGLER, "Strangle");
    m.insert(MT_INFOBULLE_ACTION_HERBS, "Herbs");
    m.insert(MT_INFOBULLE_ACTION_GIGOT, "Leg of lamb");
    m.insert(MT_INFOBULLE_ACTION_BEER, "Ale");
    m.insert(MT_INFOBULLE_ACTION_SHIELD, "Shield");
    m.insert(MT_INFOBULLE_ACTION_WASP, "Wasp nest");
    m.insert(MT_INFOBULLE_ACTION_SPY, "Listen");
    m.insert(MT_INFOBULLE_ACTION_COURTE_ECHELLE, "Help to climb");
    m.insert(MT_INFOBULLE_ACTION_SIMULER_MENDIANT, "Play the beggar");
    m.insert(MT_INFOBULLE_ACTION_SIFFLER, "Whistle");
    m.insert(MT_INFOBULLE_MISSION_ABANDON, "Abandon mission");
    m.insert(MT_INFOBULLE_MISSION_FINISH, "Finish mission");
    m.insert(MT_INFOBULLE_QG_BEGIN_MISSION, "Begin mission");
    m.insert(MT_INFOBULLE_QG_BACKTOMAP, "Return to the campaign map");
    // Scancode → key-name fallbacks for the shortcut rebind list.  These
    // mirror the names in `1033/Data/Interface/Start.sxt` so the rebind
    // screen stays readable when the localised `.sxt` is missing.
    m.insert(MT_STR_KEY_UP, "Up");
    m.insert(MT_STR_KEY_DOWN, "Down");
    m.insert(MT_STR_KEY_LEFT, "Left");
    m.insert(MT_STR_KEY_RIGHT, "Right");
    m.insert(MT_STR_KEY_SHIFT_LEFT, "Left Shift");
    m.insert(MT_STR_KEY_SHIFT_RIGHT, "Right Shift");
    m.insert(MT_STR_KEY_CAPS_LOCK, "Caps Lock");
    m.insert(MT_STR_KEY_CTRL_LEFT, "Left Ctrl");
    m.insert(MT_STR_KEY_CTRL_RIGHT, "Right Ctrl");
    m.insert(MT_STR_KEY_SPACE, "Space");
    m.insert(MT_STR_KEY_BACKSPACE, "Backspace");
    m.insert(MT_STR_KEY_ESC, "Escape");
    m.insert(MT_STR_KEY_F1, "F1");
    m.insert(MT_STR_KEY_F2, "F2");
    m.insert(MT_STR_KEY_F3, "F3");
    m.insert(MT_STR_KEY_F4, "F4");
    m.insert(MT_STR_KEY_F5, "F5");
    m.insert(MT_STR_KEY_F6, "F6");
    m.insert(MT_STR_KEY_F7, "F7");
    m.insert(MT_STR_KEY_F8, "F8");
    m.insert(MT_STR_KEY_F9, "F9");
    m.insert(MT_STR_KEY_F10, "F10");
    m.insert(MT_STR_KEY_F11, "F11");
    m.insert(MT_STR_KEY_F12, "F12");
    m.insert(MT_STR_KEY_RETURN, "Return");
    m.insert(MT_STR_KEY_NUM_LOCK, "Num Lock");
    m.insert(MT_STR_KEY_NUM_SLASH, "Keypad /");
    m.insert(MT_STR_KEY_NUM_STAR, "Keypad *");
    m.insert(MT_STR_KEY_NUM_DASH, "Keypad -");
    m.insert(MT_STR_KEY_NUM_CROSS, "Keypad +");
    m.insert(MT_STR_KEY_NUM_RETURN, "Keypad Enter");
    m.insert(MT_STR_KEY_NUM_7, "Keypad 7");
    m.insert(MT_STR_KEY_NUM_8, "Keypad 8");
    m.insert(MT_STR_KEY_NUM_9, "Keypad 9");
    m.insert(MT_STR_KEY_NUM_4, "Keypad 4");
    m.insert(MT_STR_KEY_NUM_5, "Keypad 5");
    m.insert(MT_STR_KEY_NUM_6, "Keypad 6");
    m.insert(MT_STR_KEY_NUM_1, "Keypad 1");
    m.insert(MT_STR_KEY_NUM_2, "Keypad 2");
    m.insert(MT_STR_KEY_NUM_3, "Keypad 3");
    m.insert(MT_STR_KEY_NUM_0, "Keypad 0");
    m.insert(MT_STR_KEY_NUM_SUP, "Keypad .");
    m.insert(MT_STR_KEY_INS, "Insert");
    m.insert(MT_STR_KEY_SUP, "Delete");
    m.insert(MT_STR_KEY_ALT, "Left Alt");
    m.insert(MT_STR_KEY_ALT_GR, "Right Alt");
    m.insert(MT_STR_KEY_TAB, "Tab");
    m.insert(MT_STR_KEY_PAGE_UP, "Page Up");
    m.insert(MT_STR_KEY_PAGE_DOWN, "Page Down");
    m.insert(MT_STR_KEY_HOME, "Home");
    m.insert(MT_STR_KEY_END, "End");
    m.insert(MT_STR_KEY_PRINT, "Print Screen");
    m.insert(MT_STR_KEY_SCROLL_LOCK, "Scroll Lock");
    m.insert(MT_STR_KEY_PAUSE, "Pause");
    m.insert(MT_STR_KEY_NONE, "<None>");
    m.insert(MT_STR_KEY_RESERVED, "<Reserved>");
    m
}

impl robin_engine::sherwood_stat::MenuTextLookup for MenuText {
    fn get(&self, id: usize) -> String {
        MenuText::get(self, id)
    }
}

// ═══════════════════════════════════════════════════════════════════
// Menu surface handle
// ═══════════════════════════════════════════════════════════════════

/// A loaded picture surface along with its source-image dimensions.
#[derive(Copy, Clone, Debug)]
pub struct MenuSurface {
    pub id: u32,
    pub width: i32,
    pub height: i32,
}

// ═══════════════════════════════════════════════════════════════════
// Fonts used by the in-game menus
// ═══════════════════════════════════════════════════════════════════

/// Cached menu fonts loaded via `manager.cfg`.
#[derive(Default)]
pub struct MenuFonts {
    pub menu_button_enabled: Option<NativeFont>,
    pub menu_button_disabled: Option<NativeFont>,
    pub mission_title: Option<NativeFont>,
    pub popup_scroll: Option<NativeFont>,
    pub default: Option<NativeFont>,
    pub edit_field: Option<NativeFont>,
    pub menu_text: Option<NativeFont>,
    pub debrief: Option<NativeFont>,
    pub active_short_briefing: Option<NativeFont>,
    pub inactive_short_briefing: Option<NativeFont>,
    pub mission_title_any: Option<Font>,
    pub popup_scroll_any: Option<Font>,
    pub default_any: Option<Font>,
    pub list_default: Option<Font>,
    pub list_focused: Option<Font>,
    pub list_selected: Option<Font>,
    pub list_fallback: Option<Font>,
}

impl MenuFonts {
    pub fn load() -> Self {
        let config = match native_font::load_font_config() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Menu fonts: config load failed: {e}");
                return Self::default();
            }
        };
        // legacy implementation keeps menu fonts behind `SBFont`; cache the Rust `Font` enum
        // for the same native/TrueType split. Native-only legacy accessors
        // below still expose bitmap fonts to callers that have not moved to
        // the polymorphic text helpers.
        let load_any = |name: &str| match native_font::load_font_by_name(&config, name) {
            Ok(font) if font.is_renderable() => Some(font),
            Ok(native_font::Font::TrueType(tt)) => {
                tracing::info!(
                    "Menu font '{name}' resolved to unusable TrueType '{}' (missing face); falling back where possible",
                    tt.truetype_name_str()
                );
                None
            }
            Ok(native_font::Font::Native(_)) => None,
            Err(e) => {
                tracing::info!("Menu font '{name}' missing: {e}");
                None
            }
        };
        let load_native = |name: &str| {
            let font = load_any(name)?;
            match font {
                Font::Native(native) => Some(native),
                Font::TrueType(tt) => {
                    tracing::info!(
                        "Menu font '{name}' resolved to TrueType '{}'; native-only callers will use fallbacks",
                        tt.truetype_name_str()
                    );
                    None
                }
            }
        };
        Self {
            menu_button_enabled: load_native("MenuButtonEnabled"),
            menu_button_disabled: load_native("MenuButtonDisabled"),
            mission_title: load_native("MissionTitle"),
            popup_scroll: load_native("PopupScroll"),
            default: load_native("Default"),
            edit_field: load_native("EditField"),
            menu_text: load_native("MenuText"),
            debrief: load_native("Debrief"),
            active_short_briefing: load_native("ActiveShortBriefing"),
            inactive_short_briefing: load_native("InactiveShortBriefing"),
            mission_title_any: load_any("MissionTitle"),
            popup_scroll_any: load_any("PopupScroll"),
            default_any: load_any("Default"),
            list_default: load_any("ListDefault"),
            list_focused: load_any("ListFocused"),
            list_selected: load_any("ListSelected"),
            list_fallback: match native_font::load_font_by_name(&config, "Default") {
                Ok(font) if font.is_renderable() => Some(font),
                Ok(_) => None,
                Err(_) => None,
            },
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Shared ingame menu resources
// ═══════════════════════════════════════════════════════════════════

/// Resources shared by every in-game menu.  Loaded once per mission from
/// the game session loop and reused until the mission ends.
pub struct IngameMenuResources {
    /// The DEFAULT.RES manager, also used to lazily load portraits.
    pub(crate) res: ResourceManager,

    // ── Window / widget sprites ─────────────────────────────────────
    pub button_surfaces: Vec<Option<u32>>,
    pub button_w: i32,
    pub button_h: i32,
    /// `RHID_OK` (= 145) — the small round "V" seal button used by
    /// popup scroll, mission description, dialogue Skip, etc.  Has no
    /// label and a dedicated sprite pack, distinct from
    /// `RHID_MENU_BUTTON` (= 190) which is the wide rectangular menu
    /// button.
    pub ok_button_surfaces: Vec<Option<u32>>,
    pub ok_button_w: i32,
    pub ok_button_h: i32,
    /// `RHID_CANCEL` (= 146) — the small round "X" seal button paired
    /// with `RHID_OK` in the buy-blazons / mission-description Cancel /
    /// Quit widgets.
    pub cancel_button_surfaces: Vec<Option<u32>>,
    pub cancel_button_w: i32,
    pub cancel_button_h: i32,
    pub parchment_huge: Option<MenuSurface>,
    pub menu_bg_small: Option<MenuSurface>,
    pub menu_bg: [Option<MenuSurface>; 4],
    /// Radio / toggle button background (`RHID_MENU_INPUT_FIELD`).
    pub input_field: Vec<Option<u32>>,
    pub input_field_w: i32,
    pub input_field_h: i32,
    /// Radio/toggle button sprite pack (`RHID_RADIO`).
    pub radio_surfaces: Vec<Option<u32>>,
    pub radio_w: i32,
    pub radio_h: i32,
    /// Slider sprite frames (`RHID_SLIDER`).
    pub slider_frames: Vec<Option<u32>>,
    pub slider_w: i32,
    pub slider_h: i32,
    /// Listbox frame (`RHID_MENU_LIST_BOX`).
    pub list_box: Option<MenuSurface>,
    /// Listbox scrollbar 3-slice sprites from `RHID_MENU_LIST_BOX`
    /// sub-resources 0-5:
    ///   0 = track start (top cap)
    ///   1 = track fill (tiled)
    ///   2 = track end (bottom cap)
    ///   3 = thumb start
    ///   4 = thumb fill
    ///   5 = thumb end
    pub list_scrollbar: [Option<MenuSurface>; 6],
    /// Separator bar for the short briefings list (`RHID_SEPARATOR`).
    pub separator: Option<MenuSurface>,
    /// Yes/No checkmark icon (`RHID_YES_NO`).
    pub check_mark: Option<MenuSurface>,
    /// `RHID_BLAZON_TINY` sub-pictures indexed by
    /// [`robin_engine::widget_state::blazon_set::EMPTY_BLAZON_SUB`] etc.
    /// Used by the blazon-set grid when the layout box is too narrow /
    /// short for huge icons.
    pub blazon_tiny: [Option<MenuSurface>; 3],
    /// `RHID_BLAZON_HUGE` sub-pictures.  Used by the blazon-set grid
    /// in the pre-mission description modal.
    pub blazon_huge: [Option<MenuSurface>; 3],

    // ── Fonts and text ──────────────────────────────────────────────
    pub fonts: MenuFonts,
    pub menu_text: MenuText,

    // ── Lazily loaded portraits (RHID_DLG_*) ───────────────────────
    portrait_cache: HashMap<i32, MenuSurface>,
}

impl IngameMenuResources {
    /// Attempt to load all shared menu resources.  Returns `None` if
    /// `Data/Interface/DEFAULT.RES` cannot be opened at all.
    pub fn new(
        renderer: &mut Renderer,
        shipping: Option<&robin_assets::shipping_datadir::ShippingDatadir>,
    ) -> Option<Self> {
        let mut res = ResourceManager::new();
        if let Err(e) = res.attach_or_from_shipping("Data/Interface/DEFAULT.RES", shipping) {
            tracing::warn!("Ingame menu resources: DEFAULT.RES unavailable: {e}");
            return None;
        }

        // `Data/Text/Level.res` is already attached by the game session
        // loop for portrait names; it also contains the menu text string
        // table.  Attach it here on a scratch manager if it's missing so
        // `MenuText::load` has a table to read.
        let mut text_res = ResourceManager::new();
        let _ = text_res.attach_or_from_shipping("Data/Text/Level.res", shipping);
        let _ = text_res.attach_or_from_shipping("Data/Interface/Start.sxt", shipping);
        let menu_text = MenuText::load(&mut text_res);

        let (button_w, button_h, button_surfaces) =
            load_sprite_pack(&mut res, renderer, resource_ids::RHID_MENU_BUTTON);
        let (ok_button_w, ok_button_h, ok_button_surfaces) =
            load_sprite_pack(&mut res, renderer, resource_ids::RHID_OK);
        let (cancel_button_w, cancel_button_h, cancel_button_surfaces) =
            load_sprite_pack(&mut res, renderer, resource_ids::RHID_CANCEL);
        let (radio_w, radio_h, radio_surfaces) =
            load_sprite_pack(&mut res, renderer, resource_ids::RHID_RADIO);

        // Menu-button packs render with a 50% shadow intensity
        // (`MENU_BUTTON_SHADOW_ALPHA`).  Override the per-surface
        // shadow alpha so the GPU `BlendMode::Blend` matches that
        // intensity at draw time.  Other sprites stay at the default
        // 40% from `FrameHolder::global_shadow()`.
        for pack in [
            &button_surfaces,
            &ok_button_surfaces,
            &cancel_button_surfaces,
            &radio_surfaces,
        ] {
            for id in pack.iter().flatten() {
                renderer.set_shadow_alpha(*id, crate::renderer::MENU_BUTTON_SHADOW_ALPHA);
            }
        }

        let parchment_huge = load_surface(&mut res, renderer, resource_ids::RHID_PARCHMENT_HUGE);
        let menu_bg_small =
            load_surface(&mut res, renderer, resource_ids::RHID_MENU_BACKGROUND_SMALL);
        let menu_bg = [
            load_surface(&mut res, renderer, resource_ids::RHID_MENU_BACKGROUND_0),
            load_surface(&mut res, renderer, resource_ids::RHID_MENU_BACKGROUND_1),
            load_surface(&mut res, renderer, resource_ids::RHID_MENU_BACKGROUND_2),
            load_surface(&mut res, renderer, resource_ids::RHID_MENU_BACKGROUND_3),
        ];

        let (input_field_w, input_field_h, input_field) =
            load_sprite_pack(&mut res, renderer, resource_ids::RHID_MENU_INPUT_FIELD);
        let (slider_w, slider_h, slider_frames) =
            load_sprite_pack(&mut res, renderer, resource_ids::RHID_SLIDER);
        let list_box = load_surface(&mut res, renderer, resource_ids::RHID_MENU_LIST_BOX);
        let list_scrollbar = [
            load_surface_sub(&mut res, renderer, resource_ids::RHID_MENU_LIST_BOX, 0),
            load_surface_sub(&mut res, renderer, resource_ids::RHID_MENU_LIST_BOX, 1),
            load_surface_sub(&mut res, renderer, resource_ids::RHID_MENU_LIST_BOX, 2),
            load_surface_sub(&mut res, renderer, resource_ids::RHID_MENU_LIST_BOX, 3),
            load_surface_sub(&mut res, renderer, resource_ids::RHID_MENU_LIST_BOX, 4),
            load_surface_sub(&mut res, renderer, resource_ids::RHID_MENU_LIST_BOX, 5),
        ];
        let separator = load_surface(&mut res, renderer, resource_ids::RHID_SEPARATOR);
        let check_mark = load_surface(&mut res, renderer, 142 /* RHID_YES_NO */);

        // Blazon-set sprite packs.  Both resource IDs carry 3 sub-
        // pictures in the order: 0=empty, 1=normal (won),
        // 2=castle (to collect).
        let blazon_tiny = [
            load_surface_sub(&mut res, renderer, resource_ids::RHID_BLAZON_TINY, 0),
            load_surface_sub(&mut res, renderer, resource_ids::RHID_BLAZON_TINY, 1),
            load_surface_sub(&mut res, renderer, resource_ids::RHID_BLAZON_TINY, 2),
        ];
        let blazon_huge = [
            load_surface_sub(&mut res, renderer, resource_ids::RHID_BLAZON_HUGE, 0),
            load_surface_sub(&mut res, renderer, resource_ids::RHID_BLAZON_HUGE, 1),
            load_surface_sub(&mut res, renderer, resource_ids::RHID_BLAZON_HUGE, 2),
        ];

        let fonts = MenuFonts::load();

        tracing::info!(
            "Ingame menus: button={}x{} ({} frames), input_field={}x{}, parchment={:?}, menu_text={} entries",
            button_w,
            button_h,
            button_surfaces.len(),
            input_field_w,
            input_field_h,
            parchment_huge.is_some(),
            menu_text.strings.len(),
        );

        Some(Self {
            res,
            button_surfaces,
            button_w,
            button_h,
            ok_button_surfaces,
            ok_button_w,
            ok_button_h,
            cancel_button_surfaces,
            cancel_button_w,
            cancel_button_h,
            parchment_huge,
            menu_bg_small,
            menu_bg,
            input_field,
            input_field_w,
            input_field_h,
            radio_surfaces,
            radio_w,
            radio_h,
            slider_frames,
            slider_w,
            slider_h,
            list_box,
            list_scrollbar,
            separator,
            check_mark,
            blazon_tiny,
            blazon_huge,
            fonts,
            menu_text,
            portrait_cache: HashMap::new(),
        })
    }

    pub fn button_dimensions(&self) -> (i32, i32) {
        (self.button_w.max(128), self.button_h.max(25))
    }

    pub fn input_field_dimensions(&self) -> (i32, i32) {
        (self.input_field_w.max(80), self.input_field_h.max(20))
    }

    pub fn radio_dimensions(&self) -> (i32, i32) {
        if self.radio_w > 0 && self.radio_h > 0 {
            (self.radio_w, self.radio_h)
        } else {
            self.button_dimensions()
        }
    }

    pub fn button_surface(&self, state: usize) -> Option<u32> {
        self.button_surfaces
            .get(state)
            .copied()
            .flatten()
            .or_else(|| self.button_surfaces.first().copied().flatten())
    }

    /// Dimensions of the `RHID_OK` seal button.  Falls back to the
    /// rectangular menu-button size if the pack didn't load.
    pub fn ok_button_dimensions(&self) -> (i32, i32) {
        if self.ok_button_w > 0 && self.ok_button_h > 0 {
            (self.ok_button_w, self.ok_button_h)
        } else {
            self.button_dimensions()
        }
    }

    /// Sprite for a given state of the `RHID_OK` seal button.  Falls back
    /// to the rectangular menu-button sprite if this pack didn't load.
    pub fn ok_button_surface(&self, state: usize) -> Option<u32> {
        self.ok_button_surfaces
            .get(state)
            .copied()
            .flatten()
            .or_else(|| self.ok_button_surfaces.first().copied().flatten())
            .or_else(|| self.button_surface(state))
    }

    /// Dimensions of the `RHID_CANCEL` seal button.  Falls back through
    /// `RHID_OK` then the rectangular menu-button size if the pack
    /// didn't load.
    pub fn cancel_button_dimensions(&self) -> (i32, i32) {
        if self.cancel_button_w > 0 && self.cancel_button_h > 0 {
            (self.cancel_button_w, self.cancel_button_h)
        } else {
            self.ok_button_dimensions()
        }
    }

    /// Sprite for a given state of the `RHID_CANCEL` seal button.  Falls
    /// back through `RHID_OK` then the rectangular menu-button sprite.
    pub fn cancel_button_surface(&self, state: usize) -> Option<u32> {
        self.cancel_button_surfaces
            .get(state)
            .copied()
            .flatten()
            .or_else(|| self.cancel_button_surfaces.first().copied().flatten())
            .or_else(|| self.ok_button_surface(state))
    }

    pub fn input_field_surface(&self, selected: bool) -> Option<u32> {
        let idx = if selected { 1 } else { 0 };
        self.input_field
            .get(idx)
            .copied()
            .flatten()
            .or_else(|| self.input_field.first().copied().flatten())
    }

    pub fn input_field_selected_surface(&self) -> Option<u32> {
        self.input_field
            .get(3)
            .copied()
            .flatten()
            .or_else(|| self.input_field_surface(true))
    }

    pub fn radio_surface(&self, state: usize) -> Option<u32> {
        self.radio_surfaces
            .get(state)
            .copied()
            .flatten()
            .or_else(|| self.radio_surfaces.first().copied().flatten())
            .or_else(|| self.button_surface(state))
    }

    fn first_native<'a>(
        fonts: impl IntoIterator<Item = Option<&'a NativeFont>>,
    ) -> Option<&'a NativeFont> {
        fonts.into_iter().flatten().next()
    }

    pub fn menu_button_font(&self, enabled: bool) -> Option<&NativeFont> {
        if enabled {
            Self::first_native([
                self.fonts.menu_button_enabled.as_ref(),
                self.fonts.menu_button_disabled.as_ref(),
            ])
        } else {
            Self::first_native([
                self.fonts.menu_button_disabled.as_ref(),
                self.fonts.menu_button_enabled.as_ref(),
            ])
        }
    }

    pub fn title_font_any(&self) -> Option<&Font> {
        self.fonts
            .mission_title_any
            .as_ref()
            .or(self.fonts.popup_scroll_any.as_ref())
            .or(self.fonts.default_any.as_ref())
    }

    pub fn title_font(&self) -> Option<&NativeFont> {
        Self::first_native([
            self.fonts.mission_title.as_ref(),
            self.fonts.popup_scroll.as_ref(),
            self.fonts.menu_button_enabled.as_ref(),
        ])
    }

    pub fn popup_font_any(&self) -> Option<&Font> {
        self.fonts
            .popup_scroll_any
            .as_ref()
            .or(self.fonts.default_any.as_ref())
    }

    /// Body font for generic modal dialogs (YesNo, mission state popup).
    pub fn popup_font(&self) -> Option<&NativeFont> {
        Self::first_native([
            self.fonts.popup_scroll.as_ref(),
            self.fonts.default.as_ref(),
            self.fonts.menu_button_enabled.as_ref(),
        ])
    }

    /// Debriefing body text font (falls back to popup scroll).
    pub fn debrief_font(&self) -> Option<&NativeFont> {
        Self::first_native([
            self.fonts.debrief.as_ref(),
            self.fonts.popup_scroll.as_ref(),
            self.fonts.default.as_ref(),
        ])
    }

    /// Resolve a font by its `font.cfg` name (`"Debrief"`,
    /// `"PopupScroll"`, …).  Used when a popup-scroll caller passes an
    /// explicit font name.  Returns `None` for unknown names so the
    /// caller can fall back to the default popup font.
    pub fn font_by_name(&self, name: &str) -> Option<&NativeFont> {
        match name {
            "MenuButtonEnabled" => self.fonts.menu_button_enabled.as_ref(),
            "MenuButtonDisabled" => self.fonts.menu_button_disabled.as_ref(),
            "MissionTitle" => self.fonts.mission_title.as_ref(),
            "PopupScroll" => self.fonts.popup_scroll.as_ref(),
            "Default" => self.fonts.default.as_ref(),
            "EditField" => self.fonts.edit_field.as_ref(),
            "MenuText" => self.fonts.menu_text.as_ref(),
            "Debrief" => self.fonts.debrief.as_ref(),
            "ActiveShortBriefing" => self.fonts.active_short_briefing.as_ref(),
            "InactiveShortBriefing" => self.fonts.inactive_short_briefing.as_ref(),
            _ => None,
        }
    }

    /// Regular label font (Options hub, labels on Graphics/Sounds).
    pub fn label_font(&self) -> Option<&NativeFont> {
        Self::first_native([
            self.fonts.default.as_ref(),
            self.fonts.popup_scroll.as_ref(),
        ])
    }

    /// Profile-info / sidebar text font (`MenuText` in `font.cfg`),
    /// distinct from the "Default" font used elsewhere.
    pub fn menu_text_font(&self) -> Option<&NativeFont> {
        Self::first_native([
            self.fonts.menu_text.as_ref(),
            self.fonts.default.as_ref(),
            self.fonts.popup_scroll.as_ref(),
        ])
    }

    /// Font used for large radio/toggle button labels.
    pub fn edit_field_font(&self) -> Option<&NativeFont> {
        Self::first_native([
            self.fonts.edit_field.as_ref(),
            self.fonts.menu_button_enabled.as_ref(),
            self.fonts.default.as_ref(),
        ])
    }

    pub fn active_briefing_font(&self) -> Option<&NativeFont> {
        Self::first_native([
            self.fonts.active_short_briefing.as_ref(),
            self.fonts.popup_scroll.as_ref(),
            self.fonts.default.as_ref(),
        ])
    }

    pub fn inactive_briefing_font(&self) -> Option<&NativeFont> {
        Self::first_native([
            self.fonts.inactive_short_briefing.as_ref(),
            self.fonts.active_short_briefing.as_ref(),
            self.fonts.default.as_ref(),
        ])
    }

    /// Pick the list-row font based on focus/selection state.
    ///
    /// `list_*` fonts are TrueType-only in the shipping config, so this
    /// returns a `Font` enum. List renderers go through
    /// `render_text_virt_font` / `render_text_screen_font`, which
    /// dispatch to `Renderer::render_text_truetype` for the TT variant
    /// (per-string ARGB rasterisation via `ab_glyph`).
    pub fn list_font(&self, focused: bool, selected: bool) -> Option<&Font> {
        if selected {
            self.fonts
                .list_selected
                .as_ref()
                .or(self.fonts.list_focused.as_ref())
                .or(self.fonts.list_default.as_ref())
        } else if focused {
            self.fonts
                .list_focused
                .as_ref()
                .or(self.fonts.list_default.as_ref())
        } else {
            self.fonts.list_default.as_ref()
        }
        .or(self.fonts.list_fallback.as_ref())
    }

    /// `list_font()` as a `NativeFont`, falling back to `default` when
    /// the list font resolves to TrueType.
    ///
    /// Prefer [`Self::list_font`] + the `Font`-polymorphic layout
    /// helpers — that path actually rasterises the configured TrueType
    /// font instead of substituting the bitmap default. This narrowed
    /// view exists for a couple of remaining bitmap-only callers
    /// (`ui.rs` widget shims) that haven't been migrated yet.
    pub fn list_font_native(&self, focused: bool, selected: bool) -> Option<&NativeFont> {
        self.list_font(focused, selected)
            .and_then(|f| f.as_native())
            .or(self.fonts.default.as_ref())
    }

    /// 6-state list font lookup (state × normal/alternate style):
    /// alternate rows get a distinct style, and within each style the
    /// row's focus/selection state picks the variant.
    ///
    /// The shipping `manager.cfg` only populates the three "normal"
    /// fonts (`ListDefault` / `ListFocused` / `ListSelected`); no
    /// alternate variants are loaded.  The alternate branches fall
    /// through to the normal-style fonts when no alternate exists.
    pub fn list_font_native_with_style(
        &self,
        focused: bool,
        selected: bool,
        alternate: bool,
    ) -> Option<&NativeFont> {
        // Alternate fonts aren't loaded today (see note above).  Once
        // `manager.cfg` carries `ListDefaultAlternate` / etc., swap the
        // alternate arm to pull those via `MenuFonts` directly.
        let _ = alternate;
        self.list_font_native(focused, selected)
    }

    /// 6-state list font lookup that preserves the `Font` enum so
    /// callers can render via either the native bitmap or the TrueType
    /// path. Same alternate-fallback logic as
    /// [`Self::list_font_native_with_style`].
    pub fn list_font_with_style(
        &self,
        focused: bool,
        selected: bool,
        alternate: bool,
    ) -> Option<&Font> {
        let _ = alternate;
        self.list_font(focused, selected)
    }

    /// Test-only constructor: build a resources struct with empty
    /// surfaces, a default fallback menu text table, and no loaded
    /// fonts.  Used by tests in sibling modules that only exercise
    /// pure state-machine logic (button layouts, keyboard navigation).
    #[cfg(test)]
    pub(super) fn stub() -> Self {
        Self {
            res: ResourceManager::new(),
            button_surfaces: Vec::new(),
            button_w: 128,
            button_h: 25,
            ok_button_surfaces: Vec::new(),
            ok_button_w: 0,
            ok_button_h: 0,
            cancel_button_surfaces: Vec::new(),
            cancel_button_w: 0,
            cancel_button_h: 0,
            parchment_huge: None,
            menu_bg_small: None,
            menu_bg: [None, None, None, None],
            input_field: Vec::new(),
            input_field_w: 0,
            input_field_h: 0,
            radio_surfaces: Vec::new(),
            radio_w: 0,
            radio_h: 0,
            slider_frames: Vec::new(),
            slider_w: 0,
            slider_h: 0,
            list_box: None,
            list_scrollbar: [None, None, None, None, None, None],
            separator: None,
            check_mark: None,
            blazon_tiny: [None, None, None],
            blazon_huge: [None, None, None],
            fonts: MenuFonts::default(),
            menu_text: MenuText {
                strings: Vec::new(),
                fallbacks: default_fallbacks(),
            },
            portrait_cache: HashMap::new(),
        }
    }

    /// Load a dialogue portrait sprite, caching it on first access.
    pub fn portrait(&mut self, renderer: &mut Renderer, id: i32) -> Option<MenuSurface> {
        if let Some(s) = self.portrait_cache.get(&id) {
            return Some(*s);
        }
        let surf = load_surface(&mut self.res, renderer, id)?;
        self.portrait_cache.insert(id, surf);
        Some(surf)
    }

    /// Load a picture from the caller-supplied resource manager (e.g.
    /// `Data/Text/Level.res` for per-level popup-scroll pictures that
    /// aren't in `DEFAULT.RES`) and cache the resulting surface on the
    /// shared `portrait_cache`.  Falls back to the local res first so a
    /// miss in the level file still finds generic assets like
    /// `RHID_DEFAULT_POPUP_SCROLL_PICTURE` that live in `DEFAULT.RES`.
    pub fn picture_from(
        &mut self,
        renderer: &mut Renderer,
        external: &mut ResourceManager,
        id: i32,
    ) -> Option<MenuSurface> {
        // A picture id of 0 means "no picture widget", so a popup-text
        // entry whose picture id is intentionally 0 renders picture-
        // less; the early-out also keeps `portrait_cache` from being
        // poisoned with a stray 0-key entry.
        if id <= 0 {
            return None;
        }
        if let Some(s) = self.portrait_cache.get(&id) {
            return Some(*s);
        }
        if let Some(surf) = load_surface(&mut self.res, renderer, id) {
            self.portrait_cache.insert(id, surf);
            return Some(surf);
        }
        let surf = load_surface(external, renderer, id)?;
        self.portrait_cache.insert(id, surf);
        Some(surf)
    }

    /// Load a DEFAULT.RES picture by resource id and cache the uploaded
    /// renderer surface. Used by small modal screens that need resource
    /// sprites not preloaded by the shared menu cache.
    pub fn default_picture(&mut self, renderer: &mut Renderer, id: i32) -> Option<MenuSurface> {
        if let Some(s) = self.portrait_cache.get(&id) {
            return Some(*s);
        }
        let surf = load_surface(&mut self.res, renderer, id)?;
        self.portrait_cache.insert(id, surf);
        Some(surf)
    }

    /// Load a specific DEFAULT.RES sub-picture and cache it separately
    /// from sub-picture 0.
    pub fn default_picture_sub(
        &mut self,
        renderer: &mut Renderer,
        id: i32,
        sub_id: usize,
    ) -> Option<MenuSurface> {
        let key = id.saturating_mul(1000).saturating_add(sub_id as i32);
        if let Some(s) = self.portrait_cache.get(&key) {
            return Some(*s);
        }
        let surf = load_surface_sub(&mut self.res, renderer, id, sub_id)?;
        self.portrait_cache.insert(key, surf);
        Some(surf)
    }
}

// ═══════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════

fn load_surface(
    res: &mut ResourceManager,
    renderer: &mut Renderer,
    id: i32,
) -> Option<MenuSurface> {
    // Use sub-picture 0's own dimensions — `get_dimension` returns the
    // MAX width/height across every sub-picture of the resource, which
    // can be larger than frame 0 when the resource packs multiple state
    // variants at different sizes.  Using the max stretched popup-scroll
    // pictures taller than they should be.
    let pic = res.get_picture(id, 0).ok()?;
    let width = pic.width as i32;
    let height = pic.height as i32;
    let surface_id = picture_to_surface(renderer, pic);
    Some(MenuSurface {
        id: surface_id,
        width,
        height,
    })
}

/// Load a specific sub-picture (by index) as its own [`MenuSurface`].
/// Used for composite 3-slice sprites like the listbox scrollbar where
/// each slice has its own source dimensions.
fn load_surface_sub(
    res: &mut ResourceManager,
    renderer: &mut Renderer,
    id: i32,
    sub_id: usize,
) -> Option<MenuSurface> {
    let pic = res.get_picture(id, sub_id).ok()?;
    let width = pic.width as i32;
    let height = pic.height as i32;
    let surface_id = picture_to_surface(renderer, pic);
    Some(MenuSurface {
        id: surface_id,
        width,
        height,
    })
}

/// Load a multi-frame sprite pack and upload every frame as a surface.
fn load_sprite_pack(
    res: &mut ResourceManager,
    renderer: &mut Renderer,
    id: i32,
) -> (i32, i32, Vec<Option<u32>>) {
    let dims = res.get_dimension(id).ok();
    let surfaces: Vec<Option<u32>> = match res.get_pictures(id) {
        Ok(pics) => pics
            .iter()
            .map(|opt| opt.as_ref().map(|p| picture_to_surface(renderer, p)))
            .collect(),
        Err(_) => Vec::new(),
    };
    let (w, h) = dims.map(|(w, h)| (w as i32, h as i32)).unwrap_or((0, 0));
    (w, h, surfaces)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_text_fallback_is_english() {
        let text = MenuText::default();
        assert_eq!(text.get(MT_BTN_OK), ""); // no fallback populated by default()

        let text = MenuText {
            strings: Vec::new(),
            fallbacks: default_fallbacks(),
        };
        assert_eq!(text.get(MT_BTN_OK), "OK");
        assert_eq!(text.get(MT_BTN_CONTINUE), "Continue");
        assert_eq!(text.get(MT_TTL_MISSION_WON), "Mission Won");
    }

    #[test]
    fn menu_text_prefers_resource_over_fallback() {
        let mut strings = vec![String::new(); 32];
        strings[MT_BTN_OK] = "Aceptar".to_string();
        let text = MenuText {
            strings,
            fallbacks: default_fallbacks(),
        };
        assert_eq!(text.get(MT_BTN_OK), "Aceptar");
    }

    #[test]
    fn menu_text_empty_resource_falls_through() {
        let strings = vec![String::new(); 32];
        let text = MenuText {
            strings,
            fallbacks: default_fallbacks(),
        };
        // Empty string in table — should fall back
        assert_eq!(text.get(MT_BTN_OK), "OK");
    }

    #[test]
    fn menu_title_preserves_truetype_for_polymorphic_callers() {
        let mut name = [0u8; 32];
        name[..12].copy_from_slice(b"MissionTitle");
        let mut tt_name = [0u8; 32];
        tt_name[..11].copy_from_slice(b"MissingFace");
        let tt = crate::font::TrueTypeFont::from_parts(&name, 14, 0, 0, &tt_name, 0x00FF_FFFF, &[]);
        let mut resources = IngameMenuResources::stub();
        resources.fonts.mission_title_any = Some(Font::TrueType(tt));

        assert!(matches!(
            resources.title_font_any(),
            Some(Font::TrueType(_))
        ));
        assert!(resources.title_font().is_none());
    }
}

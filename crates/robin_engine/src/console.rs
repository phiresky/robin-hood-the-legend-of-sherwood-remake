//! In-game debug console.
//!
//! Processes text commands for cheating/debugging (give money, teleport,
//! win mission, toggle display overlays, etc.).
//!
//! This module owns the parser and the `ConsoleCommand` enum.  Actual
//! command dispatch lives on the engine side in
//! `engine::console_dispatch`, which mutates engine/campaign state and
//! returns a human-readable response.

// ─── Command enum ───────────────────────────────────────────────

/// Every console command recognized by the game.
///
/// Commands that take arguments carry them inline. Commands from the
/// "final" cheat list (CASH, GOODLUCK, etc.) are parsed as aliases to
/// the same variant as their dev-mode equivalents.
#[derive(
    Debug,
    Clone,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum ConsoleCommand {
    // ── Campaign / mission flow ──
    GiveMoney {
        amount: u32,
        /// True when the user invoked `CASH`/`EZB` with no argument.
        /// In that case the dispatcher prints a four-line help listing
        /// (`Try also the following:`, `CASH CENT`, etc.) before
        /// applying the 1000-gold default.
        show_help: bool,
    },
    GiveBlazon {
        amount: u32,
    },
    GiveAmulets {
        amount: u32,
    },
    AddPeasant,
    WinMission,
    WinCampaign,
    LoseMission,
    CampaignReport,
    LoadCampaign {
        filename: String,
    },

    // ── Actor / AI operations ──
    Goldeneye,
    Elevation,
    BigBrother,
    BudSpencer,
    Nuke,
    Wakeup,
    Highlander,
    Highlander2,
    Honolulu,
    LastManStanding,
    DiesIrae,
    Freeze,
    StupidSoldiers,
    RoterAlarm,
    MisterSandman,
    Morpheus,
    Hades,
    Coma,
    Reinforcement,
    SanPetrus,
    WaspMaster,
    GiveArrows,
    GiveAmmo,
    Ubiquity,
    Lukas {
        pcs: Option<String>,
    },
    Call {
        actor: String,
        method: String,
    },

    // ── Display toggles ──
    Ai,
    Anim,
    Babylon,
    CestLaZone,
    Companies,
    Einstein,
    EnergyDisplay,
    Euler,
    Fps,
    Light,
    LevelText {
        option: Option<String>,
    },
    Motion,
    Noise,
    PcSight,
    Projection,
    Railroad,
    SeekAndDestroy,
    Shadow,
    Sphere,
    SpriteMasks,
    Surface,
    StatusFramecache,
    StatusHardware,
    StatusShadow,
    StatusPc,

    // ── Misc ──
    Help,
    AssertFalse,
    Forget,
    Sarkozy,
    Optimize,

    /// A known keyword that was invoked with the wrong number of
    /// arguments.  Carries the in-body "USAGE: …" / "Verboten …"
    /// help text (e.g. campaign with < 2 args).  The dispatcher turns
    /// this into a plain `Ok(msg)` so
    /// the overlay shows the help text instead of a generic
    /// "Unknown command" error.
    UsageError(String),
}

impl ConsoleCommand {
    /// Whether this command only changes host-owned developer presentation.
    ///
    /// These commands are dispatched by the host before frame admission and
    /// must never be placed in the deterministic external-action journal.
    pub fn is_host_only(&self) -> bool {
        matches!(
            self,
            Self::Elevation
                | Self::BigBrother
                | Self::Einstein
                | Self::EnergyDisplay
                | Self::Euler
                | Self::Fps
                | Self::Light
                | Self::LevelText { .. }
                | Self::Motion
                | Self::Noise
                | Self::PcSight
                | Self::Projection
                | Self::Railroad
                | Self::SeekAndDestroy
                | Self::Shadow
                | Self::Sphere
                | Self::SpriteMasks
                | Self::Surface
                | Self::Anim
                | Self::Companies
                | Self::CestLaZone
                | Self::StatusFramecache
                | Self::StatusHardware
                | Self::StatusShadow
                | Self::StatusPc
                | Self::Help
                | Self::AssertFalse
                | Self::Forget
                | Self::Sarkozy
                | Self::Optimize
                | Self::UsageError(_)
        )
    }
}

// ─── Console struct ─────────────────────────────────────────────

const HISTORY_SIZE: usize = 10;

#[derive(Debug, Clone)]
pub struct Console {
    pub history: Vec<String>,
    pub enabled: bool,
    /// When true, only the "final" (release) cheat set is available.
    /// Exposed as a runtime flag so developer tooling (HTTP API, debug
    /// overlays) can force-enable the dev cheat set even in a shipping
    /// build.  The parser, tab completion, and help text all honour
    /// this flag.
    pub use_final: bool,
    /// Extra output lines emitted by cheats during dispatch.  The
    /// overlay drains this every frame and appends each entry to the
    /// scrollback.  Cheat handlers use this side channel to emit
    /// multi-line diagnostics (e.g. `STATUS PC`, `STATUS HARDWARE`,
    /// `SAN PETRUS` epitaphs).
    pub pending_output: Vec<String>,
}

impl Default for Console {
    fn default() -> Self {
        Console {
            history: Vec::with_capacity(HISTORY_SIZE),
            enabled: false,
            use_final: false,
            pending_output: Vec::new(),
        }
    }
}

// ─── Parsing ────────────────────────────────────────────────────

/// Parse a console input string into a `ConsoleCommand`.
///
/// Input is case-insensitive (uppercased before matching).
/// Multi-word commands like "BIG BROTHER" are matched by checking the
/// first N tokens.
pub fn parse(input: &str) -> Option<ConsoleCommand> {
    parse_with_final(input, false)
}

/// Parse with the "final" flag controlling which command set is used.
pub fn parse_with_final(input: &str, use_final: bool) -> Option<ConsoleCommand> {
    let upper = input.trim().to_ascii_uppercase();
    let tokens: Vec<&str> = upper.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    // Final (release) cheats — a smaller set with different command strings
    if use_final {
        return parse_final(&tokens);
    }

    // Dev cheats — the full set
    parse_dev(&tokens)
}

fn parse_final(tokens: &[&str]) -> Option<ConsoleCommand> {
    match tokens[0] {
        "CASH" => Some(parse_money_args(&tokens[1..])),
        "GOODLUCK" => Some(parse_amulets_args(&tokens[1..])),
        "EINSTEIN" => Some(ConsoleCommand::Einstein),
        "IMMUNITY" => Some(ConsoleCommand::Highlander),
        "MERRYMAN" => Some(ConsoleCommand::AddPeasant),
        "PAM" => Some(ConsoleCommand::StupidSoldiers),
        "UNBLIP" => Some(ConsoleCommand::Ubiquity),
        "WINNER" => Some(ConsoleCommand::WinMission),
        "BINGO" => Some(ConsoleCommand::GiveAmmo),
        _ => None,
    }
}

fn parse_dev(tokens: &[&str]) -> Option<ConsoleCommand> {
    match tokens[0] {
        "AI" => Some(ConsoleCommand::Ai),
        "ALARM" => Some(ConsoleCommand::Reinforcement),
        "AMOR" => Some(ConsoleCommand::GiveArrows),
        "AMULETS" => Some(parse_amulets_args(&tokens[1..])),
        "ASSERTFALSE" => Some(ConsoleCommand::AssertFalse),
        "BABYLON" => Some(ConsoleCommand::Babylon),
        "BIG" if tokens.get(1) == Some(&"BROTHER") => Some(ConsoleCommand::BigBrother),
        "IDS" => Some(ConsoleCommand::BigBrother),
        "BUD" if tokens.get(1) == Some(&"SPENCER") => Some(ConsoleCommand::BudSpencer),
        "CALL" if tokens.len() >= 3 => Some(ConsoleCommand::Call {
            actor: tokens[1].to_string(),
            method: tokens[2].to_string(),
        }),
        "COMA" => Some(ConsoleCommand::Coma),
        "COMPANIES" => Some(ConsoleCommand::Companies),
        "CESTLAZONE" => Some(ConsoleCommand::CestLaZone),
        "CAMPAIGN" if tokens.len() >= 2 => Some(ConsoleCommand::LoadCampaign {
            filename: tokens[1].to_string(),
        }),
        "CAMPAIGN" => Some(ConsoleCommand::UsageError(
            "Verboten : Please enter a valid filename !".to_owned(),
        )),
        "DIES" if tokens.get(1) == Some(&"IRAE") => Some(ConsoleCommand::DiesIrae),
        "EINSTEIN" => Some(ConsoleCommand::Einstein),
        "ELEVATION" => Some(ConsoleCommand::Elevation),
        "EULER" => Some(ConsoleCommand::Euler),
        "EZB" => Some(parse_money_args(&tokens[1..])),
        "FPS" => Some(ConsoleCommand::Fps),
        "FORGET" => Some(ConsoleCommand::Forget),
        "FREEZE" => Some(ConsoleCommand::Freeze),
        "FULLHOUSE" => Some(ConsoleCommand::GiveAmmo),
        "GOLDENEYE" => Some(ConsoleCommand::Goldeneye),
        "HADES" => Some(ConsoleCommand::Hades),
        "HELP" => Some(ConsoleCommand::Help),
        "HIGHLANDER" => Some(ConsoleCommand::Highlander),
        "HIGHLANDER2" => Some(ConsoleCommand::Highlander2),
        "HONOLULU" => Some(ConsoleCommand::Honolulu),
        "I" if tokens.len() >= 4
            && tokens[1] == "AM"
            && tokens[2] == "THE"
            && tokens[3] == "WINNER" =>
        {
            Some(ConsoleCommand::WinCampaign)
        }
        "KOLKOZ" => Some(ConsoleCommand::AddPeasant),
        "LAST" if tokens.get(1) == Some(&"MAN") && tokens.get(2) == Some(&"STANDING") => {
            Some(ConsoleCommand::LastManStanding)
        }
        "LEVEL" if tokens.get(1) == Some(&"TEXT") => Some(ConsoleCommand::LevelText {
            option: tokens.get(2).map(|s| s.to_string()),
        }),
        "LIGHT" => Some(ConsoleCommand::Light),
        "LOOSE" => Some(ConsoleCommand::LoseMission),
        "LUKAS" => Some(ConsoleCommand::Lukas {
            pcs: tokens.get(1).map(|s| s.to_string()),
        }),
        "MISTER" if tokens.get(1) == Some(&"SANDMAN") => Some(ConsoleCommand::MisterSandman),
        "MORPHEUS" => Some(ConsoleCommand::Morpheus),
        "MOTION" => Some(ConsoleCommand::Motion),
        "NOISE" => Some(ConsoleCommand::Noise),
        "NUKE" => Some(ConsoleCommand::Nuke),
        "OPTIMIZE" => Some(ConsoleCommand::Optimize),
        "PAMELA" if tokens.get(1) == Some(&"ANDERSON") => Some(ConsoleCommand::StupidSoldiers),
        "PCSIGHT" => Some(ConsoleCommand::PcSight),
        "PROJECTION" => Some(ConsoleCommand::Projection),
        "RAILROAD" => Some(ConsoleCommand::Railroad),
        "REPORT" => Some(ConsoleCommand::CampaignReport),
        "ROTER" if tokens.get(1) == Some(&"ALARM") => Some(ConsoleCommand::RoterAlarm),
        "SAN" if tokens.get(1) == Some(&"PETRUS") => Some(ConsoleCommand::SanPetrus),
        "SARKOZY" => Some(ConsoleCommand::Sarkozy),
        "SEEKANDDESTROY" => Some(ConsoleCommand::SeekAndDestroy),
        "SHADOW" => Some(ConsoleCommand::Shadow),
        "SPHERE" => Some(ConsoleCommand::Sphere),
        "SPRITEMASKS" | "SPRITE_MASKS" => Some(ConsoleCommand::SpriteMasks),
        "SPRITE" if tokens.get(1) == Some(&"MASKS") => Some(ConsoleCommand::SpriteMasks),
        "SURFACE" => Some(ConsoleCommand::Surface),
        "STATUS" => match tokens.get(1).copied() {
            Some("FRAMECACHE") => Some(ConsoleCommand::StatusFramecache),
            Some("HARDWARE") => Some(ConsoleCommand::StatusHardware),
            Some("SHADOW") => Some(ConsoleCommand::StatusShadow),
            Some("PC") => Some(ConsoleCommand::StatusPc),
            _ => None,
        },
        "UBIQUITY" => Some(ConsoleCommand::Ubiquity),
        "ANIM" => Some(ConsoleCommand::Anim),
        "WAKEUP" => Some(ConsoleCommand::Wakeup),
        "WASP" if tokens.get(1) == Some(&"MASTER") => Some(ConsoleCommand::WaspMaster),
        "WAPPEN" => Some(parse_blazon_args(&tokens[1..])),
        "WIN" => Some(ConsoleCommand::WinMission),
        _ => None,
    }
}

fn parse_money_args(args: &[&str]) -> ConsoleCommand {
    let (amount, show_help) = if let Some(&arg) = args.first() {
        let amount = match arg {
            "HUNDRED" => 100,
            "THOUSAND" => 1000,
            "TENTHOUSAND" => 10_000,
            "HUNDREDTHOUSAND" => 100_000,
            _ => arg.parse::<u32>().unwrap_or(1000),
        };
        (amount, false)
    } else {
        // No-arg default + help-text-emitting branch.
        (1000, true)
    };
    ConsoleCommand::GiveMoney { amount, show_help }
}

fn parse_amulets_args(args: &[&str]) -> ConsoleCommand {
    let amount = args
        .first()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(100);
    ConsoleCommand::GiveAmulets { amount }
}

fn parse_blazon_args(args: &[&str]) -> ConsoleCommand {
    let amount = args
        .first()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1);
    ConsoleCommand::GiveBlazon { amount }
}

impl Console {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a line to the command history (ring buffer, max HISTORY_SIZE).
    pub fn push_history(&mut self, line: &str) {
        if self.history.len() >= HISTORY_SIZE {
            self.history.remove(0);
        }
        self.history.push(line.to_string());
    }

    /// Append a line to the pending output queue.  Dispatchers use this
    /// when a single cheat needs to emit many lines.  The overlay
    /// drains the queue every frame and appends each entry to its
    /// scrollback.
    pub fn push_output(&mut self, line: impl Into<String>) {
        self.pending_output.push(line.into());
    }

    /// Take ownership of every queued output line, leaving the queue
    /// empty.  Callers (the overlay renderer, the HTTP snapshot) are
    /// responsible for presenting the drained lines.
    pub fn drain_output(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_output)
    }
}

// ─── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_commands() {
        assert_eq!(
            parse("EZB THOUSAND"),
            Some(ConsoleCommand::GiveMoney {
                amount: 1000,
                show_help: false
            })
        );
        assert_eq!(
            parse("ezb thousand"),
            Some(ConsoleCommand::GiveMoney {
                amount: 1000,
                show_help: false
            })
        );
        assert_eq!(
            parse("EZB 500"),
            Some(ConsoleCommand::GiveMoney {
                amount: 500,
                show_help: false
            })
        );
        assert_eq!(
            parse("EZB"),
            Some(ConsoleCommand::GiveMoney {
                amount: 1000,
                show_help: true
            })
        );
        assert_eq!(parse("HELP"), Some(ConsoleCommand::Help));
        assert_eq!(parse("WIN"), Some(ConsoleCommand::WinMission));
        assert_eq!(parse("LOOSE"), Some(ConsoleCommand::LoseMission));
        assert_eq!(parse("NUKE"), Some(ConsoleCommand::Nuke));
        assert_eq!(parse("HIGHLANDER"), Some(ConsoleCommand::Highlander));
        assert_eq!(parse("HIGHLANDER2"), Some(ConsoleCommand::Highlander2));
        assert_eq!(parse("GOLDENEYE"), Some(ConsoleCommand::Goldeneye));
        assert_eq!(parse("FREEZE"), Some(ConsoleCommand::Freeze));
        assert_eq!(parse("FULLHOUSE"), Some(ConsoleCommand::GiveAmmo));
        assert_eq!(parse("IDS"), Some(ConsoleCommand::BigBrother));
    }

    #[test]
    fn parse_multi_word_commands() {
        assert_eq!(parse("BIG BROTHER"), Some(ConsoleCommand::BigBrother));
        assert_eq!(parse("BUD SPENCER"), Some(ConsoleCommand::BudSpencer));
        assert_eq!(parse("DIES IRAE"), Some(ConsoleCommand::DiesIrae));
        assert_eq!(
            parse("LAST MAN STANDING"),
            Some(ConsoleCommand::LastManStanding)
        );
        assert_eq!(parse("I AM THE WINNER"), Some(ConsoleCommand::WinCampaign));
        assert_eq!(parse("ROTER ALARM"), Some(ConsoleCommand::RoterAlarm));
        assert_eq!(parse("SAN PETRUS"), Some(ConsoleCommand::SanPetrus));
        assert_eq!(parse("SPRITE MASKS"), Some(ConsoleCommand::SpriteMasks));
        assert_eq!(parse("WASP MASTER"), Some(ConsoleCommand::WaspMaster));
        assert_eq!(parse("MISTER SANDMAN"), Some(ConsoleCommand::MisterSandman));
        assert_eq!(
            parse("PAMELA ANDERSON"),
            Some(ConsoleCommand::StupidSoldiers)
        );
        assert_eq!(
            parse("STATUS FRAMECACHE"),
            Some(ConsoleCommand::StatusFramecache)
        );
        assert_eq!(
            parse("STATUS HARDWARE"),
            Some(ConsoleCommand::StatusHardware)
        );
        assert_eq!(
            parse("LEVEL TEXT"),
            Some(ConsoleCommand::LevelText { option: None })
        );
        assert_eq!(
            parse("LEVEL TEXT DG"),
            Some(ConsoleCommand::LevelText {
                option: Some("DG".to_string())
            })
        );
    }

    #[test]
    fn parse_final_cheats() {
        assert_eq!(
            parse_with_final("CASH", true),
            Some(ConsoleCommand::GiveMoney {
                amount: 1000,
                show_help: true
            })
        );
        assert_eq!(
            parse_with_final("CASH THOUSAND", true),
            Some(ConsoleCommand::GiveMoney {
                amount: 1000,
                show_help: false
            })
        );
        assert_eq!(
            parse_with_final("GOODLUCK", true),
            Some(ConsoleCommand::GiveAmulets { amount: 100 })
        );
        assert_eq!(
            parse_with_final("IMMUNITY", true),
            Some(ConsoleCommand::Highlander)
        );
        assert_eq!(
            parse_with_final("WINNER", true),
            Some(ConsoleCommand::WinMission)
        );
        assert_eq!(
            parse_with_final("BINGO", true),
            Some(ConsoleCommand::GiveAmmo)
        );
        assert_eq!(
            parse_with_final("MERRYMAN", true),
            Some(ConsoleCommand::AddPeasant)
        );
        assert_eq!(
            parse_with_final("PAM", true),
            Some(ConsoleCommand::StupidSoldiers)
        );
        assert_eq!(
            parse_with_final("UNBLIP", true),
            Some(ConsoleCommand::Ubiquity)
        );
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(parse("XYZZY"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("   "), None);
    }

    #[test]
    fn parse_amulets_with_amount() {
        assert_eq!(
            parse("AMULETS 50"),
            Some(ConsoleCommand::GiveAmulets { amount: 50 })
        );
        assert_eq!(
            parse("AMULETS"),
            Some(ConsoleCommand::GiveAmulets { amount: 100 })
        );
    }

    #[test]
    fn parse_blazon() {
        assert_eq!(
            parse("WAPPEN 5"),
            Some(ConsoleCommand::GiveBlazon { amount: 5 })
        );
        assert_eq!(
            parse("WAPPEN"),
            Some(ConsoleCommand::GiveBlazon { amount: 1 })
        );
    }

    #[test]
    fn console_history_ring_buffer() {
        let mut console = Console::new();
        for i in 0..15 {
            console.push_history(&format!("EZB {}", i));
        }
        assert_eq!(console.history.len(), HISTORY_SIZE);
        // Oldest entries should have been dropped
        assert_eq!(console.history[0], "EZB 5");
    }

    #[test]
    fn parse_call_command() {
        assert_eq!(
            parse("CALL ABC123 HIDEINTERFACE"),
            Some(ConsoleCommand::Call {
                actor: "ABC123".to_string(),
                method: "HIDEINTERFACE".to_string(),
            })
        );
    }

    #[test]
    fn parse_campaign_missing_filename_emits_verboten() {
        // CAMPAIGN with no filename emits the "Verboten : …" usage string.
        assert_eq!(
            parse("CAMPAIGN"),
            Some(ConsoleCommand::UsageError(
                "Verboten : Please enter a valid filename !".to_owned()
            ))
        );
    }

    #[test]
    fn parse_lukas() {
        assert_eq!(parse("LUKAS"), Some(ConsoleCommand::Lukas { pcs: None }));
        assert_eq!(
            parse("LUKAS RJTS"),
            Some(ConsoleCommand::Lukas {
                pcs: Some("RJTS".to_string())
            })
        );
    }
}

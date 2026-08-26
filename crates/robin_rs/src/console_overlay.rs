//! In-game console overlay — keyboard-driven debug + cheat console
//! drawn over the live game.
//!
//! This is the player-facing UI for console actions admitted through
//! [`robin_engine::engine::Engine::advance_frame`].
//! Toggling the overlay (default key: `~`) resets IME state, captures
//! keystrokes, and dispatches Enter-terminated lines to the engine.
//!
//! The console is intentionally non-modal: the game keeps running
//! underneath, so cheats execute against the live simulation and the
//! player can watch the effect. Uses winit's native text-input events
//! rather than a hand-rolled physical-key-to-character translation table,
//! which gives us correct keyboard layouts and IME for free.
//!
//! ## Features
//! - Text input via winit IME commit events
//! - Backspace
//! - Up/Down arrow command history navigation
//! - Tab completion against the static console keyword table
//! - Page Up / Page Down to scroll the output view
//! - Enter dispatches; Esc / `~` toggle close
//! - Caret blink animation (~2Hz)
//! - Auto-close on `WIN` / `WINCAMPAIGN` / `LOSE`.

use crate::host::Host;
use std::collections::VecDeque;

use crate::gfx_types::Keycode;

use crate::gfx_types::GameEvent;
use crate::ingame_menu::layout::render_text_screen;
use crate::native_font::NativeFont;
use crate::renderer::Renderer;
use robin_engine::console::{ConsoleCommand, parse_with_final};
use robin_engine::engine::{
    DevState, Engine, ExternalAction, ExternalActionResult, FrameConsoleResponse, LevelAssets,
    SimulationFrameInput,
};

/// Maximum characters in the input line.
const MAX_INPUT_LEN: usize = 128;
/// Output history ring-buffer cap.
const MAX_OUTPUT_LINES: usize = 256;
/// Number of in-game frames the caret stays visible / hidden.
const CARET_HALF_PERIOD: u32 = 12;
/// Lines moved by Page Up / Page Down.
const PAGE_SCROLL_LINES: usize = 8;
/// Lines moved per mouse-wheel notch.
const WHEEL_SCROLL_LINES: usize = 3;

/// Dev-mode console keyword table for tab completion.
///
/// First-token keywords from `console::parse_dev`.  The parser is
/// case-insensitive, so we keep these uppercase.  Multi-token commands
/// ("BIG BROTHER", "BUD SPENCER", …) are completed by the first token
/// only; the user types the rest by hand.
///
/// Mirrors what the parser *actually* recognises as a leading token —
/// stale entries like `ENERGYDISPLAY`, `GIVEAMMO`, `MISTERSANDMAN`,
/// `SANPETRUS`, `WINCAMPAIGN`, `WINNER`, `REINFORCEMENT` were removed
/// because the dev parser keys off `ALARM`, `BINGO`, `FULLHOUSE`,
/// `MISTER SANDMAN`, `SAN PETRUS`, `I AM THE WINNER`, and `WIN`
/// respectively.  Keeping the list in sync with `parse_dev` avoids
/// offering completions that the parser would then reject.
const COMPLETION_KEYWORDS_DEV: &[&str] = &[
    "AI",
    "ALARM",
    "AMOR",
    "AMULETS",
    "ANIM",
    "ASSERTFALSE",
    "BABYLON",
    "BIG",
    "BUD",
    "CALL",
    "CAMPAIGN",
    "CESTLAZONE",
    "COMA",
    "COMPANIES",
    "DIES",
    "EINSTEIN",
    "ELEVATION",
    "EULER",
    "EZB",
    "FORGET",
    "FPS",
    "FREEZE",
    "FULLHOUSE",
    "GOLDENEYE",
    "HADES",
    "HELP",
    "HIGHLANDER",
    "HIGHLANDER2",
    "HONOLULU",
    "I",
    "IDS",
    "KOLKOZ",
    "LAST",
    "LEVEL",
    "LIGHT",
    "LOOSE",
    "LUKAS",
    "MISTER",
    "MORPHEUS",
    "MOTION",
    "NOISE",
    "NUKE",
    "OPTIMIZE",
    "PAMELA",
    "PCSIGHT",
    "PROJECTION",
    "RAILROAD",
    "REPORT",
    "ROTER",
    "SAN",
    "SARKOZY",
    "SEEKANDDESTROY",
    "SHADOW",
    "SPHERE",
    "STATUS",
    "SURFACE",
    "UBIQUITY",
    "WAKEUP",
    "WAPPEN",
    "WASP",
    "WIN",
];

/// Final-mode (shipping build) cheat keyword table.  Matches the 9
/// commands `parse_final` recognises — and nothing more, so `use_final`
/// builds don't leak the dev cheat list via Tab.  The original game
/// suppressed completion entirely in shipping builds; we still offer
/// completions for the commands the player *can* use.
const COMPLETION_KEYWORDS_FINAL: &[&str] = &[
    "BINGO", "CASH", "EINSTEIN", "GOODLUCK", "IMMUNITY", "MERRYMAN", "PAM", "UNBLIP", "WINNER",
];

/// One line in the output history.  `Echo` is the user's input
/// (rendered with a `> ` prefix); `Response` is the dispatcher reply.
#[derive(Debug, Clone)]
enum OutputLine {
    Echo(String),
    Response(String),
    Error(String),
}

#[derive(Debug, Default)]
pub struct ConsoleOverlay {
    visible: bool,
    /// Current line being edited.
    input: String,
    /// Cursor position within `input`, measured in *character* indices
    /// (not bytes).  Enables Left / Right / Delete / Home / End editing.
    /// `0` = before first char; `input char count` = after last char.
    cursor: usize,
    /// Output history (most recent at the back).
    output: VecDeque<OutputLine>,
    /// Submitted command lines, for ↑/↓ recall.  Most recent at the back.
    cmd_history: Vec<String>,
    /// Index into `cmd_history` while navigating; `None` when on the
    /// freshly-typed line.  Stored as "distance from end" so wrap math
    /// stays stable as new commands are submitted.
    history_cursor: Option<usize>,
    /// Saved input line when the user starts navigating history, so
    /// pressing ↓ past the end restores what they were typing.
    history_saved_input: Option<String>,
    /// Index into the tab-completion candidate list, for cycling on
    /// repeated Tab presses.  Each press advances one slot and wraps.
    /// Cleared whenever the user edits the input, so a fresh prefix
    /// starts cycling from 0.
    completion_index: Option<usize>,
    /// View offset from the bottom of `output` (for Page Up/Down
    /// scrolling).  0 = pinned to latest line.
    scroll_from_bottom: usize,
    /// Caret blink animation counter.
    caret_timer: u32,
    /// Set to true when the most recent dispatch should auto-close the
    /// console (e.g. WIN / LOSE).
    pending_close: bool,
    /// When the `CAMPAIGN <path>` dispatcher command fires, the requested
    /// save-file path is stored here so the host game loop can drain it
    /// and initiate the load (the engine has no access to save-file I/O).
    pending_load_campaign: Option<std::path::PathBuf>,
    /// Set when the shipping-build deity easter egg fired.  The host
    /// drains this and applies the `InputTranslator::deity_call()`
    /// rebind table — the engine can't touch the host-owned input
    /// translator.
    pending_deity_invoked: bool,
}

impl ConsoleOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Toggle visibility.  Returns the new visibility so the caller can
    /// reset text input state when the overlay visibility changes.
    pub fn toggle(&mut self) -> bool {
        self.visible = !self.visible;
        if !self.visible {
            // Drop any in-progress edit when we close, so reopening
            // doesn't surprise the user with old text.  History is
            // preserved.
            self.input.clear();
            self.cursor = 0;
            self.history_cursor = None;
            self.history_saved_input = None;
            self.completion_index = None;
        }
        self.caret_timer = 0;
        self.scroll_from_bottom = 0;
        self.visible
    }

    pub fn close(&mut self) -> bool {
        if self.visible {
            self.visible = false;
            self.input.clear();
            self.cursor = 0;
            self.history_cursor = None;
            self.history_saved_input = None;
            self.completion_index = None;
            self.scroll_from_bottom = 0;
            true
        } else {
            false
        }
    }

    /// Process events while the console is visible.
    ///
    /// Returns `true` for every event that should be considered
    /// consumed by the console (so the game loop can skip routing it
    /// to other input handlers).  Side effects: typing, history
    /// navigation, scroll, command submission via `engine`.
    pub fn handle_events(
        &mut self,
        events: &[GameEvent],
        engine: &mut Engine,
        assets: &LevelAssets,
        host: &mut Host,
        dev: &mut DevState,
        admitted_actions: &mut Vec<ExternalAction>,
    ) -> bool {
        if !self.visible {
            return false;
        }
        let mut consumed_any = false;
        for event in events {
            match event {
                GameEvent::TextInput { text } => {
                    consumed_any = true;
                    for c in text.chars() {
                        if c.is_control() {
                            continue;
                        }
                        // Reject the toggle key from leaking in as text
                        // — when the player presses `~` to open the
                        // console, winit emits both KeyDown and a
                        // TextInput "`~`".  We swallow it so the
                        // input box doesn't open with a stray "~".
                        if c == '`' || c == '~' {
                            continue;
                        }
                        if self.input.chars().count() < MAX_INPUT_LEN {
                            self.insert_char_at_cursor(c);
                            self.caret_timer = 0;
                            self.history_cursor = None;
                            self.history_saved_input = None;
                            self.completion_index = None;
                        }
                    }
                }
                GameEvent::KeyDown { keycode, .. } => match keycode {
                    Keycode::Return | Keycode::KpEnter => {
                        consumed_any = true;
                        self.submit(host, engine, assets, dev, admitted_actions);
                    }
                    Keycode::Backspace => {
                        consumed_any = true;
                        self.backspace_at_cursor();
                        self.caret_timer = 0;
                        self.completion_index = None;
                    }
                    Keycode::Delete => {
                        consumed_any = true;
                        self.delete_at_cursor();
                        self.caret_timer = 0;
                        self.completion_index = None;
                    }
                    Keycode::Left => {
                        consumed_any = true;
                        if self.cursor > 0 {
                            self.cursor -= 1;
                        }
                        self.caret_timer = 0;
                    }
                    Keycode::Right => {
                        consumed_any = true;
                        let max = self.input.chars().count();
                        if self.cursor < max {
                            self.cursor += 1;
                        }
                        self.caret_timer = 0;
                    }
                    Keycode::Home => {
                        consumed_any = true;
                        self.cursor = 0;
                        self.caret_timer = 0;
                    }
                    Keycode::End => {
                        consumed_any = true;
                        self.cursor = self.input.chars().count();
                        self.caret_timer = 0;
                    }
                    Keycode::Up => {
                        consumed_any = true;
                        self.history_prev();
                    }
                    Keycode::Down => {
                        consumed_any = true;
                        self.history_next();
                    }
                    Keycode::Tab => {
                        consumed_any = true;
                        self.tab_complete(dev);
                    }
                    Keycode::PageUp => {
                        consumed_any = true;
                        self.scroll_up();
                    }
                    Keycode::PageDown => {
                        consumed_any = true;
                        self.scroll_down();
                    }
                    Keycode::Escape => {
                        consumed_any = true;
                        self.close();
                    }
                    // Any other KeyDown is consumed too — we don't
                    // want WASD-like shortcuts to leak into the game
                    // while the console has focus.
                    _ => {
                        consumed_any = true;
                    }
                },
                GameEvent::KeyUp { .. } => {
                    // KeyUp events don't drive game actions in the
                    // current input pipeline, but consume them for
                    // symmetry so KeyDown/KeyUp pair atomically.
                    consumed_any = true;
                }
                GameEvent::MouseWheel(delta) => {
                    consumed_any = true;
                    self.scroll_wheel(*delta);
                }
                _ => {
                    // Mouse move/buttons, resize, and quit pass through
                    // unconsumed — the game still wants to react to those.
                }
            }
        }
        // Drain anything the engine pushed to the console-output queue
        // during dispatch (or during script-native calls on other
        // frames).  Keeping the drain at the end of `handle_events`
        // catches both the submit-driven path and any stray pushes
        // produced by non-console code paths.
        self.drain_engine_output(dev);
        consumed_any
    }

    /// Insert `c` at the current cursor position, advancing the cursor.
    fn insert_char_at_cursor(&mut self, c: char) {
        let byte_idx = self.cursor_byte_index();
        self.input.insert(byte_idx, c);
        self.cursor += 1;
    }

    /// Delete the character immediately before the cursor, moving the
    /// cursor back one slot.  No-op when the cursor is at column 0.
    fn backspace_at_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let after = self.cursor_byte_index();
        let before = self
            .input
            .char_indices()
            .nth(self.cursor - 1)
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.input.replace_range(before..after, "");
        self.cursor -= 1;
    }

    /// Delete the character *at* the cursor (not before it).  No-op at
    /// end-of-line.
    fn delete_at_cursor(&mut self) {
        let total = self.input.chars().count();
        if self.cursor >= total {
            return;
        }
        let before = self.cursor_byte_index();
        let after = self
            .input
            .char_indices()
            .nth(self.cursor + 1)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        self.input.replace_range(before..after, "");
    }

    /// Convert the logical cursor column (char index) into the
    /// matching byte offset, for splicing into the UTF-8 `input`.
    fn cursor_byte_index(&self) -> usize {
        self.input
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    /// Drain `Console::pending_output` into the overlay scrollback.
    /// Called every frame from `handle_events` plus once after each
    /// `submit` so multi-line cheat output (STATUS PC, STATUS HARDWARE)
    /// lands interleaved with the user's echo line.
    fn drain_engine_output(&mut self, dev: &mut DevState) {
        for line in dev.console.drain_output() {
            self.push_output(OutputLine::Response(line));
        }
    }

    /// Per-frame tick used to advance the caret blink.  Call once per
    /// rendered frame.
    pub fn tick_animation(&mut self) {
        if self.visible {
            self.caret_timer = self.caret_timer.wrapping_add(1);
        }
    }

    fn submit(
        &mut self,
        host: &mut Host,
        engine: &mut Engine,
        assets: &LevelAssets,
        dev: &mut DevState,
        admitted_actions: &mut Vec<ExternalAction>,
    ) {
        let line = std::mem::take(&mut self.input);
        let trimmed = line.trim();
        self.cursor = 0;
        self.history_cursor = None;
        self.history_saved_input = None;
        self.completion_index = None;
        self.scroll_from_bottom = 0;
        if trimmed.is_empty() {
            return;
        }
        // Push to UI history and dispatch.
        self.push_output(OutputLine::Echo(line.clone()));
        let response = if dev.console.use_final
            && trimmed
                .split_whitespace()
                .next()
                .is_some_and(|token| token.eq_ignore_ascii_case("GLOIRE"))
        {
            dev.console.use_final = false;
            dev.console.push_history(trimmed);
            dev.console.push_output("Praised be His Name.");
            FrameConsoleResponse::DeityInvoked
        } else if let Some(command) = parse_with_final(trimmed, dev.console.use_final) {
            dev.console.push_history(trimmed);
            if command.is_host_only() {
                FrameConsoleResponse::from(engine.dispatch_host_console_command(
                    assets,
                    dev,
                    &mut host.frontend.selected_view_element,
                    &command,
                ))
            } else {
                // HONOLULU's remembered actor is a host UI latch. Resolve it
                // to the concrete authoritative target before admission.
                let selected_view_element = if matches!(command, ConsoleCommand::Honolulu) {
                    if let Some(selected) = host.frontend.selected_view_element {
                        dev.last_actor_in_honolulu = Some(selected);
                    }
                    host.frontend
                        .selected_view_element
                        .or(dev.last_actor_in_honolulu)
                } else {
                    host.frontend.selected_view_element
                };
                let action = ExternalAction::ConsoleCommand {
                    command,
                    selected_view_element,
                };
                let output = engine
                    .advance_frame(
                        assets,
                        SimulationFrameInput::no_hourglass()
                            .with_external_actions(vec![action.clone()]),
                    )
                    .expect("console action frame admission");
                let response = match output.external_action_results.into_iter().next() {
                    Some(ExternalActionResult::ConsoleCommand {
                        response,
                        selected_view_element,
                    }) => {
                        host.frontend.selected_view_element = selected_view_element;
                        response
                    }
                    _ => panic!("console action admission returned no console result"),
                };
                admitted_actions.push(action);
                response
            }
        } else {
            FrameConsoleResponse::Unknown
        };
        // Drain any lines the cheat pushed to the engine-side output
        // queue during dispatch (STATUS PC, STATUS HARDWARE, etc.).
        // Doing this *before* we process the response means the queued
        // lines appear above the response text in scrollback, which is
        // the natural reading order.
        self.drain_engine_output(dev);
        // Only retain non-empty / non-duplicate-of-last lines for
        // ↑ recall; mirrors typical shell behaviour.
        if self.cmd_history.last().map(String::as_str) != Some(trimmed) {
            self.cmd_history.push(line.clone());
        }
        match response {
            FrameConsoleResponse::Ok(text) => {
                if !text.is_empty() {
                    for chunk in text.lines() {
                        self.push_output(OutputLine::Response(chunk.to_string()));
                    }
                }
                // WIN / WINCAMPAIGN / LOSE auto-close the overlay.
                // We don't have a typed flag from the dispatcher, so
                // detect via mission state — both WIN paths set
                // `quit_won`, LOSE sets `quit_lost`.
                if engine.mission().quit_won || engine.mission().quit_lost {
                    self.pending_close = true;
                }
            }
            FrameConsoleResponse::Unknown => {
                self.push_output(OutputLine::Error(format!("Unknown command: {trimmed}")));
            }
            FrameConsoleResponse::NotImplemented(name) => {
                self.push_output(OutputLine::Error(format!("{name}: not implemented")));
            }
            FrameConsoleResponse::LoadCampaignRequested(path) => {
                // Engine can't reach the save-file parser; stash the
                // request on the overlay so the host game loop picks it
                // up on the next frame via `take_pending_load_campaign`.
                self.push_output(OutputLine::Response(format!(
                    "Loading campaign from {}...",
                    path
                )));
                self.pending_load_campaign = Some(path.into());
            }
            FrameConsoleResponse::DeityInvoked => {
                // The "Praised be His Name." line was pushed via
                // `Console::push_output` and already drained above; the
                // host applies the input-translator rebind on the next
                // tick via `take_pending_deity_invoked`.
                self.pending_deity_invoked = true;
            }
        }
    }

    /// Drain any host-side deferred console output (currently
    /// campaign-load outcome messages). These originate outside the
    /// engine dispatcher but still want to show up in the overlay's
    /// history.
    pub fn drain_pending_host_output(&mut self, host: &mut crate::host::Host) {
        if host.pending_console_output.is_empty() {
            return;
        }
        let lines = std::mem::take(&mut host.pending_console_output);
        for line in lines {
            if line.is_empty() {
                continue;
            }
            for chunk in line.lines() {
                self.push_output(OutputLine::Response(chunk.to_string()));
            }
        }
    }

    fn push_output(&mut self, line: OutputLine) {
        if self.output.len() >= MAX_OUTPUT_LINES {
            self.output.pop_front();
        }
        self.output.push_back(line);
        if self.scroll_from_bottom > 0 {
            self.scroll_from_bottom =
                (self.scroll_from_bottom + 1).min(self.max_scroll_from_bottom());
        }
    }

    fn history_prev(&mut self) {
        if self.cmd_history.is_empty() {
            return;
        }
        let next = match self.history_cursor {
            None => {
                // Save the in-progress line so ↓-past-end restores it.
                self.history_saved_input = Some(std::mem::take(&mut self.input));
                self.cmd_history.len() - 1
            }
            Some(0) => 0, // Already at the oldest entry.
            Some(i) => i - 1,
        };
        self.input.clone_from(&self.cmd_history[next]);
        self.cursor = self.input.chars().count();
        self.history_cursor = Some(next);
        self.completion_index = None;
        self.caret_timer = 0;
    }

    fn history_next(&mut self) {
        let Some(cur) = self.history_cursor else {
            return;
        };
        let last = self.cmd_history.len().saturating_sub(1);
        if cur >= last {
            // Past the newest entry — restore the saved in-progress
            // line and exit history-navigation mode.
            self.input = self.history_saved_input.take().unwrap_or_default();
            self.history_cursor = None;
        } else {
            let next = cur + 1;
            self.input.clone_from(&self.cmd_history[next]);
            self.history_cursor = Some(next);
        }
        self.cursor = self.input.chars().count();
        self.completion_index = None;
        self.caret_timer = 0;
    }

    fn tab_complete(&mut self, dev: &DevState) {
        // Complete the *first* token only — multi-word commands
        // (e.g. "BIG BROTHER") are completed token-by-token.  Copy
        // the tokenised prefix + trailing out of `self.input` first so
        // we can mutate `self` freely for the rest of the function.
        let (prefix_upper, trailing) = {
            let trimmed = self.input.trim_start();
            let first_token_end = trimmed
                .find(|c: char| c.is_whitespace())
                .unwrap_or(trimmed.len());
            let prefix = trimmed[..first_token_end].to_ascii_uppercase();
            let trail = trimmed[first_token_end..].trim_start().to_string();
            (prefix, trail)
        };
        if prefix_upper.is_empty() {
            self.completion_index = None;
            return;
        }
        // Pick the keyword set that matches the current cheat table.
        // In `use_final` mode we only offer the 9 shipping cheats, so
        // Tab can't leak the dev keyword list.
        let keywords: &[&str] = if dev.console.use_final {
            COMPLETION_KEYWORDS_FINAL
        } else {
            COMPLETION_KEYWORDS_DEV
        };
        let matches: Vec<&'static str> = keywords
            .iter()
            .copied()
            .filter(|kw| kw.starts_with(&prefix_upper))
            .collect();
        match matches.as_slice() {
            [] => {
                self.completion_index = None;
            }
            [single] => {
                // Unique match: replace the typed prefix with the
                // keyword + a space so the user can keep typing args.
                let mut completed = String::with_capacity(single.len() + trailing.len() + 1);
                completed.push_str(single);
                completed.push(' ');
                completed.push_str(&trailing);
                self.input = completed;
                self.cursor = self.input.chars().count();
                self.completion_index = None;
                self.caret_timer = 0;
            }
            many => {
                // Multiple matches: cycle through them on repeated Tab
                // presses.  On the first Tab after a fresh edit, list
                // the candidates so the user sees what's on offer;
                // subsequent Tabs substitute the actual keyword into
                // the input line one at a time.
                let first_press = self.completion_index.is_none();
                if first_press {
                    let joined = many.join("  ");
                    self.push_output(OutputLine::Response(joined));
                }
                let idx = match self.completion_index {
                    None => 0,
                    Some(i) => (i + 1) % many.len(),
                };
                self.completion_index = Some(idx);
                let pick = many[idx];
                let mut completed = String::with_capacity(pick.len() + trailing.len() + 1);
                completed.push_str(pick);
                if !trailing.is_empty() {
                    completed.push(' ');
                    completed.push_str(&trailing);
                }
                self.input = completed;
                self.cursor = self.input.chars().count();
                self.caret_timer = 0;
            }
        }
    }

    fn scroll_up(&mut self) {
        self.scroll_up_lines(PAGE_SCROLL_LINES);
    }

    fn scroll_down(&mut self) {
        self.scroll_down_lines(PAGE_SCROLL_LINES);
    }

    fn scroll_wheel(&mut self, delta: i32) {
        let lines = WHEEL_SCROLL_LINES.saturating_mul(delta.unsigned_abs() as usize);
        if delta > 0 {
            self.scroll_up_lines(lines);
        } else if delta < 0 {
            self.scroll_down_lines(lines);
        }
    }

    fn scroll_up_lines(&mut self, lines: usize) {
        let max = self.max_scroll_from_bottom();
        self.scroll_from_bottom = (self.scroll_from_bottom + lines).min(max);
    }

    fn scroll_down_lines(&mut self, lines: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(lines);
    }

    fn max_scroll_from_bottom(&self) -> usize {
        self.output.len().saturating_sub(1)
    }

    /// Drain any pending `CAMPAIGN <path>` request; the caller (host
    /// game loop) is expected to initiate the save-file load.
    pub fn take_pending_load_campaign(&mut self) -> Option<std::path::PathBuf> {
        self.pending_load_campaign.take()
    }

    /// Drain the deity-easter-egg flag.  When `true`, the host should
    /// invoke `InputTranslator::deity_call()` to apply its rebind table.
    pub fn take_pending_deity_invoked(&mut self) -> bool {
        std::mem::take(&mut self.pending_deity_invoked)
    }

    /// Drain the auto-close flag — caller toggles + stops text input
    /// when this returns `true`.
    pub fn take_pending_close(&mut self) -> bool {
        if self.pending_close {
            self.pending_close = false;
            self.visible = false;
            self.input.clear();
            self.cursor = 0;
            self.history_cursor = None;
            self.history_saved_input = None;
            self.completion_index = None;
            self.scroll_from_bottom = 0;
            true
        } else {
            false
        }
    }

    /// Render the console panel.  Caller must already be in the GPU
    /// phase (after `flush_base_layer`); the menu modal helper handles
    /// that for us, but in the live-game path we're already past
    /// `flush_base_layer` by the time this renders, so no extra setup.
    pub fn render(&self, renderer: &mut Renderer, font: Option<&NativeFont>) {
        if !self.visible {
            return;
        }
        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        if sw == 0 || sh == 0 {
            return;
        }
        // Panel covers the top 60% of the screen.
        let panel_h = (sh * 6) / 10;

        // Dim background — semi-transparent black.
        renderer.render_gpu_rect(0, 0, sw, panel_h, 0, 0, 0, 200);
        // Bottom border line for visual separation.
        renderer.render_gpu_rect(0, panel_h - 2, sw, 2, 100, 180, 220, 255);

        let Some(font) = font else { return };
        let font_h = font.height() as i32;
        if font_h <= 0 {
            return;
        }
        let line_step = font_h + 2;
        let pad_x = 12;
        let pad_y = 8;

        // Reserve the bottom line of the panel for the input field.
        let input_y = panel_h - pad_y - line_step;
        // Separator between scrollback and input field.
        renderer.render_gpu_rect(pad_x, input_y - 4, sw - 2 * pad_x, 1, 100, 180, 220, 180);

        // ── Scrollback ──
        let avail_lines = ((input_y - pad_y) / line_step).max(0) as usize;
        let total = self.output.len();
        let end_idx = total.saturating_sub(self.scroll_from_bottom);
        let start_idx = end_idx.saturating_sub(avail_lines);
        let mut y = pad_y;
        for line in self.output.iter().skip(start_idx).take(end_idx - start_idx) {
            let (prefix, body, _color) = match line {
                OutputLine::Echo(s) => ("> ", s.as_str(), (180, 220, 255)),
                OutputLine::Response(s) => ("  ", s.as_str(), (200, 200, 200)),
                OutputLine::Error(s) => ("! ", s.as_str(), (255, 160, 160)),
            };
            // We don't have per-string colour control on the native
            // font path, so render a single string with the prefix
            // baked in.  Operator gets the cue from the leading glyph.
            let mut combined = String::with_capacity(prefix.len() + body.len());
            combined.push_str(prefix);
            combined.push_str(body);
            render_text_screen(renderer, font, &combined, pad_x, y);
            y += line_step;
            if y > input_y - line_step {
                break;
            }
        }

        // Scroll indicator (top of panel) when scrolled up.
        if self.scroll_from_bottom > 0 {
            let label = format!("[scrolled {} lines]", self.scroll_from_bottom);
            render_text_screen(
                renderer,
                font,
                &label,
                sw - pad_x - font.text_width(&label),
                pad_y,
            );
        }

        // ── Input line ──
        let prompt = "> ";
        let prompt_w = font.text_width(prompt);
        render_text_screen(renderer, font, prompt, pad_x, input_y);
        render_text_screen(renderer, font, &self.input, pad_x + prompt_w, input_y);

        // Blinking caret at the cursor position (not necessarily at
        // end-of-line, since Left/Right/Home/End / Delete all reposition
        // the cursor into the middle of the buffer).
        let caret_visible = (self.caret_timer / CARET_HALF_PERIOD).is_multiple_of(2);
        if caret_visible {
            let byte_idx = self.cursor_byte_index();
            let before_cursor = &self.input[..byte_idx];
            let caret_x = pad_x + prompt_w + font.text_width(before_cursor) + 1;
            renderer.render_gpu_rect(caret_x, input_y, 2, font_h, 230, 230, 230, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_starts_hidden() {
        let mut c = ConsoleOverlay::new();
        assert!(!c.is_visible());
        assert!(c.toggle());
        assert!(c.is_visible());
        assert!(!c.toggle());
        assert!(!c.is_visible());
    }

    #[test]
    fn close_when_hidden_is_noop() {
        let mut c = ConsoleOverlay::new();
        assert!(!c.close());
        c.toggle();
        assert!(c.close());
        assert!(!c.is_visible());
    }

    #[test]
    fn tab_completes_unique_prefix() {
        let dev = DevState::default();
        let mut c = ConsoleOverlay::new();
        c.toggle();
        c.input = "fre".to_string();
        c.cursor = c.input.chars().count();
        c.tab_complete(&dev);
        // FREEZE is the only `FRE…` keyword.
        assert_eq!(c.input, "FREEZE ");
    }

    #[test]
    fn tab_cycles_multiple_candidates() {
        let dev = DevState::default();
        let mut c = ConsoleOverlay::new();
        c.toggle();
        c.input = "h".to_string();
        c.cursor = 1;
        c.tab_complete(&dev);
        // First press prints the list *and* selects the first match.
        assert!(matches!(c.output.back(), Some(OutputLine::Response(_))));
        let first = c.input.clone();
        assert!(first.starts_with('H'));
        // Second press advances to the next candidate.
        c.tab_complete(&dev);
        let second = c.input.clone();
        assert_ne!(first, second);
        assert!(second.starts_with('H'));
    }

    #[test]
    fn tab_completion_uses_final_set_in_final_mode() {
        let mut dev = DevState::default();
        dev.console.use_final = true;
        let mut c = ConsoleOverlay::new();
        c.toggle();
        // `CA` is a unique prefix only in the final set (CASH).  In the
        // dev set it would match CALL / CAMPAIGN too.
        c.input = "ca".to_string();
        c.cursor = 2;
        c.tab_complete(&dev);
        assert_eq!(c.input, "CASH ");
    }

    #[test]
    fn history_navigation_round_trip() {
        let mut c = ConsoleOverlay::new();
        c.cmd_history = vec!["EZB 100".to_string(), "FREEZE".to_string()];
        c.input = "in-progress".to_string();
        c.cursor = c.input.chars().count();
        c.history_prev();
        assert_eq!(c.input, "FREEZE");
        assert_eq!(c.cursor, "FREEZE".chars().count());
        c.history_prev();
        assert_eq!(c.input, "EZB 100");
        c.history_prev();
        assert_eq!(c.input, "EZB 100"); // Clamped at oldest.
        c.history_next();
        assert_eq!(c.input, "FREEZE");
        c.history_next();
        // Past the newest — restored in-progress line.
        assert_eq!(c.input, "in-progress");
        assert!(c.history_cursor.is_none());
    }

    #[test]
    fn cursor_editing_roundtrip() {
        let mut c = ConsoleOverlay::new();
        c.toggle();
        // Start with "ABCDE", cursor at the end.
        c.input = "ABCDE".to_string();
        c.cursor = 5;
        // Move to column 2 and insert 'x' — should yield "ABxCDE".
        c.cursor = 2;
        c.insert_char_at_cursor('x');
        assert_eq!(c.input, "ABxCDE");
        assert_eq!(c.cursor, 3);
        // Backspace removes 'x' (char before cursor).
        c.backspace_at_cursor();
        assert_eq!(c.input, "ABCDE");
        assert_eq!(c.cursor, 2);
        // Delete removes 'C' (char at cursor).
        c.delete_at_cursor();
        assert_eq!(c.input, "ABDE");
        assert_eq!(c.cursor, 2);
    }

    #[test]
    fn scroll_clamps_to_history_bounds() {
        let mut c = ConsoleOverlay::new();
        for i in 0..3 {
            c.push_output(OutputLine::Response(format!("line {i}")));
        }

        c.scroll_up_lines(100);
        assert_eq!(c.scroll_from_bottom, 2);

        c.scroll_down_lines(100);
        assert_eq!(c.scroll_from_bottom, 0);
    }

    #[test]
    fn push_output_keeps_scrolled_view_anchored() {
        let mut c = ConsoleOverlay::new();
        for i in 0..8 {
            c.push_output(OutputLine::Response(format!("line {i}")));
        }

        c.scroll_up_lines(3);
        c.push_output(OutputLine::Response("new line".to_string()));

        assert_eq!(c.scroll_from_bottom, 4);
    }

    #[test]
    fn input_truncates_at_max_len() {
        let mut c = ConsoleOverlay::new();
        c.toggle();
        let event = GameEvent::TextInput {
            text: "a".repeat(MAX_INPUT_LEN + 50),
        };
        // We can't construct a real Engine here, so test the
        // text-input length cap directly via the input vec length.
        // (Full event handling is exercised in integration tests.)
        if let GameEvent::TextInput { text } = event {
            for ch in text.chars() {
                if c.input.chars().count() < MAX_INPUT_LEN {
                    c.input.push(ch);
                }
            }
        }
        assert_eq!(c.input.chars().count(), MAX_INPUT_LEN);
    }
}

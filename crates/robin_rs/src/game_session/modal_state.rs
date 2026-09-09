//! Modal-state machinery: dialogue / popup-scroll / debriefing /
//! mission-state batches, the unified `ActiveModal` enum, and the
//! `start_/tick_/drain_pending_*` helpers that drive them.

use super::session_policy::ModalBatchState;
use crate::audio_backend::KiraAudioBackend;
use crate::console_overlay::ConsoleOverlay;
use crate::cursor::CursorRenderer;
use crate::game::Game;
use crate::host::Host;
use crate::host::HostSignal;
use crate::ingame_menu::modal_net::ModalDismissalGate;
use crate::ingame_menu::widget_bridge::default_modal_cursor;
use crate::ingame_menu::{
    self, DebriefingModalState, DebriefingOutcome, DialogueModalState, DialogueSentence,
    IngameMenuResources, MissionStatePopupState, ModalNet, PopupScrollItem, PopupScrollModalState,
    TradingModalState, TradingOutcome, layout::TextAlign,
};
use crate::renderer::Renderer;
use crate::window::{GameWindow, start_text_input};
use robin_assets::res_descr as assets_res_descr;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::engine::Engine;
use robin_engine::player_command as engine_player_command;
use robin_engine::player_command::DebriefingTextId;
use robin_engine::profiles as engine_profiles;
use robin_engine::resource_ids::RHID_DEFAULT_POPUP_SCROLL_PICTURE;
use robin_engine::sherwood_stat::{ProductionForecastCycle, ScoreInfo, SherwoodStat};
use robin_engine::sound_cache::SampleLoader;
use robin_engine::sound_config::SoundConfig;
use std::collections::VecDeque;

/// Presentation-side plumbing every modal lane needs: the event pump,
/// the renderer, cursor drawing, audio output, the shared ingame-menu
/// resources, and the current frame's host-control collector.
///
/// The engine/host half deliberately stays out of this bundle —
/// functions that need it take `&mut Host` alongside the context, and
/// pull `host.audio.sound` / `host.transport.net` from disjoint fields
/// so the two borrows coexist.
pub(crate) struct ModalContext<'a> {
    pub window: &'a mut GameWindow,
    pub renderer: &'a mut Renderer,
    pub cursor_res: &'a mut ResourceManager,
    pub cursor_renderer: &'a mut CursorRenderer,
    pub audio_backend: &'a mut Option<KiraAudioBackend>,
    pub sample_loader: &'a SampleLoader,
    pub menu_resources: &'a mut Option<IngameMenuResources>,
    pub modal_dismissals: &'a mut Vec<engine_player_command::PlayerCommand>,
}

/// One modal screen driven frame-by-frame inside a [`ModalBatch`].
///
/// Each lane (dialogue, popup scroll, debriefing) supplies the same
/// life cycle: pop an item, short-circuit on a pre-recorded replay
/// result, otherwise open the screen and tick it until it yields an
/// outcome, record the dismissal, and move to the next item.  The
/// shared driver lives in [`ModalBatch::tick`] so a fix to the batch
/// flow can't miss one of the lanes.
pub(super) trait ModalScreen: Sized {
    /// Queued content for one screen of this lane.
    type Item;
    /// What the screen reports when the player dismisses it.
    type Outcome;
    /// Warning logged (once per tick) when the menu resources vanish
    /// mid-batch; the batch is dropped in that case.
    const MISSING_RESOURCES_WARN: &'static str;
    /// Dialogue and popup states already keep rendering while a client waits
    /// for the host decision. Simpler screens delegate that waiting state to
    /// the shared batch driver.
    const HANDLES_NETWORK_AUTHORITY: bool = false;

    fn item_kind(item: &Self::Item) -> engine_player_command::ModalKind;

    /// Open the screen for `item`.  Only called after the driver has
    /// verified `ctx.menu_resources` is populated.
    fn begin(host: &mut Host, ctx: &mut ModalContext<'_>, item: Self::Item) -> Self;

    /// Advance the screen by one frame; `Some` means dismissed.
    fn step(
        &mut self,
        kind: &engine_player_command::ModalKind,
        host: &mut Host,
        ctx: &mut ModalContext<'_>,
    ) -> Option<Self::Outcome>;

    /// Pump presentation without accepting input, timeouts, or remote results.
    /// Strict replay alone owns the dismissal of this screen.
    fn render_replay_wait(&mut self, host: &mut Host, ctx: &mut ModalContext<'_>);

    fn finish_replay(
        &mut self,
        _host: &mut Host,
        _ctx: &mut ModalContext<'_>,
        _result: engine_player_command::DialogResult,
    ) {
    }

    /// Map the screen's outcome onto the replay-recorded result.
    fn to_result(outcome: &Self::Outcome) -> engine_player_command::DialogResult;
}

/// A queue of modal items plus the currently open screen, advanced one
/// frame per [`Self::tick`] call.
pub(super) struct ModalBatch<S: ModalScreen> {
    lifecycle: ModalBatchState<S::Item>,
    current: Option<(engine_player_command::ModalKind, S)>,
    dismissal: ModalDismissalGate,
}

pub(super) type ActiveDialogueBatch = ModalBatch<DialogueModalState>;
pub(super) type ActivePopupScrollBatch = ModalBatch<PopupScrollModalState>;
pub(super) type ActiveDebriefingBatch = ModalBatch<DebriefingModalState>;

impl<S: ModalScreen> ModalBatch<S> {
    fn new(pending: VecDeque<S::Item>) -> Self {
        Self {
            lifecycle: ModalBatchState::new(pending),
            current: None,
            dismissal: ModalDismissalGate::default(),
        }
    }

    fn is_empty(&self) -> bool {
        self.lifecycle.is_empty()
    }

    fn current_kind(&self) -> Option<engine_player_command::ModalKind> {
        self.lifecycle.current_kind(S::item_kind)
    }

    fn apply_replay_result(
        &mut self,
        kind: engine_player_command::ModalKind,
        result: engine_player_command::DialogResult,
        ctx: &mut ModalContext<'_>,
    ) {
        self.dismissal.retire();
        self.lifecycle.finish(&kind, result);
        ctx.modal_dismissals
            .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
    }

    fn tick(
        &mut self,
        host: &mut Host,
        ctx: &mut ModalContext<'_>,
        replay_modal_dismissals: &mut ReplayModalDismissals,
    ) {
        loop {
            let pending_controls = replay_modal_dismissals.len();
            self.tick_one(host, ctx, replay_modal_dismissals);
            if self.lifecycle.is_empty()
                || replay_modal_dismissals.is_empty()
                || replay_modal_dismissals.len() == pending_controls
            {
                break;
            }
        }
    }

    fn tick_one(
        &mut self,
        host: &mut Host,
        ctx: &mut ModalContext<'_>,
        replay_modal_dismissals: &mut ReplayModalDismissals,
    ) {
        if ctx.menu_resources.is_none() {
            tracing::warn!("{}", S::MISSING_RESOURCES_WARN);
            self.lifecycle.clear();
            self.current = None;
            return;
        }

        if self.current.is_none()
            && let Some(item) = self.lifecycle.start_next(S::item_kind)
        {
            let kind = S::item_kind(&item);
            let screen = S::begin(host, ctx, item);
            self.current = Some((kind, screen));
            self.dismissal = ModalDismissalGate::default();
        }

        let admission = self
            .current
            .as_ref()
            .map(|(kind, _)| replay_modal_dismissals.admit_screen(kind));
        if let Some(ModalScreenAdmission::Recorded(result)) = admission {
            let (kind, mut screen) = self
                .current
                .take()
                .expect("active modal disappeared while applying replay dismissal");
            screen.finish_replay(host, ctx, result);
            self.apply_replay_result(kind, result, ctx);
            return;
        }

        if admission == Some(ModalScreenAdmission::AwaitRecorded) {
            if let Some((_, screen)) = self.current.as_mut() {
                screen.render_replay_wait(host, ctx);
            }
            return;
        }

        if !S::HANDLES_NETWORK_AUTHORITY
            && let Some((kind, _)) = self.current.as_ref()
        {
            let modal_net = host.transport.net().map(|net| {
                ModalNet::new(
                    net,
                    kind.clone(),
                    host.transport.local_seat() == engine_player_command::PlayerId::HOST,
                )
            });
            if let Some(result) = self.dismissal.poll(modal_net.as_ref()) {
                let (kind, _) = self
                    .current
                    .take()
                    .expect("active modal disappeared while applying host decision");
                self.apply_replay_result(kind, result, ctx);
                return;
            }
            if self.dismissal.is_pending() {
                return;
            }
        }

        let Some((kind, screen)) = self.current.as_mut() else {
            return;
        };

        if let Some(outcome) = screen.step(kind, host, ctx) {
            let mut result = S::to_result(&outcome);
            if !S::HANDLES_NETWORK_AUTHORITY {
                let modal_net = host.transport.net().map(|net| {
                    ModalNet::new(
                        net,
                        kind.clone(),
                        host.transport.local_seat() == engine_player_command::PlayerId::HOST,
                    )
                });
                if let Some(confirmed) = self.dismissal.request(result, modal_net.as_ref()) {
                    result = confirmed;
                } else {
                    return;
                }
            }
            ctx.modal_dismissals
                .push(engine_player_command::PlayerCommand::ModalDismiss {
                    kind: kind.clone(),
                    result,
                });
            self.lifecycle.finish(kind, result);
            self.current = None;
        }
    }
}

pub(super) struct ActiveDialogueItem {
    kind: engine_player_command::ModalKind,
    sentences: Vec<DialogueSentence>,
}

impl ModalScreen for DialogueModalState {
    type Item = ActiveDialogueItem;
    type Outcome = engine_player_command::DialogResult;
    const MISSING_RESOURCES_WARN: &'static str =
        "DisplayDialog: menu resources unavailable — dropping active dialogue";
    const HANDLES_NETWORK_AUTHORITY: bool = true;

    fn item_kind(item: &Self::Item) -> engine_player_command::ModalKind {
        item.kind.clone()
    }

    fn begin(_host: &mut Host, ctx: &mut ModalContext<'_>, item: Self::Item) -> Self {
        let ModalContext {
            window,
            renderer,
            menu_resources,
            ..
        } = ctx;
        let resources = menu_resources
            .as_mut()
            .expect("ModalBatch::tick verified menu resources before begin");
        DialogueModalState::new(window, renderer, resources, item.sentences)
    }

    fn step(
        &mut self,
        kind: &engine_player_command::ModalKind,
        host: &mut Host,
        ctx: &mut ModalContext<'_>,
    ) -> Option<Self::Outcome> {
        let ModalContext {
            window,
            renderer,
            cursor_res,
            cursor_renderer,
            audio_backend,
            menu_resources,
            ..
        } = ctx;
        let resources = menu_resources
            .as_mut()
            .expect("ModalBatch::tick verified menu resources before step");
        let sound_cfg = SoundConfig::default();
        let sound_enabled = audio_backend.is_some();
        let modal_net = host.transport.net().map(|net| {
            ModalNet::new(
                net,
                kind.clone(),
                host.transport.local_seat() == engine_player_command::PlayerId::HOST,
            )
        });
        let cursor = default_modal_cursor(cursor_renderer, cursor_res, renderer);
        self.tick(
            window,
            renderer,
            resources,
            &mut host.audio.sound,
            &sound_cfg,
            audio_backend
                .as_mut()
                .map(|b| b as &mut dyn crate::sound::AudioBackend),
            sound_enabled,
            Some(&cursor),
            modal_net.as_ref(),
        )
    }

    fn to_result(outcome: &Self::Outcome) -> engine_player_command::DialogResult {
        *outcome
    }

    fn render_replay_wait(&mut self, host: &mut Host, ctx: &mut ModalContext<'_>) {
        let mut audio = ctx
            .audio_backend
            .as_mut()
            .map(|backend| backend as &mut dyn crate::sound::AudioBackend);
        let sound_enabled = audio.is_some();
        self.advance_replay_presentation(
            &mut host.audio.sound,
            &SoundConfig::default(),
            &mut audio,
            sound_enabled,
        );
        let cursor = default_modal_cursor(ctx.cursor_renderer, ctx.cursor_res, ctx.renderer);
        self.render_replay_wait(
            ctx.window,
            ctx.renderer,
            ctx.menu_resources
                .as_mut()
                .expect("active dialogue resources"),
            Some(&cursor),
        );
    }

    fn finish_replay(
        &mut self,
        host: &mut Host,
        ctx: &mut ModalContext<'_>,
        result: engine_player_command::DialogResult,
    ) {
        self.finish_replay_audio(
            &mut host.audio.sound,
            &SoundConfig::default(),
            ctx.audio_backend
                .as_mut()
                .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
            result,
        );
    }
}

impl ModalScreen for PopupScrollModalState {
    type Item = PopupScrollItem;
    type Outcome = engine_player_command::DialogResult;
    const MISSING_RESOURCES_WARN: &'static str =
        "DisplayPopupText: menu resources unavailable — dropping active popup";
    const HANDLES_NETWORK_AUTHORITY: bool = true;

    fn item_kind(item: &Self::Item) -> engine_player_command::ModalKind {
        item.kind.clone()
    }

    fn begin(_host: &mut Host, ctx: &mut ModalContext<'_>, item: Self::Item) -> Self {
        let ModalContext {
            window,
            renderer,
            menu_resources,
            ..
        } = ctx;
        let resources = menu_resources
            .as_mut()
            .expect("ModalBatch::tick verified menu resources before begin");
        PopupScrollModalState::new(
            window,
            renderer,
            resources,
            item.title,
            item.picture,
            item.body,
            item.body_font_name,
            item.align,
            item.universal_frame,
        )
    }

    fn step(
        &mut self,
        kind: &engine_player_command::ModalKind,
        host: &mut Host,
        ctx: &mut ModalContext<'_>,
    ) -> Option<Self::Outcome> {
        let ModalContext {
            window,
            renderer,
            cursor_res,
            cursor_renderer,
            audio_backend,
            sample_loader,
            menu_resources,
            ..
        } = ctx;
        let resources = menu_resources
            .as_mut()
            .expect("ModalBatch::tick verified menu resources before step");
        let modal_net = host.transport.net().map(|net| {
            ModalNet::new(
                net,
                kind.clone(),
                host.transport.local_seat() == engine_player_command::PlayerId::HOST,
            )
        });
        let cursor = default_modal_cursor(cursor_renderer, cursor_res, renderer);
        self.tick(
            window,
            renderer,
            resources,
            &mut host.audio.sound,
            audio_backend
                .as_mut()
                .map(|b| b as &mut dyn crate::sound::AudioBackend),
            *sample_loader,
            Some(cursor),
            modal_net.as_ref(),
        )
    }

    fn to_result(outcome: &Self::Outcome) -> engine_player_command::DialogResult {
        *outcome
    }

    fn render_replay_wait(&mut self, _host: &mut Host, ctx: &mut ModalContext<'_>) {
        let cursor = default_modal_cursor(ctx.cursor_renderer, ctx.cursor_res, ctx.renderer);
        self.render_replay_wait(
            ctx.window,
            ctx.renderer,
            ctx.menu_resources.as_ref().expect("active popup resources"),
            Some(&cursor),
        );
    }
}

pub(super) struct ActiveDebriefingItem {
    kind: engine_player_command::ModalKind,
    body: String,
    won: bool,
}

impl ModalScreen for DebriefingModalState {
    type Item = ActiveDebriefingItem;
    type Outcome = DebriefingOutcome;
    const MISSING_RESOURCES_WARN: &'static str =
        "DisplayDebriefing: menu resources unavailable — dropping active debriefing";

    fn item_kind(item: &Self::Item) -> engine_player_command::ModalKind {
        item.kind.clone()
    }

    fn begin(_host: &mut Host, ctx: &mut ModalContext<'_>, item: Self::Item) -> Self {
        let resources = ctx
            .menu_resources
            .as_ref()
            .expect("ModalBatch::tick verified menu resources before begin");
        DebriefingModalState::new(
            resources, item.body, None, 0, item.won, false, None, false, false,
        )
    }

    fn step(
        &mut self,
        _kind: &engine_player_command::ModalKind,
        _host: &mut Host,
        ctx: &mut ModalContext<'_>,
    ) -> Option<Self::Outcome> {
        let ModalContext {
            window,
            renderer,
            cursor_res,
            cursor_renderer,
            menu_resources,
            ..
        } = ctx;
        let resources = menu_resources
            .as_ref()
            .expect("ModalBatch::tick verified menu resources before step");
        let cursor = default_modal_cursor(cursor_renderer, cursor_res, renderer);
        self.tick(window, renderer, resources, Some(cursor))
    }

    fn to_result(outcome: &Self::Outcome) -> engine_player_command::DialogResult {
        if matches!(outcome, DebriefingOutcome::EmergencyEnd) {
            engine_player_command::DialogResult::Aborted
        } else {
            engine_player_command::DialogResult::Completed
        }
    }

    fn render_replay_wait(&mut self, _host: &mut Host, ctx: &mut ModalContext<'_>) {
        let cursor = default_modal_cursor(ctx.cursor_renderer, ctx.cursor_res, ctx.renderer);
        self.render_scripted_replay_wait(
            ctx.window,
            ctx.renderer,
            ctx.menu_resources
                .as_ref()
                .expect("active debriefing resources"),
            Some(&cursor),
        );
    }
}

pub(super) enum ActiveModal {
    Dialogue(Box<ActiveDialogueBatch>),
    PopupScroll(Box<ActivePopupScrollBatch>),
    Debriefing(Box<ActiveDebriefingBatch>),
    MissionState {
        kind: engine_player_command::ModalKind,
        state: MissionStatePopupState,
        replay_result: Option<engine_player_command::DialogResult>,
        dismissal: ModalDismissalGate,
    },
    Trading(Box<TradingModalState>),
}

impl ActiveModal {
    pub(super) fn is_empty(&self) -> bool {
        match self {
            ActiveModal::Dialogue(batch) => batch.is_empty(),
            ActiveModal::PopupScroll(batch) => batch.is_empty(),
            ActiveModal::Debriefing(batch) => batch.is_empty(),
            ActiveModal::MissionState { .. } => false,
            ActiveModal::Trading(_) => false,
        }
    }

    /// Whether this local overlay freezes deterministic simulation. Trading
    /// remains modal to host input, but connected peers must keep advancing;
    /// pausing only the host would immediately diverge the multiplayer clock.
    pub(super) fn pauses_simulation(&self, multiplayer_connected: bool) -> bool {
        match self {
            ActiveModal::Trading(_) if multiplayer_connected => false,
            _ => !self.is_empty(),
        }
    }

    pub(super) fn kind(&self) -> Option<engine_player_command::ModalKind> {
        match self {
            ActiveModal::Dialogue(batch) => batch.current_kind(),
            ActiveModal::PopupScroll(batch) => batch.current_kind(),
            ActiveModal::Debriefing(batch) => batch.current_kind(),
            ActiveModal::MissionState { kind, .. } => Some(kind.clone()),
            // Trading is a local non-pausing control panel rather than a
            // deterministic dialogue outcome synchronized as ModalDismiss.
            ActiveModal::Trading(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActiveModalOutcome {
    None,
    QuitMissionRequested,
    SellSherwoodItem {
        request_id: u64,
        prod_type: robin_engine::sector_production::Type,
        quantity: robin_engine::trading::TradeQuantity,
    },
}

use super::session_policy::ModalScreenAdmission;
pub(super) use super::session_policy::{ReplayModalDismissals, pop_matching_dismissal};

fn debriefing_replay_result(result: engine_player_command::DialogResult) -> DebriefingOutcome {
    match result {
        engine_player_command::DialogResult::Completed => DebriefingOutcome::Ok {
            text_remaining: String::new(),
        },
        engine_player_command::DialogResult::Aborted => DebriefingOutcome::EmergencyEnd,
        engine_player_command::DialogResult::Restart
        | engine_player_command::DialogResult::Load { .. } => {
            tracing::warn!(
                ?result,
                "queued debriefing replay result is only valid for final debriefing; treating as completed"
            );
            DebriefingOutcome::Ok {
                text_remaining: String::new(),
            }
        }
    }
}

pub(super) fn drain_pending_console_display(host: &mut Host, console_overlay: &mut ConsoleOverlay) {
    // ── Drain pending console-display request ──
    // Script native `DisplayConsole` (and the forthcoming cheat key)
    // sets `pending_show_console`.
    if host.effects.take_signal(HostSignal::ShowConsole) && !console_overlay.is_visible() {
        let now_visible = console_overlay.toggle();
        if now_visible {
            start_text_input();
        }
    }
}

/// Drain script-queued dialogues for the frame.
///
/// Script natives queue `StartDialog` commands during the tick; we
/// display them synchronously here so the dialogue runs inline
/// during script execution.
///
/// During replay, the dismiss result was pre-extracted from this
/// frame's command stream above and is passed straight to
/// `show_dialogue`, which short-circuits its event loop. During
/// recording, the interactive result is appended to the recorder so
/// future replays of this file can reproduce the dismissal.
pub(super) fn start_active_dialogue_batch(
    dialog_ids: Vec<i32>,
    text_res: &mut ResourceManager,
    game: &Game,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
) -> Option<ActiveDialogueBatch> {
    if dialog_ids.is_empty() {
        return None;
    }
    let Some(descriptors) = level_descriptors else {
        tracing::warn!(
            "DisplayDialog: level descriptors unavailable — dropping {} dialogue(s)",
            dialog_ids.len()
        );
        return None;
    };

    let mut pending = VecDeque::with_capacity(dialog_ids.len());
    for dialog_id in dialog_ids {
        let sentences = build_dialogue_sentences(
            dialog_id,
            descriptors,
            text_res,
            &game.global_options.text_directory,
        );
        if sentences.is_empty() {
            continue;
        }
        let kind = engine_player_command::ModalKind::Dialog { dialog_id };
        pending.push_back(ActiveDialogueItem { kind, sentences });
    }

    (!pending.is_empty()).then(|| ModalBatch::new(pending))
}

pub(super) async fn drain_pending_dialogues(
    host: &mut Host,
    ctx: &mut ModalContext<'_>,
    text_res: &mut ResourceManager,
    game: &Game,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    replay_modal_dismissals: &mut ReplayModalDismissals,
    headless: bool,
) {
    // ── Drain pending dialogues ──
    // Script natives queue `StartDialog` commands during the tick;
    // we display them synchronously here so the dialogue runs inline
    // during script execution.
    //
    // During replay, the dismiss result was pre-extracted from this
    // frame's command stream above and is passed straight to
    // `show_dialogue`, which short-circuits its event loop. During
    // recording, the interactive result is appended to the recorder
    // so future replays of this file can reproduce the dismissal.
    if host.effects.dialogue_count() != 0 {
        let dialog_ids: Vec<i32> = host.effects.take_dialogues();
        if headless {
            tracing::debug!(
                count = dialog_ids.len(),
                "headless: auto-dismissing pending dialogues"
            );
            for dialog_id in dialog_ids {
                let kind = engine_player_command::ModalKind::Dialog { dialog_id };
                let result = pop_matching_dismissal(replay_modal_dismissals, &kind)
                    .unwrap_or(engine_player_command::DialogResult::Completed);
                ctx.modal_dismissals
                    .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
            }
            return;
        }
        if let Some(descriptors) = level_descriptors
            && ctx.menu_resources.is_some()
        {
            // Pre-build every entry so we can hand a contiguous
            // slice to `show_dialogue_batch`.  `replay_result` pulls
            // from the per-frame replay queue so playback reproduces
            // the recorded dismissal exactly.
            let mut sentences_per_id: Vec<(i32, Vec<DialogueSentence>)> =
                Vec::with_capacity(dialog_ids.len());
            for dialog_id in dialog_ids {
                let sentences = build_dialogue_sentences(
                    dialog_id,
                    descriptors,
                    text_res,
                    &game.global_options.text_directory,
                );
                if sentences.is_empty() {
                    continue;
                }
                sentences_per_id.push((dialog_id, sentences));
            }
            let entries: Vec<ingame_menu::BatchDialogue<'_>> = sentences_per_id
                .iter()
                .map(|(dialog_id, sentences)| {
                    let kind = engine_player_command::ModalKind::Dialog {
                        dialog_id: *dialog_id,
                    };
                    let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
                    let modal_net = host.transport.net().map(|net| {
                        ModalNet::new(
                            net,
                            kind.clone(),
                            host.transport.local_seat() == engine_player_command::PlayerId::HOST,
                        )
                    });
                    ingame_menu::BatchDialogue {
                        sentences: sentences.as_slice(),
                        replay_result,
                        modal_net,
                    }
                })
                .collect();

            let results =
                ingame_menu::show_dialogue_batch(ctx, &mut host.audio.sound, &entries).await;

            for ((dialog_id, _), result) in sentences_per_id.iter().zip(results.iter().copied()) {
                let kind = engine_player_command::ModalKind::Dialog {
                    dialog_id: *dialog_id,
                };
                ctx.modal_dismissals
                    .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
            }
        }
    }
}

pub(super) fn start_active_popup_scroll_batch(
    text_ids: Vec<i32>,
    ctx: &mut ModalContext<'_>,
    text_res: &mut ResourceManager,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    universal_frame: u32,
) -> Option<ActivePopupScrollBatch> {
    if text_ids.is_empty() {
        return None;
    }
    if ctx.menu_resources.is_none() {
        tracing::warn!(
            "DisplayPopupText: menu resources unavailable — dropping {} popup(s)",
            text_ids.len()
        );
        return None;
    }

    let mut pending = VecDeque::with_capacity(text_ids.len());
    for text_id in text_ids {
        let (text, picture_id) = if let Some(descriptors) = level_descriptors.as_ref() {
            let table_id = descriptors.popup_text.text_table_id;
            let text = usize::try_from(text_id)
                .ok()
                .and_then(|index| descriptors.custom_popup_texts.get(index))
                .and_then(Option::as_ref)
                .cloned()
                .unwrap_or_else(|| match text_res.get_string(table_id, text_id as usize) {
                    Ok(s) => s.to_string(),
                    Err(e) => {
                        tracing::warn!("DisplayPopupText({text_id}): text lookup failed: {e}");
                        "Invalid popup text ID...".to_string()
                    }
                });
            let pid = descriptors
                .popup_text
                .picture_ids
                .get(text_id as usize)
                .copied()
                .unwrap_or(RHID_DEFAULT_POPUP_SCROLL_PICTURE);
            (text, pid)
        } else {
            tracing::warn!("DisplayPopupText({text_id}): level descriptors unavailable");
            (
                "No popup texts for the current level !".to_string(),
                RHID_DEFAULT_POPUP_SCROLL_PICTURE,
            )
        };
        let picture = ctx
            .menu_resources
            .as_mut()
            .expect("checked above")
            .picture_from(ctx.renderer, text_res, picture_id);
        let kind = engine_player_command::ModalKind::PopupText { text_id };
        let replay_result = None;
        pending.push_back(PopupScrollItem {
            kind,
            title: None,
            picture,
            body: text,
            body_font_name: None,
            align: TextAlign::Justified,
            universal_frame,
            replay_result,
        });
    }

    (!pending.is_empty()).then(|| ModalBatch::new(pending))
}

pub(super) fn start_active_sherwood_report(
    host: &mut Host,
    ctx: &mut ModalContext<'_>,
    engine: &Engine,
    profiles: &engine_profiles::ProfileManager,
) -> Option<ActivePopupScrollBatch> {
    let Some(resources) = ctx.menu_resources.as_ref() else {
        tracing::warn!("DisplaySherwoodReport: menu resources unavailable — skipped");
        return None;
    };
    let profile = host
        .application_context()
        .active_profile_snapshot()
        .unwrap_or_else(|error| panic!("Sherwood report requires an active profile: {error}"));
    let score_info = ScoreInfo {
        score: profile.score as i32,
        preserved_lives: profile.preserved_lives as i32,
        play_time_seconds: profile.play_time,
    };
    let text = build_sherwood_report_text(
        engine,
        profiles,
        &score_info,
        &resources.menu_text,
        profile.gameplay_config.show_production_forecast,
        profile.gameplay_config.sherwood_trading,
    );
    let kind = engine_player_command::ModalKind::SherwoodReport;
    let replay_result = None;
    let item = PopupScrollItem {
        kind,
        title: None,
        picture: None,
        body: text,
        body_font_name: Some("Debrief".to_string()),
        align: TextAlign::Left,
        universal_frame: engine.frame_counter(),
        replay_result,
    };

    Some(ModalBatch::new(VecDeque::from([item])))
}

fn build_sherwood_report_text(
    engine: &Engine,
    profiles: &engine_profiles::ProfileManager,
    score_info: &ScoreInfo,
    menu_text: &crate::ingame_menu::resources::MenuText,
    show_forecast: bool,
    show_trading: bool,
) -> String {
    let campaign = engine.campaign();
    let sherwood = SherwoodStat;
    if !show_forecast {
        let mut text = sherwood.get_text(
            &campaign.production_sectors,
            &campaign.characters,
            profiles,
            score_info,
            menu_text,
        );
        append_trading_hint(&mut text, menu_text, show_trading);
        return text;
    }

    let live_sectors = engine.live_production_sectors(profiles);
    let forecast_cycle =
        campaign
            .next_mission_idx
            .map_or_else(ProductionForecastCycle::default, |mission_idx| {
                let mission = campaign.missions.get(mission_idx).unwrap_or_else(|| {
                    panic!("selected production-forecast mission index {mission_idx} is missing")
                });
                let mission_profile = mission.profile(profiles);
                ProductionForecastCycle {
                    mission_name: Some(mission_profile.mission_name.clone()),
                    duration_seconds: Some(mission_profile.length),
                }
            });
    let mut text = sherwood.get_text_with_forecast(
        &live_sectors,
        &campaign.characters,
        profiles,
        score_info,
        menu_text,
        Some(&forecast_cycle),
    );
    append_trading_hint(&mut text, menu_text, show_trading);
    text
}

fn append_trading_hint(
    text: &mut String,
    menu_text: &crate::ingame_menu::resources::MenuText,
    show_trading: bool,
) {
    if show_trading {
        text.push_str("\n\n");
        text.push_str(&menu_text.get(crate::ingame_menu::resources::MT_STR_TRADING_HINT));
    }
}

pub(super) fn start_active_debriefing_batch(
    ids: Vec<DebriefingTextId>,
    ctx: &mut ModalContext<'_>,
    text_res: &mut ResourceManager,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
) -> Option<ActiveDebriefingBatch> {
    if ids.is_empty() {
        return None;
    }
    let Some(descriptors) = level_descriptors else {
        tracing::warn!(
            "DisplayDebriefing: level descriptors or menu resources unavailable — \
             dropping {} debriefing(s)",
            ids.len()
        );
        return None;
    };
    if ctx.menu_resources.is_none() {
        tracing::warn!(
            "DisplayDebriefing: level descriptors or menu resources unavailable — \
             dropping {} debriefing(s)",
            ids.len()
        );
        return None;
    }

    let mut pending = VecDeque::new();
    for text_id in ids {
        let (index, table_id, won) = match text_id {
            DebriefingTextId::Lose { index } => {
                (index, descriptors.debriefing.lose_text_table_id, false)
            }
            DebriefingTextId::Win { index } => {
                (index, descriptors.debriefing.win_text_table_id, true)
            }
        };
        match text_res.get_string(table_id, index) {
            Ok(body) => pending.push_back(ActiveDebriefingItem {
                kind: engine_player_command::ModalKind::Debriefing { text_id },
                body: body.to_string(),
                won,
            }),
            Err(error) => {
                tracing::warn!("DisplayDebriefing({text_id:?}): text lookup failed: {error}")
            }
        }
    }

    (!pending.is_empty()).then(|| ModalBatch::new(pending))
}

pub(super) fn tick_active_modal(
    active_modal: &mut Option<ActiveModal>,
    host: &mut Host,
    ctx: &mut ModalContext<'_>,
    replay_modal_dismissals: &mut ReplayModalDismissals,
    engine: &Engine,
    profiles: &engine_profiles::ProfileManager,
) -> ActiveModalOutcome {
    let Some(modal) = active_modal.as_mut() else {
        return ActiveModalOutcome::None;
    };

    match modal {
        ActiveModal::Dialogue(batch) => {
            batch.tick(host, ctx, replay_modal_dismissals);
            if batch.is_empty() {
                *active_modal = None;
            }
            ActiveModalOutcome::None
        }
        ActiveModal::PopupScroll(batch) => {
            batch.tick(host, ctx, replay_modal_dismissals);
            if batch.is_empty() {
                *active_modal = None;
            }
            ActiveModalOutcome::None
        }
        ActiveModal::Debriefing(batch) => {
            batch.tick(host, ctx, replay_modal_dismissals);
            if batch.is_empty() {
                *active_modal = None;
            }
            ActiveModalOutcome::None
        }
        ActiveModal::MissionState {
            kind,
            state,
            replay_result,
            dismissal,
        } => {
            if replay_result.is_none() {
                *replay_result = pop_matching_dismissal(replay_modal_dismissals, kind);
            }
            if replay_result.is_none() {
                let modal_net = host.transport.net().map(|net| {
                    ModalNet::new(
                        net,
                        kind.clone(),
                        host.transport.local_seat() == engine_player_command::PlayerId::HOST,
                    )
                });
                *replay_result = dismissal.poll(modal_net.as_ref());
            }
            if let Some(result) = replay_result.take() {
                ctx.modal_dismissals
                    .push(engine_player_command::PlayerCommand::ModalDismiss {
                        kind: kind.clone(),
                        result,
                    });
                *active_modal = None;
                return match result {
                    engine_player_command::DialogResult::Completed => {
                        ActiveModalOutcome::QuitMissionRequested
                    }
                    engine_player_command::DialogResult::Aborted => ActiveModalOutcome::None,
                    engine_player_command::DialogResult::Restart
                    | engine_player_command::DialogResult::Load { .. } => {
                        tracing::warn!(
                            ?result,
                            "mission-state replay result is only yes/no; treating as aborted"
                        );
                        ActiveModalOutcome::None
                    }
                };
            }
            if dismissal.is_pending() {
                return ActiveModalOutcome::None;
            }
            let ModalContext {
                window,
                renderer,
                cursor_res,
                cursor_renderer,
                menu_resources,
                modal_dismissals,
                ..
            } = ctx;
            let Some(resources) = menu_resources.as_ref() else {
                tracing::warn!("mission-state popup: menu resources unavailable — skipped");
                *active_modal = None;
                return ActiveModalOutcome::None;
            };
            let cursor = default_modal_cursor(cursor_renderer, cursor_res, renderer);
            if let Some(confirmed) = state.tick(window, renderer, resources, Some(cursor)) {
                let result = if confirmed {
                    engine_player_command::DialogResult::Completed
                } else {
                    engine_player_command::DialogResult::Aborted
                };
                let modal_net = host.transport.net().map(|net| {
                    ModalNet::new(
                        net,
                        kind.clone(),
                        host.transport.local_seat() == engine_player_command::PlayerId::HOST,
                    )
                });
                let Some(result) = dismissal.request(result, modal_net.as_ref()) else {
                    return ActiveModalOutcome::None;
                };
                modal_dismissals.push(engine_player_command::PlayerCommand::ModalDismiss {
                    kind: kind.clone(),
                    result,
                });
                *active_modal = None;
                if result == engine_player_command::DialogResult::Completed {
                    ActiveModalOutcome::QuitMissionRequested
                } else {
                    ActiveModalOutcome::None
                }
            } else {
                ActiveModalOutcome::None
            }
        }
        ActiveModal::Trading(state) => {
            let ModalContext {
                window,
                renderer,
                cursor_res,
                cursor_renderer,
                menu_resources,
                ..
            } = ctx;
            let Some(resources) = menu_resources.as_ref() else {
                tracing::warn!("Sherwood trading: menu resources unavailable — closing");
                *active_modal = None;
                return ActiveModalOutcome::None;
            };
            let receipts = host.effects.take_trade_receipts();
            let sectors = engine.live_tradable_production_sectors(profiles);
            let cursor = default_modal_cursor(cursor_renderer, cursor_res, renderer);
            match state.tick(
                window,
                renderer,
                resources,
                receipts,
                &sectors,
                Some(cursor),
            ) {
                Some(TradingOutcome::Close) => {
                    *active_modal = None;
                    ActiveModalOutcome::None
                }
                Some(TradingOutcome::Sell {
                    prod_type,
                    quantity,
                }) => {
                    let request_id = host.effects.allocate_trade_request_id();
                    state.assign_request_id(request_id, prod_type, quantity);
                    ActiveModalOutcome::SellSherwoodItem {
                        request_id,
                        prod_type,
                        quantity,
                    }
                }
                None => ActiveModalOutcome::None,
            }
        }
    }
}

/// Drain script-queued popup-scroll texts for the frame.
///
/// Script natives `DisplayPopupText` and the `DisplayAllPopupTexts`
/// cheat push text IDs onto `pending_popup_texts`.
pub(super) async fn drain_pending_popup_scroll(
    host: &mut Host,
    ctx: &mut ModalContext<'_>,
    text_res: &mut ResourceManager,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    replay_modal_dismissals: &mut ReplayModalDismissals,
    universal_frame: u32,
) {
    // ── Drain pending popup-scroll texts ──
    // Script natives `DisplayPopupText` and the `DisplayAllPopupTexts`
    // cheat push text IDs onto `pending_popup_texts`.
    if host.effects.popup_text_count() != 0 {
        let text_ids: Vec<i32> = host.effects.take_popup_texts();
        if ctx.menu_resources.is_none() {
            // Without `IngameMenuResources` the parchment background, OK
            // button sprite, and font cache are all unavailable — we
            // genuinely cannot render anything, so drop the queue.
            tracing::warn!(
                "DisplayPopupText: menu resources unavailable — dropping {} popup(s)",
                text_ids.len()
            );
            return;
        }
        for text_id in text_ids {
            // Always show a parchment body — when the level
            // resource, text table, or popup-text id can't be
            // resolved, substitute one of the fixed placeholder
            // strings rather than dropping the popup, so a
            // broken-resource scenario still shows the same UI.
            let (text, picture_id) = if let Some(descriptors) = level_descriptors.as_ref() {
                let table_id = descriptors.popup_text.text_table_id;
                let text = usize::try_from(text_id)
                    .ok()
                    .and_then(|index| descriptors.custom_popup_texts.get(index))
                    .and_then(Option::as_ref)
                    .cloned()
                    .unwrap_or_else(|| match text_res.get_string(table_id, text_id as usize) {
                        Ok(s) => s.to_string(),
                        Err(e) => {
                            tracing::warn!("DisplayPopupText({text_id}): text lookup failed: {e}");
                            // Both the missing-text-table and missing-id
                            // branches render the same UI shape; collapse
                            // them to "Invalid popup text ID..." and rely
                            // on the warn log to disambiguate.
                            "Invalid popup text ID...".to_string()
                        }
                    });
                // Look up the picture resource ID.  When the index
                // is in range, return the array entry verbatim —
                // including a literal `0`, which `picture_from` then
                // treats as "no picture widget".  Only an
                // out-of-range index (or a missing descriptor) falls
                // back to `RHID_DEFAULT_POPUP_SCROLL_PICTURE` (164).
                // Per-level popup pictures live in `Level.res`
                // (the same file the text table came from), while
                // the generic default picture lives in DEFAULT.RES
                // — `picture_from` searches both.
                let pid = descriptors
                    .popup_text
                    .picture_ids
                    .get(text_id as usize)
                    .copied()
                    .unwrap_or(RHID_DEFAULT_POPUP_SCROLL_PICTURE);
                (text, pid)
            } else {
                tracing::warn!("DisplayPopupText({text_id}): level descriptors unavailable");
                (
                    "No popup texts for the current level !".to_string(),
                    RHID_DEFAULT_POPUP_SCROLL_PICTURE,
                )
            };
            let picture = ctx
                .menu_resources
                .as_mut()
                .expect("checked above")
                .picture_from(ctx.renderer, text_res, picture_id);
            let kind = engine_player_command::ModalKind::PopupText { text_id };
            let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
            let modal_net = host.transport.net().map(|net| {
                ModalNet::new(
                    net,
                    kind.clone(),
                    host.transport.local_seat() == engine_player_command::PlayerId::HOST,
                )
            });
            let item = PopupScrollItem {
                kind: kind.clone(),
                title: None,
                picture,
                body: text,
                body_font_name: None,
                align: TextAlign::Justified,
                universal_frame,
                replay_result,
            };
            let result =
                ingame_menu::show_popup_scroll(ctx, &mut host.audio.sound, modal_net, item).await;
            ctx.modal_dismissals
                .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
        }
    }
}

/// Drain a script-queued Sherwood stat report for the frame.
///
/// Script native `DisplaySherwoodReport` sets `pending_sherwood_report`.
pub(super) async fn drain_pending_sherwood_stat(
    host: &mut Host,
    ctx: &mut ModalContext<'_>,
    engine: &Engine,
    profiles: &engine_profiles::ProfileManager,
    replay_modal_dismissals: &mut ReplayModalDismissals,
) {
    // ── Drain pending Sherwood stat report ──
    // Script native `DisplaySherwoodReport` sets
    // `pending_sherwood_report`.
    if host.effects.take_sherwood_report() {
        if let Some(resources) = ctx.menu_resources.as_ref() {
            // The Sherwood stat panel pulls score / preserved lives
            // / play time from the active player profile.
            let profile = host
                .application_context()
                .active_profile_snapshot()
                .unwrap_or_else(|error| {
                    panic!("Sherwood report requires an active profile: {error}")
                });
            let score_info = ScoreInfo {
                score: profile.score as i32,
                preserved_lives: profile.preserved_lives as i32,
                play_time_seconds: profile.play_time,
            };
            let text = build_sherwood_report_text(
                engine,
                profiles,
                &score_info,
                &resources.menu_text,
                profile.gameplay_config.show_production_forecast,
                profile.gameplay_config.sherwood_trading,
            );
            let kind = engine_player_command::ModalKind::SherwoodReport;
            let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
            let modal_net = host.transport.net().map(|net| {
                ModalNet::new(
                    net,
                    kind.clone(),
                    host.transport.local_seat() == engine_player_command::PlayerId::HOST,
                )
            });
            // The Sherwood report uses the "Debrief" font and is
            // left-aligned (not the popup-scroll default).
            let item = PopupScrollItem {
                kind: kind.clone(),
                title: None,
                picture: None,
                body: text,
                body_font_name: Some("Debrief".to_string()),
                align: TextAlign::Left,
                universal_frame: engine.frame_counter(),
                replay_result,
            };
            let result =
                ingame_menu::show_popup_scroll(ctx, &mut host.audio.sound, modal_net, item).await;
            ctx.modal_dismissals
                .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
        } else {
            tracing::warn!(
                "DisplaySherwoodReport: campaign or menu resources unavailable — skipped"
            );
        }
    }
}

/// Drain cheat-queued debriefing requests for the frame.
///
/// Cheat `DisplayAllDebriefings` pushes typed text IDs onto
/// `pending_debriefings`.
pub(super) async fn drain_pending_debriefings(
    host: &mut Host,
    ctx: &mut ModalContext<'_>,
    text_res: &mut ResourceManager,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    replay_modal_dismissals: &mut ReplayModalDismissals,
) {
    // ── Drain pending debriefing requests ──
    // The lose phase and win phase run as two distinct calls — each
    // starts with a fresh emergency-end state, so an EmergencyEnd in
    // the lose phase breaks only the lose loop and the win phase
    // still runs.  We replicate that by partitioning the typed queue
    // into a lose phase and a win phase and iterating each
    // independently.
    if host.effects.debriefing_count() != 0 {
        let ids: Vec<DebriefingTextId> = host.effects.take_debriefings();
        if let Some(descriptors) = level_descriptors
            && ctx.menu_resources.is_some()
        {
            let (lose_ids, win_ids): (Vec<_>, Vec<_>) = ids
                .into_iter()
                .partition(|text_id| matches!(text_id, DebriefingTextId::Lose { .. }));

            // Lose phase: one pass over the queued lose texts.
            for text_id in lose_ids {
                let DebriefingTextId::Lose { index } = text_id else {
                    unreachable!("lose_ids was partitioned from DebriefingTextId::Lose");
                };
                let kind = engine_player_command::ModalKind::Debriefing { text_id };
                let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
                let table_id = descriptors.debriefing.lose_text_table_id;
                let text = match text_res.get_string(table_id, index) {
                    Ok(s) => s.to_string(),
                    Err(e) => {
                        tracing::warn!("DisplayDebriefing({text_id:?}): text lookup failed: {e}");
                        continue;
                    }
                };
                let debrief_outcome = if let Some(result) = replay_result {
                    debriefing_replay_result(result)
                } else {
                    // The `DisplayAllDebriefings` cheat iterates
                    // debriefing texts but never invokes the stat
                    // overload — stats don't appear in this flow, so
                    // pass `None`.
                    let ModalContext {
                        window,
                        renderer,
                        cursor_res,
                        cursor_renderer,
                        menu_resources,
                        ..
                    } = &mut *ctx;
                    let resources = menu_resources.as_ref().expect("checked above");
                    let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                    ingame_menu::show_debriefing(
                        window, renderer, resources, cursor, &text, None, 0, false, false,
                        // Cheat path passes no restart, so the
                        // quick-load translator is never enabled.
                        None, false, false,
                    )
                    .await
                };
                let result = if matches!(debrief_outcome, DebriefingOutcome::EmergencyEnd) {
                    engine_player_command::DialogResult::Aborted
                } else {
                    engine_player_command::DialogResult::Completed
                };
                ctx.modal_dismissals
                    .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
                // The iteration breaks out when an emergency-end
                // fires — but only for THIS phase, not the win phase
                // below.
                if matches!(debrief_outcome, DebriefingOutcome::EmergencyEnd) {
                    break;
                }
            }

            // Win phase: a fresh pass over the queued win texts.
            for text_id in win_ids {
                let DebriefingTextId::Win { index } = text_id else {
                    unreachable!("win_ids was partitioned from DebriefingTextId::Win");
                };
                let kind = engine_player_command::ModalKind::Debriefing { text_id };
                let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
                let table_id = descriptors.debriefing.win_text_table_id;
                let text = match text_res.get_string(table_id, index) {
                    Ok(s) => s.to_string(),
                    Err(e) => {
                        tracing::warn!("DisplayDebriefing({text_id:?}): text lookup failed: {e}");
                        continue;
                    }
                };
                let debrief_outcome = if let Some(result) = replay_result {
                    debriefing_replay_result(result)
                } else {
                    let ModalContext {
                        window,
                        renderer,
                        cursor_res,
                        cursor_renderer,
                        menu_resources,
                        ..
                    } = &mut *ctx;
                    let resources = menu_resources.as_ref().expect("checked above");
                    let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                    ingame_menu::show_debriefing(
                        window, renderer, resources, cursor, &text, None, 0, true, false, None,
                        false, false,
                    )
                    .await
                };
                let result = if matches!(debrief_outcome, DebriefingOutcome::EmergencyEnd) {
                    engine_player_command::DialogResult::Aborted
                } else {
                    engine_player_command::DialogResult::Completed
                };
                ctx.modal_dismissals
                    .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
                if matches!(debrief_outcome, DebriefingOutcome::EmergencyEnd) {
                    break;
                }
            }
        } else {
            tracing::warn!(
                "DisplayDebriefing: level descriptors or menu resources unavailable — \
                 dropping {} debriefing(s)",
                ids.len()
            );
        }
    }
}

/// Build [`DialogueSentence`]s from a dialogue descriptor and the
/// resource manager that holds the text / wave tables.
///
/// Pre-builds the full array up front so `show_dialogue` can run its
/// own event loop without needing the resource manager.
fn build_dialogue_sentences(
    dialog_id: i32,
    descriptors: &assets_res_descr::LevelDescriptors,
    res: &mut ResourceManager,
    text_directory: &str,
) -> Vec<DialogueSentence> {
    // Project convention: panic on missing data rather than fall
    // back to a hard-coded default.  An empty `text_directory` means
    // the global-options holder wasn't initialized.
    assert!(
        !text_directory.is_empty(),
        "global_options.text_directory must be set before dialogue playback"
    );

    let idx = dialog_id as usize;
    let Some(desc) = descriptors.dialogues.get(idx) else {
        // When the dialogue descriptor is missing, still open the
        // dialogue and display a single placeholder sentence so the
        // player sees *why* nothing happened.  Portrait index falls
        // through to the "bad portrait" slot via clamping.
        tracing::warn!(
            "StartDialog({dialog_id}): no descriptor (level has {} dialogues)",
            descriptors.dialogues.len()
        );
        return vec![DialogueSentence {
            portrait_index: usize::MAX,
            text: "Invalid dialogue ID...".to_string(),
            sound_path: String::new(),
        }];
    };

    let sentence_count = desc.portrait_ids.len();
    let custom_sentences = descriptors
        .custom_dialogue_texts
        .get(idx)
        .and_then(Option::as_ref);
    let mut sentences = Vec::with_capacity(sentence_count);

    for i in 0..sentence_count {
        // Missing text is still rendered, not skipped — the user
        // needs to see that something broke and step through it.
        let mut text = custom_sentences
            .and_then(|sentences| sentences.get(i))
            .cloned()
            .unwrap_or_else(|| match res.get_string(desc.text_table_id, i) {
                Ok(s) => s.to_string(),
                Err(e) => {
                    tracing::warn!("Dialogue {dialog_id} sentence {i}: text lookup failed: {e}");
                    "Unable to retrieve the sentence text : invalide resource !".to_string()
                }
            });

        // When the sample lookup fails the error is *appended* to
        // the visible text (preserving the dialogue's normal text
        // above it) and the sound path is left empty so playback is
        // skipped.
        let sound_path = if custom_sentences.is_some() {
            String::new()
        } else {
            match res.get_sample(desc.sound_table_id, i) {
                Ok(s) => format!("{text_directory}/{s}"),
                Err(e) => {
                    tracing::debug!("Dialogue {dialog_id} sentence {i}: sound lookup failed: {e}");
                    text.push_str("Unable to retreive the sentence sound : invalide resource !");
                    String::new()
                }
            }
        };

        let portrait_index = desc.portrait_ids[i] as usize;

        sentences.push(DialogueSentence {
            portrait_index,
            text,
            sound_path,
        });
    }

    tracing::info!("Built dialogue {dialog_id}: {} sentences", sentences.len());
    sentences
}

#[cfg(test)]
mod tests {
    use super::{ModalScreenAdmission, ReplayModalDismissals, pop_matching_dismissal};
    use robin_engine::player_command::{
        DebriefingTextId, DialogResult, MissionStateModalKind, ModalKind, PlayerCommand,
    };
    use std::collections::VecDeque;

    #[test]
    fn strict_replay_waits_for_later_recorded_control_for_every_scripted_lane() {
        for kind in [
            ModalKind::PopupText { text_id: 0 },
            ModalKind::Dialog { dialog_id: 0 },
            ModalKind::Debriefing {
                text_id: DebriefingTextId::Win { index: 0 },
            },
        ] {
            let mut created_frame = ReplayModalDismissals::default();
            created_frame.begin_replay_frame();
            // Empty recorded control means presentation-only, never a local
            // screen step (which could accept Return or an audio timeout).
            assert_eq!(
                created_frame.admit_screen(&kind),
                ModalScreenAdmission::AwaitRecorded
            );
            created_frame.assert_consumed();
            let mut dismissal_frame = ReplayModalDismissals::default();
            dismissal_frame.begin_replay_frame();
            dismissal_frame.push_back(PlayerCommand::ModalDismiss {
                kind: kind.clone(),
                result: DialogResult::Completed,
            });
            assert_eq!(
                dismissal_frame.admit_screen(&kind),
                ModalScreenAdmission::Recorded(DialogResult::Completed)
            );
            assert!(dismissal_frame.is_empty());
            dismissal_frame.assert_consumed();
        }
    }

    #[test]
    fn live_modal_without_recorded_control_remains_interactive() {
        assert_eq!(
            ReplayModalDismissals::default().admit_screen(&ModalKind::PopupText { text_id: 0 }),
            ModalScreenAdmission::Interactive
        );
    }

    #[test]
    #[should_panic(expected = "recorded modal dismissal(s) were unused")]
    fn strict_replay_preserves_unmatched_control_for_desync_check() {
        let mut frame = ReplayModalDismissals::default();
        frame.begin_replay_frame();
        frame.push_back(PlayerCommand::ModalDismiss {
            kind: ModalKind::PopupText { text_id: 1 },
            result: DialogResult::Aborted,
        });
        assert_eq!(
            frame.admit_screen(&ModalKind::PopupText { text_id: 0 }),
            ModalScreenAdmission::AwaitRecorded
        );
        assert_eq!(frame.len(), 1);
        frame.assert_consumed();
    }

    #[test]
    fn pop_matching_dismissal_removes_only_matching_modal() {
        let mut queue: ReplayModalDismissals = VecDeque::from([
            PlayerCommand::ModalDismiss {
                kind: ModalKind::PopupText { text_id: 7 },
                result: DialogResult::Completed,
            },
            PlayerCommand::ModalDismiss {
                kind: ModalKind::Debriefing {
                    text_id: DebriefingTextId::Lose { index: 1 },
                },
                result: DialogResult::Aborted,
            },
            PlayerCommand::ModalDismiss {
                kind: ModalKind::MissionState {
                    kind: MissionStateModalKind::LeaveMissionNow,
                },
                result: DialogResult::Completed,
            },
        ])
        .into();

        let result = pop_matching_dismissal(
            &mut queue,
            &ModalKind::Debriefing {
                text_id: DebriefingTextId::Lose { index: 1 },
            },
        );

        assert_eq!(result, Some(DialogResult::Aborted));
        assert_eq!(queue.len(), 2);
        assert!(matches!(
            queue.iter().next().unwrap(),
            PlayerCommand::ModalDismiss {
                kind: ModalKind::PopupText { text_id: 7 },
                ..
            }
        ));
        assert!(matches!(
            queue.iter().nth(1).unwrap(),
            PlayerCommand::ModalDismiss {
                kind: ModalKind::MissionState {
                    kind: MissionStateModalKind::LeaveMissionNow,
                },
                ..
            }
        ));
    }

    #[test]
    fn pop_matching_dismissal_leaves_unmatched_queue_intact() {
        let mut queue: ReplayModalDismissals = VecDeque::from([PlayerCommand::ModalDismiss {
            kind: ModalKind::Debriefing {
                text_id: DebriefingTextId::Win { index: 1 },
            },
            result: DialogResult::Completed,
        }])
        .into();

        let result = pop_matching_dismissal(
            &mut queue,
            &ModalKind::Debriefing {
                text_id: DebriefingTextId::Lose { index: 0 },
            },
        );

        assert_eq!(result, None);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn missing_control_keeps_modal_open_until_later_replay_frame() {
        let kind = ModalKind::Dialog { dialog_id: 3 };
        let mut creation_frame = ReplayModalDismissals::default();
        creation_frame.begin_replay_frame();

        assert_eq!(pop_matching_dismissal(&mut creation_frame, &kind), None);
        creation_frame.assert_consumed();

        let mut dismissal_frame: ReplayModalDismissals =
            VecDeque::from([PlayerCommand::ModalDismiss {
                kind: kind.clone(),
                result: DialogResult::Aborted,
            }])
            .into();
        dismissal_frame.begin_replay_frame();

        assert_eq!(
            pop_matching_dismissal(&mut dismissal_frame, &kind),
            Some(DialogResult::Aborted)
        );
        dismissal_frame.assert_consumed();
    }

    #[test]
    #[should_panic(expected = "recorded modal dismissal(s) were unused in their host frame")]
    fn supplied_but_unmatched_control_is_a_replay_desync() {
        let mut queue: ReplayModalDismissals = VecDeque::from([PlayerCommand::ModalDismiss {
            kind: ModalKind::Dialog { dialog_id: 4 },
            result: DialogResult::Completed,
        }])
        .into();
        queue.begin_replay_frame();

        queue.assert_consumed();
    }
}

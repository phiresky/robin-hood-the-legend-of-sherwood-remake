//! Modal-state machinery: dialogue / popup-scroll / debriefing /
//! mission-state batches, the unified `ActiveModal` enum, and the
//! `start_/tick_/drain_pending_*` helpers that drive them.

use crate::audio_backend::KiraAudioBackend;
use crate::console_overlay::ConsoleOverlay;
use crate::cursor::CursorRenderer;
use crate::game::Game;
use crate::host::Host;
use crate::host::HostSignal;
use crate::ingame_menu::widget_bridge::default_modal_cursor;
use crate::ingame_menu::{
    self, DebriefingModalState, DebriefingOutcome, DialogueModalState, DialogueSentence,
    IngameMenuResources, MissionStatePopupState, ModalNet, PopupScrollItem, PopupScrollModalState,
    layout::TextAlign,
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
use robin_engine::sherwood_stat::{ScoreInfo, SherwoodStat};
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

    fn item_kind(item: &Self::Item) -> engine_player_command::ModalKind;
    fn item_replay_result(item: &Self::Item) -> Option<engine_player_command::DialogResult>;

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

    /// Map the screen's outcome onto the replay-recorded result.
    fn to_result(outcome: &Self::Outcome) -> engine_player_command::DialogResult;

    /// Lane-specific reaction to a dismissal (e.g. dropping the queued
    /// remainder on an emergency end).  Default: nothing.
    fn on_dismiss(_outcome: &Self::Outcome, _pending: &mut VecDeque<Self::Item>) {}
}

/// A queue of modal items plus the currently open screen, advanced one
/// frame per [`Self::tick`] call.
pub(super) struct ModalBatch<S: ModalScreen> {
    pending: VecDeque<S::Item>,
    current: Option<(engine_player_command::ModalKind, S)>,
}

pub(super) type ActiveDialogueBatch = ModalBatch<DialogueModalState>;
pub(super) type ActivePopupScrollBatch = ModalBatch<PopupScrollModalState>;
pub(super) type ActiveDebriefingBatch = ModalBatch<DebriefingModalState>;

impl<S: ModalScreen> ModalBatch<S> {
    fn new(pending: VecDeque<S::Item>) -> Self {
        Self {
            pending,
            current: None,
        }
    }

    fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.current.is_none()
    }

    fn tick(&mut self, host: &mut Host, ctx: &mut ModalContext<'_>) {
        if ctx.menu_resources.is_none() {
            tracing::warn!("{}", S::MISSING_RESOURCES_WARN);
            self.pending.clear();
            self.current = None;
            return;
        }

        if self.current.is_none()
            && let Some(item) = self.pending.pop_front()
        {
            if let Some(result) = S::item_replay_result(&item) {
                ctx.modal_dismissals
                    .push(engine_player_command::PlayerCommand::ModalDismiss {
                        kind: S::item_kind(&item),
                        result,
                    });
            } else {
                let kind = S::item_kind(&item);
                let screen = S::begin(host, ctx, item);
                self.current = Some((kind, screen));
            }
        }

        let Some((kind, screen)) = self.current.as_mut() else {
            return;
        };

        if let Some(outcome) = screen.step(kind, host, ctx) {
            let result = S::to_result(&outcome);
            ctx.modal_dismissals
                .push(engine_player_command::PlayerCommand::ModalDismiss {
                    kind: kind.clone(),
                    result,
                });
            S::on_dismiss(&outcome, &mut self.pending);
            self.current = None;
        }
    }
}

pub(super) struct ActiveDialogueItem {
    kind: engine_player_command::ModalKind,
    sentences: Vec<DialogueSentence>,
    replay_result: Option<engine_player_command::DialogResult>,
}

impl ModalScreen for DialogueModalState {
    type Item = ActiveDialogueItem;
    type Outcome = engine_player_command::DialogResult;
    const MISSING_RESOURCES_WARN: &'static str =
        "DisplayDialog: menu resources unavailable — dropping active dialogue";

    fn item_kind(item: &Self::Item) -> engine_player_command::ModalKind {
        item.kind.clone()
    }

    fn item_replay_result(item: &Self::Item) -> Option<engine_player_command::DialogResult> {
        item.replay_result
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
        let modal_net = host
            .transport
            .net
            .as_ref()
            .map(|net| ModalNet::new(net, kind.clone()));
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
}

impl ModalScreen for PopupScrollModalState {
    type Item = PopupScrollItem;
    type Outcome = engine_player_command::DialogResult;
    const MISSING_RESOURCES_WARN: &'static str =
        "DisplayPopupText: menu resources unavailable — dropping active popup";

    fn item_kind(item: &Self::Item) -> engine_player_command::ModalKind {
        item.kind.clone()
    }

    fn item_replay_result(item: &Self::Item) -> Option<engine_player_command::DialogResult> {
        item.replay_result
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
        let modal_net = host
            .transport
            .net
            .as_ref()
            .map(|net| ModalNet::new(net, kind.clone()));
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
}

pub(super) struct ActiveDebriefingItem {
    kind: engine_player_command::ModalKind,
    body: String,
    won: bool,
    replay_result: Option<engine_player_command::DialogResult>,
}

impl ModalScreen for DebriefingModalState {
    type Item = ActiveDebriefingItem;
    type Outcome = DebriefingOutcome;
    const MISSING_RESOURCES_WARN: &'static str =
        "DisplayDebriefing: menu resources unavailable — dropping active debriefing";

    fn item_kind(item: &Self::Item) -> engine_player_command::ModalKind {
        item.kind.clone()
    }

    fn item_replay_result(item: &Self::Item) -> Option<engine_player_command::DialogResult> {
        item.replay_result
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

    fn on_dismiss(outcome: &Self::Outcome, pending: &mut VecDeque<Self::Item>) {
        if matches!(outcome, DebriefingOutcome::EmergencyEnd) {
            // We flatten the queued phase ordering, so dropping the
            // remaining items is the conservative no-surprise
            // behavior on an external close.
            pending.clear();
        }
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
    },
}

impl ActiveModal {
    pub(super) fn is_empty(&self) -> bool {
        match self {
            ActiveModal::Dialogue(batch) => batch.is_empty(),
            ActiveModal::PopupScroll(batch) => batch.is_empty(),
            ActiveModal::Debriefing(batch) => batch.is_empty(),
            ActiveModal::MissionState { .. } => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActiveModalOutcome {
    None,
    QuitMissionRequested,
}

/// Run a game session: mission selection loop -> game -> repeat.
///
/// Build the auto-assigned replay recording path used when the user
/// starts the game without `--record`: `<data_dir>/robin_hood/replays/`
/// joined with a local-time ISO-8601 stamp including the timezone
/// offset (colons replaced with `-` so the filename works on every
/// filesystem — Windows in particular rejects `:`).  The directory
/// is created lazily on first write.
/// Per-frame queue of recorded modal dismissals during replay playback.
///
/// Serializes transparently as the plain command queue so snapshot and
/// save formats are unchanged. `strict_replay` is stamped on frames fed
/// from a replay so missing modal facts fail immediately instead of fabricating
/// a result and hiding a replay desynchronization.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub(super) struct ReplayModalDismissals {
    queue: std::collections::VecDeque<engine_player_command::PlayerCommand>,
    #[serde(skip)]
    pub(super) strict_replay: bool,
}

impl From<std::collections::VecDeque<engine_player_command::PlayerCommand>>
    for ReplayModalDismissals
{
    fn from(queue: std::collections::VecDeque<engine_player_command::PlayerCommand>) -> Self {
        Self {
            queue,
            strict_replay: false,
        }
    }
}

impl ReplayModalDismissals {
    pub(super) fn push_back(&mut self, command: engine_player_command::PlayerCommand) {
        self.queue.push_back(command);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.queue.len()
    }

    pub(super) fn is_strict_replay(&self) -> bool {
        self.strict_replay
    }
}

/// Pop the first `ModalDismiss` whose `kind` matches the target out of
/// the per-frame replay dismissal queue, returning the recorded result.
///
/// Matching by kind keeps the queue stable even if the engine queues
/// modals in a slightly different order within a frame (e.g. a dialog
/// and a popup both fired), and lets an unrelated modal without a
/// recording fall through to interactive handling. Playback-fed frames are
/// strict: an unrecorded modal is a replay mismatch and cannot be assigned a
/// fabricated result.
pub(super) fn pop_matching_dismissal(
    queue: &mut ReplayModalDismissals,
    target: &engine_player_command::ModalKind,
) -> Option<engine_player_command::DialogResult> {
    let pos = queue.queue.iter().position(|c| {
        matches!(
            c,
            engine_player_command::PlayerCommand::ModalDismiss { kind, .. }
                if kind == target
        )
    });
    let Some(pos) = pos else {
        if queue.strict_replay {
            panic!("replay desync: modal {target:?} has no recorded dismissal");
        }
        return None;
    };
    match queue.queue.remove(pos)? {
        engine_player_command::PlayerCommand::ModalDismiss { result, .. } => Some(result),
        _ => None,
    }
}

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
    if host.effects.take_signal(HostSignal::ShowConsole) {
        if !console_overlay.is_visible() {
            let now_visible = console_overlay.toggle();
            if now_visible {
                start_text_input();
            }
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
    host: &mut Host,
    text_res: &mut ResourceManager,
    game: &Game,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    replay_modal_dismissals: &mut ReplayModalDismissals,
) -> Option<ActiveDialogueBatch> {
    if host.effects.dialogue_count() == 0 {
        return None;
    }
    let Some(descriptors) = level_descriptors else {
        tracing::warn!(
            "DisplayDialog: level descriptors unavailable — dropping {} dialogue(s)",
            host.effects.dialogue_count()
        );
        drop(host.effects.take_dialogues());
        return None;
    };

    let dialog_ids: Vec<i32> = host.effects.take_dialogues();
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
        let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
        pending.push_back(ActiveDialogueItem {
            kind,
            sentences,
            replay_result,
        });
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
                    let modal_net = host
                        .transport
                        .net
                        .as_ref()
                        .map(|net| ModalNet::new(net, kind.clone()));
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
    host: &mut Host,
    ctx: &mut ModalContext<'_>,
    text_res: &mut ResourceManager,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    replay_modal_dismissals: &mut ReplayModalDismissals,
    universal_frame: u32,
) -> Option<ActivePopupScrollBatch> {
    if host.effects.popup_text_count() == 0 {
        return None;
    }
    let text_ids: Vec<i32> = host.effects.take_popup_texts();
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
            let text = match text_res.get_string(table_id, text_id as usize) {
                Ok(s) => s.to_string(),
                Err(e) => {
                    tracing::warn!("DisplayPopupText({text_id}): text lookup failed: {e}");
                    "Invalid popup text ID...".to_string()
                }
            };
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
    replay_modal_dismissals: &mut ReplayModalDismissals,
) -> Option<ActivePopupScrollBatch> {
    if !host.effects.take_sherwood_report() {
        return None;
    }
    let Some(resources) = ctx.menu_resources.as_ref() else {
        tracing::warn!("DisplaySherwoodReport: menu resources unavailable — skipped");
        return None;
    };
    let campaign = engine.campaign();

    let sherwood = SherwoodStat;
    let profile = host
        .application_context
        .active_profile_snapshot()
        .unwrap_or_else(|error| panic!("Sherwood report requires an active profile: {error}"));
    let score_info = ScoreInfo {
        score: profile.score as i32,
        preserved_lives: profile.preserved_lives as i32,
        play_time_seconds: profile.play_time,
    };
    let text = sherwood.get_text(
        &campaign.production_sectors,
        &campaign.characters,
        profiles,
        &score_info,
        &resources.menu_text,
    );
    let kind = engine_player_command::ModalKind::SherwoodReport;
    let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
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

pub(super) fn start_active_debriefing_batch(
    host: &mut Host,
    ctx: &mut ModalContext<'_>,
    text_res: &mut ResourceManager,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    replay_modal_dismissals: &mut ReplayModalDismissals,
) -> Option<ActiveDebriefingBatch> {
    if host.effects.debriefing_count() == 0 {
        return None;
    }
    let ids: Vec<DebriefingTextId> = host.effects.take_debriefings();
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

    let (lose_ids, win_ids): (Vec<_>, Vec<_>) = ids
        .into_iter()
        .partition(|text_id| matches!(text_id, DebriefingTextId::Lose { .. }));
    let mut pending = VecDeque::new();
    for text_id in lose_ids {
        let DebriefingTextId::Lose { index } = text_id else {
            unreachable!("lose_ids was partitioned from DebriefingTextId::Lose");
        };
        let table_id = descriptors.debriefing.lose_text_table_id;
        match text_res.get_string(table_id, index) {
            Ok(s) => {
                let kind = engine_player_command::ModalKind::Debriefing { text_id };
                let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
                pending.push_back(ActiveDebriefingItem {
                    kind,
                    body: s.to_string(),
                    won: false,
                    replay_result,
                });
            }
            Err(e) => tracing::warn!("DisplayDebriefing({text_id:?}): text lookup failed: {e}"),
        }
    }
    for text_id in win_ids {
        let DebriefingTextId::Win { index } = text_id else {
            unreachable!("win_ids was partitioned from DebriefingTextId::Win");
        };
        let table_id = descriptors.debriefing.win_text_table_id;
        match text_res.get_string(table_id, index) {
            Ok(s) => {
                let kind = engine_player_command::ModalKind::Debriefing { text_id };
                let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
                pending.push_back(ActiveDebriefingItem {
                    kind,
                    body: s.to_string(),
                    won: true,
                    replay_result,
                });
            }
            Err(e) => tracing::warn!("DisplayDebriefing({text_id:?}): text lookup failed: {e}"),
        }
    }

    (!pending.is_empty()).then(|| ModalBatch::new(pending))
}

pub(super) fn tick_active_modal(
    active_modal: &mut Option<ActiveModal>,
    host: &mut Host,
    ctx: &mut ModalContext<'_>,
) -> ActiveModalOutcome {
    let Some(modal) = active_modal.as_mut() else {
        return ActiveModalOutcome::None;
    };

    match modal {
        ActiveModal::Dialogue(batch) => {
            batch.tick(host, ctx);
            if batch.is_empty() {
                *active_modal = None;
            }
            ActiveModalOutcome::None
        }
        ActiveModal::PopupScroll(batch) => {
            batch.tick(host, ctx);
            if batch.is_empty() {
                *active_modal = None;
            }
            ActiveModalOutcome::None
        }
        ActiveModal::Debriefing(batch) => {
            batch.tick(host, ctx);
            if batch.is_empty() {
                *active_modal = None;
            }
            ActiveModalOutcome::None
        }
        ActiveModal::MissionState {
            kind,
            state,
            replay_result,
        } => {
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
                modal_dismissals.push(engine_player_command::PlayerCommand::ModalDismiss {
                    kind: kind.clone(),
                    result,
                });
                *active_modal = None;
                if confirmed {
                    ActiveModalOutcome::QuitMissionRequested
                } else {
                    ActiveModalOutcome::None
                }
            } else {
                ActiveModalOutcome::None
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
                let text = match text_res.get_string(table_id, text_id as usize) {
                    Ok(s) => s.to_string(),
                    Err(e) => {
                        tracing::warn!("DisplayPopupText({text_id}): text lookup failed: {e}");
                        // Both the missing-text-table and missing-id
                        // branches render the same UI shape; collapse
                        // them to "Invalid popup text ID..." and rely
                        // on the warn log to disambiguate.
                        "Invalid popup text ID...".to_string()
                    }
                };
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
            let modal_net = host
                .transport
                .net
                .as_ref()
                .map(|net| ModalNet::new(net, kind.clone()));
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
            let campaign = engine.campaign();
            let sherwood = SherwoodStat;
            // The Sherwood stat panel pulls score / preserved lives
            // / play time from the active player profile.
            let profile = host
                .application_context
                .active_profile_snapshot()
                .unwrap_or_else(|error| {
                    panic!("Sherwood report requires an active profile: {error}")
                });
            let score_info = ScoreInfo {
                score: profile.score as i32,
                preserved_lives: profile.preserved_lives as i32,
                play_time_seconds: profile.play_time,
            };
            let text = sherwood.get_text(
                &campaign.production_sectors,
                &campaign.characters,
                profiles,
                &score_info,
                &resources.menu_text,
            );
            let kind = engine_player_command::ModalKind::SherwoodReport;
            let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
            let modal_net = host
                .transport
                .net
                .as_ref()
                .map(|net| ModalNet::new(net, kind.clone()));
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
    let mut sentences = Vec::with_capacity(sentence_count);

    for i in 0..sentence_count {
        // Missing text is still rendered, not skipped — the user
        // needs to see that something broke and step through it.
        let mut text = match res.get_string(desc.text_table_id, i) {
            Ok(s) => s.to_string(),
            Err(e) => {
                tracing::warn!("Dialogue {dialog_id} sentence {i}: text lookup failed: {e}");
                "Unable to retrieve the sentence text : invalide resource !".to_string()
            }
        };

        // When the sample lookup fails the error is *appended* to
        // the visible text (preserving the dialogue's normal text
        // above it) and the sound path is left empty so playback is
        // skipped.
        let sound_path = match res.get_sample(desc.sound_table_id, i) {
            Ok(s) => format!("{text_directory}/{s}"),
            Err(e) => {
                tracing::debug!("Dialogue {dialog_id} sentence {i}: sound lookup failed: {e}");
                text.push_str("Unable to retreive the sentence sound : invalide resource !");
                String::new()
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
    use super::{ReplayModalDismissals, pop_matching_dismissal};
    use robin_engine::player_command::{
        DebriefingTextId, DialogResult, MissionStateModalKind, ModalKind, PlayerCommand,
    };
    use std::collections::VecDeque;

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
            queue.queue[0],
            PlayerCommand::ModalDismiss {
                kind: ModalKind::PopupText { text_id: 7 },
                ..
            }
        ));
        assert!(matches!(
            queue.queue[1],
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
    #[should_panic(expected = "replay desync: modal")]
    fn unrecorded_modal_is_rejected_on_playback_frames() {
        let mut queue = ReplayModalDismissals::default();
        queue.strict_replay = true;

        let _ = pop_matching_dismissal(&mut queue, &ModalKind::Dialog { dialog_id: 3 });
    }

    #[test]
    fn recorded_dismissal_is_consumed_on_strict_playback() {
        let mut queue: ReplayModalDismissals = VecDeque::from([PlayerCommand::ModalDismiss {
            kind: ModalKind::Dialog { dialog_id: 3 },
            result: DialogResult::Aborted,
        }])
        .into();
        queue.strict_replay = true;

        let result = pop_matching_dismissal(&mut queue, &ModalKind::Dialog { dialog_id: 3 });

        assert_eq!(result, Some(DialogResult::Aborted));
        assert!(queue.is_empty());
    }
}

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
    self, BatchDialogue, DebriefingModalState, DebriefingOutcome, DialogueModalState,
    DialogueSentence, IngameMenuResources, MenuSurface, MissionStatePopupState, ModalNet,
    PopupScrollModalState, layout::TextAlign,
};
use crate::renderer::Renderer;
use crate::window::{GameWindow, start_text_input};
use robin_assets::res_descr as assets_res_descr;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::engine::Engine;
use robin_engine::player_command as engine_player_command;
use robin_engine::player_command::DebriefingTextId;
use robin_engine::profiles as engine_profiles;
use robin_engine::replay::ReplayRecorder;
use robin_engine::resource_ids::RHID_DEFAULT_POPUP_SCROLL_PICTURE;
use robin_engine::sherwood_stat::{ScoreInfo, SherwoodStat};
use robin_engine::sound_cache::SampleLoader;
use robin_engine::sound_config::SoundConfig;
use std::collections::VecDeque;

pub(super) struct ActiveDialogueItem {
    dialog_id: i32,
    kind: engine_player_command::ModalKind,
    sentences: Vec<DialogueSentence>,
    replay_result: Option<engine_player_command::DialogResult>,
}

pub(super) struct ActiveDialogueBatch {
    pending: VecDeque<ActiveDialogueItem>,
    current: Option<(i32, engine_player_command::ModalKind, DialogueModalState)>,
}

impl ActiveDialogueBatch {
    fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.current.is_none()
    }
}

pub(super) struct ActivePopupScrollItem {
    kind: engine_player_command::ModalKind,
    title: Option<String>,
    picture: Option<MenuSurface>,
    body: String,
    body_font_name: Option<String>,
    align: TextAlign,
    universal_frame: u32,
    replay_result: Option<engine_player_command::DialogResult>,
}

pub(super) struct ActivePopupScrollBatch {
    pending: VecDeque<ActivePopupScrollItem>,
    current: Option<(engine_player_command::ModalKind, PopupScrollModalState)>,
}

impl ActivePopupScrollBatch {
    fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.current.is_none()
    }
}

pub(super) struct ActiveDebriefingItem {
    kind: engine_player_command::ModalKind,
    body: String,
    won: bool,
    replay_result: Option<engine_player_command::DialogResult>,
}

pub(super) struct ActiveDebriefingBatch {
    pending: VecDeque<ActiveDebriefingItem>,
    current: Option<(engine_player_command::ModalKind, DebriefingModalState)>,
}

impl ActiveDebriefingBatch {
    fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.current.is_none()
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
/// Pop the first `ModalDismiss` whose `kind` matches the target out of
/// the per-frame replay dismissal queue, returning the recorded result.
///
/// Matching by kind keeps the queue stable even if the engine queues
/// modals in a slightly different order within a frame (e.g. a dialog
/// and a popup both fired), and lets an unrelated modal without a
/// recording fall through to interactive handling.
pub(super) fn pop_matching_dismissal(
    queue: &mut std::collections::VecDeque<engine_player_command::PlayerCommand>,
    target: &engine_player_command::ModalKind,
) -> Option<engine_player_command::DialogResult> {
    let pos = queue.iter().position(|c| {
        matches!(
            c,
            engine_player_command::PlayerCommand::ModalDismiss { kind, .. }
                if kind == target
        )
    })?;
    match queue.remove(pos)? {
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
    replay_modal_dismissals: &mut std::collections::VecDeque<engine_player_command::PlayerCommand>,
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
            dialog_id,
            kind,
            sentences,
            replay_result,
        });
    }

    (!pending.is_empty()).then_some(ActiveDialogueBatch {
        pending,
        current: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn tick_active_dialogue_batch(
    batch: &mut ActiveDialogueBatch,
    host: &mut Host,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    audio_backend: &mut Option<KiraAudioBackend>,
    menu_resources: &mut Option<IngameMenuResources>,
    replay_recorder: &mut Option<ReplayRecorder>,
) {
    let Some(resources) = menu_resources.as_mut() else {
        tracing::warn!("DisplayDialog: menu resources unavailable — dropping active dialogue");
        batch.pending.clear();
        batch.current = None;
        return;
    };

    if batch.current.is_none()
        && let Some(item) = batch.pending.pop_front()
    {
        if let Some(result) = item.replay_result {
            if let Some(recorder) = replay_recorder.as_mut() {
                recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                    kind: item.kind,
                    result,
                });
            }
        } else {
            let state = DialogueModalState::new(event_pump, renderer, resources, item.sentences);
            batch.current = Some((item.dialog_id, item.kind, state));
        }
    }

    let Some((dialog_id, kind, state)) = batch.current.as_mut() else {
        return;
    };

    let sound_cfg = SoundConfig::default();
    let sound_enabled = audio_backend.is_some();
    let modal_net = host
        .transport
        .net
        .as_ref()
        .map(|net| ModalNet::new(net, kind.clone()));
    let cursor = default_modal_cursor(cursor_renderer, cursor_res, renderer);
    if let Some(result) = state.tick(
        event_pump,
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
    ) {
        if let Some(recorder) = replay_recorder.as_mut() {
            recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                kind: engine_player_command::ModalKind::Dialog {
                    dialog_id: *dialog_id,
                },
                result,
            });
        }
        batch.current = None;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn drain_pending_dialogues(
    host: &mut Host,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    audio_backend: &mut Option<KiraAudioBackend>,
    text_res: &mut ResourceManager,
    game: &Game,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    menu_resources: &mut Option<IngameMenuResources>,
    replay_recorder: &mut Option<ReplayRecorder>,
    replay_modal_dismissals: &mut std::collections::VecDeque<engine_player_command::PlayerCommand>,
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
                if let Some(recorder) = replay_recorder.as_mut() {
                    recorder
                        .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
                }
            }
            return;
        }
        if let (Some(descriptors), Some(resources)) = (&level_descriptors, menu_resources) {
            let sound_cfg = SoundConfig::default();
            let sound_enabled = audio_backend.is_some();

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
            let entries: Vec<BatchDialogue<'_>> = sentences_per_id
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
                    BatchDialogue {
                        sentences: sentences.as_slice(),
                        replay_result,
                        modal_net,
                    }
                })
                .collect();

            let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
            let results = ingame_menu::show_dialogue_batch(
                event_pump,
                renderer,
                resources,
                &mut host.audio.sound,
                &sound_cfg,
                audio_backend
                    .as_mut()
                    .map(|b| b as &mut dyn crate::sound::AudioBackend),
                sound_enabled,
                cursor,
                &entries,
            )
            .await;

            if let Some(recorder) = replay_recorder {
                for ((dialog_id, _), result) in sentences_per_id.iter().zip(results.iter().copied())
                {
                    let kind = engine_player_command::ModalKind::Dialog {
                        dialog_id: *dialog_id,
                    };
                    recorder
                        .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_active_popup_scroll_batch(
    host: &mut Host,
    renderer: &mut Renderer,
    text_res: &mut ResourceManager,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    menu_resources: &mut Option<IngameMenuResources>,
    replay_modal_dismissals: &mut std::collections::VecDeque<engine_player_command::PlayerCommand>,
    universal_frame: u32,
) -> Option<ActivePopupScrollBatch> {
    if host.effects.popup_text_count() == 0 {
        return None;
    }
    let text_ids: Vec<i32> = host.effects.take_popup_texts();
    let Some(resources) = menu_resources.as_mut() else {
        tracing::warn!(
            "DisplayPopupText: menu resources unavailable — dropping {} popup(s)",
            text_ids.len()
        );
        return None;
    };

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
        let picture = resources.picture_from(renderer, text_res, picture_id);
        let kind = engine_player_command::ModalKind::PopupText { text_id };
        let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
        pending.push_back(ActivePopupScrollItem {
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

    (!pending.is_empty()).then_some(ActivePopupScrollBatch {
        pending,
        current: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_active_sherwood_report(
    host: &mut Host,
    engine: &Engine,
    profiles: &engine_profiles::ProfileManager,
    menu_resources: &mut Option<IngameMenuResources>,
    replay_modal_dismissals: &mut std::collections::VecDeque<engine_player_command::PlayerCommand>,
) -> Option<ActivePopupScrollBatch> {
    if !host.effects.take_sherwood_report() {
        return None;
    }
    let Some(resources) = menu_resources.as_mut() else {
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
    let item = ActivePopupScrollItem {
        kind,
        title: None,
        picture: None,
        body: text,
        body_font_name: Some("Debrief".to_string()),
        align: TextAlign::Left,
        universal_frame: engine.frame_counter(),
        replay_result,
    };

    Some(ActivePopupScrollBatch {
        pending: VecDeque::from([item]),
        current: None,
    })
}

pub(super) fn start_active_debriefing_batch(
    host: &mut Host,
    text_res: &mut ResourceManager,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    menu_resources: &Option<IngameMenuResources>,
    replay_modal_dismissals: &mut std::collections::VecDeque<engine_player_command::PlayerCommand>,
) -> Option<ActiveDebriefingBatch> {
    if host.effects.debriefing_count() == 0 {
        return None;
    }
    let ids: Vec<DebriefingTextId> = host.effects.take_debriefings();
    let (Some(descriptors), Some(_resources)) = (level_descriptors, menu_resources) else {
        tracing::warn!(
            "DisplayDebriefing: level descriptors or menu resources unavailable — \
             dropping {} debriefing(s)",
            ids.len()
        );
        return None;
    };

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

    (!pending.is_empty()).then_some(ActiveDebriefingBatch {
        pending,
        current: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn tick_active_popup_scroll_batch(
    batch: &mut ActivePopupScrollBatch,
    host: &mut Host,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    audio_backend: &mut Option<KiraAudioBackend>,
    sample_loader: &SampleLoader,
    menu_resources: &mut Option<IngameMenuResources>,
    replay_recorder: &mut Option<ReplayRecorder>,
) {
    let Some(resources) = menu_resources.as_mut() else {
        tracing::warn!("DisplayPopupText: menu resources unavailable — dropping active popup");
        batch.pending.clear();
        batch.current = None;
        return;
    };

    if batch.current.is_none()
        && let Some(item) = batch.pending.pop_front()
    {
        if let Some(result) = item.replay_result {
            if let Some(recorder) = replay_recorder.as_mut() {
                recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                    kind: item.kind,
                    result,
                });
            }
        } else {
            let state = PopupScrollModalState::new(
                event_pump,
                renderer,
                resources,
                item.title,
                item.picture,
                item.body,
                item.body_font_name,
                item.align,
                item.universal_frame,
            );
            batch.current = Some((item.kind, state));
        }
    }

    let Some((kind, state)) = batch.current.as_mut() else {
        return;
    };

    let modal_net = host
        .transport
        .net
        .as_ref()
        .map(|net| ModalNet::new(net, kind.clone()));
    let cursor = default_modal_cursor(cursor_renderer, cursor_res, renderer);
    if let Some(result) = state.tick(
        event_pump,
        renderer,
        resources,
        &mut host.audio.sound,
        audio_backend
            .as_mut()
            .map(|b| b as &mut dyn crate::sound::AudioBackend),
        sample_loader,
        Some(cursor),
        modal_net.as_ref(),
    ) {
        if let Some(recorder) = replay_recorder.as_mut() {
            recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                kind: kind.clone(),
                result,
            });
        }
        batch.current = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn tick_active_debriefing_batch(
    batch: &mut ActiveDebriefingBatch,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    _host: &Host,
    menu_resources: &Option<IngameMenuResources>,
    replay_recorder: &mut Option<ReplayRecorder>,
) {
    let Some(resources) = menu_resources.as_ref() else {
        tracing::warn!(
            "DisplayDebriefing: menu resources unavailable — dropping active debriefing"
        );
        batch.pending.clear();
        batch.current = None;
        return;
    };
    if batch.current.is_none()
        && let Some(item) = batch.pending.pop_front()
    {
        if let Some(result) = item.replay_result {
            if let Some(recorder) = replay_recorder.as_mut() {
                recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                    kind: item.kind,
                    result,
                });
            }
        } else {
            batch.current = Some((
                item.kind,
                DebriefingModalState::new(
                    resources, item.body, None, 0, item.won, false, None, false, false,
                ),
            ));
        }
    }
    let Some((kind, state)) = batch.current.as_mut() else {
        return;
    };
    let cursor = default_modal_cursor(cursor_renderer, cursor_res, renderer);
    if let Some(outcome) = state.tick(event_pump, renderer, resources, Some(cursor)) {
        let result = if matches!(outcome, DebriefingOutcome::EmergencyEnd) {
            engine_player_command::DialogResult::Aborted
        } else {
            engine_player_command::DialogResult::Completed
        };
        if let Some(recorder) = replay_recorder.as_mut() {
            recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                kind: kind.clone(),
                result,
            });
        }
        if matches!(outcome, DebriefingOutcome::EmergencyEnd) {
            // We flatten the queued phase ordering, so dropping the
            // remaining items is the conservative no-surprise
            // behavior on an external close.
            batch.pending.clear();
        }
        batch.current = None;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn tick_active_modal(
    active_modal: &mut Option<ActiveModal>,
    host: &mut Host,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    audio_backend: &mut Option<KiraAudioBackend>,
    sample_loader: &SampleLoader,
    menu_resources: &mut Option<IngameMenuResources>,
    replay_recorder: &mut Option<ReplayRecorder>,
) -> ActiveModalOutcome {
    let Some(modal) = active_modal.as_mut() else {
        return ActiveModalOutcome::None;
    };

    match modal {
        ActiveModal::Dialogue(batch) => {
            tick_active_dialogue_batch(
                batch,
                host,
                event_pump,
                renderer,
                cursor_res,
                cursor_renderer,
                audio_backend,
                menu_resources,
                replay_recorder,
            );
            if batch.is_empty() {
                *active_modal = None;
            }
            ActiveModalOutcome::None
        }
        ActiveModal::PopupScroll(batch) => {
            tick_active_popup_scroll_batch(
                batch,
                host,
                event_pump,
                renderer,
                cursor_res,
                cursor_renderer,
                audio_backend,
                sample_loader,
                menu_resources,
                replay_recorder,
            );
            if batch.is_empty() {
                *active_modal = None;
            }
            ActiveModalOutcome::None
        }
        ActiveModal::Debriefing(batch) => {
            tick_active_debriefing_batch(
                batch,
                event_pump,
                renderer,
                cursor_res,
                cursor_renderer,
                host,
                menu_resources,
                replay_recorder,
            );
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
                if let Some(recorder) = replay_recorder.as_mut() {
                    recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                        kind: kind.clone(),
                        result,
                    });
                }
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
            let Some(resources) = menu_resources.as_ref() else {
                tracing::warn!("mission-state popup: menu resources unavailable — skipped");
                *active_modal = None;
                return ActiveModalOutcome::None;
            };
            let cursor = default_modal_cursor(cursor_renderer, cursor_res, renderer);
            if let Some(confirmed) = state.tick(event_pump, renderer, resources, Some(cursor)) {
                if let Some(recorder) = replay_recorder.as_mut() {
                    let result = if confirmed {
                        engine_player_command::DialogResult::Completed
                    } else {
                        engine_player_command::DialogResult::Aborted
                    };
                    recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                        kind: kind.clone(),
                        result,
                    });
                }
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
#[allow(clippy::too_many_arguments)]
pub(super) async fn drain_pending_popup_scroll(
    host: &mut Host,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    audio_backend: &mut Option<KiraAudioBackend>,
    sample_loader: &SampleLoader,
    text_res: &mut ResourceManager,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    menu_resources: &mut Option<IngameMenuResources>,
    replay_recorder: &mut Option<ReplayRecorder>,
    replay_modal_dismissals: &mut std::collections::VecDeque<engine_player_command::PlayerCommand>,
    universal_frame: u32,
) {
    // ── Drain pending popup-scroll texts ──
    // Script natives `DisplayPopupText` and the `DisplayAllPopupTexts`
    // cheat push text IDs onto `pending_popup_texts`.
    if host.effects.popup_text_count() != 0 {
        let text_ids: Vec<i32> = host.effects.take_popup_texts();
        let Some(resources) = menu_resources.as_mut() else {
            // Without `IngameMenuResources` the parchment background, OK
            // button sprite, and font cache are all unavailable — we
            // genuinely cannot render anything, so drop the queue.
            tracing::warn!(
                "DisplayPopupText: menu resources unavailable — dropping {} popup(s)",
                text_ids.len()
            );
            return;
        };
        let sound_cfg = SoundConfig::default();
        let sound_enabled = audio_backend.is_some();
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
            let picture = resources.picture_from(renderer, text_res, picture_id);
            let kind = engine_player_command::ModalKind::PopupText { text_id };
            let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
            let modal_net = host
                .transport
                .net
                .as_ref()
                .map(|net| ModalNet::new(net, kind.clone()));
            let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
            let result = ingame_menu::show_popup_scroll(
                event_pump,
                renderer,
                resources,
                &mut host.audio.sound,
                &sound_cfg,
                audio_backend
                    .as_mut()
                    .map(|b| b as &mut dyn crate::sound::AudioBackend),
                sound_enabled,
                sample_loader,
                cursor,
                None,
                picture,
                &text,
                None,
                TextAlign::Justified,
                universal_frame,
                replay_result,
                modal_net,
            )
            .await;
            if let Some(recorder) = replay_recorder.as_mut() {
                recorder.push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
            }
        }
    }
}

/// Drain a script-queued Sherwood stat report for the frame.
///
/// Script native `DisplaySherwoodReport` sets `pending_sherwood_report`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn drain_pending_sherwood_stat(
    host: &mut Host,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    engine: &Engine,
    profiles: &engine_profiles::ProfileManager,
    audio_backend: &mut Option<KiraAudioBackend>,
    sample_loader: &SampleLoader,
    menu_resources: &mut Option<IngameMenuResources>,
    replay_recorder: &mut Option<ReplayRecorder>,
    replay_modal_dismissals: &mut std::collections::VecDeque<engine_player_command::PlayerCommand>,
) {
    // ── Drain pending Sherwood stat report ──
    // Script native `DisplaySherwoodReport` sets
    // `pending_sherwood_report`.
    if host.effects.take_sherwood_report() {
        if let Some(resources) = menu_resources.as_mut() {
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
            let sound_cfg = SoundConfig::default();
            let sound_enabled = audio_backend.is_some();
            let kind = engine_player_command::ModalKind::SherwoodReport;
            let replay_result = pop_matching_dismissal(replay_modal_dismissals, &kind);
            let modal_net = host
                .transport
                .net
                .as_ref()
                .map(|net| ModalNet::new(net, kind.clone()));
            // The Sherwood report uses the "Debrief" font and is
            // left-aligned (not the popup-scroll default).
            let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
            let result = ingame_menu::show_popup_scroll(
                event_pump,
                renderer,
                resources,
                &mut host.audio.sound,
                &sound_cfg,
                audio_backend
                    .as_mut()
                    .map(|b| b as &mut dyn crate::sound::AudioBackend),
                sound_enabled,
                sample_loader,
                cursor,
                None,
                None,
                &text,
                Some("Debrief"),
                TextAlign::Left,
                engine.frame_counter(),
                replay_result,
                modal_net,
            )
            .await;
            if let Some(recorder) = replay_recorder.as_mut() {
                recorder.push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
            }
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
#[allow(clippy::too_many_arguments)]
pub(super) async fn drain_pending_debriefings(
    host: &mut Host,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    text_res: &mut ResourceManager,
    level_descriptors: &Option<assets_res_descr::LevelDescriptors>,
    menu_resources: &Option<IngameMenuResources>,
    replay_recorder: &mut Option<ReplayRecorder>,
    replay_modal_dismissals: &mut std::collections::VecDeque<engine_player_command::PlayerCommand>,
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
        if let (Some(descriptors), Some(resources)) = (&level_descriptors, &menu_resources) {
            let (lose_ids, win_ids): (Vec<_>, Vec<_>) = ids
                .into_iter()
                .partition(|text_id| matches!(text_id, DebriefingTextId::Lose { .. }));

            // Lose phase: each `Display(loseVec, false, false)` call.
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
                    let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                    ingame_menu::show_debriefing(
                        event_pump, renderer, resources, cursor, &text, None, 0, false, false,
                        // Cheat path passes `bRestartAllowed=false`, so
                        // the quick-load translator is never enabled.
                        None, false, false,
                    )
                    .await
                };
                let result = if matches!(debrief_outcome, DebriefingOutcome::EmergencyEnd) {
                    engine_player_command::DialogResult::Aborted
                } else {
                    engine_player_command::DialogResult::Completed
                };
                if let Some(recorder) = replay_recorder.as_mut() {
                    recorder
                        .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
                }
                // The iteration breaks out when an emergency-end
                // fires — but only for THIS phase, not the win phase
                // below.
                if matches!(debrief_outcome, DebriefingOutcome::EmergencyEnd) {
                    break;
                }
            }

            // Win phase: fresh `Display(winVec, true, false)` call.
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
                    let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                    ingame_menu::show_debriefing(
                        event_pump, renderer, resources, cursor, &text, None, 0, true, false, None,
                        false, false,
                    )
                    .await
                };
                let result = if matches!(debrief_outcome, DebriefingOutcome::EmergencyEnd) {
                    engine_player_command::DialogResult::Aborted
                } else {
                    engine_player_command::DialogResult::Completed
                };
                if let Some(recorder) = replay_recorder.as_mut() {
                    recorder
                        .push(engine_player_command::PlayerCommand::ModalDismiss { kind, result });
                }
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
    use super::pop_matching_dismissal;
    use robin_engine::player_command::{
        DebriefingTextId, DialogResult, MissionStateModalKind, ModalKind, PlayerCommand,
    };
    use std::collections::VecDeque;

    #[test]
    fn pop_matching_dismissal_removes_only_matching_modal() {
        let mut queue = VecDeque::from([
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
        ]);

        let result = pop_matching_dismissal(
            &mut queue,
            &ModalKind::Debriefing {
                text_id: DebriefingTextId::Lose { index: 1 },
            },
        );

        assert_eq!(result, Some(DialogResult::Aborted));
        assert_eq!(queue.len(), 2);
        assert!(matches!(
            queue[0],
            PlayerCommand::ModalDismiss {
                kind: ModalKind::PopupText { text_id: 7 },
                ..
            }
        ));
        assert!(matches!(
            queue[1],
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
        let mut queue = VecDeque::from([PlayerCommand::ModalDismiss {
            kind: ModalKind::Debriefing {
                text_id: DebriefingTextId::Win { index: 1 },
            },
            result: DialogResult::Completed,
        }]);

        let result = pop_matching_dismissal(
            &mut queue,
            &ModalKind::Debriefing {
                text_id: DebriefingTextId::Lose { index: 0 },
            },
        );

        assert_eq!(result, None);
        assert_eq!(queue.len(), 1);
    }
}

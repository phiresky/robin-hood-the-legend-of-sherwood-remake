//! Character dialogue screen.
//!
//! A 496x463 parchment window showing a portrait in `(330,50)..(450,210)`
//! and the current sentence in `(50,50)..(450,350)`.  The text renderer
//! reserves a 130x170 "dropped initial" region on the top-right so the
//! portrait column sits on empty parchment; text wraps around it in two
//! passes.  Skip (Return) advances to the next sentence; Stop (Escape)
//! abandons the dialogue.
//!
//! The dialogue timer fires every 100 ms while sound is enabled.  On
//! each tick we look at the dialogue sample's current volume and pick a
//! mouth frame (0..4).  When three consecutive frames show the same
//! mouth, a random "blink" swaps between 0 and 1 so the portrait never
//! looks frozen — see [`MAX_FACE_COUNT`].
//!
//! Sound playback is delegated to [`crate::sound::SoundManager`]: each
//! sentence starts with `play_dialog()` and the timer polls
//! `is_dialog_finished()` to auto-advance.
//!
//! Buttons are driven by the [`crate::widget`] system via the
//! [`super::widget_bridge`].

use robin_engine::coordinates as engine_coordinates;
use robin_engine::player_command::DialogResult;
use robin_engine::sprite::BBox;
use std::time::Duration;
use web_time::Instant;

use crate::gfx_types::Keycode;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::sound::{AudioBackend, SoundManager};
use crate::widget::FrameWnd;
use robin_engine::resource_ids;

use super::layout::{
    MENU_H, MENU_W, MenuTransform, TextAlign, TooltipState, dim_screen, draw_background,
    enter_modal_gpu_phase,
};
use super::resources::{
    IngameMenuResources, MT_INFOBULLE_BUTTON_DIALOG_ABANDON, MT_INFOBULLE_BUTTON_DIALOG_CONTINUE,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

/// Virtual window geometry.
pub const WIN_W: i32 = 496;
pub const WIN_H: i32 = 463;

/// Text bounding box inside the window — `(50, 50, 450, 350)`.  The box
/// spans the whole window interior: the portrait column on the right is
/// painted over the text after the text renders, so any characters that
/// land under the portrait are harmlessly overdrawn.
const TEXT_X: i32 = 50;
const TEXT_Y: i32 = 50;
const TEXT_W: i32 = 400; // 450 - 50
const TEXT_H: i32 = 300; // 350 - 50

/// Portrait box inside the window (`(330,50)..(450,210)`).
const PORTRAIT_X: i32 = 330;
const PORTRAIT_Y: i32 = 50;
const PORTRAIT_W: i32 = 120;
const PORTRAIT_H: i32 = 160;

/// Dropped-initial reserved region. This is a pure layout reservation —
/// the text is pushed out of this box (on the top-right) so the portrait
/// widget can sit there on blank parchment.  Nothing is rendered into
/// the reserved box by the text pass itself.
const DROP_CAP_W: i32 = 130;
const DROP_CAP_H: i32 = 170;

/// Auto-advance timer tick.
const TIMER_INTERVAL: Duration = Duration::from_millis(100);

/// Same-face count before a random blink kicks in.
const MAX_FACE_COUNT: u8 = 3;

/// Per-frame fade step — fades the new portrait in over four blended
/// frames before snapping to the fully-opaque state.
const PORTRAIT_FADE_STEP: f32 = 0.25;

/// Widget IDs for the two buttons.
const ID_SKIP: u32 = 0;
const ID_STOP: u32 = 1;

/// Portrait ID table.  Dialogue scripts reference these by index 0..15.
pub const DIALOGUE_PORTRAIT_IDS: [i32; 16] = [
    resource_ids::RHID_DLG_ROBIN,
    resource_ids::RHID_DLG_GODWIN,
    resource_ids::RHID_DLG_GUISBOURNE,
    resource_ids::RHID_DLG_SCARLET,
    resource_ids::RHID_DLG_SOLDIER,
    resource_ids::RHID_DLG_LITTLE_JOHN,
    resource_ids::RHID_DLG_MARIANNE,
    resource_ids::RHID_DLG_PRINCE_JOHN,
    resource_ids::RHID_DLG_LONGCHAMP,
    resource_ids::RHID_DLG_SHERIF,
    resource_ids::RHID_DLG_SCATHLOCK,
    resource_ids::RHID_DLG_RANULPH,
    resource_ids::RHID_DLG_ALLAN,
    resource_ids::RHID_DLG_STUTELEY,
    resource_ids::RHID_DLG_TUCK,
    resource_ids::RHID_DLG_BAD_PORTRAIT,
];

/// A single dialogue sentence — text, portrait index and optional
/// voice-acting sample path.
#[derive(Debug, Clone)]
pub struct DialogueSentence {
    pub portrait_index: usize,
    pub text: String,
    /// Path to the dialogue .wav relative to the data directory.  Empty
    /// disables voice playback for the sentence.
    pub sound_path: String,
}

/// Cross-fade state machine for the dialogue portrait widget.
///
/// When the speaker changes mid-dialogue the old portrait is held fully
/// opaque for four frames while the new one is blended in at
/// 25 %, 50 %, 75 %, 100 %, after which the transition snaps closed and
/// only the new portrait is drawn.  All that matters here is the
/// (previous, current, fade) triple.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PortraitFade {
    current: i32,
    previous: i32,
    fade: f32,
}

/// "No previous portrait" sentinel — used so the first-ever render runs
/// through the fade-in path with nothing to blend against.
/// `Renderer::blit_to_screen_alpha` returns `false` for an unknown
/// `src_id`, so an id of 0 is naturally a no-op blit.
pub(crate) const NO_PORTRAIT: i32 = 0;

impl PortraitFade {
    /// Seed the state so the first sentence's portrait fades in from
    /// nothing over four frames.  The first render treats any non-zero
    /// widget id as a fresh transition.
    pub(crate) fn new(initial: i32) -> Self {
        Self {
            current: initial,
            previous: NO_PORTRAIT,
            fade: 0.0,
        }
    }

    /// Note that the dialogue has asked to display `new_id`. If it
    /// differs from the currently-showing portrait, promote the old one
    /// to `previous` and restart the fade.
    pub(crate) fn set(&mut self, new_id: i32) {
        if self.current != new_id {
            self.previous = self.current;
            self.current = new_id;
            self.fade = 0.0;
        }
    }

    /// Advance the fade by one frame. Once the new portrait is fully
    /// opaque the previous slot is promoted and the transition ends.
    pub(crate) fn tick(&mut self) {
        if self.previous != self.current {
            self.fade += PORTRAIT_FADE_STEP;
            if self.fade >= 1.0 {
                self.fade = 1.0;
                self.previous = self.current;
            }
        }
    }

    /// Is the cross-fade still in progress this frame?
    pub(crate) fn is_fading(&self) -> bool {
        self.previous != self.current
    }

    /// Current new-portrait alpha, expressed as an integer percentage
    /// 0..=100 to match the renderer's `blit_to_screen_alpha` range.
    pub(crate) fn fade_percent(&self) -> u16 {
        (self.fade.clamp(0.0, 1.0) * 100.0) as u16
    }
}

impl DialogueSentence {
    /// Clamp an out-of-range portrait index to the "bad portrait"
    /// placeholder (the last entry in the table).
    pub fn resolved_portrait_id(&self) -> i32 {
        let idx = self.portrait_index.min(DIALOGUE_PORTRAIT_IDS.len() - 1);
        DIALOGUE_PORTRAIT_IDS[idx]
    }
}

/// Play out a sequence of dialogue sentences.  Returns
/// [`DialogResult::Completed`] when the player saw every sentence and
/// [`DialogResult::Aborted`] if they pressed Stop / Escape.
///
/// When `replay_result` is `Some`, the interactive loop is skipped and
/// the pre-recorded result is returned immediately.  This is what lets
/// replays dismiss modal briefings without a human at the keyboard.
#[allow(clippy::too_many_arguments)]
pub async fn show_dialogue(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &mut IngameMenuResources,
    sound: &mut SoundManager,
    sound_config: &robin_engine::sound_config::SoundConfig,
    audio: Option<&mut dyn AudioBackend>,
    sound_enabled: bool,
    cursor: Option<ModalCursor<'_>>,
    sentences: &[DialogueSentence],
    replay_result: Option<DialogResult>,
    mut modal_net: Option<super::ModalNet<'_>>,
) -> DialogResult {
    if sentences.is_empty() {
        return DialogResult::Completed;
    }
    if let Some(res) = replay_result {
        return res;
    }

    // Enter dialogue mode — ducks other audio while playing the voice stream.
    sound.enter_dialogue(sound_config);

    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let transform = MenuTransform::centered(sw, sh);

    let virt_x = (MENU_W - WIN_W) / 2;
    let virt_y = (MENU_H - WIN_H) / 2;

    // Skip / Stop are the round `RHID_OK` / `RHID_CANCEL` wax-seal
    // sprites with no label, centred horizontally as a pair at y=384
    // like the original dialogue window.
    let (ok_w, ok_h) = resources.ok_button_dimensions();
    let (cancel_w, cancel_h) = resources.cancel_button_dimensions();
    let btn_w = ok_w.max(cancel_w);
    let btn_h = ok_h.max(cancel_h);
    let n = 2i32;
    let spacing = 8;
    let total_w = n * btn_w + (n - 1) * spacing;
    let start_x = virt_x + (WIN_W - total_w) / 2;
    let btn_y = (virt_y + 384).min(virt_y + WIN_H - btn_h - 16);

    // Build a FrameWnd with Skip and Stop buttons.
    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    frame.add_widget_absolute(widget_bridge::make_button_with_resource(
        ID_SKIP,
        "",
        true,
        robin_engine::resource_ids::RHID_OK,
        start_x,
        btn_y,
        btn_w,
        btn_h,
    ));
    frame.add_widget_absolute(widget_bridge::make_button_with_resource(
        ID_STOP,
        "",
        true,
        robin_engine::resource_ids::RHID_CANCEL,
        start_x + btn_w + spacing,
        btn_y,
        btn_w,
        btn_h,
    ));
    widget_bridge::attach_alpha_masks(&mut frame, resources, renderer);

    // Per-widget tooltip text rendered by the hover-tooltip loop below.
    let skip_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_DIALOG_CONTINUE);
    let stop_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_DIALOG_ABANDON);
    if let Some(w) = frame.widget_mut(ID_SKIP) {
        w.base_mut().set_tooltip_text(&skip_tooltip);
    }
    if let Some(w) = frame.widget_mut(ID_STOP) {
        w.base_mut().set_tooltip_text(&stop_tooltip);
    }

    // ── Animation state ────────────────────────────────────────────
    let mut mouth_frame: u8 = 0;
    let mut same_face_count: u8 = 0;
    let mut last_timer_tick = Instant::now();
    let mut rng = fastrand::Rng::new();

    // Hover-tooltip tracker; see `super::layout::TooltipState` for the
    // timing model.
    let mut tooltip = TooltipState::new();

    // Ownership dance: store the audio backend in an Option so we can
    // re-borrow it fresh on every helper call.
    let mut audio_slot = audio;

    let mut sentence_idx: usize = 0;
    let mut aborted = false;
    let mut portrait_fade = PortraitFade::new(sentences[0].resolved_portrait_id());

    // Start the first sentence's audio.
    start_sentence(sound, &mut audio_slot, sound_enabled, &sentences[0]);

    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_window(event_pump, transform);

    let mut remote_result = None;
    'outer: loop {
        if let Some(net) = modal_net.as_ref()
            && let Some(result) = net.poll_remote_dismissal()
        {
            remote_result = Some(result);
            break 'outer;
        }
        if aborted {
            break 'outer;
        }
        let sentence = &sentences[sentence_idx];

        // ── Input ───────────────────────────────────────────────
        let mut advance = false;

        for event in event_pump.poll_events() {
            input_state.update_from_event(&event, transform);

            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => {
                    aborted = true;
                }
                // Skip is bound to Return and Keypad-Enter only — no
                // Space shortcut.
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    advance = true;
                }
                _ => {}
            }
        }

        // ── Widget input processing ─────────────────────────────
        let widget_input = input_state.as_widget_input();
        let events = frame.process_input(&widget_input);
        input_state.end_frame();

        if let Some(id) = widget_bridge::find_activated(&events) {
            match id {
                ID_SKIP => advance = true,
                ID_STOP => aborted = true,
                _ => {}
            }
        }

        // ── Timer tick: mouth animation + auto-advance ─────────
        if last_timer_tick.elapsed() >= TIMER_INTERVAL {
            last_timer_tick = Instant::now();
            if sound_enabled {
                if sound.is_dialog_finished() {
                    advance = true;
                } else {
                    update_mouth(
                        sound,
                        &mut audio_slot,
                        &mut mouth_frame,
                        &mut same_face_count,
                        &mut rng,
                    );
                }
            }
        }

        if advance {
            // Only stop the current sample if it's actually still
            // playing.  An auto-advance that fires because
            // `is_dialog_finished()` already returned true leaves the
            // slot quiesced; calling `close_dialog` on a finished stream
            // just races the backend for no gain.
            if !sound.is_dialog_finished()
                && let Some(backend) = audio_slot.as_deref_mut()
            {
                sound.close_dialog(backend);
            }
            sentence_idx += 1;
            if sentence_idx >= sentences.len() {
                break 'outer;
            }
            start_sentence(
                sound,
                &mut audio_slot,
                sound_enabled,
                &sentences[sentence_idx],
            );
            mouth_frame = 0;
            same_face_count = 0;
        }

        // ── Portrait cross-fade: update state for this frame ──
        portrait_fade.set(sentence.resolved_portrait_id());

        // ── Render ──────────────────────────────────────────────
        // Pre-load every portrait we might need this frame so the
        // subsequent renderer calls don't need a `&mut` pass through
        // `resources`.  On a same-speaker frame this is a single cache
        // hit; during a cross-fade we need both the old and new surface.
        let current_portrait = resources.portrait(renderer, portrait_fade.current);
        let previous_portrait = if portrait_fade.is_fading() {
            resources.portrait(renderer, portrait_fade.previous)
        } else {
            None
        };

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.parchment_huge {
            draw_background(renderer, transform, &bg, virt_x, virt_y, WIN_W, WIN_H);
        }

        // Draw the old portrait opaquely while the new one fades in on
        // top.  Always draw previous first; the fade is expressed by
        // the alpha on the current blit that goes on top.
        if portrait_fade.is_fading()
            && let Some(p) = previous_portrait
        {
            draw_portrait_frame_alpha(
                renderer,
                transform,
                &p,
                virt_x + PORTRAIT_X,
                virt_y + PORTRAIT_Y,
                PORTRAIT_W,
                PORTRAIT_H,
                mouth_frame,
                100,
            );
        }
        if let Some(p) = current_portrait {
            let alpha = if portrait_fade.is_fading() {
                portrait_fade.fade_percent()
            } else {
                100
            };
            draw_portrait_frame_alpha(
                renderer,
                transform,
                &p,
                virt_x + PORTRAIT_X,
                virt_y + PORTRAIT_Y,
                PORTRAIT_W,
                PORTRAIT_H,
                mouth_frame,
                alpha,
            );
        }
        portrait_fade.tick();

        if let Some(font) = resources.popup_font() {
            render_dropped_initial_text(
                renderer,
                font,
                transform,
                &sentence.text,
                virt_x + TEXT_X,
                virt_y + TEXT_Y,
                TEXT_W,
                TEXT_H,
            );
        }

        // Draw buttons via widget bridge.
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);

        // Hover-tooltip: track which button the cursor is over and
        // paint the tooltip text after the idle delay.  Shared helper —
        // see `super::layout::TooltipState`.
        let mouse_pt = engine_coordinates::ScreenPoint::new(input_state.virt_x, input_state.virt_y);
        tooltip.update(&frame, mouse_pt);
        if let Some(font) = resources.popup_font() {
            tooltip.draw(renderer, font, transform, &frame, mouse_pt);
        }

        if let Some(c) = &cursor {
            c.draw(renderer, transform, &input_state);
        }

        renderer.present();
        crate::window::sleep_ms(16).await;
    }

    #[allow(clippy::needless_option_as_deref)]
    if let Some(backend) = audio_slot.as_deref_mut() {
        sound.close_dialog(backend);
    }
    sound.leave_dialogue(sound_config);

    let result = if let Some(result) = remote_result {
        result
    } else if aborted {
        DialogResult::Aborted
    } else {
        DialogResult::Completed
    };
    if remote_result.is_none()
        && let Some(net) = modal_net.as_mut()
    {
        net.publish(result);
    }
    result
}

/// One-frame dialogue modal state.
///
/// `show_dialogue` above is the legacy blocking wrapper. Multiplayer
/// mission code owns this state directly and calls [`tick`](Self::tick)
/// once from the outer frame loop so networking, pacing, and replay
/// bookkeeping can continue while the dialogue is visible.
pub struct DialogueModalState {
    sentences: Vec<DialogueSentence>,
    frame: crate::widget::FrameWnd,
    input_state: ModalInputState,
    tooltip: TooltipState,
    transform: MenuTransform,
    virt_x: i32,
    virt_y: i32,
    mouth_frame: u8,
    same_face_count: u8,
    last_timer_tick: Instant,
    rng: fastrand::Rng,
    sentence_idx: usize,
    aborted: bool,
    portrait_fade: PortraitFade,
    entered_dialogue: bool,
}

impl DialogueModalState {
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &mut IngameMenuResources,
        sentences: Vec<DialogueSentence>,
    ) -> Self {
        assert!(
            !sentences.is_empty(),
            "DialogueModalState requires at least one sentence"
        );

        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        let transform = MenuTransform::centered(sw, sh);
        let virt_x = (MENU_W - WIN_W) / 2;
        let virt_y = (MENU_H - WIN_H) / 2;

        // Same round wax-seal Skip / Stop pair as `show_dialogue`.
        let (ok_w, ok_h) = resources.ok_button_dimensions();
        let (cancel_w, cancel_h) = resources.cancel_button_dimensions();
        let btn_w = ok_w.max(cancel_w);
        let btn_h = ok_h.max(cancel_h);
        let n = 2i32;
        let spacing = 8;
        let total_w = n * btn_w + (n - 1) * spacing;
        let start_x = virt_x + (WIN_W - total_w) / 2;
        let btn_y = (virt_y + 384).min(virt_y + WIN_H - btn_h - 16);

        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            ID_SKIP,
            "",
            true,
            robin_engine::resource_ids::RHID_OK,
            start_x,
            btn_y,
            btn_w,
            btn_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            ID_STOP,
            "",
            true,
            robin_engine::resource_ids::RHID_CANCEL,
            start_x + btn_w + spacing,
            btn_y,
            btn_w,
            btn_h,
        ));
        widget_bridge::attach_alpha_masks(&mut frame, resources, renderer);

        let skip_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_DIALOG_CONTINUE);
        let stop_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_DIALOG_ABANDON);
        if let Some(w) = frame.widget_mut(ID_SKIP) {
            w.base_mut().set_tooltip_text(&skip_tooltip);
        }
        if let Some(w) = frame.widget_mut(ID_STOP) {
            w.base_mut().set_tooltip_text(&stop_tooltip);
        }

        let mut input_state = ModalInputState::new();
        input_state.seed_mouse_from_window(event_pump, transform);
        let portrait_fade = PortraitFade::new(sentences[0].resolved_portrait_id());

        Self {
            sentences,
            frame,
            input_state,
            tooltip: TooltipState::new(),
            transform,
            virt_x,
            virt_y,
            mouth_frame: 0,
            same_face_count: 0,
            last_timer_tick: Instant::now(),
            rng: fastrand::Rng::new(),
            sentence_idx: 0,
            aborted: false,
            portrait_fade,
            entered_dialogue: false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &mut IngameMenuResources,
        sound: &mut SoundManager,
        sound_config: &robin_engine::sound_config::SoundConfig,
        mut audio: Option<&mut dyn AudioBackend>,
        sound_enabled: bool,
        cursor: Option<&ModalCursor<'_>>,
        modal_net: Option<&super::ModalNet<'_>>,
    ) -> Option<DialogResult> {
        if !self.entered_dialogue {
            sound.enter_dialogue(sound_config);
            start_sentence(
                sound,
                &mut audio,
                sound_enabled,
                &self.sentences[self.sentence_idx],
            );
            self.entered_dialogue = true;
        }

        if let Some(result) = modal_net.and_then(|net| net.poll_remote_dismissal()) {
            return Some(self.finish(sound, sound_config, audio, result, true, modal_net));
        }

        let mut advance = false;
        for event in event_pump.poll_events() {
            self.input_state.update_from_event(&event, self.transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => self.aborted = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => advance = true,
                _ => {}
            }
        }

        let widget_input = self.input_state.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        self.input_state.end_frame();
        if let Some(id) = widget_bridge::find_activated(&events) {
            match id {
                ID_SKIP => advance = true,
                ID_STOP => self.aborted = true,
                _ => {}
            }
        }

        if self.last_timer_tick.elapsed() >= TIMER_INTERVAL {
            self.last_timer_tick = Instant::now();
            if sound_enabled {
                if sound.is_dialog_finished() {
                    advance = true;
                } else {
                    update_mouth(
                        sound,
                        &mut audio,
                        &mut self.mouth_frame,
                        &mut self.same_face_count,
                        &mut self.rng,
                    );
                }
            }
        }

        if self.aborted {
            return Some(self.finish(
                sound,
                sound_config,
                audio,
                DialogResult::Aborted,
                false,
                modal_net,
            ));
        }

        if advance {
            if !sound.is_dialog_finished()
                && let Some(backend) = audio.as_deref_mut()
            {
                sound.close_dialog(backend);
            }
            self.sentence_idx += 1;
            if self.sentence_idx >= self.sentences.len() {
                return Some(self.finish(
                    sound,
                    sound_config,
                    audio,
                    DialogResult::Completed,
                    false,
                    modal_net,
                ));
            }
            start_sentence(
                sound,
                &mut audio,
                sound_enabled,
                &self.sentences[self.sentence_idx],
            );
            self.mouth_frame = 0;
            self.same_face_count = 0;
        }

        self.render(renderer, resources, cursor);
        renderer.present();
        None
    }

    fn finish(
        &mut self,
        sound: &mut SoundManager,
        sound_config: &robin_engine::sound_config::SoundConfig,
        audio: Option<&mut dyn AudioBackend>,
        result: DialogResult,
        remote: bool,
        modal_net: Option<&super::ModalNet<'_>>,
    ) -> DialogResult {
        if let Some(backend) = audio {
            sound.close_dialog(backend);
        }
        if self.entered_dialogue {
            sound.leave_dialogue(sound_config);
            self.entered_dialogue = false;
        }
        if !remote && let Some(net) = modal_net {
            net.publish(result);
        }
        result
    }

    fn render(
        &mut self,
        renderer: &mut Renderer,
        resources: &mut IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) {
        let sentence = &self.sentences[self.sentence_idx];
        self.portrait_fade.set(sentence.resolved_portrait_id());

        let current_portrait = resources.portrait(renderer, self.portrait_fade.current);
        let previous_portrait = if self.portrait_fade.is_fading() {
            resources.portrait(renderer, self.portrait_fade.previous)
        } else {
            None
        };

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.parchment_huge {
            draw_background(
                renderer,
                self.transform,
                &bg,
                self.virt_x,
                self.virt_y,
                WIN_W,
                WIN_H,
            );
        }

        if self.portrait_fade.is_fading()
            && let Some(p) = previous_portrait
        {
            draw_portrait_frame_alpha(
                renderer,
                self.transform,
                &p,
                self.virt_x + PORTRAIT_X,
                self.virt_y + PORTRAIT_Y,
                PORTRAIT_W,
                PORTRAIT_H,
                self.mouth_frame,
                100,
            );
        }
        if let Some(p) = current_portrait {
            let alpha = if self.portrait_fade.is_fading() {
                self.portrait_fade.fade_percent()
            } else {
                100
            };
            draw_portrait_frame_alpha(
                renderer,
                self.transform,
                &p,
                self.virt_x + PORTRAIT_X,
                self.virt_y + PORTRAIT_Y,
                PORTRAIT_W,
                PORTRAIT_H,
                self.mouth_frame,
                alpha,
            );
        }
        self.portrait_fade.tick();

        if let Some(font) = resources.popup_font() {
            render_dropped_initial_text(
                renderer,
                font,
                self.transform,
                &sentence.text,
                self.virt_x + TEXT_X,
                self.virt_y + TEXT_Y,
                TEXT_W,
                TEXT_H,
            );
        }

        widget_bridge::draw_frame_buttons(renderer, resources, self.transform, &self.frame);

        let mouse_pt =
            engine_coordinates::ScreenPoint::new(self.input_state.virt_x, self.input_state.virt_y);
        self.tooltip.update(&self.frame, mouse_pt);
        if let Some(font) = resources.popup_font() {
            self.tooltip
                .draw(renderer, font, self.transform, &self.frame, mouse_pt);
        }

        if let Some(c) = cursor {
            c.draw(renderer, self.transform, &self.input_state);
        }
    }
}

/// A single entry in a dialogue playback batch.
pub struct BatchDialogue<'a> {
    /// The sentence list for this dialogue, already built via
    /// e.g. `game_session::build_dialogue_sentences`.
    pub sentences: &'a [DialogueSentence],
    /// Optional pre-recorded result — when present, `show_dialogue`
    /// short-circuits without opening a window.  Used by the replay
    /// path to dismiss modal dialogues without human input.
    pub replay_result: Option<DialogResult>,
    pub modal_net: Option<super::ModalNet<'a>>,
}

/// Play out a list of dialogues back-to-back.
///
/// Loops over every entry and calls `show_dialogue` for each, without
/// early-exit on Abort.  Returns one `DialogResult` per entry so
/// callers can record each dismissal to a replay recorder.
///
/// Re-borrowing the shared audio backend and cursor each iteration
/// keeps the inner `show_dialogue` call's borrow scope short so the
/// outer batch function can loop without aliasing conflicts.
pub(crate) async fn show_dialogue_batch(
    ctx: &mut crate::game_session::ModalContext<'_>,
    sound: &mut SoundManager,
    entries: &[BatchDialogue<'_>],
) -> Vec<DialogResult> {
    let crate::game_session::ModalContext {
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
        .expect("show_dialogue_batch requires ingame menu resources");
    let sound_config = robin_engine::sound_config::SoundConfig::default();
    let sound_enabled = audio_backend.is_some();
    let mut cursor =
        super::widget_bridge::default_modal_cursor(cursor_renderer, cursor_res, renderer);
    let mut results = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.sentences.is_empty() {
            results.push(DialogResult::Completed);
            continue;
        }
        // Explicit reborrow each iteration yields a fresh
        // `Option<&mut dyn AudioBackend>` whose borrow ends the moment
        // `show_dialogue` returns, so the next iteration can borrow
        // the backend again.
        let audio_ref: Option<&mut dyn AudioBackend> =
            audio_backend.as_mut().map(|b| b as &mut dyn AudioBackend);
        let modal_net = entry.modal_net.as_ref().map(|net| net.reborrow());
        let result = show_dialogue(
            window,
            renderer,
            resources,
            sound,
            &sound_config,
            audio_ref,
            sound_enabled,
            Some(cursor.reborrow()),
            entry.sentences,
            entry.replay_result,
            modal_net,
        )
        .await;
        results.push(result);
    }
    results
}

/// Start playing a sentence's voice sample, if sound is enabled and the
/// sentence has one.
fn start_sentence(
    sound: &mut SoundManager,
    audio: &mut Option<&mut dyn AudioBackend>,
    sound_enabled: bool,
    sentence: &DialogueSentence,
) {
    if !sound_enabled || sentence.sound_path.is_empty() {
        return;
    }
    if let Some(backend) = audio.as_deref_mut() {
        sound.play_dialog(&sentence.sound_path, backend);
    }
}

/// Update the mouth frame from the current dialogue sample volume.
///
/// Picks a frame from 0 to 4 based on the sample's current volume, and
/// if three consecutive frames show the same mouth state it randomly
/// toggles between 0 and 1 for an idle "blink".
fn update_mouth(
    sound: &SoundManager,
    audio: &mut Option<&mut dyn AudioBackend>,
    mouth_frame: &mut u8,
    same_face_count: &mut u8,
    rng: &mut fastrand::Rng,
) {
    let volume = audio
        .as_deref_mut()
        .map(|b| sound.get_dialog_volume(b))
        .unwrap_or(0.0);

    let new_frame = if volume < 0.01 {
        0
    } else if volume < 0.02 {
        1
    } else if volume < 0.15 {
        2
    } else if volume < 0.30 {
        3
    } else {
        4
    };

    if new_frame == *mouth_frame {
        *same_face_count += 1;
        if *same_face_count >= MAX_FACE_COUNT {
            *mouth_frame = (rng.u32(0..2)) as u8;
            *same_face_count = 0;
        }
    } else {
        *mouth_frame = new_frame;
        *same_face_count = 0;
    }
}

/// Render the dialogue text with a reserved "dropped-initial" region
/// on the top-right.
///
/// The text body is laid out in two passes.
///
/// * First pass ("beside the dropped initial"): the right-hand
///   `DROP_CAP_W` pixels are reserved for whatever widget sits in the
///   drop cap (the portrait, in this window), so the first
///   `DROP_CAP_H / line_h` lines render at the full `box_x` with width
///   `box_w - DROP_CAP_W`.
/// * Second pass: whatever text didn't fit in the first pass flows at
///   full `box_w`, starting immediately below the reserved area.
///
/// There is no scaled glyph — the dropped-initial region is purely a
/// layout reservation so the portrait can sit in the top-right corner
/// without the text running behind it.
#[allow(clippy::too_many_arguments)]
fn render_dropped_initial_text(
    renderer: &mut Renderer,
    font: &crate::native_font::NativeFont,
    transform: MenuTransform,
    text: &str,
    box_x: i32,
    box_y: i32,
    box_w: i32,
    box_h: i32,
) {
    if text.is_empty() {
        return;
    }

    let line_h = font.height() as i32;
    if line_h <= 0 {
        return;
    }

    // Number of lines reserved for the drop-cap region: `DROP_CAP_H /
    // line_h`, ceil-rounded so a partial line still gets reserved.
    let mut di_lines = DROP_CAP_H / line_h;
    if DROP_CAP_H % line_h != 0 {
        di_lines += 1;
    }
    let di_lines = di_lines.max(0) as usize;

    // First pass — beside the drop cap, narrower box on the LEFT.
    let beside_h = (di_lines as i32 * line_h).min(box_h);
    let beside_w = (box_w - DROP_CAP_W).max(0);
    let remainder = super::layout::render_text_in_box(
        renderer,
        font,
        transform,
        text,
        box_x,
        box_y,
        beside_w,
        beside_h,
        TextAlign::Justified,
    );

    // Second pass — below the drop cap at full box width.
    if !remainder.is_empty() {
        let below_y = box_y + beside_h;
        let below_h = (box_h - beside_h).max(0);
        if below_h > 0 {
            let _ = super::layout::render_text_in_box(
                renderer,
                font,
                transform,
                &remainder,
                box_x,
                below_y,
                box_w,
                below_h,
                TextAlign::Justified,
            );
        }
    }
}

/// Blit the `mouth_frame`-th sub-frame of a horizontal portrait strip
/// with an optional constant alpha.
///
/// `alpha_percent` is 0..=100 to match
/// [`crate::renderer::Renderer::blit_to_screen_alpha`] — 0 skips the
/// blit entirely, 100 uses the opaque fast path, and any value in
/// between falls through to the alpha-modulated GPU blit.
#[allow(clippy::too_many_arguments)]
fn draw_portrait_frame_alpha(
    renderer: &mut Renderer,
    transform: MenuTransform,
    portrait: &super::resources::MenuSurface,
    vx: i32,
    vy: i32,
    vw: i32,
    vh: i32,
    mouth_frame: u8,
    alpha_percent: u16,
) {
    if alpha_percent == 0 {
        return;
    }

    let (sx, sy) = transform.to_screen(vx, vy);
    // Portrait sprites are a horizontal strip of 5 frames (mouth 0..4).
    const FRAMES: i32 = 5;
    let frame_w = (portrait.width / FRAMES).max(1);
    let frame_h = portrait.height;
    let fx = (mouth_frame as i32 % FRAMES) * frame_w;

    let src = BBox::from_coords(fx as f32, 0.0, (fx + frame_w) as f32, frame_h as f32);
    let dst = BBox::from_coords(sx as f32, sy as f32, (sx + vw) as f32, (sy + vh) as f32);

    if alpha_percent >= 100 {
        renderer.blit_to_screen(
            portrait.id,
            Some(&src),
            Some(&dst),
            crate::renderer::BLIT_SOURCE_TRANSPARENT,
        );
    } else {
        // Our `alpha_percent` is opacity (100 = opaque, 0 = transparent),
        // but `Renderer::blit_to_screen_alpha` uses the inverse
        // convention (0 = opaque, 100 = transparent). Invert the
        // percentage so a fade with `alpha_percent = 25` renders the
        // new portrait at 25 % opacity.
        renderer.blit_to_screen_alpha(
            portrait.id,
            Some(&src),
            Some(&dst),
            100 - alpha_percent,
            crate::renderer::BLIT_SOURCE_TRANSPARENT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The constructor seeds the machine so the first portrait fades
    /// in from nothing.
    #[test]
    fn portrait_fade_initial_state_is_fading_from_zero() {
        let f = PortraitFade::new(100);
        assert!(f.is_fading(), "initial render should fade in from nothing");
        assert_eq!(f.previous, NO_PORTRAIT);
        assert_eq!(f.current, 100);
        assert_eq!(f.fade_percent(), 0);
    }

    /// Cross-fade mid-dialogue: `set(new)` should capture the previous
    /// id, reset `fade` to 0, and leave `is_fading()` on.
    #[test]
    fn portrait_fade_starts_on_speaker_change() {
        let mut f = PortraitFade::new(100);
        // Complete the opening fade-in so we start from a steady state.
        for _ in 0..4 {
            f.tick();
        }
        assert!(!f.is_fading());
        assert_eq!(f.fade_percent(), 100);

        f.set(200);
        assert!(f.is_fading());
        assert_eq!(f.previous, 100);
        assert_eq!(f.current, 200);
        assert_eq!(f.fade_percent(), 0);
    }

    /// Setting the same portrait id is a no-op.
    #[test]
    fn portrait_fade_same_id_is_noop() {
        let mut f = PortraitFade::new(100);
        for _ in 0..4 {
            f.tick();
        }
        f.set(100);
        assert!(!f.is_fading());
        assert_eq!(f.fade_percent(), 100);
    }

    /// Four ticks of `0.25` should reach full opacity and snap the
    /// fade machine shut (four blended frames, then a fifth fully-
    /// opaque frame).
    #[test]
    fn portrait_fade_completes_after_four_ticks() {
        // Start from a steady state so the test exercises a
        // portrait-to-portrait transition, not the opening fade-in.
        let mut f = PortraitFade::new(100);
        for _ in 0..4 {
            f.tick();
        }
        f.set(200);
        assert_eq!(f.fade_percent(), 0);

        f.tick();
        assert_eq!(f.fade_percent(), 25);
        assert!(f.is_fading());

        f.tick();
        assert_eq!(f.fade_percent(), 50);

        f.tick();
        assert_eq!(f.fade_percent(), 75);

        f.tick();
        assert_eq!(f.fade_percent(), 100);
        assert!(!f.is_fading(), "fourth tick snaps the machine shut");
    }

    /// Interrupting a cross-fade with a third portrait restarts the
    /// transition — the just-fading-in one becomes the new `previous`.
    #[test]
    fn portrait_fade_retriggers_on_new_set_mid_transition() {
        let mut f = PortraitFade::new(100);
        for _ in 0..4 {
            f.tick();
        }
        f.set(200);
        f.tick(); // fade 0.25
        f.tick(); // fade 0.5

        f.set(300);
        assert_eq!(f.previous, 200);
        assert_eq!(f.current, 300);
        assert_eq!(f.fade_percent(), 0);
    }
}

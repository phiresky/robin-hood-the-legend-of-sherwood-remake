//! Popup-scroll parchment window.
//!
//! A 496x463 parchment window with a text box at `(50,50)..(450,375)`,
//! an optional right-side picture widget at `(446-w,50)..(446,50+h)`,
//! and a single OK button (`RHID_OK`) at the bottom.  Pagination is
//! handled by the shared `render_text_in_box_with_drop_cap` routine;
//! it returns the tail of the string that didn't fit, and we loop
//! until the whole body has been shown.
//!
//! When a picture is present, the text renderer reserves a
//! picture-shaped hole in the top-right corner of the text box: the
//! first N lines wrap to a narrower width that ends before the picture,
//! and subsequent lines use the full body width.
//!
//! Buttons are driven by the [`crate::widget`] system via the
//! [`super::widget_bridge`].

use std::sync::atomic::{AtomicU32, Ordering};

use crate::gfx_types::Keycode;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::resource_ids;
use crate::sound::{AudioBackend, SoundManager};
use crate::sound_config::SoundConfig;
use robin_engine::coordinates::ScreenBBox;
use robin_engine::player_command::DialogResult;
use robin_engine::sound_cache::SampleLoader;

use super::layout::{
    MENU_H, MENU_W, MenuTransform, TextAlign, TextFontTable, TextWidgetState, TooltipState,
    dim_screen, draw_background, enter_modal_gpu_phase, render_text_in_box_with_drop_cap,
    render_text_virt,
};
use super::resources::{IngameMenuResources, MT_INFOBULLE_BUTTON_OK, MenuSurface};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

/// Virtual window geometry: `(0, 0, 496, 463)`.
pub const WIN_W: i32 = 496;
pub const WIN_H: i32 = 463;

/// Text box inside the window: `(left, top, right, bottom) = (50, 50,
/// 450, 375)`.  Width is therefore 400 and height is 325 (not 375 —
/// that's the absolute bottom edge).
const BODY_LEFT: i32 = 50;
const BODY_TOP: i32 = 50;
const BODY_RIGHT: i32 = 450;
const BODY_BOTTOM: i32 = 375;
const BODY_W: i32 = BODY_RIGHT - BODY_LEFT;
const BODY_H: i32 = BODY_BOTTOM - BODY_TOP;

/// Title strip above the body (when present).  The popup scroll has no
/// dedicated title widget; we reuse the top 40 px of the body area for
/// a centered title and push the body down by that much.
const TITLE_H: i32 = 40;

/// OK button vertical position; centered horizontally within the window.
const BTN_Y: i32 = 380;

/// Picture widget right edge — picture is placed at `(446 - pic_w, 50)`.
const PIC_RIGHT: i32 = 446;
const PIC_TOP: i32 = 50;

/// Extra padding added to the picture bounds when the text renderer
/// reserves the drop-cap hole (`pic_w + 20`, `pic_h + 15`).
const PIC_TEXT_PAD_X: i32 = 20;
const PIC_TEXT_PAD_Y: i32 = 15;

const BTN_OK_ID: u32 = 0;

/// Universal-frame-counter value when the last popup-scroll dismissed.
///
/// Written at the tail of `show_popup_scroll` and read by
/// [`needs_bkgnd_colorization`] when the next popup opens.
static LAST_FRAME: AtomicU32 = AtomicU32::new(u32::MAX);

/// Returns `true` when the current universal frame differs from the last
/// frame a popup was dismissed on (i.e. the backdrop needs to be dimmed
/// freshly).  When two popups fire back-to-back in the same engine
/// frame, returns `false` so the second popup reuses the already-dimmed
/// scene without re-colorizing.
fn needs_bkgnd_colorization(universal_frame: u32) -> bool {
    LAST_FRAME.load(Ordering::Relaxed) != universal_frame
}

/// Display a popup-scroll window with optional title, optional picture,
/// and paginated body.
///
/// Blocks until the player dismisses the window (clicks OK, presses
/// Enter, Space, or Escape).  When the body text overflows the text
/// box, the function loops, showing further pages on each successive
/// OK.
///
/// `picture` is the optional pre-loaded portrait / illustration shown
/// in the top-right corner of the scroll.  `None` means no picture.
/// The caller is responsible for picking the right resource file for
/// the lookup — popup-text pictures live in the level `.res` (same
/// file the text table came from), not in `DEFAULT.RES` which
/// `IngameMenuResources` owns.
#[allow(clippy::too_many_arguments)]
pub async fn show_popup_scroll(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &mut IngameMenuResources,
    sound: &mut SoundManager,
    _sound_cfg: &SoundConfig,
    audio_backend: Option<&mut dyn AudioBackend>,
    _sound_enabled: bool,
    sample_loader: &SampleLoader,
    mut cursor: Option<ModalCursor<'_>>,
    title: Option<&str>,
    picture: Option<MenuSurface>,
    body: &str,
    body_font_name: Option<&str>,
    align: TextAlign,
    universal_frame: u32,
    replay_result: Option<robin_engine::player_command::DialogResult>,
    modal_net: Option<super::ModalNet<'_>>,
) -> robin_engine::player_command::DialogResult {
    // During replay, skip the interactive pages entirely — the window
    // is purely informational so the only thing to reproduce is the
    // dismissal edge. Matches what `show_dialogue` does.
    if let Some(res) = replay_result {
        return res;
    }

    let mut state = PopupScrollModalState::new(
        event_pump,
        renderer,
        resources,
        title.map(str::to_string),
        picture,
        body.to_string(),
        body_font_name.map(str::to_string),
        align,
        universal_frame,
    );
    let mut audio_slot = audio_backend;
    loop {
        let result = state.tick(
            event_pump,
            renderer,
            resources,
            sound,
            audio_slot
                .as_mut()
                .map(|b| &mut **b as &mut dyn AudioBackend),
            sample_loader,
            cursor.as_mut().map(|c| c.reborrow()),
            modal_net.as_ref(),
        );
        if let Some(result) = result {
            return result;
        }
        crate::window::sleep_ms(16).await;
    }
}

/// One-frame popup-scroll modal state.
///
/// The legacy [`show_popup_scroll`] wrapper drives this state in a local
/// loop. The mission loop owns it directly for multiplayer so networking
/// and replay bookkeeping keep advancing while the parchment is visible.
pub struct PopupScrollModalState {
    title: Option<String>,
    picture: Option<MenuSurface>,
    body_font_name: Option<String>,
    align: TextAlign,
    universal_frame: u32,
    colorize: bool,
    transform: MenuTransform,
    virt_x: i32,
    virt_y: i32,
    frame: crate::widget::FrameWnd,
    input_state: ModalInputState,
    tooltip: TooltipState,
    picture_widget: Option<crate::widget::WidgetPicture>,
    body_y: i32,
    body_h: i32,
    pic_w: i32,
    pic_h: i32,
    drop_cap_w: i32,
    drop_cap_h: i32,
    page_body: String,
    text_remaining: String,
}

impl PopupScrollModalState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &mut IngameMenuResources,
        title: Option<String>,
        picture: Option<MenuSurface>,
        body: String,
        body_font_name: Option<String>,
        align: TextAlign,
        universal_frame: u32,
    ) -> Self {
        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        let transform = MenuTransform::centered(sw, sh);

        let virt_x = (MENU_W - WIN_W) / 2;
        let virt_y = (MENU_H - WIN_H) / 2;

        let (btn_w, btn_h) = resources.ok_button_dimensions();
        let bx = virt_x + (WIN_W - btn_w) / 2;
        let by = virt_y + BTN_Y;

        let mut frame = crate::widget::FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            BTN_OK_ID,
            "",
            true,
            resource_ids::RHID_OK,
            bx,
            by,
            btn_w,
            btn_h,
        ));

        let ok_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_OK);
        if let Some(w) = frame.widget_mut(BTN_OK_ID) {
            w.base_mut().set_tooltip_text(&ok_tooltip);
        }
        widget_bridge::attach_alpha_masks(&mut frame, resources, renderer);

        let mut input_state = ModalInputState::new();
        input_state.seed_mouse_from_sdl(event_pump, transform);

        let mut state = Self {
            title,
            picture,
            body_font_name,
            align,
            universal_frame,
            colorize: needs_bkgnd_colorization(universal_frame),
            transform,
            virt_x,
            virt_y,
            frame,
            input_state,
            tooltip: TooltipState::new(),
            picture_widget: None,
            body_y: 0,
            body_h: 0,
            pic_w: 0,
            pic_h: 0,
            drop_cap_w: 0,
            drop_cap_h: 0,
            page_body: body,
            text_remaining: String::new(),
        };
        state.rebuild_page_widgets();
        state
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        sound: &mut SoundManager,
        audio_backend: Option<&mut dyn AudioBackend>,
        sample_loader: &SampleLoader,
        cursor: Option<ModalCursor<'_>>,
        modal_net: Option<&super::ModalNet<'_>>,
    ) -> Option<DialogResult> {
        let mut dismissed = false;
        let remote_result = modal_net.and_then(|net| net.poll_remote_dismissal());
        if remote_result.is_some() {
            dismissed = true;
        }

        for event in event_pump.poll_events() {
            self.input_state.update_from_event(&event, self.transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => dismissed = true,
                _ => {}
            }
        }

        let widget_input = self.input_state.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        self.input_state.end_frame();
        if let Some(backend) = audio_backend {
            widget_bridge::play_widget_noise(
                &events,
                widget_bridge::WIDGET_NOISY_BUTTON,
                sound,
                Some(backend),
                sample_loader,
            );
        }
        if widget_bridge::find_activated(&events).is_some() {
            dismissed = true;
        }

        self.render(renderer, resources, cursor.as_ref());
        renderer.present();

        if let Some(result) = remote_result {
            return Some(self.finish(result, true, modal_net));
        }
        if dismissed {
            if !self.text_remaining.is_empty() && self.text_remaining != self.page_body {
                self.page_body = std::mem::take(&mut self.text_remaining);
                self.rebuild_page_widgets();
                return None;
            }
            return Some(self.finish(DialogResult::Completed, false, modal_net));
        }
        None
    }

    fn rebuild_page_widgets(&mut self) {
        let (body_y, body_h) = if self.title.is_some() {
            (self.virt_y + BODY_TOP + TITLE_H, BODY_H - TITLE_H)
        } else {
            (self.virt_y + BODY_TOP, BODY_H)
        };
        self.body_y = body_y;
        self.body_h = body_h;

        let (pic_virt_x, pic_virt_y, pic_w, pic_h) = match self.picture {
            Some(p) => {
                let pw = p.width;
                let ph = p.height;
                (PIC_RIGHT - pw, PIC_TOP, pw, ph)
            }
            None => (0, 0, 0, 0),
        };
        self.pic_w = pic_w;
        self.pic_h = pic_h;
        self.drop_cap_w = if pic_w > 0 { pic_w + PIC_TEXT_PAD_X } else { 0 };
        self.drop_cap_h = if pic_h > 0 { pic_h + PIC_TEXT_PAD_Y } else { 0 };
        self.picture_widget = self.picture.map(|pic| {
            let mut widget = crate::widget::WidgetPicture::new(u32::MAX);
            let bbox = ScreenBBox::from_coords(
                (self.virt_x + pic_virt_x) as f32,
                (self.virt_y + pic_virt_y) as f32,
                (self.virt_x + pic_virt_x + pic_w) as f32,
                (self.virt_y + pic_virt_y + pic_h) as f32,
            );
            widget.base.create("", bbox, 0);
            widget.set_alternate_picture(pic.id);
            widget
        });
        self.text_remaining.clear();
    }

    fn finish(
        &self,
        result: DialogResult,
        remote: bool,
        modal_net: Option<&super::ModalNet<'_>>,
    ) -> DialogResult {
        LAST_FRAME.store(self.universal_frame, Ordering::Relaxed);
        if !remote && let Some(net) = modal_net {
            net.publish(result);
        }
        result
    }

    fn render(
        &mut self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) {
        enter_modal_gpu_phase(renderer);
        if self.colorize {
            dim_screen(renderer);
        }

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

        if let Some(widget) = &self.picture_widget {
            widget_bridge::draw_picture_alternate_surface(
                renderer,
                self.transform,
                widget,
                self.pic_w,
                self.pic_h,
                true,
            );
        }

        if let (Some(t), Some(font)) = (self.title.as_deref(), resources.title_font())
            && !t.is_empty()
        {
            let tw = font.text_width(t);
            let tx = self.virt_x + BODY_LEFT + (BODY_W - tw) / 2;
            let ty = self.virt_y + BODY_TOP + (TITLE_H - font.height() as i32) / 2;
            render_text_virt(renderer, font, self.transform, t, tx, ty);
        }

        let body_font = self
            .body_font_name
            .as_deref()
            .and_then(|name| resources.font_by_name(name))
            .or_else(|| resources.popup_font());
        let fonts = TextFontTable::uniform(body_font);
        self.text_remaining = render_text_in_box_with_drop_cap(
            renderer,
            &fonts,
            TextWidgetState::Default,
            self.transform,
            &self.page_body,
            self.virt_x + BODY_LEFT,
            self.body_y,
            BODY_W,
            self.body_h,
            self.drop_cap_w,
            self.drop_cap_h,
            self.align,
        );

        widget_bridge::draw_frame_buttons(renderer, resources, self.transform, &self.frame);

        let mouse_pt = robin_engine::coordinates::ScreenPoint::new(
            self.input_state.virt_x,
            self.input_state.virt_y,
        );
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

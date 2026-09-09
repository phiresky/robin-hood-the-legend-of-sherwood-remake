//! Load-slot modal and shared save-picker presentation helpers.
//!
//! Main-menu and terminal loading drive one LoadPickerModalState. In-mission
//! saving owns its cooperative UI task and reuses the picker/name-edit helpers.

use crate::gfx_types::Keycode;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::sound_cache::SampleLoader;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::savegame::{SaveGame, SaveGameManager, SlotName};
use crate::sound::{AudioBackend, SoundManager};
use crate::ui::{MouseButtons, UiKeyboard, UiState};
use crate::widget::{WidgetInput, WidgetInputField, WidgetPicture};
use jiff::{Timestamp, tz::TimeZone};

use super::layout::{
    MenuRect, MenuTransform, align_bottom_right, dim_screen, draw_screen_background,
    enter_modal_gpu_phase, render_text_virt_font,
};
use super::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_DELETE, MT_BTN_LOAD, MT_MSG_REALLY_DELETE_SAVEGAME,
};
pub(crate) use super::save_picker::{
    ID_CANCEL, ID_DELETE, ID_LOAD_SAVE, ListRow, PickerAction, PickerController, PickerModel,
    PickerSlot, PickerTarget, retire_thumbnail,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};
use super::yesno::YesNoModalState;

/// Which flavour of slot picker to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SaveLoadMode {
    Save,
    Load,
}

/// Outcome of the picker — the caller turns this into a `SaveLoadRequest`.
#[derive(Debug, Clone, Copy)]
pub enum SaveLoadOutcome {
    /// User cancelled; no action.
    Cancel,
    /// User accepted the save or load, referring to `saves[slot]` on the
    /// save manager at the time the outcome was produced.
    Slot(usize),
}

/// One-frame load-slot picker used by terminal mission debriefing.
///
/// The main menu drives this state with a small async loop; mission-time loads
/// tick it cooperatively so networking and automation continue between frames.
pub struct LoadPickerModalState {
    model: PickerModel,
    thumb_widget: WidgetPicture,
    thumb_cache: Option<ThumbnailCache>,
    controller: PickerController,
    noise_tracker: widget_bridge::NoisyTracker,
    delete_confirmation: Option<YesNoModalState>,
    error_notice: Option<crate::save_recovery::ErrorNotice>,
    detailed_metadata: bool,
    local_time_zone: Option<TimeZone>,
    clock_error_reported: bool,
}

impl LoadPickerModalState {
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        save_manager: &mut SaveGameManager,
        detailed_metadata: bool,
        multiplayer_connected: bool,
    ) -> Self {
        save_manager.sort_by_time();
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let mut input_state = ModalInputState::new();
        input_state.seed_mouse_from_window(event_pump, transform);
        let row_height = if detailed_metadata {
            DETAILED_ROW_HEIGHT
        } else {
            COMPACT_ROW_HEIGHT
        };
        Self {
            model: PickerModel::new(
                SaveLoadMode::Load,
                multiplayer_connected,
                (LOAD_LIST_RECT.h / row_height).max(1) as usize,
                picker_slots(save_manager),
            ),
            thumb_widget: WidgetPicture::new(u32::MAX),
            thumb_cache: None,
            controller: PickerController::new(input_state),
            noise_tracker: widget_bridge::NoisyTracker::new(),
            delete_confirmation: None,
            error_notice: None,
            detailed_metadata,
            local_time_zone: TimeZone::try_system()
                .inspect_err(|error| tracing::warn!("Save menu local time is unavailable: {error}"))
                .ok(),
            clock_error_reported: false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<ModalCursor<'_>>,
        save_manager: &mut SaveGameManager,
        sound: Option<&mut SoundManager>,
        audio_backend: Option<&mut dyn AudioBackend>,
        sample_loader: Option<&SampleLoader>,
    ) -> Option<SaveLoadOutcome> {
        self.model.refresh(picker_slots(save_manager));
        if self.error_notice.is_none()
            && let Some(error) = self.model.operation_error()
        {
            self.error_notice = Some(crate::save_recovery::ErrorNotice::new(error.to_string()));
        }
        if let Some(notice) = &mut self.error_notice {
            if notice.tick(event_pump, renderer, resources, cursor.as_ref()) {
                self.error_notice = None;
                self.model.dismiss_error();
                if event_pump.close_requested {
                    return Some(SaveLoadOutcome::Cancel);
                }
            }
            // This frame belongs exclusively to the notice. The outer mission
            // driver can still service networking and automation between ticks.
            return None;
        }
        if let Some(confirmation) = self.delete_confirmation.as_mut() {
            let outcome = confirmation.tick(event_pump, renderer, resources, cursor.as_ref());
            let confirmed = outcome?;
            self.delete_confirmation = None;
            finish_picker_delete(&mut self.model, save_manager, confirmed);
            sync_thumbnail_cache(
                &mut self.thumb_cache,
                &mut self.thumb_widget,
                self.model.selected_row(),
                &self.model.visible(),
                save_manager,
                renderer,
                SaveLoadMode::Load,
            );
            return None;
        }

        let visible = self.model.visible();

        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let row_height = if self.detailed_metadata {
            DETAILED_ROW_HEIGHT
        } else {
            COMPACT_ROW_HEIGHT
        };
        let (btn_w, btn_h) = resources.button_dimensions();
        let load_label = resources.menu_text.get(MT_BTN_LOAD);
        let delete_label = resources.menu_text.get(MT_BTN_DELETE);
        let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);
        let bottom_buttons = align_bottom_right(
            &[
                (&load_label, false),
                (&delete_label, false),
                (&cancel_label, true),
            ],
            btn_w,
            btn_h,
        );
        let btn_positions = [
            (
                ID_LOAD_SAVE,
                load_label.as_str(),
                bottom_buttons[0].x,
                bottom_buttons[0].y,
            ),
            (
                ID_DELETE,
                delete_label.as_str(),
                bottom_buttons[1].x,
                bottom_buttons[1].y,
            ),
            (
                ID_CANCEL,
                cancel_label.as_str(),
                bottom_buttons[2].x,
                bottom_buttons[2].y,
            ),
        ];
        self.controller
            .configure_list(&mut self.model, LOAD_LIST_RECT, row_height, resources);
        self.controller
            .begin_frame(&self.model, &btn_positions, btn_w, btn_h);
        for event in event_pump.poll_events() {
            self.controller
                .handle_event(&mut self.model, &event, transform);
        }
        let selected = self.model.selected_row();
        let widget_events = self.controller.process_widgets(&self.model);
        let widget_input = self.controller.input.as_widget_input();
        let mouse_virt = widget_input.mouse_position;
        self.controller.input.end_frame();
        if let (Some(sound), Some(loader)) = (sound, sample_loader) {
            widget_bridge::play_frame_widget_noise(
                &widget_events,
                self.controller.frame(),
                widget_bridge::WIDGET_NOISY_BUTTON,
                sound,
                audio_backend,
                loader,
                &mut self.noise_tracker,
            );
        }
        match self.controller.take_action() {
            Some(PickerAction::Cancel) => return Some(SaveLoadOutcome::Cancel),
            Some(PickerAction::Accept(PickerTarget::Existing(name))) => {
                match save_manager.find_by_filename(name.as_str()) {
                    Some(slot) => return Some(SaveLoadOutcome::Slot(slot)),
                    None => self
                        .model
                        .report_error("the selected save is no longer available".into()),
                }
            }
            Some(PickerAction::ConfirmDelete(name)) => {
                if begin_picker_delete(&mut self.model, name) {
                    let message = resources.menu_text.get(MT_MSG_REALLY_DELETE_SAVEGAME);
                    self.delete_confirmation = Some(YesNoModalState::new(
                        event_pump, renderer, resources, message,
                    ));
                }
            }
            Some(PickerAction::Accept(PickerTarget::New)) => {
                unreachable!("Load picker cannot create a save")
            }
            None => {}
        }

        sync_thumbnail_cache(
            &mut self.thumb_cache,
            &mut self.thumb_widget,
            selected,
            &visible,
            save_manager,
            renderer,
            SaveLoadMode::Load,
        );
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(background) = resources.menu_bg[3] {
            draw_screen_background(renderer, &background);
        }
        let metadata_text = EnglishSaveMetadataText;
        let now_unix = if self.detailed_metadata {
            match crate::save_file::unix_timestamp_now() {
                Ok(now) => Some(now),
                Err(error) => {
                    if !self.clock_error_reported {
                        tracing::warn!("Save menu relative time is unavailable: {error:#}");
                        self.clock_error_reported = true;
                    }
                    None
                }
            }
        } else {
            None
        };
        let view = self.controller.view();
        let row_area_x = LOAD_LIST_RECT.x + 10;
        let row_area_w = view.content_width() - 20;
        let hovered_row = view
            .row_at(mouse_virt.x as i32, mouse_virt.y as i32)
            .and_then(|row| self.model.row_at(row));
        for row_index in view.visible_range() {
            let row = self
                .model
                .row_at(row_index)
                .expect("rendered row is within model bounds");
            let row_y = view.row_y(row_index);
            let Some(font) = resources.list_font(hovered_row == Some(row), selected == Some(row))
            else {
                continue;
            };
            let label = truncate_to_pixel_width(
                font,
                &row_label(row, save_manager, &visible, &metadata_text),
                row_area_w,
            );
            if !label.is_empty() {
                render_text_virt_font(renderer, font, transform, &label, row_area_x, row_y);
            }
            for (line_index, detail) in row_detail_lines(
                row,
                save_manager,
                &visible,
                now_unix,
                self.local_time_zone.as_ref(),
                &metadata_text,
                self.detailed_metadata,
            )
            .iter()
            .filter(|line| !line.is_empty())
            .enumerate()
            {
                let detail = truncate_to_pixel_width(font, detail, row_area_w);
                if !detail.is_empty() {
                    render_text_virt_font(
                        renderer,
                        font,
                        transform,
                        &detail,
                        row_area_x,
                        row_y + DETAIL_LINE_HEIGHT * (line_index as i32 + 1),
                    );
                }
            }
        }
        view.draw_scrollbar(renderer, transform, resources);
        draw_preview(
            renderer,
            transform,
            selected,
            &visible,
            self.thumb_cache.as_ref(),
            &self.thumb_widget,
            save_manager,
            resources,
            now_unix,
            self.local_time_zone.as_ref(),
            &metadata_text,
            self.detailed_metadata,
        );
        widget_bridge::draw_frame_buttons(renderer, resources, transform, self.controller.frame());
        if let Some(cursor) = &cursor {
            cursor.draw(renderer, transform, &self.controller.input);
        }
        renderer.present();
        None
    }

    pub fn close(&mut self, renderer: &mut Renderer) {
        clear_thumbnail_cache(&mut self.thumb_cache, &mut self.thumb_widget, renderer);
    }
}

const LOAD_LIST_RECT: MenuRect = MenuRect {
    x: 30,
    y: 10,
    w: 420,
    h: 450,
};
const THUMB_RECT: MenuRect = MenuRect {
    x: 460,
    y: 0,
    w: 180,
    h: 135,
};
const COMPACT_ROW_HEIGHT: i32 = 36;
const DETAILED_ROW_HEIGHT: i32 = 52;
const DETAIL_LINE_HEIGHT: i32 = 16;

/// Units passed through the save-metadata localization seam. The original
/// string table has no relative-time phrases, so the save UI uses this small
/// adapter instead of inventing numeric Original resource IDs. A port-owned
/// language catalog can implement the same interface later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelativeTimeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

pub(crate) trait SaveMetadataText {
    fn new_save_label(&self) -> String;
    fn new_save_hint(&self) -> String;
    fn mission(&self, value: &str) -> String;
    fn player(&self, value: &str) -> String;
    fn saved(&self, value: &str) -> String;
    fn exact_date(&self, value: &str) -> String;
    fn campaign_progress(&self, progress: u32) -> String;
    fn missions(&self, done: usize, total: usize) -> String;
    fn gang_size(&self, size: usize) -> String;
    fn ransom(&self, value: i32) -> String;
    fn blazons(&self, value: i32) -> String;
    fn amulets(&self, value: i32) -> String;
    fn legacy_value_unavailable(&self) -> String;
    fn invalid_timestamp(&self) -> String;
    fn relative_time_unavailable(&self) -> String;
    fn local_time_unavailable(&self) -> String;
    fn just_now(&self) -> String;
    fn elapsed(&self, value: u64, unit: RelativeTimeUnit) -> String;
    fn future(&self, value: u64, unit: RelativeTimeUnit) -> String;
    fn compact_saved(&self, value: &str) -> String;
    fn compact_campaign_progress(&self, progress: u32) -> String;
    fn compact_missions(&self, done: usize, total: usize) -> String;
    fn compact_gang_size(&self, size: usize) -> String;
    fn compact_ransom(&self, value: i32) -> String;
    fn compact_blazons(&self, value: i32) -> String;
    fn compact_amulets(&self, value: i32) -> String;
}

/// English fallback used until the port-wide language catalog supplies an
/// implementation of [`SaveMetadataText`]. Keeping all new copy behind the
/// adapter prevents relative-time grammar from leaking through the UI code.
pub(crate) struct EnglishSaveMetadataText;

impl EnglishSaveMetadataText {
    fn quantity(value: u64, unit: RelativeTimeUnit) -> String {
        let singular = match unit {
            RelativeTimeUnit::Second => "second",
            RelativeTimeUnit::Minute => "minute",
            RelativeTimeUnit::Hour => "hour",
            RelativeTimeUnit::Day => "day",
            RelativeTimeUnit::Week => "week",
            RelativeTimeUnit::Month => "month",
            RelativeTimeUnit::Year => "year",
        };
        if value == 1 {
            format!("1 {singular}")
        } else {
            format!("{value} {singular}s")
        }
    }
}

impl SaveMetadataText for EnglishSaveMetadataText {
    fn new_save_label(&self) -> String {
        "< New Save >".to_string()
    }

    fn new_save_hint(&self) -> String {
        "Name optional - creates a new save slot".to_string()
    }

    fn mission(&self, value: &str) -> String {
        format!("Mission: {value}")
    }

    fn player(&self, value: &str) -> String {
        format!("Player: {value}")
    }

    fn saved(&self, value: &str) -> String {
        format!("Saved: {value}")
    }

    fn exact_date(&self, value: &str) -> String {
        format!("Date: {value}")
    }

    fn campaign_progress(&self, progress: u32) -> String {
        format!("Campaign: {progress}%")
    }

    fn missions(&self, done: usize, total: usize) -> String {
        format!("Missions: {done}/{total}")
    }

    fn gang_size(&self, size: usize) -> String {
        format!("Gang: {size}")
    }

    fn ransom(&self, value: i32) -> String {
        format!("Ransom: {value}")
    }

    fn blazons(&self, value: i32) -> String {
        format!("Blazons: {value}")
    }

    fn amulets(&self, value: i32) -> String {
        format!("Amulets: {value}")
    }

    fn legacy_value_unavailable(&self) -> String {
        "unavailable (legacy save)".to_string()
    }

    fn invalid_timestamp(&self) -> String {
        "invalid timestamp".to_string()
    }

    fn relative_time_unavailable(&self) -> String {
        "relative time unavailable".to_string()
    }

    fn local_time_unavailable(&self) -> String {
        "local time unavailable".to_string()
    }

    fn just_now(&self) -> String {
        "just now".to_string()
    }

    fn elapsed(&self, value: u64, unit: RelativeTimeUnit) -> String {
        format!("{} ago", Self::quantity(value, unit))
    }

    fn future(&self, value: u64, unit: RelativeTimeUnit) -> String {
        format!("in {}", Self::quantity(value, unit))
    }

    fn compact_saved(&self, value: &str) -> String {
        format!("Saved {value}")
    }

    fn compact_campaign_progress(&self, progress: u32) -> String {
        format!("{progress}% campaign")
    }

    fn compact_missions(&self, done: usize, total: usize) -> String {
        format!("{done}/{total} missions")
    }

    fn compact_gang_size(&self, size: usize) -> String {
        format!("Gang {size}")
    }

    fn compact_ransom(&self, value: i32) -> String {
        format!("Ransom {value}")
    }

    fn compact_blazons(&self, value: i32) -> String {
        format!("Blazons {value}")
    }

    fn compact_amulets(&self, value: i32) -> String {
        format!("Amulets {value}")
    }
}

/// Longest allowed save name — passed to the input field as its
/// max-length cap.
#[cfg(test)]
const MAX_NAME_LEN: usize = 45;

/// Main-menu load picker. Mission-time loading drives the same state one tick
/// at a time; saving belongs to the cooperative in-mission save task.
pub async fn show_load_picker(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    mut cursor: Option<ModalCursor<'_>>,
    save_manager: &mut SaveGameManager,
    detailed_metadata: bool,
) -> SaveLoadOutcome {
    let mut state =
        LoadPickerModalState::new(event_pump, renderer, save_manager, detailed_metadata, false);
    loop {
        if let Some(outcome) = state.tick(
            event_pump,
            renderer,
            resources,
            cursor.as_mut().map(|cursor| cursor.reborrow()),
            save_manager,
            None,
            None,
            None,
        ) {
            state.close(renderer);
            return outcome;
        }
        crate::window::sleep_ui_frame().await;
    }
}

/// Tracks a loaded thumbnail so we don't rebuild the GPU surface on
/// every frame while the selection is stable.
#[derive(serde::Serialize, serde::Deserialize)]
struct ThumbnailCache {
    slot: SlotName,
    surface: crate::renderer::OwnedSurface,
    width: u16,
    height: u16,
}

fn sync_thumbnail_cache(
    cache: &mut Option<ThumbnailCache>,
    widget: &mut crate::widget::WidgetPicture,
    selected: Option<ListRow>,
    visible: &[usize],
    save_manager: &SaveGameManager,
    renderer: &mut Renderer,
    mode: SaveLoadMode,
) {
    if let Some(current) = cache.as_ref() {
        renderer
            .surface_dimensions(current.surface.handle())
            .expect("thumbnail cache requires its originating renderer");
    }
    // Save-mode never previews a thumbnail — the picture widget stays
    // disabled and the entire reload branch is gated on Load mode.
    let target_slot = match (mode, selected) {
        (SaveLoadMode::Load, Some(ListRow::Existing(v))) => Some(
            *visible
                .get(v)
                .expect("selected thumbnail row must be visible"),
        ),
        _ => None,
    };
    let target_name = target_slot.map(|slot| {
        save_manager
            .slot_name(slot)
            .expect("thumbnail slot must have a validated identity")
    });
    if retire_thumbnail(
        cache.as_ref().map(|cache| &cache.slot),
        target_name.as_ref(),
    ) {
        clear_thumbnail_cache(cache, widget, renderer);
    }
    match (&*cache, target_slot) {
        (Some(c), Some(_)) if Some(&c.slot) == target_name.as_ref() => {}
        (_, None) => {
            clear_thumbnail_cache(cache, widget, renderer);
        }
        (_, Some(slot)) => {
            clear_thumbnail_cache(cache, widget, renderer);
            if let Some(thumb) = save_manager.load_thumbnail(slot) {
                let surface = renderer
                    .upload_rgb565(thumb.width, thumb.height, &thumb.pixels)
                    .expect("save thumbnail dimensions must match RGB565 payload");
                widget.set_alternate_picture(surface.handle());
                *cache = Some(ThumbnailCache {
                    slot: target_name.expect("thumbnail load requires a slot identity"),
                    surface,
                    width: thumb.width,
                    height: thumb.height,
                });
            }
        }
    }
}

fn clear_thumbnail_cache(
    cache: &mut Option<ThumbnailCache>,
    widget: &mut WidgetPicture,
    renderer: &mut Renderer,
) {
    if let Some(old) = cache.take() {
        renderer.retire_surface(old.surface);
    }
    widget.reset_alternate_picture();
}

#[allow(clippy::too_many_arguments)]
fn draw_preview(
    renderer: &mut Renderer,
    transform: MenuTransform,
    selected: Option<ListRow>,
    visible: &[usize],
    thumb_cache: Option<&ThumbnailCache>,
    thumb_widget: &crate::widget::WidgetPicture,
    save_manager: &SaveGameManager,
    resources: &IngameMenuResources,
    now_unix: Option<u64>,
    local_time_zone: Option<&TimeZone>,
    text: &impl SaveMetadataText,
    detailed_metadata: bool,
) {
    let slot = match selected {
        Some(ListRow::Existing(v)) => *visible
            .get(v)
            .expect("selected visible row must resolve to a save slot"),
        _ => return,
    };

    // Thumbnail image. The original game's load/save menu disables the picture
    // widget when there is no selected save or no thumbnail file; it
    // does not draw a placeholder frame or metadata panel.
    if let Some(cache) = thumb_cache
        && cache.slot
            == save_manager
                .slot_name(slot)
                .expect("preview slot must have a validated identity")
    {
        renderer
            .surface_dimensions(cache.surface.handle())
            .expect("thumbnail drawing requires its originating renderer");
        let mut widget = thumb_widget.clone();
        widget
            .base
            .set_position(engine_coordinates::ScreenBBox::from_coords(
                (THUMB_RECT.x + 4) as f32,
                (THUMB_RECT.y + 4) as f32,
                (THUMB_RECT.x + THUMB_RECT.w - 4) as f32,
                (THUMB_RECT.y + THUMB_RECT.h - 4) as f32,
            ));
        widget_bridge::draw_picture_alternate_surface(
            renderer,
            transform,
            &widget,
            i32::from(cache.width),
            i32::from(cache.height),
            true,
        );
    }

    if !detailed_metadata {
        return;
    }

    let save = save_manager
        .get(slot)
        .expect("selected visible slot must resolve to a save");
    let Some(font) = resources.list_font(false, true) else {
        return;
    };
    let panel_x = THUMB_RECT.x + 4;
    let panel_y = THUMB_RECT.y + THUMB_RECT.h + 8;
    let panel_w = THUMB_RECT.w - 8;
    for (line_index, line) in selected_metadata_lines(save, now_unix, local_time_zone, text)
        .iter()
        .enumerate()
    {
        let fitted = truncate_to_pixel_width(font, line, panel_w);
        if !fitted.is_empty() {
            render_text_virt_font(
                renderer,
                font,
                transform,
                &fitted,
                panel_x,
                panel_y + line_index as i32 * DETAIL_LINE_HEIGHT,
            );
        }
    }
}

/// Build the listbox row label. The original menu adds only
/// original-game save text to the list box.
fn row_label(
    row: ListRow,
    save_manager: &SaveGameManager,
    visible: &[usize],
    text: &impl SaveMetadataText,
) -> String {
    match row {
        ListRow::New => text.new_save_label(),
        ListRow::Existing(v_idx) => {
            let slot = visible[v_idx];
            let save = save_manager
                .get(slot)
                .expect("visible slot must resolve to a save");
            if save.is_autosave() {
                format!("Autosave - {}", save.text)
            } else {
                save.text.clone()
            }
        }
    }
}

fn row_detail_lines(
    row: ListRow,
    save_manager: &SaveGameManager,
    visible: &[usize],
    now_unix: Option<u64>,
    local_time_zone: Option<&TimeZone>,
    text: &impl SaveMetadataText,
    detailed_metadata: bool,
) -> [String; 2] {
    match row {
        ListRow::New => [text.new_save_hint(), String::new()],
        ListRow::Existing(v_idx) => {
            let slot = visible[v_idx];
            let save = save_manager
                .get(slot)
                .expect("visible slot must resolve to a save");
            existing_save_row_detail_lines(save, now_unix, local_time_zone, text, detailed_metadata)
        }
    }
}

fn existing_save_row_detail_lines(
    save: &SaveGame,
    now_unix: Option<u64>,
    local_time_zone: Option<&TimeZone>,
    text: &impl SaveMetadataText,
    detailed_metadata: bool,
) -> [String; 2] {
    if !detailed_metadata {
        return [
            compact_row_detail(save, local_time_zone, text),
            String::new(),
        ];
    }

    let mission = mission_with_time(save, text);
    let player = metadata_value(&save.player_name, text);
    let relative = format_relative_saved_time(&save.timestamp, now_unix, text);
    let exact = format_exact_saved_time(&save.timestamp, local_time_zone, text);
    [
        format!("{} | {}", text.mission(&mission), text.player(&player)),
        format!("{} | {}", text.saved(&relative), text.exact_date(&exact)),
    ]
}

/// Shared metadata presentation used by the frame-owned in-mission picker.
/// Keeping this adapter here ensures the cooperative and standalone menus use
/// the same relative-time, provenance, and compact-detail wording.
pub(crate) fn cooperative_save_row_detail_lines(
    save: &SaveGame,
    detailed_metadata: bool,
    now_unix: Option<u64>,
    local_time_zone: Option<&TimeZone>,
) -> [String; 2] {
    existing_save_row_detail_lines(
        save,
        now_unix,
        local_time_zone,
        &EnglishSaveMetadataText,
        detailed_metadata,
    )
}

fn compact_row_detail(
    save: &SaveGame,
    local_time_zone: Option<&TimeZone>,
    text: &impl SaveMetadataText,
) -> String {
    let exact = format_compact_saved_time(&save.timestamp, local_time_zone, text);
    let mut parts = vec![text.compact_saved(&exact)];
    if let Some(seconds) = save.mission_elapsed_seconds {
        parts.push(format!("{:02}:{:02}", seconds / 60, seconds % 60));
    }
    if !save.mission_name.is_empty() && save.text.trim() != save.mission_name.trim() {
        parts.push(save.mission_name.clone());
    }
    if let Some(progress) = save.campaign_progress {
        parts.push(text.compact_campaign_progress(progress));
    }
    if let (Some(done), Some(total)) = (save.missions_done, save.missions_total) {
        parts.push(text.compact_missions(done, total));
    }
    if let Some(gang) = save.gang_size {
        parts.push(text.compact_gang_size(gang));
    }
    if let Some(ransom) = save.ransom {
        parts.push(text.compact_ransom(ransom));
    }
    if let Some(blazons) = save.blazons {
        parts.push(text.compact_blazons(blazons));
    }
    if let Some(amulets) = save.amulets {
        parts.push(text.compact_amulets(amulets));
    }
    parts.join(" | ")
}

fn selected_metadata_lines(
    save: &SaveGame,
    now_unix: Option<u64>,
    local_time_zone: Option<&TimeZone>,
    text: &impl SaveMetadataText,
) -> Vec<String> {
    let mission = mission_with_time(save, text);
    let player = metadata_value(&save.player_name, text);
    let relative = format_relative_saved_time(&save.timestamp, now_unix, text);
    let exact = format_exact_saved_time(&save.timestamp, local_time_zone, text);
    let mut lines = vec![
        text.mission(&mission),
        text.player(&player),
        text.saved(&relative),
        text.exact_date(&exact),
    ];
    if let Some(progress) = save.campaign_progress {
        lines.push(text.campaign_progress(progress));
    }
    if let (Some(done), Some(total)) = (save.missions_done, save.missions_total) {
        lines.push(text.missions(done, total));
    }
    if let Some(gang) = save.gang_size {
        lines.push(text.gang_size(gang));
    }
    if let Some(ransom) = save.ransom {
        lines.push(text.ransom(ransom));
    }
    if let Some(blazons) = save.blazons {
        lines.push(text.blazons(blazons));
    }
    if let Some(amulets) = save.amulets {
        lines.push(text.amulets(amulets));
    }
    lines
}

fn mission_with_time(save: &SaveGame, text: &impl SaveMetadataText) -> String {
    let mission = metadata_value(&save.mission_name, text);
    match save.mission_elapsed_seconds {
        Some(seconds) => format!("{mission} ({:02}:{:02})", seconds / 60, seconds % 60),
        None => mission,
    }
}

fn metadata_value(value: &str, text: &impl SaveMetadataText) -> String {
    if value.is_empty() {
        text.legacy_value_unavailable()
    } else {
        value.to_string()
    }
}

fn parse_save_timestamp(timestamp: &str) -> Result<u64, ()> {
    timestamp.parse::<u64>().map_err(|_| ())
}

fn format_exact_saved_time(
    timestamp: &str,
    time_zone: Option<&TimeZone>,
    text: &impl SaveMetadataText,
) -> String {
    let Ok(seconds) = parse_save_timestamp(timestamp) else {
        return text.invalid_timestamp();
    };
    let Ok(seconds) = i64::try_from(seconds) else {
        return text.invalid_timestamp();
    };
    let Ok(timestamp) = Timestamp::from_second(seconds) else {
        return text.invalid_timestamp();
    };
    let Some(time_zone) = time_zone else {
        return text.local_time_unavailable();
    };
    timestamp
        .to_zoned(time_zone.clone())
        .strftime("%Y-%m-%d %H:%M:%S %Z")
        .to_string()
}

fn format_compact_saved_time(
    timestamp: &str,
    time_zone: Option<&TimeZone>,
    text: &impl SaveMetadataText,
) -> String {
    let Ok(seconds) = parse_save_timestamp(timestamp) else {
        return text.invalid_timestamp();
    };
    let Ok(seconds) = i64::try_from(seconds) else {
        return text.invalid_timestamp();
    };
    let Ok(timestamp) = Timestamp::from_second(seconds) else {
        return text.invalid_timestamp();
    };
    let Some(time_zone) = time_zone else {
        return text.local_time_unavailable();
    };
    timestamp
        .to_zoned(time_zone.clone())
        .strftime("%Y-%m-%d %H:%M")
        .to_string()
}

fn format_relative_saved_time(
    timestamp: &str,
    now_unix: Option<u64>,
    text: &impl SaveMetadataText,
) -> String {
    let Ok(saved_unix) = parse_save_timestamp(timestamp) else {
        return text.invalid_timestamp();
    };
    let Some(now_unix) = now_unix else {
        return text.relative_time_unavailable();
    };
    if saved_unix > now_unix {
        let (value, unit) = relative_time_quantity(saved_unix - now_unix);
        return text.future(value, unit);
    }

    let elapsed = now_unix - saved_unix;
    if elapsed <= 4 {
        return text.just_now();
    }
    let (value, unit) = relative_time_quantity(elapsed);
    text.elapsed(value, unit)
}

fn relative_time_quantity(seconds: u64) -> (u64, RelativeTimeUnit) {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    const MONTH: u64 = 30 * DAY;
    const YEAR: u64 = 365 * DAY;

    match seconds {
        0..MINUTE => (seconds.max(1), RelativeTimeUnit::Second),
        MINUTE..HOUR => (seconds / MINUTE, RelativeTimeUnit::Minute),
        HOUR..DAY => (seconds / HOUR, RelativeTimeUnit::Hour),
        DAY..WEEK => (seconds / DAY, RelativeTimeUnit::Day),
        WEEK..MONTH => (seconds / WEEK, RelativeTimeUnit::Week),
        MONTH..YEAR => (seconds / MONTH, RelativeTimeUnit::Month),
        _ => (seconds / YEAR, RelativeTimeUnit::Year),
    }
}

/// Truncate `text` to the longest prefix that fits in `max_w` pixels
/// when rendered with `font`. Oversize text gets an ASCII ellipsis so
/// clipped metadata is visibly abbreviated instead of looking like a
/// broken string.
pub(crate) fn truncate_to_pixel_width(
    font: &crate::native_font::Font,
    text: &str,
    max_w: i32,
) -> String {
    if max_w <= 0 {
        return String::new();
    }
    if font.text_width(text) <= max_w {
        return text.to_string();
    }

    let ellipsis = "...";
    let ellipsis_w = font.text_width(ellipsis);
    if ellipsis_w > max_w {
        return String::new();
    }

    let budget = max_w - ellipsis_w;
    // `text` doesn't fit in full — scan prefix-by-prefix for the
    // longest one that does.  `char_indices()` yields byte offsets at
    // the *start* of each char, so `text[..idx]` is the prefix with
    // `idx` excluded.
    let mut fit_end = 0;
    for (idx, _) in text.char_indices() {
        if font.text_width(&text[..idx]) > budget {
            return format!("{}{}", &text[..fit_end], ellipsis);
        }
        fit_end = idx;
    }
    format!("{}{}", &text[..fit_end], ellipsis)
}

/// Resync the input-field widget to the current selection. In Save
/// mode, an existing-slot selection prefills the widget with that
/// slot's display text (so the user can edit in place and overwrite);
/// the New pseudo-row clears it. Load mode clears unconditionally —
/// the field isn't shown.
///
/// `set_text` leaves the widget in `SelectedEditable` (it only touches
/// the buffer + caret) so subsequent text input keeps flowing through.
pub(crate) fn sync_input_for_selection(
    input_widget: &mut WidgetInputField,
    selection: Option<ListRow>,
    mode: SaveLoadMode,
    visible: &[usize],
    save_manager: &SaveGameManager,
) {
    if mode != SaveLoadMode::Save {
        input_widget.set_text("");
        return;
    }
    match selection {
        Some(ListRow::Existing(v_idx)) => {
            let slot = visible[v_idx];
            let save = save_manager
                .get(slot)
                .expect("visible slot must resolve to a save");
            input_widget.set_text(&save.text);
        }
        _ => input_widget.set_text(""),
    }
    // Park the caret at the end so the user is typing after the
    // prefilled name, not in the middle of it.
    input_widget.caret_offset = input_widget.edit_text.chars().count();
}

/// Snapshot manager rows for the pure model's shared filtering policy.
pub(crate) fn picker_slots(save_manager: &SaveGameManager) -> Vec<PickerSlot> {
    (0..save_manager.count())
        .map(|i| {
            let save = save_manager
                .get(i)
                .expect("index from 0..count() must resolve");
            let name = save_manager
                .slot_name(i)
                .expect("save picker requires validated slot identities");
            let state = save_manager
                .slot_state(&name)
                .expect("save picker row must have lifecycle state");
            PickerSlot {
                name,
                manager_index: i,
                special: save.is_special(),
                hidden_from_load: save.is_continue()
                    || save.is_restart()
                    || state == crate::savegame::SlotState::Draft,
                autosave: save.is_autosave(),
                multiplayer_diagnostic: save.multiplayer_diagnostic,
            }
        })
        .collect()
}

/// Shared press-edge editing; text insertion is consumed once by the widget.
pub(crate) fn edit_save_name(field: &mut WidgetInputField, event: &GameEvent) -> bool {
    match event {
        GameEvent::KeyDown { keycode, .. } => match keycode {
            Keycode::Backspace => {
                field.backspace();
            }
            Keycode::Delete => {
                field.delete_char();
            }
            Keycode::Left => field.move_caret_left(),
            Keycode::Right => field.move_caret_right(),
            Keycode::Home => field.move_caret_home(),
            Keycode::End => field.move_caret_end(),
            _ => return false,
        },
        GameEvent::TextInput { .. } => {}
        _ => return false,
    }
    true
}

pub(crate) fn feed_save_name(
    field: &mut WidgetInputField,
    input: &WidgetInput<'_>,
    empty_keyboard: &UiKeyboard,
) -> Vec<crate::ui::UiEvent> {
    let field_input = WidgetInput {
        mouse_position: input.mouse_position,
        mouse_z: input.mouse_z,
        mouse_button: MouseButtons::empty(),
        keyboard: empty_keyboard,
        text_input: input.text_input,
        capture: None,
    };
    let events = field.process_input(&field_input);
    if field.base.state != UiState::SelectedEditable {
        field.enter_edit_mode();
    }
    events
}

pub(crate) fn begin_picker_delete(model: &mut PickerModel, name: SlotName) -> bool {
    match model.request_delete_named(name) {
        Ok(()) => true,
        Err(error) => {
            model.report_error(error);
            false
        }
    }
}

/// Both scheduling adapters resolve confirmation and publish its outcome here.
/// An error can follow index publication, so refresh even when deletion fails.
pub(crate) fn finish_picker_delete(
    model: &mut PickerModel,
    manager: &mut SaveGameManager,
    confirmed: bool,
) {
    model.refresh(picker_slots(manager));
    let error = match model.confirm_delete(confirmed) {
        Ok(Some(slot)) => manager
            .remove_by_filename(slot.as_str())
            .err()
            .map(|error| format!("{error:#}")),
        Ok(None) => None,
        Err(error) => Some(error),
    };
    manager.sort_by_time();
    model.finish_delete(picker_slots(manager), error);
    if let Some(error) = model.operation_error() {
        tracing::error!("Delete save failed (cleanup may be pending): {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_leaves_committed_ime_text_and_caret_editing_to_save_adapter() {
        let mut model = PickerModel::new(SaveLoadMode::Save, false, 2, vec![]);
        let mut controller = PickerController::new(ModalInputState::new());
        controller.scroll_view = Some(crate::scroll_view::ScrollView::with_geometry(
            [LOAD_LIST_RECT.x, LOAD_LIST_RECT.y + 4, LOAD_LIST_RECT.w, 40],
            20,
            16,
            16,
            false,
        ));
        let buttons = [
            (ID_LOAD_SAVE, "Save", 460, 300),
            (ID_DELETE, "Delete", 460, 350),
            (ID_CANCEL, "Cancel", 460, 400),
        ];
        let transform = MenuTransform::centered(640, 480);
        let mut field = WidgetInputField::new(1000);
        field.set_max_length(MAX_NAME_LEN);
        field.enter_edit_mode();
        let keyboard = UiKeyboard::default();
        for text in ["é雪", "Ω"] {
            controller.begin_frame(&model, &buttons, 150, 40);
            if text == "Ω" {
                let left = GameEvent::KeyDown {
                    keycode: Keycode::Left,
                    physical_key: None,
                };
                assert!(!controller.handle_event(&mut model, &left, transform));
                field.move_caret_left(); // Save-only adapter remains its owner.
            }
            assert!(!controller.handle_event(
                &mut model,
                &GameEvent::TextInput { text: text.into() },
                transform
            ));
            controller.process_widgets(&model);
            feed_save_name(&mut field, &controller.input.as_widget_input(), &keyboard);
            controller.input.end_frame();
            assert_eq!(controller.take_action(), None);
        }
        assert_eq!(field.edit_text, "éΩ雪");
        assert_eq!(field.caret_offset, 2);
        assert_eq!(field.base.state, UiState::SelectedEditable);
        assert_eq!(model.selected_row(), Some(ListRow::New));
    }

    #[test]
    fn shared_input_trace_is_independent_of_cooperative_frame_boundaries() {
        let mut manager = SaveGameManager::new("unused-picker-model-store".into());
        for index in 0..5 {
            manager.insert_test_slot(
                published_metadata(&format!("Savegame_{index:03}")),
                crate::savegame::SlotState::Published,
            );
        }
        let mut cooperative =
            PickerModel::new(SaveLoadMode::Load, false, 2, picker_slots(&manager));
        let mut standalone = cooperative.clone();
        let mut cooperative_input = PickerController::new(ModalInputState::new());
        cooperative_input.scroll_view = Some(crate::scroll_view::ScrollView::with_geometry(
            [LOAD_LIST_RECT.x, LOAD_LIST_RECT.y + 4, LOAD_LIST_RECT.w, 40],
            20,
            16,
            16,
            false,
        ));
        let mut standalone_input = PickerController::new(ModalInputState::new());
        standalone_input.scroll_view = Some(crate::scroll_view::ScrollView::with_geometry(
            [LOAD_LIST_RECT.x, LOAD_LIST_RECT.y + 4, LOAD_LIST_RECT.w, 40],
            20,
            16,
            16,
            false,
        ));
        let buttons = [
            (ID_LOAD_SAVE, "Load", 460, 300),
            (ID_DELETE, "Delete", 460, 350),
            (ID_CANCEL, "Cancel", 460, 400),
        ];
        let transform = MenuTransform::centered(640, 480);
        let down = GameEvent::KeyDown {
            keycode: Keycode::Down,
            physical_key: None,
        };
        let up = GameEvent::KeyDown {
            keycode: Keycode::Up,
            physical_key: None,
        };
        cooperative_input.input.virt_x = (LOAD_LIST_RECT.x + 10) as f32;
        cooperative_input.input.virt_y = (LOAD_LIST_RECT.y + 10) as f32;
        standalone_input.input.virt_x = cooperative_input.input.virt_x;
        standalone_input.input.virt_y = cooperative_input.input.virt_y;
        let events = [
            down.clone(),
            down.clone(),
            GameEvent::MouseWheel(-1),
            down.clone(),
            down,
            up,
            GameEvent::MouseWheel(1),
        ];
        // The actual adapters share this input bridge, but the cooperative
        // driver may return to the host between every event. Refresh must not
        // turn its stable selection into an old presentation offset.
        for event in &events {
            cooperative.refresh(picker_slots(&manager));
            cooperative_input.begin_frame(&cooperative, &buttons, 150, 40);
            cooperative_input.handle_event(&mut cooperative, event, transform);
            cooperative_input.process_widgets(&cooperative);
            cooperative_input.input.end_frame();
        }
        standalone_input.begin_frame(&standalone, &buttons, 150, 40);
        for event in &events {
            standalone_input.handle_event(&mut standalone, event, transform);
        }
        standalone_input.process_widgets(&standalone);
        standalone_input.input.end_frame();
        assert_eq!(cooperative, standalone);
        assert_eq!(cooperative.selected_row(), Some(ListRow::Existing(2)));
        assert_eq!(cooperative.scroll_offset(), 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn shared_delete_bridge_handles_cancel_success_and_cleanup_error() {
        let directory = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(directory.path().to_string_lossy().into_owned());
        for index in 0..2 {
            let save = published_metadata(&format!("Savegame_{index:03}"));
            manager.insert_test_slot(save, crate::savegame::SlotState::Published);
        }
        manager.save_index().unwrap();
        manager = SaveGameManager::load_index(directory.path().to_str().unwrap()).unwrap();
        assert_eq!(
            manager.count(),
            2,
            "fixture index must be loadable, not unpublished drafts"
        );
        let mut model = PickerModel::new(SaveLoadMode::Load, false, 1, picker_slots(&manager));
        model.navigate(true);
        model.request_delete().unwrap();
        finish_picker_delete(&mut model, &mut manager, false);
        assert_eq!(manager.count(), 2);
        assert_eq!(model.selected_row(), Some(ListRow::Existing(0)));
        model.request_delete().unwrap();
        finish_picker_delete(&mut model, &mut manager, true);
        assert_eq!(manager.count(), 1);
        assert_eq!(model.selected_row(), None);
        assert_eq!(model.operation_error(), None);
        manager = SaveGameManager::load_index(directory.path().to_str().unwrap()).unwrap();
        assert_eq!(manager.count(), 1);
        assert_eq!(manager.slot_name(0).unwrap().as_str(), "Savegame_001");

        // A directory cannot be unlinked as a payload file. The durable intent
        // has already removed the row, so both adapters must show the new list
        // and preserve the reported error rather than restoring a ghost row.
        std::fs::create_dir(directory.path().join("Savegame_001.json")).unwrap();
        model.navigate(true);
        model.request_delete().unwrap();
        finish_picker_delete(&mut model, &mut manager, true);
        assert_eq!(manager.count(), 0);
        assert_eq!(model.selected_row(), None);
        assert_eq!(model.scroll_offset(), 0);
        assert!(model.operation_error().unwrap().contains("cleanup"));
        assert!(SaveGameManager::load_index(directory.path().to_str().unwrap()).is_err());
        std::fs::remove_dir(directory.path().join("Savegame_001.json")).unwrap();
        let recovered = SaveGameManager::load_index(directory.path().to_str().unwrap()).unwrap();
        assert_eq!(recovered.count(), 0);
    }

    fn saved_at(timestamp: &str) -> SaveGame {
        let mut save = SaveGame::new("Savegame_000".into(), "My save".into(), 7);
        save.timestamp = timestamp.to_string();
        save.mission_name = "The Silver Arrow".into();
        save.player_profile_id = Some(12);
        save.player_name = "Alice".into();
        save
    }

    #[test]
    fn save_info_shows_mission_minutes_without_wrapping_at_an_hour() {
        let mut save = saved_at("123");
        for (seconds, expected) in [(0, "00:00"), (65, "01:05"), (3903, "65:03")] {
            save.mission_elapsed_seconds = Some(seconds);
            for detailed in [false, true] {
                assert!(
                    cooperative_save_row_detail_lines(&save, detailed, Some(123), None)[0]
                        .contains(expected)
                );
            }
            assert!(
                selected_metadata_lines(&save, Some(123), None, &EnglishSaveMetadataText)[0]
                    .contains(expected)
            );
        }
        save.mission_elapsed_seconds = None;
        assert_eq!(
            mission_with_time(&save, &EnglishSaveMetadataText),
            "The Silver Arrow"
        );
    }

    fn published_metadata(filename: &str) -> SaveGame {
        let mut save = SaveGame::new(filename.into(), "The Silver Arrow".into(), 7);
        save.timestamp = "123".into();
        save.mission_name = "The Silver Arrow".into();
        save.player_profile_id = Some(12);
        save.player_name = "Alice".into();
        save.campaign_progress = Some(0);
        save.missions_done = Some(0);
        save.missions_total = Some(1);
        save.gang_size = Some(1);
        save.ransom = Some(0);
        save.blazons = Some(0);
        save.amulets = Some(0);
        save.validate_published_metadata().unwrap();
        save
    }

    #[test]
    fn failed_save_drafts_can_be_retried_but_are_not_loadable() {
        let mut manager = SaveGameManager::new("unused-picker-draft-store".into());
        manager.insert_test_slot(
            SaveGame::new("Savegame_000".into(), "Unpublished".into(), 7),
            crate::savegame::SlotState::Draft,
        );
        let load = PickerModel::new(SaveLoadMode::Load, false, 2, picker_slots(&manager));
        assert!(load.visible().is_empty());
        let mut save = PickerModel::new(SaveLoadMode::Save, false, 2, picker_slots(&manager));
        assert_eq!(save.visible(), vec![0]);
        save.report_error("payload publication failed".into());
        save.refresh(picker_slots(&manager));
        assert_eq!(save.operation_error(), Some("payload publication failed"));
        save.dismiss_error();
        assert_eq!(save.operation_error(), None);
    }

    #[test]
    fn autosaves_are_loadable_but_not_overwritable_or_deletable() {
        let mut manager = SaveGameManager::new("/tmp/test_saves".into());
        manager.insert_test_slot(
            published_metadata("Autosave_100_0000"),
            crate::savegame::SlotState::Published,
        );

        let mut load_model = PickerModel::new(SaveLoadMode::Load, false, 3, picker_slots(&manager));
        let load_visible = load_model.visible();
        assert_eq!(load_visible, vec![0]);
        load_model.select(Some(ListRow::Existing(0)));
        assert!(!load_model.can_delete());
        assert!(
            PickerModel::new(SaveLoadMode::Save, false, 3, picker_slots(&manager))
                .visible()
                .is_empty()
        );
        assert_eq!(
            row_label(
                ListRow::Existing(0),
                &manager,
                &load_visible,
                &EnglishSaveMetadataText,
            ),
            "Autosave - The Silver Arrow"
        );
    }

    #[test]
    fn relative_time_covers_thresholds_and_future_clock_changes() {
        let text = EnglishSaveMetadataText;
        let now = 2_000_000;
        let cases = [
            (now, "just now"),
            (now - 4, "just now"),
            (now - 5, "5 seconds ago"),
            (now - 60, "1 minute ago"),
            (now - 3_600, "1 hour ago"),
            (now - 86_400, "1 day ago"),
            (now - 604_800, "1 week ago"),
        ];
        for (saved, expected) in cases {
            assert_eq!(
                format_relative_saved_time(&saved.to_string(), Some(now), &text),
                expected
            );
        }
        assert_eq!(
            format_relative_saved_time(&(now + 7_200).to_string(), Some(now), &text),
            "in 2 hours"
        );
    }

    #[test]
    fn invalid_and_unavailable_clocks_are_reported_honestly() {
        let text = EnglishSaveMetadataText;
        assert_eq!(
            format_relative_saved_time("not-a-clock", Some(10), &text),
            "invalid timestamp"
        );
        assert_eq!(
            format_relative_saved_time("10", None, &text),
            "relative time unavailable"
        );
        assert_eq!(
            format_exact_saved_time("not-a-clock", Some(&TimeZone::UTC), &text),
            "invalid timestamp"
        );
        assert_eq!(
            format_exact_saved_time("10", None, &text),
            "local time unavailable"
        );
    }

    #[test]
    fn exact_time_uses_the_requested_zone() {
        let text = EnglishSaveMetadataText;
        assert_eq!(
            format_exact_saved_time("0", Some(&TimeZone::UTC), &text),
            "1970-01-01 00:00:00 UTC"
        );
    }

    #[test]
    fn every_existing_row_leads_with_required_metadata() {
        let text = EnglishSaveMetadataText;
        let mut manager = SaveGameManager::new("/tmp/test_saves".into());
        manager.insert_test_slot(saved_at("100"), crate::savegame::SlotState::Draft);
        let lines = row_detail_lines(
            ListRow::Existing(0),
            &manager,
            &[0],
            Some(3_700),
            Some(&TimeZone::UTC),
            &text,
            true,
        );
        assert_eq!(lines[0], "Mission: The Silver Arrow | Player: Alice");
        assert!(lines[1].starts_with("Saved: 1 hour ago | Date: "));
    }

    #[test]
    fn incomplete_original_import_row_does_not_invent_player_or_mission() {
        let text = EnglishSaveMetadataText;
        let mut manager = SaveGameManager::new("/tmp/test_saves".into());
        manager.insert_test_slot(saved_at("100"), crate::savegame::SlotState::Draft);
        manager.get_mut(0).unwrap().mission_name.clear();
        manager.get_mut(0).unwrap().player_name.clear();
        let lines = row_detail_lines(
            ListRow::Existing(0),
            &manager,
            &[0],
            Some(100),
            Some(&TimeZone::UTC),
            &text,
            true,
        );
        assert!(lines[0].contains("Mission: unavailable (legacy save)"));
        assert!(lines[0].contains("Player: unavailable (legacy save)"));
    }

    #[test]
    fn compact_mode_hides_expanded_provenance_without_discarding_it() {
        let text = EnglishSaveMetadataText;
        let mut manager = SaveGameManager::new("/tmp/test_saves".into());
        let mut save = saved_at("3600");
        save.campaign_progress = Some(25);
        manager.insert_test_slot(save, crate::savegame::SlotState::Draft);

        let lines = row_detail_lines(
            ListRow::Existing(0),
            &manager,
            &[0],
            None,
            Some(&TimeZone::UTC),
            &text,
            false,
        );

        assert_eq!(
            lines,
            [
                "Saved 1970-01-01 01:00 | The Silver Arrow | 25% campaign".to_string(),
                String::new(),
            ]
        );
        assert!(!lines[0].contains("Player:"));
        assert!(!lines[0].contains("ago"));
    }
}

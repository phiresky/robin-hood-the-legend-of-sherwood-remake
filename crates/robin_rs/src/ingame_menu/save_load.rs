//! Load-slot modal and shared save-picker presentation helpers.
//!
//! The main menu, terminal debriefing and the in-mission pause task all drive
//! one [`SavePickerModalState`]; only frame pacing differs between them.

use crate::gfx_types::Keycode;
use robin_engine::coordinates as engine_coordinates;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::savegame::{SaveGame, SaveGameManager, SlotName};
use crate::ui::{MouseButtons, UiKeyboard, UiState};
use crate::widget::{WidgetInput, WidgetInputField, WidgetPicture};
use jiff::{Timestamp, tz::TimeZone};
use robin_engine::profiles::ProfileManager;
use std::borrow::Cow;

use super::layout::{
    MenuRect, MenuTransform, TruncationMarker, align_bottom_right, draw_screen_background,
    render_text_virt_font, truncate_to_pixel_width,
};
use super::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_DELETE, MT_BTN_LOAD, MT_BTN_SAVE,
    MT_MSG_REALLY_DELETE_SAVEGAME, MT_MSG_REALLY_OVERWRITE_SAVEGAME,
};
pub(crate) use super::save_picker::{
    ID_CANCEL, ID_DELETE, ID_LOAD_SAVE, ListRow, PickerAction, PickerController, PickerModel,
    PickerSlot, PickerTarget, retire_thumbnail,
};
use super::widget_bridge::{self, ModalInputState, ModalScreenIo, ScreenAudio, ScreenFrame};
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

/// What one picker is opened for, fixed for its lifetime.
#[derive(Debug, Clone, Copy)]
pub struct SavePickerConfig {
    pub mode: SaveLoadMode,
    /// Mission recorded on new drafts and used for default save names;
    /// required by Save pickers, unused by Load pickers.
    pub mission_id: Option<u32>,
    pub detailed_metadata: bool,
    pub multiplayer_connected: bool,
    /// Upload and show the selected save's thumbnail. An owner that enables
    /// this must call [`SavePickerModalState::close`] with the same renderer;
    /// cooperative tasks that can be cancelled without one leave it off.
    pub previews: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum SaveConfirmation {
    Overwrite(SlotName),
    Delete,
}

/// One-frame save/load slot picker.
///
/// The main menu drives this state with a small async loop; mission-time
/// pickers tick it cooperatively so networking and automation continue
/// between frames.
pub struct SavePickerModalState {
    config: SavePickerConfig,
    model: PickerModel,
    thumb_widget: WidgetPicture,
    thumb_cache: Option<ThumbnailCache>,
    controller: PickerController,
    name: WidgetInputField,
    noise_tracker: widget_bridge::NoisyTracker,
    confirmation: Option<(SaveConfirmation, YesNoModalState)>,
    error_notice: Option<crate::save_recovery::ErrorNotice>,
    text_input_active: bool,
    local_time_zone: Option<TimeZone>,
    clock_error_reported: bool,
}

impl SavePickerModalState {
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        save_manager: &mut SaveGameManager,
        config: SavePickerConfig,
    ) -> Self {
        save_manager.sort_by_time();
        let input_state = ModalInputState::for_screen(event_pump, renderer);
        let mut state = Self::with_input(save_manager, config, input_state);
        state.text_input_active = config.mode == SaveLoadMode::Save;
        if state.text_input_active {
            crate::window::start_text_input();
        }
        state
    }

    fn with_input(
        save_manager: &SaveGameManager,
        config: SavePickerConfig,
        input_state: ModalInputState,
    ) -> Self {
        let mut name = WidgetInputField::new(1000);
        name.set_max_length(MAX_NAME_LEN);
        if config.mode == SaveLoadMode::Save {
            name.enter_edit_mode();
        }
        Self {
            config,
            model: PickerModel::new(
                config.mode,
                config.multiplayer_connected,
                (list_rect(config.mode).h / row_height(config.detailed_metadata)).max(1) as usize,
                picker_slots(save_manager),
            ),
            thumb_widget: WidgetPicture::new(u32::MAX),
            thumb_cache: None,
            controller: PickerController::new(input_state),
            name,
            noise_tracker: widget_bridge::NoisyTracker::new(),
            confirmation: None,
            error_notice: None,
            // Only the window-backed constructor acquires process IME state.
            text_input_active: false,
            local_time_zone: TimeZone::try_system()
                .inspect_err(|error| tracing::warn!("Save menu local time is unavailable: {error}"))
                .ok(),
            clock_error_reported: false,
        }
    }

    fn refresh(&mut self, manager: &SaveGameManager) {
        let selected = self.model.selected_slot().cloned();
        self.model.refresh(picker_slots(manager));
        if selected.as_ref() != self.model.selected_slot() {
            self.sync_name(manager);
        }
    }

    fn sync_name(&mut self, manager: &SaveGameManager) {
        sync_input_for_selection(
            &mut self.name,
            self.model.selected_manager_index(),
            self.config.mode,
            manager,
        );
    }

    fn handle_event(
        &mut self,
        event: &GameEvent,
        transform: MenuTransform,
        manager: &SaveGameManager,
    ) {
        if self
            .controller
            .handle_event(&mut self.model, event, transform)
        {
            self.sync_name(manager);
        }
        if self.config.mode == SaveLoadMode::Save {
            edit_save_name(&mut self.name, event);
        }
    }

    /// Deliver the frame's input to the widgets; returns their events and
    /// the virtual mouse position used for row hover.
    fn finish_input(&mut self) -> (Vec<crate::ui::UiEvent>, engine_coordinates::ScreenPoint) {
        let events = self.controller.process_widgets(&self.model);
        let mouse = self.controller.input.as_widget_input().mouse_position;
        if self.config.mode == SaveLoadMode::Save {
            feed_save_name(
                &mut self.name,
                &self.controller.input.as_widget_input(),
                &UiKeyboard::default(),
            );
        }
        self.controller.input.end_frame();
        (events, mouse)
    }

    fn sync_thumbnail(&mut self, save_manager: &SaveGameManager, renderer: &mut Renderer) {
        if !self.config.previews {
            return;
        }
        sync_thumbnail_cache(
            &mut self.thumb_cache,
            &mut self.thumb_widget,
            self.model.selected_manager_index(),
            save_manager,
            renderer,
            self.config.mode,
        );
    }

    /// A window close request reports `Cancel`; owners that distinguish it
    /// read `io.window.close_requested`.
    pub fn tick(
        &mut self,
        io: &mut ModalScreenIo<'_, '_>,
        save_manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
        audio: ScreenAudio<'_>,
    ) -> Option<SaveLoadOutcome> {
        self.refresh(save_manager);
        if self.error_notice.is_none()
            && let Some(error) = self.model.operation_error()
        {
            self.error_notice = Some(crate::save_recovery::ErrorNotice::new(error.to_string()));
        }
        if let Some(notice) = &mut self.error_notice {
            if notice.tick(io) {
                self.error_notice = None;
                self.model.dismiss_error();
                if io.window.close_requested {
                    return Some(SaveLoadOutcome::Cancel);
                }
            }
            // This frame belongs exclusively to the notice. The outer mission
            // driver can still service networking and automation between ticks.
            return None;
        }
        if let Some((_, confirmation)) = self.confirmation.as_mut() {
            let confirmed = confirmation.tick(io)?;
            let (action, _) = self
                .confirmation
                .take()
                .expect("resolved confirmation exists");
            let outcome = self.finish_confirmation(action, confirmed, save_manager, profiles);
            self.sync_thumbnail(save_manager, io.renderer);
            return outcome;
        }

        let visible = self.model.visible();

        // The picker controller updates its own input per event (list drag and
        // wheel hit-testing read it), so events are polled without `begin`.
        let screen = ScreenFrame::poll(io);
        let transform = screen.transform;
        let resources = io.resources;
        let mode = self.config.mode;
        let detailed_metadata = self.config.detailed_metadata;
        let (btn_w, btn_h) = resources.button_dimensions();
        let accept_label = resources.menu_text.get(match mode {
            SaveLoadMode::Save => MT_BTN_SAVE,
            SaveLoadMode::Load => MT_BTN_LOAD,
        });
        let delete_label = resources.menu_text.get(MT_BTN_DELETE);
        let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);
        let bottom_buttons = align_bottom_right(
            &[
                (&accept_label, false),
                (&delete_label, false),
                (&cancel_label, true),
            ],
            btn_w,
            btn_h,
        );
        let btn_positions = [
            (
                ID_LOAD_SAVE,
                accept_label.as_str(),
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
        self.controller.configure_list(
            &mut self.model,
            list_rect(mode),
            row_height(detailed_metadata),
            resources,
        );
        self.controller
            .begin_frame(&self.model, &btn_positions, btn_w, btn_h);
        for event in &screen.events {
            self.handle_event(event, transform, save_manager);
        }
        let selected = self.model.selected_row();
        let (widget_events, mouse_virt) = self.finish_input();
        widget_bridge::play_frame_widget_noise(
            &widget_events,
            self.controller.frame(),
            widget_bridge::WIDGET_NOISY_BUTTON,
            audio,
            &mut self.noise_tracker,
        );
        match self.controller.take_action() {
            Some(PickerAction::Cancel) => return Some(SaveLoadOutcome::Cancel),
            Some(PickerAction::Accept(PickerTarget::Existing(name)))
                if mode == SaveLoadMode::Save =>
            {
                let message = resources.menu_text.get(MT_MSG_REALLY_OVERWRITE_SAVEGAME);
                self.confirmation = Some((
                    SaveConfirmation::Overwrite(name),
                    YesNoModalState::new(io.window, io.renderer, resources, message),
                ));
            }
            Some(PickerAction::Accept(target)) => {
                if let Some(outcome) = self.accept_target(target, save_manager, profiles) {
                    return Some(outcome);
                }
            }
            Some(PickerAction::ConfirmDelete(name)) => {
                if begin_picker_delete(&mut self.model, name) {
                    let message = resources.menu_text.get(MT_MSG_REALLY_DELETE_SAVEGAME);
                    self.confirmation = Some((
                        SaveConfirmation::Delete,
                        YesNoModalState::new(io.window, io.renderer, resources, message),
                    ));
                }
            }
            None => {}
        }

        self.sync_thumbnail(save_manager, io.renderer);
        let renderer = &mut *io.renderer;
        screen.begin_draw(renderer);
        if let Some(background) = resources.menu_bg[3] {
            draw_screen_background(renderer, &background);
        }
        let metadata_text = SaveMetadataText::new(resources.menu_text.presentation_locale());
        let now_unix = if detailed_metadata {
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
        let row_area_x = list_rect(mode).x + 10;
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
            let raw_label = row_label(row, save_manager, &visible, &metadata_text);
            let label = truncate_to_pixel_width(
                font,
                &raw_label,
                row_area_w,
                TruncationMarker::AsciiEllipsis,
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
                detailed_metadata,
            )
            .iter()
            .filter(|line| !line.is_empty())
            .enumerate()
            {
                let detail = truncate_to_pixel_width(
                    font,
                    detail,
                    row_area_w,
                    TruncationMarker::AsciiEllipsis,
                );
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
        if mode == SaveLoadMode::Save
            && let Some(font) = resources.label_font_any()
        {
            let (before, after): (String, String) = (
                self.name
                    .edit_text
                    .chars()
                    .take(self.name.caret_offset)
                    .collect(),
                self.name
                    .edit_text
                    .chars()
                    .skip(self.name.caret_offset)
                    .collect(),
            );
            render_text_virt_font(
                renderer,
                font,
                transform,
                &format!("Name: {before}|{after}"),
                SAVE_NAME_POS.0,
                SAVE_NAME_POS.1,
            );
        }
        SavePreview {
            selected,
            visible: &visible,
            thumb_cache: self.thumb_cache.as_ref(),
            thumb_widget: &self.thumb_widget,
            save_manager,
            resources,
            now_unix,
            local_time_zone: self.local_time_zone.as_ref(),
            text: &metadata_text,
            detailed_metadata,
        }
        .draw(renderer, transform);
        widget_bridge::draw_frame_buttons(renderer, resources, transform, self.controller.frame());
        screen.finish(io, &self.controller.input);
        None
    }

    fn finish_confirmation(
        &mut self,
        action: SaveConfirmation,
        confirmed: bool,
        manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
    ) -> Option<SaveLoadOutcome> {
        match action {
            SaveConfirmation::Delete => {
                let selected = self.model.selected_slot().cloned();
                finish_picker_delete(&mut self.model, manager, confirmed);
                if selected.as_ref() != self.model.selected_slot() {
                    self.sync_name(manager);
                }
                None
            }
            SaveConfirmation::Overwrite(name) if confirmed => {
                self.accept_target(PickerTarget::Existing(name), manager, profiles)
            }
            SaveConfirmation::Overwrite(_) => None,
        }
    }

    fn accept_target(
        &mut self,
        target: PickerTarget,
        manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
    ) -> Option<SaveLoadOutcome> {
        let mode = self.config.mode;
        let mission_id = self.config.mission_id;
        let mission_id = || mission_id.expect("Save picker requires a mission id");
        let result = (|| -> Result<usize, String> {
            match target {
                PickerTarget::New => {
                    if mode != SaveLoadMode::Save {
                        return Err("load picker cannot create a save".into());
                    }
                    let text = if self.name.edit_text.trim().is_empty() {
                        mission_name(mission_id(), profiles)
                            .unwrap_or_else(|| format!("Save {}", manager.count() + 1))
                    } else {
                        self.name.edit_text.trim().to_owned()
                    };
                    let handle = manager
                        .create_draft(text, mission_id())
                        .map_err(|error| format!("{error:#}"))?;
                    Ok(manager
                        .find_by_filename(handle.name().as_str())
                        .expect("new save draft is catalogued"))
                }
                PickerTarget::Existing(name) => {
                    self.refresh(manager);
                    let slot = self
                        .model
                        .visible_slot_index(&name)
                        .ok_or_else(|| "the selected save is no longer available".to_string())?;
                    if mode == SaveLoadMode::Save {
                        let text = accepted_name(
                            &self.name.edit_text,
                            manager,
                            slot,
                            mission_id(),
                            profiles,
                        );
                        let handle = manager
                            .slot_handle(slot)
                            .expect("validated overwrite slot exists");
                        manager
                            .rename_slot(&handle, text)
                            .map_err(|error| format!("{error:#}"))?;
                    }
                    Ok(slot)
                }
            }
        })();
        match result {
            Ok(slot) => Some(SaveLoadOutcome::Slot(slot)),
            Err(error) => {
                self.model.report_error(error);
                None
            }
        }
    }

    /// Release the preview surface. Required before drop when
    /// [`SavePickerConfig::previews`] is set.
    pub fn close(&mut self, renderer: &mut Renderer) {
        clear_thumbnail_cache(&mut self.thumb_cache, &mut self.thumb_widget, renderer);
    }

    /// Release process IME state acquired by a Save picker.
    pub fn stop_text_input(&mut self) {
        if self.text_input_active {
            crate::window::stop_text_input();
            self.text_input_active = false;
        }
    }
}

impl Drop for SavePickerModalState {
    fn drop(&mut self) {
        self.stop_text_input();
    }
}

fn accepted_name(
    input: &str,
    save_manager: &SaveGameManager,
    slot: usize,
    mission_id: u32,
    profiles: Option<&ProfileManager>,
) -> String {
    let trimmed = input.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    let existing = save_manager.get(slot).expect("accepted save slot exists");
    if !existing.text.trim().is_empty() {
        return existing.text.clone();
    }
    mission_name(mission_id, profiles)
        .unwrap_or_else(|| format!("Save {}", save_manager.count() + 1))
}

fn mission_name(mission_id: u32, profiles: Option<&ProfileManager>) -> Option<String> {
    profiles?
        .missions
        .iter()
        .find(|mission| mission.id == mission_id)
        .map(|mission| mission.mission_name.clone())
        .filter(|name| !name.trim().is_empty())
}

/// The Save picker gives up the list's bottom rows to the name line.
fn list_rect(mode: SaveLoadMode) -> MenuRect {
    match mode {
        SaveLoadMode::Load => LOAD_LIST_RECT,
        SaveLoadMode::Save => MenuRect {
            h: SAVE_NAME_POS.1 - 8 - LOAD_LIST_RECT.y,
            ..LOAD_LIST_RECT
        },
    }
}

fn row_height(detailed_metadata: bool) -> i32 {
    if detailed_metadata {
        DETAILED_ROW_HEIGHT
    } else {
        COMPACT_ROW_HEIGHT
    }
}

const SAVE_NAME_POS: (i32, i32) = (34, 438);
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

use crate::localization::PortTextKey;
pub(crate) use crate::localization::RelativeTimeUnit;

/// Pure save-copy formatting, borrowing the menu's prepared presentation locale.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct SaveMetadataText<'a> {
    #[serde(borrow)]
    locale: Option<&'a str>,
}

impl<'a> SaveMetadataText<'a> {
    fn new(locale: Option<&'a str>) -> Self {
        Self { locale }
    }

    fn format(&self, key: PortTextKey, arguments: &[(&str, &str)]) -> String {
        crate::localization::format_port_text(self.locale, key, arguments)
            .unwrap_or_else(|error| panic!("invalid save metadata catalog entry {key:?}: {error}"))
    }

    fn new_save_label(&self) -> String {
        crate::localization::port_text(self.locale, PortTextKey::SaveNewSaveLabel).to_owned()
    }

    fn new_save_hint(&self) -> String {
        crate::localization::port_text(self.locale, PortTextKey::SaveNewSaveHint).to_owned()
    }

    fn mission(&self, value: &str) -> String {
        self.format(PortTextKey::SaveMission, &[("value", value)])
    }

    fn player(&self, value: &str) -> String {
        self.format(PortTextKey::SavePlayer, &[("value", value)])
    }

    fn saved(&self, value: &str) -> String {
        self.format(PortTextKey::SaveSaved, &[("value", value)])
    }

    fn exact_date(&self, value: &str) -> String {
        self.format(PortTextKey::SaveExactDate, &[("value", value)])
    }

    fn campaign_progress(&self, progress: u32) -> String {
        self.format(
            PortTextKey::SaveCampaignProgress,
            &[("progress", &progress.to_string())],
        )
    }

    fn missions(&self, done: usize, total: usize) -> String {
        self.format(
            PortTextKey::SaveMissions,
            &[("done", &done.to_string()), ("total", &total.to_string())],
        )
    }

    fn gang_size(&self, size: usize) -> String {
        self.format(PortTextKey::SaveGangSize, &[("size", &size.to_string())])
    }

    fn ransom(&self, value: i32) -> String {
        self.format(PortTextKey::SaveRansom, &[("value", &value.to_string())])
    }

    fn blazons(&self, value: i32) -> String {
        self.format(PortTextKey::SaveBlazons, &[("value", &value.to_string())])
    }

    fn amulets(&self, value: i32) -> String {
        self.format(PortTextKey::SaveAmulets, &[("value", &value.to_string())])
    }

    fn legacy_value_unavailable(&self) -> String {
        crate::localization::port_text(self.locale, PortTextKey::SaveLegacyValueUnavailable)
            .to_owned()
    }

    fn invalid_timestamp(&self) -> String {
        crate::localization::port_text(self.locale, PortTextKey::SaveInvalidTimestamp).to_owned()
    }

    fn relative_time_unavailable(&self) -> String {
        crate::localization::port_text(self.locale, PortTextKey::SaveRelativeTimeUnavailable)
            .to_owned()
    }

    fn local_time_unavailable(&self) -> String {
        crate::localization::port_text(self.locale, PortTextKey::SaveLocalTimeUnavailable)
            .to_owned()
    }

    fn just_now(&self) -> String {
        crate::localization::port_text(self.locale, PortTextKey::SaveJustNow).to_owned()
    }

    fn compact_saved(&self, value: &str) -> String {
        self.format(PortTextKey::SaveCompactSaved, &[("value", value)])
    }

    fn compact_campaign_progress(&self, progress: u32) -> String {
        self.format(
            PortTextKey::SaveCompactCampaignProgress,
            &[("progress", &progress.to_string())],
        )
    }

    fn compact_missions(&self, done: usize, total: usize) -> String {
        self.format(
            PortTextKey::SaveCompactMissions,
            &[("done", &done.to_string()), ("total", &total.to_string())],
        )
    }

    fn compact_gang_size(&self, size: usize) -> String {
        self.format(
            PortTextKey::SaveCompactGangSize,
            &[("size", &size.to_string())],
        )
    }

    fn compact_ransom(&self, value: i32) -> String {
        self.format(
            PortTextKey::SaveCompactRansom,
            &[("value", &value.to_string())],
        )
    }

    fn compact_blazons(&self, value: i32) -> String {
        self.format(
            PortTextKey::SaveCompactBlazons,
            &[("value", &value.to_string())],
        )
    }

    fn compact_amulets(&self, value: i32) -> String {
        self.format(
            PortTextKey::SaveCompactAmulets,
            &[("value", &value.to_string())],
        )
    }

    fn elapsed(&self, value: u64, unit: RelativeTimeUnit) -> String {
        self.format(
            PortTextKey::SaveRelativeTime {
                unit,
                future: false,
                singular: value == 1,
            },
            &[("value", &value.to_string())],
        )
    }

    fn future(&self, value: u64, unit: RelativeTimeUnit) -> String {
        self.format(
            PortTextKey::SaveRelativeTime {
                unit,
                future: true,
                singular: value == 1,
            },
            &[("value", &value.to_string())],
        )
    }
}

/// Longest allowed save name — passed to the input field as its
/// max-length cap.
const MAX_NAME_LEN: usize = 45;

/// Main-menu load picker: the shared picker state paced by [`widget_bridge::run_modal`].
pub async fn show_load_picker(
    io: &mut ModalScreenIo<'_, '_>,
    save_manager: &mut SaveGameManager,
    detailed_metadata: bool,
) -> SaveLoadOutcome {
    let mut state = SavePickerModalState::new(
        io.window,
        io.renderer,
        save_manager,
        SavePickerConfig {
            mode: SaveLoadMode::Load,
            mission_id: None,
            detailed_metadata,
            multiplayer_connected: false,
            previews: true,
        },
    );
    let outcome = widget_bridge::run_modal(io, |io| {
        let no_audio = ScreenAudio {
            sound: None,
            backend: None,
            sample_loader: None,
        };
        state.tick(io, save_manager, None, no_audio)
    })
    .await;
    state.close(io.renderer);
    outcome
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
    selected_manager_index: Option<usize>,
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
    let target_slot = match mode {
        SaveLoadMode::Load => selected_manager_index,
        SaveLoadMode::Save => None,
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

/// Everything the load picker's right-hand preview column reads for one frame.
struct SavePreview<'a> {
    selected: Option<ListRow>,
    visible: &'a [usize],
    thumb_cache: Option<&'a ThumbnailCache>,
    thumb_widget: &'a crate::widget::WidgetPicture,
    save_manager: &'a SaveGameManager,
    resources: &'a IngameMenuResources,
    now_unix: Option<u64>,
    local_time_zone: Option<&'a TimeZone>,
    text: &'a SaveMetadataText<'a>,
    detailed_metadata: bool,
}

impl SavePreview<'_> {
    fn draw(&self, renderer: &mut Renderer, transform: MenuTransform) {
        let SavePreview {
            selected,
            visible,
            thumb_cache,
            thumb_widget,
            save_manager,
            resources,
            now_unix,
            local_time_zone,
            text,
            detailed_metadata,
        } = *self;
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
            let fitted =
                truncate_to_pixel_width(font, line, panel_w, TruncationMarker::AsciiEllipsis);
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
}

/// Build the listbox row label. The original menu adds only
/// original-game save text to the list box.
fn row_label<'a>(
    row: ListRow,
    save_manager: &'a SaveGameManager,
    visible: &[usize],
    text: &SaveMetadataText<'_>,
) -> Cow<'a, str> {
    match row {
        ListRow::New => Cow::Owned(text.new_save_label()),
        ListRow::Existing(v_idx) => {
            let slot = visible[v_idx];
            let save = save_manager
                .get(slot)
                .expect("visible slot must resolve to a save");
            existing_save_row_label(save)
        }
    }
}

pub(crate) fn existing_save_row_label(save: &SaveGame) -> Cow<'_, str> {
    if save.is_autosave() {
        Cow::Owned(format!("Autosave - {}", save.text))
    } else {
        Cow::Borrowed(&save.text)
    }
}

fn row_detail_lines(
    row: ListRow,
    save_manager: &SaveGameManager,
    visible: &[usize],
    now_unix: Option<u64>,
    local_time_zone: Option<&TimeZone>,
    text: &SaveMetadataText<'_>,
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
    text: &SaveMetadataText<'_>,
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

fn compact_row_detail(
    save: &SaveGame,
    local_time_zone: Option<&TimeZone>,
    text: &SaveMetadataText<'_>,
) -> String {
    let exact = format_compact_saved_time(&save.timestamp, local_time_zone, text);
    let mut output = text.compact_saved(&exact);
    let mut append = |part: &str| {
        output.push_str(" | ");
        output.push_str(part);
    };
    if let Some(seconds) = save.mission_elapsed_seconds {
        append(&format!("{:02}:{:02}", seconds / 60, seconds % 60));
    }
    if !save.mission_name.is_empty() && save.text.trim() != save.mission_name.trim() {
        append(&save.mission_name);
    }
    if let Some(progress) = save.campaign_progress {
        append(&text.compact_campaign_progress(progress));
    }
    if let (Some(done), Some(total)) = (save.missions_done, save.missions_total) {
        append(&text.compact_missions(done, total));
    }
    if let Some(gang) = save.gang_size {
        append(&text.compact_gang_size(gang));
    }
    if let Some(ransom) = save.ransom {
        append(&text.compact_ransom(ransom));
    }
    if let Some(blazons) = save.blazons {
        append(&text.compact_blazons(blazons));
    }
    if let Some(amulets) = save.amulets {
        append(&text.compact_amulets(amulets));
    }
    output
}

fn selected_metadata_lines(
    save: &SaveGame,
    now_unix: Option<u64>,
    local_time_zone: Option<&TimeZone>,
    text: &SaveMetadataText<'_>,
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

fn mission_with_time(save: &SaveGame, text: &SaveMetadataText<'_>) -> String {
    let mission = metadata_value(&save.mission_name, text);
    match save.mission_elapsed_seconds {
        Some(seconds) => format!("{mission} ({:02}:{:02})", seconds / 60, seconds % 60),
        None => mission.into_owned(),
    }
}

fn metadata_value<'a>(value: &'a str, text: &SaveMetadataText<'_>) -> Cow<'a, str> {
    if value.is_empty() {
        Cow::Owned(text.legacy_value_unavailable())
    } else {
        Cow::Borrowed(value)
    }
}

fn parse_save_timestamp(timestamp: &str) -> Result<u64, ()> {
    timestamp.parse::<u64>().map_err(|_| ())
}

fn format_exact_saved_time(
    timestamp: &str,
    time_zone: Option<&TimeZone>,
    text: &SaveMetadataText<'_>,
) -> String {
    format_local_saved_time(timestamp, time_zone, text, "%Y-%m-%d %H:%M:%S %Z")
}

fn format_compact_saved_time(
    timestamp: &str,
    time_zone: Option<&TimeZone>,
    text: &SaveMetadataText<'_>,
) -> String {
    format_local_saved_time(timestamp, time_zone, text, "%Y-%m-%d %H:%M")
}

fn format_local_saved_time(
    timestamp: &str,
    time_zone: Option<&TimeZone>,
    text: &SaveMetadataText<'_>,
    format: &str,
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
        .strftime(format)
        .to_string()
}

fn format_relative_saved_time(
    timestamp: &str,
    now_unix: Option<u64>,
    text: &SaveMetadataText<'_>,
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
    selected_manager_index: Option<usize>,
    mode: SaveLoadMode,
    save_manager: &SaveGameManager,
) {
    if mode != SaveLoadMode::Save {
        input_widget.set_text("");
        return;
    }
    match selected_manager_index {
        Some(slot) => {
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
    #[test]
    fn save_catalog_fallback_preserves_complete_relative_time_grammar() {
        for locale in [
            None,
            Some("en-US"),
            Some("de-DE"),
            Some("ru-RU"),
            Some("ja-JP"),
        ] {
            let text = SaveMetadataText::new(locale);
            for (unit, word) in [
                (RelativeTimeUnit::Second, "second"),
                (RelativeTimeUnit::Minute, "minute"),
                (RelativeTimeUnit::Hour, "hour"),
                (RelativeTimeUnit::Day, "day"),
                (RelativeTimeUnit::Week, "week"),
                (RelativeTimeUnit::Month, "month"),
                (RelativeTimeUnit::Year, "year"),
            ] {
                for value in [0, 1, 2, 21] {
                    let suffix = if value == 1 { "" } else { "s" };
                    assert_eq!(
                        text.elapsed(value, unit),
                        format!("{value} {word}{suffix} ago")
                    );
                    assert_eq!(
                        text.future(value, unit),
                        format!("in {value} {word}{suffix}")
                    );
                }
            }
            assert_eq!(text.just_now(), "just now");
            assert_eq!(text.new_save_label(), "< New Save >");
        }
    }

    #[test]
    fn save_catalog_inserts_metadata_without_reinterpreting_braces() {
        let text = SaveMetadataText::new(Some("untranslated"));
        assert_eq!(
            text.mission("Sherwood {value}"),
            "Mission: Sherwood {value}"
        );
        assert_eq!(text.player("{done}/{total}"), "Player: {done}/{total}");
        assert_eq!(text.missions(2, 12), "Missions: 2/12");
        assert_eq!(text.compact_missions(2, 12), "2/12 missions");
        assert_eq!(text.campaign_progress(40), "Campaign: 40%");
    }

    #[test]
    fn save_text_truncation_borrows_unchanged_text() {
        use crate::ingame_menu::layout::truncate_to_pixel_width_by;
        const POLICY: TruncationMarker = TruncationMarker::AsciiEllipsis;
        let text = String::from("café");
        let measure = |text: &str| text.chars().count() as i32;
        let fitted = truncate_to_pixel_width_by(&text, 4, POLICY, measure);
        assert!(matches!(fitted, std::borrow::Cow::Borrowed(_)));
        assert_eq!(fitted.as_ptr(), text.as_ptr());
        for width in [0, 1, 2] {
            let empty = truncate_to_pixel_width_by(&text, width, POLICY, measure);
            assert!(matches!(empty, std::borrow::Cow::Borrowed("")));
        }
        let shortened = truncate_to_pixel_width_by(&text, 3, POLICY, measure);
        assert!(matches!(shortened, std::borrow::Cow::Owned(_)));
        assert_eq!(shortened, "...");
    }

    #[test]
    fn save_text_truncation_keeps_graphemes_and_ascii_ellipsis_policy() {
        let measure = |text: &str| text.chars().count() as i32;
        for (text, width, expected) in [
            ("abcdef", 5, "ab..."),
            ("abcdef", 3, "..."),
            ("abcdef", 2, ""),
            ("abcdef", 0, ""),
            ("abcdef", -1, ""),
            ("a", 1, "a"),
            ("", 1, ""),
            ("e\u{301}clair", 4, "..."),
            ("e\u{301}clair", 5, "e\u{301}..."),
            ("👩‍💻abc", 5, "..."),
            ("👩‍💻abcd", 6, "👩‍💻..."),
            ("ab cd ef", 6, "ab ..."),
        ] {
            assert_eq!(
                crate::ingame_menu::layout::truncate_to_pixel_width_by(
                    text,
                    width,
                    TruncationMarker::AsciiEllipsis,
                    measure
                ),
                expected
            );
        }
    }

    #[test]
    fn clip_truncation_keeps_whole_graphemes_without_marker() {
        let measure = |text: &str| text.chars().count() as i32;
        for (text, width, expected) in [
            ("abcdef", 6, "abcdef"),
            ("abcdef", 4, "abcd"),
            ("abcdef", 0, ""),
            ("abcdef", -1, ""),
            ("e\u{301}clair", 1, ""),
            ("e\u{301}clair", 3, "e\u{301}c"),
        ] {
            let fitted = crate::ingame_menu::layout::truncate_to_pixel_width_by(
                text,
                width,
                TruncationMarker::Clip,
                measure,
            );
            assert!(matches!(fitted, std::borrow::Cow::Borrowed(_)));
            assert_eq!(fitted, expected);
        }
    }

    use super::*;
    use crate::scroll_view::ScrollView;

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
                    existing_save_row_detail_lines(
                        &save,
                        Some(123),
                        None,
                        &SaveMetadataText::default(),
                        detailed,
                    )[0]
                    .contains(expected)
                );
            }
            assert!(
                selected_metadata_lines(&save, Some(123), None, &SaveMetadataText::default())[0]
                    .contains(expected)
            );
        }
        save.mission_elapsed_seconds = None;
        assert_eq!(
            mission_with_time(&save, &SaveMetadataText::default()),
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
    fn save_row_labels_borrow_unmodified_text_and_own_autosave_prefixes() {
        let save = published_metadata("Savegame_000");
        let label = existing_save_row_label(&save);
        assert!(matches!(label, Cow::Borrowed(_)));
        assert_eq!(label.as_ptr(), save.text.as_ptr());
        assert_eq!(label, "The Silver Arrow");
        let autosave = published_metadata("Autosave_100_0000");
        let label = existing_save_row_label(&autosave);
        assert!(matches!(label, Cow::Owned(_)));
        assert_eq!(label, "Autosave - The Silver Arrow");
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
                &SaveMetadataText::default(),
            ),
            "Autosave - The Silver Arrow"
        );
    }

    #[test]
    fn relative_time_covers_thresholds_and_future_clock_changes() {
        let text = SaveMetadataText::default();
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
        let text = SaveMetadataText::default();
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
    fn local_time_formats_share_validation_before_timezone_availability() {
        let text = SaveMetadataText::default();
        for timestamp in [
            "",
            "not-a-clock",
            "-1",
            "18446744073709551616",
            "18446744073709551615",
            "9223372036854775807",
        ] {
            for zone in [None, Some(&TimeZone::UTC)] {
                assert_eq!(
                    format_exact_saved_time(timestamp, zone, &text),
                    "invalid timestamp"
                );
                assert_eq!(
                    format_compact_saved_time(timestamp, zone, &text),
                    "invalid timestamp"
                );
            }
        }
        for timestamp in ["0", "59", "86400"] {
            assert_eq!(
                format_exact_saved_time(timestamp, None, &text),
                "local time unavailable"
            );
            assert_eq!(
                format_compact_saved_time(timestamp, None, &text),
                "local time unavailable"
            );
        }
        assert_eq!(
            format_compact_saved_time("59", Some(&TimeZone::UTC), &text),
            "1970-01-01 00:00"
        );
        assert_eq!(
            format_exact_saved_time("59", Some(&TimeZone::UTC), &text),
            "1970-01-01 00:00:59 UTC"
        );
    }

    #[test]
    fn exact_time_uses_the_requested_zone() {
        let text = SaveMetadataText::default();
        assert_eq!(
            format_exact_saved_time("0", Some(&TimeZone::UTC), &text),
            "1970-01-01 00:00:00 UTC"
        );
    }

    #[test]
    fn metadata_values_borrow_present_text_without_trimming() {
        let text = SaveMetadataText::default();
        for value in ["The Silver Arrow", "Alice", "  ", " é "] {
            let rendered = metadata_value(value, &text);
            assert!(matches!(rendered, Cow::Borrowed(_)));
            assert_eq!(rendered, value);
            assert_eq!(rendered.as_ptr(), value.as_ptr());
        }
        let missing = metadata_value("", &text);
        assert!(matches!(missing, Cow::Owned(_)));
        assert_eq!(missing, text.legacy_value_unavailable());
    }

    #[test]
    fn every_existing_row_leads_with_required_metadata() {
        let text = SaveMetadataText::default();
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
        let text = SaveMetadataText::default();
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
    fn compact_save_details_preserve_order_zeroes_and_missing_fields() {
        let mut save = saved_at("3600");
        save.campaign_progress = Some(0);
        save.missions_done = Some(0);
        save.missions_total = Some(12);
        save.gang_size = Some(0);
        save.ransom = Some(-1);
        save.blazons = Some(0);
        save.amulets = Some(0);
        assert_eq!(
            compact_row_detail(&save, Some(&TimeZone::UTC), &SaveMetadataText::default()),
            "Saved 1970-01-01 01:00 | The Silver Arrow | 0% campaign | 0/12 missions | Gang 0 | Ransom -1 | Blazons 0 | Amulets 0"
        );
        save.text = format!("  {}  ", save.mission_name);
        save.campaign_progress = None;
        save.missions_total = None;
        save.gang_size = None;
        save.ransom = None;
        save.blazons = None;
        save.amulets = None;
        assert_eq!(
            compact_row_detail(&save, Some(&TimeZone::UTC), &SaveMetadataText::default()),
            "Saved 1970-01-01 01:00"
        );
    }

    #[test]
    fn compact_mode_hides_expanded_provenance_without_discarding_it() {
        let text = SaveMetadataText::default();
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

    fn pause_config(mode: SaveLoadMode, multiplayer_connected: bool) -> SavePickerConfig {
        SavePickerConfig {
            mode,
            mission_id: Some(1),
            detailed_metadata: false,
            multiplayer_connected,
            previews: false,
        }
    }

    fn pause_picker(manager: &SaveGameManager, mode: SaveLoadMode) -> SavePickerModalState {
        SavePickerModalState::with_input(manager, pause_config(mode, false), ModalInputState::new())
    }

    fn picker_fixture(manager: &mut SaveGameManager, name: &str) {
        let mut save = SaveGame::new(name.into(), name.into(), 1);
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
        manager.insert_test_slot(save, crate::savegame::SlotState::Published);
    }

    fn picker_event_frame(
        state: &mut SavePickerModalState,
        manager: &SaveGameManager,
        events: &[GameEvent],
    ) -> Option<PickerAction> {
        state.controller.scroll_view.get_or_insert_with(|| {
            ScrollView::with_geometry([30, 42, 420, 360], 36, 16, 16, false)
        });
        state.refresh(manager);
        state.controller.begin_frame(
            &state.model,
            &[
                (0, "Accept", 460, 300),
                (1, "Delete", 460, 350),
                (2, "Cancel", 460, 400),
            ],
            150,
            40,
        );
        for event in events {
            state.handle_event(event, MenuTransform::centered(640, 480), manager);
        }
        state.finish_input();
        state.controller.take_action()
    }

    fn picker_key(keycode: Keycode) -> GameEvent {
        GameEvent::KeyDown {
            keycode,
            physical_key: None,
        }
    }

    #[test]
    fn pause_picker_uses_shared_draft_autosave_and_multiplayer_policy() {
        let mut manager = SaveGameManager::new("unused-pause-picker-policy".into());
        picker_fixture(&mut manager, "Autosave_100_0000");
        picker_fixture(&mut manager, "Savegame_001");
        manager.create_draft("Draft".into(), 1).unwrap();
        let mut pause = pause_picker(&manager, SaveLoadMode::Load);
        assert_eq!(
            pause.model.visible().len(),
            2,
            "drafts never appear in load picker"
        );
        picker_event_frame(&mut pause, &manager, &[picker_key(Keycode::Down)]);
        assert!(!pause.model.can_delete());
        assert!(pause.model.request_delete().is_none());
        for event in [
            GameEvent::MouseMove {
                x: 480,
                y: 360,
                xrel: 0,
                yrel: 0,
            },
            GameEvent::MouseDown(480, 360, 1, 1),
            GameEvent::MouseUp(480, 360, 1),
        ] {
            assert_eq!(
                picker_event_frame(&mut pause, &manager, &[event]),
                None,
                "pause delete button must not offer protected autosave deletion"
            );
        }
        assert_eq!(
            pause.model.selected_slot().unwrap().as_str(),
            "Autosave_100_0000"
        );
        let manual = manager.find_by_filename("Savegame_001").unwrap();
        manager.get_mut(manual).unwrap().multiplayer_diagnostic = true;
        let connected = SavePickerModalState::with_input(
            &manager,
            pause_config(SaveLoadMode::Load, true),
            ModalInputState::new(),
        );
        assert_eq!(connected.model.visible().len(), 1);
        let save = pause_picker(&manager, SaveLoadMode::Save);
        assert_eq!(save.model.selected_row(), Some(ListRow::New));
        assert!(
            save.model
                .visible()
                .iter()
                .all(|index| !manager.get(*index).unwrap().is_special())
        );
    }

    #[test]
    fn actual_pause_input_matches_standalone_actions_and_edits_non_ascii_at_caret() {
        let manager = SaveGameManager::new("unused-pause-picker-input".into());
        let mut pause = pause_picker(&manager, SaveLoadMode::Save);
        let mut standalone = PickerModel::new(
            SaveLoadMode::Save,
            false,
            (list_rect(SaveLoadMode::Save).h / row_height(false)) as usize,
            picker_slots(&manager),
        );
        let mut controller = PickerController::new(ModalInputState::new());
        controller.scroll_view = Some(ScrollView::with_geometry(
            [30, 42, 420, 360],
            36,
            16,
            16,
            false,
        ));
        let traces = [
            vec![GameEvent::TextInput {
                text: "é雪".into()
            }],
            vec![
                picker_key(Keycode::Left),
                GameEvent::TextInput { text: "Ω".into() },
            ],
            vec![picker_key(Keycode::Backspace)],
            vec![picker_key(Keycode::Delete)],
            vec![picker_key(Keycode::Return)],
            vec![picker_key(Keycode::Escape)],
        ];
        for events in traces {
            let action = picker_event_frame(&mut pause, &manager, &events);
            controller.begin_frame(
                &standalone,
                &[
                    (0, "Save", 460, 300),
                    (1, "Delete", 460, 350),
                    (2, "Cancel", 460, 400),
                ],
                150,
                40,
            );
            for event in &events {
                controller.handle_event(&mut standalone, event, MenuTransform::centered(640, 480));
            }
            controller.process_widgets(&standalone);
            controller.input.end_frame();
            assert_eq!(action, controller.take_action());
            assert_eq!(pause.model, standalone);
        }
        assert_eq!(pause.name.edit_text, "é");
        assert_eq!(pause.name.caret_offset, 1);
        assert_eq!(pause.name.base.state, crate::ui::UiState::SelectedEditable);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pause_confirmation_handles_changed_catalog_and_surfaces_delete_failure() {
        let directory = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(directory.path().to_str().unwrap().into());
        picker_fixture(&mut manager, "Savegame_000");
        picker_fixture(&mut manager, "Savegame_001");
        manager.save_index().unwrap();
        let mut pause = pause_picker(&manager, SaveLoadMode::Load);
        picker_event_frame(&mut pause, &manager, &[picker_key(Keycode::Down)]);
        let name = pause.model.selected_slot().unwrap().clone();
        assert!(begin_picker_delete(&mut pause.model, name.clone()));
        manager.remove_by_filename(name.as_str()).unwrap();
        pause.finish_confirmation(SaveConfirmation::Delete, true, &mut manager, None);
        assert!(
            pause
                .model
                .operation_error()
                .unwrap()
                .contains("no longer available")
        );
        assert_eq!(
            manager.count(),
            1,
            "a changed catalog must not redirect deletion"
        );
        pause.model.dismiss_error();
        picker_event_frame(&mut pause, &manager, &[picker_key(Keycode::Down)]);
        let name = pause.model.selected_slot().unwrap().clone();
        assert!(begin_picker_delete(&mut pause.model, name));
        std::fs::create_dir(directory.path().join("Savegame_001.json")).unwrap();
        pause.finish_confirmation(SaveConfirmation::Delete, true, &mut manager, None);
        assert_eq!(
            manager.count(),
            0,
            "published removal is not rolled back after cleanup failure"
        );
        assert!(pause.model.operation_error().unwrap().contains("cleanup"));
        assert_eq!(pause.model.selected_row(), None);
    }

    #[test]
    fn pause_overwrite_confirmation_revalidates_identity_after_catalog_change() {
        let mut manager = SaveGameManager::new("unused-pause-overwrite".into());
        picker_fixture(&mut manager, "Savegame_000");
        let mut pause = pause_picker(&manager, SaveLoadMode::Save);
        picker_event_frame(&mut pause, &manager, &[picker_key(Keycode::Down)]);
        let name = pause.model.selected_slot().unwrap().clone();
        pause.name.set_text("Edited but not saved");
        assert!(begin_picker_delete(&mut pause.model, name.clone()));
        assert!(
            pause
                .finish_confirmation(SaveConfirmation::Delete, false, &mut manager, None)
                .is_none()
        );
        assert_eq!(
            pause.name.edit_text, "Edited but not saved",
            "cancelling deletion retains pending name edits"
        );
        // Switching to a fresh catalog simulates disappearance while the
        // confirmation's stable identity remains alive.
        let mut replacement = SaveGameManager::new("unused-pause-overwrite".into());
        picker_fixture(&mut replacement, "Savegame_001");
        assert!(
            pause
                .finish_confirmation(
                    SaveConfirmation::Overwrite(name),
                    true,
                    &mut replacement,
                    None
                )
                .is_none()
        );
        assert!(
            pause
                .model
                .operation_error()
                .unwrap()
                .contains("no longer available")
        );
        assert_eq!(replacement.get(0).unwrap().text, "Savegame_001");
    }

    #[test]
    fn pause_existing_slot_actions_match_standalone_controller() {
        let mut manager = SaveGameManager::new("unused-pause-action-trace".into());
        picker_fixture(&mut manager, "Savegame_000");
        picker_fixture(&mut manager, "Savegame_001");
        let mut pause = pause_picker(&manager, SaveLoadMode::Load);
        let mut model = pause.model.clone();
        let mut controller = PickerController::new(ModalInputState::new());
        controller.scroll_view = Some(ScrollView::with_geometry(
            [30, 42, 420, 360],
            36,
            16,
            16,
            false,
        ));
        let mut actions = Vec::new();
        for events in [
            vec![
                picker_key(Keycode::Down),
                picker_key(Keycode::Down),
                picker_key(Keycode::Return),
            ],
            vec![GameEvent::MouseMove {
                x: 480,
                y: 360,
                xrel: 0,
                yrel: 0,
            }],
            vec![GameEvent::MouseDown(480, 360, 1, 1)],
            vec![GameEvent::MouseUp(480, 360, 1)],
            vec![picker_key(Keycode::Escape)],
        ] {
            let action = picker_event_frame(&mut pause, &manager, &events);
            controller.begin_frame(
                &model,
                &[
                    (0, "Load", 460, 300),
                    (1, "Delete", 460, 350),
                    (2, "Cancel", 460, 400),
                ],
                150,
                40,
            );
            for event in &events {
                controller.handle_event(&mut model, event, MenuTransform::centered(640, 480));
            }
            controller.process_widgets(&model);
            controller.input.end_frame();
            assert_eq!(action, controller.take_action());
            assert_eq!(pause.model, model);
            if let Some(action) = action {
                actions.push(action);
            }
        }
        let name = SlotName::new("Savegame_001").unwrap();
        assert_eq!(
            actions,
            vec![
                PickerAction::Accept(PickerTarget::Existing(name.clone())),
                PickerAction::ConfirmDelete(name),
                PickerAction::Cancel
            ]
        );
    }
}

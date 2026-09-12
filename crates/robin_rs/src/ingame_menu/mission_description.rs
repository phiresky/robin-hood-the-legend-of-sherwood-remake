//! Pre-mission description modal.
//!
//! A 496x463 parchment window with a title label in the top-left, a
//! mission picture in the top-right, a justified description below
//! (either wrapping around the picture as a dropped-initial, or
//! sitting below it when the mission requires blazons), and a
//! horizontally-centred button row along the bottom.
//!
//! The state machine lives on [`MissionDescriptionScreen`] in
//! [`crate::ui_screens`]; this module is the frame-driven presentation
//! shell that drives it — loads the picture, lays out buttons,
//! wires tooltip lookups and the Return/Escape focus-manager
//! shortcuts, and returns the player's [`MissionChoice`].
//!
//! The grid of coat-of-arms icons is rendered down the left side when
//! the mission requires blazons (see [`super::blazon_set`]); layout +
//! classification comes from
//! [`robin_engine::widget_state::blazon_set::build_blazon_set_state`]
//! and per-slot tooltips are delayed the same ~750 ms the button
//! tooltips use.

use crate::gfx_types::Keycode;
use crate::ingame_menu::resources::SealButton;
use robin_engine::campaign::CampaignValue;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::engine::{
    Engine, ExternalAction, ExternalActionResult, LevelAssets, SimulationFrameInput,
};
use robin_engine::profiles as engine_profiles;
use robin_engine::sprite::BBox;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::ui_screens::{
    MissionChoice, MissionDescriptionButton, MissionDescriptionScreen, center_horizontally_x,
    mission_description_layout as layout_consts,
};
use crate::widget::FrameWnd;
use robin_assets::res_descr::LevelDescriptors;
use robin_assets::resource_manager::ResourceManager;

use super::blazon_set::{self, BlazonTooltipTracker};
use super::buy_blazons::{BuyBlazonsModalState, BuyBlazonsOutcome};
use super::layout::{
    MENU_H, MENU_W, MenuTransform, TextAlign, TooltipState, VAlign, dim_screen, draw_background,
    enter_modal_gpu_phase, render_text_in_box_aligned_font, render_text_in_box_font,
    render_text_in_box_with_drop_cap_font,
};
use super::resources::{IngameMenuResources, MenuSurface};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

/// Widget IDs within the [`crate::widget::FrameWnd`].  Mapped back to
/// [`MissionDescriptionButton`] via [`button_for_widget`] / vice versa.
const ID_CONVERT_PEASANTS: u32 = 0;
const ID_CONVERT_MONEY: u32 = 1;
const ID_CONVERT_MISSION: u32 = 2;
const ID_START_MISSION: u32 = 3;
const ID_CANCEL: u32 = 4;

/// Execute one campaign-map mutation synchronously with the hourglass gated,
/// then retain the same typed action in the enclosing authoritative frame.
///
/// The modal needs the buy result immediately, while replay/rewind need the
/// mutation journaled. `MissionFrame`'s applied-action cursor ensures live
/// execution does not apply it twice when the enclosing frame advances.
pub(crate) fn admit_paused_campaign_action(
    engine: &mut Engine,
    assets: &LevelAssets,
    action: ExternalAction,
) -> ExternalActionResult {
    let output = engine
        .advance_frame(
            assets,
            SimulationFrameInput::no_hourglass().with_external_actions(vec![action.clone()]),
        )
        .unwrap_or_else(|error| panic!("paused campaign action admission failed: {error}"));
    let mut results = output.external_action_results.into_iter();
    let result = results
        .next()
        .expect("paused campaign admission returned no action result");
    assert!(
        results.next().is_none(),
        "one paused campaign action returned multiple results"
    );
    result
}

fn widget_id_for(button: MissionDescriptionButton) -> u32 {
    match button {
        MissionDescriptionButton::ConvertPeasants => ID_CONVERT_PEASANTS,
        MissionDescriptionButton::ConvertMoney => ID_CONVERT_MONEY,
        MissionDescriptionButton::ConvertMission => ID_CONVERT_MISSION,
        MissionDescriptionButton::StartMission => ID_START_MISSION,
        MissionDescriptionButton::Cancel => ID_CANCEL,
    }
}

fn button_for_widget(id: u32) -> Option<MissionDescriptionButton> {
    match id {
        ID_CONVERT_PEASANTS => Some(MissionDescriptionButton::ConvertPeasants),
        ID_CONVERT_MONEY => Some(MissionDescriptionButton::ConvertMoney),
        ID_CONVERT_MISSION => Some(MissionDescriptionButton::ConvertMission),
        ID_START_MISSION => Some(MissionDescriptionButton::StartMission),
        ID_CANCEL => Some(MissionDescriptionButton::Cancel),
        _ => None,
    }
}

/// Persistent one-frame driver for the pre-mission description flow.
///
/// This includes the money-to-blazon child confirmation, so opening a
/// mission from Sherwood never enters a nested event loop.
pub struct MissionDescriptionModalState {
    mission_index: usize,
    screen: MissionDescriptionScreen,
    picture: Option<MenuSurface>,
    pic_w: i32,
    pic_h: i32,
    pic_x: i32,
    pic_y: i32,
    win_x: i32,
    win_y: i32,
    blazon_box_w: u32,
    blazon_box_h: u32,
    frame: FrameWnd,
    input_state: ModalInputState,
    tooltip: TooltipState,
    blazon_tooltip: BlazonTooltipTracker,
    buy_blazons: Option<BuyBlazonsModalState>,
}

impl MissionDescriptionModalState {
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &mut IngameMenuResources,
        mission_index: usize,
        engine: &Engine,
        profiles: &engine_profiles::ProfileManager,
        level_descriptors: Option<&LevelDescriptors>,
        text_resources: &mut ResourceManager,
    ) -> Self {
        let screen = {
            let campaign = engine.campaign();
            let mission = &campaign.missions[mission_index];
            MissionDescriptionScreen::create(
                mission_index,
                mission,
                campaign,
                profiles,
                level_descriptors,
                text_resources,
            )
        };
        let picture = resources.picture_from(renderer, text_resources, screen.picture_id);
        let (pic_w, pic_h) = picture.map(|p| (p.width, p.height)).unwrap_or((0, 0));
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let win_x = (MENU_W - layout_consts::WINDOW_WIDTH) / 2;
        let win_y = (MENU_H - layout_consts::WINDOW_HEIGHT) / 2;
        let pic_x = layout_consts::PICTURE_FRAME_RIGHT_EDGE - pic_w;
        let pic_y = layout_consts::PICTURE_FRAME_Y;
        let (btn_w, btn_h) = resources.seal_button_dimensions(SealButton::Ok);
        let buttons = screen.buttons();
        let widths: Vec<i32> = buttons.iter().map(|_| btn_w).collect();
        let xs = center_horizontally_x(
            &widths,
            layout_consts::WINDOW_WIDTH,
            layout_consts::BUTTON_GAP,
        );
        let mut frame = FrameWnd::interactive();
        for (idx, button) in buttons.iter().enumerate() {
            let sprite_id = match button {
                MissionDescriptionButton::Cancel => robin_engine::resource_ids::RHID_CANCEL,
                MissionDescriptionButton::StartMission => {
                    if screen.requires_blazons {
                        robin_engine::resource_ids::RHID_START_MISSION_FOR_BLAZONS
                    } else {
                        robin_engine::resource_ids::RHID_OK
                    }
                }
                MissionDescriptionButton::ConvertPeasants => {
                    robin_engine::resource_ids::RHID_CONVERT_PEASANTS_TO_BLAZONS
                }
                MissionDescriptionButton::ConvertMoney => {
                    robin_engine::resource_ids::RHID_CONVERT_MONEY_TO_BLAZONS
                }
                MissionDescriptionButton::ConvertMission => {
                    robin_engine::resource_ids::RHID_CONVERT_MISSION_TO_BLAZONS
                }
            };
            let widget = widget_bridge::make_button_with_resource(
                widget_id_for(*button),
                "",
                screen.is_enabled(*button),
                sprite_id,
                win_x + xs[idx],
                win_y + layout_consts::BUTTON_ROW_Y,
                btn_w,
                btn_h,
            );
            frame.add_widget_absolute(widget);
            let tooltip = MissionDescriptionScreen::tooltip(*button, &resources.menu_text);
            if let Some(w) = frame.widget_mut(widget_id_for(*button)) {
                w.base_mut().set_tooltip_text(&tooltip);
            }
        }
        widget_bridge::attach_alpha_masks(&mut frame, resources, renderer);
        let blazon_box_w =
            (pic_x - layout_consts::BLAZON_BOX_PICTURE_GAP - layout_consts::BLAZON_BOX_X).max(0)
                as u32;
        let blazon_box_h =
            (layout_consts::BLAZON_BOX_BOTTOM - layout_consts::BLAZON_BOX_Y).max(0) as u32;
        let input_state = ModalInputState::from_window(event_pump, transform);
        Self {
            mission_index,
            screen,
            picture,
            pic_w,
            pic_h,
            pic_x,
            pic_y,
            win_x,
            win_y,
            blazon_box_w,
            blazon_box_h,
            frame,
            input_state,
            tooltip: TooltipState::new(),
            blazon_tooltip: BlazonTooltipTracker::new(),
            buy_blazons: None,
        }
    }

    fn apply_buy_outcome(
        &mut self,
        outcome: BuyBlazonsOutcome,
        engine: &mut Engine,
        assets: &LevelAssets,
        admitted_actions: &mut Vec<ExternalAction>,
        profiles: &engine_profiles::ProfileManager,
    ) {
        if outcome == BuyBlazonsOutcome::Bought {
            let mission_index = u32::try_from(self.mission_index)
                .expect("campaign mission index does not fit authoritative action");
            let action = ExternalAction::CampaignBuyBlazon { mission_index };
            let closed_by_cascade =
                match admit_paused_campaign_action(engine, assets, action.clone()) {
                    ExternalActionResult::CampaignBuyBlazon { closed_by_cascade } => {
                        closed_by_cascade
                    }
                    result => {
                        panic!("campaign buy admission returned unexpected result {result:?}")
                    }
                };
            admitted_actions.push(action);
            if closed_by_cascade {
                self.screen.on_cancel();
                return;
            }
        }

        let campaign = engine.campaign();
        self.screen.update_conversion_state(
            campaign.can_convert_merry_men_to_blazons(self.mission_index, profiles),
            campaign.can_convert_money_to_blazons(self.mission_index, profiles),
            campaign.can_convert_mission_to_blazons(self.mission_index, profiles),
        );
        for button in [
            MissionDescriptionButton::ConvertPeasants,
            MissionDescriptionButton::ConvertMoney,
            MissionDescriptionButton::ConvertMission,
        ] {
            if let Some(widget) = self.frame.widget_mut(widget_id_for(button)) {
                widget.base_mut().enabled = self.screen.is_enabled(button);
            }
        }
    }

    pub fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &mut IngameMenuResources,
        mut cursor: Option<ModalCursor<'_>>,
        engine: &mut Engine,
        assets: &LevelAssets,
        admitted_actions: &mut Vec<ExternalAction>,
        profiles: &engine_profiles::ProfileManager,
    ) -> Option<(MissionChoice, bool)> {
        if let Some(child) = self.buy_blazons.as_mut() {
            if let Some(outcome) = child.tick(
                event_pump,
                renderer,
                resources,
                cursor.as_mut().map(|cursor| cursor.reborrow()),
            ) {
                self.buy_blazons = None;
                self.apply_buy_outcome(outcome, engine, assets, admitted_actions, profiles);
            }
            return self
                .screen
                .closed
                .then_some((self.screen.user_choice, self.screen.men_to_blazon_mode));
        }

        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            self.input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => self.screen.activate(MissionDescriptionButton::Cancel),
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter,
                    ..
                } => self.screen.activate(MissionDescriptionButton::StartMission),
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => self.screen.activate(MissionDescriptionButton::Cancel),
                _ => {}
            }
        }

        let widget_input = self.input_state.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        let mouse_virt =
            engine_coordinates::ScreenPoint::new(self.input_state.virt_x, self.input_state.virt_y);
        self.tooltip.update(&self.frame, mouse_virt);
        let blazon_state = self.screen.requires_blazons.then(|| {
            blazon_set::build_for_mission(
                engine.campaign(),
                profiles,
                self.mission_index,
                layout_consts::BLAZON_BOX_X,
                layout_consts::BLAZON_BOX_Y,
                self.blazon_box_w,
                self.blazon_box_h,
                0,
            )
        });
        if let Some(state) = &blazon_state {
            self.blazon_tooltip.update(
                state,
                self.win_x,
                self.win_y,
                mouse_virt.x as i32,
                mouse_virt.y as i32,
            );
        }
        self.input_state.end_frame();

        if let Some(id) = widget_bridge::find_activated(&events)
            && let Some(button) = button_for_widget(id)
        {
            if button == MissionDescriptionButton::ConvertMoney
                && self
                    .screen
                    .is_enabled(MissionDescriptionButton::ConvertMoney)
            {
                let mission = &engine.campaign().missions[self.mission_index];
                self.buy_blazons = Some(BuyBlazonsModalState::new(
                    event_pump,
                    renderer,
                    resources,
                    self.mission_index,
                    mission.get_blazon_price() as u32,
                    engine.campaign().get_value(CampaignValue::Ransom).max(0) as u32,
                ));
            } else {
                self.screen.activate(button);
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(parchment) = resources.parchment_huge {
            draw_background(
                renderer,
                transform,
                &parchment,
                self.win_x,
                self.win_y,
                layout_consts::WINDOW_WIDTH,
                layout_consts::WINDOW_HEIGHT,
            );
        } else {
            let (sx, sy) = transform.to_screen(self.win_x, self.win_y);
            renderer.fill_screen(
                Some(&BBox::from_coords(
                    sx as f32,
                    sy as f32,
                    (sx + layout_consts::WINDOW_WIDTH) as f32,
                    (sy + layout_consts::WINDOW_HEIGHT) as f32,
                )),
                Renderer::create_color_16(40, 30, 20),
            );
        }
        if let Some(picture) = self.picture {
            draw_background(
                renderer,
                transform,
                &picture,
                self.win_x + self.pic_x,
                self.win_y + self.pic_y,
                self.pic_w,
                self.pic_h,
            );
        }
        let title_right = if self.pic_w > 0 {
            self.pic_x - layout_consts::TITLE_PICTURE_GAP
        } else {
            layout_consts::DESCRIPTION_RIGHT
        };
        if let Some(font) = resources
            .title_font_any()
            .or_else(|| resources.popup_font_any())
        {
            let _ = render_text_in_box_aligned_font(
                renderer,
                font,
                transform,
                &self.screen.title,
                self.win_x + layout_consts::TITLE_X,
                self.win_y + layout_consts::TITLE_Y,
                (title_right - layout_consts::TITLE_X).max(0),
                layout_consts::TITLE_BOTTOM - layout_consts::TITLE_Y,
                TextAlign::Left,
                VAlign::Top,
            );
        }
        if let Some(font) = resources.popup_font_any() {
            if self.screen.requires_blazons {
                let top = self.pic_y + self.pic_h + layout_consts::DESCRIPTION_PICTURE_GAP;
                let _ = render_text_in_box_font(
                    renderer,
                    font,
                    transform,
                    &self.screen.description,
                    self.win_x + layout_consts::DESCRIPTION_X,
                    self.win_y + top,
                    layout_consts::DESCRIPTION_RIGHT - layout_consts::DESCRIPTION_X,
                    layout_consts::DESCRIPTION_BOTTOM - top,
                    TextAlign::Justified,
                );
            } else {
                let desc_w = layout_consts::DESCRIPTION_RIGHT - layout_consts::DESCRIPTION_X;
                let desc_h =
                    layout_consts::DESCRIPTION_BOTTOM - layout_consts::DESCRIPTION_TOP_NO_BLAZONS;
                let (drop_cap_w, drop_cap_h) = self
                    .screen
                    .description_drop_cap(self.pic_w, self.pic_h)
                    .unwrap_or((0, 0));
                let _ = render_text_in_box_with_drop_cap_font(
                    renderer,
                    font,
                    transform,
                    &self.screen.description,
                    self.win_x + layout_consts::DESCRIPTION_X,
                    self.win_y + layout_consts::DESCRIPTION_TOP_NO_BLAZONS,
                    desc_w,
                    desc_h,
                    drop_cap_w,
                    drop_cap_h,
                    TextAlign::Justified,
                );
            }
        }
        if let Some(state) = &blazon_state {
            blazon_set::render(
                renderer, transform, resources, state, self.win_x, self.win_y,
            );
        }
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &self.frame);
        if let Some(font) = resources.popup_font_any() {
            self.tooltip
                .draw(renderer, font, transform, &self.frame, mouse_virt);
            if self.tooltip.hover_widget().is_none() && blazon_state.is_some() {
                self.blazon_tooltip.draw(
                    renderer,
                    font,
                    transform,
                    resources,
                    mouse_virt.x as i32,
                    mouse_virt.y as i32,
                );
            }
        }
        if let Some(cursor) = &cursor {
            cursor.draw(renderer, transform, &self.input_state);
        }
        renderer.present();

        self.screen
            .closed
            .then_some((self.screen.user_choice, self.screen.men_to_blazon_mode))
    }
}

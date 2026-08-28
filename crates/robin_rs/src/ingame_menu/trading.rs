//! Live Sherwood inventory-trading panel.

use crate::gfx_types::{GameEvent, Keycode};
use crate::renderer::Renderer;
use crate::widget::FrameWnd;
use robin_engine::campaign::CampaignValue;
use robin_engine::sector_production::{SectorProduction, Type};
use robin_engine::trading::{
    TRADE_ITEMS, TradeOutcome, TradeQuantity, TradeReceipt, TradeRejectReason,
};

use super::layout::{
    MENU_W, MenuTransform, TextAlign, dim_screen, draw_screen_background, enter_modal_gpu_phase,
    render_text_in_box, render_text_virt,
};
use super::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_SELL_FIVE, MT_BTN_SELL_ONE, MT_STR_TRADE_CONFIRM,
    MT_STR_TRADE_DISABLED, MT_STR_TRADE_RANSOM, MT_STR_TRADE_REASON_HOST, MT_STR_TRADE_REASON_ITEM,
    MT_STR_TRADE_REASON_LOCATION, MT_STR_TRADE_REASON_OVERFLOW, MT_STR_TRADE_REASON_STOCK,
    MT_STR_TRADE_REJECTED, MT_STR_TRADE_ROW, MT_STR_TRADE_SOLD, MT_STR_TRADE_WAITING,
    MT_TTL_SHERWOOD_TRADING,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

const ID_SELL_ONE: u32 = 400;
const ID_SELL_FIVE: u32 = 401;
const ID_CLOSE: u32 = 402;
const ROW_X: i32 = 72;
const ROW_Y: i32 = 82;
const ROW_W: i32 = 496;
const ROW_H: i32 = 31;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingOutcome {
    Close,
    Sell {
        prod_type: Type,
        quantity: TradeQuantity,
    },
}

#[derive(Debug, Clone)]
struct TradingRow {
    prod_type: Type,
    name: String,
    stock: u16,
    unit_price: u16,
}

pub struct TradingModalState {
    rows: Vec<TradingRow>,
    selected: usize,
    focus: usize,
    ransom: i32,
    pending_confirmation: Option<(Type, TradeQuantity)>,
    awaiting_receipt: bool,
    status: String,
    frame: FrameWnd,
    input: ModalInputState,
}

impl TradingModalState {
    pub fn new(
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        sectors: &[SectorProduction],
        ransom: i32,
    ) -> Self {
        let rows = TRADE_ITEMS
            .iter()
            .map(|definition| {
                let sector = sectors
                    .iter()
                    .find(|sector| sector.prod_type == definition.prod_type)
                    .unwrap_or_else(|| {
                        panic!(
                            "Sherwood trading table item {:?} has no campaign production sector",
                            definition.prod_type
                        )
                    });
                TradingRow {
                    prod_type: definition.prod_type,
                    name: resources
                        .menu_text
                        .get(robin_engine::sherwood_stat::bonus_text_id(
                            definition.prod_type,
                        )),
                    stock: sector.amount,
                    unit_price: definition.unit_price,
                }
            })
            .collect();

        let (button_w, button_h) = resources.button_dimensions();
        let gap = 12;
        let total = button_w * 3 + gap * 2;
        let start_x = (MENU_W - total) / 2;
        let button_y = 402;
        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_SELL_ONE,
            &resources.menu_text.get(MT_BTN_SELL_ONE),
            start_x,
            button_y,
            button_w,
            button_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_SELL_FIVE,
            &resources.menu_text.get(MT_BTN_SELL_FIVE),
            start_x + button_w + gap,
            button_y,
            button_w,
            button_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_CLOSE,
            &resources.menu_text.get(MT_BTN_CANCEL),
            start_x + (button_w + gap) * 2,
            button_y,
            button_w,
            button_h,
        ));
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let mut input = ModalInputState::new();
        input.seed_mouse_from_window(window, transform);
        let mut state = Self {
            rows,
            selected: 0,
            focus: 0,
            ransom,
            pending_confirmation: None,
            awaiting_receipt: false,
            status: String::new(),
            frame,
            input,
        };
        state.update_button_enablement();
        state
    }

    pub fn tick(
        &mut self,
        window: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        receipts: Vec<TradeReceipt>,
        sectors: &[SectorProduction],
        cursor: Option<ModalCursor<'_>>,
    ) -> Option<TradingOutcome> {
        self.refresh_stocks(sectors);
        for receipt in receipts {
            self.apply_receipt(resources, receipt);
        }

        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let mut keyboard_activation = None;
        for event in window.poll_events() {
            self.input.update_from_event(&event, transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => return Some(TradingOutcome::Close),
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => self.move_selection(-1),
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => self.move_selection(1),
                GameEvent::KeyDown {
                    keycode: Keycode::Tab | Keycode::Right,
                    ..
                } => self.focus = (self.focus + 1) % 3,
                GameEvent::KeyDown {
                    keycode: Keycode::Left,
                    ..
                } => self.focus = (self.focus + 2) % 3,
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter,
                    ..
                } => keyboard_activation = Some([ID_SELL_ONE, ID_SELL_FIVE, ID_CLOSE][self.focus]),
                GameEvent::KeyDown {
                    keycode: Keycode::Char(b'1'),
                    ..
                } => keyboard_activation = Some(ID_SELL_ONE),
                GameEvent::KeyDown {
                    keycode: Keycode::Char(b'5'),
                    ..
                } => keyboard_activation = Some(ID_SELL_FIVE),
                GameEvent::MouseUp(_, _, 1) => self.select_row_at_cursor(),
                _ => {}
            }
        }

        self.update_button_enablement();
        let widget_input = self.input.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        self.input.end_frame();
        let activated = widget_bridge::find_activated(&events).or(keyboard_activation);
        let outcome = match activated {
            Some(ID_SELL_ONE) => self.request_sale(resources, TradeQuantity::One),
            Some(ID_SELL_FIVE) => self.request_sale(resources, TradeQuantity::Five),
            Some(ID_CLOSE) => Some(TradingOutcome::Close),
            _ => None,
        };

        self.render(renderer, resources, transform, cursor);
        outcome
    }

    fn refresh_stocks(&mut self, sectors: &[SectorProduction]) {
        for row in &mut self.rows {
            let sector = sectors
                .iter()
                .find(|sector| sector.prod_type == row.prod_type)
                .unwrap_or_else(|| {
                    panic!(
                        "live Sherwood inventory lost production sector {:?}",
                        row.prod_type
                    )
                });
            row.stock = sector.amount;
        }
        self.update_button_enablement();
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.rows.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
        self.pending_confirmation = None;
        self.status.clear();
    }

    fn select_row_at_cursor(&mut self) {
        let x = self.input.virt_x as i32;
        let y = self.input.virt_y as i32;
        if !(ROW_X..ROW_X + ROW_W).contains(&x) || y < ROW_Y {
            return;
        }
        let index = ((y - ROW_Y) / ROW_H) as usize;
        if index < self.rows.len() {
            self.selected = index;
            self.pending_confirmation = None;
            self.status.clear();
        }
    }

    fn update_button_enablement(&mut self) {
        let stock = self.rows[self.selected].stock;
        self.frame
            .widget_mut(ID_SELL_ONE)
            .expect("Sell 1 widget")
            .base_mut()
            .enabled = stock >= 1 && !self.awaiting_receipt;
        self.frame
            .widget_mut(ID_SELL_FIVE)
            .expect("Sell 5 widget")
            .base_mut()
            .enabled = stock >= 5 && !self.awaiting_receipt;
    }

    fn request_sale(
        &mut self,
        resources: &IngameMenuResources,
        quantity: TradeQuantity,
    ) -> Option<TradingOutcome> {
        if self.awaiting_receipt || self.rows[self.selected].stock < quantity.units() {
            return None;
        }
        let row = &self.rows[self.selected];
        let key = (row.prod_type, quantity);
        if self.pending_confirmation != Some(key) {
            self.pending_confirmation = Some(key);
            let total = u32::from(row.unit_price) * u32::from(quantity.units());
            self.status = substitute(
                &resources.menu_text.get(MT_STR_TRADE_CONFIRM),
                &[&quantity.units().to_string(), &row.name, &total.to_string()],
            );
            return None;
        }
        self.pending_confirmation = None;
        self.awaiting_receipt = true;
        self.status = resources.menu_text.get(MT_STR_TRADE_WAITING);
        Some(TradingOutcome::Sell {
            prod_type: row.prod_type,
            quantity,
        })
    }

    fn apply_receipt(&mut self, resources: &IngameMenuResources, receipt: TradeReceipt) {
        let row = self
            .rows
            .iter_mut()
            .find(|row| row.prod_type == receipt.prod_type)
            .unwrap_or_else(|| panic!("receipt for unlisted trade item {:?}", receipt.prod_type));
        match receipt.outcome {
            TradeOutcome::Sold {
                units,
                total_price,
                remaining_stock,
                ransom_after,
                ..
            } => {
                row.stock = remaining_stock;
                self.ransom = ransom_after;
                self.status = substitute(
                    &resources.menu_text.get(MT_STR_TRADE_SOLD),
                    &[
                        &units.to_string(),
                        &row.name,
                        &total_price.to_string(),
                        &remaining_stock.to_string(),
                    ],
                );
            }
            TradeOutcome::Rejected(reason) => {
                let reason = match reason {
                    TradeRejectReason::TradingDisabled => {
                        resources.menu_text.get(MT_STR_TRADE_DISABLED)
                    }
                    TradeRejectReason::HostOnly => {
                        resources.menu_text.get(MT_STR_TRADE_REASON_HOST)
                    }
                    TradeRejectReason::NotInSherwood => {
                        resources.menu_text.get(MT_STR_TRADE_REASON_LOCATION)
                    }
                    TradeRejectReason::UnsupportedProductionType => {
                        resources.menu_text.get(MT_STR_TRADE_REASON_ITEM)
                    }
                    TradeRejectReason::InsufficientStock { available } => substitute(
                        &resources.menu_text.get(MT_STR_TRADE_REASON_STOCK),
                        &[&available.to_string()],
                    ),
                    TradeRejectReason::CurrencyOverflow => {
                        resources.menu_text.get(MT_STR_TRADE_REASON_OVERFLOW)
                    }
                };
                self.status =
                    substitute(&resources.menu_text.get(MT_STR_TRADE_REJECTED), &[&reason]);
            }
        }
        self.awaiting_receipt = false;
        self.pending_confirmation = None;
        self.update_button_enablement();
    }

    fn render(
        &self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        transform: MenuTransform,
        cursor: Option<ModalCursor<'_>>,
    ) {
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(background) = resources.menu_bg[0] {
            draw_screen_background(renderer, &background);
        }
        if let Some(font) = resources.title_font() {
            let title = resources.menu_text.get(MT_TTL_SHERWOOD_TRADING);
            render_text_virt(
                renderer,
                font,
                transform,
                &title,
                (MENU_W - font.text_width(&title)) / 2,
                24,
            );
        }
        if let Some(font) = resources.label_font() {
            let ransom = substitute(
                &resources.menu_text.get(MT_STR_TRADE_RANSOM),
                &[&self.ransom.to_string()],
            );
            render_text_virt(renderer, font, transform, &ransom, ROW_X, 57);
            for (index, row) in self.rows.iter().enumerate() {
                let y = ROW_Y + index as i32 * ROW_H;
                if index == self.selected {
                    let (sx, sy) = transform.to_screen(ROW_X - 4, y - 3);
                    renderer.fill_screen(
                        Some(&robin_engine::sprite::BBox::from_coords(
                            sx as f32,
                            sy as f32,
                            (sx + ROW_W) as f32,
                            (sy + ROW_H - 2) as f32,
                        )),
                        Renderer::create_color_16(55, 74, 45),
                    );
                }
                let line = substitute(
                    &resources.menu_text.get(MT_STR_TRADE_ROW),
                    &[
                        &row.name,
                        &row.stock.to_string(),
                        &row.unit_price.to_string(),
                    ],
                );
                render_text_virt(renderer, font, transform, &line, ROW_X, y);
            }
            if !self.status.is_empty() {
                let _ = render_text_in_box(
                    renderer,
                    font,
                    transform,
                    &self.status,
                    ROW_X,
                    342,
                    ROW_W,
                    48,
                    TextAlign::Left,
                );
            }
        }
        for (index, id) in [ID_SELL_ONE, ID_SELL_FIVE, ID_CLOSE].iter().enumerate() {
            if let Some(widget) = self.frame.widget(*id) {
                widget_bridge::draw_widget_button(
                    renderer,
                    resources,
                    transform,
                    widget,
                    self.focus == index,
                );
            }
        }
        if let Some(cursor) = cursor {
            cursor.draw(renderer, transform, &self.input);
        }
        renderer.present();
    }
}

fn substitute(template: &str, values: &[&str]) -> String {
    let mut output = template.to_string();
    for value in values {
        let Some(position) = ["%u", "%d", "%s"]
            .iter()
            .filter_map(|placeholder| output.find(placeholder))
            .min()
        else {
            break;
        };
        output.replace_range(position..position + 2, value);
    }
    output
}

pub fn ransom_from_engine(engine: &robin_engine::engine::Engine) -> i32 {
    engine.campaign().get_value(CampaignValue::Ransom)
}

#[cfg(test)]
mod tests {
    use super::substitute;

    #[test]
    fn localized_trade_templates_replace_placeholders_in_order() {
        assert_eq!(
            substitute("Sold %u %s for £%u", &["5", "nets", "35"]),
            "Sold 5 nets for £35"
        );
    }
}

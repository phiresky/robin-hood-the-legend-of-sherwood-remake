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
// Nine shipped MAKE_* inventory types fit above the status area at this
// spacing. Keep selection hit-testing and rendering on the same constant.
const ROW_H: i32 = 28;

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
    /// `(assigned request id, item, quantity)` for the one command in flight.
    /// The id is filled by the host immediately after this state emits Sell.
    awaiting_receipt: Option<(Option<u64>, Type, TradeQuantity)>,
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
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let mut input = ModalInputState::new();
        input.seed_mouse_from_window(window, transform);
        Self::build(resources, sectors, ransom, input)
    }

    fn build(
        resources: &IngameMenuResources,
        sectors: &[SectorProduction],
        ransom: i32,
        input: ModalInputState,
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
        let mut state = Self {
            rows,
            selected: 0,
            focus: 0,
            ransom,
            pending_confirmation: None,
            awaiting_receipt: None,
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

        let (events, transform) = super::layout::poll_events_with_transform(window, renderer);
        let outcome = self.handle_events(resources, &events, transform);

        self.render(renderer, resources, transform, cursor);
        outcome
    }

    /// Process pointer transitions in event order. Collapsing a complete
    /// down/up tap into one final input snapshot loses the widget's required
    /// Focused -> Pushed -> Activated transition on fast touchscreens.
    fn handle_events(
        &mut self,
        resources: &IngameMenuResources,
        events: &[GameEvent],
        transform: MenuTransform,
    ) -> Option<TradingOutcome> {
        self.update_button_enablement();
        let mut activated = None;
        for event in events {
            self.input.update_from_event(&event, transform);
            let keyboard_activation = match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => return Some(TradingOutcome::Close),
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => {
                    self.move_selection(-1);
                    None
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => {
                    self.move_selection(1);
                    None
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Tab | Keycode::Right,
                    ..
                } => {
                    self.focus = (self.focus + 1) % 3;
                    None
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Left,
                    ..
                } => {
                    self.focus = (self.focus + 2) % 3;
                    None
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter,
                    ..
                } => Some([ID_SELL_ONE, ID_SELL_FIVE, ID_CLOSE][self.focus]),
                GameEvent::KeyDown {
                    keycode: Keycode::Char(b'1'),
                    ..
                } => Some(ID_SELL_ONE),
                GameEvent::KeyDown {
                    keycode: Keycode::Char(b'5'),
                    ..
                } => Some(ID_SELL_FIVE),
                _ => None,
            };

            let widget_input = self.input.as_widget_input();
            let widget_events = self.frame.process_input(&widget_input);
            self.input.end_frame();
            activated = activated
                .or_else(|| widget_bridge::find_activated(&widget_events))
                .or(keyboard_activation);

            if matches!(event, GameEvent::MouseUp(_, _, 1)) {
                self.select_row_at_cursor();
            }
        }

        match activated {
            Some(ID_SELL_ONE) => self.request_sale(resources, TradeQuantity::One),
            Some(ID_SELL_FIVE) => self.request_sale(resources, TradeQuantity::Five),
            Some(ID_CLOSE) => Some(TradingOutcome::Close),
            _ => None,
        }
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
            .enabled = stock >= 1 && self.awaiting_receipt.is_none();
        self.frame
            .widget_mut(ID_SELL_FIVE)
            .expect("Sell 5 widget")
            .base_mut()
            .enabled = stock >= 5 && self.awaiting_receipt.is_none();
    }

    fn request_sale(
        &mut self,
        resources: &IngameMenuResources,
        quantity: TradeQuantity,
    ) -> Option<TradingOutcome> {
        if self.awaiting_receipt.is_some() || self.rows[self.selected].stock < quantity.units() {
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
        self.awaiting_receipt = Some((None, row.prod_type, quantity));
        self.status = resources.menu_text.get(MT_STR_TRADE_WAITING);
        Some(TradingOutcome::Sell {
            prod_type: row.prod_type,
            quantity,
        })
    }

    /// Bind the host-issued correlation id to the sale emitted by the last
    /// tick. A missing or mismatched pending sale is a programming error.
    pub(crate) fn assign_request_id(
        &mut self,
        request_id: u64,
        prod_type: Type,
        quantity: TradeQuantity,
    ) {
        let pending = self
            .awaiting_receipt
            .as_mut()
            .expect("assigning a Sherwood request id without an awaiting sale");
        assert_eq!((pending.1, pending.2), (prod_type, quantity));
        assert!(pending.0.replace(request_id).is_none());
    }

    fn apply_receipt(&mut self, resources: &IngameMenuResources, receipt: TradeReceipt) {
        if self.awaiting_receipt
            != Some((
                Some(receipt.request_id),
                receipt.prod_type,
                receipt.quantity,
            ))
        {
            tracing::warn!(
                request_id = receipt.request_id,
                ?receipt.prod_type,
                ?receipt.quantity,
                "discarding stale or unsolicited Sherwood trade receipt"
            );
            return;
        }
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
        self.awaiting_receipt = None;
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
    use super::{
        ID_CLOSE, ID_SELL_ONE, ROW_H, ROW_Y, TradingModalState, TradingOutcome, substitute,
    };
    use crate::gfx_types::GameEvent;
    use crate::ingame_menu::IngameMenuResources;
    use crate::ingame_menu::layout::MenuTransform;
    use crate::ingame_menu::widget_bridge::ModalInputState;
    use robin_engine::sector_production::{SectorProduction, Type};
    use robin_engine::trading::{TRADE_ITEMS, TradeOutcome, TradeQuantity, TradeReceipt};

    fn button_point(state: &TradingModalState, id: u32) -> (i32, i32) {
        let rect = state
            .frame
            .widget(id)
            .expect("trading button")
            .base()
            .bbox
            .0
            .expect("trading button bbox");
        (rect.min().x as i32 + 2, rect.min().y as i32 + 2)
    }

    fn tap(x: i32, y: i32, include_hover: bool) -> Vec<GameEvent> {
        let mut events = Vec::new();
        if include_hover {
            events.push(GameEvent::MouseMove {
                x,
                y,
                xrel: 0,
                yrel: 0,
            });
        }
        events.push(GameEvent::MouseDown(x, y, 1, 1));
        events.push(GameEvent::MouseUp(x, y, 1));
        events
    }

    #[test]
    fn localized_trade_templates_replace_placeholders_in_order() {
        assert_eq!(
            substitute("Sold %u %s for £%u", &["5", "nets", "35"]),
            "Sold 5 nets for £35"
        );
    }

    #[test]
    fn every_inventory_row_fits_above_the_status_area() {
        const STATUS_Y: i32 = 342;
        assert!(ROW_Y + ROW_H * TRADE_ITEMS.len() as i32 <= STATUS_Y);
    }

    fn trading_state(stock: u16) -> (IngameMenuResources, TradingModalState) {
        let resources = IngameMenuResources::stub();
        let sectors: Vec<_> = TRADE_ITEMS
            .iter()
            .map(|item| {
                let mut sector = SectorProduction::new(item.prod_type);
                sector.amount = stock;
                sector
            })
            .collect();
        let state = TradingModalState::build(&resources, &sectors, 100, ModalInputState::new());
        (resources, state)
    }

    #[test]
    fn sell_one_and_sell_five_each_require_exact_second_activation() {
        for quantity in [TradeQuantity::One, TradeQuantity::Five] {
            let (resources, mut state) = trading_state(10);
            assert_eq!(state.request_sale(&resources, quantity), None);
            assert_eq!(
                state.request_sale(&resources, quantity),
                Some(TradingOutcome::Sell {
                    prod_type: Type::MakeArrow,
                    quantity,
                })
            );
        }
    }

    #[test]
    fn sell_five_never_arms_when_only_one_item_is_available() {
        let (resources, mut state) = trading_state(1);
        assert_eq!(state.request_sale(&resources, TradeQuantity::Five), None);
        assert_eq!(state.pending_confirmation, None);
        assert!(state.awaiting_receipt.is_none());
    }

    #[test]
    fn complete_pointer_tap_in_one_poll_activates_buttons_exactly_once() {
        let (resources, mut state) = trading_state(10);
        let (sell_x, sell_y) = button_point(&state, ID_SELL_ONE);
        let transform = MenuTransform::centered(640, 480);

        assert_eq!(
            state.handle_events(&resources, &tap(sell_x, sell_y, true), transform),
            None,
            "first tap arms the exact sale"
        );
        assert_eq!(
            state.handle_events(&resources, &tap(sell_x, sell_y, false), transform),
            Some(TradingOutcome::Sell {
                prod_type: Type::MakeArrow,
                quantity: TradeQuantity::One,
            }),
            "second fast tap confirms once"
        );

        let (_, mut close_state) = trading_state(10);
        let (close_x, close_y) = button_point(&close_state, ID_CLOSE);
        assert_eq!(
            close_state.handle_events(&resources, &tap(close_x, close_y, true), transform,),
            Some(TradingOutcome::Close)
        );
    }

    #[test]
    fn only_the_exact_correlated_receipt_can_unlock_or_mutate_a_sale() {
        let (resources, mut state) = trading_state(10);
        let receipt = |request_id, remaining_stock, ransom_after| TradeReceipt {
            request_id,
            prod_type: Type::MakeArrow,
            quantity: TradeQuantity::One,
            outcome: TradeOutcome::Sold {
                units: 1,
                unit_price: 1,
                total_price: 1,
                remaining_stock,
                ransom_after,
            },
        };

        state.apply_receipt(&resources, receipt(1, 0, 999));
        assert_eq!(state.rows[0].stock, 10);
        assert_eq!(state.ransom, 100);

        assert_eq!(state.request_sale(&resources, TradeQuantity::One), None);
        assert!(state.request_sale(&resources, TradeQuantity::One).is_some());
        state.assign_request_id(42, Type::MakeArrow, TradeQuantity::One);

        state.apply_receipt(&resources, receipt(41, 0, 999));
        assert_eq!(state.rows[0].stock, 10);
        assert_eq!(state.ransom, 100);
        assert!(state.awaiting_receipt.is_some());

        state.apply_receipt(&resources, receipt(42, 9, 101));
        assert_eq!(state.rows[0].stock, 9);
        assert_eq!(state.ransom, 101);
        assert!(state.awaiting_receipt.is_none());
    }
}

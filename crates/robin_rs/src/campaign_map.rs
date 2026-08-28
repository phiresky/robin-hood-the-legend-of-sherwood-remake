//! Campaign map screen — picks a mission and returns it.
//!
//! A blocking modal that draws the DEFAULT.RES campaign map and waits
//! for a location selection.

use crate::campaign_progress::{CampaignProgressGraph, ExhibitGridNavigator, MissionProgressState};
use crate::gfx_types::{GameEvent, Keycode};
use crate::ingame_menu::blazon_set;
use crate::ingame_menu::layout::{self, MenuTransform, TextAlign};
use crate::ingame_menu::resources::{IngameMenuResources, MenuSurface};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::menu::{CampaignMapState, LOCATION_POSITIONS, mission_location_from_index};
use crate::native_font::{self, NativeFont};
use crate::renderer::Renderer;
use crate::ui::UiState;
use crate::ui_screens::MissionDescriptionScreen;
use crate::widget::FrameWnd;
use robin_assets::res_descr::{self, LevelDescriptors};
use robin_assets::resource_manager::ResourceManager;
use robin_assets::shipping_datadir as assets_shipping_datadir;
use robin_engine::campaign::Campaign;
use robin_engine::coordinates::ScreenBBox;
use robin_engine::gameplay_config::CampaignPresentationMode;
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::MissionLocation;
use robin_engine::resource_ids;
use robin_engine::widget_state as engine_widget_state;

const MAP_W: i32 = 640;
const MAP_H: i32 = 480;
const CLOSE_WIDGET_ID: u32 = 10_000;
const MAP_BACKGROUND_WIDGET_ID: u32 = 10_001;
const STATUS_WIDGET_ID: u32 = 10_002;
const SHORT_DESC_BG_WIDGET_ID: u32 = 10_003;
const SHORT_DESC_LIFETIME_WIDGET_ID: u32 = 10_004;
const SHORT_DESC_TEXT_WIDGET_ID: u32 = 10_005;
const ATTACK_WIDGET_ID_BASE: u32 = 10_100;
const FLAG_WIDGET_ID_BASE: u32 = 10_200;
const BLAZON_WIDGET_ID_BASE: u32 = 10_300;

const LOCATION_RESOURCE_IDS: [i32; 10] = [
    0,
    resource_ids::RHID_CROSS_1,
    resource_ids::RHID_CROSS_2,
    resource_ids::RHID_CROSS_3,
    resource_ids::RHID_DERBY,
    resource_ids::RHID_LEICESTER,
    resource_ids::RHID_LINCOLN,
    resource_ids::RHID_NOTTINGHAM,
    0,
    resource_ids::RHID_YORK,
];

const BLAZON_POSITIONS: [(i32, i32); 10] = [
    (0, 0),
    (220, 178),
    (246, 331),
    (355, 170),
    (102, 260),
    (443, 369),
    (474, 106),
    (349, 299),
    (0, 0),
    (171, 89),
];

const FLAG_POSITIONS: [(i32, i32); 10] = [
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (109, 176),
    (452, 316),
    (486, 33),
    (319, 217),
    (0, 0),
    (173, 33),
];

const ATTACK_POSITIONS: [(i32, i32); 10] = [
    (0, 0),
    (144, 173),
    (493, 159),
    (255, 26),
    (141, 322),
    (65, 106),
    (77, 24),
    (250, 123),
    (179, 115),
    (0, 0),
];

/// What the player chose from the campaign map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CampaignMapChoice {
    SelectMission(usize),
    PseudoDebriefTimer,
    Quit,
    /// A reshow was requested while the modal was open (e.g. from the
    /// in-game menu's options-resolution-change path), so the campaign
    /// map needs to be torn down and re-opened at the new size.  The
    /// caller should leave `campaign_map_active = true` and re-enter
    /// `show_campaign_map` on the next frame.
    Redisplay,
}

#[derive(Debug, Clone)]
struct CampaignMapItem {
    loc_idx: usize,
    mission_idx: usize,
    location: MissionLocation,
    name: String,
    description: String,
    remaining_lifetime: u32,
    show_blazons: bool,
}

#[derive(Default)]
struct CampaignMapAssets {
    background: Option<MenuSurface>,
    locations: [Option<MenuSurface>; 10],
    mini_blazon: Option<MenuSurface>,
    maxi_blazon: Option<MenuSurface>,
    flag: Option<MenuSurface>,
    attacks: [Option<MenuSurface>; 10],
    close: Option<MenuSurface>,
    tooltip_bg: Option<MenuSurface>,
    lifetime: [Option<MenuSurface>; 5],
    font: Option<NativeFont>,
}

struct ShortMissionDescriptionWindow {
    frame: FrameWnd,
    x: i32,
    y: i32,
    blazons: Option<engine_widget_state::blazon_set::BlazonSetState>,
}

impl CampaignMapAssets {
    fn load(renderer: &mut Renderer, resources: Option<&mut IngameMenuResources>) -> Self {
        let Some(resources) = resources else {
            return Self {
                font: load_campaign_font(),
                ..Self::default()
            };
        };
        Self {
            background: resources.default_picture(renderer, resource_ids::RHID_CAMPAIGN_MAP),
            locations: std::array::from_fn(|i| {
                let id = LOCATION_RESOURCE_IDS[i];
                (id != 0)
                    .then(|| resources.default_picture(renderer, id))
                    .flatten()
            }),
            mini_blazon: resources.default_picture(renderer, resource_ids::RHID_MINI_BLAZON),
            maxi_blazon: resources.default_picture(renderer, resource_ids::RHID_MAXI_BLAZON),
            flag: resources.default_picture(renderer, resource_ids::RHID_RICHARD_FLAG),
            close: resources.default_picture(renderer, resource_ids::RHID_CAMPAIGN_MAP_CLOSE),
            tooltip_bg: resources
                .default_picture(renderer, resource_ids::RHID_SHORT_MISSION_DESCRIPTION),
            lifetime: std::array::from_fn(|i| {
                resources.default_picture_sub(renderer, resource_ids::RHID_MISSION_LIFETIME, i)
            }),
            attacks: std::array::from_fn(|i| {
                let id = match i {
                    0 => resource_ids::RHID_ATTACK_0,
                    1 => resource_ids::RHID_ATTACK_1,
                    2 => resource_ids::RHID_ATTACK_2,
                    3 => resource_ids::RHID_ATTACK_3,
                    4 => resource_ids::RHID_ATTACK_4,
                    5 => resource_ids::RHID_ATTACK_5,
                    6 => resource_ids::RHID_ATTACK_6,
                    7 => resource_ids::RHID_ATTACK_7,
                    8 => resource_ids::RHID_ATTACK_8,
                    9 => resource_ids::RHID_ATTACK_9,
                    _ => 0,
                };
                resources.default_picture(renderer, id)
            }),
            font: load_campaign_font(),
        }
    }
}

/// Persistent campaign-map presentation state.
///
/// The mission driver owns this value and advances it once per outer frame.
/// Keeping the event loop outside the modal lets network ingress, replay, and
/// HTTP automation continue to run while the map is open.
pub(crate) struct CampaignMapModalState {
    items: Vec<CampaignMapItem>,
    assets: CampaignMapAssets,
    frame: FrameWnd,
    input: ModalInputState,
    pseudo_debrief_at_ms: Option<u32>,
    selected: usize,
}

impl CampaignMapModalState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        renderer: &mut Renderer,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        campaign_map: &CampaignMapState,
        menu_resources: Option<&mut IngameMenuResources>,
        text_resources: &mut ResourceManager,
        shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
        pseudo_debrief_pending: bool,
    ) -> Self {
        let items = campaign_map_items(campaign, profiles, campaign_map, text_resources, shipping);
        if items.is_empty() {
            tracing::warn!("No missions on campaign map — this shouldn't happen");
        }

        tracing::info!("Campaign map open ({} missions)", items.len());
        for (i, item) in items.iter().enumerate() {
            tracing::info!("  [{i}] {:?}: {}", item.location, item.name);
        }

        let assets = CampaignMapAssets::load(renderer, menu_resources);
        let frame = build_campaign_frame(&items, campaign_map, &assets);
        let pseudo_debrief_at_ms =
            pseudo_debrief_pending.then(|| crate::window::process_uptime_ms().saturating_add(500));
        Self {
            items,
            assets,
            frame,
            input: ModalInputState::new(),
            pseudo_debrief_at_ms,
            selected: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tick(
        &mut self,
        window: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        game: &mut crate::game::Game,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        campaign_map: &CampaignMapState,
        menu_resources: Option<&IngameMenuResources>,
        cursor: Option<ModalCursor<'_>>,
    ) -> Option<CampaignMapChoice> {
        if game.take_campaign_map_redisplay() {
            return Some(CampaignMapChoice::Redisplay);
        }

        let (events, transform) = layout::poll_events_with_transform(window, renderer);
        let mut final_choice = None;
        let input_enabled = self
            .pseudo_debrief_at_ms
            .map(|at| crate::window::process_uptime_ms() >= at)
            .unwrap_or(true);

        for event in events {
            if !input_enabled {
                self.input.update_from_event(&event, transform);
                continue;
            }
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => {
                    final_choice = Some(CampaignMapChoice::Quit);
                    break;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } if !self.items.is_empty() => self.selected = self.selected.saturating_sub(1),
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } if self.selected + 1 < self.items.len() => self.selected += 1,
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::Space,
                    ..
                } if !self.items.is_empty() => {
                    final_choice = Some(CampaignMapChoice::SelectMission(
                        self.items[self.selected].mission_idx,
                    ));
                    break;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::Space,
                    ..
                } if self.items.is_empty() => {
                    final_choice = Some(CampaignMapChoice::Quit);
                    break;
                }
                GameEvent::MouseMove { x, y, .. } => {
                    let (vx, vy) = transform.from_screen(x, y);
                    self.input.virt_x = vx as f32;
                    self.input.virt_y = vy as f32;
                }
                GameEvent::MouseDown(x, y, 1, _) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    self.input.virt_x = vx as f32;
                    self.input.virt_y = vy as f32;
                }
                _ => {}
            }
            self.input.update_from_event(&event, transform);
            let widget_input = self.input.as_widget_input();
            let events = self.frame.process_input(&widget_input);
            self.input.end_frame();

            for (idx, item) in self.items.iter().enumerate() {
                if self
                    .frame
                    .widget(item.loc_idx as u32)
                    .is_some_and(|w| w.base().state != UiState::Default)
                {
                    self.selected = idx;
                }
            }

            if let Some(id) = widget_bridge::find_activated(&events) {
                if id == CLOSE_WIDGET_ID {
                    final_choice = Some(CampaignMapChoice::Quit);
                    break;
                }
                if let Some(item) = self.items.iter().find(|item| item.loc_idx as u32 == id) {
                    final_choice = Some(CampaignMapChoice::SelectMission(item.mission_idx));
                    break;
                }
            }
        }

        layout::enter_modal_gpu_phase(renderer);
        render_campaign_map(
            renderer,
            transform,
            campaign,
            profiles,
            campaign_map,
            &self.items,
            self.selected,
            &self.assets,
            menu_resources,
            &self.input,
            &self.frame,
        );
        if let Some(cursor) = &cursor {
            cursor.draw(renderer, transform, &self.input);
        }
        renderer.present();

        if final_choice.is_some() {
            return final_choice;
        }
        self.pseudo_debrief_at_ms
            .is_some_and(|at| crate::window::process_uptime_ms() >= at)
            .then_some(CampaignMapChoice::PseudoDebriefTimer)
    }
}

#[allow(clippy::too_many_arguments)]
async fn show_campaign_progress(
    window: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    game: &mut crate::game::Game,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    assets: &CampaignMapAssets,
    cursor: Option<&ModalCursor<'_>>,
    pseudo_debrief_pending: bool,
    initial_presentation: CampaignPresentationMode,
    lifetime_history: &robin_engine::campaign_history::ProfileCampaignHistory,
) -> Result<CampaignMapChoice, String> {
    let graph = CampaignProgressGraph::build(campaign, profiles, Some(lifetime_history));
    if graph.nodes.is_empty() {
        tracing::warn!("Campaign history presentation has no non-Sherwood missions");
        return Ok(CampaignMapChoice::Quit);
    }
    let mut presentation = initial_presentation;
    let mut selected = graph.first_selectable().unwrap_or(0);
    let mut exhibit_grid = ExhibitGridNavigator::new(graph.nodes.len(), selected);
    let mut input = ModalInputState::new();
    let pseudo_debrief_at = pseudo_debrief_pending
        .then(|| std::time::Instant::now() + std::time::Duration::from_millis(500));

    loop {
        if game.take_campaign_map_redisplay() {
            return Ok(CampaignMapChoice::Redisplay);
        }
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let input_enabled = pseudo_debrief_at
            .map(|at| std::time::Instant::now() >= at)
            .unwrap_or(true);
        let mut final_choice = None;
        for event in window.poll_events() {
            if !input_enabled {
                input.update_from_event(&event, transform);
                continue;
            }
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => final_choice = Some(CampaignMapChoice::Quit),
                GameEvent::KeyDown {
                    keycode: Keycode::Tab,
                    ..
                } => {
                    presentation = match presentation {
                        CampaignPresentationMode::ProgressTree => {
                            exhibit_grid = ExhibitGridNavigator::new(graph.nodes.len(), selected);
                            CampaignPresentationMode::SherwoodMuseum
                        }
                        CampaignPresentationMode::SherwoodMuseum => {
                            CampaignPresentationMode::ProgressTree
                        }
                        CampaignPresentationMode::ClassicMap => unreachable!(),
                    };
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => match presentation {
                    CampaignPresentationMode::ProgressTree => selected = selected.saturating_sub(1),
                    CampaignPresentationMode::SherwoodMuseum => {
                        exhibit_grid.navigate(0, -1);
                        selected = exhibit_grid.selected;
                    }
                    CampaignPresentationMode::ClassicMap => unreachable!(),
                },
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => match presentation {
                    CampaignPresentationMode::ProgressTree => {
                        selected = (selected + 1).min(graph.nodes.len() - 1)
                    }
                    CampaignPresentationMode::SherwoodMuseum => {
                        exhibit_grid.navigate(0, 1);
                        selected = exhibit_grid.selected;
                    }
                    CampaignPresentationMode::ClassicMap => unreachable!(),
                },
                GameEvent::KeyDown {
                    keycode: Keycode::Left,
                    ..
                } if presentation == CampaignPresentationMode::SherwoodMuseum => {
                    exhibit_grid.navigate(-1, 0);
                    selected = exhibit_grid.selected;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Right,
                    ..
                } if presentation == CampaignPresentationMode::SherwoodMuseum => {
                    exhibit_grid.navigate(1, 0);
                    selected = exhibit_grid.selected;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter | Keycode::Space,
                    ..
                } if graph.nodes[selected].selectable => {
                    final_choice = Some(CampaignMapChoice::SelectMission(
                        graph.nodes[selected].mission_idx,
                    ));
                }
                GameEvent::MouseMove { x, y, .. } => {
                    let (vx, vy) = transform.from_screen(x, y);
                    input.virt_x = vx as f32;
                    input.virt_y = vy as f32;
                }
                GameEvent::MouseDown(x, y, 1, _) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    input.virt_x = vx as f32;
                    input.virt_y = vy as f32;
                    if let Some(index) = progress_hit_test(&graph, presentation, selected, vx, vy) {
                        selected = index;
                        exhibit_grid = ExhibitGridNavigator::new(graph.nodes.len(), selected);
                        if graph.nodes[index].selectable {
                            final_choice = Some(CampaignMapChoice::SelectMission(
                                graph.nodes[index].mission_idx,
                            ));
                        }
                    }
                }
                _ => {}
            }
            input.update_from_event(&event, transform);
            if final_choice.is_some() {
                break;
            }
        }

        layout::enter_modal_gpu_phase(renderer);
        render_campaign_progress(
            renderer,
            transform,
            &graph,
            selected,
            presentation,
            assets,
            lifetime_history.totals(),
        );
        if let Some(cursor) = cursor {
            cursor.draw(renderer, transform, &input);
        }
        renderer.present();
        crate::window::sleep_ms(16).await;

        if let Some(choice) = final_choice {
            return Ok(choice);
        }
        if let Some(at) = pseudo_debrief_at
            && std::time::Instant::now() >= at
        {
            return Ok(CampaignMapChoice::PseudoDebriefTimer);
        }
    }
}

fn progress_hit_test(
    graph: &CampaignProgressGraph,
    presentation: CampaignPresentationMode,
    selected: usize,
    x: i32,
    y: i32,
) -> Option<usize> {
    graph.nodes.iter().enumerate().find_map(|(idx, _)| {
        if presentation == CampaignPresentationMode::SherwoodMuseum && idx / 16 != selected / 16 {
            return None;
        }
        let (rx, ry, rw, rh) = progress_node_rect(graph, presentation, idx);
        (x >= rx && x < rx + rw && y >= ry && y < ry + rh).then_some(idx)
    })
}

fn progress_node_rect(
    graph: &CampaignProgressGraph,
    presentation: CampaignPresentationMode,
    index: usize,
) -> (i32, i32, i32, i32) {
    match presentation {
        CampaignPresentationMode::ProgressTree => {
            let node = &graph.nodes[index];
            let max_depth = graph.nodes.iter().map(|node| node.depth).max().unwrap_or(0);
            let x_step = if max_depth == 0 {
                0
            } else {
                480 / max_depth as i32
            };
            let lane_count = graph
                .nodes
                .iter()
                .filter(|candidate| candidate.depth == node.depth)
                .count()
                .max(1);
            let y_step = 310 / lane_count as i32;
            (
                30 + node.depth as i32 * x_step,
                70 + node.lane as i32 * y_step,
                112,
                38,
            )
        }
        CampaignPresentationMode::SherwoodMuseum => {
            let page_index = index % 16;
            let col = page_index % 4;
            let row = page_index / 4;
            (35 + col as i32 * 150, 72 + row as i32 * 84, 126, 58)
        }
        CampaignPresentationMode::ClassicMap => unreachable!(),
    }
}

fn render_campaign_progress(
    renderer: &mut Renderer,
    transform: MenuTransform,
    graph: &CampaignProgressGraph,
    selected: usize,
    presentation: CampaignPresentationMode,
    assets: &CampaignMapAssets,
    lifetime_totals: robin_engine::campaign_history::CampaignHistoryTotals,
) {
    if let Some(background) = assets.background.as_ref() {
        layout::draw_background(renderer, transform, background, 0, 0, MAP_W, MAP_H);
    } else {
        renderer.render_gpu_rect(
            transform.origin_x,
            transform.origin_y,
            MAP_W,
            MAP_H,
            38,
            31,
            20,
            255,
        );
    }
    // The translucent wash leaves the original campaign artwork visible as
    // the museum/tree room while keeping graph text readable.
    renderer.render_gpu_rect(
        transform.origin_x + 10,
        transform.origin_y + 10,
        620,
        455,
        18,
        23,
        25,
        215,
    );

    let color_for = |state: MissionProgressState| match state {
        MissionProgressState::Completed => (40, 105, 62),
        MissionProgressState::Available => (132, 102, 30),
        MissionProgressState::Lost => (125, 45, 38),
        MissionProgressState::Expired => (78, 55, 55),
        MissionProgressState::Locked => (58, 61, 65),
    };

    if presentation == CampaignPresentationMode::ProgressTree {
        for (idx, node) in graph.nodes.iter().enumerate() {
            let (x, y, _w, h) = progress_node_rect(graph, presentation, idx);
            for &parent in &node.prerequisite_nodes {
                let (px, py, pw, ph) = progress_node_rect(graph, presentation, parent);
                renderer.render_gpu_line(
                    transform.origin_x + px + pw,
                    transform.origin_y + py + ph / 2,
                    transform.origin_x + x,
                    transform.origin_y + y + h / 2,
                    155,
                    137,
                    91,
                );
            }
        }
    }

    for (idx, node) in graph.nodes.iter().enumerate() {
        if presentation == CampaignPresentationMode::SherwoodMuseum && idx / 16 != selected / 16 {
            continue;
        }
        let (x, y, w, h) = progress_node_rect(graph, presentation, idx);
        let (r, g, b) = color_for(node.state);
        renderer.render_gpu_rect(
            transform.origin_x + x,
            transform.origin_y + y,
            w,
            h,
            r,
            g,
            b,
            245,
        );
        if idx == selected {
            renderer.draw_rect_outline_screen(
                transform.origin_x + x - 3,
                transform.origin_y + y - 3,
                transform.origin_x + x + w + 3,
                transform.origin_y + y + h + 3,
                Renderer::create_color_16(255, 230, 95),
            );
        }

        if presentation == CampaignPresentationMode::SherwoodMuseum {
            let loc_idx = node.location as usize;
            if let Some(surface) = assets.locations.get(loc_idx).copied().flatten() {
                layout::draw_background(renderer, transform, &surface, x + 4, y + 4, 22, 22);
            }
        }
        if let Some(font) = assets.font.as_ref() {
            let max_chars = if presentation == CampaignPresentationMode::SherwoodMuseum {
                15
            } else {
                13
            };
            let name: String = node.name.chars().take(max_chars).collect();
            layout::render_text_virt(renderer, font, transform, &name, x + 5, y + h - 17);
            if node.attempt_count != 0 {
                layout::render_text_virt(
                    renderer,
                    font,
                    transform,
                    &format!("{}x  {} badge", node.attempt_count, node.badge_count),
                    x + 31,
                    y + 5,
                );
            }
        }
    }

    // Existing flag art marks the selected exhibit in the modal grid.
    if presentation == CampaignPresentationMode::SherwoodMuseum
        && let Some(flag) = assets.flag.as_ref()
    {
        let (x, y, _, h) = progress_node_rect(graph, presentation, selected);
        layout::draw_background(renderer, transform, flag, x + 52, y + h + 2, 20, 24);
    }

    if let Some(font) = assets.font.as_ref() {
        let title = match presentation {
            CampaignPresentationMode::ProgressTree => "Campaign Progress Tree".to_owned(),
            CampaignPresentationMode::SherwoodMuseum => format!(
                "Sherwood Hall of Deeds - Gallery {}/{}",
                selected / 16 + 1,
                (graph.nodes.len() + 15) / 16
            ),
            CampaignPresentationMode::ClassicMap => unreachable!(),
        };
        layout::render_text_virt(renderer, font, transform, &title, 25, 22);
        layout::render_text_virt(
            renderer,
            font,
            transform,
            &format!(
                "{} / {} missions completed    Tab: switch view    Esc: leave",
                graph.completed_missions, graph.known_missions
            ),
            25,
            43,
        );
        layout::render_text_virt(
            renderer,
            font,
            transform,
            &format!(
                "Lifetime: {} attempts / {} wins / {}h {:02}m",
                lifetime_totals.attempts,
                lifetime_totals.wins,
                lifetime_totals.known_duration_seconds / 3600,
                (lifetime_totals.known_duration_seconds / 60) % 60,
            ),
            315,
            43,
        );
        let node = &graph.nodes[selected];
        let action = if node.history_replay {
            "Enter: replay (campaign changes discarded)"
        } else if node.selectable {
            "Enter: inspect and launch"
        } else {
            "Locked: inspect only"
        };
        layout::render_text_virt(renderer, font, transform, action, 25, 414);
        layout::render_text_virt(renderer, font, transform, &node.summary(), 25, 437);
        if graph.cyclic_prerequisites {
            layout::render_text_virt(
                renderer,
                font,
                transform,
                "Warning: cyclic mission prerequisites detected",
                355,
                22,
            );
        }
    }
}

fn campaign_map_items(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    campaign_map: &CampaignMapState,
    text_resources: &mut ResourceManager,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
) -> Vec<CampaignMapItem> {
    campaign_map
        .locations
        .iter()
        .enumerate()
        .filter_map(|(loc_idx, loc)| {
            if !loc.enabled {
                return None;
            }
            let mission_idx = loc.mission_idx?;
            let mission = campaign.missions.get(mission_idx)?;
            let profile = mission.profile(profiles);
            let name = profile.mission_name.clone();
            let location = mission_location_from_index(loc_idx).unwrap_or(MissionLocation::Nowhere);
            if matches!(
                location,
                MissionLocation::Nowhere | MissionLocation::Sherwood
            ) {
                return None;
            }
            let descriptors = load_level_descriptors(profile.id, shipping);
            let description =
                MissionDescriptionScreen::get_mission_text(descriptors.as_ref(), text_resources, 1);
            let remaining_lifetime = u32::from(profile.life_time)
                .saturating_sub(u32::from(mission.age))
                .saturating_sub(1);
            Some(CampaignMapItem {
                loc_idx,
                mission_idx,
                location,
                name,
                description,
                remaining_lifetime,
                show_blazons: mission.requires_blazons(profiles),
            })
        })
        .collect()
}

fn load_level_descriptors(
    mission_id: u32,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
) -> Option<LevelDescriptors> {
    let filename = res_descr::red_filename(mission_id);
    shipping
        .and_then(|dd| dd.red_files.get(&filename).cloned())
        .or_else(|| {
            let path = format!("Data/Text/{filename}");
            res_descr::load(&path).ok()
        })
}

fn build_campaign_frame(
    items: &[CampaignMapItem],
    campaign_map: &CampaignMapState,
    assets: &CampaignMapAssets,
) -> FrameWnd {
    let mut frame = FrameWnd::new(
        "Campaign Map",
        ScreenBBox::from_coords(0.0, 0.0, 629.0, 480.0),
        0,
    );
    frame.set_frame_id(resource_ids::RHID_CAMPAIGN_MAP as u32);
    frame.add_widget_absolute(widget_bridge::make_picture_with_resource(
        MAP_BACKGROUND_WIDGET_ID,
        resource_ids::RHID_CAMPAIGN_MAP,
        0,
        0,
        MAP_W,
        MAP_H,
    ));
    frame.add_widget_absolute(widget_bridge::make_label(
        STATUS_WIDGET_ID,
        &campaign_map.status_text,
        100,
        460,
        440,
        20,
    ));

    for (i, visible) in campaign_map
        .attack_arrows_visible
        .iter()
        .copied()
        .enumerate()
    {
        if visible {
            let (x, y) = ATTACK_POSITIONS[i];
            let Some(surface) = assets.attacks[i] else {
                continue;
            };
            frame.add_widget_absolute(widget_bridge::make_picture_with_resource(
                ATTACK_WIDGET_ID_BASE + i as u32,
                attack_resource_id(i),
                x,
                y,
                surface.width,
                surface.height,
            ));
        }
    }

    for (loc_idx, loc) in campaign_map.locations.iter().enumerate() {
        if loc.show_flag
            && let Some(surface) = assets.flag
        {
            let (x, y) = FLAG_POSITIONS[loc_idx];
            frame.add_widget_absolute(widget_bridge::make_picture_with_resource(
                FLAG_WIDGET_ID_BASE + loc_idx as u32,
                resource_ids::RHID_RICHARD_FLAG,
                x,
                y,
                surface.width,
                surface.height,
            ));
        }
        if loc.show_blazon {
            let (resource_id, surface) = if matches!(
                mission_location_from_index(loc_idx),
                Some(MissionLocation::Cross1 | MissionLocation::Cross2 | MissionLocation::Cross3)
            ) {
                (resource_ids::RHID_MINI_BLAZON, assets.mini_blazon)
            } else {
                (resource_ids::RHID_MAXI_BLAZON, assets.maxi_blazon)
            };
            if let Some(surface) = surface {
                let (x, y) = BLAZON_POSITIONS[loc_idx];
                frame.add_widget_absolute(widget_bridge::make_picture_with_resource(
                    BLAZON_WIDGET_ID_BASE + loc_idx as u32,
                    resource_id,
                    x,
                    y,
                    surface.width,
                    surface.height,
                ));
            }
        }
    }

    for item in items {
        let (x, y) = LOCATION_POSITIONS[item.loc_idx];
        let (w, h) = assets.locations[item.loc_idx]
            .map(|s| (s.width.max(18), s.height.max(18)))
            .unwrap_or((24, 24));
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            item.loc_idx as u32,
            "",
            true,
            LOCATION_RESOURCE_IDS[item.loc_idx],
            x as i32,
            y as i32,
            w,
            h,
        ));
    }

    if items.is_empty() {
        let (w, h) = assets
            .close
            .map(|s| (s.width.max(21), s.height.max(21)))
            .unwrap_or((21, 21));
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            CLOSE_WIDGET_ID,
            "",
            true,
            resource_ids::RHID_CAMPAIGN_MAP_CLOSE,
            574,
            5,
            w,
            h,
        ));
    }

    frame
}

#[allow(clippy::too_many_arguments)]
fn render_campaign_map(
    renderer: &mut Renderer,
    transform: MenuTransform,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    campaign_map: &CampaignMapState,
    items: &[CampaignMapItem],
    selected: usize,
    assets: &CampaignMapAssets,
    resources: Option<&IngameMenuResources>,
    input: &ModalInputState,
    frame: &FrameWnd,
) {
    if assets.background.is_none() {
        renderer.render_gpu_rect(
            transform.origin_x,
            transform.origin_y,
            MAP_W,
            MAP_H,
            52,
            43,
            27,
            255,
        );
        renderer.draw_rect_outline_screen(
            transform.origin_x,
            transform.origin_y,
            transform.origin_x + MAP_W,
            transform.origin_y + MAP_H,
            Renderer::create_color_16(180, 150, 90),
        );
    }

    widget_bridge::draw_frame_bitmap_widgets(renderer, transform, frame, |resource_id, sub_id| {
        campaign_surface_for_resource(assets, resource_id, sub_id)
    });

    for (loc_idx, loc) in campaign_map.locations.iter().enumerate() {
        if loc.enabled {
            let (x, y) = LOCATION_POSITIONS[loc_idx];
            if let Some(surface) = assets.locations[loc_idx] {
                let focused = frame
                    .widget(loc_idx as u32)
                    .is_some_and(|w| w.base().state != UiState::Default);
                if focused {
                    renderer.draw_rect_outline_screen(
                        transform.origin_x + x as i32 - 4,
                        transform.origin_y + y as i32 - 4,
                        transform.origin_x + x as i32 + surface.width + 4,
                        transform.origin_y + y as i32 + surface.height + 4,
                        Renderer::create_color_16(255, 230, 90),
                    );
                }
                if loc.blinking && blink_on() {
                    renderer.draw_rect_outline_screen(
                        transform.origin_x + x as i32 - 3,
                        transform.origin_y + y as i32 - 3,
                        transform.origin_x + x as i32 + surface.width + 3,
                        transform.origin_y + y as i32 + surface.height + 3,
                        Renderer::create_color_16(255, 240, 120),
                    );
                }
            } else {
                draw_marker(renderer, transform, x as i32, y as i32, loc.blinking);
            }
        }
    }

    if let Some(item) = items.get(selected) {
        draw_selection(renderer, transform, item.loc_idx, &assets.locations);
        render_tooltip(
            renderer, transform, campaign, profiles, item, assets, resources, input,
        );
    } else {
        draw_close_button(renderer, transform, assets, frame);
    }

    if let Some(font) = assets.font.as_ref() {
        widget_bridge::draw_frame_labels(renderer, transform, frame, font, TextAlign::Center);
        layout::render_text_virt(renderer, font, transform, "Tab: History & Practice", 18, 12);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_tooltip(
    renderer: &mut Renderer,
    transform: MenuTransform,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    item: &CampaignMapItem,
    assets: &CampaignMapAssets,
    resources: Option<&IngameMenuResources>,
    input: &ModalInputState,
) {
    let Some(font) = resources
        .and_then(|r| r.fonts.popup_scroll.as_ref())
        .or(assets.font.as_ref())
    else {
        return;
    };

    let short_desc = ShortMissionDescriptionWindow::new(campaign, profiles, item, input, assets);

    if assets.tooltip_bg.is_none() {
        renderer.render_gpu_rect(
            transform.origin_x + short_desc.x,
            transform.origin_y + short_desc.y,
            220,
            100,
            42,
            32,
            18,
            235,
        );
        renderer.draw_rect_outline_screen(
            transform.origin_x + short_desc.x,
            transform.origin_y + short_desc.y,
            transform.origin_x + short_desc.x + 220,
            transform.origin_y + short_desc.y + 100,
            Renderer::create_color_16(210, 180, 110),
        );
    }

    widget_bridge::draw_frame_bitmap_widgets(
        renderer,
        transform,
        &short_desc.frame,
        |resource_id, sub_id| match resource_id {
            resource_ids::RHID_SHORT_MISSION_DESCRIPTION => assets.tooltip_bg,
            resource_ids::RHID_MISSION_LIFETIME => assets.lifetime.get(sub_id as usize).copied()?,
            _ => None,
        },
    );
    widget_bridge::draw_frame_labels(
        renderer,
        transform,
        &short_desc.frame,
        font,
        TextAlign::Left,
    );

    if item.show_blazons
        && let Some(resources) = resources
        && let Some(state) = &short_desc.blazons
    {
        blazon_set::render(
            renderer,
            transform,
            resources,
            state,
            short_desc.x,
            short_desc.y,
        );
    }
}

impl ShortMissionDescriptionWindow {
    fn new(
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        item: &CampaignMapItem,
        input: &ModalInputState,
        assets: &CampaignMapAssets,
    ) -> Self {
        let mut x = input.virt_x as i32 + 25;
        let mut y = input.virt_y as i32 + 25;
        x = x.clamp(0, MAP_W - 220);
        y = y.clamp(0, MAP_H - 100);

        let mut frame = FrameWnd::new(
            "Short mission description",
            ScreenBBox::from_coords(x as f32, y as f32, (x + 220) as f32, (y + 100) as f32),
            0,
        );
        frame.add_widget_absolute(widget_bridge::make_picture_with_resource(
            SHORT_DESC_BG_WIDGET_ID,
            resource_ids::RHID_SHORT_MISSION_DESCRIPTION,
            x,
            y,
            220,
            100,
        ));
        let lifetime_idx = item.remaining_lifetime.min(4);
        let (life_w, life_h) = assets.lifetime[lifetime_idx as usize]
            .map(|s| (s.width, s.height))
            .unwrap_or((20, 20));
        frame.add_widget_absolute(widget_bridge::make_multi_picture_with_resource(
            SHORT_DESC_LIFETIME_WIDGET_ID,
            resource_ids::RHID_MISSION_LIFETIME,
            lifetime_idx,
            x + 9,
            y + 8,
            life_w,
            life_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_label(
            SHORT_DESC_TEXT_WIDGET_ID,
            &item.description,
            x + 48,
            y + 5,
            167,
            75,
        ));

        let blazons = item.show_blazons.then(|| {
            blazon_set::build_for_mission(campaign, profiles, item.mission_idx, 5, 80, 210, 15, 0)
        });

        Self {
            frame,
            x,
            y,
            blazons,
        }
    }
}

fn draw_close_button(
    renderer: &mut Renderer,
    transform: MenuTransform,
    assets: &CampaignMapAssets,
    frame: &FrameWnd,
) {
    let hovered = frame
        .widget(CLOSE_WIDGET_ID)
        .is_some_and(|w| w.base().state != UiState::Default);
    if assets.close.is_none() {
        renderer.render_gpu_rect(
            transform.origin_x + 574,
            transform.origin_y + 5,
            21,
            21,
            80,
            45,
            35,
            255,
        );
    }
    if hovered {
        renderer.draw_rect_outline_screen(
            transform.origin_x + 573,
            transform.origin_y + 4,
            transform.origin_x + 596,
            transform.origin_y + 27,
            Renderer::create_color_16(255, 230, 90),
        );
    }
}

fn attack_resource_id(index: usize) -> i32 {
    match index {
        0 => resource_ids::RHID_ATTACK_0,
        1 => resource_ids::RHID_ATTACK_1,
        2 => resource_ids::RHID_ATTACK_2,
        3 => resource_ids::RHID_ATTACK_3,
        4 => resource_ids::RHID_ATTACK_4,
        5 => resource_ids::RHID_ATTACK_5,
        6 => resource_ids::RHID_ATTACK_6,
        7 => resource_ids::RHID_ATTACK_7,
        8 => resource_ids::RHID_ATTACK_8,
        9 => resource_ids::RHID_ATTACK_9,
        _ => 0,
    }
}

fn campaign_surface_for_resource(
    assets: &CampaignMapAssets,
    resource_id: i32,
    sub_id: u8,
) -> Option<MenuSurface> {
    match resource_id {
        resource_ids::RHID_CAMPAIGN_MAP => assets.background,
        resource_ids::RHID_MINI_BLAZON => assets.mini_blazon,
        resource_ids::RHID_MAXI_BLAZON => assets.maxi_blazon,
        resource_ids::RHID_RICHARD_FLAG => assets.flag,
        resource_ids::RHID_CAMPAIGN_MAP_CLOSE => assets.close,
        resource_ids::RHID_MISSION_LIFETIME => assets.lifetime.get(sub_id as usize).copied()?,
        id => LOCATION_RESOURCE_IDS
            .iter()
            .position(|&loc_id| loc_id == id)
            .and_then(|idx| assets.locations[idx])
            .or_else(|| {
                (0..assets.attacks.len())
                    .find(|&idx| attack_resource_id(idx) == id)
                    .and_then(|idx| assets.attacks[idx])
            }),
    }
}

fn load_campaign_font() -> Option<NativeFont> {
    let config = native_font::load_font_config().ok()?;
    match native_font::load_font_by_name(&config, "Default").ok()? {
        native_font::Font::Native(font) => Some(font),
        native_font::Font::TrueType(_) => None,
    }
}

fn draw_marker(renderer: &mut Renderer, transform: MenuTransform, x: i32, y: i32, blinking: bool) {
    let sx = transform.origin_x + x;
    let sy = transform.origin_y + y;
    let color = if blinking {
        Renderer::create_color_16(255, 220, 80)
    } else {
        Renderer::create_color_16(220, 40, 40)
    };
    renderer.draw_line_screen(sx, sy - 8, sx + 8, sy, color);
    renderer.draw_line_screen(sx + 8, sy, sx, sy + 8, color);
    renderer.draw_line_screen(sx, sy + 8, sx - 8, sy, color);
    renderer.draw_line_screen(sx - 8, sy, sx, sy - 8, color);
    if blinking && blink_on() {
        renderer.draw_rect_outline_screen(
            sx - 11,
            sy - 11,
            sx + 11,
            sy + 11,
            Renderer::create_color_16(255, 240, 120),
        );
    }
}

fn draw_selection(
    renderer: &mut Renderer,
    transform: MenuTransform,
    loc_idx: usize,
    location_surfaces: &[Option<MenuSurface>; 10],
) {
    let (x, y) = LOCATION_POSITIONS[loc_idx];
    let (w, h) = location_surfaces[loc_idx]
        .map(|s| (s.width.max(18), s.height.max(18)))
        .unwrap_or((22, 22));
    let sx = transform.origin_x + x as i32 - 4;
    let sy = transform.origin_y + y as i32 - 4;
    renderer.draw_rect_outline_screen(
        sx,
        sy,
        sx + w + 8,
        sy + h + 8,
        Renderer::create_color_16(255, 230, 90),
    );
}

fn blink_on() -> bool {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    (ms / 350).is_multiple_of(2)
}

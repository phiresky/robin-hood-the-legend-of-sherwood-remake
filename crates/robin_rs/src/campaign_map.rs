//! Campaign map screen — picks a mission and returns it.
//!
//! A blocking modal that draws the DEFAULT.RES campaign map and waits
//! for a location selection.

use crate::campaign_progress::{
    CampaignProgressGraph, ExhibitGridNavigator, MissionKind, MissionProgressState,
};
use crate::gfx_types::{GameEvent, Keycode};
use crate::host::ApplicationContext;
use crate::ingame_menu::blazon_set;
use crate::ingame_menu::layout::{self, MenuTransform, TextAlign};
use crate::ingame_menu::resources::{IngameMenuResources, MenuSurface};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::menu::{CampaignMapState, LOCATION_POSITIONS, mission_location_from_index};
use crate::native_font::{self, Font};
use crate::renderer::Renderer;
use crate::ui::UiState;
use crate::ui_screens::MissionDescriptionScreen;
use crate::widget::FrameWnd;
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
    achievement_badges: Vec<crate::achievement_hud::AchievementBadgePresentation>,
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
    font: Option<Font>,
    progress_font: Option<Font>,
    progress_title_font: Option<Font>,
}

struct ShortMissionDescriptionWindow {
    frame: FrameWnd,
    x: i32,
    y: i32,
    blazons: Option<engine_widget_state::blazon_set::BlazonSetState>,
}

impl CampaignMapAssets {
    fn load(
        renderer: &mut Renderer,
        resources: Option<&mut IngameMenuResources>,
        files: &robin_engine::sbfile::SbFileSystem,
    ) -> Self {
        let Some(resources) = resources else {
            return Self {
                font: load_campaign_font(files),
                progress_font: load_progress_font(files, "MenuButtonEnabled"),
                progress_title_font: load_progress_font(files, "MissionTitle"),
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
            font: load_campaign_font(files),
            progress_font: load_progress_font(files, "MenuButtonEnabled"),
            progress_title_font: load_progress_font(files, "MissionTitle"),
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
    graph: CampaignProgressGraph,
    presentation: CampaignPresentationMode,
    exhibit_grid: ExhibitGridNavigator,
    input: ModalInputState,
    pseudo_debrief_at_ms: Option<u32>,
    selected_classic: usize,
    selected_progress: usize,
    show_achievement_badges: bool,
    achievement_overview: bool,
    details_scroll: Option<usize>,
    history_scroll: usize,
    selected_play: usize,
    replay_status: String,
    recording_index: std::sync::Arc<crate::mission_replays::RecordingIndex>,
    lifetime_totals: robin_engine::campaign_history::CampaignHistoryTotals,
    lifetime_achievements: robin_engine::achievement::AchievementAggregationSummary,
}

impl CampaignMapModalState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        application_context: &ApplicationContext,
        renderer: &mut Renderer,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        campaign_map: &CampaignMapState,
        menu_resources: Option<&mut IngameMenuResources>,
        text_resources: &mut ResourceManager,
        shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
        pseudo_debrief_pending: bool,
        presentation: CampaignPresentationMode,
        show_achievement_badges: bool,
        lifetime_history: &robin_engine::campaign_history::ProfileCampaignHistory,
    ) -> Self {
        let items = campaign_map_items(
            application_context,
            campaign,
            profiles,
            Some(lifetime_history),
            campaign_map,
            text_resources,
            shipping,
        );
        if items.is_empty() {
            tracing::warn!("No missions on campaign map — this shouldn't happen");
        }

        tracing::info!("Campaign map open ({} missions)", items.len());
        for (i, item) in items.iter().enumerate() {
            tracing::info!("  [{i}] {:?}: {}", item.location, item.name);
        }

        let assets = CampaignMapAssets::load(
            renderer,
            menu_resources,
            application_context
                .preparation_files()
                .expect("campaign-map presentation requires prepared resources"),
        );
        let frame = build_campaign_frame(&items, campaign_map, &assets);
        let mut graph = CampaignProgressGraph::build(campaign, profiles, Some(lifetime_history));
        for node in &mut graph.nodes {
            node.name = application_context.localized_mission_name(node.mission_id, &node.name);
        }
        load_campaign_descriptions(application_context, &mut graph, text_resources, shipping);
        if presentation != CampaignPresentationMode::ClassicMap && graph.nodes.is_empty() {
            tracing::warn!("Campaign history presentation has no non-Sherwood missions");
        }
        let recording_index = application_context.recording_index().clone();
        load_recording_links(&recording_index, &mut graph);
        if let Err(error) = recording_index.refresh_index() {
            tracing::warn!("Cannot refresh recording index: {error}");
        }
        let selected_progress = graph.first_selectable().unwrap_or(0);
        let exhibit_grid = ExhibitGridNavigator::new(graph.nodes.len(), selected_progress);
        let pseudo_debrief_at_ms =
            pseudo_debrief_pending.then(|| crate::window::process_uptime_ms().saturating_add(500));
        Self {
            items,
            assets,
            frame,
            graph,
            presentation,
            exhibit_grid,
            input: ModalInputState::new(),
            pseudo_debrief_at_ms,
            selected_classic: 0,
            selected_progress,
            show_achievement_badges,
            achievement_overview: false,
            details_scroll: None,
            history_scroll: 0,
            selected_play: 0,
            replay_status: String::new(),
            recording_index,
            lifetime_totals: lifetime_history.totals(),
            lifetime_achievements: lifetime_history.achievement_aggregation(),
        }
    }

    /// A local, read-only view, with no campaign-map flags or mission timers.
    pub(crate) fn new_browser(
        application_context: &ApplicationContext,
        renderer: &mut Renderer,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        resources: &mut IngameMenuResources,
    ) -> Self {
        let profile = application_context
            .active_profile_snapshot()
            .expect("campaign manager requires an active profile");
        let mut graph =
            CampaignProgressGraph::build(campaign, profiles, Some(&profile.campaign_history));
        for node in &mut graph.nodes {
            node.name = application_context.localized_mission_name(node.mission_id, &node.name);
        }
        let mut text_resources = ResourceManager::with_files(
            application_context
                .preparation_files()
                .expect("campaign descriptions require prepared resources")
                .clone(),
        );
        let shipping = application_context
            .shipping()
            .expect("campaign descriptions require application services");
        match text_resources.attach_or_from_shipping("Data/Text/Level.res", shipping) {
            Ok(()) => load_campaign_descriptions(
                application_context,
                &mut graph,
                &mut text_resources,
                shipping,
            ),
            Err(error) => tracing::warn!("Campaign mission descriptions unavailable: {error}"),
        }
        let recording_index = application_context.recording_index().clone();
        load_recording_links(&recording_index, &mut graph);
        if let Err(error) = recording_index.refresh_index() {
            tracing::warn!("Cannot refresh recording index: {error}");
        }
        let selected_progress = graph.first_selectable().unwrap_or(0);
        let assets = CampaignMapAssets::load(
            renderer,
            Some(resources),
            application_context
                .preparation_files()
                .expect("campaign manager requires prepared resources"),
        );
        Self {
            items: Vec::new(),
            assets,
            frame: FrameWnd::default(),
            exhibit_grid: ExhibitGridNavigator::new(graph.nodes.len(), selected_progress),
            graph,
            presentation: match profile.gameplay_config.campaign_presentation {
                CampaignPresentationMode::ClassicMap => CampaignPresentationMode::ProgressTree,
                mode => mode,
            },
            input: ModalInputState::new(),
            pseudo_debrief_at_ms: None,
            selected_classic: 0,
            selected_progress,
            show_achievement_badges: profile.gameplay_config.show_achievement_badges,
            achievement_overview: false,
            details_scroll: None,
            history_scroll: 0,
            selected_play: 0,
            replay_status: String::new(),
            recording_index,
            lifetime_totals: profile.campaign_history.totals(),
            lifetime_achievements: profile.campaign_history.achievement_aggregation(),
        }
    }

    /// Returns whether the application should exit when the browser closes.
    pub(crate) fn tick_browser(
        &mut self,
        window: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        cursor: Option<&ModalCursor<'_>>,
    ) -> Option<bool> {
        let (events, _) = layout::poll_events_with_transform(window, renderer);
        let transform = progress_transform(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let exit_requested = events.iter().any(|event| matches!(event, GameEvent::Quit));
        let choice = self.handle_events(events, transform, true);
        layout::enter_modal_gpu_phase(renderer);
        render_campaign_progress(
            renderer,
            transform,
            &self.graph,
            self.selected_progress,
            self.presentation,
            &self.assets,
            self.show_achievement_badges,
            self.lifetime_totals,
            true,
            self.achievement_overview,
            self.details_scroll,
            self.history_scroll,
            self.selected_play,
            &self.replay_status,
        );
        if let Some(cursor) = cursor {
            cursor.draw(renderer, transform, &self.input);
        }
        renderer.present();
        choice.map(|choice| {
            assert!(
                matches!(choice, CampaignMapChoice::Quit),
                "campaign browser cannot launch missions"
            );
            exit_requested
        })
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
        if self.presentation != CampaignPresentationMode::ClassicMap && self.graph.nodes.is_empty()
        {
            return Some(CampaignMapChoice::Quit);
        }

        let (events, transform) = layout::poll_events_with_transform(window, renderer);
        let input_transform = if self.presentation == CampaignPresentationMode::ClassicMap {
            transform
        } else {
            progress_transform(
                renderer.screen_width() as i32,
                renderer.screen_height() as i32,
            )
        };
        let final_choice = self.handle_events(events, input_transform, false);
        let transform = if self.presentation == CampaignPresentationMode::ClassicMap {
            transform
        } else {
            progress_transform(
                renderer.screen_width() as i32,
                renderer.screen_height() as i32,
            )
        };

        layout::enter_modal_gpu_phase(renderer);
        match self.presentation {
            CampaignPresentationMode::ClassicMap => render_campaign_map(
                renderer,
                transform,
                campaign,
                profiles,
                campaign_map,
                &self.items,
                self.selected_classic,
                &self.assets,
                menu_resources,
                &self.input,
                &self.frame,
                self.show_achievement_badges,
                campaign.achievement_aggregation(profiles),
                self.lifetime_achievements,
            ),
            CampaignPresentationMode::ProgressTree | CampaignPresentationMode::SherwoodMuseum => {
                render_campaign_progress(
                    renderer,
                    transform,
                    &self.graph,
                    self.selected_progress,
                    self.presentation,
                    &self.assets,
                    self.show_achievement_badges,
                    self.lifetime_totals,
                    false,
                    self.achievement_overview,
                    self.details_scroll,
                    self.history_scroll,
                    self.selected_play,
                    &self.replay_status,
                )
            }
        }
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

    fn move_progress_page(&mut self, direction: i32) {
        if self.achievement_overview {
            // selected_play is reset when switching between the achievement
            // overview and mission history; here it stores the catalogue page.
            let count =
                crate::achievement_hud::permanent_badge_presentations(Default::default()).len();
            self.selected_play = self
                .selected_play
                .saturating_add_signed(direction as isize)
                .min(count.saturating_sub(1) / 4);
            return;
        }
        let step = if self.presentation == CampaignPresentationMode::SherwoodMuseum {
            PROGRESS_PAGE_SIZE
        } else {
            1
        };
        self.selected_progress = self
            .selected_progress
            .saturating_add_signed(direction as isize * step as isize)
            .min(self.graph.nodes.len().saturating_sub(1));
        self.exhibit_grid =
            ExhibitGridNavigator::new(self.graph.nodes.len(), self.selected_progress);
    }

    fn scroll_details(&mut self, direction: i32, history: bool) {
        let Some(offset) = self.details_scroll else {
            return;
        };
        let node = &self.graph.nodes[self.selected_progress];
        if history {
            self.history_scroll = self
                .history_scroll
                .saturating_add_signed(direction as isize)
                .min(node.plays.len().saturating_sub(5));
        } else {
            self.details_scroll = Some(
                offset.saturating_add_signed(direction as isize).min(
                    detail_lines(node, &self.assets)
                        .len()
                        .saturating_sub(DETAIL_VISIBLE_LINES),
                ),
            );
        }
    }

    fn watch_selected_play(&mut self) {
        let Some(play) = self
            .graph
            .nodes
            .get(self.selected_progress)
            .and_then(|node| node.plays.get(self.selected_play))
        else {
            return;
        };
        self.replay_status = match play.recording.as_deref() {
            Some(path) => match crate::mission_replays::watch(
                path,
                (
                    play.attempt.key(
                        play.campaign_run_id
                            .expect("linked recording requires a campaign identity"),
                    ),
                    play.attempt.completed_at_unix_seconds(),
                ),
            ) {
                Ok(()) => "Replay opened in a separate window.".into(),
                Err(error) => error,
            },
            None => "No recording is available for this play.".into(),
        };
    }

    fn handle_events(
        &mut self,
        events: Vec<GameEvent>,
        transform: MenuTransform,
        browsing: bool,
    ) -> Option<CampaignMapChoice> {
        if let Some(completion) = self.recording_index.take_completion() {
            load_recording_links(&self.recording_index, &mut self.graph);
            if let Err(error) = completion {
                tracing::warn!("Cannot index previous recordings: {error}");
                self.replay_status = format!("Recording index unavailable: {error}");
            }
        }
        let mut final_choice = None;
        let input_enabled = self
            .pseudo_debrief_at_ms
            .map(|at| crate::window::process_uptime_ms() >= at)
            .unwrap_or(true);

        for event in events {
            let previous_selection = self.selected_progress;
            if !input_enabled {
                self.input.update_from_event(&event, transform);
                continue;
            }
            if self.details_scroll.is_some() {
                let history = (560.0..992.0).contains(&self.input.virt_x);
                let step = match event {
                    GameEvent::KeyDown {
                        keycode: Keycode::PageDown,
                        ..
                    } => Some(if history {
                        5
                    } else {
                        DETAIL_VISIBLE_LINES as i32
                    }),
                    GameEvent::KeyDown {
                        keycode: Keycode::PageUp,
                        ..
                    } => Some(if history {
                        -5
                    } else {
                        -(DETAIL_VISIBLE_LINES as i32)
                    }),
                    GameEvent::MouseWheel(delta) => {
                        let over_text = (32.0..544.0).contains(&self.input.virt_x)
                            && (254.0..662.0).contains(&self.input.virt_y);
                        let over_history = history && (260.0..620.0).contains(&self.input.virt_y);
                        if over_text || over_history {
                            self.scroll_details(
                                delta
                                    .saturating_mul(if history { 1 } else { 3 })
                                    .saturating_neg(),
                                history,
                            );
                        }
                        continue;
                    }
                    _ => None,
                };
                if let Some(step) = step {
                    self.scroll_details(step, history);
                    continue;
                }
            }
            if self.details_scroll.is_some() && !self.graph.nodes.is_empty() {
                match event {
                    GameEvent::KeyDown {
                        keycode: Keycode::Left | Keycode::Right,
                        ..
                    } => {
                        let step = if matches!(
                            event,
                            GameEvent::KeyDown {
                                keycode: Keycode::Left,
                                ..
                            }
                        ) {
                            -1
                        } else {
                            1
                        };
                        self.selected_progress = self
                            .selected_progress
                            .saturating_add_signed(step)
                            .min(self.graph.nodes.len() - 1);
                        self.exhibit_grid = ExhibitGridNavigator::new(
                            self.graph.nodes.len(),
                            self.selected_progress,
                        );
                        self.selected_play = 0;
                        self.history_scroll = 0;
                        self.details_scroll = Some(0);
                        self.replay_status.clear();
                        continue;
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::Escape,
                        ..
                    } => {
                        self.details_scroll = None;
                        continue;
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::Up | Keycode::Down,
                        ..
                    } => {
                        let count = self
                            .graph
                            .nodes
                            .get(self.selected_progress)
                            .map_or(0, |node| node.plays.len());
                        let step = if matches!(
                            event,
                            GameEvent::KeyDown {
                                keycode: Keycode::Up,
                                ..
                            }
                        ) {
                            -1
                        } else {
                            1
                        };
                        self.selected_play = self
                            .selected_play
                            .saturating_add_signed(step)
                            .min(count.saturating_sub(1));
                        if self.selected_play < self.history_scroll {
                            self.history_scroll = self.selected_play;
                        } else if self.selected_play >= self.history_scroll + 5 {
                            self.history_scroll = self.selected_play - 4;
                        }
                        self.replay_status.clear();
                        continue;
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::Return | Keycode::KpEnter | Keycode::Space,
                        ..
                    } => {
                        self.watch_selected_play();
                        continue;
                    }
                    GameEvent::MouseDown(x, y, 1, clicks) => {
                        let (x, y) = transform.from_screen(x, y);
                        self.input.virt_x = x as f32;
                        self.input.virt_y = y as f32;
                        if (560..992).contains(&x) && (260..620).contains(&y) {
                            let index = self.history_scroll + ((y - 260) / 72) as usize;
                            if index < self.graph.nodes[self.selected_progress].plays.len() {
                                self.selected_play = index;
                                self.replay_status.clear();
                                if clicks >= 2 {
                                    self.watch_selected_play();
                                }
                            }
                            continue;
                        }
                        if (560..992).contains(&x) && (630..666).contains(&y) {
                            self.watch_selected_play();
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            match event {
                GameEvent::KeyDown {
                    keycode: Keycode::Char(b'r' | b'd' | b'b'),
                    ..
                } if self.presentation != CampaignPresentationMode::ClassicMap => {
                    self.details_scroll = if self.details_scroll.is_some() {
                        None
                    } else {
                        Some(0)
                    };
                    self.selected_play = 0;
                    self.history_scroll = 0;
                    self.replay_status.clear();
                    self.achievement_overview = false;
                }
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => {
                    final_choice = Some(CampaignMapChoice::Quit);
                    break;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Tab,
                    ..
                } => {
                    self.achievement_overview = false;
                    self.details_scroll = None;
                    self.selected_play = 0;
                    self.history_scroll = 0;
                    self.replay_status.clear();
                    self.presentation = match self.presentation {
                        CampaignPresentationMode::ClassicMap => {
                            CampaignPresentationMode::ProgressTree
                        }
                        CampaignPresentationMode::ProgressTree => {
                            self.exhibit_grid = ExhibitGridNavigator::new(
                                self.graph.nodes.len(),
                                self.selected_progress,
                            );
                            CampaignPresentationMode::SherwoodMuseum
                        }
                        CampaignPresentationMode::SherwoodMuseum => {
                            CampaignPresentationMode::ProgressTree
                        }
                    };
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Char(b'a'),
                    ..
                } if self.presentation != CampaignPresentationMode::ClassicMap
                    && self.show_achievement_badges =>
                {
                    self.achievement_overview = !self.achievement_overview;
                    self.details_scroll = None;
                    self.selected_play = 0;
                    self.history_scroll = 0;
                    self.replay_status.clear();
                }
                GameEvent::KeyDown {
                    keycode: Keycode::PageDown,
                    ..
                } if self.presentation != CampaignPresentationMode::ClassicMap => {
                    self.move_progress_page(1)
                }
                GameEvent::KeyDown {
                    keycode: Keycode::PageUp,
                    ..
                } if self.presentation != CampaignPresentationMode::ClassicMap => {
                    self.move_progress_page(-1)
                }
                GameEvent::MouseWheel(delta)
                    if self.presentation != CampaignPresentationMode::ClassicMap =>
                {
                    self.move_progress_page(-delta.signum())
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Left | Keycode::Right,
                    ..
                } if self.presentation == CampaignPresentationMode::ProgressTree => {
                    let dx = if matches!(
                        event,
                        GameEvent::KeyDown {
                            keycode: Keycode::Left,
                            ..
                        }
                    ) {
                        -1
                    } else {
                        1
                    };
                    self.selected_progress =
                        tree_neighbor(&self.graph, self.selected_progress, dx, 0);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } if self.presentation == CampaignPresentationMode::ClassicMap
                    && !self.items.is_empty() =>
                {
                    self.selected_classic = self.selected_classic.saturating_sub(1)
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } if self.presentation == CampaignPresentationMode::ClassicMap
                    && self.selected_classic + 1 < self.items.len() =>
                {
                    self.selected_classic += 1
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } if self.presentation != CampaignPresentationMode::ClassicMap => {
                    match self.presentation {
                        CampaignPresentationMode::ProgressTree => {
                            self.selected_progress =
                                tree_neighbor(&self.graph, self.selected_progress, 0, -1)
                        }
                        CampaignPresentationMode::SherwoodMuseum => {
                            self.exhibit_grid.navigate(0, -1);
                            self.selected_progress = self.exhibit_grid.selected;
                        }
                        CampaignPresentationMode::ClassicMap => unreachable!(),
                    }
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } if self.presentation != CampaignPresentationMode::ClassicMap => {
                    match self.presentation {
                        CampaignPresentationMode::ProgressTree => {
                            self.selected_progress =
                                tree_neighbor(&self.graph, self.selected_progress, 0, 1)
                        }
                        CampaignPresentationMode::SherwoodMuseum => {
                            self.exhibit_grid.navigate(0, 1);
                            self.selected_progress = self.exhibit_grid.selected;
                        }
                        CampaignPresentationMode::ClassicMap => unreachable!(),
                    }
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Left,
                    ..
                } if self.presentation == CampaignPresentationMode::SherwoodMuseum => {
                    self.exhibit_grid.navigate(-1, 0);
                    self.selected_progress = self.exhibit_grid.selected;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Right,
                    ..
                } if self.presentation == CampaignPresentationMode::SherwoodMuseum => {
                    self.exhibit_grid.navigate(1, 0);
                    self.selected_progress = self.exhibit_grid.selected;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter | Keycode::Space,
                    ..
                } if self.presentation == CampaignPresentationMode::ClassicMap
                    && !self.items.is_empty() =>
                {
                    final_choice = Some(CampaignMapChoice::SelectMission(
                        self.items[self.selected_classic].mission_idx,
                    ));
                    break;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter | Keycode::Space,
                    ..
                } if self.presentation == CampaignPresentationMode::ClassicMap
                    && self.items.is_empty() =>
                {
                    final_choice = Some(CampaignMapChoice::Quit);
                    break;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter | Keycode::Space,
                    ..
                } if self.presentation != CampaignPresentationMode::ClassicMap
                    && !browsing
                    && !self.achievement_overview
                    && self.details_scroll.is_none()
                    && !self.graph.nodes.is_empty()
                    && self.graph.nodes[self.selected_progress].selectable =>
                {
                    final_choice = Some(CampaignMapChoice::SelectMission(
                        self.graph.nodes[self.selected_progress].mission_idx,
                    ));
                    break;
                }
                GameEvent::MouseMove { x, y, .. } => {
                    let (vx, vy) = transform.from_screen(x, y);
                    self.input.virt_x = vx as f32;
                    self.input.virt_y = vy as f32;
                }
                GameEvent::MouseDown(x, y, 1, clicks) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    self.input.virt_x = vx as f32;
                    self.input.virt_y = vy as f32;
                    if self.presentation != CampaignPresentationMode::ClassicMap {
                        if (76..116).contains(&vy) {
                            match vx {
                                32..=207 => {
                                    self.presentation = CampaignPresentationMode::ProgressTree;
                                    self.achievement_overview = false;
                                    self.details_scroll = None;
                                    self.selected_play = 0;
                                    self.history_scroll = 0;
                                    self.replay_status.clear();
                                }
                                220..=427 => {
                                    self.presentation = CampaignPresentationMode::SherwoodMuseum;
                                    self.achievement_overview = false;
                                    self.details_scroll = None;
                                    self.selected_play = 0;
                                    self.history_scroll = 0;
                                    self.replay_status.clear();
                                }
                                440..=647 if self.show_achievement_badges => {
                                    self.achievement_overview = !self.achievement_overview;
                                    self.details_scroll = None;
                                    self.selected_play = 0;
                                    self.history_scroll = 0;
                                    self.replay_status.clear();
                                }
                                884..=991 => {
                                    if self.details_scroll.take().is_some() {
                                        continue;
                                    }
                                    final_choice = Some(CampaignMapChoice::Quit);
                                    break;
                                }
                                660..=867 => {
                                    self.details_scroll = if self.details_scroll.is_some() {
                                        None
                                    } else {
                                        Some(0)
                                    };
                                    self.selected_play = 0;
                                    self.history_scroll = 0;
                                    self.replay_status.clear();
                                    self.achievement_overview = false;
                                }
                                _ => {}
                            }
                            self.exhibit_grid = ExhibitGridNavigator::new(
                                self.graph.nodes.len(),
                                self.selected_progress,
                            );
                            continue;
                        }
                        if (482..518).contains(&vy)
                            && !self.achievement_overview
                            && self.details_scroll.is_none()
                        {
                            if (588..804).contains(&vx) {
                                self.details_scroll = Some(0);
                                self.history_scroll = 0;
                                self.selected_play = 0;
                                self.replay_status.clear();
                            }
                            if (32..208).contains(&vx) {
                                self.move_progress_page(-1);
                            }
                            if (816..992).contains(&vx) {
                                self.move_progress_page(1);
                            }
                            continue;
                        }
                        if (712..752).contains(&vy)
                            && (840..992).contains(&vx)
                            && !browsing
                            && !self.achievement_overview
                            && self.details_scroll.is_none()
                            && self
                                .graph
                                .nodes
                                .get(self.selected_progress)
                                .is_some_and(|n| n.selectable)
                        {
                            final_choice = Some(CampaignMapChoice::SelectMission(
                                self.graph.nodes[self.selected_progress].mission_idx,
                            ));
                            break;
                        }
                    }
                    if self.presentation != CampaignPresentationMode::ClassicMap
                        && !self.achievement_overview
                        && self.details_scroll.is_none()
                        && let Some(index) = progress_hit_test(
                            &self.graph,
                            self.presentation,
                            self.selected_progress,
                            vx,
                            vy,
                        )
                    {
                        self.selected_progress = index;
                        self.exhibit_grid =
                            ExhibitGridNavigator::new(self.graph.nodes.len(), index);
                        if clicks >= 2 {
                            self.details_scroll = Some(0);
                            self.history_scroll = 0;
                            self.selected_play = 0;
                            self.replay_status.clear();
                        }
                    }
                }
                _ => {}
            }
            if previous_selection != self.selected_progress && self.details_scroll.is_some() {
                self.details_scroll = Some(0);
                self.selected_play = 0;
                self.history_scroll = 0;
                self.replay_status.clear();
            }

            self.input.update_from_event(&event, transform);
            if self.presentation == CampaignPresentationMode::ClassicMap {
                let widget_input = self.input.as_widget_input();
                let events = self.frame.process_input(&widget_input);
                self.input.end_frame();

                for (idx, item) in self.items.iter().enumerate() {
                    if self
                        .frame
                        .widget(item.loc_idx as u32)
                        .is_some_and(|w| w.base().state != UiState::Default)
                    {
                        self.selected_classic = idx;
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
            if final_choice.is_some() {
                break;
            }
        }

        final_choice
    }
}

// TODO: Move campaign-manager labels and status text into the localization catalog.
const PROGRESS_W: i32 = 1024;
const PROGRESS_H: i32 = 768;
const PROGRESS_PAGE_SIZE: usize = 12;
const CARD_W: i32 = 228;
const CARD_H: i32 = 88;

fn progress_transform(width: i32, height: i32) -> MenuTransform {
    MenuTransform {
        origin_x: (width - PROGRESS_W) / 2,
        origin_y: (height - PROGRESS_H) / 2,
    }
}

fn tree_neighbor(graph: &CampaignProgressGraph, selected: usize, dx: i32, dy: i32) -> usize {
    if graph.nodes.is_empty() {
        return 0;
    }
    let current = &graph.nodes[selected];
    graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            if dx != 0 {
                if dx < 0 {
                    node.depth < current.depth
                } else {
                    node.depth > current.depth
                }
            } else {
                node.depth == current.depth
                    && if dy < 0 {
                        node.lane < current.lane
                    } else {
                        node.lane > current.lane
                    }
            }
        })
        .min_by_key(|(_, node)| {
            (
                node.depth.abs_diff(current.depth),
                node.lane.abs_diff(current.lane),
            )
        })
        .map_or(selected, |(index, _)| index)
}

fn progress_node_rect(
    graph: &CampaignProgressGraph,
    presentation: CampaignPresentationMode,
    selected: usize,
    index: usize,
) -> (i32, i32, i32, i32) {
    let (column, row) = match presentation {
        CampaignPresentationMode::ProgressTree => {
            let focus = &graph.nodes[selected];
            let max_depth = graph.nodes.iter().map(|n| n.depth).max().unwrap_or(0);
            let first_depth = focus
                .depth
                .saturating_sub(1)
                .min(max_depth.saturating_sub(3));
            // Keep the story route visible while paging its side branches.
            let first_branch = focus.lane.saturating_sub(1) / 2 * 2 + 1;
            let lane = graph.nodes[index].lane;
            (
                graph.nodes[index].depth as i32 - first_depth as i32,
                if lane == 0 {
                    0
                } else if (first_branch..first_branch + 2).contains(&lane) {
                    (lane - first_branch + 1) as i32
                } else {
                    -1
                },
            )
        }
        CampaignPresentationMode::SherwoodMuseum => {
            let page_index =
                index as i32 - (selected / PROGRESS_PAGE_SIZE * PROGRESS_PAGE_SIZE) as i32;
            (page_index.rem_euclid(4), page_index.div_euclid(4))
        }
        CampaignPresentationMode::ClassicMap => unreachable!(),
    };
    (32 + column * 244, 176 + row * 100, CARD_W, CARD_H)
}

fn card_visible(rect: (i32, i32, i32, i32)) -> bool {
    let (x, y, w, h) = rect;
    x >= 32 && x + w <= 992 && y >= 176 && y + h <= 464
}

fn progress_hit_test(
    graph: &CampaignProgressGraph,
    presentation: CampaignPresentationMode,
    selected: usize,
    x: i32,
    y: i32,
) -> Option<usize> {
    graph.nodes.iter().enumerate().find_map(|(index, _)| {
        let rect = progress_node_rect(graph, presentation, selected, index);
        let (rx, ry, w, h) = rect;
        (card_visible(rect) && (rx..rx + w).contains(&x) && (ry..ry + h).contains(&y))
            .then_some(index)
    })
}

fn progress_rect(
    renderer: &mut Renderer,
    transform: MenuTransform,
    rect: (i32, i32, i32, i32),
    color: (u8, u8, u8),
) {
    let (x, y, w, h) = rect;
    renderer.render_gpu_rect(
        transform.origin_x + x,
        transform.origin_y + y,
        w,
        h,
        color.0,
        color.1,
        color.2,
        255,
    );
}

fn progress_text(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
) {
    let text = fit_progress_text(font, text, width);
    layout::render_text_virt_font(renderer, font, transform, &text, x, y);
}

fn fit_progress_text(font: &Font, text: &str, width: i32) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if font.text_width(text) <= width {
        return text.to_owned();
    }
    let mut end = text.len();
    for (index, _) in text.grapheme_indices(true).rev() {
        end = index;
        if font.text_width(&format!("{}...", &text[..end])) <= width {
            break;
        }
    }
    format!("{}...", &text[..end])
}

fn progress_button(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    label: &str,
    rect: (i32, i32, i32, i32),
    active: bool,
) {
    let (x, y, w, h) = rect;
    progress_rect(
        renderer,
        transform,
        rect,
        if active { (48, 39, 17) } else { (15, 23, 16) },
    );
    progress_rect(
        renderer,
        transform,
        (x, y + h - 2, w, 2),
        if active { (191, 164, 93) } else { (74, 82, 66) },
    );
    progress_text(
        renderer,
        font,
        transform,
        label,
        x + 12,
        y + (h - font.height() as i32) / 2,
        w - 24,
    );
}

fn permanent_achievement_status(
    current: robin_engine::achievement::AchievementAggregationProgress,
    archived: robin_engine::achievement::AchievementAggregationProgress,
) -> &'static str {
    use robin_engine::achievement::AchievementAggregationStatus;
    if current.earned() || archived.earned() {
        "Earned"
    } else if current.status == AchievementAggregationStatus::Unverifiable
        || archived.status == AchievementAggregationStatus::Unverifiable
    {
        "Unverified - incomplete records"
    } else {
        "Not earned"
    }
}

fn current_achievement_status(
    progress: robin_engine::achievement::AchievementAggregationProgress,
) -> String {
    use robin_engine::achievement::AchievementAggregationStatus;
    let status = match progress.status {
        AchievementAggregationStatus::Earned => "earned".to_owned(),
        AchievementAggregationStatus::Unverifiable => "incomplete records".to_owned(),
        AchievementAggregationStatus::MissingRequirements => "requirements not met".to_owned(),
        AchievementAggregationStatus::InProgress if progress.required_missions != 0 => format!(
            "{} / {} missions",
            progress.earned_missions, progress.required_missions
        ),
        AchievementAggregationStatus::InProgress => "in progress".to_owned(),
    };
    format!("Current campaign: {status}")
}

#[allow(clippy::too_many_arguments)]
fn render_campaign_progress(
    renderer: &mut Renderer,
    transform: MenuTransform,
    graph: &CampaignProgressGraph,
    selected: usize,
    presentation: CampaignPresentationMode,
    assets: &CampaignMapAssets,
    show_achievement_badges: bool,
    lifetime_totals: robin_engine::campaign_history::CampaignHistoryTotals,
    browsing: bool,
    achievement_overview: bool,
    details_scroll: Option<usize>,
    history_scroll: usize,
    selected_play: usize,
    replay_status: &str,
) {
    // Opaque panels isolate the text from both the map artwork and the menu beneath it.
    progress_rect(
        renderer,
        transform,
        (0, 0, PROGRESS_W, PROGRESS_H),
        (3, 6, 4),
    );
    progress_rect(renderer, transform, (16, 16, 992, 736), (7, 12, 8));
    progress_rect(renderer, transform, (16, 16, 992, 2), (157, 137, 81));
    let font = assets
        .progress_font
        .as_ref()
        .expect("campaign manager needs a readable font");
    let title_font = assets
        .progress_title_font
        .as_ref()
        .expect("campaign title font");
    let gallery = presentation == CampaignPresentationMode::SherwoodMuseum;
    progress_text(
        renderer,
        title_font,
        transform,
        "Campaign Manager",
        32,
        30,
        560,
    );
    progress_text(
        renderer,
        font,
        transform,
        &if achievement_overview {
            String::new()
        } else if gallery {
            format!(
                "{} wins / {}h {:02}m recorded",
                lifetime_totals.wins,
                lifetime_totals.known_duration_seconds / 3600,
                lifetime_totals.known_duration_seconds / 60 % 60
            )
        } else {
            format!(
                "{} / {} missions completed",
                graph.completed_missions, graph.known_missions
            )
        },
        640,
        38,
        352,
    );
    progress_button(
        renderer,
        font,
        transform,
        "Campaign",
        (32, 76, 176, 40),
        presentation == CampaignPresentationMode::ProgressTree
            && !achievement_overview
            && details_scroll.is_none(),
    );
    progress_button(
        renderer,
        font,
        transform,
        "Hall of Deeds",
        (220, 76, 208, 40),
        presentation == CampaignPresentationMode::SherwoodMuseum
            && !achievement_overview
            && details_scroll.is_none(),
    );
    if show_achievement_badges {
        progress_button(
            renderer,
            font,
            transform,
            "Achievements",
            (440, 76, 208, 40),
            achievement_overview,
        );
    }
    progress_button(
        renderer,
        font,
        transform,
        "Mission Details",
        (660, 76, 208, 40),
        details_scroll.is_some(),
    );
    progress_button(renderer, font, transform, "Back", (884, 76, 108, 40), false);
    progress_text(
        renderer,
        font,
        transform,
        if details_scroll.is_some() {
            "Briefing, requirements and recorded plays / Left and Right: select mission"
        } else if achievement_overview {
            "Permanent achievements for this player"
        } else if gallery {
            "Mission badges and best results across all attempts"
        } else {
            "Story route above / side missions below / D: mission details"
        },
        32,
        132,
        960,
    );
    progress_rect(renderer, transform, (32, 162, 960, 1), (70, 79, 59));

    if achievement_overview {
        let mut earned = graph.lifetime_achievements.earned();
        earned.union_with(graph.campaign_achievements.earned());
        for (index, badge) in crate::achievement_hud::permanent_badge_presentations(earned)
            .iter()
            .skip(selected_play * 4)
            .take(4)
            .enumerate()
        {
            let x = 32 + index as i32 % 2 * 496;
            let y = 176 + index as i32 / 2 * 244;
            progress_rect(renderer, transform, (x, y, 464, 220), (12, 20, 13));
            let current = graph.campaign_achievements.get(badge.id);
            let archived = graph.lifetime_achievements.get(badge.id);
            draw_achievement_badge_icon(
                renderer,
                badge.id,
                badge.earned,
                transform.origin_x + x + 10,
                transform.origin_y + y + 8,
            );
            progress_text(renderer, font, transform, &badge.label, x + 38, y + 6, 412);
            progress_text(
                renderer,
                font,
                transform,
                permanent_achievement_status(current, archived),
                x + 38,
                y + 40,
                412,
            );
            use robin_engine::achievement::AchievementId;
            // TODO: Localize these descriptions with the campaign manager labels.
            let requirement = match badge.id {
                AchievementId::CleanHands => {
                    "Complete one campaign, winning every required mission without causing a death. Deaths caused by NPCs also count if enabled."
                }
                AchievementId::Ghost => {
                    "Complete one campaign, winning every required mission without any gang member being seen by a living enemy."
                }
                AchievementId::PileOBones => {
                    "Have at least 10 people knocked out, tied, netted, carried, or dead in one building at once, then win the mission."
                }
                _ => badge.id.description(),
            };
            let wrapped = layout::wrap_text_for_box_font(font, requirement, 432, 4);
            for (line, text) in wrapped.lines.iter().enumerate() {
                progress_text(
                    renderer,
                    font,
                    transform,
                    text,
                    x + 16,
                    y + 78 + line as i32 * 23,
                    432,
                );
            }
            progress_text(
                renderer,
                font,
                transform,
                &current_achievement_status(current),
                x + 16,
                y + 190,
                432,
            );
        }
        progress_text(
            renderer,
            font,
            transform,
            "Earned achievements stay with this player when you load an older save.",
            32,
            656,
            960,
        );
        progress_text(
            renderer,
            font,
            transform,
            "Campaign awards cannot combine missions from different playthroughs.",
            32,
            686,
            960,
        );
        progress_text(
            renderer,
            font,
            transform,
            &format!(
                "Page {} / {}   PgUp/PgDn or wheel: browse   A: missions   Esc: back",
                selected_play + 1,
                crate::achievement_hud::permanent_badge_presentations(earned)
                    .len()
                    .div_ceil(4)
            ),
            32,
            722,
            960,
        );
        return;
    }
    if graph.nodes.is_empty() {
        progress_text(
            renderer,
            font,
            transform,
            "No campaign missions in this content.",
            32,
            184,
            960,
        );
        return;
    }
    let node = &graph.nodes[selected];
    if let Some(offset) = details_scroll {
        render_mission_details(
            renderer,
            transform,
            node,
            assets,
            offset,
            history_scroll,
            selected_play,
            replay_status,
        );
        return;
    }
    if presentation == CampaignPresentationMode::ProgressTree {
        for (index, child) in graph.nodes.iter().enumerate() {
            let rect = progress_node_rect(graph, presentation, selected, index);
            if !card_visible(rect) {
                continue;
            }
            let (x, y, _, h) = rect;
            for &parent in &child.prerequisite_nodes {
                let (px, py, pw, ph) = progress_node_rect(graph, presentation, selected, parent);
                // Off-screen dependencies end at the viewport edge, never over the header/details.
                renderer.render_gpu_line(
                    transform.origin_x + (px + pw).clamp(32, 992),
                    transform.origin_y + (py + ph / 2).clamp(170, 472),
                    transform.origin_x + x,
                    transform.origin_y + y + h / 2,
                    114,
                    111,
                    74,
                );
            }
        }
    }
    for (index, entry) in graph.nodes.iter().enumerate() {
        let rect = progress_node_rect(graph, presentation, selected, index);
        if !card_visible(rect) {
            continue;
        }
        let (x, y, w, h) = rect;
        let (status, mut color) = match entry.state {
            MissionProgressState::Completed => ("Completed", (102, 157, 101)),
            MissionProgressState::Available => ("Available", (193, 163, 78)),
            MissionProgressState::InProgress => ("In progress", (193, 163, 78)),
            MissionProgressState::Lost => ("Lost", (174, 102, 81)),
            MissionProgressState::Expired => ("Expired", (139, 121, 103)),
            MissionProgressState::Locked => ("Locked", (99, 112, 101)),
        };
        if gallery {
            color = if entry.lifetime_win_count != 0 {
                (102, 157, 101)
            } else if entry.lifetime_attempt_count != 0 {
                (193, 163, 78)
            } else {
                (99, 112, 101)
            };
        }
        if index == selected {
            progress_rect(
                renderer,
                transform,
                (x - 2, y - 2, w + 4, h + 4),
                (194, 165, 88),
            );
        }
        progress_rect(
            renderer,
            transform,
            rect,
            if index == selected {
                (30, 40, 19)
            } else {
                (12, 20, 14)
            },
        );
        progress_rect(renderer, transform, (x, y, 3, h), color);
        let wrap = layout::wrap_text_for_box_font(font, &entry.name, w - 20, 2);
        for (line, text) in wrap.lines.iter().enumerate() {
            let text = if line == 1 && !wrap.remaining.is_empty() {
                format!("{text}...")
            } else {
                text.clone()
            };
            progress_text(
                renderer,
                font,
                transform,
                &text,
                x + 10,
                y + 10 + line as i32 * 23,
                w - 20,
            );
        }
        let kind = match entry.kind {
            MissionKind::CampaignEvent => "Event",
            MissionKind::Unavailable => "Archived",
            kind => kind.label(),
        };
        let status = if gallery {
            if entry.lifetime_attempt_count == 0 {
                format!("{kind} / Unplayed")
            } else {
                format!("{kind} / {} wins", entry.lifetime_win_count)
            }
        } else {
            format!("{kind} / {status}")
        };
        progress_text(renderer, font, transform, &status, x + 10, y + 62, w - 20);
    }
    progress_button(
        renderer,
        font,
        transform,
        "Previous",
        (32, 482, 176, 36),
        false,
    );
    progress_button(
        renderer,
        font,
        transform,
        "Next",
        (816, 482, 176, 36),
        false,
    );
    let page = match presentation {
        CampaignPresentationMode::SherwoodMuseum => format!(
            "Gallery {} / {}",
            selected / PROGRESS_PAGE_SIZE + 1,
            graph.nodes.len().div_ceil(PROGRESS_PAGE_SIZE)
        ),
        _ => format!(
            "Stage {} / Branch page {}",
            node.depth + 1,
            node.lane.saturating_sub(1) / 2 + 1
        ),
    };
    progress_text(renderer, font, transform, &page, 236, 490, 340);
    progress_button(
        renderer,
        font,
        transform,
        "Details (D)",
        (588, 482, 216, 36),
        false,
    );
    progress_rect(renderer, transform, (32, 534, 960, 1), (91, 94, 67));
    progress_text(
        renderer,
        font,
        transform,
        &format!("{} / {}", node.name, node.kind.label()),
        32,
        550,
        960,
    );
    let mut record = format!(
        "All attempts: {} / Wins: {}",
        node.lifetime_attempt_count, node.lifetime_win_count
    );
    if let Some(seconds) = node.best.fastest_win_seconds {
        record.push_str(&format!(" / Best: {}m {:02}s", seconds / 60, seconds % 60));
    }
    if let Some(score) = node.best.highest_score {
        record.push_str(&format!(" / Score: {score}"));
    }
    let stats = if gallery {
        record
    } else {
        format!(
            "Current campaign: {} / Attempts: {} / Wins: {}",
            node.state.label(),
            node.attempt_count,
            node.win_count
        )
    };
    progress_text(renderer, font, transform, &stats, 32, 590, 960);
    progress_text(
        renderer,
        font,
        transform,
        node.description
            .as_deref()
            .or(node.briefing.as_deref())
            .unwrap_or("No mission description is available in this content."),
        32,
        616,
        960,
    );
    if show_achievement_badges && node.kind.is_field_mission() {
        for (index, badge) in crate::achievement_hud::mission_badge_presentations(
            if gallery {
                node.badges
            } else {
                node.campaign_badges
            },
            |_| None,
        )
        .iter()
        .filter(|badge| node.available_badges.contains(badge.id))
        .enumerate()
        {
            let x = 32 + index as i32 % 4 * 242;
            let y = 650 + index as i32 / 4 * 24;
            draw_achievement_badge_icon(
                renderer,
                badge.id,
                badge.earned,
                transform.origin_x + x,
                transform.origin_y + y,
            );
            progress_text(
                renderer,
                font,
                transform,
                &format!(
                    "{}: {}",
                    badge.label,
                    if badge.earned { "earned" } else { "not earned" }
                ),
                x + 18,
                y,
                218,
            );
        }
    }
    let action = if browsing {
        "Browse only / launch missions from Sherwood"
    } else if node.history_replay {
        "Practice replay / campaign rewards unchanged"
    } else if node.selectable {
        "Double-click to inspect mission"
    } else {
        "D: view mission details"
    };
    progress_text(renderer, font, transform, action, 32, 724, 780);
    if !browsing && node.selectable {
        progress_button(
            renderer,
            font,
            transform,
            "Inspect",
            (840, 712, 152, 40),
            false,
        );
    }
    if graph.cyclic_prerequisites {
        progress_text(
            renderer,
            font,
            transform,
            "Cyclic prerequisites",
            544,
            132,
            448,
        );
    }
}

fn render_achievement_aggregation_summary(
    renderer: &mut Renderer,
    transform: MenuTransform,
    scope_label: &str,
    summary: robin_engine::achievement::AchievementAggregationSummary,
    font: &Font,
    y: i32,
) {
    let entries = crate::achievement_hud::achievement_aggregation_presentations(summary);
    let earned = entries
        .iter()
        .filter(|entry| entry.progress.earned())
        .count();
    layout::render_text_virt_font(
        renderer,
        font,
        transform,
        &format!(
            "{scope_label}: {earned}/{} achievements earned",
            entries.len()
        ),
        25,
        y,
    );
}

/// Small code-native fallback icons. Stable badge icon keys remain available
/// to future asset packs without coupling campaign data to presentation art.
fn draw_achievement_badge_icon(
    renderer: &mut Renderer,
    id: robin_engine::achievement::AchievementId,
    earned: bool,
    x: i32,
    y: i32,
) {
    let color = if earned {
        Renderer::create_color_16(245, 210, 95)
    } else {
        Renderer::create_color_16(92, 82, 68)
    };
    use robin_engine::achievement::AchievementId;
    match id {
        AchievementId::CleanHands => {
            renderer.draw_rect_outline_screen(x + 3, y + 5, x + 9, y + 12, color);
            for finger in 0..4 {
                renderer.draw_line_screen(
                    x + 2 + finger * 2,
                    y + 5,
                    x + 2 + finger * 2,
                    y + 1 + (finger & 1),
                    color,
                );
            }
        }
        AchievementId::Ghost => {
            renderer.draw_rect_outline_screen(x + 2, y + 3, x + 10, y + 11, color);
            renderer.draw_line_screen(x + 2, y + 11, x + 4, y + 9, color);
            renderer.draw_line_screen(x + 4, y + 9, x + 6, y + 11, color);
            renderer.draw_line_screen(x + 6, y + 11, x + 8, y + 9, color);
            renderer.render_gpu_rect(x + 4, y + 5, 1, 1, 245, 225, 160, 255);
            renderer.render_gpu_rect(x + 8, y + 5, 1, 1, 245, 225, 160, 255);
        }
        AchievementId::PileOBones => {
            renderer.draw_line_screen(x + 1, y + 2, x + 11, y + 11, color);
            renderer.draw_line_screen(x + 11, y + 2, x + 1, y + 11, color);
            renderer.render_gpu_rect(x, y + 1, 3, 3, 245, 225, 160, 255);
            renderer.render_gpu_rect(x + 10, y + 10, 3, 3, 245, 225, 160, 255);
        }
        _ => {
            renderer.draw_rect_outline_screen(x + 2, y + 5, x + 11, y + 12, color);
            renderer.draw_line_screen(x + 1, y + 5, x + 6, y + 1, color);
            renderer.draw_line_screen(x + 6, y + 1, x + 12, y + 5, color);
            renderer.draw_rect_outline_screen(x + 5, y + 8, x + 8, y + 12, color);
        }
    }
}

fn load_recording_links(
    index: &crate::mission_replays::RecordingIndex,
    graph: &mut CampaignProgressGraph,
) {
    for play in graph.nodes.iter_mut().flat_map(|node| &mut node.plays) {
        play.recording = play.campaign_run_id.and_then(|run| {
            index.find(
                play.attempt.key(run),
                play.attempt.completed_at_unix_seconds(),
            )
        });
    }
}

fn load_campaign_descriptions(
    application: &ApplicationContext,
    graph: &mut CampaignProgressGraph,
    text: &mut ResourceManager,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
) {
    for node in &mut graph.nodes {
        // Scripted events reuse field-mission resource slots, which can contain
        // unrelated narrative (the epilogue points at an unused ambush).
        if !node.kind.is_field_mission() {
            continue;
        }

        let Some(descriptor) =
            crate::mission_descriptors::for_presentation(application, shipping, node.mission_id)
        else {
            continue; // Optional content: leave absence explicit, never invent story text.
        };
        for (index, target) in [(1, &mut node.description), (2, &mut node.briefing)] {
            match text.get_string(descriptor.mission_description.text_table_id, index) {
                Ok(value)
                    if !value.trim().is_empty()
                        && !value.trim().eq_ignore_ascii_case("NOTUSED") =>
                {
                    *target = Some(value.to_owned())
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    mission_id = node.mission_id,
                    index,
                    "Campaign description text unavailable: {error}"
                ),
            }
        }
    }
}

const DETAIL_VISIBLE_LINES: usize = 16;

fn detail_lines(
    node: &crate::campaign_progress::CampaignProgressNode,
    assets: &CampaignMapAssets,
) -> Vec<String> {
    let font = assets.progress_font.as_ref().expect("mission details font");
    let mut sections = vec!["ENTRY REQUIREMENTS".to_owned()];
    if node.availability_notes.is_empty() {
        sections.push("This mission is currently offered in Sherwood.".into());
    } else {
        sections.extend(node.availability_notes.iter().cloned());
    }
    sections.push(String::new());
    sections.push("MISSION BRIEFING".into());
    sections.push(
        node.briefing
            .as_deref()
            .or(node.description.as_deref())
            .unwrap_or("No mission briefing is available in this content.")
            .into(),
    );
    sections
        .into_iter()
        .flat_map(|text| {
            if text.is_empty() {
                vec![String::new()]
            } else {
                layout::wrap_text_for_box_font(font, &text, 480, usize::MAX).lines
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn render_detail_scrollbar(
    renderer: &mut Renderer,
    transform: MenuTransform,
    x: i32,
    y: i32,
    height: i32,
    offset: usize,
    visible: usize,
    total: usize,
) {
    if total <= visible {
        return;
    }
    let thumb = (height * visible as i32 / total as i32).max(16);
    let top = (height - thumb) * offset as i32 / (total - visible) as i32;
    progress_rect(renderer, transform, (x, y, 4, height), (43, 55, 31));
    progress_rect(renderer, transform, (x, y + top, 4, thumb), (193, 163, 78));
}

#[allow(clippy::too_many_arguments)]
fn render_mission_details(
    renderer: &mut Renderer,
    transform: MenuTransform,
    node: &crate::campaign_progress::CampaignProgressNode,
    assets: &CampaignMapAssets,
    text_scroll: usize,
    history_scroll: usize,
    selected_play: usize,
    status: &str,
) {
    let font = assets.progress_font.as_ref().expect("mission details font");
    progress_text(renderer, font, transform, &node.name, 32, 180, 960);
    progress_text(
        renderer,
        font,
        transform,
        &format!("{} / {}", node.kind.label(), node.state.label()),
        32,
        218,
        512,
    );
    progress_rect(renderer, transform, (32, 254, 512, 408), (12, 20, 13));
    let lines = detail_lines(node, assets);
    let text_scroll = text_scroll.min(lines.len().saturating_sub(DETAIL_VISIBLE_LINES));
    for (row, text) in lines
        .iter()
        .skip(text_scroll)
        .take(DETAIL_VISIBLE_LINES)
        .enumerate()
    {
        progress_text(
            renderer,
            font,
            transform,
            text,
            48,
            264 + row as i32 * 24,
            480,
        );
    }
    render_detail_scrollbar(
        renderer,
        transform,
        536,
        254,
        408,
        text_scroll,
        DETAIL_VISIBLE_LINES,
        lines.len(),
    );
    progress_text(
        renderer,
        font,
        transform,
        &format!("Previous plays ({})", node.plays.len()),
        560,
        218,
        432,
    );
    if node.plays.is_empty() {
        progress_text(
            renderer,
            font,
            transform,
            "No previous plays recorded.",
            572,
            280,
            408,
        );
    }
    let history_scroll = history_scroll.min(node.plays.len().saturating_sub(5));
    for (index, play) in node.plays.iter().enumerate().skip(history_scroll).take(5) {
        let y = 260 + (index - history_scroll) as i32 * 72;
        progress_rect(
            renderer,
            transform,
            (560, y, 432, 66),
            if index == selected_play {
                (43, 55, 31)
            } else {
                (12, 20, 13)
            },
        );
        let date = play
            .attempt
            .completed_at_unix_seconds()
            .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
            .map(|date| date.strftime("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "Date not recorded".into());
        let duration = play
            .attempt
            .duration_seconds()
            .map(|s| format!("{}m {:02}s", s / 60, s % 60))
            .unwrap_or_else(|| "Time unknown".into());
        let kind = match play.attempt.kind() {
            robin_engine::campaign_history::MissionAttemptKind::Campaign => "Campaign",
            robin_engine::campaign_history::MissionAttemptKind::HistoryReplay => "Practice",
        };
        progress_text(
            renderer,
            font,
            transform,
            &format!("{:?} / {duration} / {kind}", play.attempt.outcome()),
            572,
            y + 6,
            408,
        );
        progress_text(renderer, font, transform, &date, 572, y + 36, 408);
    }
    let can_watch = node
        .plays
        .get(selected_play)
        .is_some_and(|play| play.recording.is_some());
    progress_button(
        renderer,
        font,
        transform,
        if can_watch {
            "Watch selected replay"
        } else {
            "Recording unavailable"
        },
        (560, 630, 432, 36),
        can_watch,
    );
    render_detail_scrollbar(
        renderer,
        transform,
        984,
        260,
        360,
        history_scroll,
        5,
        node.plays.len(),
    );
    progress_text(
        renderer,
        font,
        transform,
        if status.is_empty() {
            "Scroll over either column / PgUp/PgDn: scroll / Up/Down: plays / Enter: watch"
        } else {
            status
        },
        32,
        722,
        960,
    );
}

fn campaign_map_items(
    application_context: &ApplicationContext,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    lifetime: Option<&robin_engine::campaign_history::ProfileCampaignHistory>,
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
            let name =
                application_context.localized_mission_name(profile.id, &profile.mission_name);
            let location = mission_location_from_index(loc_idx).unwrap_or(MissionLocation::Nowhere);
            if matches!(
                location,
                MissionLocation::Nowhere | MissionLocation::Sherwood
            ) {
                return None;
            }
            let descriptors = crate::mission_descriptors::for_presentation(
                application_context,
                shipping,
                profile.id,
            );
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
                achievement_badges: crate::achievement_hud::mission_badge_presentations(
                    crate::campaign_progress::combined_mission_badges(
                        mission.achievement_badges(),
                        profile.id,
                        lifetime,
                    ),
                    |_| None,
                )
                .into_iter()
                .filter(|badge| {
                    robin_engine::achievement::available_mission_badges(profile, profiles)
                        .contains(badge.id)
                })
                .collect(),
            })
        })
        .collect()
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
    show_achievement_badges: bool,
    campaign_achievements: robin_engine::achievement::AchievementAggregationSummary,
    lifetime_achievements: robin_engine::achievement::AchievementAggregationSummary,
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
            renderer,
            transform,
            campaign,
            profiles,
            item,
            assets,
            resources,
            input,
            show_achievement_badges,
        );
    } else {
        draw_close_button(renderer, transform, assets, frame);
    }

    if let Some(font) = assets.font.as_ref() {
        widget_bridge::draw_frame_labels(renderer, transform, frame, font, TextAlign::Center);
        layout::render_text_virt_font(renderer, font, transform, "Tab: History & Practice", 18, 12);
        if show_achievement_badges {
            render_achievement_aggregation_summary(
                renderer,
                transform,
                "Campaign",
                campaign_achievements,
                font,
                28,
            );
            render_achievement_aggregation_summary(
                renderer,
                transform,
                "Lifetime",
                lifetime_achievements,
                font,
                60,
            );
        }
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
    show_achievement_badges: bool,
) {
    let Some(font) = resources
        .and_then(IngameMenuResources::popup_font_any)
        .or(assets.font.as_ref())
    else {
        return;
    };

    let short_desc = ShortMissionDescriptionWindow::new(
        campaign,
        profiles,
        item,
        input,
        assets,
        show_achievement_badges,
    );
    let tooltip_height = if show_achievement_badges { 344 } else { 100 };

    if assets.tooltip_bg.is_none() {
        renderer.render_gpu_rect(
            transform.origin_x + short_desc.x,
            transform.origin_y + short_desc.y,
            220,
            tooltip_height,
            42,
            32,
            18,
            235,
        );
        renderer.draw_rect_outline_screen(
            transform.origin_x + short_desc.x,
            transform.origin_y + short_desc.y,
            transform.origin_x + short_desc.x + 220,
            transform.origin_y + short_desc.y + tooltip_height,
            Renderer::create_color_16(210, 180, 110),
        );
    }

    if show_achievement_badges {
        // The shipped tooltip bitmap is only 100 pixels tall. Extend it with
        // a neutral panel for the mission badge catalogue.
        renderer.render_gpu_rect(
            transform.origin_x + short_desc.x,
            transform.origin_y + short_desc.y + 98,
            220,
            246,
            42,
            32,
            18,
            235,
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

    if show_achievement_badges {
        for (index, badge) in item.achievement_badges.iter().enumerate() {
            let x = short_desc.x + 8;
            let y = short_desc.y + 104 + index as i32 * 24;
            draw_achievement_badge_icon(
                renderer,
                badge.id,
                badge.earned,
                transform.origin_x + x,
                transform.origin_y + y + 1,
            );
            let text = layout::wrap_text_for_box_font(font, &badge.label, 192, 1);
            if let Some(label) = text.lines.first() {
                layout::render_text_virt_font(renderer, font, transform, label, x + 14, y);
            }
        }
    }
}

impl ShortMissionDescriptionWindow {
    fn new(
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        item: &CampaignMapItem,
        input: &ModalInputState,
        assets: &CampaignMapAssets,
        show_achievement_badges: bool,
    ) -> Self {
        let mut x = input.virt_x as i32 + 25;
        let mut y = input.virt_y as i32 + 25;
        x = x.clamp(0, MAP_W - 220);
        let height = if show_achievement_badges { 344 } else { 100 };
        y = y.clamp(0, MAP_H - height);

        let mut frame = FrameWnd::new(
            "Short mission description",
            ScreenBBox::from_coords(x as f32, y as f32, (x + 220) as f32, (y + height) as f32),
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

fn load_progress_font(files: &robin_engine::sbfile::SbFileSystem, key: &str) -> Option<Font> {
    let config = native_font::load_font_config(files)
        .unwrap_or_else(|error| panic!("campaign font configuration unavailable: {error}"));
    Some(
        native_font::load_font_by_name_for_active_locale(&config, key, files)
            .unwrap_or_else(|error| panic!("campaign font {key} unavailable: {error}")),
    )
}

fn load_campaign_font(files: &robin_engine::sbfile::SbFileSystem) -> Option<Font> {
    let config = native_font::load_font_config(files).ok()?;
    native_font::load_font_by_name_for_active_locale(&config, "Default", files).ok()
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

#[cfg(test)]
fn fixture_plays(count: u64) -> Vec<crate::campaign_progress::MissionPlay> {
    use robin_engine::campaign_history::MissionAttemptOutcome;
    let mut campaign = Campaign::default();
    campaign
        .missions
        .push(robin_engine::mission::Mission::new());
    for index in 0..count {
        campaign.record_mission_attempt(
            0,
            match index % 3 {
                0 => MissionAttemptOutcome::Won,
                1 => MissionAttemptOutcome::Lost,
                _ => MissionAttemptOutcome::Interrupted,
            },
            Some(1_783_600_000 + index as i64 * 3600),
            Some(42),
            185 + index as u32 * 35,
            Default::default(),
            &Default::default(),
            None,
        );
    }
    campaign.missions[0]
        .attempt_history()
        .attempts()
        .iter()
        .rev()
        .map(|attempt| crate::campaign_progress::MissionPlay {
            campaign_run_id: Some(42),
            attempt: attempt.clone(),
            recording: None,
        })
        .collect()
}

#[cfg(test)]
mod browser_tests {
    use super::*;
    use robin_engine::{mission::Mission, profiles::MissionProfile};

    #[test]
    fn details_can_select_every_play_without_launching_the_mission() {
        let mut state = browser();
        state.graph.nodes[0].plays = fixture_plays(13);
        let transform = progress_transform(1024, 768);
        state.handle_events(vec![key(Keycode::Char(b'd'))], transform, true);
        for _ in 0..20 {
            state.handle_events(vec![key(Keycode::Down)], transform, true);
        }
        assert_eq!(state.selected_play, 12);
        assert_eq!(
            state.handle_events(vec![key(Keycode::Return)], transform, true),
            None
        );
        assert!(state.replay_status.contains("No recording"));
        assert_eq!(state.history_scroll, 8);
        state.input.virt_x = 580.0;
        state.input.virt_y = 300.0;
        state.handle_events(vec![GameEvent::MouseWheel(3)], transform, true);
        assert_eq!(state.history_scroll, 5);
        assert_eq!(state.selected_play, 12);
        state.handle_events(vec![GameEvent::MouseDown(580, 265, 1, 1)], transform, true);
        assert_eq!(state.selected_play, 5);
        state.handle_events(vec![key(Keycode::Right)], transform, true);
        assert_eq!(state.selected_play, 0);
        assert_eq!(state.exhibit_grid.selected, state.selected_progress);
        assert_eq!(state.details_scroll, Some(0));
        assert!(state.replay_status.is_empty());
    }

    #[test]
    fn detail_columns_scroll_independently_and_clamp_to_content() {
        let mut state = browser();
        let font_bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/core-datadir/Data/Interface/Fonts/arial.ttf"),
        )
        .expect("read checked-in Arial fixture");
        state.assets.progress_font = Some(Font::TrueType(crate::font::TrueTypeFont::from_parts(
            &[0; 32],
            15,
            0,
            0,
            &[0; 32],
            0x00FFFFFF,
            &font_bytes,
        )));
        state.graph.nodes[0].availability_notes = vec!["A requirement".into(); 50];
        state.graph.nodes[0].plays = fixture_plays(13);
        let transform = progress_transform(1024, 768);
        state.handle_events(vec![key(Keycode::Char(b'd'))], transform, true);
        state.input.virt_x = 100.0;
        state.input.virt_y = 300.0;
        state.handle_events(vec![GameEvent::MouseWheel(-1)], transform, true);
        assert_eq!(state.details_scroll, Some(3));
        assert_eq!(state.history_scroll, 0);
        state.input.virt_x = 600.0;
        state.handle_events(vec![GameEvent::MouseWheel(-2)], transform, true);
        assert_eq!(state.history_scroll, 2);
        assert_eq!(state.details_scroll, Some(3));
        state.handle_events(vec![GameEvent::MouseDown(580, 265, 1, 1)], transform, true);
        assert_eq!(state.selected_play, 2);
        state.handle_events(vec![GameEvent::MouseWheel(-100)], transform, true);
        assert_eq!(state.history_scroll, 8);
        state.handle_events(vec![GameEvent::MouseWheel(100)], transform, true);
        assert_eq!(state.history_scroll, 0);
        state.input.virt_x = 100.0;
        state.handle_events(vec![GameEvent::MouseWheel(-100)], transform, true);
        assert_eq!(
            state.details_scroll,
            Some(detail_lines(&state.graph.nodes[0], &state.assets).len() - DETAIL_VISIBLE_LINES)
        );
        state.handle_events(vec![GameEvent::MouseWheel(100)], transform, true);
        assert_eq!(state.details_scroll, Some(0));
        state.input.virt_y = 100.0;
        state.handle_events(vec![GameEvent::MouseWheel(-1)], transform, true);
        assert_eq!(state.details_scroll, Some(0));
    }

    #[test]
    fn double_click_opens_details_even_for_locked_missions() {
        for presentation in [
            CampaignPresentationMode::ProgressTree,
            CampaignPresentationMode::SherwoodMuseum,
        ] {
            for browsing in [false, true] {
                for selectable in [false, true] {
                    let mut state = browser();
                    state.presentation = presentation;
                    state.graph.nodes[0].selectable = selectable;
                    let (x, y, _, _) = progress_node_rect(&state.graph, presentation, 0, 0);
                    assert_eq!(
                        state.handle_events(
                            vec![GameEvent::MouseDown(x + 5, y + 5, 1, 2)],
                            progress_transform(1024, 768),
                            browsing
                        ),
                        None
                    );
                    assert_eq!(state.details_scroll, Some(0));
                    assert_eq!(state.history_scroll, 0);
                }
            }
        }
    }

    #[test]
    fn mission_details_cannot_launch_a_mission_and_tabs_restore_navigation() {
        let mut state = browser();
        state.graph.nodes[0].availability_notes = vec!["A requirement".into(); 5];
        state.graph.nodes[0].selectable = true;
        let transform = progress_transform(1024, 768);
        assert_eq!(
            state.handle_events(vec![key(Keycode::Char(b'r'))], transform, false),
            None
        );
        assert_eq!(state.details_scroll, Some(0));
        assert_eq!(
            state.handle_events(vec![key(Keycode::Return)], transform, false),
            None
        );
        state.handle_events(vec![key(Keycode::Down)], transform, false);
        assert_eq!(state.selected_play, 0);
        assert_eq!(state.details_scroll, Some(0));
        state.handle_events(vec![key(Keycode::Tab)], transform, false);
        state.handle_events(vec![key(Keycode::Char(b'd'))], transform, false);
        assert_eq!(state.details_scroll, Some(0));
        state.handle_events(vec![key(Keycode::Char(b'r'))], transform, false);
        assert_eq!(state.details_scroll, None);
        assert_eq!(state.presentation, CampaignPresentationMode::SherwoodMuseum);
    }

    #[test]
    fn permanent_award_survives_older_save_without_fabricating_current_progress() {
        use robin_engine::achievement::{
            AchievementAggregationInput, AchievementId, aggregate_achievement,
        };
        let current = aggregate_achievement(
            AchievementId::CleanHands,
            AchievementAggregationInput {
                earned_missions: 2,
                required_missions: 10,
                ..Default::default()
            },
        );
        let archived = aggregate_achievement(
            AchievementId::CleanHands,
            AchievementAggregationInput {
                envelope_complete: true,
                earned_missions: 10,
                required_missions: 10,
                ..Default::default()
            },
        );
        assert_eq!(permanent_achievement_status(current, archived), "Earned");
        assert_eq!(
            current_achievement_status(current),
            "Current campaign: 2 / 10 missions"
        );
        let unknown = aggregate_achievement(
            AchievementId::CleanHands,
            AchievementAggregationInput {
                envelope_unverifiable: true,
                ..Default::default()
            },
        );
        assert_eq!(
            permanent_achievement_status(current, unknown),
            "Unverified - incomplete records"
        );
        assert_eq!(permanent_achievement_status(unknown, archived), "Earned");
        assert_eq!(
            current_achievement_status(unknown),
            "Current campaign: incomplete records"
        );
    }

    fn browser() -> CampaignMapModalState {
        let mut profiles = engine_profiles::ProfileManager::new();
        profiles.missions.push(MissionProfile {
            id: 1,
            mission_name: "Sherwood".into(),
            location: MissionLocation::Sherwood,
            ..Default::default()
        });
        for id in [10, 20] {
            profiles.missions.push(MissionProfile {
                id,
                mission_name: format!("Mission {id}"),
                ..Default::default()
            });
        }
        let mut campaign = Campaign::default();
        for idx in 0..3 {
            campaign.missions.push(Mission {
                profile_idx: Some(idx),
                ..Mission::new()
            });
            campaign.accessible_mission_indices.push(idx as usize);
        }
        let graph = CampaignProgressGraph::build(&campaign, &profiles, None);
        assert_eq!(graph.nodes.len(), 2);
        assert!(graph.nodes.iter().all(|node| node.selectable));
        let history = robin_engine::campaign_history::ProfileCampaignHistory::default();
        let recording_index =
            std::sync::Arc::new(crate::mission_replays::RecordingIndex::disabled());
        CampaignMapModalState {
            items: Vec::new(),
            assets: CampaignMapAssets::default(),
            frame: FrameWnd::default(),
            exhibit_grid: ExhibitGridNavigator::new(graph.nodes.len(), 0),
            graph,
            presentation: CampaignPresentationMode::ProgressTree,
            input: ModalInputState::new(),
            pseudo_debrief_at_ms: None,
            selected_classic: 0,
            selected_progress: 0,
            show_achievement_badges: true,
            achievement_overview: false,
            details_scroll: None,
            history_scroll: 0,
            selected_play: 0,
            replay_status: String::new(),
            recording_index,
            lifetime_totals: history.totals(),
            lifetime_achievements: history.achievement_aggregation(),
        }
    }

    fn key(keycode: Keycode) -> GameEvent {
        GameEvent::KeyDown {
            keycode,
            physical_key: None,
        }
    }

    #[test]
    fn browser_navigates_same_views_but_cannot_launch() {
        let mut state = browser();
        let transform = progress_transform(1024, 768);
        assert_eq!(
            state.handle_events(vec![key(Keycode::Down)], transform, true),
            None
        );
        assert_eq!(state.selected_progress, 1);
        for presentation in [
            CampaignPresentationMode::ProgressTree,
            CampaignPresentationMode::SherwoodMuseum,
        ] {
            state.presentation = presentation;
            for code in [Keycode::Return, Keycode::KpEnter, Keycode::Space] {
                assert_eq!(state.handle_events(vec![key(code)], transform, true), None);
            }
            let (x, y, _, _) =
                progress_node_rect(&state.graph, presentation, state.selected_progress, 0);
            assert_eq!(
                state.handle_events(
                    vec![GameEvent::MouseDown(x + 5, y + 5, 1, 0)],
                    transform,
                    true
                ),
                None
            );
            assert_eq!(state.selected_progress, 0);
            // The same selection still launches normally from Sherwood.
            assert_eq!(
                state.handle_events(vec![key(Keycode::Return)], transform, false),
                Some(CampaignMapChoice::SelectMission(1))
            );
        }
        assert_eq!(
            state.handle_events(vec![key(Keycode::Tab)], transform, true),
            None
        );
        assert_eq!(state.presentation, CampaignPresentationMode::ProgressTree);
        assert_eq!(
            state.handle_events(vec![key(Keycode::Escape)], transform, true),
            Some(CampaignMapChoice::Quit)
        );
    }

    #[test]
    fn large_campaign_cards_never_overlap_and_hit_testing_matches_the_viewport() {
        let mut state = browser();
        let template = state.graph.nodes[0].clone();
        state.graph.nodes = (0..62)
            .map(|index| {
                let mut node = template.clone();
                node.mission_idx = index;
                node.depth = index % 8;
                node.lane = index / 8;
                node
            })
            .collect();
        for presentation in [
            CampaignPresentationMode::ProgressTree,
            CampaignPresentationMode::SherwoodMuseum,
        ] {
            for selected in 0..state.graph.nodes.len() {
                assert!(card_visible(progress_node_rect(
                    &state.graph,
                    presentation,
                    selected,
                    selected
                )));
                let visible: Vec<_> = (0..state.graph.nodes.len())
                    .filter_map(|index| {
                        let rect = progress_node_rect(&state.graph, presentation, selected, index);
                        card_visible(rect).then_some((index, rect))
                    })
                    .collect();
                assert!(visible.len() <= PROGRESS_PAGE_SIZE);
                if presentation == CampaignPresentationMode::ProgressTree {
                    let spine = state
                        .graph
                        .nodes
                        .iter()
                        .position(|node| {
                            node.depth == state.graph.nodes[selected].depth && node.lane == 0
                        })
                        .unwrap();
                    assert!(visible.iter().any(|(index, _)| *index == spine));
                }
                for (a, &(index, (x, y, w, h))) in visible.iter().enumerate() {
                    assert_eq!(
                        progress_hit_test(
                            &state.graph,
                            presentation,
                            selected,
                            x + w / 2,
                            y + h / 2
                        ),
                        Some(index)
                    );
                    for &(_, (bx, by, bw, bh)) in &visible[a + 1..] {
                        assert!(x + w <= bx || bx + bw <= x || y + h <= by || by + bh <= y);
                    }
                }
            }
        }
    }

    #[test]
    fn pointer_tabs_and_back_work_without_launching_or_stale_gallery_selection() {
        let mut state = browser();
        let transform = progress_transform(1024, 768);
        state.selected_progress = 1;
        let click = |x, y| {
            let (x, y) = transform.to_screen(x, y);
            vec![GameEvent::MouseDown(x, y, 1, 1)]
        };
        assert_eq!(state.handle_events(click(250, 90), transform, true), None);
        assert_eq!(state.presentation, CampaignPresentationMode::SherwoodMuseum);
        assert_eq!(state.exhibit_grid.selected, 1);
        assert_eq!(state.handle_events(click(470, 90), transform, true), None);
        assert!(state.achievement_overview);
        assert_eq!(
            state.handle_events(vec![key(Keycode::Return)], transform, false),
            None
        );
        assert_eq!(state.handle_events(click(50, 90), transform, true), None);
        assert!(!state.achievement_overview);
        let (x, y, _, _) =
            progress_node_rect(&state.graph, state.presentation, state.selected_progress, 0);
        assert_eq!(
            state.handle_events(click(x + 5, y + 5), transform, false),
            None
        );
        assert_eq!(state.selected_progress, 0);
        assert_eq!(
            state.handle_events(click(900, 90), transform, true),
            Some(CampaignMapChoice::Quit)
        );
    }

    #[test]
    fn browser_with_no_missions_still_navigates_and_closes() {
        let mut state = browser();
        state.graph.nodes.clear();
        state.exhibit_grid = ExhibitGridNavigator::new(0, 0);
        let transform = progress_transform(1024, 768);
        for code in [
            Keycode::Down,
            Keycode::Return,
            Keycode::Tab,
            Keycode::Right,
            Keycode::Return,
        ] {
            assert_eq!(state.handle_events(vec![key(code)], transform, true), None);
        }
        assert_eq!(
            state.handle_events(vec![key(Keycode::Escape)], transform, true),
            Some(CampaignMapChoice::Quit)
        );
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod capture_tests {
    use super::*;
    use std::sync::Arc;

    /// Real campaign names and fonts, rendered through the production screen.
    /// Explicitly opt in because this requires game data and a GPU/software adapter.
    #[test]
    #[ignore = "requires game data and an offscreen wgpu adapter; see docs/CAMPAIGN_HISTORY.md"]
    fn capture_campaign_ui() {
        // Run this opt-in capture alone: the real resource loader uses the install root.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        std::env::set_current_dir(root).unwrap();
        let data = std::env::var("ROBINHOOD_DATA_DIR").expect("set ROBINHOOD_DATA_DIR");
        let output =
            std::env::var("ROBIN_UI_CAPTURE_DIR").unwrap_or_else(|_| "target/campaign-ui".into());
        std::fs::create_dir_all(&output).unwrap();
        let (campaign, profiles, context) =
            crate::main_entry::rust_init_with_data_dir(Some(std::path::Path::new(&data)))
                .expect("initialize capture content");
        let gpu = pollster::block_on(async {
            let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
            descriptor.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::VULKAN);
            let instance = Arc::new(wgpu::Instance::new(descriptor));
            let options = wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            };
            let adapter = instance
                .request_adapter(&options)
                .await
                .expect("offscreen adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("campaign UI capture"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                        .using_resolution(adapter.limits()),
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    trace: wgpu::Trace::Off,
                })
                .await
                .expect("offscreen device");
            crate::window::GpuContext {
                instance,
                adapter: Arc::new(adapter),
                device: Arc::new(device),
                queue: Arc::new(queue),
                surface_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            }
        });
        for (width, height) in [(1024, 768)] {
            let mut renderer = Renderer::offscreen(gpu.clone(), width, height);
            let mut resources = IngameMenuResources::new(
                &mut renderer,
                context.shipping().unwrap(),
                context.preparation_files().unwrap().clone(),
            )
            .expect("campaign menu assets");
            let mut state = CampaignMapModalState::new_browser(
                &context,
                &mut renderer,
                &campaign,
                &profiles,
                &mut resources,
            );
            assert!(
                !state.graph.nodes.is_empty(),
                "capture content must contain campaign missions"
            );
            serde_json::to_writer_pretty(
                std::fs::File::create(std::path::Path::new(&output).join("mission-profiles.json"))
                    .unwrap(),
                &profiles.missions,
            )
            .expect("capture mission metadata");
            // Keep real current-campaign availability, with synthetic archived
            // records to exercise loading an older save without losing badges.
            for (idx, node) in state.graph.nodes.iter_mut().enumerate() {
                node.lifetime_attempt_count = node.attempt_count;
                node.lifetime_win_count = node.win_count;
                // Older-save fixture: this mission has records but is not yet available here.
                if idx % 5 == 4 && node.kind.is_field_mission() {
                    node.plays = fixture_plays(13);
                    node.lifetime_attempt_count = node.plays.len();
                    node.lifetime_win_count = 2;
                    node.best.fastest_win_seconds = Some(185);
                    node.badges
                        .insert(robin_engine::achievement::AchievementId::CleanHands);
                    node.badge_count = node.badges.len();
                }
            }
            use robin_engine::achievement::{
                AchievementAggregationInput, AchievementAggregationSummary, AchievementId,
            };
            state.graph.campaign_achievements =
                AchievementAggregationSummary::from_inputs(|id| AchievementAggregationInput {
                    earned_missions: u32::from(id == AchievementId::CleanHands) * 2,
                    required_missions: 10,
                    envelope_unverifiable: id == AchievementId::Ruthless,
                    unverifiable_missions: u32::from(id == AchievementId::Ruthless),
                    ..Default::default()
                });
            state.graph.lifetime_achievements =
                AchievementAggregationSummary::from_inputs(|id| AchievementAggregationInput {
                    envelope_complete: true,
                    earned_missions: if matches!(
                        id,
                        AchievementId::CleanHands | AchievementId::PileOBones
                    ) {
                        10
                    } else {
                        0
                    },
                    required_missions: 10,
                    unverifiable_missions: u32::from(id == AchievementId::Ruthless),
                    ..Default::default()
                });
            state.lifetime_totals.attempts = state
                .graph
                .nodes
                .iter()
                .map(|node| node.lifetime_attempt_count as u64)
                .sum();
            state.lifetime_totals.wins = state
                .graph
                .nodes
                .iter()
                .map(|node| node.lifetime_win_count as u64)
                .sum();
            state.graph.completed_missions = state
                .graph
                .nodes
                .iter()
                .filter(|node| {
                    node.kind.is_field_mission() && node.state == MissionProgressState::Completed
                })
                .count();
            serde_json::to_writer_pretty(
                std::fs::File::create(std::path::Path::new(&output).join("campaign-graph.json"))
                    .unwrap(),
                &state.graph,
            )
            .expect("capture graph metadata");
            for (name, mode) in [
                ("tree", CampaignPresentationMode::ProgressTree),
                ("gallery", CampaignPresentationMode::SherwoodMuseum),
                ("achievements", CampaignPresentationMode::ProgressTree),
                ("details", CampaignPresentationMode::ProgressTree),
                ("details-more", CampaignPresentationMode::ProgressTree),
            ] {
                for selected in [
                    state
                        .graph
                        .nodes
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, node)| detail_lines(node, &state.assets).len())
                        .unwrap()
                        .0,
                    0,
                    4.min(state.graph.nodes.len() - 1),
                    state.graph.nodes.len() / 2,
                    state.graph.nodes.len() - 1,
                    state
                        .graph
                        .nodes
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, node)| node.lane)
                        .unwrap()
                        .0,
                ] {
                    renderer.begin_gpu_frame_clear();
                    renderer.begin_ui_only_frame();
                    render_campaign_progress(
                        &mut renderer,
                        progress_transform(width as i32, height as i32),
                        &state.graph,
                        selected,
                        mode,
                        &state.assets,
                        true,
                        state.lifetime_totals,
                        true,
                        name == "achievements",
                        name.starts_with("details")
                            .then_some(usize::from(name == "details-more")),
                        if name == "details-more" { 8 } else { 0 },
                        if name == "details-more" { 10 } else { 0 },
                        "",
                    );
                    let (w, h, pixels) = renderer.try_capture_frame_rgba().expect("read UI pixels");
                    let path = std::path::Path::new(&output)
                        .join(format!("{name}-{width}x{height}-{selected}.png"));
                    let file = std::fs::File::create(&path).unwrap();
                    let mut encoder = png::Encoder::new(file, w, h);
                    encoder.set_color(png::ColorType::Rgba);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder
                        .write_header()
                        .unwrap()
                        .write_image_data(&pixels)
                        .unwrap();
                    eprintln!("Captured {}", path.display());
                }
            }
        }
    }
}

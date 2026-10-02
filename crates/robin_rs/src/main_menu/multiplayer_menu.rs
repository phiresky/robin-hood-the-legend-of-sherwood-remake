//! Main-menu multiplayer screen: the matchmaking game browser plus
//! the pre-game lobby (hosted / joined waiting room).

use robin_engine::campaign::Campaign;

use crate::application::require;
use crate::gfx_types::{GameEvent, Keycode};
use crate::host::ApplicationContext;
use crate::ingame_menu::layout::{
    MENU_H, MENU_W, MenuRect, MenuTransform, TruncationMarker, draw_screen_background,
    render_text_virt_font, truncate_to_pixel_width_by, wrap_text_font,
};
use crate::ingame_menu::resources::IngameMenuResources;
use crate::ingame_menu::widget_bridge::{
    self, AnimatedScreenIo, ModalInputState, ModalScreenIo, ScreenFrame, ScreenKey,
};
use crate::localization::PortTextKey;
use crate::main_menu::custom_missions::CustomMissionLaunch;
use crate::multiplayer::matchmaking::{self, GameListing, JoinedGame};
use crate::renderer::Renderer;
use crate::scroll_view::ScrollView;
use crate::widget::{ColumnAlign, ColumnLayout, FrameWnd};
use robin_engine::profiles as engine_profiles;
use robin_engine::sprite::BBox;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const LIST_RECT: MenuRect = MenuRect {
    x: 28,
    y: 76,
    w: 400,
    h: 318,
};
const ROW_HEIGHT: i32 = 24;

const ID_JOIN: u32 = 0;
const ID_CREATE: u32 = 1;
const ID_START: u32 = 2;
const ID_BACK: u32 = 3;
const ID_LOCAL: u32 = 4;
const ID_RULE: u32 = 5;
const ID_SCALE: u32 = 6;
const ID_KEYBOARD: u32 = 7;
const ID_COPY_BASE: u32 = 10;
const ID_CHARACTER_BASE: u32 = 20;
const ID_REMOVE_BASE: u32 = 40;
const ID_ASSIGNMENTS: u32 = 8;
const ID_HERO_SETUP: u32 = 9;

const SCREEN: &str = "Multiplayer menu";

fn localized_text(application_context: &ApplicationContext, key: PortTextKey) -> &'static str {
    require(application_context.port_text(key), SCREEN)
}

fn localized_format(
    application_context: &ApplicationContext,
    key: PortTextKey,
    arguments: &[(&str, &str)],
) -> String {
    require(application_context.format_port_text(key, arguments), SCREEN)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum MultiplayerRole {
    /// Host on this install's persistent iroh identity (the id the
    /// matchmaking service advertised as the game's `connect_addr`).
    Host,
    Local,
    /// Join the host at the given iroh endpoint id.
    Client {
        connect_addr: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MultiplayerLaunch {
    #[serde(default)]
    pub coop: robin_engine::coop::CoopRules,
    #[serde(skip)]
    pub local_custom: Option<CustomMissionLaunch>,
    pub mission_id: u32,
    pub mission_name: String,
    pub role: MultiplayerRole,
    pub expected_players: u32,
    pub start_at_epoch_ms: Option<u64>,
    /// Exact canonical full-mod envelope. Hosts distribute it; interactive
    /// clients populate it from the validated durable cache before launch.
    #[serde(skip)]
    pub distributed_mod: Option<Arc<[u8]>>,
    /// Host-only installed locator for the same exact archives inside the
    /// canonical envelope. Clients restore by distributed-cache identity.
    #[serde(skip)]
    pub distributed_installed_locator:
        Option<robin_engine::mission_assets::InstalledArchiveLocator>,
}

#[derive(Debug, Clone)]
struct MissionChoice {
    mission_id: u32,
    #[cfg(target_arch = "wasm32")]
    authoritative_basename: String,
    mission_name: String,
    label: String,
    custom: Option<CustomMissionLaunch>,
    usual_team: Option<String>,
}

#[derive(Debug, Clone)]
enum MenuMode {
    Games,
    Missions,
    Hosted {
        game: GameListing,
    },
    Joined {
        game: JoinedGame,
        listing: Option<GameListing>,
    },
}

fn discard_disconnected_matchmaking_state(games: &mut Vec<GameListing>, mode: &mut MenuMode) {
    // Signed direct invites are independently dialable without discovery.
    games.retain(|game| game.state == "direct_invite");
    *mode = MenuMode::Games;
}

/// The button a keyboard Confirm or a row double-click triggers in `mode`.
fn activation_for_mode(mode: &MenuMode) -> Option<u32> {
    match mode {
        MenuMode::Games => Some(ID_JOIN),
        MenuMode::Missions => Some(ID_CREATE),
        MenuMode::Hosted { .. } => Some(ID_START),
        MenuMode::Joined { .. } => None,
    }
}

/// Borrowed campaign data the mission picker lists (not screen state).
pub(crate) struct MultiplayerMissionSources<'a> {
    pub campaign: &'a Campaign,
    pub profiles: &'a engine_profiles::ProfileManager,
}

pub(crate) async fn show_multiplayer_menu(
    application_context: &ApplicationContext,
    io: &mut AnimatedScreenIo<'_, '_>,
    sources: MultiplayerMissionSources<'_>,
    initial_direct_invite: Option<&str>,
) -> Option<MultiplayerLaunch> {
    let nickname = multiplayer_nickname(application_context);
    let missions = mission_choices(sources.campaign, sources.profiles, application_context);
    let initial_direct_error = if let Some(connect_addr) = initial_direct_invite {
        match prepare_direct_browser_launch(
            connect_addr,
            &missions,
            application_context,
            &mut io.screen_io(),
        )
        .await
        {
            Ok(launch) => return Some(launch),
            Err(error) => Some(error),
        }
    } else {
        None
    };
    let mut state = MultiplayerMenuState::new(
        nickname,
        missions,
        initial_direct_invite,
        initial_direct_error,
        io.resources,
    );
    loop {
        match state.tick(application_context, io).await {
            MultiplayerMenuTick::Finished(outcome) => return outcome,
            MultiplayerMenuTick::Refresh => continue,
            MultiplayerMenuTick::Pending => crate::window::sleep_ui_frame().await,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum MultiplayerMenuTick {
    Pending,
    /// A nested dialog or admission request finished; refresh without adding
    /// a present/sleep that the original event branch did not perform.
    Refresh,
    Finished(Option<MultiplayerLaunch>),
}

// Owns a live matchmaking worker and input/capture state, not serialized UI.
// Field order preserves the old locals' reverse-drop order: in particular,
// retire matchmaking before releasing its prepared content owner.
struct MultiplayerMenuState {
    coop: robin_engine::coop::CoopRules,
    local: bool,
    edit_assignments: bool,
    hero_setup: bool,
    team_slot: usize,
    team_focus: Option<u32>,
    frame: FrameWnd,
    input_state: ModalInputState,
    scroll_view: ScrollView,
    selected: usize,
    mode: MenuMode,
    status: String,
    games: Vec<GameListing>,
    matchmaking_label: String,
    matchmaking_client: Option<matchmaking::MatchmakingSession>,
    prepared_host_content: Option<crate::distributed_mod::PreparedDistributedMod>,
    missions: Vec<MissionChoice>,
}

impl MultiplayerMenuState {
    fn new(
        nickname: String,
        missions: Vec<MissionChoice>,
        initial_direct_invite: Option<&str>,
        initial_direct_error: Option<String>,
        resources: &IngameMenuResources,
    ) -> Self {
        let prepared_host_content: Option<crate::distributed_mod::PreparedDistributedMod> = None;
        let (matchmaking_client, matchmaking_label) =
            match matchmaking::MatchmakingSession::open(nickname.clone()) {
                Ok(session) => (
                    Some(session),
                    "Matchmaking: searching for players...".to_string(),
                ),
                Err(err) => {
                    tracing::warn!("Multiplayer matchmaking unavailable: {err}");
                    (None, err)
                }
            };
        let mut games = initial_direct_invite
            .and_then(signed_direct_listing)
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(listing) = direct_browser_listing()
            && games.iter().all(|existing| existing.id != listing.id)
        {
            games.push(listing);
        }
        let status = initial_direct_error.unwrap_or_else(|| matchmaking_label.clone());
        let mode = MenuMode::Games;
        let selected: usize = 0;
        let mut scroll_view = ScrollView::new(
            [
                LIST_RECT.x + 4,
                LIST_RECT.y + 4,
                LIST_RECT.w - 8,
                LIST_RECT.h - 8,
            ],
            ROW_HEIGHT,
            resources,
        );
        scroll_view.set_wheel_step(1);
        let input_state = ModalInputState::new();
        let frame = FrameWnd::interactive();
        Self {
            coop: robin_engine::coop::CoopRules {
                team: [b'R', 0, 0, 0, 0],
                ..Default::default()
            },
            local: false,
            edit_assignments: false,
            hero_setup: false,
            team_slot: 0,
            team_focus: None,
            missions,
            prepared_host_content,
            matchmaking_client,
            matchmaking_label,
            games,
            status,
            mode,
            selected,
            scroll_view,
            input_state,
            frame,
        }
    }

    // Await points only enter the same existing nested admission dialogs; the
    // worker drain, event dispatch, rendering and presentation order is fixed.
    async fn tick(
        &mut self,
        application_context: &ApplicationContext,
        io: &mut AnimatedScreenIo<'_, '_>,
    ) -> MultiplayerMenuTick {
        self.clamp_selection_to_rows();
        while let Some(event) = self.next_matchmaking_event() {
            if let Some(tick) = self
                .handle_matchmaking_event(event, application_context, &mut io.screen_io())
                .await
            {
                return tick;
            }
        }
        self.update_buttons(io.window.local_players.keyboard, io.resources);
        let (screen, activated) = self.handle_input(&mut io.screen_io());
        if let Some(id) = activated
            && let Some(tick) = self
                .activate(id, application_context, &mut io.screen_io())
                .await
        {
            return tick;
        }
        self.draw(application_context, io, &screen);
        MultiplayerMenuTick::Pending
    }

    fn clamp_selection_to_rows(&mut self) {
        let rows_len = match &self.mode {
            MenuMode::Games => self.games.len(),
            MenuMode::Missions => self.missions.len(),
            MenuMode::Hosted { .. } => 1,
            MenuMode::Joined { .. } => 1,
        };
        if rows_len == 0 {
            self.selected = 0;
            self.scroll_view.reset();
        } else if self.selected >= rows_len {
            self.selected = rows_len - 1;
        }
        self.scroll_view.set_total(rows_len);
    }

    fn next_matchmaking_event(&self) -> Option<matchmaking::MatchmakingEvent> {
        self.matchmaking_client
            .as_ref()
            .and_then(|client| match client.try_recv() {
                Ok(event) => event,
                Err(error) => Some(matchmaking::MatchmakingEvent::Disconnected(error)),
            })
    }

    /// Apply one matchmaking worker event; `None` keeps draining the queue.
    async fn handle_matchmaking_event(
        &mut self,
        event: matchmaking::MatchmakingEvent,
        application_context: &ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
    ) -> Option<MultiplayerMenuTick> {
        match event {
            matchmaking::MatchmakingEvent::Games(next) => {
                self.games = next;
                self.status = self.matchmaking_label.clone();
            }
            matchmaking::MatchmakingEvent::Created(created) => {
                self.publish_rules();
                self.status = "Game created. Press Start when ready.".to_string();
                self.mode = MenuMode::Hosted { game: created };
                self.selected = 0;
            }
            matchmaking::MatchmakingEvent::Joined(joined) => {
                self.coop = joined.coop;
                if joined.connect_addr.is_empty() {
                    self.status = "Matchmaking did not return a host address".to_string();
                } else if joined.start_at_epoch_ms.is_some() {
                    match prepare_joined_launch(joined, application_context, io).await {
                        Ok(launch) => return Some(MultiplayerMenuTick::Finished(Some(launch))),
                        Err(error) => self.status = error,
                    }
                } else {
                    let listing = self.games.iter().find(|g| g.id == joined.game_id).cloned();
                    self.status = "Joined game. Waiting for host to start...".to_string();
                    self.mode = MenuMode::Joined {
                        game: joined,
                        listing,
                    };
                    self.selected = 0;
                }
            }
            matchmaking::MatchmakingEvent::Started(started) => {
                if let MenuMode::Hosted { game, .. } = &self.mode
                    && game.id == started.game_id
                {
                    if let Some(advertised) = started.host_content.as_ref() {
                        let Some(prepared) = self.prepared_host_content.as_ref() else {
                            self.status = localized_text(
                                application_context,
                                PortTextKey::SpellforgeMpPreparedContentLost,
                            )
                            .to_owned();
                            return None;
                        };
                        let local_offer = match crate::distributed_mod::make_distributed_mod_offer(
                            &prepared.validated,
                            prepared.encoded.len() as u64,
                            advertised.host_endpoint_id.clone(),
                        ) {
                            Ok(offer) => offer,
                            Err(error) => {
                                self.status = localized_format(
                                    application_context,
                                    PortTextKey::SpellforgeMpCannotVerifyPreparedContent,
                                    &[("error", &error.to_string())],
                                );
                                return None;
                            }
                        };
                        if &local_offer != advertised {
                            self.status = localized_text(
                                application_context,
                                PortTextKey::SpellforgeMpPreparedContentChanged,
                            )
                            .to_owned();
                            return None;
                        }
                    }
                    return Some(MultiplayerMenuTick::Finished(Some(MultiplayerLaunch {
                        local_custom: None,
                        coop: started.coop,
                        mission_id: started.mission_id,
                        mission_name: application_context
                            .localized_mission_name(started.mission_id, &started.mission_name),
                        role: MultiplayerRole::Host,
                        expected_players: started.expected_players,
                        start_at_epoch_ms: started.start_at_epoch_ms,
                        distributed_mod: self
                            .prepared_host_content
                            .as_ref()
                            .map(|prepared| Arc::clone(&prepared.encoded)),
                        distributed_installed_locator: self
                            .prepared_host_content
                            .as_ref()
                            .map(|prepared| prepared.installed_locator.clone()),
                    })));
                }
            }
            matchmaking::MatchmakingEvent::GameUpdated(updated) => {
                if let MenuMode::Joined { game, .. } = &mut self.mode {
                    if game.game_id == updated.id {
                        self.coop = updated.coop;
                        game.coop = updated.coop;
                    }
                }
                let updated = upsert_game(&mut self.games, updated);
                match &mut self.mode {
                    MenuMode::Hosted { game, .. } if game.id == updated.id => {
                        let previous_players = game.players;
                        *game = updated.clone();
                        if previous_players != game.players {
                            self.coop.assignments = [0, 1, 2, 3, 4];
                        }
                        self.coop.players = game.players as u8;
                        if game.players != previous_players {
                            self.status = format!(
                                "{} player{} in game",
                                game.players,
                                if game.players == 1 { "" } else { "s" }
                            );
                        }
                    }
                    MenuMode::Joined { game, listing } if game.game_id == updated.id => {
                        self.status = format!(
                            "{} player{} in game. Waiting for host to start...",
                            updated.players,
                            if updated.players == 1 { "" } else { "s" }
                        );
                        *listing = Some(updated.clone());
                    }
                    _ => {}
                }
            }
            matchmaking::MatchmakingEvent::GameStarted(started) => {
                if let MenuMode::Joined { game, .. } = &self.mode
                    && game.game_id == started.game_id
                {
                    if let Err(error) = validate_started_game(game, &started) {
                        self.status = localized_format(
                            application_context,
                            PortTextKey::SpellforgeMpRejectedChangedStart,
                            &[("error", &error)],
                        );
                        return None;
                    }
                    match prepare_joined_launch(started, application_context, io).await {
                        Ok(launch) => return Some(MultiplayerMenuTick::Finished(Some(launch))),
                        Err(error) => self.status = error,
                    }
                }
            }
            matchmaking::MatchmakingEvent::Neighbors(count) => {
                self.matchmaking_label = if count == 0 {
                    "Matchmaking: searching for players...".to_string()
                } else {
                    format!(
                        "Matchmaking: {count} player{} online",
                        if count == 1 { "" } else { "s" }
                    )
                };
                if matches!(self.mode, MenuMode::Games) {
                    self.status = self.matchmaking_label.clone();
                }
            }
            matchmaking::MatchmakingEvent::Error(err) => self.status = err,
            matchmaking::MatchmakingEvent::Disconnected(err) => {
                self.status = err;
                self.matchmaking_client = None;
                // Discovery listings and hosted/joined controls are no longer
                // actionable. Signed direct invites use a different transport.
                discard_disconnected_matchmaking_state(&mut self.games, &mut self.mode);
                self.hero_setup = false;
                self.edit_assignments = false;
                self.team_focus = None;
                self.selected = 0;
                self.scroll_view.reset();
                self.prepared_host_content = None;
            }
        }
        None
    }

    fn publish_rules(&mut self) {
        if self.coop.validate().is_ok()
            && !self.local
            && let Some(session) = &self.matchmaking_client
        {
            if let Err(error) = session.set_rules(self.coop) {
                self.status = error;
            }
        }
    }

    fn update_buttons(&mut self, keyboard_player: bool, resources: &IngameMenuResources) {
        use robin_engine::coop::{CharacterControl, TEAM_CHARACTERS, team_character_name};
        let connected = self.matchmaking_client.is_some();
        let editing = matches!(self.mode, MenuMode::Missions | MenuMode::Hosted { .. });
        let (w, h) = resources.button_dimensions();
        let x = MENU_W - w - 10;
        let bottom = MENU_H - h - 10;
        let valid = self.coop.validate().is_ok();
        let mut buttons = Vec::new();
        let mut add = |id, label: String, enabled, bx, y, bw, bh| {
            buttons.push((id, label, enabled, bx, y, bw, bh));
        };
        if editing && self.hero_setup {
            let count = self.coop.team_len();
            self.team_slot = self.team_slot.min(count).min(4);
            for slot in 0..5 {
                let y = 90 + slot as i32 * 58;
                let label = if slot < count {
                    format!(
                        "{} {}. {}",
                        if slot == self.team_slot { ">" } else { "" },
                        slot + 1,
                        team_character_name(self.coop.team[slot])
                    )
                } else if slot == count {
                    format!(
                        "{} {}. Add character",
                        if slot == self.team_slot { ">" } else { "" },
                        slot + 1
                    )
                } else {
                    format!("{}. Empty slot", slot + 1)
                };
                add(
                    ID_COPY_BASE + slot as u32,
                    label,
                    slot <= count,
                    40,
                    y,
                    260,
                    28,
                );
                if slot < count {
                    add(
                        ID_REMOVE_BASE + slot as u32,
                        "Remove".into(),
                        count > 1,
                        316,
                        y,
                        100,
                        28,
                    );
                }
            }
            for (index, &(_, name, _)) in TEAM_CHARACTERS.iter().enumerate() {
                add(
                    ID_CHARACTER_BASE + index as u32,
                    name.into(),
                    true,
                    x,
                    84 + index as i32 * 29,
                    w,
                    25,
                );
            }
            add(
                ID_HERO_SETUP,
                "Mission / rules".into(),
                true,
                28,
                bottom,
                190,
                h,
            );
        } else {
            match &self.mode {
                MenuMode::Games => {
                    let can_join = self
                        .games
                        .get(self.selected)
                        .is_some_and(|game| connected || game.state == "direct_invite");
                    add(ID_JOIN, "Join game".into(), can_join, x, 76, w, h);
                    add(
                        ID_CREATE,
                        "Host online".into(),
                        connected,
                        x,
                        76 + h + 12,
                        w,
                        h,
                    );
                    add(
                        ID_LOCAL,
                        "Local co-op".into(),
                        true,
                        x,
                        76 + 2 * (h + 12),
                        w,
                        h,
                    );
                }
                _ => {}
            }
            if editing {
                let mut y = 76;
                if self.local {
                    add(
                        ID_KEYBOARD,
                        if keyboard_player {
                            "Keyboard: on"
                        } else {
                            "Keyboard: off"
                        }
                        .into(),
                        true,
                        x,
                        y,
                        w,
                        h,
                    );
                    y += h + 12;
                }
                add(
                    ID_RULE,
                    match self.coop.control {
                        CharacterControl::Shared => "Control: shared",
                        CharacterControl::Exclusive => "Control: exclusive",
                        CharacterControl::Assigned => "Control: assigned",
                    }
                    .into(),
                    true,
                    x,
                    y,
                    w,
                    h,
                );
                y += h + 12;
                add(
                    ID_SCALE,
                    format!("Enemy HP: +{}%", self.coop.enemy_health_per_duplicate),
                    true,
                    x,
                    y,
                    w,
                    h,
                );
                y += h + 12;
                add(
                    ID_HERO_SETUP,
                    format!("Edit team ({}/5)", self.coop.team_len()),
                    true,
                    x,
                    y,
                    w,
                    h,
                );
                y += h + 12;
                if self.coop.control == CharacterControl::Assigned {
                    add(
                        ID_ASSIGNMENTS,
                        if self.edit_assignments {
                            "Choose mission"
                        } else {
                            "Assign players"
                        }
                        .into(),
                        true,
                        x,
                        y,
                        w,
                        h,
                    );
                }
                if self.edit_assignments {
                    for slot in 0..self.coop.players as usize {
                        let choice = self.coop.assignments[slot] as usize;
                        let name = self
                            .coop
                            .team
                            .get(choice)
                            .copied()
                            .filter(|&code| code != 0)
                            .map(team_character_name)
                            .unwrap_or("Unassigned");
                        add(
                            ID_COPY_BASE + slot as u32,
                            format!("Player {}: slot {} - {}", slot + 1, choice + 1, name),
                            true,
                            40,
                            90 + slot as i32 * 58,
                            376,
                            30,
                        );
                    }
                }
            }
        }
        match self.mode {
            MenuMode::Missions => add(
                ID_CREATE,
                if self.hero_setup {
                    if self.local {
                        "Start local"
                    } else {
                        "Create lobby"
                    }
                } else {
                    "Review team"
                }
                .into(),
                valid && !self.missions.is_empty() && (self.local || connected),
                x,
                bottom - h - 12,
                w,
                h,
            ),
            MenuMode::Hosted { .. } => add(
                ID_START,
                if self.hero_setup {
                    "Start mission"
                } else {
                    "Review team"
                }
                .into(),
                valid && connected,
                x,
                bottom - h - 12,
                w,
                h,
            ),
            _ => {}
        }
        add(
            ID_BACK,
            if matches!(self.mode, MenuMode::Games) {
                "Back"
            } else {
                "Back to games"
            }
            .into(),
            true,
            x,
            bottom,
            w,
            h,
        );
        let obsolete: Vec<_> = self
            .frame
            .widgets()
            .iter()
            .map(|widget| widget.id())
            .filter(|id| !buttons.iter().any(|(next, ..)| next == id))
            .collect();
        for id in obsolete {
            self.frame.remove_widget(id);
        }
        for (id, label, enabled, bx, y, bw, bh) in buttons {
            if self.frame.widget(id).is_none() {
                self.frame
                    .add_widget_absolute(widget_bridge::make_button_enabled(
                        id, &label, enabled, bx, y, bw, bh,
                    ));
            } else {
                self.frame.update_widget(id, Some(&label), enabled);
                self.frame
                    .widget_mut(id)
                    .expect("existing menu button")
                    .base_mut()
                    .set_position(robin_engine::coordinates::ScreenBBox::from_coords(
                        bx as f32,
                        y as f32,
                        (bx + bw) as f32,
                        (y + bh) as f32,
                    ));
            }
        }
    }

    fn move_team_focus(&mut self, key: Keycode) {
        let mut buttons: Vec<_> = self
            .frame
            .widgets()
            .iter()
            .filter(|w| w.base().enabled)
            .collect();
        buttons.sort_by_key(|widget| {
            let id = widget.id();
            let group = if (ID_COPY_BASE..ID_COPY_BASE + 5).contains(&id)
                || (ID_REMOVE_BASE..ID_REMOVE_BASE + 5).contains(&id)
            {
                0
            } else if (ID_CHARACTER_BASE..ID_CHARACTER_BASE + 10).contains(&id) {
                1
            } else {
                2
            };
            let bounds = widget.base().bbox.0.expect("button bounds");
            (group, bounds.min().y as i32, bounds.min().x as i32)
        });
        let Some(current) = buttons.iter().position(|w| Some(w.id()) == self.team_focus) else {
            self.team_focus = buttons.first().map(|w| w.id());
            return;
        };
        if key == Keycode::Tab {
            self.team_focus = Some(buttons[(current + 1) % buttons.len()].id());
            return;
        }
        let center = buttons[current]
            .base()
            .bbox
            .0
            .expect("button bounds")
            .center();
        let next = buttons
            .iter()
            .filter_map(|w| {
                let point = w.base().bbox.0.expect("button bounds").center();
                let dx = point.x - center.x;
                let dy = point.y - center.y;
                let (forward, cross) = match key {
                    Keycode::Up => (-dy, dx),
                    Keycode::Down => (dy, dx),
                    Keycode::Left => (-dx, dy),
                    Keycode::Right => (dx, dy),
                    _ => return None,
                };
                (forward > 1.0).then_some((w.id(), forward + cross.abs() * 3.0))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = next {
            self.team_focus = Some(id);
        }
    }

    fn draw_buttons(
        &self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        transform: MenuTransform,
    ) {
        for widget in self.frame.widgets() {
            widget_bridge::draw_widget_button(
                renderer,
                resources,
                transform,
                widget,
                (self.hero_setup || self.edit_assignments) && self.team_focus == Some(widget.id()),
            );
        }
    }

    /// Poll and route this frame's input. Scroll-view hit testing reads the
    /// live cursor between events, so input updates stay interleaved.
    fn handle_input(&mut self, io: &mut ModalScreenIo<'_, '_>) -> (ScreenFrame, Option<u32>) {
        let rows_len = match &self.mode {
            MenuMode::Games => self.games.len(),
            MenuMode::Missions => self.missions.len(),
            MenuMode::Hosted { .. } | MenuMode::Joined { .. } => 1,
        };
        self.scroll_view.set_total(rows_len);
        self.selected = self.selected.min(rows_len.saturating_sub(1));
        let mut activated: Option<u32> = None;
        let screen = ScreenFrame::poll(io);
        let mut players_joined = false;
        if self.local {
            let before = io.window.local_players.count();
            for event in &screen.events {
                io.window.local_players.join_event(event);
            }
            players_joined = io.window.local_players.count() > before;
            let count = io.window.local_players.count().max(1) as u8;
            if self.coop.players != count {
                self.coop.assignments = [0, 1, 2, 3, 4];
            }
            self.coop.players = count;
            self.status = format!(
                "{} player(s) joined. Keyboard {}. Press {} on a controller to join.",
                io.window.local_players.count(),
                if io.window.local_players.keyboard {
                    "on"
                } else {
                    "off"
                },
                crate::gfx_types::GamepadButton::South.ui_name()
            );
        }
        let transform = screen.transform;
        for event in &screen.events {
            self.input_state.update_from_event(event, transform);
            if self.local {
                match event {
                    // A is the local-lobby join button. Consume it here so
                    // the generic Confirm path cannot start the mission in
                    // the same frame.
                    GameEvent::GamepadButton {
                        button: crate::gfx_types::GamepadButton::South,
                        pressed: true,
                        ..
                    } if players_joined || (!self.hero_setup && !self.edit_assignments) => continue,
                    // B removes the controller that pressed it. With no
                    // controller left, retain the normal Back behavior.
                    GameEvent::GamepadButton {
                        button: crate::gfx_types::GamepadButton::East,
                        pressed: true,
                        ..
                    } if io.window.local_players.leave_event(event) => continue,
                    _ => {}
                }
            }
            if self.hero_setup || self.edit_assignments {
                let key = self
                    .input_state
                    .gamepad_direction(event)
                    .or_else(|| match event {
                        GameEvent::KeyDown { keycode, .. } => Some(*keycode),
                        _ => None,
                    });
                if let Some(
                    key @ (Keycode::Up
                    | Keycode::Down
                    | Keycode::Left
                    | Keycode::Right
                    | Keycode::Tab),
                ) = key
                {
                    self.move_team_focus(key);
                }
                match ScreenKey::from_event(event) {
                    Some(ScreenKey::Cancel) => {
                        activated = Some(if self.hero_setup {
                            ID_HERO_SETUP
                        } else {
                            ID_ASSIGNMENTS
                        })
                    }
                    Some(ScreenKey::Quit) => activated = Some(ID_BACK),
                    Some(ScreenKey::Confirm) => {
                        activated = self
                            .team_focus
                            .filter(|&id| self.frame.widget(id).is_some_and(|w| w.base().enabled))
                    }
                    _ => {}
                }
                if matches!(event, GameEvent::MouseMove { .. }) {
                    self.team_focus = None;
                }
                continue;
            }
            if let Some(direction) = self.input_state.gamepad_direction(event) {
                match direction {
                    Keycode::Up => {
                        self.selected = self.selected.saturating_sub(1);
                        if rows_len > 0 {
                            self.scroll_view.reveal(self.selected);
                        }
                    }
                    Keycode::Down => {
                        if rows_len > 0 {
                            self.selected = (self.selected + 1).min(rows_len - 1);
                            self.scroll_view.reveal(self.selected);
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if self.scroll_view.handle_event(
                event,
                transform,
                (
                    self.input_state.virt_x as i32,
                    self.input_state.virt_y as i32,
                ),
            ) {
                continue;
            }
            match ScreenKey::from_event(event) {
                Some(ScreenKey::Quit | ScreenKey::Cancel) => activated = Some(ID_BACK),
                Some(ScreenKey::Confirm) => activated = activation_for_mode(&self.mode),
                _ => match event {
                    GameEvent::KeyDown {
                        keycode: Keycode::Up,
                        ..
                    } => {
                        self.selected = self.selected.saturating_sub(1);
                        if rows_len > 0 {
                            self.scroll_view.reveal(self.selected);
                        }
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::Down,
                        ..
                    } => {
                        if rows_len > 0 {
                            self.selected = (self.selected + 1).min(rows_len - 1);
                            self.scroll_view.reveal(self.selected);
                        }
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::PageUp,
                        ..
                    } => {
                        let step = self.scroll_view.visible_count().saturating_sub(1).max(1);
                        self.selected = self.selected.saturating_sub(step);
                        if rows_len > 0 {
                            self.scroll_view.reveal(self.selected);
                        }
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::PageDown,
                        ..
                    } => {
                        if rows_len > 0 {
                            let step = self.scroll_view.visible_count().saturating_sub(1).max(1);
                            self.selected = (self.selected + step).min(rows_len - 1);
                            self.scroll_view.reveal(self.selected);
                        }
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::Home,
                        ..
                    } => {
                        self.selected = 0;
                        if rows_len > 0 {
                            self.scroll_view.reveal(self.selected);
                        }
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::End,
                        ..
                    } => {
                        if rows_len > 0 {
                            self.selected = rows_len - 1;
                            self.scroll_view.reveal(self.selected);
                        }
                    }
                    GameEvent::MouseUp(x, y, 1) => {
                        let (vx, vy) = transform.from_screen(*x, *y);
                        if let Some(row) = self.scroll_view.row_at(vx, vy) {
                            self.selected = row;
                        }
                    }
                    GameEvent::MouseDown(x, y, 1, clicks) if *clicks >= 2 => {
                        let (vx, vy) = transform.from_screen(*x, *y);
                        if let Some(row) = self.scroll_view.row_at(vx, vy) {
                            self.selected = row;
                            activated = activation_for_mode(&self.mode);
                        }
                    }
                    _ => {}
                },
            }
        }

        let (_, widget_activated) = ScreenFrame::dispatch(&mut self.input_state, &mut self.frame);
        if let Some(id) = widget_activated {
            activated = Some(id);
        }
        (screen, activated)
    }

    /// Run an activated browser/back button; `None` continues to drawing.
    async fn activate(
        &mut self,
        id: u32,
        application_context: &ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
    ) -> Option<MultiplayerMenuTick> {
        match id {
            ID_HERO_SETUP => {
                self.hero_setup = !self.hero_setup;
                self.edit_assignments = false;
                self.team_focus = None;
                return None;
            }
            ID_LOCAL => {
                self.coop.players = 1;
                if self.coop.team_len() == 0 {
                    self.coop.team[0] = b'R';
                }
                self.hero_setup = false;
                self.edit_assignments = false;
                self.team_focus = None;
                self.scroll_view.reset();
                self.local = true;
                self.mode = MenuMode::Missions;
                self.selected = 0;
                io.window.local_players = Default::default();
                #[cfg(feature = "gamepad")]
                if let Some(gamepads) = io.window.gamepads.as_ref() {
                    for (id, _) in gamepads.gamepads() {
                        io.window.local_players.join_device(usize::from(id) as u32);
                    }
                }
                self.status =
                    "Local co-op: press A on each controller to join. Choose a mission and rules."
                        .into();
                return None;
            }
            ID_RULE => {
                use robin_engine::coop::CharacterControl::*;
                self.coop.control = match self.coop.control {
                    Shared => Exclusive,
                    Exclusive => Assigned,
                    Assigned => Shared,
                };
                self.publish_rules();
                return None;
            }
            ID_SCALE => {
                self.coop.enemy_health_per_duplicate =
                    (self.coop.enemy_health_per_duplicate + 25) % 125;
                self.publish_rules();
                return None;
            }
            ID_KEYBOARD => {
                if self.local
                    && (!io.window.local_players.keyboard && io.window.local_players.count() < 5
                        || io.window.local_players.keyboard)
                {
                    io.window.local_players.keyboard = !io.window.local_players.keyboard;
                }
                return None;
            }
            ID_ASSIGNMENTS => {
                self.edit_assignments = !self.edit_assignments;
                self.team_focus = None;
                return None;
            }
            id if (ID_CHARACTER_BASE
                ..ID_CHARACTER_BASE + robin_engine::coop::TEAM_CHARACTERS.len() as u32)
                .contains(&id) =>
            {
                let slot = self.team_slot.min(self.coop.team_len()).min(4);
                self.coop.team[slot] =
                    robin_engine::coop::TEAM_CHARACTERS[(id - ID_CHARACTER_BASE) as usize].0;
                self.publish_rules();
                return None;
            }
            id if (ID_REMOVE_BASE..ID_REMOVE_BASE + 5).contains(&id) => {
                let slot = (id - ID_REMOVE_BASE) as usize;
                let count = self.coop.team_len();
                if count > 1 && slot < count {
                    self.coop.team[slot..].rotate_left(1);
                    self.coop.team[4] = 0;
                    self.coop.assignments = [0, 1, 2, 3, 4];
                    self.team_slot = self.team_slot.min(count - 2);
                    self.publish_rules();
                }
                return None;
            }
            id if (ID_COPY_BASE..ID_COPY_BASE + 5).contains(&id) => {
                let slot = (id - ID_COPY_BASE) as usize;
                if self.hero_setup {
                    self.team_slot = slot.min(self.coop.team_len()).min(4);
                } else if self.edit_assignments {
                    let count = self.coop.team_len().max(1) as u8;
                    let next = (self.coop.assignments[slot] + 1) % count;
                    let other = self
                        .coop
                        .assignments
                        .iter()
                        .position(|&assigned| assigned == next)
                        .expect("assignment permutation");
                    self.coop.assignments.swap(slot, other);
                    self.publish_rules();
                }
                return None;
            }
            ID_CREATE | ID_START if !matches!(self.mode, MenuMode::Games) && !self.hero_setup => {
                self.hero_setup = true;
                self.edit_assignments = false;
                self.team_focus = None;
                return None;
            }
            ID_BACK => match self.mode {
                MenuMode::Games => return Some(MultiplayerMenuTick::Finished(None)),
                _ => {
                    if matches!(self.mode, MenuMode::Hosted { .. } | MenuMode::Joined { .. })
                        && let Some(session) = self.matchmaking_client.as_ref()
                        && let Err(err) = session.leave_game()
                    {
                        tracing::warn!("matchmaking leave failed: {err}");
                    }
                    self.hero_setup = false;
                    self.edit_assignments = false;
                    self.team_focus = None;
                    self.local = false;
                    io.window.local_players.enabled = false;
                    self.mode = MenuMode::Games;
                    self.selected = 0;
                    self.scroll_view.reset();
                    self.status = self.matchmaking_label.clone();
                }
            },
            ID_JOIN if matches!(self.mode, MenuMode::Games) => {
                if let Some(game) = self.games.get(self.selected) {
                    if game.state == "direct_invite" {
                        match prepare_direct_browser_launch(
                            game.connect_addr(),
                            &self.missions,
                            application_context,
                            io,
                        )
                        .await
                        {
                            Ok(launch) => {
                                return Some(MultiplayerMenuTick::Finished(Some(launch)));
                            }
                            Err(error) => self.status = error,
                        }
                        return Some(MultiplayerMenuTick::Refresh);
                    }
                    match self
                        .matchmaking_client
                        .as_ref()
                        .map(|session| session.join_game(game.id.clone()))
                    {
                        Some(Ok(())) => {
                            self.status = "Joining game...".to_string();
                        }
                        Some(Err(err)) => self.status = err,
                        None => self.status = "Matchmaking is not connected".to_string(),
                    }
                }
            }
            ID_CREATE if matches!(self.mode, MenuMode::Games) => {
                self.coop.players = 1;
                if self.coop.team_len() == 0 {
                    self.coop.team[0] = b'R';
                }
                if self.missions.is_empty() {
                    self.status = "No missions are available to host".to_string();
                } else {
                    self.mode = MenuMode::Missions;
                    self.selected = 0;
                    self.scroll_view.reset();
                    self.status = "Select a mission for the hosted game".to_string();
                }
            }
            _ => return self.activate_host_action(id, application_context, io).await,
        }
        None
    }

    /// Run an activated mission-create/start button (the `match` tail of
    /// [`Self::activate`], in the same arm order); `None` continues to drawing.
    async fn activate_host_action(
        &mut self,
        id: u32,
        application_context: &ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
    ) -> Option<MultiplayerMenuTick> {
        match id {
            ID_CREATE if self.local && matches!(self.mode, MenuMode::Missions) => {
                let player_count = io.window.local_players.count();
                if player_count == 0 {
                    self.status = "Press A to join, or enable a keyboard player.".into();
                    return None;
                }
                if let Some(mission) = self.missions.get(self.selected) {
                    io.window.local_players.enabled = true;
                    self.coop.players = player_count as u8;
                    return Some(MultiplayerMenuTick::Finished(Some(MultiplayerLaunch {
                        coop: self.coop,
                        mission_id: mission.mission_id,
                        mission_name: mission.mission_name.clone(),
                        role: MultiplayerRole::Local,
                        expected_players: player_count as u32,
                        start_at_epoch_ms: None,
                        local_custom: mission.custom.clone(),
                        distributed_mod: None,
                        distributed_installed_locator: None,
                    })));
                }
            }
            ID_CREATE if matches!(self.mode, MenuMode::Missions) => {
                if let Some(mission) = self.missions.get(self.selected).cloned() {
                    if let Some(_custom) = mission.custom.as_ref() {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            let custom = _custom;
                            let host_id =
                                match crate::multiplayer::identity::local_endpoint_id_string() {
                                    Ok(id) => id,
                                    Err(error) => {
                                        self.status = error.to_string();
                                        return Some(MultiplayerMenuTick::Refresh);
                                    }
                                };
                            let attestation = crate::ingame_menu::spellforge_content::show_host_distribution_attestation(
                                    application_context,
                                    io,
                                    custom,
                                    &host_id,
                                )
                                .await;
                            let attestation = match attestation {
                                Ok(attestation) => attestation,
                                Err(error) => {
                                    self.status = localized_format(
                                        application_context,
                                        PortTextKey::SpellforgeMpCannotReviewMetadata,
                                        &[("error", &error)],
                                    );
                                    return Some(MultiplayerMenuTick::Refresh);
                                }
                            };
                            let Some(license) = attestation else {
                                self.status = localized_text(
                                    application_context,
                                    PortTextKey::SpellforgeMpHostingCancelled,
                                )
                                .to_owned();
                                return Some(MultiplayerMenuTick::Refresh);
                            };
                            let prepared =
                                match crate::distributed_mod::prepare_local_distributed_mod(
                                    custom,
                                    Some(license),
                                ) {
                                    Ok(prepared) => prepared,
                                    Err(error) => {
                                        self.status = localized_format(
                                            application_context,
                                            PortTextKey::SpellforgeMpCannotHostMission,
                                            &[("error", &error)],
                                        );
                                        return Some(MultiplayerMenuTick::Refresh);
                                    }
                                };
                            let offer = match crate::distributed_mod::make_distributed_mod_offer(
                                &prepared.validated,
                                prepared.encoded.len() as u64,
                                host_id,
                            ) {
                                Ok(offer) => offer,
                                Err(error) => {
                                    self.status = localized_format(
                                        application_context,
                                        PortTextKey::SpellforgeMpCannotAdvertiseMission,
                                        &[("error", &error.to_string())],
                                    );
                                    return Some(MultiplayerMenuTick::Refresh);
                                }
                            };
                            match self.matchmaking_client.as_ref().map(|session| {
                                session.create_game_with_content(
                                    mission.mission_id,
                                    mission.mission_name.clone(),
                                    offer,
                                )
                            }) {
                                Some(Ok(())) => {
                                    self.prepared_host_content = Some(prepared);
                                    self.status = localized_text(
                                        application_context,
                                        PortTextKey::SpellforgeMpCreatingCustomGame,
                                    )
                                    .to_owned();
                                }
                                Some(Err(error)) => self.status = error,
                                None => self.status = "Matchmaking is not connected".to_owned(),
                            }
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            // Browsers never open the host attestation dialog.
                            let _ = io;
                            self.status = localized_text(
                                application_context,
                                PortTextKey::SpellforgeMpBrowserCannotHost,
                            )
                            .to_owned();
                        }
                    } else {
                        self.prepared_host_content = None;
                        match self.matchmaking_client.as_ref().map(|session| {
                            session.create_game(mission.mission_id, mission.mission_name.clone())
                        }) {
                            Some(Ok(())) => self.status = "Creating game...".to_string(),
                            Some(Err(err)) => self.status = err,
                            None => self.status = "Matchmaking is not connected".to_string(),
                        }
                    }
                }
            }
            ID_START => {
                if matches!(self.mode, MenuMode::Hosted { .. }) {
                    self.publish_rules();
                    match self
                        .matchmaking_client
                        .as_ref()
                        .map(|session| session.start_game())
                    {
                        Some(Ok(())) => self.status = "Starting game...".to_string(),
                        Some(Err(err)) => self.status = err,
                        None => self.status = "Matchmaking is not connected".to_string(),
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn draw(
        &mut self,
        application_context: &ApplicationContext,
        io: &mut AnimatedScreenIo<'_, '_>,
        screen: &ScreenFrame,
    ) {
        let transform = screen.transform;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        screen.begin_draw(renderer);
        if let Some(bg) = resources.menu_bg[2] {
            draw_screen_background(renderer, &bg);
        }
        self.scroll_view.set_total(match &self.mode {
            MenuMode::Games => self.games.len(),
            MenuMode::Missions => self.missions.len(),
            MenuMode::Hosted { .. } | MenuMode::Joined { .. } => 1,
        });
        self.update_buttons(io.window.local_players.keyboard, resources);
        self.render_menu(renderer, resources, transform, application_context);
        self.draw_buttons(renderer, resources, transform);
        if let Some(cursor) = io.cursor_renderer() {
            cursor.advance_ui_animation();
        }
        screen.finish(&mut io.screen_io(), &self.input_state);
    }
}

fn validate_started_game(selected: &JoinedGame, started: &JoinedGame) -> Result<(), String> {
    if started.start_at_epoch_ms.is_none() {
        return Err("start signal has no synchronized start time".to_owned());
    }
    if started.game_id != selected.game_id
        || started.connect_addr != selected.connect_addr
        || started.mission_id != selected.mission_id
        || started.mission_name != selected.mission_name
        || started.host_content != selected.host_content
    {
        return Err(
            "host identity, mission, or exact content differs from the selected listing".to_owned(),
        );
    }
    Ok(())
}

async fn prepare_joined_launch(
    joined: JoinedGame,
    application_context: &ApplicationContext,
    io: &mut ModalScreenIo<'_, '_>,
) -> Result<MultiplayerLaunch, String> {
    let distributed_mod = match joined.host_content.as_ref() {
        None => None,
        Some(advertised) => Some(
            preflight_host_content(&joined.connect_addr, advertised, application_context, io)
                .await?,
        ),
    };
    Ok(MultiplayerLaunch {
        local_custom: None,
        coop: joined.coop,
        mission_id: joined.mission_id,
        mission_name: application_context
            .localized_mission_name(joined.mission_id, &joined.mission_name),
        role: MultiplayerRole::Client {
            connect_addr: joined.connect_addr,
        },
        expected_players: joined.expected_players,
        start_at_epoch_ms: joined.start_at_epoch_ms,
        distributed_mod,
        distributed_installed_locator: None,
    })
}

#[cfg(target_arch = "wasm32")]
fn direct_browser_listing() -> Option<GameListing> {
    let params = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())?;
    let address = params.get("connect")?;
    signed_direct_listing(&address)
}

fn signed_direct_listing(address: &str) -> Option<GameListing> {
    let address = address.trim();
    if address.is_empty() {
        return None;
    }
    let ticket =
        match crate::multiplayer::join_ticket::BrowserJoinTicket::decode_authenticated(address) {
            Ok(ticket) => ticket,
            Err(error) => {
                tracing::warn!(%error, "ignoring invalid signed browser direct invite");
                return None;
            }
        };
    let payload = ticket.payload();
    Some(GameListing {
        coop: Default::default(),
        id: address.to_owned(),
        mission_id: payload.mission_profile_id.unwrap_or(u32::MAX),
        mission_name: payload.mission_id.clone(),
        host_content: None,
        host: payload.host_endpoint_id.clone(),
        players: 0,
        max_players: payload.expected_players,
        state: "direct_invite".to_owned(),
        start_at_epoch_ms: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn direct_browser_listing() -> Option<GameListing> {
    None
}

/// Resolve a stable-shell `#join=<signed-ticket>` invite through the
/// authenticated iroh handshake. A custom game performs the full explicit
/// consent/no-seat preflight before returning exact bytes. A vanilla probe may
/// transiently receive a reconnect-owned seat, then deliberately disconnects
/// and reuses the same durable public-key identity for the real mission
/// connection.
#[cfg(target_arch = "wasm32")]
async fn prepare_direct_browser_launch(
    connect_addr: &str,
    missions: &[MissionChoice],
    application_context: &ApplicationContext,
    io: &mut ModalScreenIo<'_, '_>,
) -> Result<MultiplayerLaunch, String> {
    let ticket =
        crate::multiplayer::join_ticket::BrowserJoinTicket::decode_authenticated(connect_addr)
            .map_err(|error| error.to_string())?;
    let expected_players = ticket.payload().expected_players;
    let (mut channels, incoming_tx, outgoing_rx, _frame_cursor, _snapshot) =
        crate::multiplayer::NetChannels::new();
    let handle = crate::multiplayer::connect_client(
        connect_addr,
        multiplayer_nickname(application_context),
        incoming_tx,
        outgoing_rx,
    )
    .map_err(|error| {
        localized_format(
            application_context,
            PortTextKey::SpellforgeMpConnectDirectInvite,
            &[("error", &error.to_string())],
        )
    })?;
    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(15);
    let mut probe_events = Vec::new();
    while handle.content_offer().is_none()
        && handle.session_metadata().is_none()
        && web_time::Instant::now() < deadline
    {
        match channels.try_recv_event() {
            Ok(crate::multiplayer::NetEvent::Fatal(error)) => return Err(error.to_string()),
            Ok(event) => probe_events.push(event),
            Err(std::sync::mpsc::TryRecvError::Empty) => crate::window::sleep_ms(10).await,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(localized_text(
                    application_context,
                    PortTextKey::SpellforgeMpDirectTransportClosed,
                )
                .to_owned());
            }
        }
    }
    if handle.content_offer().is_none() && handle.session_metadata().is_none() {
        return Err(localized_text(
            application_context,
            PortTextKey::SpellforgeMpDirectResolveTimeout,
        )
        .to_owned());
    }
    let authenticated_offer = handle.content_offer();
    let welcomed_mission = handle
        .session_metadata()
        .map(|metadata| metadata.mission_id);
    channels.attach_runtime(handle);
    channels.defer_events(probe_events);

    let (mission_name, distributed_mod) = match authenticated_offer {
        Some(offer) => {
            let mission_name = offer.mission_basename.clone();
            let (key, metadata) = crate::distributed_mod::offer_trust_identity(&offer)?;
            let outcome = crate::ingame_menu::spellforge_content::show_spellforge_consent(
                application_context,
                io,
                key,
                metadata,
            )
            .await?;
            if outcome
                == crate::ingame_menu::spellforge_content::SpellforgeConsentOutcome::Cancelled
            {
                channels.reject_content(
                    offer.full_mod_sha256,
                    "player cancelled exact direct-invite content approval".to_owned(),
                );
                channels.shutdown();
                return Err(localized_text(
                    application_context,
                    PortTextKey::SpellforgeMpDirectJoinCancelled,
                )
                .to_owned());
            }
            let admitted = crate::distributed_mod_admission::admit_trusted_distributed_mod(
                application_context,
                &channels,
                &offer,
                crate::distributed_mod_admission::DistributedModAdmissionPurpose::PrepareOnly,
            )
            .await?;
            let encoded = admitted.cache_lease.encoded_arc();
            await_prepared_confirmation(application_context, &channels).await?;
            drop(admitted);
            (mission_name, Some(encoded))
        }
        None => {
            let mission_name = welcomed_mission.ok_or_else(|| {
                localized_text(
                    application_context,
                    PortTextKey::SpellforgeMpDirectInviteNoMission,
                )
                .to_owned()
            })?;
            (mission_name, None)
        }
    };
    channels.shutdown();
    let mission_id = if distributed_mod.is_some() {
        u32::MAX
    } else {
        missions
            .iter()
            .find(|mission| {
                mission
                    .authoritative_basename
                    .eq_ignore_ascii_case(&mission_name)
            })
            .map(|mission| mission.mission_id)
            .ok_or_else(|| {
                localized_format(
                    application_context,
                    PortTextKey::SpellforgeMpVanillaMissionMissing,
                    &[("mission", &mission_name)],
                )
            })?
    };
    Ok(MultiplayerLaunch {
        local_custom: None,
        coop: Default::default(),
        mission_id,
        mission_name,
        role: MultiplayerRole::Client {
            connect_addr: connect_addr.to_owned(),
        },
        expected_players,
        start_at_epoch_ms: None,
        distributed_mod,
        distributed_installed_locator: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
async fn prepare_direct_browser_launch(
    _connect_addr: &str,
    _missions: &[MissionChoice],
    application_context: &ApplicationContext,
    _io: &mut ModalScreenIo<'_, '_>,
) -> Result<MultiplayerLaunch, String> {
    Err(localized_text(
        application_context,
        PortTextKey::SpellforgeMpDirectInvitesBrowserOnly,
    )
    .to_owned())
}

#[cfg(target_arch = "wasm32")]
async fn await_prepared_confirmation(
    application_context: &ApplicationContext,
    channels: &crate::multiplayer::NetChannels,
) -> Result<(), String> {
    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match channels.try_recv_event() {
            Ok(crate::multiplayer::NetEvent::Note(note))
                if note.contains("without joining a gameplay seat") =>
            {
                return Ok(());
            }
            Ok(crate::multiplayer::NetEvent::Fatal(error)) => return Err(error.to_string()),
            Ok(_) => {}
            Err(std::sync::mpsc::TryRecvError::Empty) if web_time::Instant::now() < deadline => {
                crate::window::sleep_ms(10).await;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                return Err(localized_text(
                    application_context,
                    PortTextKey::SpellforgeMpNoSeatPreflightTimeout,
                )
                .to_owned());
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(localized_text(
                    application_context,
                    PortTextKey::SpellforgeMpPreflightTransportClosed,
                )
                .to_owned());
            }
        }
    }
}

async fn preflight_host_content(
    connect_addr: &str,
    advertised: &robin_engine::multiplayer::DistributedModOffer,
    application_context: &ApplicationContext,
    io: &mut ModalScreenIo<'_, '_>,
) -> Result<Arc<[u8]>, String> {
    let (mut channels, incoming_tx, outgoing_rx, _frame_cursor, _snapshot) =
        crate::multiplayer::NetChannels::new();
    let handle = crate::multiplayer::connect_client(
        connect_addr,
        multiplayer_nickname(application_context),
        incoming_tx,
        outgoing_rx,
    )
    .map_err(|error| {
        localized_format(
            application_context,
            PortTextKey::SpellforgeMpConnectForReview,
            &[("error", &error.to_string())],
        )
    })?;
    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(15);
    while handle.content_offer().is_none()
        && handle.session_metadata().is_none()
        && web_time::Instant::now() < deadline
    {
        crate::window::sleep_ms(10).await;
    }
    let offer = handle.content_offer().ok_or_else(|| {
        if handle.session_metadata().is_some() {
            localized_text(
                application_context,
                PortTextKey::SpellforgeMpOrdinaryWelcome,
            )
            .to_owned()
        } else {
            localized_text(application_context, PortTextKey::SpellforgeMpOfferTimeout).to_owned()
        }
    })?;
    if &offer != advertised {
        let actual = robin_engine::spellforge::hex_hash(&offer.full_mod_sha256);
        let advertised = robin_engine::spellforge::hex_hash(&advertised.full_mod_sha256);
        return Err(localized_format(
            application_context,
            PortTextKey::SpellforgeMpOfferMismatch,
            &[("actual", &actual), ("advertised", &advertised)],
        ));
    }
    channels.attach_runtime(handle);
    let (key, metadata) = crate::distributed_mod::offer_trust_identity(&offer)?;
    let outcome = crate::ingame_menu::spellforge_content::show_spellforge_consent(
        application_context,
        io,
        key,
        metadata,
    )
    .await?;
    if outcome == crate::ingame_menu::spellforge_content::SpellforgeConsentOutcome::Cancelled {
        channels.reject_content(
            offer.full_mod_sha256,
            "player cancelled exact host-content approval".to_owned(),
        );
        channels.shutdown();
        return Err(
            localized_text(application_context, PortTextKey::SpellforgeMpJoinCancelled).to_owned(),
        );
    }

    let admitted = crate::distributed_mod_admission::admit_trusted_distributed_mod(
        application_context,
        &channels,
        &offer,
        crate::distributed_mod_admission::DistributedModAdmissionPurpose::PrepareOnly,
    )
    .await?;
    let encoded = admitted.cache_lease.encoded_arc();
    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match channels.try_recv_event() {
            Ok(crate::multiplayer::NetEvent::Note(note))
                if note.contains("without joining a gameplay seat") =>
            {
                break;
            }
            Ok(crate::multiplayer::NetEvent::Fatal(error)) => return Err(error.to_string()),
            Ok(_) => {}
            Err(std::sync::mpsc::TryRecvError::Empty) if web_time::Instant::now() < deadline => {
                crate::window::sleep_ms(10).await;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                return Err(localized_text(
                    application_context,
                    PortTextKey::SpellforgeMpPreflightTimeout,
                )
                .to_owned());
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(localized_text(
                    application_context,
                    PortTextKey::SpellforgeMpPreflightTransportClosed,
                )
                .to_owned());
            }
        }
    }
    channels.shutdown();
    drop(admitted);
    Ok(encoded)
}

impl MultiplayerMenuState {
    fn selected_mission(&self) -> Option<&MissionChoice> {
        match &self.mode {
            MenuMode::Missions => self.missions.get(self.selected),
            MenuMode::Hosted { game } => self.missions.iter().find(|m| {
                m.mission_id == game.mission_id
                    && (m.mission_id != u32::MAX || m.mission_name == game.mission_name)
            }),
            MenuMode::Joined { game, .. } => self
                .missions
                .iter()
                .find(|m| m.mission_id == game.mission_id),
            _ => None,
        }
    }

    fn render_team(
        &self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        transform: MenuTransform,
    ) {
        draw_panel(renderer, transform, &LIST_RECT);
        let Some(font) = resources.menu_text_font_any() else {
            return;
        };
        let heading = if self.hero_setup {
            format!("Mission team - {}/5 characters", self.coop.team_len())
        } else {
            "Player assignments".into()
        };
        render_text_virt_font(renderer, font, transform, &heading, 28, 58);
        let right = if self.hero_setup {
            format!("Choose for slot {}", self.team_slot + 1)
        } else {
            "Game rules".into()
        };
        render_text_virt_font(
            renderer,
            font,
            transform,
            &right,
            MENU_W - resources.button_dimensions().0 - 10,
            58,
        );
        if self.hero_setup {
            let usual = self
                .selected_mission()
                .and_then(|m| m.usual_team.as_deref());
            for slot in 0..5 {
                let code = self.coop.team[slot];
                let hint = if code == 0 {
                    if slot == self.coop.team_len() {
                        "Select this slot, then choose a character."
                    } else {
                        "Empty slot"
                    }
                } else if usual.is_none() {
                    "Usual team unknown for this mission."
                } else if !usual.unwrap().contains(char::from(code)) {
                    "! Not normally available in this mission"
                } else if self.coop.team[..slot].contains(&code) {
                    "Duplicate character"
                } else {
                    "Normally available in this mission"
                };
                render_text_virt_font(renderer, font, transform, hint, 44, 122 + slot as i32 * 58);
            }
        }
        let mission = self
            .selected_mission()
            .map(|m| m.label.as_str())
            .unwrap_or("Mission");
        let text = if let Err(error) = self.coop.validate() {
            error
        } else if self.hero_setup {
            format!("{mission}\n{}", self.status)
        } else {
            "Click a player to change their slot. Occupied slots swap players.".into()
        };
        let wrapped = wrap_text_font(font, &text, LIST_RECT.w, 2);
        for (line, text) in wrapped.lines.iter().enumerate() {
            render_text_virt_font(
                renderer,
                font,
                transform,
                &text.text,
                28,
                400 + line as i32 * 13,
            );
        }
    }

    fn render_menu(
        &self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        transform: MenuTransform,
        application_context: &ApplicationContext,
    ) {
        let Self {
            mode,
            games,
            missions,
            selected,
            scroll_view,
            status,
            ..
        } = self;
        let selected = *selected;
        if let Some(font) = resources.title_font_any() {
            let title = match mode {
                MenuMode::Games => "Multiplayer",
                MenuMode::Missions if self.local => "Local Co-op",
                MenuMode::Missions => "Host Online",
                MenuMode::Hosted { .. } => "Game Lobby",
                MenuMode::Joined { .. } => "Game Lobby",
            };
            let tw = font.text_width(title);
            render_text_virt_font(renderer, font, transform, title, (MENU_W - tw) / 2, 24);
        }

        if self.hero_setup || self.edit_assignments {
            self.render_team(renderer, resources, transform);
            return;
        }
        draw_panel(renderer, transform, &LIST_RECT);
        let rows_len = match mode {
            MenuMode::Games => games.len().max(1),
            MenuMode::Missions => missions.len(),
            MenuMode::Hosted { .. } | MenuMode::Joined { .. } => 1,
        };
        let rows = visible_list_rows(
            rows_len,
            scroll_view.offset(),
            scroll_view.visible_count(),
            |index| match mode {
                MenuMode::Games => {
                    if games.is_empty() {
                        "No games listed".to_string()
                    } else {
                        format_game_row(&games[index], application_context)
                    }
                }
                MenuMode::Hosted { game, .. } => format_game_row(game, application_context),
                MenuMode::Joined { game, listing } => listing
                    .as_ref()
                    .map(|listing| format_game_row(listing, application_context))
                    .unwrap_or_else(|| {
                        format!(
                            "{} | joined |  | waiting",
                            application_context
                                .localized_mission_name(game.mission_id, &game.mission_name)
                        )
                    }),
                MenuMode::Missions => {
                    let mission = &missions[index];
                    mission.label.clone()
                }
            },
        );
        let column_layout = menu_column_layout(mode);
        for (visible_i, row_idx, row) in rows {
            let is_selected = row_idx == selected
                && match mode {
                    MenuMode::Games => !games.is_empty(),
                    MenuMode::Hosted { .. } | MenuMode::Joined { .. } => true,
                    MenuMode::Missions => !missions.is_empty(),
                };
            if is_selected {
                fill_virtual_rect(
                    renderer,
                    transform,
                    LIST_RECT.x + 4,
                    LIST_RECT.y + 4 + visible_i as i32 * ROW_HEIGHT,
                    scroll_view.content_width(),
                    ROW_HEIGHT,
                    Renderer::create_color_16(72, 62, 34),
                );
            }
            if let Some(font) = resources.list_font(is_selected, is_selected) {
                let row_area_x = (LIST_RECT.x + 10) as f32;
                let row_area_w = (scroll_view.content_width() - 12) as f32;
                for cell in column_layout.layout_row(&row, row_area_x, row_area_w) {
                    let fitted = truncate_to_pixel_width_by(
                        cell.text.trim(),
                        cell.span_w as i32,
                        TruncationMarker::AsciiEllipsis,
                        // TrueType drawing retains fractional advances whereas
                        // legacy menu metrics truncate each glyph's advance.
                        |text| font.text_width(text) + text.chars().count() as i32,
                    );
                    if fitted.is_empty() {
                        continue;
                    }
                    let text_w = font.text_width(&fitted) as f32;
                    let cell_x = match cell.align {
                        ColumnAlign::Left => cell.span_x,
                        ColumnAlign::Center => cell.span_x + (cell.span_w - text_w) / 2.0,
                        ColumnAlign::Right => cell.span_x + cell.span_w - text_w,
                    };
                    render_text_virt_font(
                        renderer,
                        font,
                        transform,
                        &fitted,
                        cell_x.round() as i32,
                        LIST_RECT.y + 6 + visible_i as i32 * ROW_HEIGHT,
                    );
                }
            }
        }

        scroll_view.draw_scrollbar(renderer, transform, resources);
        if matches!(mode, MenuMode::Joined { .. }) {
            if let Some(font) = resources.menu_text_font_any() {
                render_text_virt_font(renderer, font, transform, "Host's mission team", 40, 118);
                let usual = self
                    .selected_mission()
                    .and_then(|m| m.usual_team.as_deref());
                for (slot, &code) in self.coop.team[..self.coop.team_len()].iter().enumerate() {
                    let name = format!(
                        "{}. {}",
                        slot + 1,
                        robin_engine::coop::team_character_name(code)
                    );
                    let y = 146 + slot as i32 * 46;
                    render_text_virt_font(renderer, font, transform, &name, 40, y);
                    let warning = match usual {
                        Some(team) if !team.contains(char::from(code)) => {
                            "! Not normally available in this mission"
                        }
                        None => "Usual team unknown for this mission.",
                        _ => "",
                    };
                    render_text_virt_font(renderer, font, transform, warning, 40, y + 15);
                }
            }
        }

        if let Some(font) = resources.menu_text_font_any() {
            let heading = match mode {
                MenuMode::Games => "Online games",
                MenuMode::Missions => "Choose a mission",
                _ => "Your lobby",
            };
            render_text_virt_font(renderer, font, transform, heading, LIST_RECT.x, 58);
            let sidebar = if matches!(mode, MenuMode::Games) {
                "Play together"
            } else if matches!(mode, MenuMode::Joined { .. }) {
                "Waiting for host"
            } else if self.hero_setup {
                "Hero setup"
            } else {
                "Game rules"
            };
            render_text_virt_font(
                renderer,
                font,
                transform,
                sidebar,
                MENU_W - resources.button_dimensions().0 - 10,
                58,
            );
            let help = if matches!(mode, MenuMode::Games) {
                "Join an online game, host your own, or play locally on one screen."
            } else {
                match self.coop.control {
                    robin_engine::coop::CharacterControl::Shared => {
                        "Everyone can select and control any hero."
                    }
                    robin_engine::coop::CharacterControl::Exclusive => {
                        "A hero can be controlled by one player at a time."
                    }
                    robin_engine::coop::CharacterControl::Assigned => {
                        "Each player controls their assigned hero."
                    }
                }
            };
            let team = self.coop.team[..self.coop.team_len()]
                .iter()
                .map(|&code| robin_engine::coop::team_character_name(code))
                .collect::<Vec<_>>()
                .join(", ");
            let detail = if let Err(error) = self.coop.validate() {
                error
            } else if matches!(mode, MenuMode::Games) {
                help.to_owned()
            } else {
                format!("Team: {team}. {help}")
            };
            let text = format!("{status}\n{detail}");
            let wrapped = wrap_text_font(font, &text, LIST_RECT.w, 3);
            for (line, text) in wrapped.lines.iter().enumerate() {
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    &text.text,
                    LIST_RECT.x,
                    LIST_RECT.y + LIST_RECT.h + 12 + line as i32 * 13,
                );
            }
        }
    }
}

fn visible_list_rows(
    rows_len: usize,
    scroll_offset: usize,
    visible_count: usize,
    mut format: impl FnMut(usize) -> String,
) -> impl Iterator<Item = (usize, usize, String)> {
    (0..rows_len)
        .skip(scroll_offset)
        .take(visible_count)
        .enumerate()
        .map(move |(visible_index, row_index)| (visible_index, row_index, format(row_index)))
}

/// Normal starting availability follows the campaign's prerequisite and rescue
/// progression. Unknown custom rosters remain explicitly unknown.
fn usual_mission_team(
    profiles: &engine_profiles::ProfileManager,
    filename: &str,
) -> Option<String> {
    let mut team = String::from("R");
    if matches!(
        filename.to_ascii_lowercase().as_str(),
        "h01_lin_vl" | "s01_not_vl" | "h07_not_mk"
    ) {
        return Some(team);
    }
    let mission = profiles
        .missions
        .iter()
        .find(|p| p.mission_filename.eq_ignore_ascii_case(filename))?;
    let mut completed = std::collections::HashSet::new();
    let mut pending = mission.missions_required_to_be_done.clone();
    while let Some(id) = pending.pop() {
        if !completed.insert(id) {
            continue;
        }
        let prerequisite = profiles.missions.iter().find(|p| p.id == id)?;
        pending.extend(prerequisite.missions_required_to_be_done.iter().copied());
    }
    for (rescue, code) in [
        ("S01_Not_VL", 'S'),
        ("S02_Lei_MP", 'W'),
        ("S03_FoB_MP", 'J'),
        ("S04_Der_EC", 'T'),
        ("S05_Yrk_EC", 'M'),
    ] {
        if profiles
            .missions
            .iter()
            .any(|p| p.mission_filename.eq_ignore_ascii_case(rescue) && completed.contains(&p.id))
            && !team.contains(code)
        {
            team.push(code);
        }
    }
    if team.contains('S') {
        team.push_str("ABC");
    }
    // Robin is the captive in this rescue mission.
    if filename.eq_ignore_ascii_case("H09_Not_VL") {
        team.retain(|code| code != 'R');
    }
    Some(team)
}

fn mission_choices(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    application_context: &ApplicationContext,
) -> Vec<MissionChoice> {
    #[allow(unused_mut)]
    let mut choices = campaign
        .missions
        .iter()
        .map(|m| {
            let profile = m.profile(profiles);
            let fallback = if profile.mission_name.trim().is_empty() {
                if profile.mission_filename.trim().is_empty() {
                    format!("Mission {}", profile.id)
                } else {
                    profile.mission_filename.clone()
                }
            } else {
                profile.mission_name.clone()
            };
            let mission_name = application_context.localized_mission_name(profile.id, &fallback);
            // Keep the selector's numbering consistent with the leaderboard
            // mission list; the profile id is an internal resource id and is
            // intentionally not shown here.
            let label = campaign_mission_number(&profile.mission_filename).map_or_else(
                || mission_name.clone(),
                |number| format!("{number:02} {mission_name}"),
            );
            MissionChoice {
                usual_team: crate::main_entry::detect_demo_mode_with_context(application_context)
                    .filter(|(mission, ..)| mission.eq_ignore_ascii_case(&profile.mission_filename))
                    .map(|(_, _, pcs, _)| pcs.to_owned())
                    .or_else(|| usual_mission_team(profiles, &profile.mission_filename)),
                mission_id: profile.id,
                #[cfg(target_arch = "wasm32")]
                authoritative_basename: profile.mission_filename.clone(),
                mission_name,
                label,
                custom: None,
            }
        })
        .collect::<Vec<_>>();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let default_root = crate::mod_pack::default_mods_root();
        let mods = crate::mod_pack::scan_mission_roots(
            &default_root,
            crate::main_entry::overlay_mods_dir().as_deref(),
        );
        for entry in crate::mod_pack::enumerate_missions(
            &mods,
            application_context
                .preparation_files()
                .expect("multiplayer mission discovery requires initialized application files"),
        ) {
            let crate::mod_pack::MissionStatus::Ok { map_filename } = entry.status else {
                continue;
            };
            if entry.hackable {
                continue;
            }
            let installed_source =
                match crate::mission_asset_launch::locate_installed_mission_source(
                    &entry.version_zip,
                    &default_root,
                    crate::main_entry::overlay_mods_dir().as_deref(),
                ) {
                    Ok(source) => source,
                    Err(error) => {
                        tracing::warn!(
                            archive = %entry.version_zip.display(),
                            "skipping custom multiplayer mission without durable source: {error}"
                        );
                        continue;
                    }
                };
            let label = localized_format(
                application_context,
                PortTextKey::SpellforgeMpModMissionLabel,
                &[
                    ("title", &entry.mod_title),
                    ("version", &entry.version_label),
                ],
            );
            let launch = CustomMissionLaunch {
                slug: entry.mod_slug,
                mod_title: entry.mod_title,
                claimed_author: entry.author,
                version: entry.version_label,
                source_url: entry.source_url,
                license: entry.license,
                version_zip: entry.version_zip,
                installed_source: Some(installed_source),
                version_zip_bytes: None,
                rhm_zip_entry: entry.rhm_zip_entry,
                rhm_basename: entry.rhm_basename.clone(),
                map_filename,
                requires_spellforge: entry.requires_spellforge,
            };
            choices.push(MissionChoice {
                usual_team: None,
                mission_id: u32::MAX,
                #[cfg(target_arch = "wasm32")]
                authoritative_basename: entry.rhm_basename.clone(),
                mission_name: entry.rhm_basename,
                label,
                custom: Some(launch),
            });
        }
    }
    choices
}

/// Campaign numbers used by the leaderboard/walkthrough mission list.
fn campaign_mission_number(filename: &str) -> Option<usize> {
    Some(match filename {
        "H01_Lin_VL" => 1,
        "S01_Not_VL" => 2,
        "S02_Lei_MP" => 3,
        "H02_Not_EC" => 4,
        "H03_Der_MK" => 5,
        "S03_FoB_MP" => 6,
        "H04_Lei_VL" => 7,
        "H05_Lin_EC" => 8,
        "S04_Der_EC" => 9,
        "H07_Not_MK" => 10,
        "Str02_Der_MP" => 11,
        "S05_Yrk_EC" => 12,
        "H09_Not_VL" => 13,
        "H10_Yor_VL" => 14,
        "Str03_Yor_MK" => 15,
        "H12_Not_MP" => 16,
        _ => return None,
    })
}

fn multiplayer_nickname(application_context: &ApplicationContext) -> String {
    let name = require(
        application_context.with_active_profile(|profile| profile.name.clone()),
        SCREEN,
    );
    if !name.trim().is_empty() {
        return name;
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "player".to_string())
}

fn upsert_game(games: &mut Vec<GameListing>, game: GameListing) -> &GameListing {
    let index = if let Some(index) = games.iter().position(|existing| existing.id == game.id) {
        games[index] = game;
        index
    } else {
        let index = games.len();
        games.push(game);
        index
    };
    &games[index]
}

fn format_game_row(game: &GameListing, application_context: &ApplicationContext) -> String {
    let players = if game.max_players == 0 {
        game.players.to_string()
    } else {
        format!("{}/{}", game.players, game.max_players)
    };
    let state = if game.state.is_empty() {
        "waiting"
    } else {
        &game.state
    };
    let mission_name =
        application_context.localized_mission_name(game.mission_id, &game.mission_name);
    format!("{}|{}|{}|{}", mission_name, game.host, players, state)
}

fn menu_column_layout(mode: &MenuMode) -> ColumnLayout {
    match mode {
        MenuMode::Missions => ColumnLayout::new(&[(1.0, ColumnAlign::Left)]),
        _ => ColumnLayout::new(&[
            (0.46, ColumnAlign::Left),
            (0.24, ColumnAlign::Left),
            (0.12, ColumnAlign::Center),
            (0.18, ColumnAlign::Left),
        ]),
    }
}

fn draw_panel(renderer: &mut Renderer, transform: MenuTransform, rect: &MenuRect) {
    crate::ingame_menu::layout::draw_colored_panel(
        renderer,
        transform,
        rect,
        Renderer::create_color_16(28, 24, 16),
        Renderer::create_color_16(172, 146, 84),
    );
}

fn fill_virtual_rect(
    renderer: &mut Renderer,
    transform: MenuTransform,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u16,
) {
    let (sx, sy) = transform.to_screen(x, y);
    renderer.fill_screen(
        Some(&BBox::from_coords(
            sx as f32,
            sy as f32,
            (sx + w) as f32,
            (sy + h) as f32,
        )),
        color,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn usual_roster_includes_recruited_alternatives_and_flags_early_heroes() {
        use super::*;
        let mut profiles = engine_profiles::ProfileManager::default();
        profiles.missions = vec![
            engine_profiles::MissionProfile {
                id: 1,
                mission_filename: "S01_Not_VL".into(),
                ..Default::default()
            },
            engine_profiles::MissionProfile {
                id: 2,
                mission_filename: "S02_Lei_MP".into(),
                missions_required_to_be_done: vec![1],
                ..Default::default()
            },
            engine_profiles::MissionProfile {
                id: 3,
                mission_filename: "H10_Yor_VL".into(),
                missions_required_to_be_done: vec![2],
                ..Default::default()
            },
        ];
        assert_eq!(
            usual_mission_team(&profiles, "S01_Not_VL").as_deref(),
            Some("R")
        );
        let late = usual_mission_team(&profiles, "H10_Yor_VL").unwrap();
        for code in "RSWABC".chars() {
            assert!(late.contains(code));
        }
        assert!(!late.contains('F'));
        assert!(usual_mission_team(&profiles, "Unknown custom mission").is_none());
    }

    #[test]
    fn list_rows_only_format_the_visible_window() {
        use super::*;
        for (rows_len, offset) in [
            (0, 0),
            (1, 0),
            (100, 20),
            (100, 99),
            (100, 100),
            (100, usize::MAX),
        ] {
            let mut formatted = Vec::new();
            let rows: Vec<_> = visible_list_rows(rows_len, offset, 5, |index| {
                formatted.push(index);
                index.to_string()
            })
            .collect();
            let expected: Vec<_> = (0..rows_len).skip(offset).take(5).collect();
            assert_eq!(formatted, expected);
            assert_eq!(rows.len(), expected.len());
            for (position, (visible_index, row_index, text)) in rows.into_iter().enumerate() {
                assert_eq!(visible_index, position);
                assert_eq!(row_index, expected[position]);
                assert_eq!(text, row_index.to_string());
            }
        }
    }

    #[test]
    fn game_upserts_move_records_and_preserve_listing_order() {
        use super::*;
        let listing = |id: &str, players| GameListing {
            coop: Default::default(),
            id: id.into(),
            mission_id: 1,
            mission_name: "Mission".into(),
            host_content: None,
            host: "Host".into(),
            players,
            max_players: 2,
            state: "waiting".into(),
            start_at_epoch_ms: None,
        };
        let mut games = Vec::new();
        for id in ["first", "second", "third"] {
            let incoming = listing(id, 1);
            let name_storage = incoming.mission_name.as_ptr();
            let stored = upsert_game(&mut games, incoming);
            assert_eq!(stored.id, id);
            assert_eq!(stored.mission_name.as_ptr(), name_storage);
        }
        let incoming = listing("second", 2);
        let name_storage = incoming.mission_name.as_ptr();
        let stored = upsert_game(&mut games, incoming);
        assert_eq!(stored.players, 2);
        assert_eq!(stored.mission_name.as_ptr(), name_storage);
        assert_eq!(
            games
                .iter()
                .map(|game| game.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
        assert_eq!(
            games.iter().map(|game| game.players).collect::<Vec<_>>(),
            [1, 2, 1]
        );
    }

    #[test]
    fn disconnected_discovery_removes_stale_host_and_join_controls() {
        use super::*;
        let listing = |state: &str| GameListing {
            coop: Default::default(),
            id: "host".into(),
            mission_id: 1,
            mission_name: "Mission".into(),
            host_content: None,
            host: "Host".into(),
            players: 1,
            max_players: 2,
            state: state.into(),
            start_at_epoch_ms: None,
        };
        let mut games = vec![listing("waiting"), listing("direct_invite")];
        let mut mode = MenuMode::Hosted {
            game: games[0].clone(),
        };
        discard_disconnected_matchmaking_state(&mut games, &mut mode);
        assert!(matches!(mode, MenuMode::Games));
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].state, "direct_invite");
        mode = MenuMode::Joined {
            game: selected(),
            listing: None,
        };
        discard_disconnected_matchmaking_state(&mut games, &mut mode);
        assert!(matches!(mode, MenuMode::Games));
    }

    use super::{signed_direct_listing, validate_started_game};
    use crate::multiplayer::matchmaking::JoinedGame;

    fn offer() -> robin_engine::multiplayer::DistributedModOffer {
        robin_engine::multiplayer::DistributedModOffer {
            schema_version: 1,
            full_mod_sha256: [1; 32],
            spellforge_package_sha256: None,
            spellforge_vm_abi: None,
            encoded_bytes: 123,
            mission_basename: "Mission".into(),
            mission_rhm_entry: "Data/Levels/Mission.rhm".into(),
            map_filename: "Map".into(),
            title: "Mod".into(),
            claimed_author: "Author".into(),
            version: "1".into(),
            source_url: "https://example.invalid/mod".into(),
            license: "CC0-1.0".into(),
            host_endpoint_id: "host-key".into(),
        }
    }

    fn selected() -> JoinedGame {
        JoinedGame {
            coop: Default::default(),
            game_id: "host-key".into(),
            mission_id: u32::MAX,
            mission_name: "Mission".into(),
            host_content: Some(offer()),
            connect_addr: "host-key".into(),
            expected_players: 2,
            start_at_epoch_ms: None,
        }
    }

    #[test]
    fn start_is_pinned_to_selected_host_mission_and_exact_content() {
        let selected = selected();
        let mut started = selected.clone();
        started.expected_players = 3;
        started.start_at_epoch_ms = Some(1234);
        assert!(validate_started_game(&selected, &started).is_ok());

        let mut changed_host = started.clone();
        changed_host.connect_addr = "attacker-key".into();
        assert!(validate_started_game(&selected, &changed_host).is_err());

        let mut changed_mission = started.clone();
        changed_mission.mission_name = "Other".into();
        assert!(validate_started_game(&selected, &changed_mission).is_err());

        let mut downgraded = started;
        downgraded.host_content = None;
        assert!(validate_started_game(&selected, &downgraded).is_err());
    }

    #[test]
    fn scrubbed_shell_ticket_can_be_handed_directly_to_the_menu() {
        let key = iroh::SecretKey::from_bytes(&[7; 32]);
        let address = iroh::EndpointAddr::from(key.public())
            .with_relay_url("https://relay.example.invalid/".parse().unwrap());
        let ticket = crate::multiplayer::join_ticket::BrowserJoinTicket::issue(
            &key,
            &address,
            [9; 32],
            2_000_000_000,
            crate::multiplayer::join_ticket::BrowserJoinTicketContent {
                content_edition: crate::multiplayer::join_ticket::BrowserContentEdition::Demo,
                content_identity_sha256: "01".repeat(32),
                mission_id: "Dem_Lei_MP".to_owned(),
                mission_profile_id: Some(4),
                expected_players: 2,
            },
        )
        .unwrap();
        let code = ticket.encode();

        let listing = signed_direct_listing(&code).expect("signed direct listing");
        assert_eq!(listing.id, code);
        assert_eq!(listing.mission_id, 4);
        assert_eq!(listing.mission_name, "Dem_Lei_MP");
        assert_eq!(listing.host, key.public().to_string());
        assert_eq!(listing.max_players, 2);
        assert_eq!(listing.state, "direct_invite");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod visual_tests {
    use super::*;

    #[test]
    #[ignore = "requires game data and an offscreen GPU adapter"]
    fn capture_multiplayer_team_ui() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let data = robin_test_support::original_data::data_directory("");
        let output = root.join("target/multiplayer-ui");
        std::fs::create_dir_all(&output).unwrap();
        let (campaign, profiles, context) =
            crate::main_entry::rust_init_with_roots(Some(&data), Some(root)).unwrap();
        let gpu = offscreen_gpu();
        for (width, height) in [(640, 480), (1024, 768)] {
            let mut renderer = Renderer::offscreen(gpu.clone(), width, height);
            let resources = IngameMenuResources::new(
                &mut renderer,
                context.shipping().unwrap(),
                context.preparation_files().unwrap().clone(),
            )
            .unwrap();
            let mut state = MultiplayerMenuState {
                coop: robin_engine::coop::CoopRules {
                    team: [b'R', b'T', b'T', b'W', b'M'],
                    ..Default::default()
                },
                local: true,
                edit_assignments: false,
                hero_setup: true,
                team_slot: 4,
                team_focus: None,
                frame: FrameWnd::interactive(),
                input_state: ModalInputState::new(),
                scroll_view: ScrollView::new(
                    [
                        LIST_RECT.x + 4,
                        LIST_RECT.y + 4,
                        LIST_RECT.w - 8,
                        LIST_RECT.h - 8,
                    ],
                    ROW_HEIGHT,
                    &resources,
                ),
                selected: 0,
                mode: MenuMode::Missions,
                status: "1 player joined. Keyboard on. Press A on a controller to join.".into(),
                games: Vec::new(),
                matchmaking_label: String::new(),
                matchmaking_client: None,
                prepared_host_content: None,
                missions: mission_choices(&campaign, &profiles, &context),
            };
            state.selected = state
                .missions
                .iter()
                .position(|m| m.label.starts_with("01 "))
                .unwrap_or(0);
            for view in [
                "full-team",
                "online-team",
                "add-slot",
                "mission-rules",
                "assignments",
                "invalid-team",
                "keyboard-focus",
                "joined",
                "browser",
            ] {
                state.hero_setup = matches!(
                    view,
                    "full-team" | "online-team" | "add-slot" | "invalid-team" | "keyboard-focus"
                );
                state.local = view != "online-team";
                state.status = if view == "online-team" {
                    "Select your mission team, then create an online lobby.".into()
                } else {
                    "1 player joined. Keyboard on. Press A on a controller to join.".into()
                };
                state.edit_assignments = view == "assignments";
                state.coop.control = if matches!(view, "assignments" | "invalid-team") {
                    robin_engine::coop::CharacterControl::Assigned
                } else {
                    robin_engine::coop::CharacterControl::Shared
                };
                state.coop.players = if matches!(view, "assignments" | "invalid-team") {
                    5
                } else {
                    1
                };
                state.coop.team = if matches!(view, "add-slot" | "invalid-team") {
                    [b'R', 0, 0, 0, 0]
                } else {
                    [b'R', b'T', b'T', b'W', b'M']
                };
                state.team_slot = if view == "add-slot" { 1 } else { 4 };
                state.mode = if view == "browser" {
                    MenuMode::Games
                } else if view == "joined" {
                    let mission = &state.missions[state.selected];
                    MenuMode::Joined {
                        game: JoinedGame {
                            coop: state.coop,
                            game_id: "capture".into(),
                            mission_id: mission.mission_id,
                            mission_name: mission.mission_name.clone(),
                            host_content: None,
                            connect_addr: "capture".into(),
                            expected_players: 2,
                            start_at_epoch_ms: None,
                        },
                        listing: None,
                    }
                } else {
                    MenuMode::Missions
                };
                state.team_focus = None;
                state.clamp_selection_to_rows();
                if !matches!(state.mode, MenuMode::Games) {
                    state.scroll_view.reveal(state.selected);
                }
                state.update_buttons(true, &resources);
                if view == "keyboard-focus" {
                    state.move_team_focus(Keycode::Tab);
                    assert_eq!(state.team_focus, Some(ID_COPY_BASE));
                    state.move_team_focus(Keycode::Down);
                    assert_eq!(state.team_focus, Some(ID_COPY_BASE + 1));
                    state.move_team_focus(Keycode::Right);
                    assert_eq!(state.team_focus, Some(ID_REMOVE_BASE + 1));
                }
                let widgets = state.frame.widgets();
                for (index, a) in widgets.iter().enumerate() {
                    let ab = a.base().bbox.0.unwrap();
                    for b in &widgets[index + 1..] {
                        let bb = b.base().bbox.0.unwrap();
                        assert!(
                            !(ab.min().x < bb.max().x
                                && ab.max().x > bb.min().x
                                && ab.min().y < bb.max().y
                                && ab.max().y > bb.min().y),
                            "overlapping buttons: {} / {}",
                            a.id(),
                            b.id()
                        );
                    }
                }
                renderer.begin_gpu_frame_clear();
                renderer.begin_ui_only_frame();
                let transform = MenuTransform::centered(width as i32, height as i32);
                if let Some(bg) = resources.menu_bg[2] {
                    draw_screen_background(&mut renderer, &bg);
                }
                state.render_menu(&mut renderer, &resources, transform, &context);
                state.draw_buttons(&mut renderer, &resources, transform);
                write_capture_png(
                    &mut renderer,
                    &output.join(format!("{view}-{width}x{height}.png")),
                );
            }
        }
    }
    #[test]
    #[ignore = "requires full game data and RUST_MIN_STACK=33554432"]
    fn selected_team_spawns_exactly_in_a_one_character_mission() {
        use robin_engine::engine::{Engine, EngineArgs, LevelAssets, LevelLoadArgs, SimConfig};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let data = robin_test_support::original_data::data_directory("");
        let (campaign, profiles, context) =
            crate::main_entry::rust_init_with_roots(Some(&data), Some(root)).unwrap();
        let files = context.preparation_files().unwrap().clone();
        let mission = campaign
            .missions
            .iter()
            .position(|m| {
                m.profile(&profiles)
                    .mission_filename
                    .eq_ignore_ascii_case("H01_Lin_VL")
            })
            .expect("full game's first mission");
        let mut host = crate::Host::scratch(640.0, 480.0);
        host.frontend
            .resources
            .frame_holder_before_publication_mut()
            .initialize_sprite_bank_with_files(".", &files)
            .unwrap();
        for team in [
            [b'R', 0, 0, 0, 0],
            [b'R', b'R', 0, 0, 0],
            [b'R'; 5],
            [b'M', b'T', b'R', b'J', b'W'],
        ] {
            let mut campaign = campaign.clone();
            campaign.current_mission_idx = Some(mission);
            campaign.force_next_mission(mission);
            let mut assets = LevelAssets::new();
            assets.profile_manager = profiles.clone();
            assets.sprite_scriptor = Arc::new(
                robin_engine::sprite_script::SpriteScriptor::with_resources(Arc::new(
                    robin_engine::sprite_script::MissionResourceEnvironment::from_files(&files),
                )),
            );
            assets.bank_signature = host.frontend.resources.frame_holder().signature();
            let name = campaign.missions[mission]
                .profile(&profiles)
                .mission_filename
                .clone();
            let path = files
                .resolve_data_path(&format!("Data/Levels/{name}.scb"))
                .unwrap();
            let program = robin_engine::script_manager::ScriptProgram::from_scb(
                robin_assets::scb::parse_file(&path).unwrap(),
            )
            .unwrap();
            assets.scripts.mission_programs = Arc::new(std::collections::BTreeMap::from([(
                name,
                Arc::new(program),
            )]));
            let mut text =
                robin_assets::resource_manager::ResourceManager::with_files(files.clone());
            text.attach_resource_file("Data/Text/Level.res").unwrap();
            (assets.peasant_firstnames, assets.peasant_surnames) =
                crate::game_session::load_peasant_name_pool(&mut text).unwrap();
            assets.fixed_vip_names =
                crate::game_session::load_fixed_vip_name_map(&mut text).unwrap();
            let loaded = robin_engine::engine::level_loading::load_mission_for_campaign_with_files(
                &campaign,
                &profiles,
                "Data/Levels",
                &mut |_| {},
                &files,
            )
            .unwrap();
            let config = SimConfig {
                script_enabled: true,
                coop: robin_engine::coop::CoopRules {
                    players: 1,
                    team,
                    ..Default::default()
                },
                ..Default::default()
            };
            let engine = Engine::new(EngineArgs {
                campaign,
                level: LevelLoadArgs {
                    assets: &mut assets,
                    level_directory: "Data/Levels",
                    progress: &mut |_| {},
                    loaded,
                    bg_pixel_dims: (4096.0, 4096.0),
                },
                ground_mark_sprite: None,
                titbit_row_frame_counts: Vec::new(),
                rng_seed: 0,
                original_rng_replay: None,
                sim_config: config.clone(),
            })
            .unwrap();
            let party: Vec<_> = engine
                .pc_ids()
                .iter()
                .filter_map(|&id| {
                    engine
                        .get_entity(id)
                        .and_then(robin_engine::element::Entity::pc_data)
                })
                .filter(|pc| {
                    pc.playable
                        && pc.mission_role == robin_engine::human_control::MissionRole::PlayerParty
                })
                .collect();
            assert_eq!(party.len(), config.coop.team_len());
            for (slot, pc) in party.iter().enumerate() {
                assert_eq!(pc.campaign_description_index, Some(slot as u32));
                let profile = profiles.get_character(pc.profile_index).unwrap();
                let expected = robin_engine::coop::TEAM_CHARACTERS
                    .iter()
                    .find(|entry| entry.0 == team[slot])
                    .unwrap()
                    .2;
                assert!(
                    profile.profile_name == expected
                        || (team[slot] == b'R' && pc.kind.is_some_and(|kind| kind.is_robin()))
                );
            }
            let restored =
                Engine::decode_native_snapshot(&engine.encode_native_snapshot()).unwrap();
            assert_eq!(
                robin_engine::replay::state_hash(&engine),
                robin_engine::replay::state_hash(&restored)
            );
        }
    }

    fn offscreen_gpu() -> crate::window::GpuContext {
        pollster::block_on(async {
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
                    label: Some("multiplayer UI capture"),
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
        })
    }

    fn write_capture_png(renderer: &mut Renderer, path: &std::path::Path) {
        let (w, h, pixels) = renderer.try_capture_frame_rgba().expect("read UI pixels");
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
        eprintln!("Captured {}", path.display());
    }
}

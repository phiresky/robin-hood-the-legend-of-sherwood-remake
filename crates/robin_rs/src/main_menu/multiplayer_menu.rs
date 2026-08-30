//! Main-menu multiplayer screen: the matchmaking game browser plus
//! the pre-game lobby (hosted / joined waiting room).

use robin_engine::campaign::Campaign;

use crate::gfx_types::{GameEvent, Keycode};
use crate::host::ApplicationContext;
use crate::ingame_menu::layout::{
    MENU_H, MENU_W, MenuRect, MenuTransform, dim_screen, draw_screen_background,
    enter_modal_gpu_phase, render_text_virt, render_text_virt_font,
};
use crate::ingame_menu::resources::IngameMenuResources;
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::multiplayer::matchmaking::{self, GameListing, JoinedGame};
use crate::native_font::Font;
use crate::renderer::Renderer;
use crate::widget::{ColumnAlign, ColumnLayout, FrameWnd};
use robin_engine::engine::input::MOUSE_OPACITY_DEFAULT;
use robin_engine::profiles as engine_profiles;
use robin_engine::sprite::BBox;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum MultiplayerRole {
    /// Host on this install's persistent iroh identity (the id the
    /// matchmaking service advertised as the game's `connect_addr`).
    Host,
    /// Join the host at the given iroh endpoint id.
    Client { connect_addr: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MultiplayerLaunch {
    pub mission_id: u32,
    pub mission_name: String,
    pub role: MultiplayerRole,
    pub expected_players: u32,
    pub start_at_epoch_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MissionChoice {
    mission_id: u32,
    mission_name: String,
    label: String,
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

pub(crate) async fn show_multiplayer_menu(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor_renderer: &mut crate::cursor::CursorRenderer,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    application_context: &ApplicationContext,
) -> Option<MultiplayerLaunch> {
    let nickname = multiplayer_nickname(application_context);
    let missions = mission_choices(campaign, profiles);
    let (matchmaking_client, mut matchmaking_label) =
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
    let mut games = Vec::new();
    let mut status = matchmaking_label.clone();
    let mut mode = MenuMode::Games;
    let mut selected: usize = 0;
    let mut scroll_offset: usize = 0;
    let mut input_state = ModalInputState::new();
    let (btn_w, btn_h) = resources.button_dimensions();
    let btn_x = MENU_W - btn_w - 10;
    let btn_y_base = MENU_H - btn_h - 10;
    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    for (id, y) in [
        (ID_JOIN, btn_y_base - 3 * (btn_h + 2)),
        (ID_CREATE, btn_y_base - 2 * (btn_h + 2)),
        (ID_START, btn_y_base - (btn_h + 2)),
        (ID_BACK, btn_y_base),
    ] {
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            id, "", true, btn_x, y, btn_w, btn_h,
        ));
    }

    loop {
        let rows_len = match &mode {
            MenuMode::Games => games.len(),
            MenuMode::Missions => missions.len(),
            MenuMode::Hosted { .. } => 1,
            MenuMode::Joined { .. } => 1,
        };
        if rows_len == 0 {
            selected = 0;
            scroll_offset = 0;
        } else if selected >= rows_len {
            selected = rows_len - 1;
        }
        scroll_offset = clamp_scroll_offset(scroll_offset, rows_len);
        ensure_selected_visible(selected, &mut scroll_offset, rows_len);

        while let Some(event) = matchmaking_client
            .as_ref()
            .and_then(|client| client.try_recv())
        {
            match event {
                matchmaking::MatchmakingEvent::Games(next) => {
                    games = next;
                    status = matchmaking_label.clone();
                }
                matchmaking::MatchmakingEvent::Created(created) => {
                    status = "Game created. Press Start when ready.".to_string();
                    mode = MenuMode::Hosted { game: created };
                    selected = 0;
                }
                matchmaking::MatchmakingEvent::Joined(joined) => {
                    if joined.connect_addr.is_empty() {
                        status = "Matchmaking did not return a host address".to_string();
                    } else if joined.start_at_epoch_ms.is_some() {
                        return Some(MultiplayerLaunch {
                            mission_id: joined.mission_id,
                            mission_name: joined.mission_name,
                            role: MultiplayerRole::Client {
                                connect_addr: joined.connect_addr,
                            },
                            expected_players: joined.expected_players,
                            start_at_epoch_ms: joined.start_at_epoch_ms,
                        });
                    } else {
                        let listing = games.iter().find(|g| g.id == joined.game_id).cloned();
                        status = "Joined game. Waiting for host to start...".to_string();
                        mode = MenuMode::Joined {
                            game: joined,
                            listing,
                        };
                        selected = 0;
                    }
                }
                matchmaking::MatchmakingEvent::Started(started) => {
                    if let MenuMode::Hosted { game, .. } = &mode
                        && game.id == started.game_id
                    {
                        return Some(MultiplayerLaunch {
                            mission_id: started.mission_id,
                            mission_name: started.mission_name,
                            role: MultiplayerRole::Host,
                            expected_players: started.expected_players,
                            start_at_epoch_ms: started.start_at_epoch_ms,
                        });
                    }
                }
                matchmaking::MatchmakingEvent::GameUpdated(updated) => {
                    upsert_game(&mut games, updated.clone());
                    match &mut mode {
                        MenuMode::Hosted { game, .. } if game.id == updated.id => {
                            let previous_players = game.players;
                            *game = updated;
                            if game.players != previous_players {
                                status = format!(
                                    "{} player{} in game",
                                    game.players,
                                    if game.players == 1 { "" } else { "s" }
                                );
                            }
                        }
                        MenuMode::Joined { game, listing } if game.game_id == updated.id => {
                            status = format!(
                                "{} player{} in game. Waiting for host to start...",
                                updated.players,
                                if updated.players == 1 { "" } else { "s" }
                            );
                            *listing = Some(updated);
                        }
                        _ => {}
                    }
                }
                matchmaking::MatchmakingEvent::GameStarted(started) => {
                    if let MenuMode::Joined { game, .. } = &mode
                        && game.game_id == started.game_id
                    {
                        return Some(MultiplayerLaunch {
                            mission_id: game.mission_id,
                            mission_name: game.mission_name.clone(),
                            role: MultiplayerRole::Client {
                                connect_addr: game.connect_addr.clone(),
                            },
                            expected_players: started.expected_players,
                            start_at_epoch_ms: started.start_at_epoch_ms,
                        });
                    }
                }
                matchmaking::MatchmakingEvent::Neighbors(count) => {
                    matchmaking_label = if count == 0 {
                        "Matchmaking: searching for players...".to_string()
                    } else {
                        format!(
                            "Matchmaking: {count} player{} online",
                            if count == 1 { "" } else { "s" }
                        )
                    };
                    if matches!(mode, MenuMode::Games) {
                        status = matchmaking_label.clone();
                    }
                }
                matchmaking::MatchmakingEvent::Error(err) => status = err,
                matchmaking::MatchmakingEvent::Disconnected(err) => status = err,
            }
        }

        let matchmaking_connected = matchmaking_client.is_some();
        let can_join = matchmaking_connected
            && matches!(mode, MenuMode::Games)
            && games.get(selected).is_some();
        let can_start = matches!(mode, MenuMode::Hosted { .. });
        set_button(&mut frame, ID_JOIN, "Join", can_join);
        set_button(
            &mut frame,
            ID_CREATE,
            match mode {
                MenuMode::Missions => "Create",
                _ => "Create Game",
            },
            matchmaking_connected && matches!(mode, MenuMode::Games | MenuMode::Missions),
        );
        set_button(&mut frame, ID_START, "Start", can_start);
        set_button(&mut frame, ID_BACK, "Back", true);

        let mut activated: Option<u32> = None;
        let (events, transform) =
            crate::ingame_menu::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => activated = Some(ID_BACK),
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => {
                    selected = selected.saturating_sub(1);
                    ensure_selected_visible(selected, &mut scroll_offset, rows_len);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => {
                    if rows_len > 0 {
                        selected = (selected + 1).min(rows_len - 1);
                        ensure_selected_visible(selected, &mut scroll_offset, rows_len);
                    }
                }
                GameEvent::KeyDown {
                    keycode: Keycode::PageUp,
                    ..
                } => {
                    let step = visible_row_count().saturating_sub(1).max(1);
                    selected = selected.saturating_sub(step);
                    ensure_selected_visible(selected, &mut scroll_offset, rows_len);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::PageDown,
                    ..
                } => {
                    if rows_len > 0 {
                        let step = visible_row_count().saturating_sub(1).max(1);
                        selected = (selected + step).min(rows_len - 1);
                        ensure_selected_visible(selected, &mut scroll_offset, rows_len);
                    }
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Home,
                    ..
                } => {
                    selected = 0;
                    ensure_selected_visible(selected, &mut scroll_offset, rows_len);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::End,
                    ..
                } => {
                    if rows_len > 0 {
                        selected = rows_len - 1;
                        ensure_selected_visible(selected, &mut scroll_offset, rows_len);
                    }
                }
                GameEvent::MouseWheel(delta) => {
                    if rows_len > visible_row_count() {
                        let amount = delta.unsigned_abs() as usize;
                        if delta > 0 {
                            scroll_offset = scroll_offset.saturating_sub(amount);
                        } else if delta < 0 {
                            scroll_offset += amount;
                        }
                        scroll_offset = clamp_scroll_offset(scroll_offset, rows_len);
                        selected = selected.clamp(
                            scroll_offset,
                            (scroll_offset + visible_row_count() - 1).min(rows_len - 1),
                        );
                    }
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    activated = match mode {
                        MenuMode::Games => Some(ID_JOIN),
                        MenuMode::Missions => Some(ID_CREATE),
                        MenuMode::Hosted { .. } => Some(ID_START),
                        MenuMode::Joined { .. } => None,
                    };
                }
                GameEvent::MouseUp(x, y, 1) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    if LIST_RECT.contains_virt(vx, vy) {
                        let row =
                            scroll_offset + ((vy - LIST_RECT.y - 4) / ROW_HEIGHT).max(0) as usize;
                        if row < rows_len {
                            selected = row;
                        }
                    }
                }
                GameEvent::MouseDown(x, y, 1, clicks) if clicks >= 2 => {
                    let (vx, vy) = transform.from_screen(x, y);
                    if LIST_RECT.contains_virt(vx, vy) {
                        let row =
                            scroll_offset + ((vy - LIST_RECT.y - 4) / ROW_HEIGHT).max(0) as usize;
                        if row < rows_len {
                            selected = row;
                            activated = match mode {
                                MenuMode::Games => Some(ID_JOIN),
                                MenuMode::Missions => Some(ID_CREATE),
                                MenuMode::Hosted { .. } => Some(ID_START),
                                MenuMode::Joined { .. } => None,
                            };
                        }
                    }
                }
                _ => {}
            }
        }

        let widget_input = input_state.as_widget_input();
        let widget_events = frame.process_input(&widget_input);
        input_state.end_frame();
        if let Some(id) = widget_bridge::find_activated(&widget_events) {
            activated = Some(id);
        }

        if let Some(id) = activated {
            match id {
                ID_BACK => match mode {
                    MenuMode::Games => return None,
                    _ => {
                        if matches!(mode, MenuMode::Hosted { .. } | MenuMode::Joined { .. })
                            && let Some(session) = matchmaking_client.as_ref()
                            && let Err(err) = session.leave_game()
                        {
                            tracing::warn!("matchmaking leave failed: {err}");
                        }
                        mode = MenuMode::Games;
                        selected = 0;
                        scroll_offset = 0;
                        status = matchmaking_label.clone();
                    }
                },
                ID_JOIN if matches!(mode, MenuMode::Games) => {
                    if let Some(game) = games.get(selected) {
                        match matchmaking_client
                            .as_ref()
                            .map(|session| session.join_game(game.id.clone()))
                        {
                            Some(Ok(())) => {
                                status = "Joining game...".to_string();
                            }
                            Some(Err(err)) => status = err,
                            None => status = "Matchmaking is not connected".to_string(),
                        }
                    }
                }
                ID_CREATE if matches!(mode, MenuMode::Games) => {
                    if missions.is_empty() {
                        status = "No missions are available to host".to_string();
                    } else {
                        mode = MenuMode::Missions;
                        selected = 0;
                        scroll_offset = 0;
                        status = "Select a mission for the hosted game".to_string();
                    }
                }
                ID_CREATE if matches!(mode, MenuMode::Missions) => {
                    if let Some(mission) = missions.get(selected) {
                        match matchmaking_client.as_ref().map(|session| {
                            session.create_game(mission.mission_id, mission.mission_name.clone())
                        }) {
                            Some(Ok(())) => status = "Creating game...".to_string(),
                            Some(Err(err)) => status = err,
                            None => status = "Matchmaking is not connected".to_string(),
                        }
                    }
                }
                ID_START => {
                    if matches!(mode, MenuMode::Hosted { .. }) {
                        match matchmaking_client
                            .as_ref()
                            .map(|session| session.start_game())
                        {
                            Some(Ok(())) => status = "Starting game...".to_string(),
                            Some(Err(err)) => status = err,
                            None => status = "Matchmaking is not connected".to_string(),
                        }
                    }
                }
                _ => {}
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(bg) = resources.menu_bg[2] {
            draw_screen_background(renderer, &bg);
        }
        render_menu(
            renderer,
            resources,
            transform,
            &mode,
            &games,
            &missions,
            selected,
            scroll_offset,
            &status,
        );
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);
        cursor_renderer.advance_animation();
        ModalCursor::new(cursor_renderer, MOUSE_OPACITY_DEFAULT, 0).draw(
            renderer,
            transform,
            &input_state,
        );
        renderer.present();
        crate::window::sleep_ms(16).await;
    }
}

fn set_button(frame: &mut FrameWnd, id: u32, label: &str, enabled: bool) {
    let Some(widget) = frame.widget_mut(id) else {
        panic!("Multiplayer menu: missing button widget {id}");
    };
    widget.base_mut().set_text(label);
    if widget.base().enabled != enabled {
        widget.set_enable(enabled);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_menu(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    mode: &MenuMode,
    games: &[GameListing],
    missions: &[MissionChoice],
    selected: usize,
    scroll_offset: usize,
    status: &str,
) {
    if let Some(font) = resources.title_font() {
        let title = match mode {
            MenuMode::Games => "Multiplayer",
            MenuMode::Missions => "Create Multiplayer Game",
            MenuMode::Hosted { .. } => "Game Lobby",
            MenuMode::Joined { .. } => "Game Lobby",
        };
        let tw = font.text_width(title);
        render_text_virt(renderer, font, transform, title, (MENU_W - tw) / 2, 24);
    }

    draw_panel(renderer, transform, &LIST_RECT);
    let rows: Vec<String> = match mode {
        MenuMode::Games => {
            if games.is_empty() {
                vec!["No games listed".to_string()]
            } else {
                games.iter().map(format_game_row).collect()
            }
        }
        MenuMode::Hosted { game, .. } => vec![format_game_row(game)],
        MenuMode::Joined { game, listing } => vec![
            listing
                .as_ref()
                .map(format_game_row)
                .unwrap_or_else(|| format!("{} | joined |  | waiting", game.mission_name)),
        ],
        MenuMode::Missions => missions
            .iter()
            .map(|m| format!("{} | {}", m.label, m.mission_id))
            .collect(),
    };
    for (visible_i, (row_idx, row)) in rows
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_row_count())
        .enumerate()
    {
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
                LIST_RECT.w - 8,
                ROW_HEIGHT,
                Renderer::create_color_16(72, 62, 34),
            );
        }
        if let Some(font) = resources.list_font(is_selected, is_selected) {
            let column_layout = menu_column_layout(mode);
            let row_area_x = (LIST_RECT.x + 10) as f32;
            let row_area_w = (LIST_RECT.w - 20) as f32;
            for cell in column_layout.layout_row(row, row_area_x, row_area_w) {
                let fitted = truncate_to_pixel_width(font, cell.text.trim(), cell.span_w as i32);
                if fitted.is_empty() {
                    continue;
                }
                let text_w = font.text_width(fitted) as f32;
                let cell_x = match cell.align {
                    ColumnAlign::Left => cell.span_x,
                    ColumnAlign::Center => cell.span_x + (cell.span_w - text_w) / 2.0,
                    ColumnAlign::Right => cell.span_x + cell.span_w - text_w,
                };
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    fitted,
                    cell_x.round() as i32,
                    LIST_RECT.y + 6 + visible_i as i32 * ROW_HEIGHT,
                );
            }
        }
    }

    if rows.len() > visible_row_count() {
        draw_scrollbar(renderer, transform, rows.len(), scroll_offset);
    }

    if let Some(font) = resources.menu_text_font() {
        render_text_virt(
            renderer,
            font,
            transform,
            status,
            LIST_RECT.x,
            LIST_RECT.y + LIST_RECT.h + 16,
        );
    }
}

fn visible_row_count() -> usize {
    ((LIST_RECT.h - 8) / ROW_HEIGHT).max(1) as usize
}

fn clamp_scroll_offset(offset: usize, rows_len: usize) -> usize {
    let visible = visible_row_count();
    offset.min(rows_len.saturating_sub(visible))
}

fn ensure_selected_visible(selected: usize, offset: &mut usize, rows_len: usize) {
    if rows_len == 0 {
        *offset = 0;
        return;
    }
    let visible = visible_row_count();
    if selected < *offset {
        *offset = selected;
    } else if selected >= *offset + visible {
        *offset = selected + 1 - visible;
    }
    *offset = clamp_scroll_offset(*offset, rows_len);
}

fn draw_scrollbar(
    renderer: &mut Renderer,
    transform: MenuTransform,
    rows_len: usize,
    scroll_offset: usize,
) {
    let track_x = LIST_RECT.x + LIST_RECT.w - 10;
    let track_y = LIST_RECT.y + 4;
    let track_w = 4;
    let track_h = LIST_RECT.h - 8;
    fill_virtual_rect(
        renderer,
        transform,
        track_x,
        track_y,
        track_w,
        track_h,
        Renderer::create_color_16(55, 47, 30),
    );

    let visible = visible_row_count().min(rows_len).max(1);
    let thumb_h = ((track_h as f32 * visible as f32 / rows_len as f32).round() as i32).max(12);
    let max_offset = rows_len.saturating_sub(visible);
    let travel = (track_h - thumb_h).max(0);
    let thumb_y = if max_offset == 0 {
        track_y
    } else {
        track_y + (travel as f32 * scroll_offset as f32 / max_offset as f32).round() as i32
    };
    fill_virtual_rect(
        renderer,
        transform,
        track_x,
        thumb_y,
        track_w,
        thumb_h,
        Renderer::create_color_16(172, 146, 84),
    );
}

fn mission_choices(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
) -> Vec<MissionChoice> {
    campaign
        .missions
        .iter()
        .map(|m| {
            let profile = m.profile(profiles);
            let label = if profile.mission_name.trim().is_empty() {
                if profile.mission_filename.trim().is_empty() {
                    format!("Mission {}", profile.id)
                } else {
                    profile.mission_filename.clone()
                }
            } else {
                profile.mission_name.clone()
            };
            MissionChoice {
                mission_id: profile.id,
                mission_name: label.clone(),
                label,
            }
        })
        .collect()
}

fn multiplayer_nickname(application_context: &ApplicationContext) -> String {
    let name = application_context
        .active_profile_snapshot()
        .unwrap_or_else(|error| panic!("multiplayer menu requires an active profile: {error}"))
        .name;
    if !name.trim().is_empty() {
        return name;
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "player".to_string())
}

fn upsert_game(games: &mut Vec<GameListing>, game: GameListing) {
    if let Some(existing) = games.iter_mut().find(|g| g.id == game.id) {
        *existing = game;
    } else {
        games.push(game);
    }
}

fn format_game_row(game: &GameListing) -> String {
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
    format!("{}|{}|{}|{}", game.mission_name, game.host, players, state)
}

fn menu_column_layout(mode: &MenuMode) -> ColumnLayout {
    match mode {
        MenuMode::Missions => {
            ColumnLayout::new(&[(0.82, ColumnAlign::Left), (0.18, ColumnAlign::Right)])
        }
        _ => ColumnLayout::new(&[
            (0.46, ColumnAlign::Left),
            (0.24, ColumnAlign::Left),
            (0.12, ColumnAlign::Center),
            (0.18, ColumnAlign::Left),
        ]),
    }
}

fn truncate_to_pixel_width<'a>(font: &Font, text: &'a str, max_w: i32) -> &'a str {
    if max_w <= 0 {
        return "";
    }
    if font.text_width(text) <= max_w {
        return text;
    }
    let mut fit_end = 0;
    for (idx, _) in text.char_indices() {
        if font.text_width(&text[..idx]) > max_w {
            return &text[..fit_end];
        }
        fit_end = idx;
    }
    &text[..fit_end]
}

fn draw_panel(renderer: &mut Renderer, transform: MenuTransform, rect: &MenuRect) {
    let (sx, sy) = transform.to_screen(rect.x, rect.y);
    renderer.fill_screen(
        Some(&BBox::from_coords(
            sx as f32,
            sy as f32,
            (sx + rect.w) as f32,
            (sy + rect.h) as f32,
        )),
        Renderer::create_color_16(28, 24, 16),
    );
    renderer.draw_rect_outline_screen(
        sx,
        sy,
        sx + rect.w,
        sy + rect.h,
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

//! In-game HUD text rendering using the native bitmap font system.
//!
//! Renders entity names above selected/hovered characters, HP text in
//! portrait slots, and ammunition counts in the bottom UI panel. Uses
//! the `PCPortrait` font for portrait text and the `Tooltips` font for
//! hover labels.

use crate::element::{Entity, EntityId};
use crate::host::ViewportState;
use crate::native_font::{self, NativeFont};
use crate::profiles;
use crate::renderer::Renderer;
use crate::ui_panel::PortraitCache;
use robin_engine::character_kind as engine_character_kind;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::player_command::PlayerId;

// ─── Layout constants (matching ui_panel.rs) ─────────────────────

const MARGIN: u16 = 32;
const NUMBER_OF_SLOTS: u16 = 5;
const ELEMENT_WIDTH: u16 = 112;
const BORDURE: u16 = 3;
const BOTTOM_SCROLL_HEIGHT: u16 = 23;
const ACTION_HEIGHT: u16 = 35;
const VISAGE_HEIGHT: u16 = 50;

const POSITION_BOTTOM_SCROLL: u16 = BORDURE + BOTTOM_SCROLL_HEIGHT;
const POSITION_ACTION: u16 = POSITION_BOTTOM_SCROLL + ACTION_HEIGHT;
const POSITION_VISAGE: u16 = POSITION_ACTION + VISAGE_HEIGHT;

// Closed state (non-selected) — no action area, scrolls compressed.
const CLOSE_POSITION_BOTTOM_SCROLL: u16 = POSITION_BOTTOM_SCROLL;
const CLOSE_POSITION_VISAGE: u16 = CLOSE_POSITION_BOTTOM_SCROLL + VISAGE_HEIGHT;

/// Text offset within the visage area.
/// The name is rendered in the right portion of the visage, beside the face.
const TEXT_OFFSET_X: i32 = 45;
const TEXT_OFFSET_Y: i32 = 10;

/// Text bounding box width. The companion height is computed from the
/// font metrics at render time.
const WIDTH_TEXT: i32 = 60;

/// Kerning margin inset on each side.
const KERNING_MARGIN: i32 = 2;

/// Line height reduction for native bitmap fonts. Subtracted from font
/// height to compute line spacing, making multi-line names more compact.
const NATIVE_FONT_LINE_SUBTRACT: i32 = 5;

/// Action button widths (3-button mode, from ui_panel.rs).
const ACTION1_WIDTH: u16 = 40;
const ACTION2_WIDTH: u16 = 32;
const ACTION3_WIDTH: u16 = 40;

/// Action button widths (2-button mode — peasants with action[2] == NoAction).
const ACTIONA_WIDTH: u16 = 56;
const ACTIONB_WIDTH: u16 = 56;

// ─── HUD font set ────────────────────────────────────────────────

/// Loaded fonts for in-game HUD text rendering.
///
/// - `tooltip_font`: "Tooltips" — entity names on hover
/// - `portrait_font`: "PCPortrait" — portrait name/HP text
/// - `shadow_font`: "Background" — dark shadow behind text for readability
pub struct HudFonts {
    pub tooltip_font: NativeFont,
    pub portrait_font: NativeFont,
    pub shadow_font: Option<NativeFont>,
}

impl HudFonts {
    /// Load HUD fonts from the font config file.
    ///
    /// Falls back gracefully: if portrait or shadow fonts are missing, uses
    /// the tooltip font. Returns `None` if no fonts can be loaded at all.
    pub fn load() -> Option<Self> {
        let config = native_font::load_font_config()
            .map_err(|e| tracing::warn!("HUD font config not available: {e}"))
            .ok()?;

        // HUD text uses the bitmap (native) rendering path — a TrueType
        // fallback would need a separate rasteriser. Skip the entry if
        // the font resolves to TrueType; the caller falls through to
        // the secondary key.
        let as_native = |name: &str| -> Option<NativeFont> {
            match native_font::load_font_by_name(&config, name) {
                Ok(native_font::Font::Native(f)) => Some(f),
                Ok(native_font::Font::TrueType(_)) => {
                    tracing::info!("HUD font '{name}' is TrueType-only; bitmap pipeline skips it");
                    None
                }
                Err(e) => {
                    tracing::info!("HUD font '{name}' not available: {e}");
                    None
                }
            }
        };

        let tooltip_font = as_native("Tooltips").or_else(|| as_native("PCPortrait"))?;

        let portrait_font = as_native("PCPortrait").unwrap_or_else(|| {
            tracing::info!("PCPortrait font not found, reusing tooltip font");
            as_native("Tooltips").expect("tooltip font was already loaded successfully")
        });

        let shadow_font = as_native("Background");

        tracing::info!(
            "HUD fonts loaded: tooltip={}px, portrait={}px, shadow={}",
            tooltip_font.height(),
            portrait_font.height(),
            if shadow_font.is_some() { "yes" } else { "no" },
        );

        Some(Self {
            tooltip_font,
            portrait_font,
            shadow_font,
        })
    }
}

// ─── Entity name helpers ─────────────────────────────────────────

/// Get the display name for an entity from host-side profile data.
///
/// PCs and soldiers resolve through the campaign's profile tables
/// (`campaign.profiles.characters` / `.soldiers`), with the
/// host-side `PortraitCache` providing localized overrides for the
/// seven hero profiles. For non-VIP civilians, we derive a stable
/// name deterministically from the entity ID — drawn from the
/// localised peasant name pool — so each civilian keeps the same name
/// across frames without needing mutable state on the civilian.
fn entity_display_name(
    engine: &Engine,
    assets: &LevelAssets,
    portraits: &PortraitCache,
    id: EntityId,
    entity: &Entity,
) -> Option<String> {
    match entity {
        Entity::Pc(_) => {
            let kind = engine.pc_character_kind(id)?;
            // Prefer the localized override from the host portrait
            // cache (VIP branch).
            if let Some(localized) = portraits.get_localized_name(kind) {
                return Some(localized.to_string());
            }
            Some(kind.profile_name().to_string())
        }
        Entity::Soldier(s) => assets
            .profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .map(|p| {
                if p.display_name.is_empty() {
                    p.profile_name.clone()
                } else {
                    p.display_name.clone()
                }
            }),
        Entity::Civilian(c) => {
            let civ_profile = assets
                .profile_manager
                .get_civilian(c.civilian.civilian_profile_index);
            let is_vip =
                civ_profile.is_some_and(|p| p.civilian_type == profiles::CivilianType::Vip);
            if !is_vip && let Some(name) = assets.random_peasant_name(id.index() as usize) {
                Some(name)
            } else {
                civ_profile.map(|p| {
                    if p.display_name.is_empty() {
                        p.profile_name.clone()
                    } else {
                        p.display_name.clone()
                    }
                })
            }
        }
        _ => None,
    }
}

/// Whether a PC entity is a VIP (main hero) vs a merry man (peasant).
///
/// Uses the `CharacterProfile::vip` flag when the profile is available,
/// falling back to the entity-kind heuristic otherwise.
fn is_vip_character(assets: &LevelAssets, entity: &Entity) -> bool {
    match entity {
        Entity::Pc(pc) => {
            if let Some(profile) = assets.profile_manager.get_character(pc.pc.profile_index) {
                return profile.vip;
            }
            // Fallback: VIP if not a merry-man template.
            !matches!(
                pc.pc.kind,
                Some(
                    engine_character_kind::CharacterKind::MerryManA
                        | engine_character_kind::CharacterKind::MerryManB
                        | engine_character_kind::CharacterKind::MerryManC
                ),
            )
        }
        _ => true,
    }
}

/// Prepare a portrait display name for rendering.
///
/// For non-VIP characters (merry men), replaces the first space with a
/// newline to force two-line name display.
fn prepare_portrait_name(name: &str, is_vip: bool) -> String {
    if !is_vip && let Some(pos) = name.find(' ') {
        let mut s = name.to_string();
        s.replace_range(pos..pos + 1, "\n");
        return s;
    }
    name.to_string()
}

/// Horizontal alignment for boxed text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Centered,
    Right,
}

// ─── Slot geometry helpers ───────────────────────────────────────

fn slot_width(screen_width: u16) -> u16 {
    (screen_width - 2 * MARGIN) / NUMBER_OF_SLOTS
}

fn slot_left_x(screen_width: u16, slot_index: u16) -> u16 {
    let sw = slot_width(screen_width);
    let position_in_slot = MARGIN + (sw - ELEMENT_WIDTH) / 2;
    slot_index * sw + position_in_slot
}

// ─── Rendering helpers ───────────────────────────────────────────

/// Canonical shadow+foreground text pass.
///
/// Invokes `render_at` four times with the shadow font (when available) at
/// the corner offsets `(-1,-1)`, `(-1,+1)`, `(+1,-1)`, `(+1,+1)`, then once
/// with the foreground font at `(x, y)`.  The closure abstracts over the
/// underlying rasterisation backend. Live HUD rendering routes this through
/// `Renderer::render_text_argb` so text is emitted from the GPU font atlas.
pub fn render_text_background<F>(
    foreground: &NativeFont,
    shadow: Option<&NativeFont>,
    text: &str,
    x: i32,
    y: i32,
    mut render_at: F,
) where
    F: FnMut(&NativeFont, &str, i32, i32),
{
    if let Some(shadow_font) = shadow {
        for &(dx, dy) in &[(-1i32, -1i32), (-1, 1), (1, -1), (1, 1)] {
            render_at(shadow_font, text, x + dx, y + dy);
        }
    }
    render_at(foreground, text, x, y);
}

fn render_text_with_shadow_gpu(
    renderer: &mut Renderer,
    font: &NativeFont,
    shadow: Option<&NativeFont>,
    text: &str,
    x: i32,
    y: i32,
) {
    render_text_background(font, shadow, text, x, y, |f, t, fx, fy| {
        renderer.render_text_argb(f, t, fx, fy);
    });
}

#[allow(clippy::too_many_arguments)]
fn render_text_in_box_gpu(
    renderer: &mut Renderer,
    font: &NativeFont,
    shadow: Option<&NativeFont>,
    text: &str,
    box_x: i32,
    box_y: i32,
    box_w: i32,
    alignment: Alignment,
) {
    let effective_w = box_w - 2 * KERNING_MARGIN;
    if effective_w <= 0 {
        return;
    }
    let line_height = (font.height() as i32 - NATIVE_FONT_LINE_SUBTRACT).max(1);
    let mut y = box_y;

    for segment in text.split('\n') {
        let words: Vec<&str> = segment.split_whitespace().collect();
        if words.is_empty() {
            y += line_height;
            continue;
        }

        let mut i = 0;
        while i < words.len() {
            let mut line = String::from(words[i]);
            let mut j = i + 1;
            while j < words.len() {
                let candidate = format!("{} {}", line, words[j]);
                if font.text_width(&candidate) > effective_w {
                    break;
                }
                line = candidate;
                j += 1;
            }

            if j < words.len() && j + 1 == words.len() && words[j].len() <= 5 && j > i + 1 {
                j -= 1;
                line = words[i..j].join(" ");
            }

            let tw = font.text_width(&line);
            let cx = match alignment {
                Alignment::Left => box_x + KERNING_MARGIN,
                Alignment::Centered => box_x + KERNING_MARGIN + (effective_w - tw) / 2,
                Alignment::Right => box_x + box_w - KERNING_MARGIN - tw,
            };
            render_text_with_shadow_gpu(renderer, font, shadow, &line, cx, y);
            y += line_height;
            i = j;
        }
    }
}

fn render_text_at_point_gpu(
    renderer: &mut Renderer,
    font: &NativeFont,
    shadow: Option<&NativeFont>,
    text: &str,
    x: i32,
    y: i32,
    align: Alignment,
) {
    let dx = match align {
        Alignment::Left => KERNING_MARGIN,
        Alignment::Centered => 0,
        Alignment::Right => -KERNING_MARGIN,
    };
    render_text_with_shadow_gpu(renderer, font, shadow, text, x + dx, y);
}

fn render_text_centered_gpu(
    renderer: &mut Renderer,
    font: &NativeFont,
    shadow: Option<&NativeFont>,
    text: &str,
    x_left: i32,
    x_right: i32,
    y: i32,
) {
    let tw = font.text_width(text);
    let region_w = x_right - x_left;
    let x = x_left + (region_w - tw) / 2;
    render_text_with_shadow_gpu(renderer, font, shadow, text, x, y);
}

// ─── Main HUD rendering ─────────────────────────────────────────

/// Render all in-game HUD text to the screen surface.
///
/// Called from the main game loop after UI panel rendering and before
/// the cursor. Renders:
/// 1. Portrait slot text: character name + HP in the bottom panel
/// 2. Ammunition counts below action buttons
/// 3. Floating counter titbits (coin pickups, etc.)
#[allow(clippy::too_many_arguments)]
pub fn render_hud_text(
    engine: &Engine,
    local_seat: PlayerId,
    camera: &ViewportState,
    assets: &LevelAssets,
    _draw_order: &[EntityId],
    portraits: &PortraitCache,
    renderer: &mut Renderer,
    fonts: &HudFonts,
) {
    debug_assert!(
        renderer.is_gpu_phase(),
        "render_hud_text runs after flush_base_layer"
    );

    let shadow = fonts.shadow_font.as_ref();
    render_portrait_text_gpu(
        engine, local_seat, assets, portraits, renderer, fonts, shadow,
    );
    render_ammo_counts_gpu(engine, local_seat, assets, renderer, fonts, shadow);
    render_counter_titbits_gpu(engine, camera, renderer, fonts, shadow);
}

/// Render a transient centered banner message near the bottom of the
/// screen. Drawn in the rect `(0, height-160, width, height)` with
/// `Centered` alignment while the message lifetime counter is
/// non-zero. The counter lives on `Game::message_delay` and is
/// decremented by the main loop after the render pass; this function
/// just paints it.
pub fn render_transient_message(renderer: &mut Renderer, fonts: &HudFonts, text: &str) {
    debug_assert!(
        renderer.is_gpu_phase(),
        "render_transient_message runs after flush_base_layer"
    );
    if text.is_empty() {
        return;
    }

    let sw = renderer.screen_width();
    let sh = renderer.screen_height();
    if sh < 160 {
        return;
    }
    let font = &fonts.tooltip_font;
    let shadow = fonts.shadow_font.as_ref();
    // Rect anchor is `(0, height-160, width, height)`. Draw the first
    // line at the top of that rect with Centered alignment.
    let y = (sh as i32) - 160;
    render_text_in_box_gpu(
        renderer,
        font,
        shadow,
        text,
        0,
        y,
        sw as i32,
        Alignment::Centered,
    );
}

/// Render the AI speech-log overlay.  Walks
/// `AiGlobalState::screen_remarks` and draws each `(prefix) Remark` line
/// in a horizontally-centred band starting at `y=30`, stepping 16 px
/// per entry. Gated on `host.info_displayed` (toggled by the bound
/// `DisplayInfo` / `RequestInfo` key). The timer-decrement + eviction
/// half lives in engine `tick_screen_remarks` so the list shrinks even
/// when the overlay is hidden.
pub fn render_screen_remarks(engine: &Engine, renderer: &mut Renderer, fonts: &HudFonts) {
    debug_assert!(
        renderer.is_gpu_phase(),
        "render_screen_remarks runs after flush_base_layer"
    );

    let remarks = &engine.ai_global().screen_remarks;
    if remarks.is_empty() {
        return;
    }

    let sw = renderer.screen_width();
    let sh = renderer.screen_height();
    // Right margin is 30 px.
    let x_left: i32 = 20;
    let x_right: i32 = sw as i32 - 30;
    if x_right <= x_left {
        return;
    }

    let font = &fonts.tooltip_font;
    let shadow = fonts.shadow_font.as_ref();
    // Start at y=30 and step 16 px per entry.
    let mut y: i32 = 30;
    for r in remarks {
        let line = format!("({}) {}", r.prefix, r.remark.speech());
        render_text_centered_gpu(renderer, font, shadow, &line, x_left, x_right, y);
        y += 16;
        if y + 16 > sh as i32 {
            break;
        }
    }
}

/// Dev overlay: draw each live entity's typed `EntityId` just below its feet.
/// Toggled by `/screenshot?entity_ids` over the HTTP RPC.
///
/// Shares the `render_hud_text` text-surface pattern: allocate a
/// transparent RGB565 surface, draw outlined text for every entity,
/// blit it back.  Cost is only paid when the flag is active, so this
/// is safe to call unconditionally from the render path.
pub fn render_entity_id_overlay(
    engine: &Engine,
    camera: &ViewportState,
    renderer: &mut Renderer,
    fonts: &HudFonts,
) {
    debug_assert!(
        renderer.is_gpu_phase(),
        "render_entity_id_overlay runs after flush_base_layer"
    );

    let font = &fonts.portrait_font;
    let shadow = fonts.shadow_font.as_ref();

    for (id, map_pt) in engine.active_entity_positions() {
        let Some(screen_pt) = camera.map_to_screen(map_pt) else {
            continue;
        };

        let text = id.to_string();
        let tw = font.text_width(&text);
        let text_x = screen_pt.x as i32 - tw / 2;
        // +6 px offset clears the feet / selection ring without
        // overlapping the sprite body.
        let text_y = screen_pt.y as i32 + 6;

        render_text_with_shadow_gpu(renderer, font, shadow, &text, text_x, text_y);
    }
}

fn render_counter_titbits_gpu(
    engine: &Engine,
    camera: &ViewportState,
    renderer: &mut Renderer,
    fonts: &HudFonts,
    shadow: Option<&NativeFont>,
) {
    use crate::titbit::TitbitKind;

    let font = &fonts.portrait_font;

    for titbit in engine.titbit_manager().titbits() {
        if titbit.kind != TitbitKind::Counter {
            continue;
        }

        let map_pt = engine_coordinates::MapPoint::new(titbit.position.x, titbit.position.y);
        let Some(screen_pt) = camera.map_to_screen(map_pt) else {
            continue;
        };

        let rise = 50.0 + titbit.sprite_frame as f32 * titbit.position.z;
        let text_y = screen_pt.y - rise;

        let text = titbit.phase.to_string();
        let tw = font.text_width(&text);
        let text_x = screen_pt.x as i32 - tw / 2;

        render_text_at_point_gpu(
            renderer,
            font,
            shadow,
            &text,
            text_x,
            text_y as i32,
            Alignment::Left,
        );
    }
}

fn render_portrait_text_gpu(
    engine: &Engine,
    local_seat: PlayerId,
    assets: &LevelAssets,
    portraits: &PortraitCache,
    renderer: &mut Renderer,
    fonts: &HudFonts,
    shadow: Option<&NativeFont>,
) {
    let font = &fonts.portrait_font;
    let sw = renderer.screen_width();
    let sh = renderer.screen_height();

    let num_portraits = engine.pc_ids().len().min(NUMBER_OF_SLOTS as usize);

    for slot in 0..num_portraits {
        let pc_id = engine.pc_ids()[slot];
        let entity = match engine.get_entity(pc_id) {
            Some(e) => e,
            None => continue,
        };

        let is_selected = engine.seat_selection(local_seat).contains(&pc_id);
        let is_burned = matches!(entity, Entity::Pc(pc) if pc.pc.life_points <= 0);
        if is_burned {
            continue;
        }

        let pos_visage = if is_selected {
            POSITION_VISAGE
        } else {
            CLOSE_POSITION_VISAGE
        };

        let x = slot_left_x(sw, slot as u16) as i32;

        let is_sword_fighting =
            matches!(entity, Entity::Pc(pc) if pc.actor.action_state.is_sword());
        let sword_visible =
            is_sword_fighting && (is_selected || (engine.frame_counter() / 10).is_multiple_of(2));

        let vis_top = (sh - pos_visage) as i32;
        let name =
            entity_display_name(engine, assets, portraits, pc_id, entity).unwrap_or_default();
        if !name.is_empty() && !sword_visible {
            let vip = is_vip_character(assets, entity);
            let display_name = prepare_portrait_name(&name, vip);
            let name_x = x + TEXT_OFFSET_X;
            let name_y = vis_top + TEXT_OFFSET_Y;
            render_text_in_box_gpu(
                renderer,
                font,
                shadow,
                &display_name,
                name_x,
                name_y,
                WIDTH_TEXT,
                Alignment::Centered,
            );
        }

        if let Some(label) = collect_peer_label(engine, pc_id) {
            let label_x = x + 4;
            let label_y = vis_top - font.height() as i32 - 1;
            render_text_in_box_gpu(
                renderer,
                font,
                shadow,
                &label,
                label_x,
                label_y,
                ELEMENT_WIDTH as i32 - 8,
                Alignment::Centered,
            );
        }
    }
}

/// Collect comma-joined nicknames of every active seat that currently
/// has `pc_id` in its selection.
fn collect_peer_label(engine: &Engine, pc_id: EntityId) -> Option<String> {
    let mut names = Vec::new();
    let mut active_count = 0usize;
    for (player_id, seat) in engine.active_seats() {
        active_count += 1;
        if !seat.selection.contains(&pc_id) {
            continue;
        }
        let nickname = if seat.nickname.is_empty() {
            // Fall back to "P{id}" when a peer joined without a
            // nickname (shouldn't happen in normal flow, but keeps
            // the label visible if a transport bug ever lets it).
            format!("P{}", player_id.0)
        } else {
            seat.nickname.clone()
        };
        names.push(nickname);
    }
    if active_count <= 1 {
        return None;
    }
    let mut label = names.join(", ");
    const MAX_LABEL_CHARS: usize = 18;
    if label.chars().count() > MAX_LABEL_CHARS {
        label = label
            .chars()
            .take(MAX_LABEL_CHARS.saturating_sub(3))
            .collect();
        label.push_str("...");
    }
    if label.is_empty() { None } else { Some(label) }
}

/// Whether an action type consumes ammunition (arrows, purses, etc.).
/// Matches the actions handled by `PcStatus::ammo_counter_mut`.
fn action_uses_ammo(action: profiles::Action) -> bool {
    matches!(
        action,
        profiles::Action::Ale
            | profiles::Action::Apple
            | profiles::Action::Bow
            | profiles::Action::Eat
            | profiles::Action::Guzzle
            | profiles::Action::Net
            | profiles::Action::Stone
            | profiles::Action::Heal
            | profiles::Action::Purse
            | profiles::Action::WaspNest
    )
}

/// Whether this PC uses two-button mode (action[2] == NoAction).
fn is_two_button_mode(assets: &LevelAssets, pc: &crate::element::ActorPc) -> bool {
    assets
        .profile_manager
        .get_character(pc.pc.profile_index)
        .is_some_and(|profile| profile.actions[2] == profiles::Action::NoAction)
}

/// Get the ammo quantities for a PC's 3 action buttons.
///
/// Returns `[Option<u16>; 3]` — `None` means that action has no ammo display
/// (e.g. melee actions). Reads from campaign PcStatus via profile matching.
fn pc_ammo_quantities(
    engine: &Engine,
    assets: &LevelAssets,
    pc: &crate::element::ActorPc,
) -> [Option<u16>; 3] {
    let campaign = engine.campaign();

    let profile_idx = pc.pc.profile_index;

    // Find the PcDescription whose character_profile_idx matches
    let pc_desc = campaign
        .characters
        .iter()
        .find(|desc| desc.character_profile_idx == Some(profile_idx));

    let (status, profile) = match (pc_desc, assets.profile_manager.get_character(profile_idx)) {
        (Some(desc), Some(prof)) => (&desc.status, prof),
        _ => return [None; 3],
    };

    let mut result = [None; 3];
    for (i, &action) in profile.actions.iter().enumerate() {
        let ammo = status.get_ammo(action);
        // Show ammo count only for actions that actually consume ammunition
        if action_uses_ammo(action) {
            result[i] = Some(ammo);
        }
    }
    result
}

fn render_ammo_counts_gpu(
    engine: &Engine,
    local_seat: PlayerId,
    assets: &LevelAssets,
    renderer: &mut Renderer,
    fonts: &HudFonts,
    shadow: Option<&NativeFont>,
) {
    let font = &fonts.portrait_font;
    let sw = renderer.screen_width();
    let sh = renderer.screen_height();

    let num_portraits = engine.pc_ids().len().min(NUMBER_OF_SLOTS as usize);

    for slot in 0..num_portraits {
        let pc_id = engine.pc_ids()[slot];
        if !engine.seat_selection(local_seat).contains(&pc_id) {
            continue;
        }
        let entity = match engine.get_entity(pc_id) {
            Some(Entity::Pc(pc)) => pc,
            _ => continue,
        };

        let quantities = pc_ammo_quantities(engine, assets, entity);
        let two_btn = is_two_button_mode(assets, entity);
        let (widths, num_buttons): (&[u16], usize) = if two_btn {
            (&[ACTIONA_WIDTH, ACTIONB_WIDTH], 2)
        } else {
            (&[ACTION1_WIDTH, ACTION2_WIDTH, ACTION3_WIDTH], 3)
        };

        let x = slot_left_x(sw, slot as u16) as i32;
        let scroll_top = (sh - POSITION_BOTTOM_SCROLL) as i32;
        let scroll_bot = (sh - BORDURE) as i32;
        let ammo_y = scroll_top + (scroll_bot - scroll_top - font.height() as i32) / 2;

        let mut btn_x = x;
        for i in 0..num_buttons {
            let btn_w = widths[i] as i32;
            if let Some(count) = quantities[i] {
                let text = count.to_string();
                let x_left = btn_x + if i == 0 { 8 } else { 0 };
                let x_right = btn_x + btn_w + if i == num_buttons - 1 { -8 } else { 0 };
                render_text_centered_gpu(renderer, font, shadow, &text, x_left, x_right, ammo_y);
            }
            btn_x += btn_w;
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_geometry_matches_ui_panel() {
        assert_eq!(slot_width(800), 147);
        for slot in 0..5 {
            let left = slot_left_x(800, slot);
            let right = left + ELEMENT_WIDTH;
            assert!(right <= 800, "slot {} overflows screen", slot);
        }
    }

    #[test]
    fn position_constants_consistent() {
        assert_eq!(POSITION_BOTTOM_SCROLL, 26);
        assert_eq!(POSITION_ACTION, 61);
        assert_eq!(POSITION_VISAGE, 111);
    }

    #[test]
    fn text_offset_constants() {
        assert_eq!(TEXT_OFFSET_X, 45);
        assert_eq!(TEXT_OFFSET_Y, 10);
        assert_eq!(WIDTH_TEXT, 60);
    }

    #[test]
    fn prepare_name_non_vip_wraps() {
        assert_eq!(prepare_portrait_name("John Smith", false), "John\nSmith");
        assert_eq!(prepare_portrait_name("SingleName", false), "SingleName");
    }

    #[test]
    fn prepare_name_vip_no_wrap() {
        assert_eq!(prepare_portrait_name("Robin Hood", true), "Robin Hood");
    }

    #[test]
    fn action_ammo_classification() {
        assert!(action_uses_ammo(profiles::Action::Bow));
        assert!(action_uses_ammo(profiles::Action::Stone));
        assert!(action_uses_ammo(profiles::Action::Purse));
        assert!(!action_uses_ammo(profiles::Action::Hit));
        assert!(!action_uses_ammo(profiles::Action::NoAction));
    }
}

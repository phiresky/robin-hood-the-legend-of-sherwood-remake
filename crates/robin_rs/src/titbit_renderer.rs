//! GPU renderer for titbit sprites (floating indicators).
//!
//! Loads all sprite rows from `Data/Interface/DEFAULT.RES` at startup,
//! converts each frame to an ARGB8888 GPU texture with shadow alpha
//! pre-baked, then in the GPU phase iterates `engine.titbit_manager()
//! .titbits()` and queues each one as a textured GPU draw.
//!
//! Sprite frames are uploaded through the wgpu renderer and cached by surface
//! id; this module owns only the game-facing row/frame metadata.
//!
//! The data side (`TitbitManager`, `TitbitInfo`, lifecycle) lives in
//! `robin_engine::titbit`.

use crate::gfx_types::BlendMode;
use crate::gfx_types::Rect;
use crate::host::Host;
use crate::host::HostTitbitPreview;
use robin_assets::picture::Picture;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::engine as engine_api;
use robin_engine::engine::Engine;
use robin_engine::graphic_config::TextureScaleMode;

use crate::renderer::TRANSPARENT_COLOR_KEY_16;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::profiles::Action;
use robin_engine::resource_ids::*;
use robin_engine::titbit::{SpriteRow, TitbitKind};

const NUM_ROWS: usize = SpriteRow::NumberOfRows as usize;

/// `(SpriteRow, resource_id)` mapping — the load order for titbit
/// sprite rows.  Several rows intentionally reuse the same resource
/// (a legacy quirk).  Exposed so the host can pre-count frames per
/// row without actually loading the GPU textures (used by
/// `game_session::extract_titbit_row_frame_counts`).
pub fn titbit_sprite_row_resources() -> &'static [(SpriteRow, i32)] {
    &[
        (SpriteRow::Impact, RHID_ONE_STAR),
        (SpriteRow::OneStar, RHID_ONE_STAR),
        (SpriteRow::TwoStars, RHID_TWO_STARS),
        (SpriteRow::ThreeStars, RHID_THREE_STARS),
        (SpriteRow::FourStars, RHID_FOUR_STARS),
        (SpriteRow::FiveStars, RHID_FIVE_STARS),
        (SpriteRow::QuickActionTitbits, RHID_QUICKACTION_TITBITS),
        (SpriteRow::Smoke, RHID_ONE_STAR),
        (SpriteRow::Water, RHID_TITBIT_WATER),
        (SpriteRow::Lock, RHID_TITBIT_WATER),
        (SpriteRow::EmoticonGrowingQMark, RHID_EMOTICONS_WHAT1),
        (SpriteRow::EmoticonQMark, RHID_EMOTICONS_WHAT2),
        (SpriteRow::EmoticonXMark, RHID_EMOTICONS_ACH),
        (SpriteRow::EmoticonZzz, RHIDEMOTICONS_ZZZ),
        (SpriteRow::EmoticonThunderstorm, RHID_EMOTICONS_ANGRY),
        (SpriteRow::EmoticonCloud, RHID_EMOTICONS_DISAPPOINTED),
        (SpriteRow::EmoticonDrunken, RHID_EMOTICONS_DRUNKEN),
        (SpriteRow::EmoticonSun, RHID_EMOTICONS_HAPPY),
        (SpriteRow::EmoticonKo, RHID_EMOTICONS_KO),
        (SpriteRow::Plouf, RHID_TITBIT_PLOUF),
        (SpriteRow::Ghost, RHID_GHOST_LITTLE_JOHN_SHORT_LEGS),
        (SpriteRow::AppleSmell, RHID_TITBIT_APPLE_SMELL),
        (SpriteRow::Speak, RHID_TITBIT_SPEAK),
        (SpriteRow::DangerPoint, RHID_TITBIT_DANGER_POINT),
        (SpriteRow::Hidden, RHID_TITBIT_HIDDEN),
        (SpriteRow::WorkIconArrows, RHWORKICON_ARROWS),
        (SpriteRow::WorkIconPurses, RHWORKICON_PURSES),
        (SpriteRow::WorkIconStones, RHWORKICON_STONES),
        (SpriteRow::WorkIconApples, RHWORKICON_APPLES),
        (SpriteRow::WorkIconBeer, RHWORKICON_BEER),
        (SpriteRow::WorkIconLegs, RHWORKICON_LEGS),
        (SpriteRow::WorkIconPlants, RHWORKICON_PLANTS),
        (SpriteRow::WorkIconNets, RHWORKICON_NETS),
        (SpriteRow::WorkIconWasps, RHWORKICON_WASPS),
        (SpriteRow::WorkIconBowTraining, RHWORKICON_BOW_TRAINING),
        (SpriteRow::WorkIconSwordTraining, RHWORKICON_SWORD_TRAINING),
        (SpriteRow::WorkIconRegeneration, RHWORKICON_REGENERATE),
    ]
}

/// Default day-ambience night-shadow color (`CreateColor(45, 45, 35)` packed
/// as RGB565).  Matches `markers::SelectionMarkRenderer`'s default and the
/// day-ambience night colour.
///
/// Used as the fallback shadow color when the engine hasn't loaded a level yet.
const DEFAULT_SHADOW_COLOR: u16 = 0x2964;

/// Global shadow opacity used by entity sprites (matches
/// `frame_holder::FrameHolder::global_shadow()` default of 40).
const SHADOW_LEVEL: u16 = 40;

/// Shadow opacity used for UI titbits drawn through the shifting UI
/// renderer (the QA-macro strip).  Matches the UI shadow renderer's
/// default intensity of 50.
const UI_SHADOW_LEVEL: u16 = 50;
const SPRITE_SHADOW_KEY_16: u16 = 0x001F;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TitbitAlphaMode {
    SolidWithShadow,
    BlueChannel,
    ConstantPercent(u16),
}

/// One frame of a titbit sprite — owns its wgpu texture + view.
struct TitbitFrame {
    /// Held alive for `view`'s lifetime; `view` is the only thing
    /// the renderer touches per-draw.
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u16,
    height: u16,
    offset_x: i16,
    offset_y: i16,
}

/// Loaded titbit sprite atlas + GPU render path.
///
/// Owns one [`TitbitFrame`] per (row, frame) combination, stored as
/// ready-to-blit GPU textures rather than packed sprite data.
///
/// The lifetime parameter `'a` ties the contained textures to the
/// [`TextureCreator`] passed to [`TitbitRenderer::load`].
pub struct TitbitRenderer {
    /// `rows[row_index]` = list of frames for that sprite row.
    /// Empty if the resource failed to load.
    rows: Vec<Vec<TitbitFrame>>,
    /// Maximum frame width per row.  Used for centering stars/lock/hidden
    /// so the sprite doesn't jitter as frame sizes vary across the
    /// animation.
    row_max_width: Vec<u16>,
    /// Maximum frame height per row (same purpose).
    row_max_height: Vec<u16>,
    /// Per-frame render cursor into `engine.titbit_manager().titbits()`.
    /// Advances monotonically as `render_up_to(..., display_order_max)` is
    /// called in-between entity draws to produce a back-to-front interleave.
    render_cursor: usize,
    /// Whether the current frame's host-side titbit preview has already
    /// been emitted into the interleaved titbit/entity stream.
    host_preview_rendered: bool,
}

impl Default for TitbitRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TitbitRenderer {
    pub fn new() -> Self {
        Self {
            rows: (0..NUM_ROWS).map(|_| Vec::new()).collect(),
            row_max_width: vec![0; NUM_ROWS],
            row_max_height: vec![0; NUM_ROWS],
            render_cursor: 0,
            host_preview_rendered: false,
        }
    }

    /// Load all titbit sprite rows from the resource manager and upload
    /// them as GPU textures owned by `creator`.
    ///
    /// `shadow_color` is the current ambience's night color
    /// (`engine.weather().night_color`).  Pass `0` to use the day default.
    pub fn load(
        &mut self,
        resource_manager: &mut ResourceManager,
        gpu: &crate::window::GpuContext,
        shadow_color: u16,
        scale_mode: TextureScaleMode,
    ) {
        let shadow_color = if shadow_color == 0 {
            DEFAULT_SHADOW_COLOR
        } else {
            shadow_color
        };
        let mappings = titbit_sprite_row_resources();

        let mut total_frames = 0usize;
        for &(row, resource_id) in mappings {
            // The Ghost titbit uses a wipe-shadow create flag that
            // zeroes out shadow pixels instead of writing the ambience
            // shadow colour.  Apply the same effect at load time by
            // wiping shadow alpha so the ghost silhouette doesn't drag
            // a shadow layer around.
            let wipe_shadow = matches!(row, SpriteRow::Ghost);
            // QA-strip titbits are drawn through the shifting UI renderer,
            // which uses the UI shadow intensity default of 50 rather
            // than the in-world sprite default of 40.
            let shadow_level = if matches!(row, SpriteRow::QuickActionTitbits) {
                UI_SHADOW_LEVEL
            } else {
                SHADOW_LEVEL
            };
            let alpha_mode = match row {
                // legacy implementation draws these live effects with SBDRAW_ALPHABLUEONLY.
                SpriteRow::Water | SpriteRow::Plouf => TitbitAlphaMode::BlueChannel,
                // Jump-helper ghost uses BlitAlphaConstant(..., 70, ...).
                SpriteRow::Ghost => TitbitAlphaMode::ConstantPercent(70),
                _ => TitbitAlphaMode::SolidWithShadow,
            };
            let frames = load_row(
                resource_manager,
                gpu,
                resource_id,
                shadow_color,
                wipe_shadow,
                shadow_level,
                alpha_mode,
                scale_mode,
            );
            if !frames.is_empty() {
                total_frames += frames.len();
                let ri = row as usize;
                let max_w = frames.iter().map(|f| f.width).max().unwrap_or(0);
                let max_h = frames.iter().map(|f| f.height).max().unwrap_or(0);
                self.row_max_width[ri] = max_w;
                self.row_max_height[ri] = max_h;
                self.rows[ri] = frames;
            }
        }

        tracing::info!(
            "TitbitRenderer: loaded {total_frames} frames across {} rows",
            self.rows.iter().filter(|r| !r.is_empty()).count(),
        );
    }

    /// Blit a single titbit frame to UI screen coordinates (no world
    /// transform, no shadowing tweaks).  Used by the UI panel to
    /// overlay the `RHID_QUICKACTION_TITBITS` frame of the current step
    /// on top of each QA macro icon.
    ///
    /// `cell.x()/cell.y()` is the top-left of the widget's refresh box
    /// (already shifted by the `SHIFT_STEP` fall phase).  The sprite is
    /// inset by `(4, 4)` and then centred inside the row's `(max_w,
    /// max_h)` bounding box.  `cell`'s width/height are ignored (the
    /// centring box is the row max, not the widget slot).
    ///
    /// When `run` is true, a second copy of the same sprite is blitted
    /// offset by `(3, 0)` — the QA-run double-image indicator.
    ///
    /// Returns `true` if a frame was drawn.
    pub fn blit_ui_frame(
        &mut self,
        renderer: &mut crate::renderer::Renderer,
        row: robin_engine::titbit::SpriteRow,
        frame: u16,
        cell: Rect,
        run: bool,
    ) -> bool {
        let row_idx = row as usize;
        let max_w = self.row_max_width[row_idx];
        let max_h = self.row_max_height[row_idx];
        let Some((view, w, h, ox, oy)) = self.get_frame_view(row as u16, frame) else {
            return false;
        };
        let off_x = 4 + (max_w as i32 - w as i32) / 2 + ox as i32;
        let off_y = 4 + (max_h as i32 - h as i32) / 2 + oy as i32;
        let dst = crate::gfx_types::Rect {
            x: cell.x() + off_x,
            y: cell.y() + off_y,
            w: w as i32,
            h: h as i32,
        };
        renderer.enqueue_external_texture(
            view,
            dst,
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            BlendMode::Blend,
        );
        if run {
            let dst2 = crate::gfx_types::Rect {
                x: dst.x + 3,
                y: dst.y,
                w: w as i32,
                h: h as i32,
            };
            renderer.enqueue_external_texture(
                view,
                dst2,
                [0.0, 0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
                BlendMode::Blend,
            );
        }
        true
    }

    /// Borrow the wgpu view + dimensions for `(row, frame)`.
    /// Frame index wraps modulo row length.
    fn get_frame_view(
        &self,
        row: u16,
        frame: u16,
    ) -> Option<(&wgpu::TextureView, u16, u16, i16, i16)> {
        let row_idx = row as usize;
        let frames = self.rows.get(row_idx)?;
        if frames.is_empty() {
            return None;
        }
        let idx = frame as usize % frames.len();
        let f = &frames[idx];
        Some((&f.view, f.width, f.height, f.offset_x, f.offset_y))
    }

    /// Reset the per-frame titbit render cursor.  Must be called once at
    /// the start of each frame, before any `render_up_to` calls, so the
    /// interleaved draw restarts from titbit 0.
    pub fn begin_frame(&mut self) {
        self.render_cursor = 0;
        self.host_preview_rendered = false;
    }

    fn render_host_preview_if_due(
        &mut self,
        host: &Host,
        engine: &Engine,
        renderer: &mut crate::renderer::Renderer,
        next_sim_display_order: f32,
        display_order_max: f32,
    ) -> bool {
        let Some(preview) = host.host_titbit_preview else {
            return false;
        };
        if self.host_preview_rendered {
            return false;
        }
        let display_order = preview.display_order();
        if display_order > display_order_max || display_order > next_sim_display_order {
            return false;
        }

        self.host_preview_rendered = true;
        self.render_host_preview(host, engine, renderer, preview);
        true
    }

    fn render_host_preview(
        &self,
        host: &Host,
        engine: &Engine,
        renderer: &mut crate::renderer::Renderer,
        preview: HostTitbitPreview,
    ) {
        match preview {
            HostTitbitPreview::JumpHelperGhost {
                position,
                layer: _layer,
                sector_dir,
                display_order: _display_order,
            } => {
                if (engine.frame_counter() / robin_engine::titbit::GHOST_BLINK) & 0x1 == 0 {
                    return;
                }
                let Some((view, w, h, _ox, _oy)) =
                    self.get_frame_view(SpriteRow::Ghost as u16, sector_dir + 7)
                else {
                    return;
                };
                let map_pt = engine_coordinates::MapPoint::from_world_xyz(
                    position.x, position.y, position.z,
                );
                let Some(screen_pt) = host.viewport.map_to_screen(map_pt) else {
                    return;
                };
                renderer.enqueue_external_texture(
                    view,
                    crate::gfx_types::Rect {
                        x: floor_centered(screen_pt.x, w),
                        y: floor_bottom(screen_pt.y, h),
                        w: w as i32,
                        h: h as i32,
                    },
                    [0.0, 0.0, 1.0, 1.0],
                    [1.0, 1.0, 1.0, 1.0],
                    BlendMode::Blend,
                );
            }
        }
    }

    /// Render every remaining titbit whose `display_order` is `<=
    /// display_order_max`, advancing the per-frame cursor.
    ///
    /// Pass `f32::INFINITY` to flush everything.
    ///
    /// The cursor walks monotonically through the display-order-sorted
    /// titbit list, drawing titbits that fall behind (or tied with) the
    /// caller-supplied cutoff, leaving titbits with larger display order
    /// for a later call.  Called by `render_entities_gpu` immediately
    /// before each human entity so titbits sit behind entities that
    /// occlude them and in front of entities they occlude.
    ///
    /// Per-titbit positioning:
    /// - Stars/emoticons/speak/apple_smell → `ComputeStarsPoint` + kind-
    ///   specific vertical offset (set in `refresh_titbit_positions`).
    /// - Lock → `ComputeFeetPoint`, no vertical offset.
    /// - Hidden → entity position + posture Z, centered on row max dims.
    /// - QuickAction → entity position, sprite anchored 50px above.
    /// - DangerPoint/QuickAction — only rendered when entity is selected.
    /// - Ghost — frame index offset by +7.
    pub fn render_up_to(
        &mut self,
        host: &mut Host,
        engine: &Engine,
        assets: &engine_api::LevelAssets,
        renderer: &mut crate::renderer::Renderer,
        display_order_max: f32,
    ) {
        let blink_off =
            engine.titbit_manager().blink_counter() < robin_engine::titbit::TIME_BLINK_OFF_RAW;

        let all = engine.titbit_manager().titbits();
        while self.render_cursor < all.len()
            && all[self.render_cursor].display_order <= display_order_max
        {
            if self.render_host_preview_if_due(
                host,
                engine,
                renderer,
                all[self.render_cursor].display_order,
                display_order_max,
            ) {
                continue;
            }

            let titbit = &all[self.render_cursor];
            self.render_cursor += 1;
            // Blinking titbits hide during the "off" portion of the cycle.
            if titbit.blinking && blink_off {
                continue;
            }

            // Counter titbits render their phase (damage number) as
            // text via `hud_text::render_counter_titbits`, not through
            // the sprite path. Skip here.
            if titbit.kind == TitbitKind::Counter {
                continue;
            }

            // ── Visibility guards (per-kind checks) ──

            // QuickAction: only show when the managing PC is selected
            // (or the titbit is blinking).
            if matches!(
                titbit.kind,
                TitbitKind::QuickAction | TitbitKind::QuickActionRun
            ) && !titbit.blinking
            {
                let mgr = titbit.element_manager.0;
                if !engine.selected_pc_ids().iter().any(|&id| id.index() == mgr) {
                    continue;
                }
            }

            // DangerPoint: only show when the managing PC is selected.
            if titbit.kind == TitbitKind::DangerPoint {
                let mgr = titbit.element_manager.0;
                if !engine.selected_pc_ids().iter().any(|&id| id.index() == mgr) {
                    continue;
                }
            }

            // Emoticon: skip if entity is blipped or hidden in a
            // building (i.e. the entity must be active and outside a
            // building, and not blipped).  The `draw_hidden` debug
            // toggle (`MSG_SWITCH_MASKED_DISPLAY`) overrides the
            // blipped / in-building skip so the inspector can see AI
            // reactions through walls — active+alive guard stays.
            if titbit.kind == TitbitKind::Emoticon
                && titbit.element_supplier.is_valid()
                && let Some(entity_id) = engine.entity_id_for_index(titbit.element_supplier.0)
                && let Some(entity) = engine.get_entity(entity_id)
            {
                let elem = entity.element_data();
                if !elem.active {
                    continue;
                }
                if !host.input.draw_hidden && (elem.blipped || elem.hidden_in_building) {
                    continue;
                }
            }

            // WeakStunned/Speak: skip if entity is out of order or
            // inactive.
            if matches!(titbit.kind, TitbitKind::WeakStunned | TitbitKind::Speak)
                && titbit.element_supplier.is_valid()
                && let Some(entity_id) = engine.entity_id_for_index(titbit.element_supplier.0)
                && let Some(entity) = engine.get_entity(entity_id)
                && !entity.is_active()
            {
                continue;
            }

            // Speak: also skip when the supplier is blipped or hidden
            // inside a building — the speak titbit only renders when
            // the entity is neither blipped nor inside a building.
            if titbit.kind == TitbitKind::Speak
                && titbit.element_supplier.is_valid()
                && let Some(entity_id) = engine.entity_id_for_index(titbit.element_supplier.0)
                && let Some(entity) = engine.get_entity(entity_id)
            {
                let elem = entity.element_data();
                if elem.blipped || elem.hidden_in_building {
                    continue;
                }
            }

            // WorkIcon: skip if entity is inactive.
            if titbit.kind == TitbitKind::WorkIcon
                && titbit.element_supplier.is_valid()
                && let Some(entity_id) = engine.entity_id_for_index(titbit.element_supplier.0)
                && let Some(entity) = engine.get_entity(entity_id)
                && !entity.is_active()
            {
                continue;
            }

            // WorkIcon: also skip while the men-to-blazon conversion
            // screen is up — the per-PC work icon is suppressed for
            // the duration of the conversion UI.
            if titbit.kind == TitbitKind::WorkIcon && engine.is_men_to_blazon_conversion_mode() {
                continue;
            }

            // WorkIcon BowTraining: skip when the PC's profile lacks
            // `Action::Bow` — hides the bow-training icon for PCs
            // who can't currently use a bow even if their work icon
            // is set to BowTraining.
            if titbit.kind == TitbitKind::WorkIcon
                && titbit.sprite_row == SpriteRow::WorkIconBowTraining as u16
                && titbit.element_supplier.is_valid()
                && let Some(entity_id) = engine.entity_id_for_index(titbit.element_supplier.0)
                && let Some(entity) = engine.get_entity(entity_id)
            {
                let has_bow = entity.pc_data().is_some_and(|pc| {
                    assets
                        .profile_manager
                        .get_character(pc.profile_index)
                        .is_some_and(|p| p.has_action(Action::Bow))
                });
                if !has_bow {
                    continue;
                }
            }

            // Ghost: blink at half rate.  Only renders when the
            // alternating phase `(frame_counter / GHOST_BLINK) & 0x1`
            // is set.
            if titbit.kind == TitbitKind::Ghost
                && (engine.frame_counter() / robin_engine::titbit::GHOST_BLINK) & 0x1 == 0
            {
                continue;
            }

            // UnconsciousStar: per-frame animation gate.  The stars
            // sprite is restricted to the idle KO animations
            // (`BeingUnconscious{,Bow,Sword}`); during the falling /
            // transition frames between knockout and the idle-unconscious
            // animation the titbit exists but is not drawn.
            if titbit.kind == TitbitKind::UnconsciousStar
                && titbit.element_supplier.is_valid()
                && !engine
                    .entity_id_for_index(titbit.element_supplier.0)
                    .is_some_and(|id| engine.can_have_unconscious_stars(id))
            {
                continue;
            }

            // ── Frame selection ──

            // Ghost: add +7 to sprite_frame.
            // Growing question mark: size * 8 + local_frame.
            let effective_frame = effective_titbit_frame(
                titbit.kind,
                titbit.sprite_row,
                titbit.phase,
                titbit.sprite_frame,
            );

            // Read row-max dimensions before the mutable borrow from
            // get_frame_mut, which prevents overlapping borrows on self.
            let row_idx = titbit.sprite_row as usize;
            let row_mw = self.row_max_width.get(row_idx).copied();
            let row_mh = self.row_max_height.get(row_idx).copied();

            let Some((view, w, h, ox, oy)) =
                self.get_frame_view(titbit.sprite_row, effective_frame)
            else {
                continue;
            };

            // GunImpact uses an alpha-red-only blend that produces an
            // additive red muzzle flash.  Approximate with additive
            // blend + red tint; the underlying sprite is already red-heavy
            // so additive blending lands close.
            let gun_impact = titbit.kind == TitbitKind::GunImpact;

            // Per-draw blend + tint derived from the kind.
            let (blend, tint) = if gun_impact {
                (BlendMode::Add, [1.0, 0.0, 0.0, 1.0])
            } else {
                // For star titbits the keyed alpha applies to the
                // shadow/keyed colour, not to every coloured pixel —
                // `load_row` already bakes that keyed shadow alpha
                // into the texture.
                (BlendMode::Blend, [1.0, 1.0, 1.0, 1.0])
            };

            // Convert 3D world position to 2D map: (x, y - z).
            let map_pt = engine_coordinates::MapPoint::from_world_xyz(
                titbit.position.x,
                titbit.position.y,
                titbit.position.z,
            );
            let Some(screen_pt) = host.viewport.map_to_screen(map_pt) else {
                continue;
            };

            // ── QuickAction: special positioning ──
            // QA icons sit at positionMap - (0.5*spriteWidth,
            // spriteHeight + 50), i.e., 50px above the entity, anchored
            // at bottom-center. QuickActionRun gets +3 X offset.
            if matches!(
                titbit.kind,
                TitbitKind::QuickAction | TitbitKind::QuickActionRun
            ) {
                let mut dst_x = floor_centered(screen_pt.x, w) + ox as i32;
                let dst_y = floor_anchor(screen_pt.y, h as i32 + 50) + oy as i32;
                if titbit.kind == TitbitKind::QuickActionRun {
                    dst_x += 3;
                }
                renderer.enqueue_external_texture(
                    view,
                    crate::gfx_types::Rect {
                        x: dst_x,
                        y: dst_y,
                        w: w as i32,
                        h: h as i32,
                    },
                    [0.0, 0.0, 1.0, 1.0],
                    tint,
                    blend,
                );
                continue;
            }

            let (dst_x, dst_y) = match titbit.kind {
                // These kinds call SetPositionSprite explicitly in legacy implementation;
                // GenerateBlitBox then adds the cropped-frame offset.
                TitbitKind::Emoticon => {
                    let row = titbit.sprite_row;
                    let (vertical_offset, center_vertical) = if row
                        == SpriteRow::EmoticonThunderstorm as u16
                        || row == SpriteRow::EmoticonCloud as u16
                    {
                        (25, false)
                    } else if row == SpriteRow::EmoticonDrunken as u16
                        || row == SpriteRow::EmoticonSun as u16
                    {
                        (15, true)
                    } else {
                        // GrowingQMark, QMark, XMark, Zzz, Ko
                        (12, true)
                    };
                    let center_w = row_mw.unwrap_or(w);
                    let center_h = row_mh.unwrap_or(h);
                    let x = floor_centered(screen_pt.x, center_w) + ox as i32;
                    let y = if center_vertical {
                        floor_centered(screen_pt.y - vertical_offset as f32, center_h) + oy as i32
                    } else {
                        floor_anchor(screen_pt.y, vertical_offset) + oy as i32
                    };
                    (x, y)
                }
                TitbitKind::UnconsciousStar if titbit.element_supplier.is_valid() => (
                    floor_centered(screen_pt.x, row_mw.unwrap_or(w)) + ox as i32,
                    floor_centered(screen_pt.y - 10.0, row_mh.unwrap_or(h)) + oy as i32,
                ),
                TitbitKind::WeakStunned | TitbitKind::AppleSmell | TitbitKind::Speak => (
                    floor_centered(screen_pt.x, row_mw.unwrap_or(w)) + ox as i32,
                    floor_centered(screen_pt.y - 10.0, row_mh.unwrap_or(h)) + oy as i32,
                ),
                TitbitKind::Lock | TitbitKind::Hidden => (
                    floor_centered(screen_pt.x, row_mw.unwrap_or(w)) + ox as i32,
                    floor_centered(screen_pt.y, row_mh.unwrap_or(h)) + oy as i32,
                ),
                TitbitKind::DangerPoint => (
                    floor_centered(screen_pt.x, w) + ox as i32,
                    floor_centered(screen_pt.y, h) + oy as i32,
                ),

                // These kinds rely on the final legacy implementation SetSpriteCenter()
                // pass. That center includes GetCurrentOffset(), so the
                // offset cancels out when GenerateBlitBox adds it.
                TitbitKind::Water | TitbitKind::Plouf | TitbitKind::WorkIcon => (
                    floor_centered(screen_pt.x, w),
                    floor_centered(screen_pt.y, h),
                ),
                TitbitKind::GunImpact
                | TitbitKind::Smoke
                | TitbitKind::Dust
                | TitbitKind::Ghost
                | TitbitKind::UnconsciousStar => {
                    (floor_centered(screen_pt.x, w), floor_bottom(screen_pt.y, h))
                }
                _ => (
                    floor_centered(screen_pt.x, w),
                    floor_centered(screen_pt.y, h),
                ),
            };
            renderer.enqueue_external_texture(
                view,
                crate::gfx_types::Rect {
                    x: dst_x,
                    y: dst_y,
                    w: w as i32,
                    h: h as i32,
                },
                [0.0, 0.0, 1.0, 1.0],
                tint,
                blend,
            );
        }
        self.render_host_preview_if_due(host, engine, renderer, f32::INFINITY, display_order_max);
    }
}

/// Select the resource frame forced by the original titbit renderer.
///
/// Hidden indicators are a static strip with one portrait per PC, so their
/// `phase` is the frame index. Other animated titbits normally use
/// `sprite_frame`, with the two legacy exceptions below.
fn effective_titbit_frame(kind: TitbitKind, row: u16, phase: u16, sprite_frame: u16) -> u16 {
    if kind == TitbitKind::Ghost {
        sprite_frame + 7
    } else if kind == TitbitKind::Hidden {
        phase
    } else if kind == TitbitKind::Emoticon && row == SpriteRow::EmoticonGrowingQMark as u16 {
        phase * 8 + sprite_frame
    } else {
        sprite_frame
    }
}

fn floor_centered(anchor: f32, extent: u16) -> i32 {
    (anchor - 0.5 * extent as f32).floor() as i32
}

fn floor_anchor(anchor: f32, offset: i32) -> i32 {
    (anchor - offset as f32).floor() as i32
}

fn floor_bottom(anchor: f32, extent: u16) -> i32 {
    (anchor - extent as f32).floor() as i32
}

/// Load every sub-picture of a resource into a vector of GPU textures.
///
/// Each sub-picture is converted RGB565 → ARGB8888 with the shadow key
/// (`0x001F`) replaced by the day-ambience shadow color, and that shadow
/// color baked to a semi-transparent alpha so the GPU blend produces
/// the same dim grey shadow effect as the original software blit path.
#[allow(clippy::too_many_arguments)]
fn load_row(
    resource_manager: &mut ResourceManager,
    gpu: &crate::window::GpuContext,
    resource_id: i32,
    shadow_color: u16,
    wipe_shadow: bool,
    shadow_level: u16,
    alpha_mode: TitbitAlphaMode,
    _scale_mode: TextureScaleMode,
) -> Vec<TitbitFrame> {
    let pictures = match resource_manager.get_pictures(resource_id) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("TitbitRenderer: failed to load resource {resource_id}: {e}");
            return Vec::new();
        }
    };

    // Clone slot vector to release the &mut borrow on resource_manager.
    let owned: Vec<Option<Picture>> = pictures.to_vec();

    let mut frames = Vec::with_capacity(owned.len());
    for slot in &owned {
        let Some(pic) = slot else { continue };
        if pic.width == 0 || pic.height == 0 {
            continue;
        }

        // RGB565 source pixels.
        let source_pixels: Vec<u16> = pic
            .data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let Some((crop_x, crop_y, crop_w, crop_h)) = pic.opaque_bounds_16() else {
            continue;
        };
        let mut pixels = Vec::with_capacity(crop_w as usize * crop_h as usize);
        for y in 0..crop_h as usize {
            let src = (crop_y as usize + y) * pic.width as usize + crop_x as usize;
            pixels.extend_from_slice(&source_pixels[src..src + crop_w as usize]);
        }

        // Replace the magic blue shadow key (0x001F) with the current
        // ambience shadow color for ordinary sprite draws. Blue-channel
        // alpha effects must preserve the raw blue component because legacy implementation
        // uses it as the alpha map.
        if alpha_mode == TitbitAlphaMode::SolidWithShadow {
            crate::markers::apply_arno_law(&mut pixels, shadow_color);
        }

        // Convert to ARGB8888 with shadow alpha pre-baked.
        // `wipe_shadow` overrides the shadow bake and treats shadow
        // pixels as fully transparent.
        let argb_bytes = rgb565_to_argb8888(
            &pixels,
            crop_w,
            crop_h,
            TRANSPARENT_COLOR_KEY_16,
            shadow_color,
            if wipe_shadow { 100 } else { shadow_level },
            alpha_mode,
        );

        // Build the wgpu RGBA8 texture directly. argb_bytes is
        // ARGB8888 little-endian, i.e. [B, G, R, A] in memory.
        // Swizzle to [R, G, B, A] for wgpu's Rgba8UnormSrgb.
        let mut rgba = Vec::with_capacity(argb_bytes.len());
        for px in argb_bytes.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
        let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("titbit res={resource_id}")),
            size: wgpu::Extent3d {
                width: crop_w as u32,
                height: crop_h as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(crop_w as u32 * 4),
                rows_per_image: Some(crop_h as u32),
            },
            wgpu::Extent3d {
                width: crop_w as u32,
                height: crop_h as u32,
                depth_or_array_layers: 1,
            },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());

        frames.push(TitbitFrame {
            _texture: tex,
            view,
            width: crop_w,
            height: crop_h,
            offset_x: crop_x as i16,
            offset_y: crop_y as i16,
        });
    }

    frames
}

/// Convert RGB565 pixels to ARGB8888 bytes with shadow alpha pre-baked.
///
/// For each input pixel:
/// - Equal to `transparent` → alpha 0 (skipped)
/// - In `SolidWithShadow`, equal to `shadow_color` → multiply-darken: black tint with
///   `alpha = shadow_level * 255 / 100`, so GPU blending yields
///   `dst * (1 - shadow_level/100)` (the MMX shadow path; see
///   `Renderer::ensure_sprite_cached`).
/// - In blue-channel mode, the source blue component becomes the alpha map,
///   matching `SBDRAW_ALPHABLUEONLY`.
/// - In constant mode, all non-transparent pixels use the given opacity.
fn rgb565_to_argb8888(
    pixels: &[u16],
    width: u16,
    height: u16,
    transparent: u16,
    shadow_color: u16,
    shadow_level: u16,
    alpha_mode: TitbitAlphaMode,
) -> Vec<u8> {
    let shadow_alpha = (shadow_level.min(100) as u32 * 255 / 100) as u8;
    let n = width as usize * height as usize;
    let mut bytes = Vec::with_capacity(n * 4);

    for &px in pixels.iter().take(n) {
        let transparent_pixel = px == transparent
            || (matches!(alpha_mode, TitbitAlphaMode::ConstantPercent(_))
                && px == SPRITE_SHADOW_KEY_16);
        let (b, g, r, a) = if transparent_pixel {
            (0, 0, 0, 0)
        } else if alpha_mode == TitbitAlphaMode::SolidWithShadow && px == shadow_color {
            // Black tint so the blend collapses to pure multiply-darken.
            (0, 0, 0, shadow_alpha)
        } else {
            let alpha = match alpha_mode {
                TitbitAlphaMode::SolidWithShadow => 255,
                TitbitAlphaMode::BlueChannel => alpha_from_rgb565_blue(px),
                TitbitAlphaMode::ConstantPercent(percent) => {
                    (percent.min(100) as u32 * 255 / 100) as u8
                }
            };
            (
                ((px << 3) & 0xF8) as u8,
                ((px >> 3) & 0xFC) as u8,
                ((px >> 8) & 0xF8) as u8,
                alpha,
            )
        };
        // ARGB8888 in memory (little-endian on x86): [B, G, R, A].
        bytes.extend_from_slice(&[b, g, r, a]);
    }

    bytes
}

fn alpha_from_rgb565_blue(px: u16) -> u8 {
    ((px & 0x001F) << 3) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_anchor_matches_original_floor_after_half_extent() {
        assert_eq!(floor_centered(100.0, 20), 90);
        assert_eq!(floor_centered(100.0, 21), 89);
        assert_eq!(floor_centered(100.75, 21), 90);
        assert_eq!(floor_bottom(100.75, 21), 79);
    }

    #[test]
    fn hidden_titbit_uses_character_phase_as_portrait_frame() {
        assert_eq!(
            effective_titbit_frame(TitbitKind::Hidden, SpriteRow::Hidden as u16, 0, 0),
            0
        );
        assert_eq!(
            effective_titbit_frame(TitbitKind::Hidden, SpriteRow::Hidden as u16, 5, 0),
            5
        );
        assert_eq!(
            effective_titbit_frame(TitbitKind::Hidden, SpriteRow::Hidden as u16, 8, 0),
            8
        );
    }

    #[test]
    fn blue_channel_alpha_mode_matches_titbit_blit_flags() {
        let px = 0x001F;
        let bytes = rgb565_to_argb8888(
            &[px],
            1,
            1,
            TRANSPARENT_COLOR_KEY_16,
            DEFAULT_SHADOW_COLOR,
            SHADOW_LEVEL,
            TitbitAlphaMode::BlueChannel,
        );
        assert_eq!(bytes[3], 248);
    }

    #[test]
    fn constant_alpha_mode_matches_ghost_percent_and_wipes_shadow_key() {
        let bytes = rgb565_to_argb8888(
            &[0xFFFF, SPRITE_SHADOW_KEY_16],
            2,
            1,
            TRANSPARENT_COLOR_KEY_16,
            DEFAULT_SHADOW_COLOR,
            SHADOW_LEVEL,
            TitbitAlphaMode::ConstantPercent(70),
        );
        assert_eq!(bytes[3], 178);
        assert_eq!(bytes[7], 0);
    }
}

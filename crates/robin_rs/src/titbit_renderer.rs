//! GPU renderer for titbit sprites (floating indicators).
//!
//! Loads all sprite rows from `Data/Interface/DEFAULT.RES` at startup,
//! converts each frame to an RGBA8888 GPU texture with shadow alpha
//! pre-baked, then in the GPU phase iterates `engine.titbit_manager()
//! .titbits()` and queues each one as a textured GPU draw.
//!
//! This module owns the uploaded frame textures and row metadata; draws borrow
//! their views into the renderer's frame queue.
//!
//! The data side (`TitbitManager`, `TitbitInfo`, lifecycle) lives in
//! `robin_engine::titbit`.

use crate::gfx_types::BlendMode;
use crate::gfx_types::Rect;
use crate::host::HostDraw;
use crate::host::HostTitbitPreview;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::engine as engine_api;
use robin_engine::engine::PresentationView;

use crate::renderer::TRANSPARENT_COLOR_KEY_16;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::profiles::Action;
use robin_engine::titbit::{SpriteRow, TitbitKind};

const NUM_ROWS: usize = SpriteRow::NumberOfRows as usize;

pub use robin_assets::interface_metadata::titbit_sprite_row_resources;

/// Default day-ambience night-shadow color (RGB (45, 45, 35) packed
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
    /// retain the GPU textures in this renderer.
    ///
    /// `shadow_color` is the current ambience's night color
    /// (`engine.weather().night_color`).  Pass `0` to use the day default.
    pub fn load(
        &mut self,
        resource_manager: &mut ResourceManager,
        renderer: &crate::renderer::Renderer,
        shadow_color: u16,
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
                // The original game draws these live effects with blue-only alpha.
                SpriteRow::Water | SpriteRow::Plouf => TitbitAlphaMode::BlueChannel,
                // Jump-helper ghost uses constant alpha 70.
                SpriteRow::Ghost => TitbitAlphaMode::ConstantPercent(70),
                _ => TitbitAlphaMode::SolidWithShadow,
            };
            let frames = load_row(
                resource_manager,
                renderer,
                resource_id,
                shadow_color,
                wipe_shadow,
                shadow_level,
                alpha_mode,
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
        host: &HostDraw<'_>,
        engine: &PresentationView<'_>,
        renderer: &mut crate::renderer::Renderer,
        next_sim_display_order: f32,
        display_order_max: f32,
    ) -> bool {
        let Some(preview) = host.frontend.host_titbit_preview() else {
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
        host: &HostDraw<'_>,
        engine: &PresentationView<'_>,
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
                renderer.enqueue_external_texture(
                    view,
                    world_titbit_rect(
                        host.viewport(),
                        floor_centered(map_pt.x, w),
                        floor_bottom(map_pt.y, h),
                        w,
                        h,
                    ),
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
    /// - Stars/emoticons/speak/apple_smell → stars-effect point + kind-
    ///   specific vertical offset (set in `refresh_titbit_positions`).
    /// - Lock → feet point, no vertical offset.
    /// - Hidden → entity position + posture Z, centered on row max dims.
    /// - QuickAction → entity position, sprite anchored 50px above.
    /// - DangerPoint/QuickAction — only rendered when entity is selected.
    /// - Ghost — frame index offset by +7.
    pub(crate) fn render_up_to(
        &mut self,
        host: &HostDraw<'_>,
        engine: &PresentationView<'_>,
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
            if !titbit_visible(host, engine, assets, titbit, blink_off) {
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

            let placement = titbit_placement(titbit, (w, h), (ox, oy), (row_mw, row_mh));
            renderer.enqueue_external_texture(
                view,
                world_titbit_rect(
                    host.viewport(),
                    placement.x,
                    placement.y,
                    placement.width,
                    placement.height,
                ),
                [0.0, 0.0, 1.0, 1.0],
                placement.tint,
                if placement.additive {
                    BlendMode::Add
                } else {
                    BlendMode::Blend
                },
            );
        }
        self.render_host_preview_if_due(host, engine, renderer, f32::INFINITY, display_order_max);
    }
}

fn titbit_visible(
    host: &HostDraw<'_>,
    engine: &PresentationView<'_>,
    assets: &engine_api::LevelAssets,
    titbit: &robin_engine::titbit::TitbitInfo,
    blink_off: bool,
) -> bool {
    // Blinking titbits hide during the "off" portion of the cycle.
    if titbit.blinking && blink_off {
        return false;
    }

    // Fog owns whether supplier-attached world feedback may be
    // presented. Filtering before the Counter split keeps sprite and
    // text titbits from leaking actors through explored cells.
    if let Some(supplier) = titbit.element_supplier {
        let Some(entity_id) = engine.entity_id_for_index(supplier.0) else {
            return false;
        };
        if !engine.fog_entity_visible(entity_id) {
            return false;
        }
    }

    // Counter titbits render their phase (damage number) as
    // text via `hud_text::render_counter_titbits`, not through
    // the sprite path. Skip here.
    if titbit.kind == TitbitKind::Counter {
        return false;
    }

    // ── Visibility guards (per-kind checks) ──

    // QuickAction: only show when the managing PC is selected
    // (or the titbit is blinking).
    if matches!(
        titbit.kind,
        TitbitKind::QuickAction | TitbitKind::QuickActionRun
    ) && !titbit.blinking
    {
        let Some(mgr) = titbit.element_manager.map(|manager| manager.0) else {
            return false;
        };
        if !engine
            .hero_selection(host.local_seat)
            .iter()
            .any(|&id| id.index() == mgr)
        {
            return false;
        }
    }

    // DangerPoint: only show when the managing PC is selected.
    if titbit.kind == TitbitKind::DangerPoint {
        let Some(mgr) = titbit.element_manager.map(|manager| manager.0) else {
            return false;
        };
        if !engine
            .hero_selection(host.local_seat)
            .iter()
            .any(|&id| id.index() == mgr)
        {
            return false;
        }
    }

    // Emoticon: skip if entity is blipped or hidden in a
    // building (i.e. the entity must be active and outside a
    // building, and not blipped).  The `draw_hidden` debug
    // toggle (`MSG_SWITCH_MASKED_DISPLAY`) overrides the
    // blipped / in-building skip so the inspector can see AI
    // reactions through walls — active+alive guard stays.
    if titbit.kind == TitbitKind::Emoticon
        && let Some(supplier) = titbit.element_supplier
        && let Some(entity_id) = engine.entity_id_for_index(supplier.0)
        && let Some(entity) = engine.get_entity(entity_id)
    {
        let elem = entity.element_data();
        if !elem.active {
            return false;
        }
        if !host.frontend.input.feedback.draw_hidden && (elem.blipped || elem.hidden_in_building) {
            return false;
        }
    }

    // WeakStunned/Speak: skip if entity is out of order or
    // inactive.
    if matches!(titbit.kind, TitbitKind::WeakStunned | TitbitKind::Speak)
        && let Some(supplier) = titbit.element_supplier
        && let Some(entity_id) = engine.entity_id_for_index(supplier.0)
        && let Some(entity) = engine.get_entity(entity_id)
        && !entity.is_active()
    {
        return false;
    }

    // Speak: also skip when the supplier is blipped or hidden
    // inside a building — the speak titbit only renders when
    // the entity is neither blipped nor inside a building.
    if titbit.kind == TitbitKind::Speak
        && let Some(supplier) = titbit.element_supplier
        && let Some(entity_id) = engine.entity_id_for_index(supplier.0)
        && let Some(entity) = engine.get_entity(entity_id)
    {
        let elem = entity.element_data();
        if elem.blipped || elem.hidden_in_building {
            return false;
        }
    }

    // WorkIcon: skip if entity is inactive.
    if titbit.kind == TitbitKind::WorkIcon
        && let Some(supplier) = titbit.element_supplier
        && let Some(entity_id) = engine.entity_id_for_index(supplier.0)
        && let Some(entity) = engine.get_entity(entity_id)
        && !entity.is_active()
    {
        return false;
    }

    // WorkIcon: also skip while the men-to-blazon conversion
    // screen is up — the per-PC work icon is suppressed for
    // the duration of the conversion UI.
    if titbit.kind == TitbitKind::WorkIcon && engine.is_men_to_blazon_conversion_mode() {
        return false;
    }

    // WorkIcon BowTraining: skip when the PC's profile lacks
    // `Action::Bow` — hides the bow-training icon for PCs
    // who can't currently use a bow even if their work icon
    // is set to BowTraining.
    if titbit.kind == TitbitKind::WorkIcon
        && titbit.sprite_row == SpriteRow::WorkIconBowTraining as u16
        && let Some(supplier) = titbit.element_supplier
        && let Some(entity_id) = engine.entity_id_for_index(supplier.0)
        && let Some(entity) = engine.get_entity(entity_id)
    {
        let has_bow = entity.pc_data().is_some_and(|pc| {
            assets
                .profile_manager
                .get_character(pc.profile_index)
                .is_some_and(|p| p.has_action(Action::Bow))
        });
        if !has_bow {
            return false;
        }
    }

    // Ghost: blink at half rate.  Only renders when the
    // alternating phase `(frame_counter / GHOST_BLINK) & 0x1`
    // is set.
    if titbit.kind == TitbitKind::Ghost
        && (engine.frame_counter() / robin_engine::titbit::GHOST_BLINK) & 0x1 == 0
    {
        return false;
    }

    // UnconsciousStar: per-frame animation gate.  The stars
    // sprite is restricted to the idle KO animations
    // (`BeingUnconscious{,Bow,Sword}`); during the falling /
    // transition frames between knockout and the idle-unconscious
    // animation the titbit exists but is not drawn.
    if titbit.kind == TitbitKind::UnconsciousStar
        && let Some(supplier) = titbit.element_supplier
        && !engine
            .entity_id_for_index(supplier.0)
            .is_some_and(|id| engine.can_have_unconscious_stars(id))
    {
        return false;
    }

    true
}

/// Map-space placement is independent of GPU ownership and viewport zoom.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
struct TitbitPlacement {
    x: i32,
    y: i32,
    width: u16,
    height: u16,
    additive: bool,
    tint: [f32; 4],
}

fn titbit_placement(
    titbit: &robin_engine::titbit::TitbitInfo,
    (w, h): (u16, u16),
    (ox, oy): (i16, i16),
    (row_mw, row_mh): (Option<u16>, Option<u16>),
) -> TitbitPlacement {
    // GunImpact uses an alpha-red-only blend that produces an
    // additive red muzzle flash.  Approximate with additive
    // blend + red tint; the underlying sprite is already red-heavy
    // so additive blending lands close.
    let gun_impact = titbit.kind == TitbitKind::GunImpact;

    // Per-draw blend + tint derived from the kind.
    let tint = if gun_impact {
        [1.0, 0.0, 0.0, 1.0]
    } else {
        // For star titbits the keyed alpha applies to the
        // shadow/keyed colour, not to every coloured pixel —
        // `load_row` already bakes that keyed shadow alpha
        // into the texture.
        [1.0, 1.0, 1.0, 1.0]
    };

    // Convert 3D world position to 2D map: (x, y - z).
    let map_pt = engine_coordinates::MapPoint::from_world_xyz(
        titbit.position.x,
        titbit.position.y,
        titbit.position.z,
    );

    // ── QuickAction: special positioning ──
    // QA icons sit at positionMap - (0.5*spriteWidth,
    // spriteHeight + 50), i.e., 50px above the entity, anchored
    // at bottom-center. QuickActionRun gets +3 X offset.
    if matches!(
        titbit.kind,
        TitbitKind::QuickAction | TitbitKind::QuickActionRun
    ) {
        let supplier_attached = titbit.element_supplier.is_some();
        let (mut dst_x, dst_y) = quick_action_map_origin(
            map_pt.x,
            map_pt.y,
            w,
            h,
            ox as i32,
            oy as i32,
            supplier_attached,
        );
        if titbit.kind == TitbitKind::QuickActionRun {
            dst_x += 3;
        }
        return TitbitPlacement {
            x: dst_x,
            y: dst_y,
            width: w,
            height: h,
            additive: gun_impact,
            tint,
        };
    }

    let (dst_x, dst_y) = match titbit.kind {
        // These kinds explicitly set their sprite position in the original game;
        // Blit-box generation then adds the cropped-frame offset.
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
            let x = floor_centered(map_pt.x, center_w) + ox as i32;
            let y = if center_vertical {
                floor_centered(map_pt.y - vertical_offset as f32, center_h) + oy as i32
            } else {
                floor_anchor(map_pt.y, vertical_offset) + oy as i32
            };
            (x, y)
        }
        TitbitKind::UnconsciousStar if titbit.element_supplier.is_some() => (
            floor_centered(map_pt.x, row_mw.unwrap_or(w)) + ox as i32,
            floor_centered(map_pt.y - 10.0, row_mh.unwrap_or(h)) + oy as i32,
        ),
        TitbitKind::WeakStunned | TitbitKind::AppleSmell | TitbitKind::Speak => (
            floor_centered(map_pt.x, row_mw.unwrap_or(w)) + ox as i32,
            floor_centered(map_pt.y - 10.0, row_mh.unwrap_or(h)) + oy as i32,
        ),
        TitbitKind::Lock | TitbitKind::Hidden => (
            floor_centered(map_pt.x, row_mw.unwrap_or(w)) + ox as i32,
            floor_centered(map_pt.y, row_mh.unwrap_or(h)) + oy as i32,
        ),
        TitbitKind::DangerPoint => (
            floor_centered(map_pt.x, w) + ox as i32,
            floor_centered(map_pt.y, h) + oy as i32,
        ),

        // These kinds rely on the original game's final sprite-centering step
        // pass. That center includes the current frame offset, so the
        // offset cancels out when blit-box generation adds it.
        TitbitKind::Water | TitbitKind::Plouf | TitbitKind::WorkIcon => {
            (floor_centered(map_pt.x, w), floor_centered(map_pt.y, h))
        }
        TitbitKind::GunImpact
        | TitbitKind::Smoke
        | TitbitKind::Dust
        | TitbitKind::Ghost
        | TitbitKind::UnconsciousStar => (floor_centered(map_pt.x, w), floor_bottom(map_pt.y, h)),
        _ => (floor_centered(map_pt.x, w), floor_centered(map_pt.y, h)),
    };
    TitbitPlacement {
        x: dst_x,
        y: dst_y,
        width: w,
        height: h,
        additive: gun_impact,
        tint,
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
    } else if matches!(
        kind,
        TitbitKind::Hidden | TitbitKind::QuickAction | TitbitKind::QuickActionRun
    ) {
        phase
    } else if kind == TitbitKind::Emoticon && row == SpriteRow::EmoticonGrowingQMark as u16 {
        phase * 8 + sprite_frame
    } else {
        sprite_frame
    }
}

/// RHtitbit::Draw positions and centers sprites in map units before
/// GenerateBlitBox applies the viewport zoom, including offsets above actors.
fn world_titbit_rect(
    viewport: &crate::host::ViewportState,
    x: i32,
    y: i32,
    w: u16,
    h: u16,
) -> Rect {
    crate::game_render::zoomed_sprite_rect(
        ((x as f32 - viewport.view_position.x) * viewport.zoom_factor) as i32,
        ((y as f32 - viewport.view_position.y) * viewport.zoom_factor) as i32,
        w,
        h,
        viewport.zoom_factor,
    )
}

fn floor_centered(anchor: f32, extent: u16) -> i32 {
    (anchor - 0.5 * extent as f32).floor() as i32
}

fn floor_anchor(anchor: f32, offset: i32) -> i32 {
    (anchor - offset as f32).floor() as i32
}

fn quick_action_map_origin(
    screen_x: f32,
    screen_y: f32,
    width: u16,
    height: u16,
    offset_x: i32,
    offset_y: i32,
    supplier_attached: bool,
) -> (i32, i32) {
    let x = floor_centered(screen_x, width) + offset_x;
    let y = if supplier_attached {
        floor_anchor(screen_y, height as i32 + 50) + offset_y
    } else {
        floor_centered(screen_y, height) + offset_y
    };
    (x, y)
}

fn floor_bottom(anchor: f32, extent: u16) -> i32 {
    (anchor - extent as f32).floor() as i32
}

/// Load every sub-picture of a resource into a vector of GPU textures.
///
/// Each sub-picture is converted RGB565 → RGBA8888 with the shadow key
/// (`0x001F`) replaced by the day-ambience shadow color, and that shadow
/// color baked to a semi-transparent alpha so the GPU blend produces
/// the same dim grey shadow effect as the original software blit path.
fn load_row(
    resource_manager: &mut ResourceManager,
    renderer: &crate::renderer::Renderer,
    resource_id: i32,
    shadow_color: u16,
    wipe_shadow: bool,
    shadow_level: u16,
    alpha_mode: TitbitAlphaMode,
) -> Vec<TitbitFrame> {
    let pictures = match resource_manager.get_pictures(resource_id) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("TitbitRenderer: failed to load resource {resource_id}: {e}");
            return Vec::new();
        }
    };

    let mut frames = Vec::with_capacity(pictures.len());
    // Queue uploads copy their source bytes, so scratch storage can serve every frame.
    let mut pixels = Vec::new();
    let mut rgba = Vec::new();
    for slot in pictures {
        let Some(pic) = slot else { continue };
        if pic.width == 0 || pic.height == 0 {
            continue;
        }

        let Some((crop_x, crop_y, crop_w, crop_h)) = pic.opaque_bounds_16() else {
            continue;
        };
        pixels.clear();
        pixels.reserve(crop_w as usize * crop_h as usize);
        for y in 0..crop_h as usize {
            let src = (crop_y as usize + y) * pic.width as usize + crop_x as usize;
            pixels.extend(
                pic.data[src * 2..(src + crop_w as usize) * 2]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pixel| u16::from_le_bytes(*pixel)),
            );
        }

        // Replace the magic blue shadow key (0x001F) with the current
        // ambience shadow color for ordinary sprite draws. Blue-channel
        // Alpha effects must preserve the raw blue component because the original game
        // uses it as the alpha map.
        if alpha_mode == TitbitAlphaMode::SolidWithShadow {
            crate::markers::apply_arno_law(&mut pixels, shadow_color);
        }

        // Convert to RGBA8888 with shadow alpha pre-baked.
        // Preserve the existing wipe_shadow override of 100 percent.
        // TODO: Verify this policy against original rendering: despite the
        // name, solid-mode shadow pixels become opaque black at 100 percent.
        rgb565_to_rgba8888(
            &pixels,
            &mut rgba,
            TRANSPARENT_COLOR_KEY_16,
            shadow_color,
            if wipe_shadow { 100 } else { shadow_level },
            alpha_mode,
        );

        let (tex, view) = renderer.create_static_rgba_texture(
            &rgba,
            crop_w as u32,
            crop_h as u32,
            &format!("titbit res={resource_id}"),
        );

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

/// Replace the output buffer with RGBA8888 bytes, reusing its allocation.
/// Shadow alpha is pre-baked.
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
fn rgb565_to_rgba8888(
    pixels: &[u16],
    bytes: &mut Vec<u8>,
    transparent: u16,
    shadow_color: u16,
    shadow_level: u16,
    alpha_mode: TitbitAlphaMode,
) {
    let shadow_alpha = (shadow_level.min(100) as u32 * 255 / 100) as u8;
    let constant_mode = matches!(alpha_mode, TitbitAlphaMode::ConstantPercent(_));
    let fixed_alpha = match alpha_mode {
        TitbitAlphaMode::ConstantPercent(percent) => (percent.min(100) as u32 * 255 / 100) as u8,
        TitbitAlphaMode::SolidWithShadow | TitbitAlphaMode::BlueChannel => 255,
    };
    bytes.clear();
    bytes.reserve(pixels.len() * 4);

    for &px in pixels {
        let transparent_pixel = px == transparent || (constant_mode && px == SPRITE_SHADOW_KEY_16);
        let (r, g, b, a) = if transparent_pixel {
            (0, 0, 0, 0)
        } else if alpha_mode == TitbitAlphaMode::SolidWithShadow && px == shadow_color {
            // Black tint so the blend collapses to pure multiply-darken.
            (0, 0, 0, shadow_alpha)
        } else {
            let alpha = if alpha_mode == TitbitAlphaMode::BlueChannel {
                alpha_from_rgb565_blue(px)
            } else {
                fixed_alpha
            };
            let (r, g, b) = robin_util::color::rgb565_to_rgb8(px);
            (r, g, b, alpha)
        };
        bytes.extend_from_slice(&[r, g, b, a]);
    }
}

fn alpha_from_rgb565_blue(px: u16) -> u8 {
    ((px & 0x001F) << 3) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_preserves_kind_anchors_offsets_and_additive_tint() {
        let mut titbit = robin_engine::titbit::TitbitInfo {
            kind: TitbitKind::Water,
            phase: 0,
            sprite_row: 0,
            sprite_frame: 0,
            frame_count: 1,
            element_supplier: None,
            element_manager: None,
            layer: 0,
            position: engine_coordinates::WorldPoint3D::new(100.0, 200.0, 20.0),
            display_order: 0.0,
            blinking: false,
            id: robin_engine::titbit::TitbitId::new(0).unwrap(),
        };
        for (kind, expected) in [
            (TitbitKind::Water, (90, 175)),
            (TitbitKind::GunImpact, (90, 170)),
            (TitbitKind::Lock, (83, 169)),
            (TitbitKind::Speak, (83, 159)),
            (TitbitKind::DangerPoint, (93, 179)),
            (TitbitKind::QuickAction, (93, 179)),
            (TitbitKind::QuickActionRun, (96, 179)),
        ] {
            titbit.kind = kind;
            let placement = titbit_placement(&titbit, (20, 10), (3, 4), (Some(40), Some(30)));
            assert_eq!((placement.x, placement.y), expected, "{kind:?}");
            assert_eq!((placement.width, placement.height), (20, 10));
            assert_eq!(placement.additive, kind == TitbitKind::GunImpact);
            assert_eq!(
                placement.tint,
                if placement.additive {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [1.0; 4]
                }
            );
        }
        titbit.kind = TitbitKind::QuickAction;
        titbit.element_supplier = Some(robin_engine::titbit::ElementHandle(0));
        let placement = titbit_placement(&titbit, (20, 10), (3, 4), (Some(40), Some(30)));
        assert_eq!((placement.x, placement.y), (93, 124));
    }

    #[test]
    fn world_indicator_offsets_and_sizes_follow_zoom() {
        let mut viewport = crate::host::ViewportState::default();
        viewport.view_position = engine_coordinates::MapPoint::new(100.0, 200.0);
        viewport.zoom_factor = 0.5;
        // A 20x20 supplier icon at (200,300) is positioned 50 map units
        // above its supplier before the entire destination is zoomed.
        let (x, y) = quick_action_map_origin(200.0, 300.0, 20, 20, 0, 0, true);
        let rect = world_titbit_rect(&viewport, x, y, 20, 20);
        assert_eq!((rect.x, rect.y, rect.w, rect.h), (45, 15, 10, 10));
    }

    #[test]
    fn centered_anchor_matches_original_floor_after_half_extent() {
        assert_eq!(floor_centered(100.0, 20), 90);
        assert_eq!(floor_centered(100.0, 21), 89);
        assert_eq!(floor_centered(100.75, 21), 90);
        assert_eq!(floor_bottom(100.75, 21), 79);
    }

    #[test]
    fn fixed_qa_crosshair_is_centered_but_supplier_icon_floats_above_target() {
        assert_eq!(
            quick_action_map_origin(100.0, 100.0, 20, 20, 0, 0, false),
            (90, 90)
        );
        assert_eq!(
            quick_action_map_origin(100.0, 100.0, 20, 20, 0, 0, true),
            (90, 30)
        );
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
    fn quick_action_uses_authored_phase_as_icon_frame() {
        assert_eq!(
            effective_titbit_frame(
                TitbitKind::QuickAction,
                SpriteRow::QuickActionTitbits as u16,
                robin_engine::titbit::QuickAction::BowOk as u16,
                0,
            ),
            robin_engine::titbit::QuickAction::BowOk as u16
        );
        assert_eq!(
            effective_titbit_frame(
                TitbitKind::QuickActionRun,
                SpriteRow::QuickActionTitbits as u16,
                robin_engine::titbit::QuickAction::Take as u16,
                0,
            ),
            robin_engine::titbit::QuickAction::Take as u16
        );
    }

    #[test]
    fn rgba_conversion_preserves_every_rgb565_color_and_alpha_mode() {
        let pixels: Vec<u16> = (0..=u16::MAX).collect();
        for mode in [
            TitbitAlphaMode::SolidWithShadow,
            TitbitAlphaMode::BlueChannel,
            TitbitAlphaMode::ConstantPercent(0),
            TitbitAlphaMode::ConstantPercent(70),
            TitbitAlphaMode::ConstantPercent(100),
            TitbitAlphaMode::ConstantPercent(u16::MAX),
        ] {
            let mut bytes = Vec::new();
            rgb565_to_rgba8888(
                &pixels,
                &mut bytes,
                TRANSPARENT_COLOR_KEY_16,
                DEFAULT_SHADOW_COLOR,
                50,
                mode,
            );
            assert_eq!(bytes.len(), pixels.len() * 4);
            for (&pixel, actual) in pixels.iter().zip(bytes.as_chunks::<4>().0) {
                let transparent = pixel == TRANSPARENT_COLOR_KEY_16
                    || (matches!(mode, TitbitAlphaMode::ConstantPercent(_))
                        && pixel == SPRITE_SHADOW_KEY_16);
                let expected = if transparent {
                    [0, 0, 0, 0]
                } else if mode == TitbitAlphaMode::SolidWithShadow && pixel == DEFAULT_SHADOW_COLOR
                {
                    [0, 0, 0, 127]
                } else {
                    let alpha = match mode {
                        TitbitAlphaMode::SolidWithShadow => 255,
                        TitbitAlphaMode::BlueChannel => ((pixel & 31) * 8) as u8,
                        TitbitAlphaMode::ConstantPercent(percent) => {
                            if percent >= 100 {
                                255
                            } else {
                                (u32::from(percent) * 255 / 100) as u8
                            }
                        }
                    };
                    [
                        ((pixel >> 11) * 8) as u8,
                        (((pixel >> 5) & 63) * 4) as u8,
                        ((pixel & 31) * 8) as u8,
                        alpha,
                    ]
                };
                assert_eq!(*actual, expected, "pixel {pixel:#06x}, mode {mode:?}");
            }
        }
    }

    #[test]
    fn rgba_conversion_reuses_storage_without_leaking_previous_frame_pixels() {
        let mut bytes = Vec::with_capacity(64);
        let original_ptr = bytes.as_ptr();
        let original_capacity = bytes.capacity();
        for pixels in [&[0xFFFF, 0xF800, 0x07E0][..], &[0x001F][..], &[][..]] {
            rgb565_to_rgba8888(
                pixels,
                &mut bytes,
                TRANSPARENT_COLOR_KEY_16,
                DEFAULT_SHADOW_COLOR,
                SHADOW_LEVEL,
                TitbitAlphaMode::BlueChannel,
            );
            let expected: Vec<u8> = pixels
                .iter()
                .flat_map(|&pixel| {
                    [
                        ((pixel >> 11) * 8) as u8,
                        (((pixel >> 5) & 63) * 4) as u8,
                        ((pixel & 31) * 8) as u8,
                        ((pixel & 31) * 8) as u8,
                    ]
                })
                .collect();
            assert_eq!(bytes, expected);
            assert_eq!(bytes.as_ptr(), original_ptr);
            assert_eq!(bytes.capacity(), original_capacity);
        }
    }

    #[test]
    fn blue_channel_alpha_mode_matches_titbit_blit_flags() {
        let px = 0x001F;
        let mut bytes = Vec::new();
        rgb565_to_rgba8888(
            &[px],
            &mut bytes,
            TRANSPARENT_COLOR_KEY_16,
            DEFAULT_SHADOW_COLOR,
            SHADOW_LEVEL,
            TitbitAlphaMode::BlueChannel,
        );
        assert_eq!(bytes[3], 248);
    }

    #[test]
    fn constant_alpha_mode_matches_ghost_percent_and_wipes_shadow_key() {
        let mut bytes = Vec::new();
        rgb565_to_rgba8888(
            &[0xFFFF, SPRITE_SHADOW_KEY_16],
            &mut bytes,
            TRANSPARENT_COLOR_KEY_16,
            DEFAULT_SHADOW_COLOR,
            SHADOW_LEVEL,
            TitbitAlphaMode::ConstantPercent(70),
        );
        assert_eq!(bytes[3], 178);
        assert_eq!(bytes[7], 0);
    }
}

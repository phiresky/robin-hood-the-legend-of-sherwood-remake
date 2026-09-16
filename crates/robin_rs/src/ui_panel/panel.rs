use super::*;
use crate::titbit_renderer::TitbitRenderer;

/// Render the health gauge (two-parchment composite) at the given position.
///
/// Splits the top scroll into a "live" left portion (normal parchment)
/// and a "dead" right portion (darkened parchment) at `ratio × width`.
pub(super) fn render_health_gauge(
    renderer: &mut Renderer,
    portraits: &PortraitCache,
    entity: Option<&Entity>,
    x: u16,
    top: u16,
) {
    let Some(normal_sid) = portraits.top_scroll_surface else {
        return;
    };
    let (w, h) = portrait_surface_dimensions(renderer, normal_sid);

    let ratio = match entity {
        Some(Entity::Pc(pc)) => (pc.pc.life_points.max(0) as f32 / 100.0).clamp(0.0, 1.0),
        _ => 1.0,
    };
    let split_x = (w as f32 * ratio) as u16;

    // Dead portion (right side — darkened parchment)
    if split_x < w
        && let Some(alt_sid) = portraits.top_scroll_alt_surface
    {
        let src = bbox(split_x, 0, w, h);
        let dst = bbox(x + split_x, top, x + w, top + h);
        blit_to_screen_widget(
            renderer,
            alt_sid,
            Some(&src),
            Some(&dst),
            BLIT_SOURCE_TRANSPARENT,
        );
    }
    // Live portion (left side — normal parchment)
    if split_x > 0 {
        let src = bbox(0, 0, split_x, h);
        let dst = bbox(x, top, x + split_x, top + h);
        blit_to_screen_widget(
            renderer,
            normal_sid,
            Some(&src),
            Some(&dst),
            BLIT_SOURCE_TRANSPARENT,
        );
    }
}

/// Blit a surface centered within the vertical region between two scrolls.
///
/// Centers the indicator widget within the reference box spanning from
/// upper scroll top to lower scroll bottom.
pub(super) fn blit_centered_between_scrolls(
    renderer: &mut Renderer,
    surface: Option<SurfaceHandle>,
    x: u16,
    ref_top: u16,
    ref_bot: u16,
) {
    let Some(sid) = surface else { return };
    let (iw, ih) = portrait_surface_dimensions(renderer, sid);
    let ref_h = ref_bot.saturating_sub(ref_top);
    let ix = x + (ELEMENT_WIDTH.saturating_sub(iw)) / 2;
    let iy = ref_top + (ref_h.saturating_sub(ih)) / 2;
    let dst = bbox(ix, iy, ix + iw, iy + ih);
    blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
}

/// Borrowed frame state shared by every bottom-panel draw phase.
pub struct HudDrawCtx<'a> {
    pub frontend: &'a HostFrontend,
    pub engine: &'a PresentationView<'a>,
    pub local_seat: PlayerId,
    pub profiles: &'a engine_profiles::ProfileManager,
    pub renderer: &'a mut Renderer,
    pub portraits: &'a PortraitCache,
    /// Pointer position in screen pixels (drives button hover art).
    pub mouse_x: f32,
    pub mouse_y: f32,
}

/// PC portrait slot state resolved once before its draw phases.
struct PcSlot<'e> {
    slot: u16,
    x: u16,
    pc_id: EntityId,
    entity: Option<&'e Entity>,
    is_selected: bool,
    is_dead: bool,
    is_coma: bool,
    is_sword_fighting: bool,
    is_guarded: bool,
    has_trumpet: bool,
}

impl<'e> PcSlot<'e> {
    fn resolve(
        engine: &'e PresentationView<'_>,
        local_seat: PlayerId,
        slot: u16,
        x: u16,
        pc_id: EntityId,
    ) -> Self {
        let entity = engine.get_entity(pc_id);
        let is_selected = engine.hero_selection(local_seat).contains(&pc_id);

        // ── Extract PC-specific state for overlay rendering ──
        let (is_dead, is_coma, is_sword_fighting, is_guarded, has_trumpet) = match entity {
            Some(pc_entity @ Entity::Pc(pc)) => (
                pc.pc.life_points <= 0,
                is_pc_in_coma(engine, pc_entity),
                pc.actor.action_state.is_sword(),
                pc.pc.guard.is_some(),
                pc.pc.trumpet_enabled,
            ),
            _ => (false, false, false, false, false),
        };
        Self {
            slot,
            x,
            pc_id,
            entity,
            is_selected,
            is_dead,
            is_coma,
            is_sword_fighting,
            is_guarded,
            has_trumpet,
        }
    }

    /// Burned = dead OR in coma (the burn path covers both).
    fn is_burned(&self) -> bool {
        self.is_dead || self.is_coma
    }
}

impl HudDrawCtx<'_> {
    /// Draw the bottom UI panel to the screen surface.
    ///
    /// This renders, in order:
    /// 1. Ornamental border frame pieces (corners + middle strip) around the
    ///    bottom panel area.
    /// 2. Portrait slots for each displayed PC — top/bottom scrolls, visage,
    ///    action buttons, fighting/guard/trumpet/amulet overlays, QA strip,
    ///    and burned-state variants for dead/coma PCs.
    ///
    /// The minimap and its frame are drawn separately by `render_minimap`
    /// (the `RHMAP_CORNER` sprite covers the slot in non-Sherwood missions).
    ///
    /// Should be called after entity rendering and before `renderer.flip()`.
    pub fn draw_panel(&mut self, titbit_renderer: Option<&mut TitbitRenderer>, shift_held: bool) {
        self.portraits
            .validate_renderer_identity(self.renderer)
            .expect("HUD requires its originating renderer");
        let sw = self.renderer.screen_width();
        let sh = self.renderer.screen_height();

        if sw == 0 || sh == 0 {
            return;
        }

        self.draw_border_frame(sw, sh);

        // ── Portrait slots (one per PC in the mission) ──
        // Hidden-interface PCs don't consume a slot, so filter on
        // `pc.interface_hidden` via `engine.displayed_pc_ids()` rather than
        // walking `pc_ids` directly.
        let engine = self.engine;
        let (portrait_items, paged) = portrait_bar_items(engine, self.local_seat, sw);
        let num_portraits = portrait_items.len() as u16;
        let slot_count = portrait_slot_count(sw, portrait_items.len());
        let hovered_portrait = hit_test_portrait_detailed(
            engine,
            self.local_seat,
            self.portraits,
            sw,
            sh,
            self.mouse_x,
            self.mouse_y,
        );
        let hovered_action = hovered_portrait.and_then(|hit| match hit.area {
            PortraitHitArea::ActionButton(btn) => Some((hit.slot, btn)),
            _ => None,
        });
        let hovered_allied_action = hovered_portrait.and_then(|hit| match hit.area {
            PortraitHitArea::AlliedAction(btn) => Some((hit.slot, btn)),
            _ => None,
        });

        let mut titbit_renderer_opt = titbit_renderer;

        if paged {
            self.draw_page_arrows(sw, sh);
        }

        for slot in 0..num_portraits {
            let x = slot_left_x(sw, slot, slot_count);

            let item = &portrait_items[slot as usize];
            let PortraitTarget::Pc(pc_id) = item.target() else {
                let hovered = hovered_allied_action
                    .filter(|(hovered_slot, _)| *hovered_slot == slot as u8)
                    .map(|(_, button)| button);
                self.render_allied_portrait(item, x, sh, hovered);
                continue;
            };
            let pc = PcSlot::resolve(engine, self.local_seat, slot, x, pc_id);
            if pc.is_burned() {
                self.draw_burned_pc(&pc, sh);
            } else {
                self.draw_normal_pc(
                    &pc,
                    sh,
                    hovered_action,
                    &mut titbit_renderer_opt,
                    shift_held,
                );
            }
        }
    }

    /// Panel border frame (ornamental frame around the bottom panel).
    ///
    /// Rendered BEFORE portrait widgets, in absolute screen coordinates.
    /// Blit using source surface dimensions to avoid size mismatch issues.
    fn draw_border_frame(&mut self, sw: u16, sh: u16) {
        let renderer = &mut *self.renderer;
        let portraits = self.portraits;
        if let Some(sid) = portraits.border_top_left {
            let (w, h) = portrait_surface_dimensions(renderer, sid);
            let (w, h) = (w.min(sw), h.min(sh));
            let dst = bbox(0, 0, w, h);
            blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
        }
        if let Some(sid) = portraits.border_top_right {
            let (w, h) = portrait_surface_dimensions(renderer, sid);
            let (w, h) = (w.min(sw), h.min(sh));
            let dst = bbox(sw - w, 0, sw, h);
            blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
        }
        if let Some(sid) = portraits.border_bottom_left {
            let (w, h) = portrait_surface_dimensions(renderer, sid);
            let (w, h) = (w.min(sw), h.min(sh));
            let dst = bbox(0, sh - h, w, sh);
            blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
        }
        if let Some(sid) = portraits.border_bottom_right {
            let (w, h) = portrait_surface_dimensions(renderer, sid);
            let (w, h) = (w.min(sw), h.min(sh));
            let dst = bbox(sw - w, sh - h, sw, sh);
            blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
        }
        // Center border piece — only at 800+ width (disabled at 640). The
        // original supplied fixed 800/1024 strips; tile and crop the selected
        // strip between the corners so adaptive widths never expose a gap.
        if sw > 640
            && let Some(sid) = portraits.border_middle
        {
            let (w, h) = portrait_surface_dimensions(renderer, sid);
            let h = h.min(sh);
            let mut x = portraits
                .border_bottom_left
                .map_or(0, |id| portrait_surface_dimensions(renderer, id).0)
                .min(sw);
            let right = sw.saturating_sub(
                portraits
                    .border_bottom_right
                    .map_or(0, |id| portrait_surface_dimensions(renderer, id).0)
                    .min(sw),
            );
            while x < right && w > 0 {
                let tile_width = w.min(right - x);
                let src = bbox(0, 0, tile_width, h);
                let dst = bbox(x, sh.saturating_sub(h), x + tile_width, sh);
                blit_to_screen_widget(
                    renderer,
                    sid,
                    Some(&src),
                    Some(&dst),
                    BLIT_SOURCE_TRANSPARENT,
                );
                x += tile_width;
            }
        }
    }

    /// Left/right page scrolls shown when the portrait bar is paged.
    fn draw_page_arrows(&mut self, sw: u16, sh: u16) {
        for (surface, x) in [
            (self.portraits.portrait_page_left, 4),
            (self.portraits.portrait_page_right, sw.saturating_sub(28)),
        ] {
            if let Some(surface) = surface {
                let (w, h) = portrait_surface_dimensions(self.renderer, surface);
                let y = sh.saturating_sub(PORTRAIT_TOTAL_HEIGHT / 2 + h / 2);
                blit_to_screen_widget(
                    self.renderer,
                    surface,
                    None,
                    Some(&bbox(x, y, x + w, y + h)),
                    BLIT_SOURCE_TRANSPARENT,
                );
            }
        }
    }

    fn draw_bottom_scroll(&mut self, x: u16, sh: u16) {
        if let Some(sid) = self.portraits.bottom_scroll_surface {
            let (w, h) = portrait_surface_dimensions(self.renderer, sid);
            let top = sh - POSITION_BOTTOM_SCROLL;
            let dst = bbox(x, top, x + w, top + h);
            blit_to_screen_widget(
                self.renderer,
                sid,
                None,
                Some(&dst),
                BLIT_SOURCE_TRANSPARENT,
            );
        }
    }

    /// ── BURNED STATE ──
    /// Visage and action buttons hidden. Upper scroll repositioned
    /// directly above lower scroll.
    /// Coma PCs show health gauge + enabled scrolls;
    /// fully dead PCs hide scrolls entirely.
    fn draw_burned_pc(&mut self, pc: &PcSlot<'_>, sh: u16) {
        let x = pc.x;
        let burned_upper_top = sh - POSITION_BOTTOM_SCROLL - BOTTOM_SCROLL_HEIGHT;

        if pc.is_coma {
            // Coma: scrolls enabled, health gauge visible.
            render_health_gauge(
                self.renderer,
                self.portraits,
                pc.entity,
                x,
                burned_upper_top,
            );

            self.draw_bottom_scroll(x, sh);

            // Guard indicator (centered between scrolls).
            if pc.is_guarded {
                let guard_visible = if self.engine.mission_won() {
                    (self.engine.frame_counter() / 25).is_multiple_of(2)
                } else {
                    true
                };
                if guard_visible {
                    blit_centered_between_scrolls(
                        self.renderer,
                        self.portraits.guard_surface,
                        x,
                        burned_upper_top,
                        sh - BORDURE,
                    );
                }
            }
            // Amulet/clover indicator when NOT guarded.
            if !pc.is_guarded {
                blit_centered_between_scrolls(
                    self.renderer,
                    self.portraits.amulet_surface,
                    x,
                    burned_upper_top,
                    sh - BORDURE,
                );
            }
        }
        // Fully dead (not coma): scrolls disabled, nothing rendered.
        //
        // Trumpet indicator: the trumpet only appears on dead PCs (not
        // coma). `melee.rs:3208` sets `trumpet_enabled = true` when the
        // killed PC has a non-VIP replacement available. Drawn centered
        // in the same between-scrolls region the coma amulet/guard uses.
        if pc.has_trumpet {
            blit_centered_between_scrolls(
                self.renderer,
                self.portraits.trumpet_surface,
                x,
                burned_upper_top,
                sh - BORDURE,
            );
        }
    }

    /// ── NORMAL STATE ──
    fn draw_normal_pc(
        &mut self,
        pc: &PcSlot<'_>,
        sh: u16,
        hovered_action: Option<(u8, u8)>,
        titbit_renderer: &mut Option<&mut TitbitRenderer>,
        shift_held: bool,
    ) {
        let pos_top_scroll = if pc.is_selected {
            POSITION_TOP_SCROLL
        } else {
            CLOSE_POSITION_TOP_SCROLL
        };
        let pos_visage = if pc.is_selected {
            POSITION_VISAGE
        } else {
            CLOSE_POSITION_VISAGE
        };

        // Top scroll (health gauge)
        render_health_gauge(
            self.renderer,
            self.portraits,
            pc.entity,
            pc.x,
            sh - pos_top_scroll,
        );

        // Bottom scroll
        self.draw_bottom_scroll(pc.x, sh);

        let vis_top = sh - pos_visage;
        self.draw_pc_visage(pc, sh, vis_top);

        if pc.is_sword_fighting {
            self.draw_pc_fighting_overlay(pc, vis_top);
        }

        // Action buttons — only when selected/open.
        if pc.is_selected {
            self.draw_pc_action_buttons(pc, sh, hovered_action, shift_held);
        }

        self.draw_pc_quick_action_strip(pc, sh - pos_top_scroll, titbit_renderer);

        // The trumpet widget is only enabled on death, so it never
        // appears on a living PC — nothing to draw in this branch.
    }

    /// Visage (face) — 1:1 blit at native surface dimensions.
    fn draw_pc_visage(&mut self, pc: &PcSlot<'_>, sh: u16, vis_top: u16) {
        let x = pc.x;
        let vis_bot = if pc.is_selected {
            sh - POSITION_ACTION
        } else {
            sh - CLOSE_POSITION_BOTTOM_SCROLL
        };

        let mut portrait_drawn = false;
        if let Some(visage_kind) = pc
            .entity
            .and_then(|ent| pc_custom_visage_kind(ent, self.profiles))
        {
            let visage = self.portraits.allied_visages[visage_kind.index()]
                .as_ref()
                .unwrap_or_else(|| panic!("PC visage {visage_kind:?} was not loaded"));
            self.renderer.render_gpu_image(
                visage,
                None,
                Some(&bbox(
                    x,
                    vis_top,
                    x + ELEMENT_WIDTH,
                    vis_top + VISAGE_HEIGHT,
                )),
                BlendMode::Blend,
            );
            portrait_drawn = true;
        } else if let Some(ent) = pc.entity
            && let Some(kind) = pc_character_kind(ent)
            && let Some(surface_id) = self.portraits.get_surface(kind)
        {
            let (src_w, src_h) = portrait_surface_dimensions(self.renderer, surface_id);
            if src_w > 0 && src_h > 0 {
                let dst = bbox(x, vis_top, x + src_w, vis_top + src_h);
                blit_to_screen_widget(
                    self.renderer,
                    surface_id,
                    None,
                    Some(&dst),
                    BLIT_SOURCE_TRANSPARENT,
                );
                portrait_drawn = true;
            }
        }
        if !portrait_drawn {
            self.renderer.fill_screen(
                Some(&bbox(x, vis_top, x + ELEMENT_WIDTH, vis_bot)),
                color_visage_fill(),
            );
        }
    }

    /// Fighting sword overlay (period=10 frames).
    /// Positioned at visage top-left, per-character bitmap (91×44 px).
    /// When selected the sword is always visible; otherwise it blinks
    /// on the odd half of each 10-frame cycle.
    fn draw_pc_fighting_overlay(&mut self, pc: &PcSlot<'_>, vis_top: u16) {
        let fighting_visible =
            pc.is_selected || (self.engine.frame_counter() / 10).is_multiple_of(2);
        if fighting_visible
            && let Some(ent) = pc.entity
            && let Some(kind) = pc_action_character_kind(ent, self.profiles)
            && let Some(sid) = self.portraits.get_fighting_surface(kind)
        {
            let (fw, fh) = portrait_surface_dimensions(self.renderer, sid);
            let dst = bbox(pc.x, vis_top, pc.x + fw, vis_top + fh);
            blit_to_screen_widget(
                self.renderer,
                sid,
                None,
                Some(&dst),
                BLIT_SOURCE_TRANSPARENT,
            );
        }
    }

    /// Action buttons of an open PC portrait.
    ///
    /// Switches between 3-button (40+32+40) and 2-button (56+56)
    /// layout based on whether the profile's third action is NoAction.
    /// We detect this from action_icons[2] being None.
    fn draw_pc_action_buttons(
        &mut self,
        pc: &PcSlot<'_>,
        sh: u16,
        hovered_action: Option<(u8, u8)>,
        shift_held: bool,
    ) {
        let x = pc.x;
        let entity = pc.entity;
        let profiles = self.profiles;
        let portraits = self.portraits;
        let act_top = sh - POSITION_ACTION;
        let act_bot = sh - POSITION_BOTTOM_SCROLL;

        let kind = entity.and_then(|entity| pc_action_character_kind(entity, profiles));
        let icons_for = |visual| kind.and_then(|k| portraits.action_icons(k, visual).copied());
        let action_icons = icons_for(ActionButtonVisual::Normal);
        let action_disabled = icons_for(ActionButtonVisual::Disabled);
        let action_hover = icons_for(ActionButtonVisual::Hover);
        let action_pressed = icons_for(ActionButtonVisual::Pressed);
        let action_hover_pressed = icons_for(ActionButtonVisual::HoverPressed);

        let two_button_mode = action_icons
            .as_ref()
            .is_some_and(|icons| icons[2].is_none());

        let (btn_lefts, btn_rights, num_buttons) = if two_button_mode {
            let a_right = x + ACTIONA_WIDTH;
            let b_right = a_right + ACTIONB_WIDTH;
            ([x, a_right, 0], [a_right, b_right, 0], 2)
        } else {
            let a1_right = x + ACTION1_WIDTH;
            let a2_right = a1_right + ACTION2_WIDTH;
            let a3_right = a2_right + ACTION3_WIDTH;
            ([x, a1_right, a2_right], [a1_right, a2_right, a3_right], 3)
        };

        // Determine active action button index and disabled state.
        let active_idx = if shift_held {
            entity.and_then(|entity| {
                action_index(
                    profiles,
                    entity,
                    self.engine.planned_action_for_seat(self.local_seat),
                )
            })
        } else {
            entity.and_then(|e| active_action_index(profiles, e))
        };

        for i in 0..num_buttons {
            let is_active = active_idx == Some(i as u8);
            let is_disabled = !shift_held
                && entity
                    .and_then(|e| e.pc_data())
                    .is_some_and(|pc| pc.action_slot_disabled(i));
            let is_hovered = hovered_action == Some((pc.slot as u8, i as u8));

            let mut icon_drawn = false;

            let visual = action_button_visual(is_active, is_disabled, is_hovered);
            let visual_surface = match visual {
                ActionButtonVisual::Disabled => action_disabled
                    .as_ref()
                    .and_then(|disabled| disabled[i])
                    .or_else(|| action_icons.as_ref().and_then(|icons| icons[i])),
                ActionButtonVisual::HoverPressed => action_hover_pressed
                    .as_ref()
                    .and_then(|hover_pressed| hover_pressed[i])
                    .or_else(|| action_pressed.as_ref().and_then(|pressed| pressed[i])),
                ActionButtonVisual::Pressed => {
                    action_pressed.as_ref().and_then(|pressed| pressed[i])
                }
                ActionButtonVisual::Hover => action_hover.as_ref().and_then(|hover| hover[i]),
                ActionButtonVisual::Normal => None,
            }
            .or_else(|| action_icons.as_ref().and_then(|icons| icons[i]));

            if let Some(surface_id) = visual_surface {
                let dst = bbox(btn_lefts[i], act_top, btn_rights[i], act_bot);
                blit_to_screen_widget(
                    self.renderer,
                    surface_id,
                    None,
                    Some(&dst),
                    BLIT_SOURCE_TRANSPARENT,
                );
                icon_drawn = true;
            }
            if !icon_drawn {
                self.renderer.fill_screen(
                    Some(&bbox(
                        btn_lefts[i] + 1,
                        act_top + 1,
                        btn_rights[i] - 1,
                        act_bot - 1,
                    )),
                    color_action_fill(),
                );
            }

            // Disabled-state BTTN resources are already authored as
            // grayed-out sprites.  Do not add a synthetic stipple on
            // top: it produces visible horizontal artifacts through
            // portrait action icons.
        }
    }

    /// ── Quick-action icon strip ──
    /// Positioned 20 px above the top scroll, 33 px wide each.
    /// Shared icon per slot; alternate sprite while this slot is the
    /// active recording target.
    fn draw_pc_quick_action_strip(
        &mut self,
        pc: &PcSlot<'_>,
        upper_top: u16,
        titbit_renderer: &mut Option<&mut TitbitRenderer>,
    ) {
        let engine = self.engine;
        let pc_id = pc.pc_id;
        let x = pc.x;
        let qa_strip_y = upper_top.saturating_sub(QA_ICON_HEIGHT);
        // `MacroStore` creates a PC's record lazily (`get_or_insert`) on its
        // first recorded quick-action step, so a PC that has never recorded
        // one (or whose recording is armed but has no step yet) legitimately
        // has no record: that means "no macros", not missing state.
        let macro_state = engine.portrait_macro(pc_id);
        let recording_slot = if engine.is_qa_recording_for(pc_id) {
            macro_state.and_then(|m| m.recording_slot())
        } else {
            None
        };
        for slot_idx in 0..NUMBER_OF_QA_MEMORY_U16 {
            let has_macro = macro_state.is_some_and(|m| m.has_macro(slot_idx as usize));
            let is_recording_slot = recording_slot == Some(slot_idx as u8);
            if !has_macro && !is_recording_slot {
                continue;
            }

            let sid_opt = if is_recording_slot {
                self.portraits
                    .qa_icon_recording_surface
                    .or(self.portraits.qa_icon_surface)
            } else {
                self.portraits.qa_icon_surface
            };
            let Some(sid) = sid_opt else { continue };

            let icon_x = x + slot_idx * QA_ICON_WIDTH;
            let (iw, ih) = portrait_surface_dimensions(self.renderer, sid);
            let dst = bbox(icon_x, qa_strip_y, icon_x + iw, qa_strip_y + ih);
            blit_to_screen_widget(
                self.renderer,
                sid,
                None,
                Some(&dst),
                BLIT_SOURCE_TRANSPARENT,
            );

            // Single-frame titbit sprite overlay.  Looks up the slot's
            // titbit id in the per-PC slot table and resolves to the one
            // `RHID_QUICKACTION_TITBITS` sub-frame via the titbit
            // manager's phase lookup.
            //
            // The per-slot titbit id is registered at
            // `record_macro_step_for` (`engine/commands.rs`) on the
            // first committed PlayerCommand of a recording.
            //
            // Layered on top: the falling-button refresh animation —
            // each slot tracks `shift_phase` (px) that re-arms to
            // `SHIFT_STEP` whenever the step count changes and decays
            // by `SHIFT_FALL_PER_REFRESH` each draw.  The titbit icon is
            // offset by `shift_phase` along +X to produce the slide.
            let slot_idx_usz = slot_idx as usize;
            let shift_phase = self
                .frontend
                .presentation
                .engine_display
                .macro_shift_phase(pc_id, slot_idx_usz);
            // Fizzle-blink visibility: the QA strobe toggles the per-slot
            // titbit on/off after a macro fizzles.  When blink-hidden,
            // skip the titbit blit.
            let blink_hidden = self
                .frontend
                .presentation
                .engine_display
                .macro_titbit_blink_hidden(pc_id, slot_idx_usz);
            if has_macro && !blink_hidden {
                let macro_state =
                    macro_state.expect("has_macro implies the PC's macro record exists");
                let frame = macro_state
                    .get_slot_titbit(slot_idx as usize)
                    .and_then(|id| engine.titbit_manager().get_phase(id));
                // The per-slot `run` flag carries through into the
                // shifting-titbit renderer, which then draws a second
                // copy of the sprite offset by `(3, 0)`.  The flag is
                // driven by `is_running_for_qa(...)` on the slot's
                // titbit id; a slot without a live titbit is not running.
                let run = macro_state
                    .get_slot_titbit(slot_idx as usize)
                    .is_some_and(|id| engine.titbit_manager().is_running_for_qa(id));
                if let (Some(tbr), Some(frame)) = (titbit_renderer.as_mut(), frame) {
                    let shift_px = shift_phase.round() as i32;
                    tbr.blit_ui_frame(
                        self.renderer,
                        SpriteRow::QuickActionTitbits,
                        frame,
                        Rect::new(
                            icon_x as i32 + shift_px,
                            qa_strip_y as i32,
                            iw as u32,
                            ih as u32,
                        ),
                        run,
                    );
                }
            }
        }
        render_auto_queue_ticks(
            self.frontend,
            self.renderer,
            engine,
            self.local_seat,
            crate::host::QueueStripIdentity::Pc(pc_id),
            std::slice::from_ref(&pc_id),
            x,
            i32::from(qa_strip_y.saturating_sub(8)),
        );
    }
}

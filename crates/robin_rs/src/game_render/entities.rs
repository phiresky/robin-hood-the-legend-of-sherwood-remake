//! entities presentation pass.
use super::*;

pub(super) fn render_character_masks_clipped(
    engine: &PresentationView<'_>,
    renderer: &mut Renderer,
    layer: u16,
    world_bbox: &engine_coordinates::MapBBox,
    position: engine_coordinates::MapPoint,
    clip_rect: Rect,
    draw_checkpoint: usize,
    view: engine_coordinates::MapPoint,
    zoom: f32,
) {
    let mask_indices = engine
        .fast_grid()
        .get_masks_applied_to_character(layer, world_bbox, position);
    if mask_indices.is_empty() {
        return;
    }
    let screen_masks = sprite_screen_masks(engine, &mask_indices, view, zoom);
    renderer.mask_queued_draws(draw_checkpoint, &screen_masks, clip_rect);
}

pub(super) fn sprite_screen_masks(
    engine: &PresentationView<'_>,
    mask_indices: &[engine_mask::MaskIndex],
    view: engine_coordinates::MapPoint,
    zoom: f32,
) -> Vec<(u32, Rect)> {
    let mut screen_masks = Vec::with_capacity(mask_indices.len());
    for &mask_idx in mask_indices {
        let mask = &engine.fast_grid().level.masks[usize::from(mask_idx)];
        let mask_screen_x = ((mask.bbox.x_min() - view.x) * zoom).round() as i32;
        let mask_screen_y = ((mask.bbox.y_min() - view.y) * zoom).round() as i32;
        let mask_screen_w = (mask.width as f32 * zoom).round() as u32;
        let mask_screen_h = (mask.height as f32 * zoom).round() as u32;
        if mask_screen_w == 0 || mask_screen_h == 0 {
            continue;
        }
        let mask_rect = Rect::new(mask_screen_x, mask_screen_y, mask_screen_w, mask_screen_h);
        screen_masks.push((u32::from(mask_idx), mask_rect));
    }
    screen_masks
}

pub(super) fn applicable_sprite_masks(
    engine: &PresentationView<'_>,
    assets: &LevelAssets,
    actor_layer: u16,
    sprite_world_bbox: &engine_coordinates::MapBBox,
    actor_position: engine_coordinates::MapPoint,
    projectile_position: engine_coordinates::WorldPoint3D,
    use_projectile_path: bool,
    is_flying_human: bool,
) -> Vec<engine_mask::MaskIndex> {
    if use_projectile_path {
        engine.fast_grid().get_masks_applied_to_projectile(
            engine.fast_grid().level.special_layer,
            sprite_world_bbox,
            projectile_position,
            is_flying_human,
            engine.sight_obstacles(assets),
        )
    } else {
        engine.fast_grid().get_masks_applied_to_character(
            actor_layer,
            sprite_world_bbox,
            actor_position,
        )
    }
}

// ─── GPU entity rendering ─────────────────────────────────────────

pub(super) fn entity_visual_map_position(entity: &Entity) -> MapPoint {
    entity.sprite_visual_map_position()
}

/// Render all entities using cached GPU textures.
///
/// Replaces `render_entities` for the GPU phase.  Each sprite frame is
/// decompressed once and cached as an ARGB8888 GPU texture; subsequent
/// frames with the same `(bank_id, variant, shadow_color)` key reuse the
/// cached texture in a queued GPU draw (zero CPU decompression work).
pub(crate) fn render_entities_gpu(
    host: &HostDraw<'_>,
    presentation: &FramePresentationInputs,
    engine: &PresentationView<'_>,
    assets: &LevelAssets,
    dev: &DevState,
    renderer: &mut Renderer,
    titbit_renderer: &mut TitbitRenderer,
) {
    let view = presentation.view;
    let zoom = presentation.zoom;
    let screen_w = presentation.screen_size.x as i32;
    let screen_h = presentation.screen_size.y as i32;
    let shadow_color = presentation.shadow_color;
    let global_shadow = host.frontend.resources.frame_holder().global_shadow();
    let blip_shadow = host.frontend.resources.frame_holder().global_blip_shadow();
    // When the player has disabled "Display Animations" in the graphics
    // options, unforced non-patched non-elevated non-masked FX should
    // not render.  The flag defaults to `true` so the live datadir is
    // unaffected; it only bites when the user toggles it off in the
    // options menu.
    let graphic_config = &presentation.graphic_config;
    let display_anim = graphic_config.display_anim;
    let apply_fog_to_all_sprites = graphic_config.apply_fog_to_all_sprites;

    for &entity_id in &presentation.draw_order_ids {
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        if !entity.is_active() || entity.element_data().hidden_in_building {
            continue;
        }
        if !engine.fog_entity_visible(entity_id) && !uses_pixel_fog_visibility(entity) {
            continue;
        }
        // FX entities early-return when `is_to_be_displayed` is false;
        // non-FX kinds always pass.
        if !entity.is_to_be_displayed(display_anim) {
            continue;
        }
        let variant = engine.resolve_render_variant_for_ambiance(
            entity,
            apply_fog_to_all_sprites,
            presentation.ambiance,
        );

        // ── Interleave titbits that belong behind this entity ─────
        // Immediately before drawing each human entity, flush any
        // pending titbits whose depth falls behind this entity's so
        // they render back-to-front with the entity list (projectile /
        // dust / stars sit between actors at the correct depth instead
        // of piled on top at the end).
        if entity.is_human()
            && let Some(entity_depth) = host.frontend.presentation.draw_order.depth(entity_id)
        {
            titbit_renderer.render_up_to(host, engine, assets, renderer, entity_depth);
        }

        let elem = entity.element_data();
        let visual_pos = entity_visual_map_position(entity);
        let world_x = visual_pos.x;
        let world_y = visual_pos.y;
        if world_x == 0.0 && world_y == 0.0 {
            continue;
        }

        let screen_x = ((world_x - view.x) * zoom) as i32;
        let screen_y = ((world_y - view.y) * zoom) as i32;

        if outside_sprite_cull_margin((screen_x, screen_y), (screen_w, screen_h), zoom) {
            continue;
        }

        // Try GPU sprite rendering.
        //
        // Using `current_scripts_opt` (not a direct `.scripts` field
        // read) is essential for blipped NPCs: `load_frame_info` stores
        // the normal character as primary + `blip00` as alternate, and
        // flips `use_alternate_profile` so the blip silhouette is the
        // active profile until reveal flips it back.  A direct
        // field read would show the revealed character even while it
        // should still be a shadow.  Always go through the active-
        // profile pointer.
        let sprite = &elem.sprite;
        let Some((script, frame, bank_id)) = current_render_frame(sprite) else {
            render_entity_fallback(
                renderer,
                entity.kind(),
                screen_x,
                screen_y,
                screen_w,
                screen_h,
            );
            continue;
        };

        // Blipped (undiscovered) NPCs render from the `blip00`
        // alternate profile as a silhouette sprite; the alpha-keying
        // pass uses the global blip shadow (60) for this branch vs the
        // global shadow (40) for normal characters.
        let mut shadow_level = if sprite.use_alternate_profile {
            blip_shadow
        } else {
            global_shadow
        };
        // FX entities switch on `rendering_properties`: `NeedShadow`
        // composites a shadow, `Blocky` doesn't.  Zero `shadow_level`
        // for `Blocky` FX so the cached sprite key drops the shadow
        // tint.
        if matches!(entity.kind(), ElementKind::Fx)
            && let Some(fx) = entity.fx_data()
            && fx.rendering_properties == RenderingProperties::Blocky
        {
            shadow_level = 0;
        }

        if let Some((sw, sh)) = renderer.ensure_sprite_cached(
            host.frontend.resources.frame_holder(),
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        ) {
            // Sprite screen position:
            //   sprite_pos  = floor(position_map - sprite.center)
            //   blit_origin = sprite_pos + script_offset
            //   screen_xy   = (blit_origin - view) * zoom
            // The floor() in world space (before zoom) is critical for
            // pixel-perfect alignment.
            let placement = sprite_placement(
                MapPoint::new(world_x, world_y),
                sprite.center,
                script.offsets[frame as usize],
                view,
                zoom,
            );
            let (dst_x, dst_y) = placement.screen_origin;

            let dst_rect = zoomed_sprite_rect(dst_x, dst_y, sw, sh, zoom);
            let kind = entity.kind();
            let actor_layer = elem.layer();
            let is_flying_human = elem.posture() == Posture::Flying;
            let hidden_outline_rgb = if host.frontend.input.feedback.draw_hidden {
                // Ground objects always use Hidden; actors retain their active
                // targeting/parrying outline just like the original path.
                let color_565 = if matches!(
                    kind,
                    robin_engine::element::ElementKind::ObjectBonus
                        | robin_engine::element::ElementKind::ObjectOther
                        | robin_engine::element::ElementKind::ObjectScroll
                ) {
                    elem.outline_colors[OutlineColorName::Hidden as usize]
                } else {
                    elem.active_outline_color()
                };
                (color_565 != 0).then(|| rgb565_to_rgb8(color_565))
            } else {
                None
            };

            // Cheat-teleport hulk-rebuild fade.  When
            // `teleport_counter > 0`, the PC is rendered TWICE: first
            // at `position_before_teleport` with alpha
            // `100 * counter / max_counter` (the vanishing ghost),
            // then at the current position with alpha
            // `100 - 100 * counter / max_counter` (the appearing
            // sprite).  As the counter ticks down 20→0 the ghost
            // fades out and the new sprite fades in.  The per-frame
            // decrement is done in `pre_render_engine_setup` via
            // `EngineInner::tick_pc_teleport_fades`.
            let teleport_fade = entity.pc_data().and_then(|pc| {
                if pc.teleport_counter > 0 && pc.max_teleport_counter > 0 {
                    let ratio = pc.teleport_counter as f32 / pc.max_teleport_counter as f32;
                    let old_alpha_255 = (ratio * 255.0).round().clamp(0.0, 255.0) as u8;
                    let new_alpha_255 = ((1.0 - ratio) * 255.0).round().clamp(0.0, 255.0) as u8;
                    Some((pc.position_before_teleport, old_alpha_255, new_alpha_255))
                } else {
                    None
                }
            });

            if let Some((before, old_alpha, _new_alpha)) = teleport_fade {
                // Render the vanishing ghost at the pre-teleport
                // position first, so the appearing sprite stacks on
                // top.
                let ghost = sprite_placement(
                    before,
                    sprite.center,
                    script.offsets[frame as usize],
                    view,
                    zoom,
                );
                let (ghost_dst_x, ghost_dst_y) = ghost.screen_origin;
                let ghost_x = ghost.world_origin.x;
                let ghost_y = ghost.world_origin.y;
                let ghost_rect = zoomed_sprite_rect(ghost_dst_x, ghost_dst_y, sw, sh, zoom);
                let ghost_draw_checkpoint = renderer.draw_queue_checkpoint();
                renderer.render_cached_sprite_alpha(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    ghost_rect,
                    old_alpha,
                );
                let ghost_world_bbox = engine_coordinates::MapBBox::from_coords(
                    ghost_x,
                    ghost_y,
                    ghost_x + sw as f32,
                    ghost_y + sh as f32,
                );
                let current_world = elem.position();
                let ghost_world = engine_coordinates::WorldPoint3D::new(
                    before.x,
                    before.y + current_world.z,
                    current_world.z,
                );
                let ghost_mask_indices = applicable_sprite_masks(
                    engine,
                    assets,
                    actor_layer,
                    &ghost_world_bbox,
                    before,
                    ghost_world,
                    is_flying_human,
                    is_flying_human,
                );
                let ghost_screen_masks =
                    sprite_screen_masks(engine, &ghost_mask_indices, view, zoom);
                renderer.mask_queued_draws(ghost_draw_checkpoint, &ghost_screen_masks, ghost_rect);
                if let Some(rgb) = hidden_outline_rgb {
                    for &(mask_idx, mask_rect) in &ghost_screen_masks {
                        let mask = &engine.fast_grid().level.masks[mask_idx as usize];
                        renderer.render_hidden_mask_outline(
                            host.frontend.resources.frame_holder(),
                            bank_id,
                            variant,
                            shadow_color,
                            &mask.bitmap,
                            mask.width,
                            mask.height,
                            mask_rect,
                            ghost_rect,
                            rgb,
                        );
                    }
                }
            }

            // The teleport ghost above is masked independently at its old
            // position; this checkpoint applies current-position masks only
            // to the appearing sprite.
            let sprite_draw_checkpoint = renderer.draw_queue_checkpoint();

            // When the GoldenEye cheat is on, every PC sprite is
            // composited at 50% alpha (~128/255 in 8-bit).  Teleport
            // fade takes precedence — these are `else if` siblings.
            if let Some((_, _, new_alpha)) = teleport_fade {
                renderer.render_cached_sprite_alpha(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    dst_rect,
                    new_alpha,
                );
            } else if entity.is_pc() && engine.get_golden_eye_mode() {
                renderer.render_cached_sprite_alpha(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    dst_rect,
                    128,
                );
            } else {
                renderer.render_cached_sprite(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    dst_rect,
                );
            }

            // ── Sprite occlusion masks ──
            //
            // After drawing the sprite, ask the grid for any building
            // masks that apply to this actor's position + layer, then
            // blit each mask's pre-composed background texture on top
            // of the sprite.  Where the mask is set the building
            // pixels reappear in front of the actor; elsewhere the
            // texture is transparent and the sprite stays visible.
            let sprite_world_bbox = engine_coordinates::MapBBox::from_coords(
                placement.world_origin.x,
                placement.world_origin.y,
                placement.world_origin.x + sw as f32,
                placement.world_origin.y + sh as f32,
            );
            let actor_position = engine_coordinates::MapPoint::new(world_x, world_y);
            // The mask lookup switches between
            // `get_masks_applied_to_character` and
            // `get_masks_applied_to_projectile` based on the masking
            // category.  PCs override to flying-human masking when
            // their posture is `Flying` so a PC mid-jump no longer
            // gets clipped by the building it's soaring over.  Arrows,
            // thrown bonuses and nets (`ElementKind::ObjectProjectile`
            // / `ObjectNet`) use the projectile masking category so
            // they route through the projectile polyline + 3D
            // altitude test, not the character polyline.
            // The mask pass is gated on `has_valid_box_for_masking`.
            // FX / target overlays never set the flag, so they render
            // without building-mask occlusion.  Flying humans use the
            // original projectile/flying-human mask path.
            if !kind.has_valid_box_for_masking() && !is_flying_human {
                // Nothing more to do: sprite is drawn, no mask pass.
                continue;
            }
            let use_projectile_path = is_flying_human || kind.is_projectile();
            let projectile_mask_position =
                transition_crenel_climb_up_mask_position(entity, engine, assets)
                    .unwrap_or_else(|| elem.position());
            let mask_indices = applicable_sprite_masks(
                engine,
                assets,
                actor_layer,
                &sprite_world_bbox,
                actor_position,
                projectile_mask_position,
                use_projectile_path,
                is_flying_human,
            );
            // When `draw_hidden` is on, the original mutates the
            // temporary sprite surface per mask: masked pixels become
            // transparent, except horizontal transparent/body edges
            // become the actor's outline colour. Stencil rejection does the
            // transparency part; the hidden outline pass restores those edge
            // pixels.
            let screen_masks = sprite_screen_masks(engine, &mask_indices, view, zoom);
            if use_projectile_path {
                renderer.mask_queued_draws(sprite_draw_checkpoint, &screen_masks, dst_rect);
            } else {
                renderer.mask_queued_draws_with_depth(
                    sprite_draw_checkpoint,
                    &screen_masks,
                    dst_rect,
                    view.x,
                    view.y,
                    zoom,
                    projectile_mask_position.y,
                );
            }

            for &(mask_idx, mask_rect) in &screen_masks {
                let mask = &engine.fast_grid().level.masks[mask_idx as usize];
                if let Some(rgb) = hidden_outline_rgb {
                    renderer.render_hidden_mask_outline(
                        host.frontend.resources.frame_holder(),
                        bank_id,
                        variant,
                        shadow_color,
                        &mask.bitmap,
                        mask.width,
                        mask.height,
                        mask_rect,
                        dst_rect,
                        rgb,
                    );
                }
            }
            if dev.debug.sprite_masks_display {
                render_sprite_mask_debug_overlay(
                    host,
                    engine,
                    renderer,
                    &sprite_world_bbox,
                    actor_position,
                    projectile_mask_position,
                    use_projectile_path,
                    &mask_indices,
                );
            }
        } else {
            render_entity_fallback(
                renderer,
                entity.kind(),
                screen_x,
                screen_y,
                screen_w,
                screen_h,
            );
        }
    }
}

/// Large patch animations are environment replacements, not point-sized
/// actors. Their anchor can sit behind the wall that their sprite closes, so
/// anchor visibility would discard the entire animation before the final fog
/// composite can clip it accurately per pixel.
pub(super) fn uses_pixel_fog_visibility(entity: &Entity) -> bool {
    matches!(entity, Entity::Fx(fx) if fx.fx.patch_index.is_some())
}

pub(super) fn transition_crenel_climb_up_mask_position(
    entity: &robin_engine::element::Entity,
    engine: &PresentationView<'_>,
    assets: &LevelAssets,
) -> Option<engine_coordinates::WorldPoint3D> {
    use robin_engine::order::OrderType;

    let elem = entity.element_data();
    if elem.sprite.last_action != OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
        || elem.sprite.current_frame != 0
    {
        return None;
    }
    let actor = entity.actor_data()?;
    let door_pass = actor.active_door_pass.as_ref()?;
    if door_pass.current_action != OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel {
        return None;
    }
    if !engine.has_mission_geometry() {
        return None;
    }
    let door = engine.doors().get(usize::from(door_pass.door_index))?;
    let point_mid = door.point_mid;
    let point_out = door.point_out;

    // The original game applies the high-crenel transition projection when the action finishes:
    // Set map position to the midpoint, obstacle/material to the exit projection
    // area, and old map position to the exit; then recompute all positions.
    // Frame 0 is visually still anchored at the pre-snap map point so its
    // offset lines up with frame 1, but the flying-human mask decision must
    // already use the far-side projection or the wall projectile masks erase
    // the whole frame.
    let door_sector_index = door.sector_out_index?;
    let mut best_z: Option<f32> = None;
    for obs in engine.sight_obstacles(assets).iter() {
        let Some(attachment) = obs.projection_area_ref() else {
            continue;
        };
        if attachment.layer.get() != door.layer_out
            || attachment.sector != door_sector_index
            || !obs.contains_point_projection(point_out)
        {
            continue;
        }
        let z = obs.compute_top_z(point_mid.x, point_mid.y);
        best_z = Some(best_z.map_or(z, |old| old.max(z)));
    }
    let z = best_z?;
    Some(engine_coordinates::WorldPoint3D {
        x: point_mid.x,
        y: point_mid.y + z,
        z,
    })
}

pub(super) fn render_sprite_mask_debug_overlay(
    host: &HostDraw<'_>,
    engine: &PresentationView<'_>,
    renderer: &mut Renderer,
    sprite_world_bbox: &engine_coordinates::MapBBox,
    actor_position: engine_coordinates::MapPoint,
    position_3d: engine_coordinates::WorldPoint3D,
    use_projectile_path: bool,
    mask_indices: &[engine_mask::MaskIndex],
) {
    if mask_indices.is_empty() && !use_projectile_path {
        return;
    }

    let sprite_color = if mask_indices.is_empty() {
        0x07ff
    } else {
        0xf81f
    };
    draw_map_bbox_outline(host, renderer, sprite_world_bbox, sprite_color);

    for &mask_idx in mask_indices {
        let mask = &engine.fast_grid().level.masks[usize::from(mask_idx)];
        draw_map_bbox_outline(host, renderer, &mask.bbox, 0xffe0);
    }

    draw_map_cross(host, renderer, actor_position, 0x07e0);
    if use_projectile_path {
        let projectile_test_point = position_3d.to_map();
        let actor_screen = map_to_screen(host, actor_position);
        let projectile_screen = map_to_screen(host, projectile_test_point);
        renderer.draw_line_screen(
            actor_screen.0,
            actor_screen.1,
            projectile_screen.0,
            projectile_screen.1,
            0xfd20,
        );
        draw_map_cross(host, renderer, projectile_test_point, 0xfd20);
    }
}

pub(super) fn draw_map_bbox_outline(
    host: &HostDraw<'_>,
    renderer: &mut Renderer,
    bbox: &engine_coordinates::MapBBox,
    color: u16,
) {
    if !bbox.is_somewhere() {
        return;
    }
    let (x1, y1) = map_to_screen(
        host,
        engine_coordinates::MapPoint::new(bbox.x_min(), bbox.y_min()),
    );
    let (x2, y2) = map_to_screen(
        host,
        engine_coordinates::MapPoint::new(bbox.x_max(), bbox.y_max()),
    );
    renderer.draw_rect_outline_screen(x1, y1, x2, y2, color);
}

pub(super) fn draw_map_cross(
    host: &HostDraw<'_>,
    renderer: &mut Renderer,
    point: engine_coordinates::MapPoint,
    color: u16,
) {
    let (x, y) = map_to_screen(host, point);
    renderer.draw_line_screen(x - 4, y, x + 4, y, color);
    renderer.draw_line_screen(x, y - 4, x, y + 4, color);
}

pub(super) fn map_to_screen(
    host: &HostDraw<'_>,
    point: engine_coordinates::MapPoint,
) -> (i32, i32) {
    let point = host.viewport().map_to_screen_unclamped(point);
    (point.x.round() as i32, point.y.round() as i32)
}

// ─── GPU selection outline pass ──────────────────────────────────

/// Render coloured outlines for selected PCs and the hovered entity.
///
/// The selection-outline pass runs after all entity sprites are drawn
/// so the outline is drawn ON TOP of entities and is never occluded.
///
/// For each outlined entity, the cached outline mask texture is tinted and
/// alpha-modulated by the GPU pipeline (for hulk fade animation).
pub(crate) fn render_selection_outlines_gpu(
    host: &HostDraw<'_>,
    presentation: &FramePresentationInputs,
    engine: &PresentationView<'_>,
    renderer: &mut Renderer,
) {
    let view = presentation.view;
    let zoom = presentation.zoom;
    let screen_w = presentation.screen_size.x as i32;
    let screen_h = presentation.screen_size.y as i32;
    let shadow_color = presentation.shadow_color;
    let shadow_level = host.frontend.resources.frame_holder().global_shadow();
    let apply_fog_to_all_sprites = presentation.graphic_config.apply_fog_to_all_sprites;

    for &entity_id in &presentation.draw_order_ids {
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        if !entity.is_active() || entity.element_data().hidden_in_building {
            continue;
        }
        if !engine.fog_entity_visible(entity_id) {
            continue;
        }
        let variant = engine.resolve_render_variant_for_ambiance(
            entity,
            apply_fog_to_all_sprites,
            presentation.ambiance,
        );

        let elem = entity.element_data();

        // The outline is blitted only when the PC is mouse-marked or
        // its `running_hulk` is positive.  The selection set on its
        // own does NOT draw the outline — `refresh_pc_selection_hulk`
        // seeds `running_hulk` on the first frame of selection and
        // decrements it each tick, so the glow naturally fades from
        // 100 down to 40 over `HULK_LENGTH` frames and then vanishes.
        //
        // `is_focused` stands in for the mouse-hover mark.
        // `is_action_marked` covers the requirement-bar action flag,
        // which marks every PC matching the hovered action.  Either
        // forces `hulk_level = 100` for one frame.
        // Mark contributions are prepared between the entity and outline
        // passes. Borrow this phase's completed selection through immutable
        // Host access, rather than snapshotting last frame's marks early.
        let is_focused = host.frontend.input.feedback.focused_entity_id == Some(entity_id);
        let is_action_marked = host
            .frontend
            .input
            .feedback
            .marked_pc_ids
            .contains(&entity_id);
        let hulk_running = entity.human_data().is_some_and(|h| h.running_hulk > 0);

        if !is_focused && !is_action_marked && !hulk_running {
            continue;
        }

        let is_selected_tactical = host.frontend.preferences().control_tactical_units()
            && engine
                .tactical_selection(host.local_seat)
                .contains(&entity_id);
        let outline_color_565 = if is_selected_tactical && hulk_running && !is_focused {
            robin_engine::element_kinds::outline_colors::pc_default()
        } else if is_focused || is_action_marked {
            elem.outline_colors[OutlineColorName::Default as usize]
        } else {
            elem.active_outline_color()
        };
        if outline_color_565 == 0 {
            continue;
        }

        // Alpha: focused/marked/action-marked force 100 (override any
        // in-flight fade); otherwise use `hulk_level` (40..=100) from
        // the fade state machine. The percentage (0-100) is converted
        // to the renderer's 0-255 alpha range.
        let alpha_pct = if is_focused || is_action_marked {
            100u16
        } else {
            entity.human_data().map(|h| h.hulk_level).unwrap_or(100)
        };
        let alpha_255 = ((alpha_pct as u32) * 255 / 100).min(255) as u8;

        // Resolve sprite frame (same calculation as render_entities_gpu).
        // See note there about `current_scripts_opt` vs direct field read.
        let sprite = &elem.sprite;
        let Some((script, frame, bank_id)) = current_render_frame(sprite) else {
            continue;
        };

        // Screen position. Use the same visual anchor as sprite rendering
        // so hover outlines stay aligned with targets and airborne actors.
        let visual_pos = entity_visual_map_position(entity);
        let world_x = visual_pos.x;
        let world_y = visual_pos.y;
        let screen_x = ((world_x - view.x) * zoom) as i32;
        let screen_y = ((world_y - view.y) * zoom) as i32;
        if outside_sprite_cull_margin((screen_x, screen_y), (screen_w, screen_h), zoom) {
            continue;
        }

        // Sprite-position calculation (same as render_entities_gpu).
        let placement = sprite_placement(
            MapPoint::new(world_x, world_y),
            sprite.center,
            script.offsets[frame as usize],
            view,
            zoom,
        );
        let (dst_x, dst_y) = placement.screen_origin;

        if let Some((ow, oh)) = renderer.ensure_outline_cached(
            host.frontend.resources.frame_holder(),
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        ) {
            let rgb = rgb565_to_rgb8(outline_color_565);
            let outline_x = dst_x - (OUTLINE_PAD as f32 * zoom).round() as i32;
            let outline_y = dst_y;
            let outline_rect = zoomed_sprite_rect(outline_x, outline_y, ow, oh, zoom);
            renderer.render_cached_outline(
                bank_id,
                variant,
                shadow_color,
                shadow_level,
                outline_rect,
                rgb,
                alpha_255,
            );
        }
    }
}

/// Fallback: draw a colored rectangle for entities without sprites.
pub(super) fn render_entity_fallback(
    renderer: &mut Renderer,
    kind: robin_engine::element::ElementKind,
    screen_x: i32,
    screen_y: i32,
    screen_w: i32,
    screen_h: i32,
) {
    use robin_engine::element::ElementKind;

    let (r, g, b): (u8, u8, u8) = match kind {
        ElementKind::ActorPc => (0, 255, 0),
        ElementKind::ActorSoldier => (255, 0, 0),
        ElementKind::ActorCivilian => (0, 0, 255),
        ElementKind::Fx => (255, 224, 0),
        ElementKind::Target => (255, 0, 255),
        ElementKind::ObjectBonus => (0, 255, 255),
        _ => (255, 255, 255),
    };

    let half = 4;
    let x = (screen_x - half).max(0);
    let y = (screen_y - half).max(0);
    let w = ((screen_x + half).min(screen_w) - x).max(0);
    let h = ((screen_y + half).min(screen_h) - y).max(0);
    if w > 0 && h > 0 {
        renderer.render_gpu_rect(x, y, w, h, [r, g, b, 255]);
    }
}

// ─── Background animation rendering ──────────────────────────────────

/// Render background animations (elevation-0 FX) as GPU sprites.
///
/// Iterates the background-animations list and renders them BEFORE
/// the main entity loop.  Background animations are excluded from
/// `display_order` by `sort_for_display`, so we render them in a
/// dedicated pass here.
///
/// Must be called after `flush_base_layer` (GPU phase active) and before
/// `render_entities_gpu`.
pub(crate) fn render_bg_animations_gpu(
    engine: &PresentationView<'_>,
    host: &HostDraw<'_>,
    presentation: &FramePresentationInputs,
    renderer: &mut Renderer,
) {
    let mut bg_animation_ids = engine.bg_animation_ids().peekable();
    if bg_animation_ids.peek().is_none() {
        return;
    }
    render_fx_entities_gpu(bg_animation_ids, engine, host, presentation, renderer);
}

pub(super) fn render_fx_entities_gpu<I>(
    entity_ids: I,
    engine: &PresentationView<'_>,
    host: &HostDraw<'_>,
    presentation: &FramePresentationInputs,
    renderer: &mut Renderer,
) where
    I: IntoIterator<Item = engine_element::EntityId>,
{
    let view = host.viewport().view_position;
    let zoom = host.viewport().zoom_factor;
    let screen_w = host.viewport().screen_size.x as i32;
    let screen_h = host.viewport().screen_size.y as i32;
    let shadow_color = presentation.shadow_color;
    let global_shadow = host.frontend.resources.frame_holder().global_shadow();

    // Bg animations are unforced ground-level non-masked FX, so they
    // are suppressed when the player has disabled "Display Animations"
    // unless `force_display` or `patch_index` overrides.  See
    // `render_entities_gpu` for the full gate; identical logic via
    // `Entity::is_to_be_displayed`.
    let graphic_config = &presentation.graphic_config;
    let display_anim = graphic_config.display_anim;
    let apply_fog_to_all_sprites = graphic_config.apply_fog_to_all_sprites;

    for entity_id in entity_ids {
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        if !entity.is_active() {
            continue;
        }
        if !entity.is_to_be_displayed(display_anim) {
            continue;
        }
        let variant = engine.resolve_render_variant_for_ambiance(
            entity,
            apply_fog_to_all_sprites,
            presentation.ambiance,
        );

        let elem = entity.element_data();
        let sprite = &elem.sprite;
        let Some((script, frame, bank_id)) = current_render_frame(sprite) else {
            continue;
        };

        let world_x = elem.position_map().x;
        let world_y = elem.position_map().y;
        if world_x == 0.0 && world_y == 0.0 {
            continue;
        }

        let screen_x = ((world_x - view.x) * zoom) as i32;
        let screen_y = ((world_y - view.y) * zoom) as i32;
        if outside_sprite_cull_margin((screen_x, screen_y), (screen_w, screen_h), zoom) {
            continue;
        }

        // FX entities composite a shadow when `rendering_properties`
        // is `NeedShadow`, and skip it for `Blocky`.  Zero
        // `shadow_level` for `Blocky` FX.
        let shadow_level = match entity.fx_data() {
            Some(fx) if fx.rendering_properties == RenderingProperties::Blocky => 0,
            _ => global_shadow,
        };

        if let Some((sw, sh)) = renderer.ensure_sprite_cached(
            host.frontend.resources.frame_holder(),
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        ) {
            let placement = sprite_placement(
                MapPoint::new(world_x, world_y),
                sprite.center,
                script.offsets[frame as usize],
                view,
                zoom,
            );
            let (dst_x, dst_y) = placement.screen_origin;

            let dst_rect = zoomed_sprite_rect(dst_x, dst_y, sw, sh, zoom);
            renderer.render_cached_sprite(bank_id, variant, shadow_color, shadow_level, dst_rect);
        }
    }
}

/// Renderer-path wrapper around [`crate::hud_text::render_text_background`]
/// for the ransom/amulet overlay and dev noise labels.  Routes the
/// shadow+foreground pass through the native/TrueType renderer instead of the
/// old HUD surface-raster path.
pub(super) fn render_text_with_shadow(
    renderer: &mut Renderer,
    fonts: &HudFonts,
    text: &str,
    x: i32,
    y: i32,
) {
    hud_text::render_text_background(
        &fonts.tooltip_font,
        fonts.shadow_font.as_ref(),
        text,
        x,
        y,
        |f, t, fx, fy| {
            layout::render_text_screen_font(renderer, f, t, fx, fy);
        },
    );
}

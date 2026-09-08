//! Map-patch insertion and background restoration for patch-effect FX entities.
//!
//! The renderer keeps the base map immutable and records each successful
//! map patches as persistent GPU decals. The render loop draws those
//! decals immediately after the base map texture and before gameplay
//! overlays, preserving the visible layer order without CPU-side
//! background writes.

use crate::bg_cache::BackgroundDecal;
use crate::gfx_types::Rect;
use crate::host::{HostEffectBatches, HostFrontend};
use robin_assets::frame_holder::SpriteVariant;
use robin_engine::element as engine_element;
use robin_engine::engine::{PendingBgBlit, PendingBgBlitDecal};

/// Drain queued map-patch insertion and background-restoration requests into
/// host-owned persistent background decals.
pub fn drain_pending_bg_blits(frontend: &mut HostFrontend, effects: &mut HostEffectBatches) {
    let blits: Vec<PendingBgBlit> = std::mem::take(&mut effects.background_blits);
    if blits.is_empty() {
        return;
    }

    for blit in blits {
        let _ = apply_bg_blit(frontend, blit);
    }
}

/// Render every persistent patch-effect background decal.
///
/// Called after `Engine::draw_background` has queued the base map and before
/// `Renderer::flush_base_layer`, so these sprites live in the same visual
/// layer as the old baked map pixels.
pub fn render_background_decals(frontend: &HostFrontend, renderer: &mut crate::renderer::Renderer) {
    if frontend.background_decals.is_empty() {
        return;
    }

    let view = frontend.viewport.view_position;
    let zoom = frontend.viewport.zoom_factor;
    let screen_w = frontend.viewport.screen_size.x as i32;
    let screen_h = frontend.viewport.screen_size.y as i32;
    let margin = 256;

    for entity_id in &frontend.background_decal_order {
        let Some(decal) = frontend.background_decals.get(entity_id) else {
            continue;
        };
        let dst_x = ((decal.dst_x as f32 - view.x) * zoom) as i32;
        let dst_y = ((decal.dst_y as f32 - view.y) * zoom) as i32;
        let dst_w = (decal.width as f32 * zoom).ceil().max(1.0) as u32;
        let dst_h = (decal.height as f32 * zoom).ceil().max(1.0) as u32;

        if dst_x + dst_w as i32 <= -margin
            || dst_y + dst_h as i32 <= -margin
            || dst_x >= screen_w + margin
            || dst_y >= screen_h + margin
        {
            continue;
        }

        let Some((_sw, _sh)) = renderer.ensure_sprite_cached(
            &frontend.frame_holder,
            decal.bank_id,
            SpriteVariant::Day,
            decal.shadow_color,
            decal.shadow_level,
        ) else {
            continue;
        };

        renderer.render_cached_sprite(
            decal.bank_id,
            SpriteVariant::Day,
            decal.shadow_color,
            decal.shadow_level,
            Rect::new(dst_x, dst_y, dst_w, dst_h),
        );
    }
}

/// Apply a single queued request. Returns true if the persistent decal set
/// changed.
fn apply_bg_blit(frontend: &mut HostFrontend, blit: PendingBgBlit) -> bool {
    if blit.restore_only {
        let removed = frontend.background_decals.remove(&blit.entity_id).is_some();
        if removed {
            frontend
                .background_decal_order
                .retain(|&id| id != blit.entity_id);
        }
        return removed;
    }

    let Some(decal) = build_background_decal(frontend, blit.entity_id, blit.decal) else {
        return false;
    };
    if !frontend.background_decals.contains_key(&blit.entity_id) {
        frontend.background_decal_order.push(blit.entity_id);
    }
    frontend.background_decals.insert(blit.entity_id, decal);
    true
}

fn build_background_decal(
    frontend: &HostFrontend,
    entity_id: engine_element::EntityId,
    snapshot: Option<PendingBgBlitDecal>,
) -> Option<BackgroundDecal> {
    if let Some(snapshot) = snapshot {
        let width = frontend.frame_holder.sprite_width(snapshot.bank_id) as u32;
        let height = frontend.frame_holder.sprite_height(snapshot.bank_id) as u32;
        if width == 0 || height == 0 || (width == 1 && height == 1) {
            tracing::warn!(
                ?entity_id,
                bank_id = snapshot.bank_id,
                "blit_to_map: snapshotted patch FX frame has empty sprite dimensions"
            );
            return None;
        }

        return Some(BackgroundDecal {
            bank_id: snapshot.bank_id,
            dst_x: snapshot.dst_x,
            dst_y: snapshot.dst_y,
            width,
            height,
            shadow_color: snapshot.shadow_color,
            shadow_level: frontend.frame_holder.global_shadow(),
        });
    }

    tracing::warn!(?entity_id, "blit_to_map: missing patch FX snapshot");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_drains_effects_with_only_frontend_authority() {
        let mut frontend = HostFrontend::default();
        let mut effects = HostEffectBatches::default();
        let id = engine_element::EntityId::Fx(engine_element::FxId(7));
        frontend.background_decals.insert(
            id,
            BackgroundDecal {
                bank_id: 1,
                dst_x: 0,
                dst_y: 0,
                width: 4,
                height: 4,
                shadow_color: 0,
                shadow_level: 0,
            },
        );
        frontend.background_decal_order.push(id);
        effects.background_blits.push(PendingBgBlit {
            entity_id: id,
            restore_only: true,
            decal: None,
        });
        effects.request_sherwood_report();

        drain_pending_bg_blits(&mut frontend, &mut effects);

        assert!(frontend.background_decals.is_empty());
        assert!(frontend.background_decal_order.is_empty());
        assert!(effects.background_blits.is_empty());
        assert!(effects.has_sherwood_report(), "unrelated effects survive");
    }
}

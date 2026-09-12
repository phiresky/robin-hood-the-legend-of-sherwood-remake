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
        if !apply_bg_blit(frontend, blit) {
            tracing::warn!("Background decal could not be built");
        }
    }
}

/// Render every persistent patch-effect background decal.
///
/// Called after `Engine::draw_background` has queued the base map and before
/// `Renderer::flush_base_layer`, so these sprites live in the same visual
/// layer as the old baked map pixels.
pub fn render_background_decals(
    frontend: &HostFrontend,
    viewport: &crate::host::ViewportState,
    renderer: &mut crate::renderer::Renderer,
) {
    if frontend.resources.background_decals.is_empty() {
        return;
    }

    let view = viewport.view_position;
    let zoom = viewport.zoom_factor;
    let screen_w = viewport.screen_size.x as i32;
    let screen_h = viewport.screen_size.y as i32;
    let margin = 256;

    for decal in frontend.resources.background_decals.in_draw_order() {
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
            frontend.resources.frame_holder(),
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
        return frontend
            .resources
            .background_decals
            .remove(blit.entity_id)
            .is_some();
    }

    let Some(decal) = build_background_decal(frontend, blit.entity_id, blit.decal) else {
        return false;
    };
    frontend
        .resources
        .background_decals
        .insert(blit.entity_id, decal);
    true
}

fn build_background_decal(
    frontend: &HostFrontend,
    entity_id: engine_element::EntityId,
    snapshot: Option<PendingBgBlitDecal>,
) -> Option<BackgroundDecal> {
    if let Some(snapshot) = snapshot {
        let width = frontend
            .resources
            .frame_holder()
            .sprite_width(snapshot.bank_id) as u32;
        let height = frontend
            .resources
            .frame_holder()
            .sprite_height(snapshot.bank_id) as u32;
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
            shadow_level: frontend.resources.frame_holder().global_shadow(),
        });
    }

    tracing::warn!(?entity_id, "blit_to_map: missing patch FX snapshot");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_reports_absence_without_reordering_surviving_patches() {
        let mut frontend = HostFrontend::default();
        for bank_id in 1..=4 {
            frontend.resources.background_decals.insert(
                engine_element::EntityId::Fx(engine_element::FxId(bank_id)),
                BackgroundDecal {
                    bank_id,
                    dst_x: 0,
                    dst_y: 0,
                    width: 4,
                    height: 4,
                    shadow_color: 0,
                    shadow_level: 0,
                },
            );
        }

        for expected_changed in [true, false] {
            assert_eq!(
                apply_bg_blit(
                    &mut frontend,
                    PendingBgBlit {
                        entity_id: engine_element::EntityId::Fx(engine_element::FxId(2)),
                        restore_only: true,
                        decal: None,
                    },
                ),
                expected_changed
            );
            assert_eq!(
                frontend
                    .resources
                    .background_decals
                    .in_draw_order()
                    .map(|decal| decal.bank_id)
                    .collect::<Vec<_>>(),
                [1, 3, 4]
            );
        }
    }

    #[test]
    fn restore_drains_effects_with_only_frontend_authority() {
        let mut frontend = HostFrontend::default();
        let mut effects = HostEffectBatches::default();
        let id = engine_element::EntityId::Fx(engine_element::FxId(7));
        frontend.resources.background_decals.insert(
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
        effects.background_blits.push(PendingBgBlit {
            entity_id: id,
            restore_only: true,
            decal: None,
        });
        effects.request_sherwood_report();

        drain_pending_bg_blits(&mut frontend, &mut effects);

        assert!(frontend.resources.background_decals.is_empty());
        assert!(effects.background_blits.is_empty());
        assert!(effects.has_sherwood_report(), "unrelated effects survive");
    }
}

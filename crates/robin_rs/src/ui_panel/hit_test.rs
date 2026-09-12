use super::*;

// ─── Portrait hit-testing ─────────────────────────────────────────

/// Which sub-area of a portrait was clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortraitHitArea {
    /// Top scroll (health gauge / parchment).
    TopScroll,
    /// Face / visage area.
    Visage,
    /// One of the action buttons (0-based index).
    ActionButton(u8),
    /// Allied group controls: stance, patrol, formation, follow.
    AlliedAction(u8),
    /// Pin a transient group or unpin a persistent group.
    Pin,
    PageLeft,
    PageRight,
    /// One of the quick-action macro icons (0-based QA slot).
    QuickAction(u8),
    /// Bottom scroll (ammo count area).
    BottomScroll,
    /// Guard indicator (burned state only).
    Guard,
    /// Amulet/clover indicator (burned state, not guarded).
    Amulet,
    /// Reinforcement trumpet indicator (dead state, when a replacement is
    /// still available).
    Trumpet,
}

/// Result of a portrait hit-test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortraitHit {
    /// Portrait slot index (0-based into `engine.displayed_pc_ids()`).
    /// Hidden-interface PCs are not part of the displayed list, so this
    /// index is *not* a valid index into `engine.pc_ids()` whenever any
    /// PC has `interface_hidden = true` — use `pc_id` instead of
    /// re-indexing.
    pub slot: u8,
    /// Resolved PC entity id at this slot — saves the caller from
    /// re-walking `displayed_pc_ids()`.
    pub pc_id: robin_engine::element::EntityId,
    pub target: PortraitTarget,
    /// Which sub-area was clicked.
    pub area: PortraitHitArea,
    /// Whether this portrait's PC is burned (coma/dead).
    pub is_burned: bool,
}

/// Hit-test a screen-space click against the portrait slots (simple version).
///
/// Returns the index of the clicked portrait slot (0-based into `engine.pc_ids()`),
/// or `None` if the click is outside all portrait areas.
pub fn hit_test_portrait(
    screen_width: u16,
    screen_height: u16,
    click_x: f32,
    click_y: f32,
    num_pcs: usize,
) -> Option<u8> {
    let num_slots = num_pcs.min(portrait_capacity(screen_width));
    let slot_count = portrait_slot_count(screen_width, num_slots);
    let sh = screen_height;

    let panel_top = sh - PORTRAIT_TOTAL_HEIGHT;
    let panel_bot = sh - BORDURE;

    // Quick reject: not in panel area
    if click_y < panel_top as f32 || click_y > panel_bot as f32 {
        return None;
    }

    for slot in 0..num_slots {
        let x = slot_left_x(screen_width, slot as u16, slot_count) as f32;
        let x2 = x + ELEMENT_WIDTH as f32;

        if click_x >= x && click_x <= x2 {
            return Some(slot as u8);
        }
    }

    None
}

/// Detailed hit-test returning which sub-area of which portrait was clicked.
///
/// Uses engine state to determine burned/selected per slot, and maps
/// the click Y to the appropriate sub-area.
pub fn hit_test_portrait_detailed(
    engine: &PresentationView<'_>,
    local_seat: PlayerId,
    portraits: &PortraitCache,
    screen_width: u16,
    screen_height: u16,
    click_x: f32,
    click_y: f32,
) -> Option<PortraitHit> {
    let (items, paged) = portrait_bar_items(engine, local_seat, screen_width);
    let slot_count = portrait_slot_count(screen_width, items.len());
    let num_slots = items.len();
    let sh = screen_height;
    let cy = click_y;

    let panel_top = (sh - PORTRAIT_TOTAL_HEIGHT - QA_ICON_HEIGHT) as f32;
    let panel_bot = (sh - BORDURE) as f32;

    if cy < panel_top || cy > panel_bot {
        return None;
    }

    if paged {
        let representative = items
            .first()
            .and_then(|item| item.members().first())
            .copied()
            .expect("paged portrait bar has no representative entity");
        if click_x <= 30.0 {
            return Some(PortraitHit {
                slot: 0,
                pc_id: representative,
                target: items[0].target(),
                area: PortraitHitArea::PageLeft,
                is_burned: false,
            });
        }
        if click_x >= screen_width.saturating_sub(30) as f32 {
            return Some(PortraitHit {
                slot: 0,
                pc_id: representative,
                target: items[0].target(),
                area: PortraitHitArea::PageRight,
                is_burned: false,
            });
        }
    }

    for (slot, item) in items.iter().enumerate().take(num_slots) {
        let x = slot_left_x(screen_width, slot as u16, slot_count) as f32;
        let pin_right = x + f32::from(ALLIED_PIN_LEFT + ALLIED_PIN_ICON_SIZE);
        let x2 = if matches!(item.target(), PortraitTarget::Pc(_)) {
            x + ELEMENT_WIDTH as f32
        } else {
            pin_right
        };

        if click_x < x || click_x > x2 {
            continue;
        }

        let pc_id = item.members()[0];
        if !matches!(item.target(), PortraitTarget::Pc(_)) {
            let selected = engine.tactical_selection(local_seat) == item.members();
            let top_scroll_top = if selected {
                (sh - POSITION_TOP_SCROLL) as f32
            } else {
                (sh - CLOSE_POSITION_TOP_SCROLL) as f32
            };
            let visage_top = if selected {
                (sh - POSITION_VISAGE) as f32
            } else {
                (sh - CLOSE_POSITION_VISAGE) as f32
            };
            let pin_left = x + f32::from(ALLIED_PIN_LEFT);
            let pin_top = top_scroll_top - f32::from(ALLIED_PIN_RISE);
            let pin_bottom = pin_top + f32::from(ALLIED_PIN_ICON_SIZE);
            let action_top = (sh - POSITION_ACTION) as f32;
            let bottom_scroll_top = (sh - POSITION_BOTTOM_SCROLL) as f32;
            let area =
                if click_x >= pin_left && click_x <= pin_right && cy >= pin_top && cy <= pin_bottom
                {
                    PortraitHitArea::Pin
                } else if cy >= top_scroll_top && cy < visage_top {
                    PortraitHitArea::TopScroll
                } else if cy >= visage_top && (!selected || cy < action_top) {
                    PortraitHitArea::Visage
                } else if selected && cy >= action_top && cy < bottom_scroll_top {
                    PortraitHitArea::AlliedAction(allied_action_index(click_x - x))
                } else {
                    PortraitHitArea::BottomScroll
                };
            return Some(PortraitHit {
                slot: slot as u8,
                pc_id,
                target: item.target(),
                area,
                is_burned: false,
            });
        }

        let entity = engine.get_entity(pc_id);
        if entity.is_none() {
            tracing::warn!(?pc_id, "portrait hit test references a missing entity");
            continue;
        }
        let is_selected = engine.hero_selection(local_seat).contains(&pc_id);

        let is_coma = entity.map(|e| is_pc_in_coma(engine, e)).unwrap_or(false);
        if cy < (sh - PORTRAIT_TOTAL_HEIGHT) as f32 && (!is_selected || is_coma) {
            continue;
        }
        if is_selected && !is_coma {
            let qa_strip_y = (sh - POSITION_TOP_SCROLL - QA_ICON_HEIGHT) as f32;
            let qa_strip_bot = qa_strip_y + QA_ICON_HEIGHT as f32;
            if cy >= qa_strip_y && cy < qa_strip_bot {
                let rel_x = click_x - x;
                if rel_x >= 0.0 {
                    let slot_idx = (rel_x / QA_ICON_WIDTH as f32).floor() as u8;
                    if usize::from(slot_idx) < robin_engine::macro_store::NUMBER_OF_QA_MEMORY {
                        return Some(PortraitHit {
                            slot: slot as u8,
                            pc_id,
                            target: item.target(),
                            area: PortraitHitArea::QuickAction(slot_idx),
                            is_burned: false,
                        });
                    }
                }
            }
        }
        if cy < (sh - PORTRAIT_TOTAL_HEIGHT) as f32 {
            continue;
        }

        let is_dead = match entity {
            Some(Entity::Pc(pc)) => pc.pc.life_points <= 0,
            _ => false,
        };
        let is_burned = is_dead || is_coma;
        let is_guarded = match entity {
            Some(Entity::Pc(pc)) => pc.pc.guard.is_some(),
            _ => false,
        };
        let has_trumpet = match entity {
            Some(Entity::Pc(pc)) => pc.pc.trumpet_enabled,
            _ => false,
        };

        if is_burned {
            // Burned layout: upper scroll repositioned above lower scroll.
            // Guard/amulet/trumpet indicator is between the two scrolls.
            let bottom_scroll_top = (sh - POSITION_BOTTOM_SCROLL) as f32;
            let upper_scroll_top = (sh - POSITION_BOTTOM_SCROLL - BOTTOM_SCROLL_HEIGHT) as f32;
            let upper_scroll_bot = upper_scroll_top + TOP_SCROLL_HEIGHT as f32;

            let area = if cy >= upper_scroll_top && cy < upper_scroll_bot {
                PortraitHitArea::TopScroll
            } else if cy >= upper_scroll_bot && cy < bottom_scroll_top {
                // Between scrolls — trumpet (dead only) takes priority over
                // guard/amulet (coma only).  The trumpet is only enabled on
                // dead PCs, so the two indicator families never overlap in
                // practice.
                if has_trumpet && is_dead && !is_coma {
                    PortraitHitArea::Trumpet
                } else if is_guarded {
                    PortraitHitArea::Guard
                } else {
                    PortraitHitArea::Amulet
                }
            } else {
                PortraitHitArea::BottomScroll
            };

            return Some(PortraitHit {
                slot: slot as u8,
                pc_id,
                target: item.target(),
                area,
                is_burned,
            });
        }

        // Normal (non-burned) layout
        let top_scroll_top = if is_selected {
            (sh - POSITION_TOP_SCROLL) as f32
        } else {
            (sh - CLOSE_POSITION_TOP_SCROLL) as f32
        };
        let visage_top = if is_selected {
            (sh - POSITION_VISAGE) as f32
        } else {
            (sh - CLOSE_POSITION_VISAGE) as f32
        };
        let action_top = (sh - POSITION_ACTION) as f32;
        let bottom_scroll_top = (sh - POSITION_BOTTOM_SCROLL) as f32;

        let area = if cy >= top_scroll_top && cy < visage_top {
            // Check pixel transparency on the curved scroll edges.
            // If the pixel is transparent, reject the hit so the click falls through.
            if let Some(ref mask) = portraits.top_scroll_hit_mask {
                let rel_x = (click_x - x) as u16;
                let rel_y = (cy - top_scroll_top) as u16;
                if !mask.is_opaque(rel_x, rel_y) {
                    return None;
                }
            }
            PortraitHitArea::TopScroll
        } else if cy >= visage_top && cy < action_top && is_selected {
            PortraitHitArea::Visage
        } else if cy >= visage_top && !is_selected {
            // Closed state: visage extends down to bottom scroll
            PortraitHitArea::Visage
        } else if cy >= action_top && cy < bottom_scroll_top && is_selected {
            // Determine which action button based on X.
            // Check two-button mode
            let action_icons = entity
                .and_then(pc_character_kind)
                .map(|k| k.action_resources());
            let two_btn = action_icons
                .as_ref()
                .is_some_and(|icons| icons[2].is_none());
            let rel_x = click_x - x;

            let btn_idx = if two_btn {
                if rel_x < ACTIONA_WIDTH as f32 { 0 } else { 1 }
            } else if rel_x < ACTION1_WIDTH as f32 {
                0
            } else if rel_x < (ACTION1_WIDTH + ACTION2_WIDTH) as f32 {
                1
            } else {
                2
            };
            PortraitHitArea::ActionButton(btn_idx)
        } else {
            PortraitHitArea::BottomScroll
        };

        return Some(PortraitHit {
            slot: slot as u8,
            pc_id,
            target: item.target(),
            area,
            is_burned,
        });
    }

    None
}

//! Mouse cursor, focus detection, and input processing.

use super::*;
use crate::coordinates::{GroundPoint, MapPoint};
use crate::element::{Entity, EntityId};

// ─── Mouse cursor constants ─────────────────────────────────────

/// Default mouse cursor shadow intensity.
pub const MOUSE_OPACITY_DEFAULT: u16 = 40;

/// Shadow color for bow-no / out-of-range (red: RGB565 of 0xFF0000).
pub const MOUSE_BOW_NO_COLOR: u16 = 0xF800;

/// Shadow color for bow targeting a civilian (blue: RGB565 of 0x0090FF).
pub const MOUSE_BOW_CIVIL_COLOR: u16 = 0x049F;

/// Shadow color for bow targeting a VIP (purple: RGB565 of 0xAA00FF).
pub const MOUSE_BOW_VIP_COLOR: u16 = 0xA81F;

// ─── Projectile range constants ─────────────────────────────────

/// `sqrt(1.33)` — corrects for isometric aspect ratio in range calculations.
const RANGE_BALANCE_FACTOR: f32 = 1.153;
use crate::position_interface::INVERSE_ASPECT_RATIO_PROJECTILES as INVERSE_ASPECT_RATIO_PROJ;
/// Throw angle for bow shots (radians).
const THROW_ANGLE_BOW: f32 = 0.3;

const THROW_DISTANCE_APPLE: f32 = 300.0 * RANGE_BALANCE_FACTOR;
const THROW_DISTANCE_STONE: f32 = 200.0 * RANGE_BALANCE_FACTOR;
const THROW_DISTANCE_PURSE: f32 = 300.0 * RANGE_BALANCE_FACTOR;
const THROW_DISTANCE_NET: f32 = 300.0 * RANGE_BALANCE_FACTOR;
const THROW_DISTANCE_WASP_NEST: f32 = 300.0 * RANGE_BALANCE_FACTOR;
/// Throw angle for all non-bow projectiles (radians) — apple, stone,
/// purse, net, wasp-nest all use the same value.
const THROW_ANGLE_PROJECTILE: f32 = 0.1;

// ─── Bow target result ──────────────────────────────────────────

/// Result of `can_shoot_with_bow_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BowTarget {
    Valid,
    OutOfRange,
    Invalid,
}

#[inline]
fn bow_ground_y(point: crate::coordinates::WorldPoint3D) -> f32 {
    // C++ `GetPositionGround()` returns `(GetPosition().mX, GetPosition().mY)`.
    // It does not project with `mY - mZ`; bow range, facing, and preview
    // source direction all use this 3D-space Y directly.
    point.y
}

/// Output of `compute_trajectory_preview*`. Host applies this to its
/// own `valid_trajectory` / `trajectory_preview_*` fields after the
/// readonly computation returns. See `Host` in `robin_rs::game` for
/// where these fields live.
///
/// - `Invalid`: clear points, mark invalid (e.g. non-shot action).
/// - `ShowArc { crumpled, .. }`: valid preview; render the arc. `crumpled`
///   is set when the shot will miss, used by the render layer to swap
///   the arc colour from cyan to pink.
/// - `HitNoArc`: valid but don't draw (the shot will hit).
#[derive(Debug, Clone)]
pub enum TrajectoryPreview {
    Invalid,
    ShowArc {
        points: Vec<crate::element::TrajectoryPoint>,
        start: crate::coordinates::WorldPoint3D,
        crumpled: bool,
        /// Shooter's layer — used by the host to place ground marks at
        /// the projected impact point.
        layer: u16,
    },
    HitNoArc,
}

impl EngineInner {
    // ─── Mouse cursor / focus ────────────────────────────────────

    /// Get the currently selected action for the first selected PC on
    /// the host seat. Reads the selected PC's `current_action`. Returns
    /// `Action::NoAction` if no PC is selected.
    pub fn get_selected_action(&self) -> crate::profiles::Action {
        self.selected_action_for_seat(crate::player_command::PlayerId::HOST)
    }

    /// Get the currently selected action for the first selected PC on
    /// `player_id`'s seat. Returns `Action::NoAction` if that seat has
    /// no selected PC.
    pub fn selected_action_for_seat(
        &self,
        player_id: crate::player_command::PlayerId,
    ) -> crate::profiles::Action {
        self.seat_selection(player_id)
            .first()
            .and_then(|&id| self.get_entity(id))
            .and_then(|e| e.pc_data())
            .map(|pc| pc.current_action)
            .unwrap_or(crate::profiles::Action::NoAction)
    }

    /// Map-space per-pixel hit test for a sprite.
    ///
    /// Builds a map-space AABB from
    /// `floor(position_map - sprite.center) + current_offset`, then checks
    /// containment and pixel-tests the packed sprite data against the
    /// transparent color key and night shadow color.
    ///
    /// When `blue_pixels_are_in` is `true`, night-shadow pixels count
    /// as opaque (used for blipped entities).
    pub fn is_point_on_sprite(
        &self,
        assets: &LevelAssets,
        entity: &Entity,
        point_map: MapPoint,
        blue_pixels_are_in: bool,
    ) -> bool {
        let elem = entity.element_data();
        let sprite = &elem.sprite;

        let visual_pos = entity.sprite_visual_map_position();
        let w = sprite.current_width as f32;
        let h = sprite.current_height as f32;
        if w == 0.0 || h == 0.0 {
            let dx = visual_pos.x - point_map.x;
            let dy = visual_pos.y - point_map.y;
            return dx * dx + dy * dy < 400.0;
        }

        // Box from C++ sprite position + offset to + size.
        let offset = sprite.current_offset();
        let left = visual_pos.x - sprite.center.x + offset.x;
        let top = visual_pos.y - sprite.center.y + offset.y;
        let right = left + w - 1.0;
        let bottom = top + h - 1.0;

        if point_map.x < left || point_map.x > right || point_map.y < top || point_map.y > bottom {
            return false;
        }

        // Per-pixel test: look up the packed sprite data for opacity at the
        // local coordinates within the current animation frame.
        let scripts = sprite.current_scripts_opt();
        if scripts.is_none() {
            // No scripts loaded — bbox test already passed, accept the hit.
            return true;
        }

        // The frame holder lives on the host side (engine carve-out
        // Decision 1), so we reach the packed sprite data through the
        // host-installed `PixelOpacityLookup`. If it's not wired up yet
        // (e.g. early boot, tests), keep the bbox-only hit.
        let Some(lookup) = assets.pixel_opacity.as_ref() else {
            return true;
        };

        let local_x = (point_map.x - left).floor();
        let local_y = (point_map.y - top).floor();
        if local_x < 0.0 || local_y < 0.0 {
            return true;
        }
        let lx = local_x as u16;
        let ly = local_y as u16;
        let bank_id = sprite.bank_id_for(sprite.current_row, sprite.current_frame);
        lookup.is_pixel_opaque(
            bank_id,
            lx,
            ly,
            self.world.weather.night_color,
            blue_pixels_are_in,
        )
    }

    /// Map-space fixed-AABB hit test for an object-class entity.
    ///
    /// The detection box is `position_map ± (10, -20..0)` in map
    /// coordinates, with the boundary treated as inclusive.
    ///
    /// Objects (bonus / scroll / projectile / net) use this instead of
    /// the per-pixel `is_point_on_sprite` hit test that actor entities
    /// use.
    pub fn is_point_over_object(&self, entity: &Entity, point_map: MapPoint) -> bool {
        let pos = entity.element_data().position_map();
        point_map.x >= pos.x - 10.0
            && point_map.x <= pos.x + 10.0
            && point_map.y >= pos.y - 20.0
            && point_map.y <= pos.y
    }

    /// Object-class focusable test — Bonus / Scroll / Projectile.
    ///
    /// Apples and wasp-nests are hard-wired to never be focusable.
    fn is_object_focusable(
        &self,
        entity: &Entity,
        entity_id: EntityId,
        mouse_map: MapPoint,
        focus: crate::element::Focus,
        selected_pc_id: Option<EntityId>,
    ) -> bool {
        use crate::element::{Focus, ObjectType};

        // Only the USE cursor is valid for object focus; every other
        // focus is rejected.
        if focus != Focus::Use {
            return false;
        }

        // Bonus/scroll require a selected PC; additionally, no object
        // is focusable while the selected PC is swordfighting.
        let pc = match selected_pc_id.and_then(|id| self.get_entity(id)) {
            Some(e) => e,
            None => return false,
        };
        let pc_swordfighting = pc.human_data().is_some_and(|h| !h.opponents.is_empty());
        if pc_swordfighting {
            return false;
        }

        match entity {
            Entity::Scroll(_) => {
                // Only Visible/Opened scrolls are focusable. Scroll
                // status is stored on `GameHost::scroll_status` keyed
                // by entity handle.
                use super::scroll_reveal::ScrollStatus;
                if !matches!(
                    self.scroll_status(entity_id),
                    ScrollStatus::Visible | ScrollStatus::Opened
                ) {
                    return false;
                }
            }
            Entity::Bonus(b) => {
                // Relics are not focusable on the Sherwood HQ
                // (forest) level.
                if b.is_relic() && self.world.weather.is_forest_level {
                    return false;
                }
                // Blazons never show USE.
                if b.object.object_type == ObjectType::BonusBlazon {
                    return false;
                }
            }
            Entity::Projectile(p) => {
                // Flying projectiles are not focusable; only landed
                // ones expose USE.
                if p.projectile.flying {
                    return false;
                }
                // Apple and wasp-nest projectiles are hard-wired to
                // never be focusable, even once landed.
                match p.object.object_type {
                    ObjectType::Apple
                    | ObjectType::BonusApple
                    | ObjectType::WaspNest
                    | ObjectType::BonusWaspNest
                    | ObjectType::Wasp => return false,
                    _ => {}
                }
            }
            _ => return false,
        }

        // Layer-sentinel reject, followed by the fixed-AABB hit test.
        if entity.element_data().layer() == u16::MAX {
            return false;
        }
        self.is_point_over_object(entity, mouse_map)
    }

    /// Check whether the selected PC has a given action in its main
    /// action list.
    ///
    /// Distinct from [`Self::selected_pc_has_contextual_action`],
    /// which walks the contextual-action list — net focusability for
    /// example uses the main list.
    pub fn selected_pc_has_action(
        &self,
        assets: &LevelAssets,
        selected_pc_id: Option<EntityId>,
        action: crate::profiles::Action,
    ) -> bool {
        let pc_id = match selected_pc_id {
            Some(id) => id,
            None => return false,
        };
        self.get_entity(pc_id)
            .and_then(|e| e.pc_data())
            .and_then(|pc| assets.profile_manager.get_character(pc.profile_index))
            .is_some_and(|cp| cp.has_action(action))
    }

    /// Check whether the selected PC has a given contextual action.
    ///
    /// Returns `false` when no PC is selected.
    pub fn selected_pc_has_contextual_action(
        &self,
        assets: &LevelAssets,
        selected_pc_id: Option<EntityId>,
        action: crate::profiles::Action,
    ) -> bool {
        let pc_id = match selected_pc_id {
            Some(id) => id,
            None => return false,
        };
        self.get_entity(pc_id)
            .and_then(|e| e.pc_data())
            .and_then(|pc| assets.profile_manager.get_character(pc.profile_index))
            .is_some_and(|cp| cp.has_contextual_action(action))
    }

    /// Whether every currently-selected PC has the Climb contextual action.
    ///
    /// An empty selection returns `true` (vacuously).
    pub fn all_selected_pcs_can_climb(&self, assets: &LevelAssets) -> bool {
        self.players.seats[0].selection.iter().all(|&pc_id| {
            self.selected_pc_has_contextual_action(
                assets,
                Some(pc_id),
                crate::profiles::Action::Climb,
            )
        })
    }

    /// Check whether the selected PC can carry bodies.
    ///
    /// True if the PC has either of the carry actions or the carry
    /// contextual action.
    pub fn selected_pc_can_carry(
        &self,
        assets: &LevelAssets,
        selected_pc_id: Option<EntityId>,
    ) -> bool {
        let pc_id = match selected_pc_id {
            Some(id) => id,
            None => return false,
        };
        self.get_entity(pc_id)
            .and_then(|e| e.pc_data())
            .and_then(|pc| assets.profile_manager.get_character(pc.profile_index))
            .is_some_and(|cp| cp.can_carry())
    }

    /// Check whether an entity is focusable for the given focus type at
    /// the given map position.
    ///
    /// `selected_pc_id` is the specific PC whose abilities gate contextual
    /// actions. When `None`, profile-gated branches
    /// (Search/Execute/Tie) are skipped.
    pub fn is_entity_focusable(
        &self,
        assets: &LevelAssets,
        entity_id: EntityId,
        entity: &Entity,
        mouse_map: MapPoint,
        focus: crate::element::Focus,
        selected_pc_id: Option<EntityId>,
    ) -> bool {
        use crate::element::{Camp, Focus, Posture};

        // PCs check `is_active` per-focus (SELECT and HEAL allow
        // inactive PCs that are inside a building sector via
        // `is_pc_selectable` / the HEAL inactive-in-building branch).
        // Skip the global early-return for PCs so those arms can reach
        // the per-focus gate below.
        if !entity.is_active() && !entity.is_pc() {
            return false;
        }

        // Multi-select gate:
        // - NPC targets reject everything except Sword / Interact / View.
        // - PC targets reject everything except Select / Interact.
        // Split the gate by target kind so a multi-PC selection can
        // still shift-click a third PC into the selection
        // (`Focus::Select`).
        let multi_select_allowed = if entity.is_pc() {
            matches!(focus, Focus::Select | Focus::Interact)
        } else {
            matches!(focus, Focus::Sword | Focus::Interact | Focus::View)
        };
        if !multi_select_allowed && self.players.seats[0].selection.len() > 1 {
            return false;
        }

        // ── Object-class entities ──
        // Bonus / Scroll / Projectile use a fixed-AABB hit test
        // instead of the per-pixel sprite hit below.  Net has its
        // own branch further down because it uses the sprite hit.
        if matches!(
            entity,
            Entity::Bonus(_) | Entity::Scroll(_) | Entity::Projectile(_)
        ) {
            return self.is_object_focusable(entity, entity_id, mouse_map, focus, selected_pc_id);
        }

        // Per-pixel sprite hit test. Blipped entities include
        // night-shadow pixels in the hit test.
        //
        // Exception: NPC targets under `Focus::View` use the
        // `ezekiel_2517` ("Dies irae") cheat flag instead of `blipped`
        // — the cheat lets night-shadow pixels register as hits for
        // the killer-view widget so guards in shadow can still be
        // picked.
        let blipped = entity.element_data().blipped;
        let blue_pixels_are_in = if focus == Focus::View && !entity.is_pc() {
            self.ai.global.ezekiel_2517
        } else {
            blipped
        };
        if !self.is_point_on_sprite(assets, entity, mouse_map, blue_pixels_are_in) {
            return false;
        }

        // ── PC entities ──
        if entity.is_pc() {
            let posture = entity.element_data().posture;
            return match focus {
                // SELECT requires: is-active, selectable, and either
                // not on a HelpingToClimb posture or the selected PC
                // doesn't have Jump (so the human pyramid pose isn't
                // hijacked into a select). The pixel-hit test above
                // already handles the sprite check.
                Focus::Select => {
                    self.is_pc_selectable(assets, entity_id)
                        && (posture != Posture::HelpingToClimb
                            || !self.selected_pc_has_contextual_action(
                                assets,
                                selected_pc_id,
                                crate::profiles::Action::Jump,
                            ))
                }
                Focus::Shield | Focus::ShieldPortrait => {
                    entity.is_active()
                        && !self.players.seats[0].selection.contains(&entity_id)
                        && !entity.is_dead()
                }
                // Heal-active PC must be alive, below max HP, not in
                // coma, not stuck under a net, not carried. Portrait
                // variant + the recording-macro split are elided here
                // (they require host-side context that the current
                // focus call site doesn't route through this predicate).
                // Inactive-in-building is allowed.
                Focus::Heal | Focus::HealPortrait => {
                    if entity.is_dead() {
                        return false;
                    }
                    let Some(pc_human) = entity.human_data() else {
                        return false;
                    };
                    if pc_human.unconscious {
                        return false;
                    }
                    if pc_human.stuck_under_nets_counter > 0 {
                        return false;
                    }
                    if posture == Posture::Carried {
                        return false;
                    }
                    let pc_data = match entity.pc_data() {
                        Some(d) => d,
                        None => return false,
                    };
                    if pc_data.life_points >= crate::combat::LIFEPOINTS_PC {
                        return false;
                    }
                    // In-coma check reads the campaign-level coma
                    // status flag on the PC.
                    let in_coma = self
                        .pc_description_for_pc_data(pc_data)
                        .map(|desc| desc.status.in_coma)
                        .unwrap_or(false);
                    if in_coma {
                        return false;
                    }
                    true
                }
                // Focus::Use on a PC target has three sub-branches:
                //   (a) Target posture is HelpingToClimb and selected
                //       PC has Jump and isn't mid-swordfight → allow.
                //   (b) Target is OutOfOrder, not stuck-under-net, not
                //       carried, not unreachable, and selected PC has
                //       a carry action → allow.
                //   (c) Else reject (no fall-through arm for PC targets).
                Focus::Use => {
                    use crate::profiles::Action;
                    if !entity.is_active() {
                        return false;
                    }
                    let selector_swordfighting = selected_pc_id
                        .and_then(|id| self.get_entity(id))
                        .and_then(|e| e.actor_data())
                        .is_some_and(|a| a.action_state.is_sword());
                    let has_jump = self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        Action::Jump,
                    );
                    if posture == Posture::HelpingToClimb && has_jump && !selector_swordfighting {
                        return true;
                    }
                    let out_of_order =
                        entity.is_dead() || entity.human_data().is_some_and(|h| h.unconscious);
                    let carry = self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        Action::LittleJohnCarry,
                    ) || self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        Action::FarmerCarry,
                    );
                    let is_stuck = entity
                        .human_data()
                        .is_some_and(|h| h.stuck_under_nets_counter > 0);
                    let is_carried = entity.human_data().is_some_and(|h| h.carrier.is_some());
                    if out_of_order && !is_stuck && !is_carried && carry {
                        return true;
                    }
                    false
                }
                // Focus::Interact gates on the selected PC's
                // contextual actions against target state. A PC target
                // is not a soldier/civilian/NPC, and PCs have no
                // `rider` flag, so several branches simplify away.
                Focus::Interact => {
                    use crate::profiles::Action;
                    let is_dead = entity.is_dead();
                    let carry = self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        Action::LittleJohnCarry,
                    ) || self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        Action::FarmerCarry,
                    );
                    let loot = self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        Action::Search,
                    );
                    let terminator = self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        Action::Execute,
                    );
                    let reanimator = self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        Action::Resuscitate,
                    );
                    let tie_man =
                        self.selected_pc_has_contextual_action(assets, selected_pc_id, Action::Tie);
                    // Selected PC is VIP? (needed for Give Money branch.)
                    let selector_is_vip = selected_pc_id
                        .and_then(|id| self.get_entity(id))
                        .is_some_and(|e| self.is_entity_vip(assets, e));
                    // Selected PC is Robin? (needed for Loot-VIP gating.)
                    let selector_is_robin = selected_pc_id
                        .and_then(|id| self.get_entity(id))
                        .and_then(|e| e.pc_data())
                        .is_some_and(|pc| pc.robin);
                    let target_is_vip = self.is_entity_vip(assets, entity);

                    // Carry: carry action && (blipped || !rider). PCs
                    // aren't riders, so this reduces to `carry`.
                    if carry {
                        return true;
                    }
                    // Give Money: selected PC is VIP && (target blipped ||
                    // target civilian beggar). PCs aren't civilians, so
                    // only the blipped branch applies.
                    if selector_is_vip && blipped {
                        return true;
                    }
                    // Loot: loot action && (blipped || (Robin-or-notVIP &&
                    // (!dead || NPC w/ money>0))). PCs aren't NPCs so the
                    // money sub-branch resolves to `!dead`.
                    let robin_loot_vip = selector_is_robin || !target_is_vip;
                    let blip_or_lootable = blipped || (robin_loot_vip && !is_dead);
                    if loot && blip_or_lootable {
                        return true;
                    }
                    // Kill: terminator && (blipped || aliveCommonSoldier).
                    // PCs aren't soldiers, so only blipped triggers.
                    if terminator && blipped {
                        return true;
                    }
                    // Reanimate: reanimator && (blipped || (!dead &&
                    // !civilian && royalist)). PCs are royalist allies
                    // and never civilians.
                    if reanimator && (blipped || !is_dead) {
                        return true;
                    }
                    // Tie up: tie_man && (blipped || (NPC && ...)). PCs
                    // aren't NPCs so only the blipped branch applies.
                    if tie_man && blipped {
                        return true;
                    }
                    false
                }
                _ => false,
            };
        }

        // ── FX targets ──
        // Map the requested focus to a target-filter bit, then gate
        // on a non-empty intersection with the target's advertised
        // filter, combined with the sprite pixel hit. Supported
        // focuses are Bow, Apple, Stone, Heal, Lever (single-bit
        // mapping) and Use (the generic PC-ability mask).
        if let Entity::Target(t) = entity {
            let required = match focus {
                Focus::Bow | Focus::Apple | Focus::Stone | Focus::Heal | Focus::Lever => {
                    super::target_interaction::focus_to_target_filter(focus)
                }
                Focus::Use => {
                    use crate::profiles::Action;
                    let pc_has_search = self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        Action::Search,
                    );
                    let pc_has_lever = self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        Action::Lever,
                    );
                    let pc_is_vip = selected_pc_id
                        .and_then(|id| self.get_entity(id))
                        .is_some_and(|e| self.is_entity_vip(assets, e));
                    super::target_interaction::action_filter_for_pc(
                        pc_has_search,
                        pc_has_lever,
                        pc_is_vip,
                    )
                }
                _ => crate::element::TargetFilter::empty(),
            };
            // The mapped filter must be non-empty *and* share at
            // least one bit with the target's advertised filter.
            return !required.is_empty() && !(required & t.target.action_filter).is_empty();
        }

        // ── Net (trap net on the ground) ──
        // Net is focusable when:
        //   - not flying — net has landed and is sitting on the ground
        //   - selected PC has the Net main action (not the contextual
        //     array)
        //   - single-PC selection
        //   - layer != 0xFFFF
        //   - pixel of sprite at mouse (handled by the earlier
        //     `is_point_on_sprite` check)
        if let Entity::Net(net_elem) = entity {
            if focus != Focus::Use {
                return false;
            }
            if net_elem.projectile.flying {
                return false;
            }
            if entity.element_data().layer() == u16::MAX {
                return false;
            }
            if self.players.seats[0].selection.len() > 1 {
                return false;
            }
            return self.selected_pc_has_action(
                assets,
                selected_pc_id,
                crate::profiles::Action::Net,
            );
        }

        // ── NPC entities (soldiers and civilians) ──
        if !entity.is_npc() {
            return false;
        }

        let is_soldier = entity.is_soldier();
        let is_civilian = entity.is_civilian();
        let is_unconscious = entity.human_data().is_some_and(|h| h.unconscious);
        let is_out_of_order = !entity.human_data().is_some_and(|h| {
            let lp = match entity {
                Entity::Soldier(s) => s.npc.life_points,
                Entity::Civilian(c) => c.npc.life_points,
                _ => 0,
            };
            lp > 0 && !h.unconscious
        });
        let is_dead = entity.is_dead();
        let camp = match entity {
            Entity::Soldier(s) => s.soldier.cached_camp,
            Entity::Civilian(_) => Camp::Lacklandists,
            _ => Camp::Error,
        };
        let posture = entity.element_data().posture;
        let is_tied = posture == Posture::Tied;
        let is_vip = self.is_entity_vip(assets, entity);
        let is_rider = matches!(entity, Entity::Soldier(s) if s.soldier.rider);
        let npc_scroll_attached = match entity {
            Entity::Soldier(s) => s.npc.scroll_attached,
            Entity::Civilian(c) => c.npc.scroll_attached,
            _ => false,
        };
        let is_stuck_under_net = entity
            .human_data()
            .is_some_and(|h| h.stuck_under_nets_counter > 0);
        let npc_money = match entity {
            Entity::Soldier(s) => s.npc.money,
            Entity::Civilian(c) => c.npc.money,
            _ => 0,
        };
        // Rider that is currently moving fast.
        let is_fast_rider = is_rider
            && entity
                .actor_data()
                .map(|a| a.action_state)
                .is_some_and(|s| s == crate::element::ActionState::MovingFast);
        // Whether the currently-selected PC is Robin (used for VIP looting).
        let selected_pc_is_robin = self.players.seats[0]
            .selection
            .first()
            .and_then(|&id| self.get_entity(id))
            .and_then(|e| e.pc_data())
            .is_some_and(|pc| pc.robin);

        match focus {
            Focus::Bow => !(blipped || is_out_of_order || (camp == Camp::Royalists && is_soldier)),
            Focus::Hit => {
                !blipped
                    && !is_out_of_order
                    && camp == Camp::Lacklandists
                    && is_soldier
                    && !is_vip
                    && !is_rider
            }
            // Apple has no VIP/rider exclusion.
            Focus::Apple => !blipped && !is_out_of_order && is_soldier && camp != Camp::Royalists,
            // Stone excludes VIPs.
            Focus::Stone => {
                !blipped && !is_out_of_order && is_soldier && camp != Camp::Royalists && !is_vip
            }
            Focus::View => !blipped,
            // Sword rejects only on blipped / out-of-order / Royalists
            // / fast-rider. Scroll-attached soldiers are *not* excluded:
            // the SWORD switch returns directly without falling through
            // to the NPC focusable check.
            Focus::Sword => {
                !blipped
                    && !is_out_of_order
                    && is_soldier
                    && camp != Camp::Royalists
                    && !is_fast_rider
            }
            // Strangle has two branches keyed on whether a macro is
            // being recorded.
            // Recording: reject only when `!blipped && (is_dead ||
            // Royalists || is_vip || is_rider)` — a blipped soldier
            // stays focusable, and night-shadow pixels are included in
            // the sprite hit test (handled by the earlier
            // `is_point_on_sprite` call which already passes `blipped`).
            // Non-recording: reject on `blipped || Royalists ||
            // is_out_of_order || is_vip || is_rider`.
            // Neither branch checks scroll-attached.
            Focus::Strangle => {
                if !is_soldier {
                    return false;
                }
                if self.is_recording_macro() {
                    blipped || !(is_dead || camp == Camp::Royalists || is_vip || is_rider)
                } else {
                    !blipped && !is_out_of_order && camp != Camp::Royalists && !is_vip && !is_rider
                }
            }
            // Contextual use.
            Focus::Use => {
                if blipped {
                    return false;
                }

                // Dialog / scroll-attached branch.
                if npc_scroll_attached && !is_out_of_order {
                    return true;
                }

                // Search (loot money). Requires money > 0, out-of-order,
                // not stuck under net, the SEARCH contextual action,
                // and VIPs only by Robin.
                if npc_money != 0
                    && is_out_of_order
                    && !is_stuck_under_net
                    && posture != Posture::Carried
                    && self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        crate::profiles::Action::Search,
                    )
                    && (!is_vip || selected_pc_is_robin)
                {
                    return true;
                }
                // Murder (execute unconscious enemy soldier).
                // Requires the EXECUTE contextual action.
                if !is_dead
                    && is_unconscious
                    && posture == Posture::Lying
                    && is_soldier
                    && camp != Camp::Royalists
                    && !is_vip
                    && self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        crate::profiles::Action::Execute,
                    )
                {
                    return true;
                }
                // Tie up. Requires the TIE contextual action.
                // Royalist NPCs on Merry Man Forest levels can't be tied.
                let is_merry_man_forest =
                    camp == Camp::Royalists && self.world.weather.is_forest_level && !is_rider;
                if is_unconscious
                    && !is_dead
                    && !is_tied
                    && posture != Posture::Carried
                    && !is_rider
                    && !is_vip
                    && !is_merry_man_forest
                    && self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        crate::profiles::Action::Tie,
                    )
                {
                    return true;
                }
                // Pay beggar: a VIP PC clicking a living, conscious,
                // non-scroll-attached beggar civilian with enough
                // ransom opens the Pay interaction.
                let is_beggar = matches!(entity, Entity::Civilian(c)
                    if c.civilian.cached_civilian_type
                        == crate::profiles::CivilianType::Beggar);
                // Reject the Use focus while the beggar is playing
                // one of the purse-chain animations (receiving purse,
                // waiting with purse, or the transition back). The
                // chain is driven by `AbilityKind::ReceivePurse`, so
                // we reject the focus whenever that ability is active
                // — prevents a second click from dispatching Pay (and
                // deducting salary) while the beggar is mid-chain.
                let beggar_mid_chain = entity.actor_data().is_some_and(|a| {
                    a.active_ability.kind == Some(crate::movement::AbilityKind::ReceivePurse)
                });
                if is_beggar
                    && !is_dead
                    && !is_unconscious
                    && !npc_scroll_attached
                    && !beggar_mid_chain
                    && posture != Posture::Carried
                {
                    let selector_is_vip = selected_pc_id
                        .and_then(|id| self.get_entity(id))
                        .is_some_and(|e| self.is_entity_vip(assets, e));
                    // Any VIP-targeted beggar is focusable regardless
                    // of ransom; the ransom check happens later in the
                    // cursor pick (PAY_YES vs PAY_NO) and in the click
                    // handler (clicks below salary are no-ops). See
                    // `choose_use_cursor` for the cursor split.
                    if selector_is_vip {
                        return true;
                    }
                }
                // Resuscitate arm — fall-through base for NPC targets.
                // The selected PC is always Royalist, so `same camp`
                // reduces to soldier camp == Royalists. (The PC
                // in-coma branch is unreachable here — this NPC
                // dispatcher is only called for non-PC targets.)
                if !is_dead
                    && is_unconscious
                    && is_soldier
                    && !is_civilian
                    && camp == Camp::Royalists
                    && self.selected_pc_has_contextual_action(
                        assets,
                        selected_pc_id,
                        crate::profiles::Action::Resuscitate,
                    )
                {
                    return true;
                }
                // Carry arm — fall-through base for NPC targets.
                // Note: heavy-target rejection is NOT applied here
                // (only in the cursor pick / click handler).
                if is_out_of_order
                    && posture != Posture::Carried
                    && !is_stuck_under_net
                    && !is_rider
                    && self.selected_pc_can_carry(assets, selected_pc_id)
                {
                    return true;
                }
                false
            }
            // Six contextual-action-gated branches (Carry / GiveMoney /
            // Loot / Kill / Reanimate / Tie). Every branch's blipped
            // short-circuit collapses into the early
            // `if blipped { return true; }` — the body checks are just
            // the non-blipped predicates.
            Focus::Interact => {
                if blipped {
                    return true;
                }
                let pc_has_carry = self.selected_pc_can_carry(assets, selected_pc_id);
                let pc_has_search = self.selected_pc_has_contextual_action(
                    assets,
                    selected_pc_id,
                    crate::profiles::Action::Search,
                );
                let pc_has_execute = self.selected_pc_has_contextual_action(
                    assets,
                    selected_pc_id,
                    crate::profiles::Action::Execute,
                );
                let pc_has_resuscitate = self.selected_pc_has_contextual_action(
                    assets,
                    selected_pc_id,
                    crate::profiles::Action::Resuscitate,
                );
                let pc_has_tie = self.selected_pc_has_contextual_action(
                    assets,
                    selected_pc_id,
                    crate::profiles::Action::Tie,
                );
                let pc_is_vip = selected_pc_id
                    .and_then(|id| self.get_entity(id))
                    .is_some_and(|e| self.is_entity_vip(assets, e));
                let pc_is_robin = selected_pc_id
                    .and_then(|id| self.get_entity(id))
                    .and_then(|e| e.pc_data())
                    .is_some_and(|pc| pc.robin);

                let alive_common_soldier = !is_dead && is_soldier && !is_vip;
                let robin_ally = camp == Camp::Royalists;
                let is_beggar = matches!(entity, Entity::Civilian(c)
                    if c.civilian.cached_civilian_type
                        == crate::profiles::CivilianType::Beggar);

                // 1. Carry: carry action and not a rider — blipped
                //    handled above.
                if pc_has_carry && !is_rider {
                    return true;
                }
                // 2. GiveMoney: selected PC is VIP and target is a
                //    beggar civilian — blipped handled above.
                if pc_is_vip && is_civilian && is_beggar {
                    return true;
                }
                // 3. Loot: search action, (Robin or non-VIP target),
                //    and (alive or NPC with money > 0).
                let robin_loot_vip = pc_is_robin || !is_vip;
                let rich_or_not_dead = !is_dead || npc_money > 0;
                if pc_has_search && robin_loot_vip && rich_or_not_dead {
                    return true;
                }
                // 4. Kill: execute action and alive common soldier.
                if pc_has_execute && alive_common_soldier {
                    return true;
                }
                // 5. Reanimate: resuscitate action and alive
                //    non-civilian Royalist.
                if pc_has_resuscitate && !is_dead && !is_civilian && robin_ally {
                    return true;
                }
                // 6. Tie: tie action, alive, non-VIP, non-rider — the
                //    target here is always an NPC.
                if pc_has_tie && !is_dead && !is_vip && !is_rider {
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Search display-order entities for one that is focusable.
    ///
    /// `draw_order` is the host-cached back-to-front list; we iterate
    /// it in reverse so topmost-drawn entities win the hit test.
    pub fn find_focusable_entity(
        &self,
        assets: &LevelAssets,
        draw_order: &[EntityId],
        mouse_map: MapPoint,
        focus: crate::element::Focus,
    ) -> Option<EntityId> {
        if self.players.seats[0].selection.is_empty()
            && !matches!(focus, crate::element::Focus::Select)
        {
            return None;
        }
        let selected_pc = self.players.seats[0].selection.first().copied();
        for &eid in draw_order.iter().rev() {
            if let Some(e) = self.get_entity(eid)
                && self.is_entity_focusable(assets, eid, e, mouse_map, focus, selected_pc)
            {
                return Some(eid);
            }
        }
        None
    }

    /// Search only NPC entities for one focusable at `mouse_map`.
    pub fn find_focusable_npc(
        &self,
        assets: &LevelAssets,
        mouse_map: MapPoint,
        focus: crate::element::Focus,
    ) -> Option<EntityId> {
        let selected_pc = self.players.seats[0].selection.first().copied();
        for nid in self
            .world
            .entities
            .npc_ids()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            if let Some(e) = self.get_entity(nid)
                && self.is_entity_focusable(assets, nid, e, mouse_map, focus, selected_pc)
            {
                return Some(nid);
            }
        }
        None
    }

    /// Search only PC entities for one focusable at `mouse_map`.
    pub fn find_focusable_pc(
        &self,
        assets: &LevelAssets,
        mouse_map: MapPoint,
        focus: crate::element::Focus,
    ) -> Option<EntityId> {
        let selected_pc = self.players.seats[0].selection.first().copied();
        for &pid in self.world.pc_ids.iter().rev() {
            if let Some(e) = self.get_entity(pid)
                && self.is_entity_focusable(assets, pid, e, mouse_map, focus, selected_pc)
            {
                return Some(pid);
            }
        }
        None
    }

    // ─── Sector helpers for cursor selection ─────────────────────

    /// Check whether the selected PC is in a building or on a
    /// wall/ladder lift. Many projectile/bow actions are blocked in
    /// these sectors.
    pub fn is_selected_pc_in_restricted_sector(&self) -> bool {
        let pc_id = match self.players.seats[0].selection.first() {
            Some(&id) => id,
            None => return false,
        };
        let entity = match self.get_entity(pc_id) {
            Some(e) => e,
            None => return false,
        };
        let elem = entity.element_data();
        let layer = elem.layer();
        // Look up PC's current sector in the grid.
        let pos = elem.position_map();
        let hit = self.world.fast_grid.get_sector(pos, pos, layer);
        match hit {
            crate::fast_find_grid::SectorHit::Found { sector_idx, .. } => {
                if let Some(sector) = self
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .get(usize::from(sector_idx))
                {
                    let st = sector.sector_type;
                    if st.is_building() {
                        return true;
                    }
                    if st.is_lift()
                        && let Some(lt) = sector.lift_type
                    {
                        return lt.is_wall_or_ladder();
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Check whether the mouse-targeted sector is valid for ground-targeted
    /// projectile actions (purse, net, ale).  Returns false if the sector
    /// is a door or a wall/ladder lift.
    pub fn is_mouse_sector_valid_for_ground_target(&self, mouse_map: MapPoint) -> bool {
        let reference = self.players.seats[0]
            .selection
            .first()
            .and_then(|&id| self.get_entity(id))
            .map(|e| e.element_data().position_map())
            .unwrap_or(mouse_map);
        let hit = self.world.fast_grid.get_sector_screen(mouse_map, reference);
        match hit.sector_idx {
            Some(idx) => {
                if let Some(sector) = self.world.fast_grid.level.sectors.get(usize::from(idx)) {
                    let st = sector.sector_type;
                    if st.is_door() {
                        return false;
                    }
                    if st.is_lift()
                        && let Some(lt) = sector.lift_type
                    {
                        return !lt.is_wall_or_ladder();
                    }
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    }

    /// Whether the given PC is currently on a non-stairs lift
    /// (ladder / wall) or inside a building sector.
    pub fn is_climbing_or_inside_building(&self, pc_id: EntityId) -> bool {
        let Some(entity) = self.get_entity(pc_id) else {
            return false;
        };
        let Some(sector_handle) = entity.element_data().sector() else {
            return false;
        };
        let Some(&sector_idx) =
            self.world
                .fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(
                    u16::from(sector_handle) as i16
                ))
        else {
            return false;
        };
        let Some(sector) = self.world.fast_grid.level.sectors.get(sector_idx) else {
            return false;
        };
        if sector.sector_type.is_building() {
            return true;
        }
        if sector.sector_type.is_lift()
            && let Some(lift_type) = sector.lift_type
            && lift_type != crate::sector::LiftType::Stairs
        {
            return true;
        }
        false
    }

    /// Check if an entity is a VIP (via its profile).
    pub fn is_entity_vip(&self, assets: &LevelAssets, entity: &Entity) -> bool {
        match entity {
            Entity::Soldier(s) => assets
                .profile_manager
                .get_soldier(s.soldier.soldier_profile_index)
                .map(|p| p.vip)
                .unwrap_or(false),
            Entity::Civilian(c) => {
                // Civilian VIP = CivilianType::Vip in the civilian profile.
                assets
                    .profile_manager
                    .civilians
                    .get(usize::from(c.civilian.civilian_profile_index))
                    .map(|p| p.civilian_type == crate::profiles::CivilianType::Vip)
                    .unwrap_or(false)
            }
            _ => false,
        }
    }

    // ─── Contextual use cursor dispatch ────────────────────────

    /// Choose the mouse cursor for a Focus::Use target.
    ///
    /// The dispatch chain is:
    ///   0. (Civilian) Alive beggar + selected PC is VIP →
    ///      RHMOUSE_PAY_YES / RHMOUSE_PAY_NO depending on remaining
    ///      ransom
    ///   1. Scroll-attached → RHMOUSE_TALK
    ///   2. Dead/unconscious + SEARCH → RHMOUSE_SEARCH
    ///   3. Unconscious + EXECUTE + lying → RHMOUSE_FINISH_HIM
    ///   4. Unconscious + TIE → RHMOUSE_TIE
    ///   5. Unconscious + RESUSCITATE + same camp → RHMOUSE_WAKE_UP
    ///   6. Dead/unconscious + carry + not carried →
    ///      RHMOUSE_GET_YES/NO
    ///   7. Object targets (Bonus / Scroll / Projectile / Net) route
    ///      through [`Self::choose_object_cursor`].
    ///
    /// Takes only `entity_id` (not `&Entity`) to avoid borrow conflicts
    /// with `host.input` mutation at the call site.
    pub fn choose_use_cursor(
        &self,
        assets: &LevelAssets,
        entity_id: EntityId,
        selected_pc_id: Option<EntityId>,
    ) -> i32 {
        use crate::element::Posture;
        use crate::profiles::Action as PA;
        use crate::resource_ids::*;

        let entity = match self.get_entity(entity_id) {
            Some(e) => e,
            None => return RHMOUSE_DEFAULT,
        };

        // Objects (pickups) route through their own cursor logic —
        // quantity-numeric GET_YES_N / GET_NO.
        if matches!(
            entity,
            Entity::Bonus(_) | Entity::Scroll(_) | Entity::Projectile(_) | Entity::Net(_)
        ) {
            return self.choose_object_cursor(assets, entity_id, selected_pc_id);
        }

        // FX targets — walk the target-mouse-cursor filter ladder
        // for SEARCH / LEVER / CUT / HANDLE / TAKE / MONEY. Returns 0
        // to let the caller fall through to the default arrow for
        // empty or ability-gated filters.
        if let Entity::Target(t) = entity {
            let pc_has_search =
                self.selected_pc_has_contextual_action(assets, selected_pc_id, PA::Search);
            let pc_has_lever =
                self.selected_pc_has_contextual_action(assets, selected_pc_id, PA::Lever);
            let pc_is_vip = selected_pc_id
                .and_then(|id| self.get_entity(id))
                .is_some_and(|e| self.is_entity_vip(assets, e));
            let cursor = super::target_interaction::target_mouse_cursor(
                t.target.action_filter,
                pc_has_search,
                pc_has_lever,
                pc_is_vip,
            );
            return if cursor != 0 { cursor } else { RHMOUSE_DEFAULT };
        }

        let dead = entity.is_dead();
        let unconscious = entity.human_data().is_some_and(|h| h.unconscious);
        let posture = entity.element_data().posture;
        let is_pc = entity.is_pc();

        // 0. Civilian beggar + selected PC is VIP → PAY_YES / PAY_NO.
        // Checked before the shared NPC chain because it pre-empts the
        // talk/search paths for the alive-beggar+VIP case.
        if !dead
            && !unconscious
            && posture != Posture::Carried
            && let Entity::Civilian(c) = entity
            && c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar
            && !c.npc.scroll_attached
        {
            let selector_is_vip = selected_pc_id
                .and_then(|id| self.get_entity(id))
                .is_some_and(|e| self.is_entity_vip(assets, e));
            if selector_is_vip {
                let ransom = self
                    .mission_domain
                    .campaign
                    .as_ref()
                    .map(|c| c.get_value(crate::campaign::CampaignValue::Ransom))
                    .unwrap_or(0);
                return if ransom >= crate::engine::BEGGAR_SALARY {
                    RHMOUSE_PAY_YES
                } else {
                    RHMOUSE_PAY_NO
                };
            }
        }

        // 1. Scroll-attached NPC → talk cursor
        let scroll_attached = match entity {
            Entity::Soldier(s) => s.npc.scroll_attached,
            Entity::Civilian(c) => c.npc.scroll_attached,
            _ => false,
        };
        if !dead && !unconscious && scroll_attached {
            return RHMOUSE_TALK;
        }

        // 2. Dead or unconscious + money + SEARCH → search cursor
        let npc_money = match entity {
            Entity::Soldier(s) => s.npc.money,
            Entity::Civilian(c) => c.npc.money,
            _ => 0,
        };
        if (dead || unconscious)
            && npc_money != 0
            && self.selected_pc_has_contextual_action(assets, selected_pc_id, PA::Search)
        {
            return RHMOUSE_SEARCH;
        }

        // 3. Unconscious + EXECUTE + lying → murder cursor
        if unconscious
            && self.selected_pc_has_contextual_action(assets, selected_pc_id, PA::Execute)
            && posture == Posture::Lying
        {
            return RHMOUSE_FINISH_HIM;
        }

        // 4. Unconscious + TIE → tie cursor
        if unconscious && self.selected_pc_has_contextual_action(assets, selected_pc_id, PA::Tie) {
            return RHMOUSE_TIE;
        }

        // 5. Unconscious + RESUSCITATE + same camp → wake up
        if unconscious
            && self.selected_pc_has_contextual_action(assets, selected_pc_id, PA::Resuscitate)
        {
            let same_camp = match entity {
                Entity::Soldier(s) => s.soldier.cached_camp == crate::element::Camp::Royalists,
                _ => false,
            };
            if is_pc || same_camp {
                return RHMOUSE_WAKE_UP;
            }
        }

        // 6. Dead/unconscious + carry ability + not already carried
        //    → GET_YES (or GET_NO if heavy).
        if (dead || unconscious)
            && posture != Posture::Carried
            && self.selected_pc_can_carry(assets, selected_pc_id)
        {
            let is_heavy = match entity {
                Entity::Soldier(s) => assets
                    .profile_manager
                    .get_soldier(s.soldier.soldier_profile_index)
                    .map(|p| p.heavy)
                    .unwrap_or(false),
                _ => false,
            };
            return if is_heavy {
                RHMOUSE_GET_NO
            } else {
                RHMOUSE_GET_YES
            };
        }

        // The fall-through path is unreachable when `is_entity_focusable`
        // for Focus::Use was true; return the default arrow defensively.
        RHMOUSE_DEFAULT
    }

    /// Choose the mouse cursor for a focused PC under Focus::Select.
    ///
    /// `Focus::Select` only ever fires for PCs that pass the
    /// selectability test (alive, conscious, not in coma), so the
    /// dispatch reduces to two cases:
    ///
    /// - Posture is `HelpingToClimb` and the selected PC has the Jump
    ///   contextual action → `RHMOUSE_SHORT_LEG` (climb on shoulders)
    /// - Otherwise → `RHMOUSE_DEFAULT` (the regular arrow stays put;
    ///   left-click switches selection)
    ///
    /// The dead/unconscious/in-coma branches are gated out by the
    /// selectability check upstream and so are not reachable here.
    /// `selected_pc_id` is `None` when no PC is currently selected
    /// (initial select case).
    pub fn choose_select_cursor(
        &self,
        assets: &LevelAssets,
        entity_id: EntityId,
        selected_pc_id: Option<EntityId>,
    ) -> i32 {
        use crate::element::Posture;
        use crate::profiles::Action as PA;
        use crate::resource_ids::*;

        let entity = match self.get_entity(entity_id) {
            Some(e) => e,
            None => return RHMOUSE_DEFAULT,
        };

        if entity.element_data().posture == Posture::HelpingToClimb
            && self.selected_pc_has_contextual_action(assets, selected_pc_id, PA::Jump)
        {
            return RHMOUSE_SHORT_LEG;
        }

        RHMOUSE_DEFAULT
    }

    /// Choose the mouse cursor for a focused pickup-style object.
    ///
    /// Decision ladder:
    ///
    /// ```text
    /// if (recording_macro && pc_has(assoc_action))
    ///     → RHMOUSE_GET_YES    // macro-record override
    /// else if (!is_takable(pc))
    ///     → RHMOUSE_GET_NO
    /// else if (is_unique)
    ///     → RHMOUSE_GET_YES
    /// else switch (quantity)
    ///     1..5 → RHMOUSE_GET_YES_1..5
    ///     _    → RHMOUSE_GET_YES  // overflow = plain hand
    /// ```
    ///
    /// Returns `RHMOUSE_GET_YES` / `RHMOUSE_GET_NO` /
    /// `RHMOUSE_GET_YES_N` resource ids. Falls back to plain
    /// `GET_YES` if the entity isn't an object (defensive —
    /// `choose_use_cursor` already gated this).
    pub fn choose_object_cursor(
        &self,
        assets: &LevelAssets,
        entity_id: EntityId,
        selected_pc_id: Option<EntityId>,
    ) -> i32 {
        use crate::resource_ids::*;

        let Some(entity) = self.get_entity(entity_id) else {
            return RHMOUSE_GET_YES;
        };
        let Some(obj) = entity.object_data() else {
            return RHMOUSE_GET_YES;
        };

        // Macro-recording override: if the PC owns the object's
        // associated action and the recording messenger is live, the
        // cursor shows YES unconditionally so the operator can capture
        // the pickup even if the PC's inventory is full.
        let recording = self.is_recording_macro();
        let pc_has_action = selected_pc_id
            .and_then(|id| self.get_entity(id))
            .and_then(|pc| pc.pc_data())
            .and_then(|pc| assets.profile_manager.get_character(pc.profile_index))
            .is_some_and(|profile| profile.has_action(obj.associated_action));
        if recording && pc_has_action {
            return RHMOUSE_GET_YES;
        }

        // Full inventory → NO cursor.
        let Some(pc_id) = selected_pc_id else {
            return RHMOUSE_GET_NO;
        };
        if !crate::engine::commands::is_pc_takable(self, assets, entity, pc_id) {
            return RHMOUSE_GET_NO;
        }

        // Unique items skip the per-quantity suffix.
        if obj.object_type.is_unique() {
            return RHMOUSE_GET_YES;
        }

        match obj.quantity {
            1 => RHMOUSE_GET_YES_1,
            2 => RHMOUSE_GET_YES_2,
            3 => RHMOUSE_GET_YES_3,
            4 => RHMOUSE_GET_YES_4,
            5 => RHMOUSE_GET_YES_5,
            _ => RHMOUSE_GET_YES,
        }
    }

    // ─── Bow / projectile range helpers ─────────────────────────

    /// `can_shoot_with_bow_at` for cursor selection.
    ///
    /// Dispatches to [`can_shoot_with_bow_at_point`] using the
    /// appropriate body anchor:
    ///
    /// * Human, non-leaning  → belt point
    /// * Human, leaning-out  → belt first; if only `Long` available,
    ///   retry eyes for `Normal`
    /// * Non-human in forest → raw position with forest flag (weaker
    ///   range checks, no LOS check)
    /// * Non-human in city   → raw position
    #[track_caller]
    pub fn can_shoot_with_bow_at(
        &self,
        assets: &LevelAssets,
        pc_id: EntityId,
        target_id: EntityId,
    ) -> (BowTarget, crate::weapons::ShootMode) {
        use crate::weapons::ShootMode;

        let target = match self.get_entity(target_id) {
            Some(e) => e,
            None => return (BowTarget::Invalid, ShootMode::Normal),
        };

        if target.is_human() {
            let target_posture = target.element_data().posture;
            let Some(belt) = target.compute_belt_point() else {
                tracing::warn!(
                    ?target_id,
                    "can_shoot_with_bow_at: human target missing belt hotspot"
                );
                return (BowTarget::Invalid, ShootMode::Normal);
            };

            if target_posture != crate::element::Posture::LeaningOut {
                return self.can_shoot_with_bow_at_point(assets, pc_id, belt, false);
            }

            // Leaning-out target: try belt first, then eyes as
            // fallback.
            let (status, shoot) = self.can_shoot_with_bow_at_point(assets, pc_id, belt, false);
            if status == BowTarget::Valid {
                if shoot == ShootMode::Long {
                    // Belt is only reachable as a long shot — see if a
                    // normal shot works for the head instead.
                    if let Some(eyes) = target.compute_eyes_point(None) {
                        let (s2, m2) = self.can_shoot_with_bow_at_point(assets, pc_id, eyes, false);
                        if s2 == BowTarget::Valid && m2 == ShootMode::Normal {
                            return (BowTarget::Valid, ShootMode::Normal);
                        }
                    }
                }
                return (status, shoot);
            }
            // Belt out of range/blocked — try the head.
            if let Some(eyes) = target.compute_eyes_point(None) {
                return self.can_shoot_with_bow_at_point(assets, pc_id, eyes, false);
            }
            return (status, shoot);
        }

        // Non-human target — animals/objects. C++ checks range/LOS
        // against `pTarget->GetPosition()`; the later shot trajectory
        // still aims at FX target center.
        let target_pos = target.element_data().position();
        let forest = self.world.weather.is_forest_level;
        self.can_shoot_with_bow_at_point(assets, pc_id, target_pos, forest)
    }

    /// 3D-point variant of `can_shoot_with_bow_at` for cursor
    /// selection and scripted bow shots.
    ///
    /// Uses the PC's 3D hand body point, applies a cone range check
    /// above the target or a cylinder check at/below, doubles the
    /// allowed radius for forest targets, picks the shoot mode via
    /// [`BowState::get_shoot_mode_for_distance`], and finally walks
    /// the engine sight obstacles to detect blocked LOS (skipped for
    /// forest targets).
    #[track_caller]
    pub fn can_shoot_with_bow_at_point(
        &self,
        assets: &LevelAssets,
        shooter_id: EntityId,
        target_point: crate::coordinates::WorldPoint3D,
        forest_target: bool,
    ) -> (BowTarget, crate::weapons::ShootMode) {
        use crate::weapons::{BowState, ShootMode};

        let shooter = match self.get_entity(shooter_id) {
            Some(e) => e,
            None => return (BowTarget::Invalid, ShootMode::Normal),
        };

        // Get the shooter's bow profile. C++ calls through
        // `RHElementActorHuman::CanShootWithBowAt`, so scripted NPC
        // archers must use their soldier bow profile rather than the
        // PC-only character-profile path.
        let Some((bow_profile_idx, _shooting_ability)) =
            self.bow_profile_and_ability(assets, shooter_id)
        else {
            let caller = std::panic::Location::caller();
            tracing::warn!(
                ?shooter_id,
                caller_file = caller.file(),
                caller_line = caller.line(),
                "can_shoot_with_bow_at_point: shooter missing bow profile"
            );
            return (BowTarget::Invalid, ShootMode::Normal);
        };
        let Some(bow_profile) = assets.profile_manager.get_bow(bow_profile_idx) else {
            tracing::warn!(
                ?shooter_id,
                bow_profile_idx,
                "can_shoot_with_bow_at_point: bow profile id missing"
            );
            return (BowTarget::Invalid, ShootMode::Normal);
        };

        // No ammo → invalid target.
        if !self.check_bow_ammo(shooter_id) {
            return (BowTarget::Invalid, ShootMode::Normal);
        }

        // Build a value object from the immutable bow profile so we can
        // reuse the shared range/shoot-mode helpers.  The mutable
        // per-PC state that matters here is ammo, checked above via the
        // inventory path; `BowState` carries no additional live cursor
        // targeting data in the Rust model.
        let bow_state = BowState::new(0, bow_profile, 0);
        let max_range = bow_state.get_max_range(bow_profile) as f32;

        // Compute 3D hand point for the shooter.
        let Some(hand_point) = shooter.compute_hand_point(None) else {
            tracing::warn!(
                ?shooter_id,
                "can_shoot_with_bow_at_point: shooter missing hand hotspot"
            );
            return (BowTarget::Invalid, ShootMode::Normal);
        };
        let hand_ground_y = bow_ground_y(hand_point);
        let target_ground_y = bow_ground_y(target_point);

        let rel_height = hand_point.z - target_point.z;

        // Range check (cone above / cylinder at-or-below). Forest
        // targets use a doubled radius.
        let in_range = {
            let dx = target_point.x - hand_point.x;
            let dy = (target_ground_y - hand_ground_y) * INVERSE_ASPECT_RATIO_PROJ;
            let mut radius = if rel_height > 0.0 {
                max_range + rel_height * THROW_ANGLE_BOW.tan()
            } else {
                max_range
            };
            if forest_target {
                radius *= 2.0;
            }
            dx * dx + dy * dy < radius * radius
        };

        if !in_range {
            return (BowTarget::OutOfRange, ShootMode::Long);
        }

        // Pick the shoot mode from 3D distance using the true 3D norm.
        let dx = target_point.x - hand_point.x;
        let dy = target_ground_y - hand_ground_y;
        let dz = target_point.z - hand_point.z;
        let dist_3d = (dx * dx + dy * dy + dz * dz).sqrt();
        let mut shoot_mode = bow_state.get_shoot_mode_for_distance(bow_profile, dist_3d);

        // Leaning-out shooter forces a DownShoot.
        let shooter_posture = shooter.element_data().posture;
        if shooter_posture == crate::element::Posture::LeaningOut {
            shoot_mode = ShootMode::Down;
        }

        // LOS obstacle check. Skipped for forest targets (treated as
        // always-clear).
        //
        // Uses a full 3D ray test against sight obstacles, with a +1
        // Z offset on the target to avoid false positives when the
        // target stands exactly on top of an obstacle surface.
        //
        // `compute_bow_point` adds a shoot-mode-specific Z offset
        // (+40 Normal, +50 Long, +40 plus a 20-unit forward XY shift
        // for Down) on top of the hand point so low parapets and
        // chest-high walls are cleared.
        if !forest_target {
            let direction = shooter.element_data().direction();
            let Some(sprite_hand_point) =
                crate::bow_shot::bow_sprite_hand_point(shooter, shoot_mode, direction)
            else {
                tracing::warn!(
                    ?shooter_id,
                    ?shoot_mode,
                    direction,
                    "can_shoot_with_bow_at_point: missing bow-point sprite hotspot"
                );
                return (BowTarget::Invalid, ShootMode::Normal);
            };
            let bow_point = crate::bow_shot::compute_bow_point(
                hand_point,
                shoot_mode,
                direction,
                sprite_hand_point,
            );
            let bow_3d = [bow_point.x, bow_point.y, bow_point.z];
            let target_3d = [target_point.x, target_point.y, target_point.z + 1.0];
            let blocked = !crate::sight_obstacle::is_reachable_3d(
                self.sight_obstacles(assets),
                bow_3d,
                target_3d,
                crate::sight_obstacle::SIGHTOBSTACLE_SOLID
                    | crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
            );
            if blocked {
                // Blocked sight upgrades to a long shot.
                shoot_mode = ShootMode::Long;
            }
        }

        (BowTarget::Valid, shoot_mode)
    }

    /// Compute and store a trajectory preview arc for long-distance shots.
    ///
    /// Called when the cursor hovers over a valid bow target. For
    /// `ShootMode::Long`, computes the ballistic arc and stores it in
    /// `trajectory_preview_points` for rendering. For other modes the
    /// trajectory is flat and no preview is shown.
    ///
    /// Handles arrows (bow long shots) and throwable projectiles
    /// (apple, stone, net, purse, wasp nest).
    pub fn compute_trajectory_preview(
        &self,
        assets: &LevelAssets,
        pc_id: crate::element::EntityId,
        target_id: crate::element::EntityId,
        shoot_mode: crate::weapons::ShootMode,
    ) -> TrajectoryPreview {
        // Compute target belt point (humans) / centre (FX targets).
        let target_point = match self.get_entity(target_id) {
            Some(t) if t.is_human() => {
                let Some(point) = t.compute_belt_point() else {
                    tracing::warn!(
                        ?target_id,
                        "compute_trajectory_preview: human target missing belt hotspot"
                    );
                    return TrajectoryPreview::Invalid;
                };
                point
            }
            Some(t) if t.is_fx_target() => {
                let Some(point) = t.compute_target_center() else {
                    tracing::warn!(
                        ?target_id,
                        "compute_trajectory_preview: FX target missing center hotspot"
                    );
                    return TrajectoryPreview::Invalid;
                };
                point
            }
            Some(t) => {
                tracing::warn!(
                    ?target_id,
                    kind = ?t.kind(),
                    "compute_trajectory_preview: unsupported bow preview target"
                );
                return TrajectoryPreview::Invalid;
            }
            None => return TrajectoryPreview::Invalid,
        };
        self.compute_trajectory_preview_to_point(
            assets,
            pc_id,
            target_point,
            shoot_mode,
            Some(target_id),
        )
    }

    /// Compute and store a trajectory preview arc for a jump line.
    ///
    /// Looks up the near + far jump-line midpoints and runs them
    /// through `compute_trajectory_jump`. Called from the jump-cursor
    /// branch of
    /// [`update_mouse`](crate::engine::Engine::update_mouse) when the
    /// cursor hovers a jump sector with a resolved `jump_line_idx`,
    /// returning the ghost-arc points that the host renderer draws on
    /// top of the sector.
    pub fn compute_jump_preview(&self, jump_line_idx: u32) -> TrajectoryPreview {
        let lines = &self.world.fast_grid.level.jump_lines;
        let line = match lines.get(jump_line_idx as usize) {
            Some(l) => l,
            None => return TrajectoryPreview::Invalid,
        };
        let assoc_idx = match line.associated_line_index {
            Some(i) => i,
            None => return TrajectoryPreview::Invalid,
        };
        let dest_line = match lines.get(assoc_idx as usize) {
            Some(l) => l,
            None => return TrajectoryPreview::Invalid,
        };

        // 3D midpoint of each jump line averages `point_a`/`point_b`
        // and `z_a`/`z_b`.
        let start = crate::coordinates::WorldPoint3D {
            x: 0.5 * (line.point_a.x + line.point_b.x),
            y: 0.5 * (line.point_a.y + line.point_b.y),
            z: 0.5 * (line.z_a + line.z_b),
        };
        let dest = crate::coordinates::WorldPoint3D {
            x: 0.5 * (dest_line.point_a.x + dest_line.point_b.x),
            y: 0.5 * (dest_line.point_a.y + dest_line.point_b.y),
            z: 0.5 * (dest_line.z_a + dest_line.z_b),
        };

        let layer = line.layer;
        let points_3d = crate::engine::jump::compute_trajectory_jump(start, dest);
        if points_3d.is_empty() {
            return TrajectoryPreview::Invalid;
        }
        // Jump trajectory uses 4 frames per segment (the fly-segment
        // tick interval). The preview renderer only reads `position`,
        // but keep the field populated for symmetry with the
        // ballistic path.
        let points: Vec<crate::element::TrajectoryPoint> = points_3d
            .into_iter()
            .map(|p| crate::element::TrajectoryPoint {
                position: p,
                time: 4,
            })
            .collect();
        TrajectoryPreview::ShowArc {
            points,
            start,
            crumpled: false,
            layer,
        }
    }

    /// Compute a trajectory preview to a ground-targeted 3D point.
    ///
    /// Used by the `Purse` / `WaspNest` cursor branches where the
    /// throw lands on the ground (or an obstacle roof) rather than on
    /// a specific entity. Resolves the 2D mouse position to a 3D
    /// point via
    /// [`crate::fast_find_grid::FastFindGrid::convert_2d_to_3d`] so
    /// the arc lands on upper-floor terrain correctly.
    pub fn compute_trajectory_preview_ground(
        &self,
        assets: &LevelAssets,
        pc_id: crate::element::EntityId,
        mouse_map: MapPoint,
    ) -> TrajectoryPreview {
        let target_3d = self.world.fast_grid.convert_2d_to_3d(
            mouse_map,
            crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
            self.sight_obstacles(assets),
        );
        let target_point = crate::coordinates::WorldPoint3D {
            x: target_3d.x,
            y: target_3d.y,
            z: target_3d.z,
        };
        let preview = self.compute_trajectory_preview_to_point(
            assets,
            pc_id,
            target_point,
            crate::weapons::ShootMode::Long,
            None,
        );

        // The purse preview is only valid when the projectile's
        // computed trajectory lands on a motion sector — the engine's
        // obstacle/ground impact logic leaves layer = -1 when nothing
        // valid is hit. WaspNest and Apple (the other ground-target
        // actions) skip this gate.
        if self.get_selected_action() == crate::profiles::Action::Purse
            && let TrajectoryPreview::ShowArc { points, .. } = &preview
            && let Some(last) = points.last()
        {
            let resolution = self
                .world
                .fast_grid
                .resolve_projectile_landing(last.position.to_map(), self.sight_obstacles(assets));
            if resolution.sector.is_none() || resolution.blocked_by_motion_obstacle {
                return TrajectoryPreview::Invalid;
            }
        }

        preview
    }

    /// Shared trajectory-preview pipeline used by both the entity-target
    /// and ground-target variants.  `target_entity` is forwarded to
    /// `will_hit_target` for the "arc only when the shot misses" gate —
    /// ground throws pass `None`, which makes the will-hit test fall
    /// through to a positional-only check.
    pub fn compute_trajectory_preview_to_point(
        &self,
        assets: &LevelAssets,
        pc_id: crate::element::EntityId,
        target_point: crate::coordinates::WorldPoint3D,
        shoot_mode: crate::weapons::ShootMode,
        target_entity: Option<crate::element::EntityId>,
    ) -> TrajectoryPreview {
        use crate::bow_shot;
        use crate::coordinates::{WorldPoint3D, WorldVec3D};
        use crate::weapons::ShootMode;

        // Determine mass and apex based on the selected action.
        // For bow, only preview long (arced) shots.
        let selected_action = self.get_selected_action();
        let (mass, apex_height_override) = match selected_action {
            crate::profiles::Action::Apple => (bow_shot::MASS_APPLE, Some(bow_shot::APEX_APPLE)),
            crate::profiles::Action::Stone => (bow_shot::MASS_STONE, Some(bow_shot::APEX_STONE)),
            crate::profiles::Action::Net => (bow_shot::MASS_NET, Some(bow_shot::APEX_NET)),
            crate::profiles::Action::Purse => (bow_shot::MASS_PURSE, Some(bow_shot::APEX_PURSE)),
            crate::profiles::Action::WaspNest => {
                (bow_shot::MASS_WASP_NEST, Some(bow_shot::APEX_WASP_NEST))
            }
            crate::profiles::Action::Bow => {
                if shoot_mode != ShootMode::Long {
                    return TrajectoryPreview::Invalid;
                }
                (bow_shot::MASS_ARROW_HIGH, None)
            }
            _ => return TrajectoryPreview::Invalid,
        };

        let pc = match self.get_entity(pc_id) {
            Some(e) => e,
            None => return TrajectoryPreview::Invalid,
        };

        // C++ preview code computes the source hand point using the
        // direction from the selected PC to the hovered destination for
        // every projectile action. Bow uses ComputeBowPoint(LongShoot);
        // throwables use their own throwing animation hand point.
        let direction = {
            let shooter_pos = pc.element_data().position();
            crate::position_interface::vector_to_sector_0_to_15_iso(
                target_point.x - shooter_pos.x,
                bow_ground_y(target_point) - bow_ground_y(shooter_pos),
            )
        };
        let throwable_source = |animation| {
            pc.compute_hand_point_for_posture(
                direction,
                animation,
                crate::element::Posture::Upright,
            )
        };
        let source_point = match selected_action {
            crate::profiles::Action::Bow => {
                let elevation = pc.position_iface().get_elevation();
                let shooter_pos = WorldPoint3D {
                    x: pc.element_data().position_map().x,
                    y: pc.element_data().position_map().y,
                    z: elevation,
                };
                let Some(sprite_hand_point) =
                    bow_shot::bow_sprite_hand_point(pc, ShootMode::Long, direction)
                else {
                    tracing::warn!(
                        ?pc_id,
                        direction,
                        "compute_trajectory_preview_to_point: missing long-shot bow-point sprite hotspot"
                    );
                    return TrajectoryPreview::Invalid;
                };
                bow_shot::compute_bow_point(
                    shooter_pos,
                    ShootMode::Long,
                    direction,
                    sprite_hand_point,
                )
            }
            crate::profiles::Action::Apple => {
                match throwable_source(crate::order::OrderType::ThrowingApple) {
                    Some(point) => point,
                    None => {
                        tracing::warn!(
                            ?pc_id,
                            direction,
                            "compute_trajectory_preview_to_point: missing apple hand hotspot"
                        );
                        return TrajectoryPreview::Invalid;
                    }
                }
            }
            crate::profiles::Action::Stone => {
                match throwable_source(crate::order::OrderType::ThrowingStone) {
                    Some(point) => point,
                    None => {
                        tracing::warn!(
                            ?pc_id,
                            direction,
                            "compute_trajectory_preview_to_point: missing stone hand hotspot"
                        );
                        return TrajectoryPreview::Invalid;
                    }
                }
            }
            crate::profiles::Action::Net => {
                match throwable_source(crate::order::OrderType::ThrowingNet) {
                    Some(point) => point,
                    None => {
                        tracing::warn!(
                            ?pc_id,
                            direction,
                            "compute_trajectory_preview_to_point: missing net hand hotspot"
                        );
                        return TrajectoryPreview::Invalid;
                    }
                }
            }
            crate::profiles::Action::Purse => {
                match throwable_source(crate::order::OrderType::ThrowingPurse) {
                    Some(point) => point,
                    None => {
                        tracing::warn!(
                            ?pc_id,
                            direction,
                            "compute_trajectory_preview_to_point: missing purse hand hotspot"
                        );
                        return TrajectoryPreview::Invalid;
                    }
                }
            }
            crate::profiles::Action::WaspNest => {
                match throwable_source(crate::order::OrderType::ThrowingWaspNest) {
                    Some(point) => point,
                    None => {
                        tracing::warn!(
                            ?pc_id,
                            direction,
                            "compute_trajectory_preview_to_point: missing wasp-nest hand hotspot"
                        );
                        return TrajectoryPreview::Invalid;
                    }
                }
            }
            _ => return TrajectoryPreview::Invalid,
        };

        let target_forecasted_movement = match (selected_action, target_entity) {
            (crate::profiles::Action::Bow, Some(target_id)) => self
                .get_entity(target_id)
                .filter(|target| target.is_human())
                .map(|target| target.position_iface().get_forecasted_movement()),
            (crate::profiles::Action::Stone, Some(target_id)) => self
                .get_entity(target_id)
                .filter(|target| target.is_npc())
                .map(|target| target.position_iface().get_forecasted_movement()),
            _ => None,
        };

        let dx = target_point.x - source_point.x;
        let dy = target_point.y - source_point.y;
        let dz = target_point.z - source_point.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        let apex_height = apex_height_override.unwrap_or_else(|| (distance / 10.0).max(1.0));

        let direction_vec = WorldVec3D {
            x: dx,
            y: dy,
            z: dz,
        };
        let flight_time = if selected_action == crate::profiles::Action::Stone {
            1
        } else {
            0
        };
        let velocity = bow_shot::compute_initial_throw_velocity(
            direction_vec,
            apex_height,
            mass,
            flight_time,
            target_forecasted_movement,
        );

        // Compute trajectory with obstacle checking.
        let Some(layer) = self.get_entity(pc_id).map(|e| e.element_data().layer()) else {
            tracing::warn!(
                ?pc_id,
                "compute_trajectory_preview_to_point: shooter missing for layer lookup"
            );
            return TrajectoryPreview::Invalid;
        };
        let obstacle_check = bow_shot::TrajectoryObstacleCheck {
            fast_find_grid: &self.world.fast_grid,
            layer,
            sight_obstacles: self.sight_obstacles(assets),
            water_zones: Some(&assets.water_zones),
        };
        let bow_fx_forest_magic_preview = selected_action == crate::profiles::Action::Bow
            && self.world.weather.is_forest_level
            && target_entity
                .and_then(|id| self.get_entity(id))
                .is_some_and(|target| target.is_fx_target());
        let trajectory = bow_shot::compute_trajectory_ballistic(
            source_point,
            velocity,
            mass,
            false,
            if bow_fx_forest_magic_preview {
                None
            } else {
                Some(&obstacle_check)
            },
        );

        // Net on Easy difficulty: predict whether the net would
        // crumple at its landing point and surface that as the arc
        // colour. Outside Net/Easy, fall through to the generic
        // "will-hit → HitNoArc, miss → crumpled arc" branch used by
        // arrows/stones/purses.
        if selected_action == crate::profiles::Action::Net {
            let easy = crate::player_profile::PlayerProfileManager::global()
                .as_ref()
                .and_then(|mgr| mgr.get_active())
                .map(|p| p.difficulty)
                .unwrap_or(crate::player_profile::DifficultyLevel::Medium)
                == crate::player_profile::DifficultyLevel::Easy;
            if easy {
                let landing = trajectory
                    .last()
                    .map(|p| p.position)
                    .unwrap_or(target_point);
                let crumpled = self.predict_net_crumple_at(assets, landing, layer);
                return TrajectoryPreview::ShowArc {
                    points: trajectory,
                    start: source_point,
                    crumpled,
                    layer,
                };
            }
            // Medium / Hard: net arc always renders cyan (no crumple
            // affordance).
            return TrajectoryPreview::ShowArc {
                points: trajectory,
                start: source_point,
                crumpled: false,
                layer,
            };
        }

        // The original bow hover path calls IsValidTrajectory from
        // the cursor branch before RHCOMMAND_SHOOT_BOW is dispatched.
        // Keep the same timing here: long bow shots draw their arc
        // while the player is choosing a target, with the crumpled
        // colour reserved for predicted misses.
        let will_hit = bow_shot::will_hit_target(&trajectory, source_point, target_point);
        if selected_action == crate::profiles::Action::Bow {
            TrajectoryPreview::ShowArc {
                points: trajectory,
                start: source_point,
                crumpled: !will_hit,
                layer,
            }
        } else if will_hit {
            TrajectoryPreview::HitNoArc
        } else {
            TrajectoryPreview::ShowArc {
                points: trajectory,
                start: source_point,
                crumpled: true,
                layer,
            }
        }
    }

    /// Projectile range check for cursor selection.
    ///
    /// `target_entity`: when `Some`, the target's body anchor is used
    /// — eyes point for humans (Apple/Stone), centre for FX targets.
    /// When `None`, `target_pos` is lifted to a 3D point via
    /// `fast_grid.convert_2d_to_3d` against the
    /// `SIGHTOBSTACLE_PROJECTION_AREA` set, so a ground click landing
    /// on an obstacle top resolves to the obstacle-surface elevation
    /// rather than `z = 0`.
    ///
    /// Uses the PC's 3D hand point and a cone (when above target) or
    /// sphere (same/below) range check.
    pub fn is_in_range_for_projectile(
        &self,
        assets: &LevelAssets,
        pc_id: EntityId,
        target_pos: crate::coordinates::MapPoint,
        action: crate::profiles::Action,
        target_entity: Option<EntityId>,
    ) -> bool {
        let pc = match self.get_entity(pc_id) {
            Some(e) => e,
            None => return false,
        };

        let throw_radius = match action {
            crate::profiles::Action::Apple => THROW_DISTANCE_APPLE,
            crate::profiles::Action::Stone => THROW_DISTANCE_STONE,
            crate::profiles::Action::Purse => THROW_DISTANCE_PURSE,
            crate::profiles::Action::Net => THROW_DISTANCE_NET,
            crate::profiles::Action::WaspNest => THROW_DISTANCE_WASP_NEST,
            _ => return false,
        };

        // Compute 3D hand point for the thrower.
        let Some(hand_point) = pc.compute_hand_point(None) else {
            tracing::warn!(
                ?pc_id,
                ?action,
                "Projectile range check failed: thrower missing hand hotspot"
            );
            return false;
        };

        // Resolve the 3D target point:
        // - human target → eyes point
        // - FX target    → centre
        // - no entity    → 2d→3d convert against the projection-area
        //                  obstacles so the cursor lands on the
        //                  upper-floor surface, not z=0 ground.
        let ground_3d = || {
            let p3d = self.world.fast_grid.convert_2d_to_3d(
                crate::coordinates::MapPoint::new(target_pos.x, target_pos.y),
                crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
                self.sight_obstacles(assets),
            );
            crate::coordinates::WorldPoint3D {
                x: p3d.x,
                y: p3d.y,
                z: p3d.z,
            }
        };
        let target_3d = if let Some(tid) = target_entity {
            if let Some(target) = self.get_entity(tid) {
                if target.is_human() {
                    let Some(point) = target.compute_eyes_point(None) else {
                        tracing::warn!(
                            ?pc_id,
                            target = ?tid,
                            ?action,
                            "Projectile range check failed: human target missing eyes hotspot"
                        );
                        return false;
                    };
                    point
                } else if target.is_fx_target() {
                    // FX target: lift z by half the sprite height to
                    // land mid-sprite.
                    let Some(point) = target.compute_target_center() else {
                        tracing::warn!(
                            ?pc_id,
                            target = ?tid,
                            ?action,
                            "Projectile range check failed: FX target missing center hotspot"
                        );
                        return false;
                    };
                    point
                } else {
                    tracing::warn!(
                        ?pc_id,
                        target = ?tid,
                        kind = ?target.kind(),
                        ?action,
                        "Projectile range check failed: unsupported target kind"
                    );
                    return false;
                }
            } else {
                tracing::warn!(
                    ?pc_id,
                    target = ?tid,
                    ?action,
                    "Projectile range check failed: target entity missing"
                );
                return false;
            }
        } else {
            ground_3d()
        };

        let rel_height = hand_point.z - target_3d.z;

        if rel_height > 0.0 {
            // Thrower is above target — cone range check; radius
            // expands with height × tan(angle).
            let dx = target_3d.x - hand_point.x;
            let dy = (target_3d.y - hand_point.y) * INVERSE_ASPECT_RATIO_PROJ;
            let cone_radius = throw_radius + rel_height * THROW_ANGLE_PROJECTILE.tan();
            dx * dx + dy * dy < cone_radius * cone_radius
        } else {
            // Same level or below — sphere range check.
            let dx = target_3d.x - hand_point.x;
            let dy = (target_3d.y - hand_point.y) * INVERSE_ASPECT_RATIO_PROJ;
            let dz = target_3d.z - hand_point.z;
            dx * dx + dy * dy + dz * dz < throw_radius * throw_radius
        }
    }

    /// Compute bow accuracy opacity for the mouse cursor.
    ///
    /// Returns a value 0–75 representing the cursor shadow intensity.
    /// The caller clamps: `max(MOUSE_OPACITY_DEFAULT, result)`.
    pub fn calculate_shooting_level(
        &self,
        assets: &LevelAssets,
        pc_id: EntityId,
        target_pos: crate::coordinates::MapPoint,
    ) -> u16 {
        let pc = match self.get_entity(pc_id) {
            Some(e) => e,
            None => return 0,
        };

        // Use the long-shoot range unconditionally, regardless of
        // whether the bow advertises a long-shoot capability — every
        // bow profile populates the long-shoot range.
        let long_range = pc
            .pc_data()
            .and_then(|pc_data| assets.profile_manager.get_character(pc_data.profile_index))
            .and_then(|cp| assets.profile_manager.get_bow(cp.shooting_weapon_id))
            .map(|bp| bp.long_shoot.range)
            .unwrap_or_else(|| {
                panic!(
                    "calculate_shooting_level: PC {:?} has no bow profile",
                    pc_id
                )
            }) as f32;

        let pc_pos = pc.element_data().position_map();
        let dx = target_pos.x - pc_pos.x;
        let dy = target_pos.y - pc_pos.y;
        let distance = (dx * dx + dy * dy).sqrt();

        let ratio = distance / long_range;
        if ratio >= 1.0 {
            75
        } else {
            (75.0 * ratio) as u16
        }
    }

    // Safe: read-only — scans `mission_script.game_host.patches` and
    // returns the owning patch index; performs no engine mutation, so
    // stays `&self` even though called from the host cursor path.
    pub fn find_patch_for_grid_sector(
        &self,
        sector_idx: crate::fast_find_grid::SectorIndex,
    ) -> Option<u32> {
        let game_host = self.mission_script.as_ref()?.game_host()?;
        let raw = u32::from(sector_idx);
        let pos = game_host.patches.iter().position(|p| {
            p.old_sector_indices.contains(&raw) || p.new_sector_indices.contains(&raw)
        });
        match pos {
            Some(i) => Some(i as u32),
            None => panic!(
                "patch sector {sector_idx} has no owning patch in \
                 GameHost.patches — level data inconsistency"
            ),
        }
    }

    /// Find any patch related to the given door, mirroring C++'s
    /// `RHFastFindGrid::GetPatch(pDoor)`.  Checks both directions of
    /// the door↔patch wiring:
    ///
    /// 1. `door.patch_index` — set when the patch is `door_triggered`
    ///    (opening this door fires the patch).
    /// 2. `patch.door_indices` — populated when the patch is
    ///    `triggers_door` (applying the patch swaps this door's rights).
    ///
    /// Used by rendering to defer a door's polygon to the patch FX
    /// path on either side of the wiring.  Returns `None` when no
    /// mission script / game host is loaded.
    pub fn find_patch_for_door(&self, door_idx: u32) -> Option<u32> {
        let game_host = self.mission_script.as_ref()?.game_host()?;
        // Fast path: door_triggered link cached on the door.
        if let Some(door) = game_host.doors.get(door_idx as usize)
            && let Some(p) = door.patch_index
        {
            return Some(u32::from(p));
        }
        // Fallback: scan triggers_door patches' door lists.
        game_host
            .patches
            .iter()
            .position(|p| p.door_indices.contains(&door_idx))
            .map(|i| i as u32)
    }

    // ─── ChooseMousePointerForDoor ─────────────────────────────

    // Safe: read-only — derives the cursor id from patch / door lock
    // state and selected PC auth flags. Performs no engine mutation,
    // so stays `&self` even though called from the host cursor path.
    pub fn choose_door_cursor(&self, door_idx: Option<u32>, patch_idx: Option<u32>) -> i32 {
        use crate::resource_ids::*;

        // No door → patch-lock fallback.
        let door_idx = match door_idx {
            Some(idx) => idx,
            None => {
                // Treat a missing patch as a data error and panic
                // rather than silently returning a made-up cursor
                // (see CLAUDE.md no-fake-data rule).
                let patch_idx = patch_idx.expect(
                    "choose_door_cursor: no door and no patch — selected patch must be set",
                );
                let patch_locked = self
                    .mission_script
                    .as_ref()
                    .and_then(|s| s.game_host())
                    .and_then(|h| h.patches.get(patch_idx as usize))
                    .unwrap_or_else(|| panic!("choose_door_cursor: patch {patch_idx} not found"))
                    .is_locked();
                return if patch_locked {
                    RHMOUSE_DOOR_NO
                } else {
                    RHMOUSE_DOOR_YES
                };
            }
        };

        // Snapshot door state to avoid borrow conflicts with entity access.
        let door_state = self
            .mission_script
            .as_ref()
            .and_then(|s| s.game_host())
            .and_then(|h| h.doors.get(door_idx as usize))
            .map(|d| {
                (
                    d.is_locked_pc(),
                    d.is_unlockable(),
                    d.has_special_authorisation(),
                    d.authorised_pc_direct,
                    d.authorised_pc_indirect,
                    d.sector_in,
                    d.sector_out,
                )
            });

        let (
            locked_pc,
            unlockable,
            has_special_auth,
            auth_direct,
            auth_indirect,
            sector_in,
            sector_out,
        ) = match door_state {
            Some(s) => s,
            None => return RHMOUSE_DOOR_NO,
        };

        let mut authorized = false;
        let mut lockpick = false;

        if has_special_auth {
            // Check if any selected PC has special authorisation
            // in the direction they'd be passing through.
            for &pc_id in &self.players.seats[0].selection {
                if let Some(entity) = self.get_entity(pc_id) {
                    let pc_sector = entity.element_data().sector();
                    let auth_bit = entity.actor_auth_info().pc_auth_bit;
                    // PC in sector_in → going indirect (out)
                    if pc_sector
                        == crate::position_interface::SectorHandle::new(u16::from(sector_in))
                        && (auth_indirect & auth_bit) != 0
                    {
                        authorized = true;
                    }
                    // PC in sector_out → going direct (in)
                    if pc_sector
                        == crate::position_interface::SectorHandle::new(u16::from(sector_out))
                        && (auth_direct & auth_bit) != 0
                    {
                        authorized = true;
                    }
                }
            }
        } else if locked_pc {
            // Lockpick selection requires EXACTLY one selected PC
            // and that single PC must carry the lockpick action; on
            // multi-selections including a lockpicker the cursor is
            // the plain door cursor, not the lockpick cursor.
            let has_lockpick_pc = matches!(
                self.players.seats[0].selection.as_slice(),
                [pc_id] if self.get_entity(*pc_id)
                    .map(|e| e.actor_auth_info().has_lockpick)
                    .unwrap_or(false)
            );
            if unlockable && has_lockpick_pc {
                authorized = true;
                lockpick = true;
            } else if has_lockpick_pc {
                lockpick = true;
            }
        } else {
            authorized = true;
        }

        if authorized {
            if lockpick {
                RHMOUSE_LOCKPICK_YES
            } else {
                RHMOUSE_DOOR_YES
            }
        } else if lockpick {
            RHMOUSE_LOCKPICK_NO
        } else {
            RHMOUSE_DOOR_NO
        }
    }

    /// Per-frame re-orientation of selected PCs toward the mouse map
    /// position for aim/throw/help-climb/beggar actions.
    ///
    /// Called once per frame from `host_mouse::update_mouse` via
    /// `PlayerCommand::PerformOrientation` so replay / rollback
    /// reproduce the same direction updates. Gated on: not currently
    /// recording a QA macro; at least one PC selected. Per-PC, the
    /// direction is only updated when the PC's action-state permits
    /// (e.g. `AimingWithBow` for Bow, not `Throwing*` animations for
    /// throws).
    ///
    /// The action-specific animation-level gating (matching the exact
    /// aim/throw anim) is relaxed to `ActionState` matching; the
    /// sub-frame divergence is invisible in practice because the aim
    /// state always pairs with the aim animation.
    pub(crate) fn perform_orientation(
        &mut self,
        assets: &LevelAssets,
        mouse_map: crate::coordinates::MapPoint,
    ) {
        use crate::profiles::Action;
        use crate::sight_obstacle::{SIGHTOBSTACLE_MOUSE, SIGHTOBSTACLE_PROJECTION_AREA};

        if self.is_recording_macro() {
            return;
        }
        if self.players.seats[0].selection.is_empty() {
            return;
        }

        let selected_action = self.get_selected_action();
        // Use the current draw order for topmost-hit focus
        // resolution. The host doesn't pass its cached draw order
        // into command handlers (it's render-cache, not sim state),
        // so the branches that need it recompute locally — cheap
        // given this runs at most once per frame.
        let need_draw_order = matches!(
            selected_action,
            Action::Bow
                | Action::Apple
                | Action::Stone
                | Action::Net
                | Action::WaspNest
                | Action::Purse
        );
        let draw_order = need_draw_order.then(|| self.compute_display_order());

        match selected_action {
            Action::Bow => {
                let focused = self.find_focusable_entity(
                    assets,
                    &draw_order.as_ref().unwrap().ids,
                    mouse_map,
                    crate::element::Focus::Bow,
                );
                // If the focused target is a human, aim at its belt
                // point (so the arc computation uses the torso
                // height, not the foot position). Missing body points
                // are data errors, not a reason to aim at the feet.
                // With no focused element, project the 2D mouse onto
                // the sight-obstacle grid.
                let target_3d = match focused.and_then(|id| self.get_entity(id)) {
                    Some(e) if e.is_human() => {
                        let Some(point) = e.compute_belt_point() else {
                            tracing::warn!(
                                entity = ?focused,
                                "perform_orientation: bow human target missing belt hotspot"
                            );
                            return;
                        };
                        point
                    }
                    Some(e) => e.element_data().position(),
                    None => self.world.fast_grid.convert_2d_to_3d(
                        mouse_map,
                        SIGHTOBSTACLE_MOUSE,
                        self.sight_obstacles(assets),
                    ),
                };
                self.turn_selected_pcs_in_bow_aim(assets, target_3d);
            }
            Action::Apple | Action::Stone | Action::Net | Action::WaspNest | Action::Purse => {
                let focus = match selected_action {
                    Action::Apple => crate::element::Focus::Apple,
                    _ => crate::element::Focus::Stone,
                };
                let focused = self.find_focusable_entity(
                    assets,
                    &draw_order.as_ref().unwrap().ids,
                    mouse_map,
                    focus,
                );
                let target_3d = focused
                    .and_then(|id| self.get_entity(id).map(|e| e.element_data().position()))
                    .unwrap_or_else(|| {
                        self.world.fast_grid.convert_2d_to_3d(
                            mouse_map,
                            SIGHTOBSTACLE_PROJECTION_AREA,
                            self.sight_obstacles(assets),
                        )
                    });
                let ground_pt = GroundPoint {
                    x: target_3d.x,
                    y: target_3d.y,
                };
                self.turn_selected_pcs_throw(ground_pt);
            }
            Action::HelpToClimb => {
                self.turn_selected_pcs_help_climb(mouse_map);
            }
            Action::Beggar => {
                self.turn_selected_pcs_beggar(mouse_map);
            }
            _ => {}
        }
    }

    /// Bow-aim branch. Auto-launches `RAISE_BOW` / `LOWER_BOW`
    /// sequences when the resolved shoot mode crosses the arc
    /// threshold.
    ///
    /// Per-PC gates:
    /// * Action state is `AimingWithBow` or `AimingWithBowUp`.
    /// * Current animation is one of `AimingWithBow`,
    ///   `AimingWithBowUp`, `AimingWithBowAnonymous`,
    ///   `AimingWithBowUpAnonymous` — reading the actor's current
    ///   sequence order from the sequence manager.
    /// * No pending `Command::ShootBow` in the actor's to-go queue
    ///   (via `SequenceManager::element_is_about_to_be_launched`).
    ///   `elements_to_go` covers the pending-shoots-not-yet-started
    ///   case; once a shot is in flight the action-state gate above
    ///   already rejects it.
    fn turn_selected_pcs_in_bow_aim(
        &mut self,
        assets: &LevelAssets,
        target_3d: crate::coordinates::WorldPoint3D,
    ) {
        use crate::element::{ActionState, Command};
        use crate::order::OrderType;
        use crate::position_interface::vector_to_sector_0_to_15_iso;
        use crate::weapons::ShootMode;

        let ground_pt = GroundPoint {
            x: target_3d.x,
            y: bow_ground_y(target_3d),
        };

        let ids = self.players.seats[0].selection.clone();
        let mut raise_lower: Vec<(EntityId, Command)> = Vec::new();

        for pc_id in ids {
            // Read-only gates first so we can consult `self` without
            // holding a mutable borrow of `self.world.entities`.
            let Some(entity) = self.get_entity(pc_id) else {
                continue;
            };
            let Some(actor) = entity.actor_data() else {
                continue;
            };
            let action_state = actor.action_state;
            if !matches!(
                action_state,
                ActionState::AimingWithBow | ActionState::AimingWithBowUp
            ) {
                continue;
            }

            let current_anim = self
                .sequence_manager
                .current_order_for_actor(pc_id)
                .map(|(_, _, o)| o.order_type);
            if !matches!(
                current_anim,
                Some(
                    OrderType::AimingWithBow
                        | OrderType::AimingWithBowUp
                        | OrderType::AimingWithBowAnonymous
                        | OrderType::AimingWithBowUpAnonymous
                )
            ) {
                continue;
            }

            if self
                .sequence_manager
                .element_is_about_to_be_launched(pc_id, Command::ShootBow)
            {
                continue;
            }

            let pos = entity.element_data().position();
            let dx = ground_pt.x - pos.x;
            let dy = ground_pt.y - bow_ground_y(pos);

            // C++ turns toward ptFocusHotSpot before AimWithBowAt()
            // resolves Normal vs Long.  The bow LOS check uses the
            // shooter's current facing to compute the hand/bow point,
            // so keep this order or upward/long aim can be evaluated
            // from the previous direction.
            if (dx != 0.0 || dy != 0.0)
                && let Some(Entity::Pc(pc)) = self.world.entities.get_mut(pc_id)
            {
                pc.element
                    .set_direction_goal(vector_to_sector_0_to_15_iso(dx, dy));
                pc.element.sprite.position_iface.turn();
            }

            // C++ `AimWithBowAt` ignores the reachability return value
            // from `CanShootWithBowAt` and uses the resolved shoot type
            // anyway.  This still raises for out-of-range/blocked long
            // shots so the aiming pose tracks the hovered point.
            let (_bow_status, shoot_mode) =
                self.can_shoot_with_bow_at_point(assets, pc_id, target_3d, false);
            match (action_state, shoot_mode, current_anim) {
                (ActionState::AimingWithBow, ShootMode::Long, Some(OrderType::AimingWithBow)) => {
                    raise_lower.push((pc_id, Command::RaiseBow));
                }
                (
                    ActionState::AimingWithBowUp,
                    ShootMode::Normal,
                    Some(OrderType::AimingWithBowUp),
                ) => {
                    raise_lower.push((pc_id, Command::LowerBow));
                }
                _ => {}
            }
        }

        for (pc_id, cmd) in raise_lower {
            let elem = crate::sequence::SequenceElement::new(1, cmd, Some(pc_id));
            self.launch_element(elem);
        }
    }

    /// Throw-projectile branch (apple/stone/net/wasp-nest/purse):
    /// turn selected PCs not already playing a throw animation.
    fn turn_selected_pcs_throw(&mut self, ground_pt: GroundPoint) {
        use crate::order::OrderType;
        use crate::position_interface::vector_to_sector_0_to_15_iso;

        let ids = self.players.seats[0].selection.clone();
        for pc_id in ids {
            // Skip when a throw animation is the front order of the
            // actor's current sequence element.
            let current_anim = self
                .sequence_manager
                .current_order_for_actor(pc_id)
                .map(|(_, _, o)| o.order_type);
            if matches!(
                current_anim,
                Some(
                    OrderType::ThrowingApple
                        | OrderType::ThrowingStone
                        | OrderType::ThrowingNet
                        | OrderType::ThrowingWaspNest
                        | OrderType::ThrowingPurse
                )
            ) {
                continue;
            }
            let Some(entity) = self.world.entities.get_mut(pc_id) else {
                continue;
            };
            let Entity::Pc(pc) = entity else {
                continue;
            };
            let pos = pc.element.position_map();
            let dx = ground_pt.x - pos.x;
            let dy = ground_pt.y - pos.y;
            if dx == 0.0 && dy == 0.0 {
                continue;
            }
            // Delta is world-space (ground-position to 3d mouse
            // point); set the direction goal and rotate one step.
            pc.element
                .set_direction_goal(vector_to_sector_0_to_15_iso(dx, dy));
            pc.element.sprite.position_iface.turn();
        }
    }

    /// HelpClimb branch: carrier flips 180°, climber faces the goal.
    fn turn_selected_pcs_help_climb(&mut self, mouse_map: MapPoint) {
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType;
        use crate::position_interface::vector_to_sector_0_to_15;

        let ids = self.players.seats[0].selection.clone();
        for pc_id in ids {
            let Some(entity) = self.world.entities.get_mut(pc_id) else {
                continue;
            };
            let Entity::Pc(pc) = entity else {
                continue;
            };
            let pos = pc.element.position_map();
            let dx = mouse_map.x - pos.x;
            let dy = mouse_map.y - pos.y;
            if dx == 0.0 && dy == 0.0 {
                continue;
            }
            let posture = pc.element.posture;
            let action_state = pc.actor.action_state;
            let raw_dir = vector_to_sector_0_to_15(dx, dy);
            let transition_active = self
                .sequence_manager
                .current_order_for_actor(pc_id)
                .map(|(_, _, o)| {
                    matches!(
                        o.order_type,
                        OrderType::TransitionHelpingClimbingDown
                            | OrderType::TransitionHelpingClimbingUp
                    )
                })
                .unwrap_or(false);
            if posture == Posture::OnShoulders
                && matches!(action_state, ActionState::Waiting | ActionState::Bored)
                && !transition_active
            {
                // Carrier sets direction-goal only; the rotation is
                // performed by the animation (no `turn()`).
                pc.element.set_direction_goal((raw_dir + 8) & 15);
            } else if posture != Posture::OnShoulders && posture != Posture::HelpingToClimb {
                // Climber sets goal then advances one step toward it
                // per frame.
                pc.element.set_direction_goal(raw_dir);
                pc.element.sprite.position_iface.turn();
            }
        }
    }

    /// Beggar branch: upright + idle PCs face the mouse.
    fn turn_selected_pcs_beggar(&mut self, mouse_map: MapPoint) {
        use crate::element::{ActionState, Posture};
        use crate::position_interface::vector_to_sector_0_to_15;

        let ids = self.players.seats[0].selection.clone();
        for pc_id in ids {
            let Some(entity) = self.world.entities.get_mut(pc_id) else {
                continue;
            };
            let Entity::Pc(pc) = entity else {
                continue;
            };
            let pos = pc.element.position_map();
            let dx = mouse_map.x - pos.x;
            let dy = mouse_map.y - pos.y;
            if dx == 0.0 && dy == 0.0 {
                continue;
            }
            if pc.element.posture == Posture::Upright
                && matches!(
                    pc.actor.action_state,
                    ActionState::Waiting | ActionState::Bored
                )
            {
                // Set direction goal + rotate one step per frame.
                pc.element
                    .set_direction_goal(vector_to_sector_0_to_15(dx, dy));
                pc.element.sprite.position_iface.turn();
            }
        }
    }
}

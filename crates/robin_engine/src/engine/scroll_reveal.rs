//! Scroll-reveal flow.
//!
//! The beggar-gives-info interaction runs like this:
//!
//! 1. The PC pays a beggar (`Pay` command → `ReceivingPurse` →
//!    `WaitingWithPurse` animations).
//! 2. When `WaitingWithPurse` terminates, the civilian invokes
//!    `reveal_scrolls` on itself.
//! 3. `reveal_scrolls` iterates the beggar's current scroll set, calls
//!    `reveal_scroll` on each, funnels the results into the minimap
//!    (`set_highlighted` / `display_for_delayed_elements`), and has
//!    the beggar say the appropriate remark.
//!
//! Step 1 (the Pay/Receive animation chain) lives in `abilities.rs`
//! under `begin_pay` / `begin_receive_purse` and the phase-aware
//! dispatch in `tick_abilities`; the `ReceivePurseRevealing` handler
//! in `engine/combat.rs` invokes [`EngineInner::reveal_scrolls`] on the
//! `WaitingWithPurse` → transition boundary.

use serde::{Deserialize, Serialize};

use super::{EngineInner, LevelAssets};
use crate::coordinates::MapPoint;
use crate::element::{Entity, EntityId};

/// Scroll reveal status.  Persisted in `GameHost::scroll_status`
/// (keyed by actor script handle); the script natives
/// `GetScrollStatus` / `SetScrollStatus` read/write it directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum ScrollStatus {
    Invisible = 0,
    Visible = 1,
    Taken = 2,
    Opened = 3,
}

impl ScrollStatus {
    pub(crate) fn from_i32(v: i32) -> Self {
        match v {
            0 => Self::Invisible,
            1 => Self::Visible,
            2 => Self::Taken,
            3 => Self::Opened,
            _ => Self::Invisible,
        }
    }

    /// Custom-minimap-dot value that a scroll should expose for this
    /// status.  Visible/opened scrolls render with the default dot
    /// classification; everything else is hidden.
    fn custom_minimap_dot(self) -> u16 {
        match self {
            Self::Visible | Self::Opened => 1, // CUSTOM_DOT_NOT_CUSTOMIZED
            _ => 0,                            // CUSTOM_DOT_INVISIBLE
        }
    }
}

/// A scroll that needs to be replaced by an amulet at its next
/// `&mut LevelAssets` opportunity.  Captured at reveal time because
/// amulet entities need sprite loading that the sim tick can't do.
///
/// Drained by [`EngineInner::drain_pending_scroll_amulets`].  The
/// scroll's position, layer, sector, direction, obstacle, and material
/// are copied onto the spawned amulet.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PendingScrollAmulet {
    pub position_map: MapPoint,
    pub layer: u16,
    pub sector: Option<crate::position_interface::SectorHandle>,
    pub direction: i16,
    pub obstacle_index: Option<crate::position_interface::ObstacleHandle>,
    /// Footstep material captured from the scroll's `PositionInterface`.
    /// Without it the amulet would inherit the `GameMaterial::default()`
    /// sentinel instead of the scroll's plane material.
    pub material: crate::element::GameMaterial,
    /// The scroll whose reveal triggered this spawn.  Stays in the
    /// entity list with status `Taken`.
    pub replaces: EntityId,
}

/// What remark the beggar should say after a reveal attempt.  Fired
/// internally by [`EngineInner::reveal_scrolls`] via
/// `AiController::say_with_flags` on the beggar's AI state — callers
/// only see this value for telemetry / logging.
///
/// [`EngineInner::reveal_scrolls`]: EngineInner::reveal_scrolls
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeggarRemark {
    /// Revealed scrolls in the current set and more sets still carry
    /// revealable scrolls.
    GivesInfo,
    /// Revealed the last batch of scrolls; nothing left after this.
    GivesLastInfo,
    /// The current set had no revealable scrolls but future sets do —
    /// the beggar wants another payment.
    WantsMore,
    /// The beggar has no more info to share.
    Thanx,
    /// The cursor had already walked past the end of the scroll-set
    /// table when `reveal_scrolls` was invoked (beggar already gave
    /// everything).  This variant fires without the
    /// `EMERGENCY | ALWAYS` flags so it doesn't preempt other speech.
    ExhaustedThanx,
}

impl BeggarRemark {
    fn remark(self) -> crate::ai::Remark {
        use crate::ai::Remark;
        match self {
            Self::GivesInfo => Remark::CivBeggarGivesInfo,
            Self::GivesLastInfo => Remark::CivBeggarGivesLastInfo,
            Self::WantsMore => Remark::CivBeggarWantsMore,
            Self::Thanx | Self::ExhaustedThanx => Remark::CivBeggarThanx,
        }
    }

    /// Speech flags to pass to `say_with_flags`.  The "already
    /// exhausted" branch uses no flags; everything else uses
    /// `EMERGENCY | ALWAYS` so the beggar interrupts whatever they
    /// were saying and ignores the recently-said cooldown.
    fn speech_flags(self) -> crate::ai::SpeechFlags {
        use crate::ai::SpeechFlags;
        match self {
            Self::ExhaustedThanx => SpeechFlags::empty(),
            _ => SpeechFlags::EMERGENCY | SpeechFlags::ALWAYS,
        }
    }
}

impl EngineInner {
    // ─── Scroll status accessors ─────────────────────────────────

    /// Current status of a scroll entity.  Reads
    /// `GameHost::scroll_status` (the single source of truth for
    /// scroll state — script natives read/write it directly).
    /// Returns [`ScrollStatus::Invisible`] for scrolls that have no
    /// entry or when the mission script is unavailable.
    pub fn scroll_status(&self, scroll: EntityId) -> ScrollStatus {
        let handle = crate::natives::GameHost::actor_handle(scroll);
        let raw = self
            .scripts
            .mission
            .as_ref()
            .and_then(|s| s.game_host())
            .and_then(|gh| gh.scroll_status.get(&handle).copied())
            .unwrap_or(0);
        ScrollStatus::from_i32(raw)
    }

    /// Update a scroll's status and refresh its minimap dot.
    pub(crate) fn set_scroll_status(&mut self, scroll: EntityId, status: ScrollStatus) {
        let handle = crate::natives::GameHost::actor_handle(scroll);
        if let Some(script) = self.scripts.mission.as_mut()
            && let Some(gh) = script.game_host_mut()
        {
            gh.scroll_status.insert(handle, status as i32);
        }
        if let Some(entity) = self.get_entity_mut(scroll) {
            entity.element_data_mut().custom_minimap_dot = status.custom_minimap_dot();
        }
    }

    /// PC finished the Taking animation on a scroll:
    ///
    /// 1. Flip the taken flag and set status to `Opened`.
    /// 2. Force the BonusThree sprite row (the "scroll is open" idle).
    /// 3. If a script class is bound, invoke `IScrollScript::IsTaken(pc)`.
    ///    A non-zero return promotes status `Opened → Taken` and
    ///    refreshes the minimap dot.
    ///
    /// `scroll_handle` and `pc_handle` are actor script handles.
    pub(crate) fn take_scroll(&mut self, pc: EntityId, scroll: EntityId) {
        // Flip the taken flag.
        if let Some(entity) = self.get_entity_mut(scroll)
            && let Some(obj) = entity.object_data_mut()
        {
            obj.taken = true;
        }

        // Status → Opened (+ BonusThree animation hint).  The sprite
        // row is driven by `object.animation`.
        self.set_scroll_status(scroll, ScrollStatus::Opened);
        if let Some(entity) = self.get_entity_mut(scroll)
            && let Some(obj) = entity.object_data_mut()
        {
            obj.animation = crate::order::OrderType::BonusThree;
        }

        // IScrollScript::IsTaken(pPC) — per-scroll bound script.
        // Non-zero return advances status to Taken; zero keeps it at
        // Opened.  Scrolls with no bound class get `Ok(0)` and stay
        // at Opened.
        let scroll_handle = crate::natives::GameHost::actor_handle(scroll);
        let pc_handle = crate::natives::GameHost::actor_handle(pc);
        let queries = native_query_views!(self);
        let script_result = self
            .scripts
            .mission
            .as_mut()
            .map(|script| {
                script.call_scroll_function(scroll_handle, "IsTaken", &[pc_handle], queries)
            })
            .transpose();
        match script_result {
            Ok(Some(v)) if v != 0 => {
                self.set_scroll_status(scroll, ScrollStatus::Taken);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(?scroll, ?pc, "IScrollScript::IsTaken dispatch failed: {e}");
            }
        }
    }

    // ─── Revealability checks ────────────────────────────────────

    /// Whether a scroll is revealable — active and currently invisible.
    pub fn is_scroll_revealable(&self, assets: &LevelAssets, scroll_id: u16) -> bool {
        let Some(&eid) = assets.scroll_entity_ids.get(scroll_id as usize) else {
            return false;
        };
        let Some(entity) = self.get_entity(eid) else {
            return false;
        };
        if !entity.element_data().active {
            return false;
        }
        self.scroll_status(eid) == ScrollStatus::Invisible
    }

    /// Whether any scroll set from the beggar's current cursor forward
    /// contains a revealable scroll.
    pub fn are_there_revealable_scrolls(&self, assets: &LevelAssets, beggar: EntityId) -> bool {
        let Some(Entity::Civilian(c)) = self.get_entity(beggar) else {
            return false;
        };
        let Some(scroll_sets) = c.civilian.beggar_scroll_sets.as_ref() else {
            return false;
        };
        let start = c.civilian.current_scroll_set as usize;
        scroll_sets.iter().skip(start).any(|set| {
            set.iter()
                .any(|&sid| self.is_scroll_revealable(assets, sid))
        })
    }

    /// Whether this scroll is due to be replaced by an amulet on Easy
    /// when its presence flag is unset.
    fn is_scroll_to_be_replaced_by_amulet(&self, scroll: EntityId) -> bool {
        let difficulty = crate::player_profile::DifficultyLevel::current();
        if difficulty != crate::player_profile::DifficultyLevel::Easy {
            return false;
        }
        let Some(Entity::Scroll(s)) = self.get_entity(scroll) else {
            return false;
        };
        !s.presence[0] // presence[DifficultyLevel::Easy as usize]
    }

    // ─── Reveal ──────────────────────────────────────────────────

    /// Reveal a scroll by its global scroll ID (index into
    /// [`EngineInner::scroll_entity_ids`]).
    ///
    /// * On Easy difficulty with `presence[Easy] == false`, queues a
    ///   pending amulet to spawn later (needs `&mut LevelAssets` for
    ///   sprite loading — see [`EngineInner::drain_pending_scroll_amulets`])
    ///   and marks the original scroll `Taken`.
    /// * Otherwise sets the scroll to `Visible` so it draws on the
    ///   world and on the minimap.
    ///
    /// Returns the entity that the minimap should highlight — the
    /// original scroll in both branches, since the amulet (when
    /// spawned) inherits its `position_map` via
    /// [`PendingScrollAmulet::position_map`].  Returns `None` if the
    /// scroll is not revealable.
    pub fn reveal_scroll(&mut self, assets: &LevelAssets, scroll_id: u16) -> Option<EntityId> {
        if !self.is_scroll_revealable(assets, scroll_id) {
            return None;
        }
        let eid = *assets.scroll_entity_ids.get(scroll_id as usize)?;

        if self.is_scroll_to_be_replaced_by_amulet(eid) {
            let (pos, layer, sector, direction, obstacle_index, material) = {
                let e = self.get_entity(eid)?;
                let ed = e.element_data();
                (
                    ed.position_map(),
                    ed.layer(),
                    ed.sector(),
                    ed.direction(),
                    ed.obstacle_index(),
                    ed.material(),
                )
            };
            self.orders
                .pending_scroll_amulets
                .push(PendingScrollAmulet {
                    position_map: pos,
                    layer,
                    sector,
                    direction,
                    obstacle_index,
                    material,
                    replaces: eid,
                });
            self.set_scroll_status(eid, ScrollStatus::Taken);
        } else {
            self.set_scroll_status(eid, ScrollStatus::Visible);
        }

        Some(eid)
    }

    // ─── Beggar flow ─────────────────────────────────────────────

    /// Run one iteration of the beggar's reveal flow.
    ///
    /// On a successful reveal the beggar's `current_scroll_set`
    /// advances, the minimap's delayed-highlight queue fills with the
    /// revealed scrolls, and the map opens (centred if it was closed).
    /// The beggar's speech cue is fired directly here (setting
    /// `AiController::current_remark`); `process_npc_speech` picks it
    /// up later in the tick and forwards it to the sound queue.
    /// `beggar_dont_talk_counter` is also bumped to 3 frames on the
    /// civilian's friendly-AI state so remarks don't stack.
    ///
    /// Returns the chosen [`BeggarRemark`] for logging / telemetry —
    /// `None` if the entity isn't a civilian / beggar.
    pub fn reveal_scrolls(
        &mut self,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
        beggar: EntityId,
    ) -> Option<BeggarRemark> {
        // Snapshot the beggar's current scroll set before mutating.
        let Some(Entity::Civilian(c)) = self.get_entity(beggar) else {
            tracing::warn!(?beggar, "reveal_scrolls: entity is not a civilian");
            return None;
        };
        let Some(scroll_sets) = c.civilian.beggar_scroll_sets.as_ref() else {
            tracing::warn!(?beggar, "reveal_scrolls: civilian is not a beggar");
            return None;
        };
        let current_idx = c.civilian.current_scroll_set as usize;
        let set_count = scroll_sets.len();

        // When the cursor has walked off the end, the beggar only says
        // "thanx" and nothing else happens (no cooldown bump either).
        if current_idx >= set_count {
            self.say_beggar_remark(beggar, BeggarRemark::ExhaustedThanx);
            return Some(BeggarRemark::ExhaustedThanx);
        }
        let current_set = scroll_sets[current_idx].clone();

        // Count revealable scrolls in the current set without mutating.
        let revealable_count = current_set
            .iter()
            .filter(|&&sid| self.is_scroll_revealable(assets, sid))
            .count();

        let remark = if revealable_count != 0 {
            // Reveal each scroll and queue it onto the minimap.
            for &scroll_id in &current_set {
                if let Some(revealed) = self.reveal_scroll(assets, scroll_id) {
                    display.minimap.set_highlighted(revealed.index());
                }
            }
            let screen = Self::director_camera_view_size();
            let sw = screen.x;
            let sh = screen.y;
            display.minimap.display_for_delayed_elements(sw, sh);

            // The "revealable scrolls remain?" check runs BEFORE the
            // cursor advances.  The scrolls just revealed in
            // `current_set` no longer count (their status is now
            // `Visible`), so the check reports on future sets only.
            if self.are_there_revealable_scrolls(assets, beggar) {
                BeggarRemark::GivesInfo
            } else {
                BeggarRemark::GivesLastInfo
            }
        } else if self.are_there_revealable_scrolls(assets, beggar) {
            BeggarRemark::WantsMore
        } else {
            BeggarRemark::Thanx
        };

        // Fire the speech cue on the beggar's AI controller.
        self.say_beggar_remark(beggar, remark);

        // Common tail — unconditionally bump the chat cooldown and
        // advance the scroll-set cursor in both the revealable and
        // non-revealable branches (but NOT in the exhausted-thanx
        // branch, which returns early above).
        if let Some(entity) = self.get_entity_mut(beggar)
            && let Some(ai) = entity.friendly_ai_mut()
        {
            ai.set_beggar_dont_talk_counter(3);
        }
        if let Some(Entity::Civilian(c)) = self.get_entity_mut(beggar) {
            c.civilian.current_scroll_set = c.civilian.current_scroll_set.saturating_add(1);
        }

        Some(remark)
    }

    /// Fire a beggar speech cue on the entity's AI controller.  The
    /// actual sound dispatch happens in `process_npc_speech` later in
    /// the tick.
    fn say_beggar_remark(&mut self, beggar: EntityId, remark: BeggarRemark) {
        if let Some(entity) = self.get_entity_mut(beggar)
            && let Some(ai) = entity.ai_controller_mut()
        {
            ai.say_with_flags(remark.remark(), remark.speech_flags());
        }
    }

    // ─── Deferred amulet spawn ───────────────────────────────────

    /// Drain amulet-spawn requests queued by [`Self::reveal_scroll`].
    /// Runs from `perform_hourglass` so sim-state mutation stays in
    /// the replayed tick. The amulet sprite
    /// (`BONUS_FourLeavedClover` / `"BONUS Trefle"`) is preloaded at
    /// level-load by [`EngineInner::preload_scroll_amulet_sprite`],
    /// so this path only reads the scriptor cache (`&LevelAssets`).
    pub(crate) fn drain_pending_scroll_amulets(&mut self, assets: &LevelAssets) {
        if self.orders.pending_scroll_amulets.is_empty() {
            return;
        }
        let requests: Vec<PendingScrollAmulet> =
            std::mem::take(&mut self.orders.pending_scroll_amulets);
        for req in requests {
            self.spawn_scroll_amulet(assets, req);
        }
    }

    fn spawn_scroll_amulet(&mut self, assets: &LevelAssets, req: PendingScrollAmulet) {
        // Resolve the amulet sprite from the preloaded scriptor cache
        // (`BONUS_FourLeavedClover` / "BONUS Trefle"). A miss here
        // means `preload_scroll_amulet_sprite` didn't run — treat as
        // a bug.
        let mut sprite = crate::sprite::Sprite::default();
        if let Err(e) = sprite.load_frame_info_cached(
            &assets.sprite_scriptor,
            crate::sprite_script::FrameKind::Object,
            "BONUS_FourLeavedClover",
            "BONUS Trefle",
        ) {
            tracing::error!(
                "Scroll-reveal amulet sprite cache lookup failed (scroll {:?}): {e}",
                req.replaces,
            );
            return;
        }
        sprite.force_random_sprite_frame(crate::sim_rng::RngSite::ScrollRevealFrame);

        let mut element = crate::element::ElementData {
            kind: crate::element::ElementKind::ObjectBonus,
            // Default `blipped` flag is `!IsForestLevel()` — same
            // treatment as the mission-stream bonus path.
            blipped: !self.world.weather.is_forest_level,
            sprite,
            ..Default::default()
        };
        // Copy obstacle+plane, layer, sector, direction, position_map,
        // and material onto the spawned amulet. `apply_placement` is
        // the shared helper that wraps these six fields plus the
        // pre-resolved plane.
        let plane = crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
            req.obstacle_index,
            assets.static_sight_obstacles.as_slice(),
        );
        element.sprite.apply_placement(
            req.position_map,
            req.layer,
            req.sector,
            req.direction,
            req.material,
            req.obstacle_index,
            plane,
        );
        let entity = Entity::Bonus(crate::element::ElementBonus {
            element,
            object: crate::element::ObjectData {
                quantity: 1,
                object_type: crate::element::ObjectType::BonusAmulet,
                // Amulets aren't a player-triggered action, they're
                // auto-picked on collision.
                associated_action: crate::profiles::Action::NoAction,
                ..Default::default()
            },
        });
        self.add_entity(entity);
    }
}

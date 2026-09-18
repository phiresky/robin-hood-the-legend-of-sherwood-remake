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
//! dispatch in the selected owner's `tick_ability`; the `ReceivePurseRevealing` handler
//! in `engine/archery.rs` invokes [`EngineInner::reveal_scrolls`] on the
//! `WaitingWithPurse` → transition boundary.

use crate::engine::TickCtx;
use serde::{Deserialize, Serialize};

use super::{EngineInner, LevelAssets};
use crate::element::{Entity, EntityId};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_returns_published_replacement_identity_and_preserves_placement() {
        let sim = crate::sim_rng::SimulationContext::with_seed_and_config(
            1,
            crate::engine::SimConfig {
                difficulty: crate::player_profile::DifficultyLevel::Easy,
                ..Default::default()
            },
        );
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.sprite_scriptor).insert(
            "BONUS_FourLeavedClover/BONUS Trefle",
            crate::sprite_script::SpriteInfo {
                scripts: std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                    action_id: crate::order::OrderType::BonusOne as u16,
                    frame_ids: vec![1],
                    delays: vec![1],
                    distances: vec![0],
                    offsets: vec![Default::default()],
                    sound_ids: vec![0],
                    ..Default::default()
                }]),
                conversion: std::sync::Arc::new({
                    let mut conversion = crate::engine::test_support::unmapped_conversion();
                    conversion[crate::order::OrderType::BonusOne as usize] = 0;
                    conversion
                }),
                size: crate::coordinates::SpriteSize::new(12.0, 8.0),
                center: crate::coordinates::SpriteAnchor::new(2.0, 3.0),
            },
        );
        let mut scroll = crate::element::ElementScroll::default();
        scroll.element.kind = crate::element::ElementKind::ObjectScroll;
        scroll.element.active = true;
        scroll.presence = [false; 3];
        let position = crate::coordinates::MapPoint::new(27.0, 51.0);
        scroll.element.sprite.apply_placement(
            position,
            2,
            None,
            13,
            crate::element::GameMaterial::default(),
            None,
            None,
        );
        let scroll_id = engine.add_test_entity(Entity::Scroll(scroll));
        assets.entities.scroll_entity_ids.push(scroll_id);

        let revealed = engine
            .reveal_scroll(TickCtx::new(&sim, &assets), 0)
            .expect("scroll is revealable");

        assert_ne!(revealed, scroll_id);
        let Entity::Bonus(amulet) = engine.expect_entity(revealed, "revealed amulet") else {
            panic!("replacement must be an amulet");
        };
        assert_eq!(
            amulet.object.object_type,
            crate::element::ObjectType::BonusAmulet
        );
        assert_eq!(amulet.element.position_map(), position);
        assert_eq!(amulet.element.layer(), 2);
        assert_eq!(amulet.element.direction(), 13);
        assert_eq!(engine.scroll_status(scroll_id), ScrollStatus::Taken);
        assert_eq!(engine.reveal_scroll(TickCtx::new(&sim, &assets), 0), None);
    }
}

/// Scroll reveal status. Persisted in the canonical script-domain scroll state
/// (keyed by actor script handle); the script natives
/// `GetScrollStatus` / `SetScrollStatus` read/write it directly.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
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

/// What remark the beggar should say after a reveal attempt.  Fired
/// internally by [`EngineInner::reveal_scrolls`] via
/// the live speech operation on the beggar — callers
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

    /// Speech flags for the remark. The "already
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

    /// Current status of a scroll entity. Reads the engine-owned scroll
    /// domain that is leased to script natives during a script transaction.
    /// Returns [`ScrollStatus::Invisible`] for scrolls that have no
    /// entry or when the mission script is unavailable.
    pub fn scroll_status(&self, scroll: EntityId) -> ScrollStatus {
        let handle = crate::natives::ScriptHandleCodec::actor_handle(scroll);
        let raw = self
            .script_domains
            .scrolls
            .status
            .get(&handle)
            .copied()
            .unwrap_or(0);
        ScrollStatus::from_i32(raw)
    }

    /// Update a scroll's status, sprite, and minimap dot.
    ///
    /// Original-game scroll status changes own the Opened animation side
    /// effect, so every caller (including the script `SetScrollStatus`
    /// native) must pass through this boundary.
    pub(crate) fn set_scroll_status(&mut self, scroll: EntityId, status: ScrollStatus) {
        let handle = crate::natives::ScriptHandleCodec::actor_handle(scroll);
        self.script_domains
            .scrolls
            .status
            .insert(handle, status as i32);
        if let Some(Entity::Scroll(entity)) = self.get_entity_mut(scroll) {
            entity.element.custom_minimap_dot = status.custom_minimap_dot();
            if status == ScrollStatus::Opened {
                let direction = entity.element.direction() as u16;
                entity
                    .element
                    .sprite
                    .force_animation(crate::order::OrderType::BonusThree, direction);
                entity.object.animation = crate::order::OrderType::BonusThree;
            }
        }
    }

    /// Restore the serialized status without running status-change side effects.
    ///
    /// The original game's version-48 save writes the status directly, while the common
    /// element payload separately restores the authoritative minimap dot.
    pub(crate) fn restore_legacy_scroll_status_raw(&mut self, scroll: EntityId, status: i32) {
        let handle = crate::natives::ScriptHandleCodec::actor_handle(scroll);
        self.script_domains.scrolls.status.insert(handle, status);
    }

    /// PC finished the Taking animation on a scroll:
    ///
    /// 1. Flip the taken flag and set status to `Opened`.
    /// 2. Force the BonusThree sprite row (the "scroll is open" idle).
    /// 3. If a script class is bound, invoke its `IsTaken(pc)` callback.
    ///    A non-zero return promotes status `Opened → Taken` and
    ///    refreshes the minimap dot.
    ///
    /// `scroll_handle` and `pc_handle` are actor script handles.
    pub(crate) fn take_scroll(&mut self, tcx: TickCtx<'_>, pc: EntityId, scroll: EntityId) {
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

        // The per-scroll bound script checks whether the player character took it.
        // Non-zero return advances status to Taken; zero keeps it at
        // Opened.  Scrolls with no bound class get `Ok(0)` and stay
        // at Opened.
        let scroll_handle = crate::natives::ScriptHandleCodec::actor_handle(scroll);
        let pc_handle = crate::natives::ScriptHandleCodec::actor_handle(pc);
        let script_result = self.call_script_vm(
            tcx,
            super::ScriptVmKey::Scroll(scroll_handle),
            "IsTaken",
            &[pc_handle],
            crate::natives::ScriptCallFrame::default()
                .with_script_this(scroll_handle)
                .with_current_scroll(scroll_handle),
        );
        match script_result {
            Ok(v) if v != 0 => {
                self.set_scroll_status(scroll, ScrollStatus::Taken);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(?scroll, ?pc, "scroll IsTaken callback failed: {e}");
            }
        }
    }

    // ─── Revealability checks ────────────────────────────────────

    /// Whether a scroll is revealable — active and currently invisible.
    pub fn is_scroll_revealable(&self, assets: &LevelAssets, scroll_id: u16) -> bool {
        let Some(&eid) = assets.entities.scroll_entity_ids.get(scroll_id as usize) else {
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
    fn is_scroll_to_be_replaced_by_amulet(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        scroll: EntityId,
    ) -> bool {
        let difficulty = sim.config().difficulty;
        if difficulty.rules().legacy_level != crate::player_profile::LegacyDifficultyLevel::Easy {
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
    /// * On Easy difficulty with `presence[Easy] == false`, creates an
    ///   amulet before marking the scroll `Taken`.
    /// * Otherwise sets the scroll to `Visible` so it draws on the
    ///   world and on the minimap.
    ///
    /// Returns the revealed scroll or replacement amulet for highlighting.
    /// Returns `None` if the
    /// scroll is not revealable.
    pub(crate) fn reveal_scroll(&mut self, tcx: TickCtx<'_>, scroll_id: u16) -> Option<EntityId> {
        if !self.is_scroll_revealable(tcx.assets, scroll_id) {
            return None;
        }
        let eid = *tcx
            .assets
            .entities
            .scroll_entity_ids
            .get(scroll_id as usize)?;

        if self.is_scroll_to_be_replaced_by_amulet(tcx.sim, eid) {
            let amulet = self.spawn_scroll_amulet(tcx, eid);
            self.set_scroll_status(eid, ScrollStatus::Taken);
            Some(amulet)
        } else {
            self.set_scroll_status(eid, ScrollStatus::Visible);
            Some(eid)
        }
    }

    // ─── Beggar flow ─────────────────────────────────────────────

    /// Run one iteration of the beggar's reveal flow.
    ///
    /// On a successful reveal the beggar's `current_scroll_set`
    /// advances, the minimap's delayed-highlight queue fills with the
    /// revealed scrolls, and the map opens (centred if it was closed).
    /// The beggar's speech cue is fired and settled at this interaction's
    /// owner-local return boundary.
    /// `beggar_dont_talk_counter` is also bumped to 3 frames on the
    /// civilian's friendly-AI state so remarks don't stack.
    ///
    /// Returns the chosen [`BeggarRemark`] for logging / telemetry —
    /// `None` if the entity isn't a civilian / beggar.
    pub(crate) fn reveal_scrolls(
        &mut self,
        tcx: TickCtx<'_>,
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
            self.mission_domain
                .achievements
                .record_beggar_response(beggar, true, false)
                .expect("beggar response after finalization");
            self.say_beggar_remark(tcx, beggar, BeggarRemark::ExhaustedThanx);
            return Some(BeggarRemark::ExhaustedThanx);
        }
        let current_set = scroll_sets[current_idx].clone();

        // Count revealable scrolls in the current set without mutating.
        let revealable_count = current_set
            .iter()
            .filter(|&&sid| self.is_scroll_revealable(tcx.assets, sid))
            .count();

        let remark = if revealable_count != 0 {
            // Reveal each scroll and queue it onto the minimap.
            let mut highlighted = Vec::new();
            for &scroll_id in &current_set {
                if let Some(revealed) = self.reveal_scroll(tcx, scroll_id) {
                    highlighted.push(revealed.index());
                }
            }
            let screen = Self::director_camera_view_size();
            self.feedback
                .pending_side_effects
                .host_events
                .push(super::HostEvent::Minimap(
                    super::MinimapHostEvent::HighlightDelayed {
                        element_ids: highlighted,
                        screen_width: screen.x,
                        screen_height: screen.y,
                    },
                ));

            // The "revealable scrolls remain?" check runs BEFORE the
            // cursor advances.  The scrolls just revealed in
            // `current_set` no longer count (their status is now
            // `Visible`), so the check reports on future sets only.
            if self.are_there_revealable_scrolls(tcx.assets, beggar) {
                BeggarRemark::GivesInfo
            } else {
                BeggarRemark::GivesLastInfo
            }
        } else if self.are_there_revealable_scrolls(tcx.assets, beggar) {
            BeggarRemark::WantsMore
        } else {
            BeggarRemark::Thanx
        };

        // Fire the speech cue on the beggar's AI controller.
        self.say_beggar_remark(tcx, beggar, remark);

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

        let exhausted = !self.are_there_revealable_scrolls(tcx.assets, beggar);
        self.mission_domain
            .achievements
            .record_beggar_response(beggar, exhausted, revealable_count != 0)
            .expect("beggar response after finalization");
        Some(remark)
    }

    /// Fire and settle the beggar speech cue at this interaction boundary.
    fn say_beggar_remark(&mut self, tcx: TickCtx<'_>, beggar: EntityId, remark: BeggarRemark) {
        self.execute_ai_speech(
            tcx,
            beggar,
            crate::ai::AiSpeechAttempt {
                remark: remark.remark(),
                flags: remark.speech_flags().bits(),
            },
        );
    }

    fn spawn_scroll_amulet(&mut self, tcx: TickCtx<'_>, scroll_id: EntityId) -> EntityId {
        // Resolve the amulet sprite from the preloaded scriptor cache
        // (`BONUS_FourLeavedClover` / "BONUS Trefle"). A miss here
        // means `preload_scroll_amulet_sprite` didn't run — treat as
        // a bug.
        let mut sprite = crate::sprite::Sprite::default();
        if let Err(e) = sprite.load_frame_info_cached(
            &tcx.assets.sprite_scriptor,
            crate::sprite_script::FrameKind::Object,
            "BONUS_FourLeavedClover",
            "BONUS Trefle",
        ) {
            panic!("Scroll-reveal amulet sprite cache lookup failed (scroll {scroll_id:?}): {e}");
        }
        sprite.force_random_sprite_frame(tcx.sim, crate::sim_rng::RngSite::ScrollRevealFrame);

        let mut element = {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ObjectBonus;
            // Default `blipped` flag is false in forest levels — same
            // treatment as the mission-stream bonus path.
            initial_element.blipped = !self.world.weather.is_forest_level;
            initial_element.sprite = sprite;
            initial_element
        };
        // Copy obstacle+plane, layer, sector, direction, position_map,
        // and material onto the spawned amulet. `apply_placement` is
        // the shared helper that wraps these six fields plus the
        // pre-resolved plane.
        let scroll = self
            .expect_entity(scroll_id, "scroll replacement placement")
            .element_data();
        let plane = crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
            scroll.obstacle_index(),
            tcx.assets.environment.static_sight_obstacles.as_slice(),
        );
        element.sprite.apply_placement(
            scroll.position_map(),
            scroll.layer(),
            scroll.sector(),
            scroll.direction(),
            scroll.material(),
            scroll.obstacle_index(),
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
        self.add_entity(entity)
    }
}

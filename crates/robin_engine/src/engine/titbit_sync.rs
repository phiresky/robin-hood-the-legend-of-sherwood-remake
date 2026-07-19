//! Per-frame titbit synchronization.
//!
//! Persistent titbits (emoticons, unconscious stars, weak-stunned indicators,
//! hidden icons) are maintained here by comparing current
//! entity state against existing titbits each frame and adding/removing as
//! needed.
//!
//! Sources of persistent titbits:
//! - Emoticon titbits are added once per NPC at creation; their sprite row
//!   is chosen dynamically each frame based on the current emoticon type
//!   and size.
//! - Unconscious star titbits are added when an entity is concussed and
//!   removed when it wakes up.
//! - Weak/stunned titbits are added by AI alert events and combat stun.
//! - Hidden titbits are added when a PC enters a hidden posture
//!   (cape/tree/anonymous archer).
//! - Smoke belongs to `RHElementMobile`, but every chariot record in shipped
//!   Robin Hood data has its smoke flag disabled.

use super::*;
use crate::ai::{AiState, EmoticonType};
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::element::{DetectableType, Entity, EntityId, Posture};
use crate::titbit::{ElementHandle, HiddenCharacter, INVALID_ID, SpriteRow, TitbitKind};

impl EngineInner {
    /// Sync persistent titbit indicators with current entity state.
    ///
    /// Called once per frame from `perform_hourglass`, before
    /// `titbit_manager.update()`.
    ///
    /// UnconsciousStar and WeakStunned (combat) titbits are created
    /// event-driven at their transition sites (`handle_knockout`,
    /// `try_pc_coma_save`, `tick_melee_combat`).  Only the apple-sauce
    /// WeakStunned case is synced here because the AI code doesn't have
    /// direct access to the titbit manager.
    pub(super) fn sync_titbits(&mut self, assets: &LevelAssets) {
        self.sync_emoticon_titbits();
        self.sync_apple_sauce_titbits();
        self.sync_hidden_titbits(assets);
        self.sync_apple_smell_titbits();
        self.sync_speak_titbits();
        self.sync_danger_point_titbits();
        self.sync_work_icon_titbits();
        self.refresh_titbit_positions();
    }

    // ── Event-driven titbit creation helpers ──

    /// Add an UnconsciousStar titbit for the given entity.
    /// Called from `handle_knockout` and `try_pc_coma_save` when an
    /// entity transitions to unconscious.
    pub(super) fn add_unconscious_star(&mut self, entity_id: EntityId) {
        let handle = ElementHandle(entity_id.index());
        if self
            .feedback
            .titbit_manager
            .titbit_exists(TitbitKind::UnconsciousStar, handle)
        {
            return;
        }
        let Some(entity) = self.world.entities.get(entity_id) else {
            return;
        };
        let epos = entity
            .compute_stars_point()
            .unwrap_or(entity.element_data().position());
        let pos = WorldPoint3D {
            x: epos.x,
            y: epos.y,
            z: epos.z,
        };
        let sprite_row = match entity {
            Entity::Pc(pc) => pc.human.concussion_of_the_brain,
            Entity::Soldier(s) => s.human.concussion_of_the_brain,
            Entity::Civilian(c) => c.human.concussion_of_the_brain,
            _ => 0,
        };
        let sprite_row = 1 + (sprite_row / 50).min(4);
        let layer = entity.element_data().layer();
        let titbit_id = self.feedback.titbit_manager.add_titbit(
            pos,
            layer,
            TitbitKind::UnconsciousStar,
            handle,
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            Some(pos.y),
            Some(layer),
        );
        if titbit_id != INVALID_ID
            && let Some(titbit) = self
                .feedback
                .titbit_manager
                .titbits_mut()
                .iter_mut()
                .find(|t| t.id == titbit_id)
        {
            titbit.sprite_row = sprite_row;
        }
    }

    /// Spawn a floating damage-number titbit above `entity_id`.
    ///
    /// Triggered from the life-points setter when life points decrease:
    /// a `Counter` titbit is added with `phase = damage_amount` and
    /// `position = (x, y, 2)` — X/Y track the entity, Z carries the rise
    /// speed per frame.  The titbit expires after `COUNTER_LIMIT` frames
    /// via `update_counter`.
    ///
    /// `damage` must be > 0; the `add_titbit` call no-ops for phase 0.
    pub(super) fn add_damage_number(&mut self, entity_id: EntityId, damage: u16) {
        if damage == 0 {
            return;
        }
        let handle = ElementHandle(entity_id.index());
        let Some(entity) = self.world.entities.get(entity_id) else {
            return;
        };
        let elem = entity.element_data();
        // X/Y are re-tracked to the entity by `refresh_titbit_positions`;
        // Z carries the rise speed.
        let pos = WorldPoint3D {
            x: elem.position_map().x,
            y: elem.position_map().y,
            z: 2.0,
        };
        let layer = elem.layer();
        self.feedback.titbit_manager.add_titbit(
            pos,
            layer,
            TitbitKind::Counter,
            handle,
            damage,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            Some(pos.y),
            Some(layer),
        );
    }

    // No producer as of 2026-04-29 — workspace grep
    // `rg -t rust '\badd_gun_impact_titbit\b' crates/` returns only the
    // definition. Kept as documentation of the titbit/impact-FX pairing
    // for any future gun/cannon-impact path; promote to a real callsite when
    // such a path is wired.
    /// Add a WeakStunned titbit for the given entity.
    /// Called from `tick_melee_combat` when a BeingWeakSword or
    /// BeingStunnedSword animation begins, and from `sync_apple_sauce_titbits`
    /// for the apple-in-visor AI substate.
    pub(super) fn add_weak_stunned(&mut self, entity_id: EntityId) {
        let handle = ElementHandle(entity_id.index());
        if self
            .feedback
            .titbit_manager
            .titbit_exists(TitbitKind::WeakStunned, handle)
        {
            return;
        }
        let Some(entity) = self.world.entities.get(entity_id) else {
            return;
        };
        let epos = entity
            .compute_stars_point()
            .unwrap_or(entity.element_data().position());
        let pos = WorldPoint3D {
            x: epos.x,
            y: epos.y,
            z: epos.z,
        };
        let layer = entity.element_data().layer();
        self.feedback.titbit_manager.add_titbit(
            pos,
            layer,
            TitbitKind::WeakStunned,
            handle,
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            Some(pos.y),
            Some(layer),
        );
    }

    /// Add the weak/stunned titbit and run the combat side effects
    /// (smalltalk-initiative handoff and `EventAdversaryWeak` stimulus
    /// dispatch) that fire when a melee weakness is initiated.
    pub(super) fn add_weak_stunned_combat(&mut self, entity_id: EntityId) {
        self.add_weak_stunned(entity_id);

        let opponents: Vec<EntityId> = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .map(|h| h.opponents.clone())
            .unwrap_or_default();

        let principal_id = opponents.first().copied();
        let is_mutual = principal_id
            .and_then(|pid| {
                self.get_entity(pid)
                    .and_then(|e| e.human_data())
                    .map(|h| h.opponents.first().copied() == Some(entity_id))
            })
            .unwrap_or(false);

        if let Some(entity) = self.world.entities.get_mut(entity_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_initiative = false;
        }
        if let Some(pid) = principal_id
            && is_mutual
            && let Some(entity) = self.world.entities.get_mut(pid)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_initiative = true;
            human.received_smalltalk_initiative = true;
        }

        for opp_id in opponents {
            self.dispatch_ai_stimulus(
                opp_id,
                crate::ai::Stimulus::with_human(
                    crate::ai::StimulusType::EventAdversaryWeak,
                    entity_id.index(),
                ),
            );
        }
    }

    /// Update every titbit's position and sprite_row from its
    /// element_supplier's current state.
    ///
    /// Titbit positions and sprite rows must be recomputed from the
    /// entity every frame. Without this, titbits stay stuck at their
    /// creation position when entities move, and many kinds render with
    /// sprite_row=0 (star texture) because the row was never set.
    fn refresh_titbit_positions(&mut self) {
        for t in self.feedback.titbit_manager.titbits_mut().iter_mut() {
            if !t.element_supplier.is_valid() {
                // "Teleport stars" — UnconsciousStar without a supplier
                // uses its creation position and picks star count from
                // phase>>2.
                if t.kind == TitbitKind::UnconsciousStar {
                    t.sprite_row = 1 + (t.phase >> 2).min(4);
                }
                continue; // Particle effects (smoke, dust) with no supplier
            }
            let Some(entity_id) = self.world.entities.id_at_legacy_slot(t.element_supplier.0)
            else {
                continue;
            };
            let Some(entity) = self.world.entities.get(entity_id) else {
                continue;
            };
            let elem = entity.element_data();

            // Counter uses its own positioning: X/Y follow the entity,
            // Z holds the rise speed (2.0 per frame). Text rendering
            // adds a vertical rise based on sprite_frame.
            if t.kind == TitbitKind::Counter {
                t.position.x = elem.position_map().x;
                t.position.y = elem.position_map().y;
                // Preserve z (rise speed) set by add_titbit.
                continue;
            }

            // ── Per-kind position ──
            // Different titbit kinds anchor to different body points:
            //   Stars/Emoticon/Speak/AppleSmell/WorkIcon → stars point
            //   Lock → feet point
            //   Hidden → entity position + posture Z offset
            //   Particles (Smoke/etc.) → keep creation position
            match t.kind {
                TitbitKind::UnconsciousStar
                | TitbitKind::WeakStunned
                | TitbitKind::Emoticon
                | TitbitKind::AppleSmell
                | TitbitKind::Speak => {
                    if let Some(pt) = entity.compute_stars_point() {
                        t.position = pt;
                    }
                }
                TitbitKind::WorkIcon => {
                    // Stars point lifted by 24 so the work icon sits
                    // above the head rather than at the emoticon slot.
                    if let Some(mut pt) = entity.compute_stars_point() {
                        pt.z += 24.0;
                        t.position = pt;
                    }
                }
                TitbitKind::Lock => {
                    if let Some(pt) = entity.compute_feet_point() {
                        t.position = pt;
                    }
                }
                TitbitKind::Hidden => {
                    // Entity position with a per-posture Z offset:
                    // Spy → +74, Tree → +35.
                    let z_add = match elem.posture {
                        Posture::Spy => 74.0,
                        Posture::Tree => 35.0,
                        _ => 35.0,
                    };
                    t.position = WorldPoint3D {
                        x: elem.position().x,
                        y: elem.position().y,
                        z: elem.position().z + z_add,
                    };
                }
                TitbitKind::QuickAction | TitbitKind::QuickActionRun => {
                    // legacy implementation uses raw 3D position only for FX targets
                    // (`SetPosition` + `ComputePositionMap`).  Normal
                    // suppliers use `positionMap` directly.
                    t.position = if entity.is_fx_target() {
                        elem.position()
                    } else {
                        WorldPoint3D {
                            x: elem.position_map().x,
                            y: elem.position_map().y,
                            z: 0.0,
                        }
                    };
                }
                // Particle effects (Smoke, Water, Plouf, Ghost,
                // GunImpact) and DangerPoint use their creation
                // position — don't update.  DangerPoint marks a fixed
                // world location, not the entity's current position.
                _ => {}
            }

            // ── Sprite row per kind ──
            // sprite_row is recomputed each frame from titbit kind and
            // entity state.  Emoticon row is set in
            // sync_emoticon_titbits(); Smoke/Counter are handled
            // separately (no supplier or text-rendered).
            match t.kind {
                TitbitKind::WeakStunned => {
                    t.sprite_row = SpriteRow::OneStar as u16;
                }
                TitbitKind::UnconsciousStar => {
                    // 1 + min(4, concussion/50): more concussion → more stars.
                    let concussion = match entity {
                        Entity::Pc(pc) => pc.human.concussion_of_the_brain,
                        Entity::Soldier(s) => s.human.concussion_of_the_brain,
                        Entity::Civilian(c) => c.human.concussion_of_the_brain,
                        _ => 0,
                    };
                    let stars = 1 + (concussion / 50).min(4);
                    t.sprite_row = stars; // SpriteRow 1..=5 = OneStar..FiveStars
                }
                TitbitKind::Hidden => {
                    t.sprite_row = SpriteRow::Hidden as u16;
                }
                TitbitKind::Speak => {
                    t.sprite_row = SpriteRow::Speak as u16;
                }
                TitbitKind::AppleSmell => {
                    t.sprite_row = SpriteRow::AppleSmell as u16;
                }
                TitbitKind::DangerPoint => {
                    t.sprite_row = SpriteRow::DangerPoint as u16;
                }
                TitbitKind::WorkIcon => {
                    // Sprite row tracks the PC's current work type
                    // (picked up from `pc.work_icon` each frame).
                    if let Entity::Pc(pc) = entity {
                        t.sprite_row = work_icon_to_sprite_row(pc.pc.work_icon);
                    }
                }
                _ => {}
            }
        }
    }

    /// Ensure every NPC with a non-None emoticon has an Emoticon titbit,
    /// and that its sprite_row matches the current emoticon type.
    /// Remove titbits for NPCs whose emoticon is None.
    ///
    /// The current emoticon type and size are picked each frame; an
    /// emoticon of `None` suppresses the draw entirely.
    fn sync_emoticon_titbits(&mut self) {
        // Collect NPC emoticon state to avoid borrowing conflicts with
        // titbit_manager.
        struct EmoticonState {
            entity_id: EntityId,
            emoticon: EmoticonType,
            /// Size parameter for GrowingQuestionMark (0-15). Ignored
            /// for other emoticon types.
            size: u16,
            position: WorldPoint3D,
            layer: u16,
            display_order: f32,
            show: bool,
        }

        let frame = self.control.frame_counter;
        let mut npc_states: Vec<EmoticonState> = Vec::new();

        // Pre-compute NPC IDs to avoid conflicts with point_building_sector.
        let all_ids: Vec<EntityId> = self
            .world
            .entities
            .npc_ids()
            .chain(self.world.pc_ids.iter().copied())
            .collect();

        for npc_id in all_ids {
            let Some(entity) = self.world.entities.get(npc_id) else {
                continue;
            };

            // Only NPCs (soldiers and civilians) have emoticons.
            #[derive(Clone, Copy)]
            struct NpcSnapshot {
                is_dead: bool,
                is_unconscious: bool,
                blood_alcohol: u8,
                stored_emoticon: EmoticonType,
                maximal_detection_suspect: u16,
                worst_detected_type: DetectableType,
                sorrow_level: u16,
                current_state: AiState,
                blipped: bool,
                posture: Posture,
                position: MapPoint,
                layer: u16,
                active: bool,
                display_order: f32,
                sector: Option<crate::position_interface::SectorHandle>,
            }

            let snap = match entity {
                Entity::Soldier(s) => {
                    let stored = s
                        .npc
                        .ai_brain
                        .base()
                        .map(|ai| {
                            if ai.emoticon_has_expiration_date
                                && frame >= ai.emoticon_expiration_date
                            {
                                EmoticonType::None
                            } else {
                                ai.current_emoticon_type
                            }
                        })
                        .unwrap_or(EmoticonType::None);
                    let blood = s
                        .npc
                        .ai_brain
                        .base()
                        .map(|ai| ai.blood_alcohol)
                        .unwrap_or(0);
                    let sorrow = s.npc.ai_brain.base().map(|ai| ai.sorrow_level).unwrap_or(0);
                    NpcSnapshot {
                        is_dead: s.npc.life_points <= 0,
                        is_unconscious: s.human.unconscious,
                        blood_alcohol: blood,
                        stored_emoticon: stored,
                        maximal_detection_suspect: s.npc.maximal_detection_suspect,
                        worst_detected_type: s.npc.worst_detected_type,
                        sorrow_level: sorrow,
                        current_state: s.npc.ai_state(),
                        blipped: s.element.blipped,
                        posture: s.element.posture,
                        position: s.element.position_map(),
                        layer: s.element.layer(),
                        active: s.element.active,
                        display_order: s.element.position_map().y,
                        sector: s.element.sector(),
                    }
                }
                Entity::Civilian(c) => {
                    let stored = c
                        .npc
                        .ai_brain
                        .base()
                        .map(|ai| {
                            if ai.emoticon_has_expiration_date
                                && frame >= ai.emoticon_expiration_date
                            {
                                EmoticonType::None
                            } else {
                                ai.current_emoticon_type
                            }
                        })
                        .unwrap_or(EmoticonType::None);
                    let blood = c
                        .npc
                        .ai_brain
                        .base()
                        .map(|ai| ai.blood_alcohol)
                        .unwrap_or(0);
                    let sorrow = c.npc.ai_brain.base().map(|ai| ai.sorrow_level).unwrap_or(0);
                    NpcSnapshot {
                        is_dead: c.npc.life_points <= 0,
                        is_unconscious: c.human.unconscious,
                        blood_alcohol: blood,
                        stored_emoticon: stored,
                        maximal_detection_suspect: c.npc.maximal_detection_suspect,
                        worst_detected_type: c.npc.worst_detected_type,
                        sorrow_level: sorrow,
                        current_state: c.npc.ai_state(),
                        blipped: c.element.blipped,
                        posture: c.element.posture,
                        position: c.element.position_map(),
                        layer: c.element.layer(),
                        active: c.element.active,
                        display_order: c.element.position_map().y,
                        sector: c.element.sector(),
                    }
                }
                _ => continue,
            };

            // ── Emoticon filter chain ──
            // Out-of-order entities (dead, unconscious, tied) force the
            // emoticon to None.  Still enqueue a sync row so an
            // already-created emoticon titbit is removed below.
            if snap.is_dead || snap.is_unconscious || snap.posture == Posture::Tied {
                npc_states.push(EmoticonState {
                    entity_id: npc_id,
                    emoticon: EmoticonType::None,
                    size: 0,
                    position: WorldPoint3D {
                        x: snap.position.x,
                        y: snap.position.y,
                        z: 0.0,
                    },
                    layer: snap.layer,
                    display_order: snap.display_order,
                    show: false,
                });
                continue;
            }

            // Drunken override is an *early return*, not a
            // fallthrough: when blood alcohol exceeds the titbit limit
            // and no other emoticon is stored, emit `{Drunken, 0}` and
            // skip the detection-priority arms.  Otherwise a drunken
            // NPC in `Default`/`Sleeping` with any non-zero suspect
            // would slide into the detection branch and render a
            // `GrowingQuestionMark` instead of the Drunken bubble.
            let drunk_override = snap.blood_alcohol as i32
                > crate::parameters_ai::AI_DRUNKEN_TITBIT_ALCOHOL_LIMIT
                && snap.stored_emoticon == EmoticonType::None;

            // Effective suspect = min(1000, max(maximal_detection_suspect, sorrow_level)).
            // Both fields are maintained by the detection loop
            // (engine/ai.rs write sites) and by the sorrow system
            // respectively — read them directly rather than re-deriving
            // from per-type suspects.  `sorrow_level` increment paths
            // still have gaps, but feeding it into the formula keeps
            // behaviour in sync once they land.
            let suspect = snap
                .maximal_detection_suspect
                .max(snap.sorrow_level)
                .min(1000);

            // `worst_detected_type` is the running "most threatening"
            // type across the whole detection refresh, maintained by
            // `engine/ai.rs` in parallel with the suspect accumulator.
            // Reading the stored field (vs. re-deriving per-frame)
            // preserves the across-frame latch semantics: an enemy
            // briefly seen then lost keeps the slot until the max
            // suspect drains to 0.
            let worst = snap.worst_detected_type;

            // ── State priority ──
            // Drunken override short-circuits the whole block.
            let (emoticon, size) = if drunk_override {
                (EmoticonType::Drunken, 0)
            } else {
                let show_detection = if suspect == 0 {
                    false
                } else if snap.stored_emoticon == EmoticonType::None {
                    true
                } else {
                    match snap.current_state {
                        AiState::Sleeping | AiState::Default => true,
                        AiState::Wondering => detectable_ge(worst, DetectableType::Object),
                        AiState::Seeking | AiState::Menacing | AiState::Fleeing => {
                            detectable_ge(worst, DetectableType::Body)
                        }
                        AiState::Attacking => false,
                    }
                };

                if show_detection {
                    // size = min(suspect * 0.016, 15)
                    let size = ((suspect as f32 * 0.016) as u16).min(15);
                    (EmoticonType::GrowingQuestionMark, size)
                } else {
                    (snap.stored_emoticon, 0)
                }
            };

            // ── IsActiveAndOutsideBuilding + !blipped filter ──
            // Only draw the emoticon when the NPC is active, outside a
            // building, and not currently blipped on the minimap.
            let inside_building = self.entity_building_sector(snap.sector).is_some();
            let show = snap.active && !inside_building && !snap.blipped;

            npc_states.push(EmoticonState {
                entity_id: npc_id,
                emoticon,
                size,
                position: WorldPoint3D {
                    x: snap.position.x,
                    y: snap.position.y,
                    z: 0.0,
                },
                layer: snap.layer,
                display_order: snap.display_order,
                show,
            });
        }

        // Sync titbits.
        for state in &npc_states {
            let handle = ElementHandle(state.entity_id.index());
            let has_titbit = self
                .feedback
                .titbit_manager
                .titbit_exists(TitbitKind::Emoticon, handle);

            if state.emoticon == EmoticonType::None || !state.show {
                // No emoticon → remove any existing titbit.
                if has_titbit {
                    self.feedback
                        .titbit_manager
                        .remove_titbit(TitbitKind::Emoticon, handle);
                }
            } else {
                let target_row = emoticon_to_sprite_row(state.emoticon);

                if !has_titbit {
                    // Add new emoticon titbit.
                    self.feedback.titbit_manager.add_titbit(
                        state.position,
                        state.layer,
                        TitbitKind::Emoticon,
                        handle,
                        0,
                        ElementHandle::INVALID,
                        false,
                        INVALID_ID,
                        true, // display_titbits irrelevant for emoticons
                        Some(state.display_order),
                        Some(state.layer),
                    );
                }

                // Update the sprite row to match current emoticon type.
                // The titbit update() handles frame animation; we just
                // need to ensure the row is correct.  For the growing
                // question mark, `phase` holds the size (0-15) used
                // to pick the animation section at render time.
                // Position is updated by refresh_titbit_positions() below.
                for t in self.feedback.titbit_manager.titbits_mut().iter_mut() {
                    if t.kind == TitbitKind::Emoticon && t.element_supplier == handle {
                        if t.sprite_row != target_row {
                            t.sprite_row = target_row;
                            t.sprite_frame = 0;
                            t.frame_count = 0;
                        }
                        t.phase = state.size;
                        break;
                    }
                }
            }
        }
    }

    /// Sync WeakStunned titbits for soldiers in the apple-sauce AI substate.
    ///
    /// Combat-driven WeakStunned (BeingWeakSword/BeingStunnedSword) is
    /// created event-driven in `tick_melee_combat`.  Only the apple-sauce
    /// case is synced here because the AI code doesn't have direct access
    /// to the titbit manager.
    ///
    /// Removal is handled by `update()` via `is_weak_or_stunned` query.
    fn sync_apple_sauce_titbits(&mut self) {
        use crate::ai::Substate;

        // Only soldiers can enter the apple-sauce substate.
        let npc_ids: Vec<EntityId> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            let Some(Entity::Soldier(s)) = self.world.entities.get(npc_id) else {
                continue;
            };
            if !s.element.active || s.human.unconscious || s.npc.life_points <= 0 {
                continue;
            }
            if s.npc.ai_substate() == Substate::WonderingAppleSauceInTheVisor {
                self.add_weak_stunned(npc_id);
            }
        }
    }

    /// Sync Hidden titbits for PCs in Spy or Tree posture.
    ///
    /// The update() function removes them when `is_hidden_posture`
    /// returns false.
    fn sync_hidden_titbits(&mut self, assets: &LevelAssets) {
        struct HiddenState {
            id: EntityId,
            position: WorldPoint3D,
            layer: u16,
            is_hidden: bool,
            hidden_character: HiddenCharacter,
        }

        let mut states: Vec<HiddenState> = Vec::new();

        if false {
            return;
        }

        for &pc_id in &self.world.pc_ids {
            let Some(Entity::Pc(pc)) = self.world.entities.get(pc_id) else {
                continue;
            };
            if !pc.element.active || pc.pc.life_points <= 0 {
                continue;
            }
            // Hidden titbit fires for AnonymousArcher (set by mission
            // scripts via the actor-posture native) and for Spy/Tree
            // (the cape postures, set when the waiting animation
            // initializes).
            let is_hidden = matches!(
                pc.element.posture,
                Posture::Spy | Posture::Tree | Posture::AnonymousArcher
            );
            let profile = assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "sync_hidden_titbits: PC entity {} has unknown profile_index {}",
                        pc_id.index(),
                        pc.pc.profile_index
                    )
                });
            let hidden_character = HiddenCharacter::for_pc(pc.pc.robin, &profile.filename);
            states.push(HiddenState {
                id: pc_id,
                position: WorldPoint3D {
                    x: pc.element.position_map().x,
                    y: pc.element.position_map().y,
                    z: 0.0,
                },
                layer: pc.element.layer(),
                is_hidden,
                hidden_character,
            });
        }

        for state in &states {
            let handle = ElementHandle(state.id.index());
            let has_titbit = self
                .feedback
                .titbit_manager
                .titbit_exists(TitbitKind::Hidden, handle);

            if state.is_hidden && !has_titbit {
                self.feedback.titbit_manager.add_titbit(
                    state.position,
                    state.layer,
                    TitbitKind::Hidden,
                    handle,
                    state.hidden_character.to_phase(),
                    ElementHandle::INVALID,
                    false,
                    INVALID_ID,
                    true,
                    Some(state.position.y),
                    Some(state.layer),
                );
            }
            // Removal is handled by update() via is_hidden_posture query.
        }
    }

    /// Sync AppleSmell titbits for soldiers under apple bait.
    ///
    /// We sync from the live `apple_smell` counter instead of hooking
    /// the setter, so the titbit auto-removes when the counter reaches 0.
    fn sync_apple_smell_titbits(&mut self) {
        struct AppleState {
            id: EntityId,
            position: WorldPoint3D,
            layer: u16,
            is_smelling: bool,
        }

        let mut states: Vec<AppleState> = Vec::new();
        for (npc_id, s) in self.world.entities.soldiers() {
            if !s.element.active || s.human.unconscious || s.npc.life_points <= 0 {
                continue;
            }
            states.push(AppleState {
                id: npc_id.into(),
                position: WorldPoint3D {
                    x: s.element.position_map().x,
                    y: s.element.position_map().y,
                    z: 0.0,
                },
                layer: s.element.layer(),
                is_smelling: s.is_smelling_apple(),
            });
        }

        for state in &states {
            let handle = ElementHandle(state.id.index());
            let has_titbit = self
                .feedback
                .titbit_manager
                .titbit_exists(TitbitKind::AppleSmell, handle);

            if state.is_smelling && !has_titbit {
                self.feedback.titbit_manager.add_titbit(
                    state.position,
                    state.layer,
                    TitbitKind::AppleSmell,
                    handle,
                    0,
                    ElementHandle::INVALID,
                    false,
                    INVALID_ID,
                    true,
                    Some(state.position.y),
                    Some(state.layer),
                );
            } else if !state.is_smelling && has_titbit {
                self.feedback
                    .titbit_manager
                    .remove_titbit(TitbitKind::AppleSmell, handle);
            }
        }
    }

    /// Sync Speak titbits for NPCs with an attached scroll.
    ///
    /// We sync from the engine-owned scroll domain populated by the
    /// scroll-attachment script native.
    fn sync_speak_titbits(&mut self) {
        // Collect (npc_id, has_scroll, force_refresh) from canonical entities.
        // The map uses actor script handles.
        // `force_refresh` carries the remove-then-add pulse for NPCs
        // whose attached scroll handle just changed value
        // (`scroll_attachment_dirty`, populated by the scroll-attachment
        // native).
        struct SpeakState {
            id: EntityId,
            position: WorldPoint3D,
            layer: u16,
            has_scroll: bool,
            force_refresh: bool,
        }

        // Read + drain scroll attachments from the engine-owned domain.
        let (attached, dirty): (
            std::collections::HashSet<i32>,
            std::collections::HashSet<i32>,
        ) = {
            let attached = self
                .script_domains
                .scrolls
                .attachments
                .keys()
                .copied()
                .collect();
            let dirty = std::mem::take(&mut self.script_domains.scrolls.attachment_dirty)
                .into_iter()
                .collect();
            (attached, dirty)
        };

        let mut states: Vec<SpeakState> = Vec::new();
        for (npc_id, entity) in self.world.entities.npcs() {
            let (pos, layer, active) = match entity {
                Entity::Soldier(s) => (
                    s.element.position_map(),
                    s.element.layer(),
                    s.element.active && !s.human.unconscious && s.npc.life_points > 0,
                ),
                Entity::Civilian(c) => (
                    c.element.position_map(),
                    c.element.layer(),
                    c.element.active && !c.human.unconscious && c.npc.life_points > 0,
                ),
                _ => continue,
            };
            if !active {
                continue;
            }
            let script_handle = crate::natives::ScriptHandleCodec::actor_handle(npc_id);
            states.push(SpeakState {
                id: npc_id.into(),
                position: WorldPoint3D {
                    x: pos.x,
                    y: pos.y,
                    z: 0.0,
                },
                layer,
                has_scroll: attached.contains(&script_handle),
                force_refresh: dirty.contains(&script_handle),
            });
        }

        for state in &states {
            let handle = ElementHandle(state.id.index());
            let has_titbit = self
                .feedback
                .titbit_manager
                .titbit_exists(TitbitKind::Speak, handle);

            // The scroll-attachment path pulses the SPEAK titbit
            // (remove + re-add) whenever the attached scroll pointer
            // changes; on a same-NPC scroll replace we still hold the
            // titbit, so strip it first and let the add-branch below
            // re-install it with a fresh titbit index.
            if state.force_refresh && state.has_scroll && has_titbit {
                self.feedback
                    .titbit_manager
                    .remove_titbit(TitbitKind::Speak, handle);
            }
            let has_titbit = has_titbit && !(state.force_refresh && state.has_scroll);

            if state.has_scroll && !has_titbit {
                self.feedback.titbit_manager.add_titbit(
                    state.position,
                    state.layer,
                    TitbitKind::Speak,
                    handle,
                    0,
                    ElementHandle::INVALID,
                    false,
                    INVALID_ID,
                    true,
                    Some(state.position.y),
                    Some(state.layer),
                );
            } else if !state.has_scroll && has_titbit {
                self.feedback
                    .titbit_manager
                    .remove_titbit(TitbitKind::Speak, handle);
            }
        }
    }

    /// Sync DangerPoint titbits for PCs currently shielding someone.
    fn sync_danger_point_titbits(&mut self) {
        struct DangerState {
            id: EntityId,
            position: WorldPoint3D,
            layer: u16,
            is_protecting: bool,
        }

        let mut states: Vec<DangerState> = Vec::new();
        for &pc_id in &self.world.pc_ids {
            let Some(Entity::Pc(pc)) = self.world.entities.get(pc_id) else {
                continue;
            };
            if !pc.element.active || pc.pc.life_points <= 0 || pc.human.unconscious {
                continue;
            }
            // The danger point's layer is what the player picked when
            // raising the shield, plumbed through
            // `Field::ShieldDangerPointLayer` on the queued
            // RaiseShield element and stamped onto
            // `PcData::shield_danger_point_layer` by
            // `dispatch_raise_shield`.  When the picked layer wasn't
            // supplied (AI-issued raise via the Interaction-data
            // branch), the field stays 0 and we fall back to the PC's
            // own layer.
            let layer = if pc.pc.shield_danger_point_layer != 0 {
                pc.pc.shield_danger_point_layer
            } else {
                pc.element.layer()
            };
            states.push(DangerState {
                id: pc_id,
                position: WorldPoint3D {
                    x: pc.pc.shield_danger_point.x,
                    y: pc.pc.shield_danger_point.y,
                    z: pc.pc.shield_danger_point.z,
                },
                layer,
                is_protecting: pc.pc.shield_protected.is_some(),
            });
        }

        for state in &states {
            let handle = ElementHandle(state.id.index());
            let has_titbit = self
                .feedback
                .titbit_manager
                .titbit_exists(TitbitKind::DangerPoint, handle);

            if state.is_protecting && !has_titbit {
                self.feedback.titbit_manager.add_titbit(
                    state.position,
                    state.layer,
                    TitbitKind::DangerPoint,
                    handle,
                    0,
                    ElementHandle::INVALID,
                    false,
                    INVALID_ID,
                    true,
                    Some(state.position.y),
                    Some(state.layer),
                );
            } else if !state.is_protecting && has_titbit {
                self.feedback
                    .titbit_manager
                    .remove_titbit(TitbitKind::DangerPoint, handle);
            }
        }
    }

    /// Sync WorkIcon titbits for PCs in Sherwood camp work assignments.
    ///
    /// The titbit's sprite row is set each frame from the PC's current
    /// `work_icon` via `refresh_titbit_positions`.
    fn sync_work_icon_titbits(&mut self) {
        use crate::element::WorkIcon;

        struct WorkState {
            id: EntityId,
            position: WorldPoint3D,
            layer: u16,
            work: WorkIcon,
        }

        let mut states: Vec<WorkState> = Vec::new();
        for &pc_id in &self.world.pc_ids {
            let Some(Entity::Pc(pc)) = self.world.entities.get(pc_id) else {
                continue;
            };
            if !pc.element.active || pc.pc.life_points <= 0 {
                continue;
            }
            states.push(WorkState {
                id: pc_id,
                position: WorldPoint3D {
                    x: pc.element.position_map().x,
                    y: pc.element.position_map().y,
                    z: 0.0,
                },
                layer: pc.element.layer(),
                work: pc.pc.work_icon,
            });
        }

        for state in &states {
            let handle = ElementHandle(state.id.index());
            let has_titbit = self
                .feedback
                .titbit_manager
                .titbit_exists(TitbitKind::WorkIcon, handle);

            if state.work != WorkIcon::None && !has_titbit {
                self.feedback.titbit_manager.add_titbit(
                    state.position,
                    state.layer,
                    TitbitKind::WorkIcon,
                    handle,
                    0,
                    ElementHandle::INVALID,
                    false,
                    INVALID_ID,
                    true,
                    Some(state.position.y),
                    Some(state.layer),
                );
            } else if state.work == WorkIcon::None && has_titbit {
                self.feedback
                    .titbit_manager
                    .remove_titbit(TitbitKind::WorkIcon, handle);
            }
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────

/// `worst_detected_type >= threshold` check.
///
/// The enum order is Enemy(0) < Body(1) < Object(2) < Friend(3) <
/// MissedFriend(4) < Beggar(5) < None(6), and the "worst" type is the
/// LOWEST index (Enemy is worst). So `>=` here means "at least this
/// severe or more benign".
fn detectable_ge(worst: DetectableType, threshold: DetectableType) -> bool {
    worst as u8 >= threshold as u8
}

/// Map an [`crate::element::WorkIcon`] to the corresponding [`SpriteRow`].
///
/// Returns 0 (Impact, unused) for `WorkIcon::None` — callers should
/// avoid adding a WorkIcon titbit when the work is None anyway.
fn work_icon_to_sprite_row(work: crate::element::WorkIcon) -> u16 {
    use crate::element::WorkIcon;
    match work {
        WorkIcon::Arrows => SpriteRow::WorkIconArrows as u16,
        WorkIcon::Purses => SpriteRow::WorkIconPurses as u16,
        WorkIcon::Stones => SpriteRow::WorkIconStones as u16,
        WorkIcon::Apples => SpriteRow::WorkIconApples as u16,
        WorkIcon::Beer => SpriteRow::WorkIconBeer as u16,
        WorkIcon::Legs => SpriteRow::WorkIconLegs as u16,
        WorkIcon::Plants => SpriteRow::WorkIconPlants as u16,
        WorkIcon::Nets => SpriteRow::WorkIconNets as u16,
        WorkIcon::Wasps => SpriteRow::WorkIconWasps as u16,
        WorkIcon::BowTraining => SpriteRow::WorkIconBowTraining as u16,
        WorkIcon::SwordTraining => SpriteRow::WorkIconSwordTraining as u16,
        WorkIcon::Regeneration => SpriteRow::WorkIconRegeneration as u16,
        WorkIcon::None => 0,
    }
}

/// Map an [`EmoticonType`] to the corresponding [`SpriteRow`].
fn emoticon_to_sprite_row(emoticon: EmoticonType) -> u16 {
    match emoticon {
        EmoticonType::None => 0,
        EmoticonType::GrowingQuestionMark => SpriteRow::EmoticonGrowingQMark as u16,
        EmoticonType::QuestionMark => SpriteRow::EmoticonQMark as u16,
        EmoticonType::XMark => SpriteRow::EmoticonXMark as u16,
        EmoticonType::Zzz => SpriteRow::EmoticonZzz as u16,
        EmoticonType::Cloud => SpriteRow::EmoticonCloud as u16,
        EmoticonType::Sun => SpriteRow::EmoticonSun as u16,
        EmoticonType::Thunderstorm => SpriteRow::EmoticonThunderstorm as u16,
        EmoticonType::Drunken => SpriteRow::EmoticonDrunken as u16,
    }
}

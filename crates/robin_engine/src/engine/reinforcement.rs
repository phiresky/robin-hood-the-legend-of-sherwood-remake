//! Runtime PC spawn for the `ALARM` console cheat / Robin's on-mission
//! death reinforcement.
//!
//! Picks a random reinforcement door, pulls an un-instanced non-VIP
//! peasant from the gang, spawns a new PC, registers it as an
//! enemy-detectable for every NPC, and launches a
//! `PASS_DOOR → MOVE(jitter)` sequence.
//!
//! Runs inside `perform_hourglass` (see `EngineInner::drain_pending_reinforcements`)
//! so sim-state mutation stays confined to the tick. The reinforcement
//! PC's sprite is preloaded at level-load time by
//! `preload_campaign_peasant_sprites` so the cache-only
//! `Sprite::load_frame_info_cached` call here always hits.

use super::EngineInner;
use crate::coordinates::{MapPoint, MapVec};
use crate::element::{
    ActorData, ActorPc, Command, Detectable, DetectableType, ElementData, ElementKind, Entity,
    EntityId, HULK_LENGTH, HumanData, PcData,
};
use crate::engine::LevelAssets;
use crate::order::OrderType;
use crate::sequence::{MoveFlags, Sequence, SequenceElement, SequenceElementData};

impl EngineInner {
    /// Drain the deferred reinforcement queue.
    ///
    /// Runs from `perform_hourglass` so the sim-state mutation it
    /// performs (new entities, `sim_rng` draws, `instanced` flags)
    /// lives inside the replayed tick and rollback stays deterministic.
    /// Reinforcement-PC sprites are preloaded at level-load by
    /// [`EngineInner::preload_campaign_peasant_sprites`], so this path
    /// only reads the scriptor cache (`&LevelAssets`).
    pub(crate) fn drain_pending_reinforcements(&mut self, assets: &LevelAssets) {
        if self.pending_reinforcements.is_empty() {
            return;
        }
        let requests: Vec<Option<EntityId>> = std::mem::take(&mut self.pending_reinforcements);
        for dead_pc in requests {
            self.create_reinforcement(assets, dead_pc);
        }
    }

    fn create_reinforcement(&mut self, assets: &LevelAssets, dead_pc: Option<EntityId>) {
        // Pick a random reinforcement door.
        let door_count = self.ai_global.reinforcement_doors.len();
        if door_count == 0 {
            tracing::warn!("REINFORCEMENT: no reinforcement doors on this level.");
            return;
        }
        let pick = crate::sim_rng::usize(0..door_count);
        let door_index = self.ai_global.reinforcement_doors[pick].door_index;

        // Snapshot door geometry — we'll drop the host borrow before
        // touching entities / the campaign.
        let door_snap = {
            let Some(script) = self.mission_script.as_mut() else {
                return;
            };
            let Some(game_host) = script.game_host_mut() else {
                return;
            };
            let Some(door) = game_host.doors.get(usize::from(door_index)) else {
                return;
            };
            DoorSnapshot {
                point_out: door.point_out,
                point_in: door.point_in,
                layer_out: door.layer_out,
                layer_in: door.layer_in,
                sector_out: u16::from(door.sector_out),
            }
        };

        // Preferred profile: the dead PC's profile index (if any).
        let preferred_profile_idx = dead_pc
            .and_then(|id| self.get_entity(id))
            .and_then(|e| e.pc_data())
            .map(|pc| pc.profile_index);

        // Pick the peasant via the campaign helper.
        let Some(campaign) = self.campaign.as_mut() else {
            return;
        };
        let Some(char_idx) =
            campaign.get_random_peasant_from_gang(preferred_profile_idx, &assets.profile_manager)
        else {
            tracing::info!("REINFORCEMENT: no eligible peasant in gang.");
            return;
        };
        let profile_idx = match campaign
            .characters
            .get(char_idx)
            .and_then(|d| d.character_profile_idx)
        {
            Some(p) => p,
            None => return,
        };

        // Mark the PcDescription as instanced.
        if let Some(desc) = campaign.characters.get_mut(char_idx) {
            desc.instanced = true;
        }

        // Resolve the CharacterProfile (clone what we need so we can
        // drop the campaign borrow before mutating entities).
        let (profile_filename, profile_name, kind, has_lockpick, has_climb, has_jump) = {
            let Some(profile) = assets.profile_manager.get_character(profile_idx) else {
                return;
            };
            let kind = crate::character_kind::CharacterKind::from_profile(
                &profile.filename,
                &profile.profile_name,
            );
            let (has_lockpick, has_climb, has_jump) = PcData::movement_auth_from_profile(profile);
            (
                profile.filename.clone(),
                profile.profile_name.clone(),
                kind,
                has_lockpick,
                has_climb,
                has_jump,
            )
        };

        // Hide the dead PC's interface.  Reinforcement only fires
        // mid-mission so the Sherwood-display guard is a no-op anyway.
        // The replacement PC's portrait re-display is implicit: the new
        // entity below is built with `PcData::default()`, which leaves
        // `interface_hidden = false`.
        if let Some(dead_id) = dead_pc
            && let Some(pc) = self.get_entity_mut(dead_id).and_then(|e| e.pc_data_mut())
        {
            pc.interface_hidden = true;
        }

        // Resolve the sprite from the preloaded scriptor cache. The
        // gang-peasant sprites are all loaded at mission start by
        // `preload_campaign_peasant_sprites`, so a miss here means
        // preload missed a profile — treat as a bug rather than a
        // silent fallback.
        let mut sprite = crate::sprite::Sprite::default();
        if let Err(e) = sprite.load_frame_info_cached(
            &assets.sprite_scriptor,
            crate::sprite_script::FrameKind::Character,
            &profile_filename,
            &profile_name,
        ) {
            tracing::error!(
                "REINFORCEMENT: sprite cache lookup failed for '{}' (profile {}): {e}",
                profile_name,
                profile_idx,
            );
            return;
        }

        // Build the new PC entity.  Uses door outer-point / outer-layer
        // as the spawn site — i.e. the new PC appears *outside* the map
        // and then walks in via the `PASS_DOOR` element below.
        //
        // `sector_out == 0xFFFF` is the null-sector sentinel (no
        // projection area).
        let obstacle_index = if door_snap.sector_out == 0xFFFF {
            None
        } else {
            self.get_projection_area_index(
                assets,
                door_snap.sector_out,
                door_snap.layer_out,
                door_snap.point_out,
            )
        };
        let spawn_sector = crate::position_interface::SectorHandle::new(door_snap.sector_out);

        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            sprite,
            ..Default::default()
        };
        element.set_position_map(door_snap.point_out);
        element.set_layer(door_snap.layer_out);
        element.set_sector(spawn_sector);
        if let Some(obs) = obstacle_index.and_then(crate::position_interface::ObstacleHandle::new) {
            // `set_obstacle` always pulls the top-plane from the
            // obstacle whenever the obstacle is non-null — pre-resolve
            // the plane here.
            let plane = crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                Some(obs),
                assets.static_sight_obstacles.as_slice(),
            );
            element.sprite.position_iface.set_obstacle(Some(obs), plane);
        }
        let entity = Entity::Pc(ActorPc {
            element,
            actor: ActorData::default(),
            human: HumanData {
                time_hulk: HULK_LENGTH,
                ..Default::default()
            },
            pc: PcData {
                robin: false,
                profile_index: profile_idx,
                kind,
                has_lockpick,
                has_climb,
                has_jump,
                beam_me_index: -1,
                ..Default::default()
            },
        });
        let new_id = self.add_entity(entity);

        // Register the new PC as a visible enemy for every NPC.
        self.add_detectable_for_all_npc(new_id, DetectableType::Enemy);

        // Build the PASS_DOOR → MOVE sequence: a movement element
        // through the gate (DOOR | MAP flags), followed by a straight
        // movement to a jitter point inside.
        let action = OrderType::WalkingUpright;
        let mut seq = Sequence::new();

        let mut pass = SequenceElement::new_movement(1, Command::PassDoor, Some(new_id), action);
        pass.data = SequenceElementData::Movement {
            destination: door_snap.point_in,
            layer: door_snap.layer_in,
            sector: None,
            gate_id: Some(door_index),
            line_id: None,
            element: None,
            flags: MoveFlags::DOOR | MoveFlags::MAP,
            tolerance: 0.0,
            direction: 0,
            action,
            speed_factor: 1.0,
            post_seek_sequence: None,
        };
        seq.append_element(pass);

        // Find a jitter point near the inside of the door: up to ten
        // tries, ±50 in each axis.
        let pin = door_snap.point_in;
        let hd = self
            .get_entity(new_id)
            .map(|e| e.position_iface())
            .map(|pi| pi.get_half_diagonal())
            .unwrap_or_else(|| crate::geo2d::pt(12.0, 8.0));
        let mut jitter: Option<MapPoint> = None;
        for _ in 0..10 {
            let dx = crate::sim_rng::i32(-50..=50) as f32;
            let dy = crate::sim_rng::i32(-50..=50) as f32;
            let candidate = pin + MapVec::new(dx, dy);
            if self
                .fast_grid
                .is_reachable_thick(pin, candidate, door_snap.layer_in, hd)
            {
                jitter = Some(candidate);
                break;
            }
        }
        if let Some(target) = jitter {
            let mut mv = SequenceElement::new_movement(2, Command::Move, Some(new_id), action);
            mv.data = SequenceElementData::Movement {
                destination: target,
                layer: door_snap.layer_in,
                sector: None,
                gate_id: None,
                line_id: None,
                element: None,
                flags: MoveFlags::STRAIGHT,
                tolerance: 0.0,
                direction: 0,
                action,
                speed_factor: 1.0,
                post_seek_sequence: None,
            };
            seq.append_element(mv);
        }

        let seq_id = self.launch_sequence(seq);
        tracing::info!(
            new_pc = ?new_id,
            ?seq_id,
            profile = %profile_idx,
            door = %door_index,
            "REINFORCEMENT: spawned reinforcement PC."
        );
    }

    /// Appends a `Detectable` pointing at `element_id` to every NPC's
    /// list slot for `det_type`.  Skips duplicates.  For
    /// `DetectableType::Enemy`, also applies the per-NPC camp/rank
    /// filter so only NPCs whose `AddDetectable` arm accepts the target
    /// are populated.
    pub(crate) fn add_detectable_for_all_npc(
        &mut self,
        element_id: EntityId,
        det_type: DetectableType,
    ) {
        let idx = det_type as usize;
        if idx >= DetectableType::COUNT {
            return;
        }
        // Pre-resolve target classification once for the per-NPC filter.
        let target_info = self.get_entity(element_id).map(|e| {
            (
                e.is_pc(),
                e.is_soldier(),
                e.is_civilian(),
                e.camp(),
                e.is_human(),
            )
        });
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            // For DETECTABLE_ENEMY, only push if the NPC's camp/rank
            // arm accepts the target.  Other arms unconditionally push.
            if det_type == DetectableType::Enemy {
                let Some((tgt_pc, tgt_soldier, _tgt_civilian, tgt_camp, tgt_human)) = target_info
                else {
                    continue;
                };
                if !tgt_human {
                    // ENEMY targets must be human.
                    continue;
                }
                let Some(npc_entity) = self.get_entity(npc_id) else {
                    continue;
                };
                if !crate::ai_detectable_filter::should_add_enemy_detectable(
                    npc_entity.camp(),
                    npc_entity.is_soldier(),
                    tgt_pc,
                    tgt_soldier,
                    tgt_camp,
                ) {
                    continue;
                }
            }
            if let Some(entity) = self.get_entity_mut(npc_id) {
                let npc = match entity {
                    Entity::Soldier(s) => Some(&mut s.npc),
                    Entity::Civilian(c) => Some(&mut c.npc),
                    _ => None,
                };
                let Some(npc) = npc else { continue };
                if idx >= npc.detectable_lists.len() {
                    continue;
                }
                let already = npc.detectable_lists[idx]
                    .iter()
                    .any(|d| d.element == Some(element_id));
                if !already {
                    npc.detectable_lists[idx].push(Detectable {
                        element: Some(element_id),
                        detectable_type: det_type,
                        ..Default::default()
                    });
                }
            }
        }
    }
}

struct DoorSnapshot {
    point_out: MapPoint,
    point_in: MapPoint,
    layer_out: u16,
    layer_in: u16,
    sector_out: u16,
}

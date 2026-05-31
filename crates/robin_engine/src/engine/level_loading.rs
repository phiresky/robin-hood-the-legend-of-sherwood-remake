//! Level loading, entity spawning, and background initialization.

use super::scroll_reveal::ScrollStatus;
use super::*;
use crate::coordinates::{MapBBox, MapPoint};
use crate::element::{BonusItemTypeExt, Entity, EntityId};

/// CPU-decoded background map ready for GPU upload.
///
/// Produced by [`EngineInner::pre_decode_background_map`] (slow — bzip2) and
/// consumed by [`EngineInner::apply_background_map`] (fast — GPU upload).
/// Mask compositing is no longer done CPU-side — `mask_overlay.wgsl`
/// samples the live bg texture in the fragment stage.
pub struct PreDecodedBackground {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u16>,
}

/// CPU-decoded minimap ready for GPU upload.  See [`PreDecodedBackground`].
pub struct PreDecodedMinimap {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u16>,
}

/// Minimap bitmap metadata produced by the host after GPU upload and
/// consumed by [`super::Engine::apply_level_bitmaps_loaded`] to finish
/// the minimap-widget wiring (hit mask, map size, initial position).
///
/// `saved_position` is the persisted top-left from the active player
/// profile (`(65536, 65536)` sentinel when never written).  The engine
/// validates it via `MinimapState::set_minimap_position`, snapping to
/// the default corner if the saved point is the sentinel or fully
/// off-screen.
pub struct MinimapBitmapSetup {
    pub hit_mask: crate::minimap::HitMask,
    pub map_size: crate::coordinates::MinimapSize,
    pub saved_position: crate::coordinates::ScreenPoint,
}

/// Look up the sprite file and profile name for a raw bonus type value.
///
/// One entry per bonus type, registering the RHS file and profile name.
/// The raw value comes from the mission file and corresponds to the
/// bonus-type enum (0=Arrow, 1=Stone, …, 18=SwordOfTheState).
pub(crate) fn bonus_type_to_sprite_asset(
    raw_bonus_type: u16,
) -> Option<(&'static str, &'static str, crate::element::ObjectType)> {
    use crate::element::ObjectType;
    match raw_bonus_type {
        0 => Some(("BONUS_Arrows", "BONUS Fleches", ObjectType::BonusArrow)),
        1 => Some(("BONUS_Stones", "BONUS Cailloux", ObjectType::BonusStone)),
        2 => Some(("BONUS_Apples", "BONUS Pommes", ObjectType::BonusApple)),
        3 => Some(("BONUS_Ale", "BONUS Ale", ObjectType::BonusAle)),
        4 => Some(("BONUS_LegOfLamb", "BONUS Gigots", ObjectType::BonusLambLeg)),
        5 => Some(("BONUS_Plants", "BONUS Plantes", ObjectType::BonusPlants)),
        6 => Some(("BONUS_Nets", "BONUS Filets", ObjectType::BonusNet)),
        7 => Some(("BONUS_WaspsNest", "BONUS Guepes", ObjectType::BonusWaspNest)),
        8 => Some((
            "BONUS_MoneyBag",
            "BONUS Bourses d'argent",
            ObjectType::BonusPurse,
        )),
        9 => Some((
            "BONUS_GoldBagsRansom",
            "BONUS Sac d'or rancon",
            ObjectType::BonusRansom,
        )),
        10 => Some((
            "BONUS_FourLeavedClover",
            "BONUS Trefle",
            ObjectType::BonusAmulet,
        )),
        11 => Some(("BONUS_Shield", "Shield", ObjectType::BonusBlazon)),
        12 => Some(("RELIC_Ampulla", "Huile", ObjectType::BonusAmpulla)),
        13 => Some(("RELIC_Spoon", "Cuillere", ObjectType::BonusCoronationSpoon)),
        14 => Some(("RELIC_Crown", "Couronne", ObjectType::BonusRichardsCrown)),
        15 => Some(("RELIC_Stamp", "Sceau", ObjectType::BonusRoyalSeal)),
        16 => Some(("RELIC_Sceptre", "Sceptre", ObjectType::BonusRoyalSceptre)),
        17 => Some(("RELIC_Book", "Registre", ObjectType::BonusDomesdayBook)),
        18 => Some(("RELIC_Sword", "Epee", ObjectType::BonusSwordOfTheState)),
        _ => None,
    }
}

/// Map a beam-me `actionInitial` value to a `(posture, action_state)`
/// Load the raw mission + proto-level binaries for a campaign's current
/// mission.  Standalone helper so the host can parse the mission header
/// (map filename + ambiance) *before* constructing an `Engine`,
/// allowing the background bitmap to be pre-decoded with real
/// dimensions and passed into `Engine::new` as a first-class input —
/// the RAII shape where engine construction fully initializes every
/// field, rather than leaving `fast_grid.map_bbox` zero until a
/// post-construction fixup.
///
/// Handles only the file-I/O half of mission setup; the engine-mutation
/// half (entity spawn, pending motion stash) still lives on
/// `EngineInner::initialize_from_mission`.
pub fn load_mission_for_campaign(
    campaign: &crate::campaign::Campaign,
    profiles: &crate::profiles::ProfileManager,
    level_directory: &str,
    progress: &mut dyn FnMut(f32),
) -> Result<crate::level_data::LoadedLevel, EngineError> {
    let idx = campaign
        .current_mission_idx
        .expect("load_mission_for_campaign: no current mission set");
    let profile = campaign.missions[idx].profile(profiles);
    let mission_filename = &profile.mission_filename;
    let proto_level_filename = &profile.proto_level_filename;

    // The is_beggar predicate is needed because beggar civilians have
    // extra scroll-set data in the mission file.  We parse raw data
    // before constructing entities so we pass the check as a closure.
    crate::level_data::load_level(
        mission_filename,
        proto_level_filename,
        level_directory,
        &|profile_id| {
            profiles
                .get_civilian(profile_id)
                .is_some_and(|p| p.civilian_type == crate::profiles::CivilianType::Beggar)
        },
        progress,
    )
    .map_err(|e| EngineError::Io(std::io::Error::other(e.to_string())))
}

/// pair for a PC's initial action.
///
/// The raw value is an animation ordinal; unknown values fall back to
/// `(Upright, Waiting)` with a warning log.
fn map_pc_initial_action(
    raw_action: u32,
) -> (crate::element::Posture, crate::element::ActionState) {
    use crate::element::{ActionState, Posture};
    use crate::order::OrderType;

    let anim = match OrderType::try_from(raw_action) {
        Ok(a) => a,
        Err(_) => {
            tracing::warn!(
                "PC InitializeAction: unknown animation ordinal {raw_action}; \
                 defaulting to (Upright, Waiting)"
            );
            return (Posture::Upright, ActionState::Waiting);
        }
    };

    match anim {
        OrderType::WaitingUpright => (Posture::Upright, ActionState::Waiting),
        OrderType::WaitingUprightBored => (Posture::Upright, ActionState::Bored),
        OrderType::WaitingCrouched => (Posture::Crouched, ActionState::Waiting),
        OrderType::BeingDeadFallenBack => (Posture::DeadBack, ActionState::Waiting),
        OrderType::BeingDead => (Posture::Dead, ActionState::Waiting),
        OrderType::BeingUnconscious => (Posture::Lying, ActionState::Waiting),
        // `WaitingCape` → Spy posture (costume-as-civilian).  The Rust
        // titbit sync pass inserts the hidden titbit automatically on
        // hidden postures.
        OrderType::WaitingCape => (Posture::Spy, ActionState::Waiting),
        // `WaitingHidden` → Tree (hidden-in-bush posture).
        OrderType::WaitingHidden => (Posture::Tree, ActionState::Waiting),
        OrderType::Sitting => (Posture::Sitting, ActionState::Waiting),
        OrderType::SleepingUpright => (Posture::Siesta, ActionState::Waiting),
        OrderType::BeingTied => (Posture::Tied, ActionState::Waiting),
        // `Special` is unimplemented; fall through to (Upright, Waiting)
        // with a warning. The game does not ship any level with `Special`
        // as a PC initial action, so this path should be unreachable in
        // practice.
        OrderType::Special => {
            tracing::warn!(
                "PC InitializeAction: Special animation is unimplemented; \
                 defaulting to (Upright, Waiting)"
            );
            (Posture::Upright, ActionState::Waiting)
        }
        other => {
            tracing::warn!(
                "PC InitializeAction: unsupported initial animation {other:?}; \
                 defaulting to (Upright, Waiting)"
            );
            (Posture::Upright, ActionState::Waiting)
        }
    }
}

impl EngineInner {
    // ─── Accessory sprite hydration ──────────────────────────────

    /// Attach an accessory sprite (arrow/stone/apple/net/wasp/purse/
    /// coin/ale/cape) to a freshly-spawned projectile or object entity
    /// by cloning from the preloaded master prototype.
    ///
    /// Every projectile spawn pulls its sprite from a global master
    /// registry. The Rust port preloads every accessory sprite into
    /// `LevelAssets::accessory_sprite_prototypes` at level load and
    /// clones here per-spawn — tick paths don't need mutable asset
    /// access.
    ///
    /// No-op if the object type has no preloaded accessory prototype
    /// (bonus-type projectiles spawned as throws reuse the bonus-side
    /// sprite, also preloaded at level load).
    pub(crate) fn attach_accessory_sprite(
        &mut self,
        assets: &crate::engine::LevelAssets,
        id: crate::element::EntityId,
    ) {
        let Some(entity) = self.entities.get(id) else {
            return;
        };
        let object_type = match entity {
            crate::element::Entity::Projectile(p) => p.object.object_type,
            crate::element::Entity::Net(n) => n.object.object_type,
            // Ground-dropped bonuses that carry an *accessory* object
            // type (e.g. ObjectType::Ale dropped by DropAle — see
            // `spawn_dropped_ale`) need the preloaded ACCESSORIES
            // sprite cloned in just like a projectile.  Pre-placed
            // BONUS_* bonuses already have their sprite loaded inline
            // at mission spawn time and will not be found in the
            // accessory table, so the `get(&object_type) = None` below
            // makes this a no-op for them — no need to gate explicitly.
            crate::element::Entity::Bonus(b) => b.object.object_type,
            _ => return,
        };
        let Some(prototype) = assets.accessory_sprite_prototypes.get(&object_type) else {
            return;
        };
        let sprite = prototype.clone();
        if let Some(entity) = self.entities.get_mut(id) {
            entity.element_data_mut().sprite = sprite;
        }
    }

    /// Preload accessory sprite prototypes at level-load time.
    ///
    /// Called from `initialize_from_mission` after the sprite bank is
    /// available.  Loads one sprite per accessory `ObjectType` (or
    /// `BonusNet`/`BonusWaspNest` for throw-a-pickup projectiles) into
    /// `LevelAssets::accessory_sprite_prototypes`; runtime
    /// [`attach_accessory_sprite`] calls then clone from that table.
    pub(crate) fn preload_accessory_sprite_prototypes(assets: &mut crate::engine::LevelAssets) {
        use crate::element::ObjectType;
        assets.accessory_sprite_prototypes.clear();
        let char_base_dir = "Data/Characters";
        let bank_signature = assets.bank_signature;
        // Every accessory registered as a master, plus the two `Bonus*`
        // types the throw-pickup projectile paths reuse.
        let entries: &[(ObjectType, &str, &str)] = &[
            (ObjectType::Arrow, "ACCESSORIES_Arrow", "ACCESSOIRES Fleche"),
            (
                ObjectType::Stone,
                "ACCESSORIES_Stone",
                "ACCESSOIRES Cailloux",
            ),
            (ObjectType::Ale, "ACCESSORIES_Ale", "ACCESSOIRES Ale"),
            (ObjectType::Apple, "ACCESSORIES_Apple", "ACCESSOIRES Pomme"),
            (
                ObjectType::Purse,
                "ACCESSORIES_MoneyBag",
                "ACCESSOIRES Bourse d'argent",
            ),
            (
                ObjectType::WaspNest,
                "ACCESSORIES_Wasp",
                "ACCESSOIRES Guepes",
            ),
            (ObjectType::Cape, "ACCESSORIES_Coat", "Manteau"),
            (ObjectType::Net, "ACCESSORIES_Net", "ACCESSOIRES Filet"),
            (
                ObjectType::Coin,
                "ACCESSORIES_Coin",
                "ACCESSOIRES Piece d'or",
            ),
            (ObjectType::Wasp, "ACCESSORIES_WaspSting", "Guepe"),
            (ObjectType::BonusNet, "BONUS_Nets", "BONUS Filets"),
            (ObjectType::BonusWaspNest, "BONUS_WaspsNest", "BONUS Guepes"),
        ];
        for (object_type, file, profile) in entries {
            let mut sprite = crate::sprite::Sprite::default();
            if let Err(e) = sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                crate::sprite_script::FrameKind::Object,
                char_base_dir,
                file,
                profile,
                bank_signature,
                None,
            ) {
                tracing::error!(
                    "Failed to preload accessory sprite '{file}' profile '{profile}': {e}",
                );
                continue;
            }
            assets
                .accessory_sprite_prototypes
                .insert(*object_type, sprite);
        }
    }

    /// Preload the scroll-amulet bonus sprite
    /// (`BONUS_FourLeavedClover` / `"BONUS Trefle"`).
    ///
    /// Called at level load so the mid-tick scroll-reveal path
    /// ([`Self::drain_pending_scroll_amulets`]) can hit the scriptor
    /// cache through `&LevelAssets` instead of needing `&mut` to
    /// load on demand (which would break the
    /// "mutation-only-in-perform_hourglass" invariant).
    pub(crate) fn preload_scroll_amulet_sprite(&mut self, assets: &mut crate::engine::LevelAssets) {
        let bank_signature = assets.bank_signature;
        let mut sprite = crate::sprite::Sprite::default();
        if let Err(e) = sprite.load_frame_info(
            assets.sprite_scriptor_mut(),
            crate::sprite_script::FrameKind::Object,
            "Data/Characters",
            "BONUS_FourLeavedClover",
            "BONUS Trefle",
            bank_signature,
            Some(self.weather.ambiance.to_sprite_ambiance()),
        ) {
            tracing::error!("Failed to preload scroll-amulet sprite: {e}");
        }
        // We only care that the scriptor cache is populated; discard
        // the Sprite itself — the runtime spawn builds its own.
        drop(sprite);
    }

    /// Preload character sprites for every non-VIP gang peasant who
    /// could be drafted as a reinforcement.
    ///
    /// The reinforcement spawn ([`Self::drain_pending_reinforcements`])
    /// picks a random non-instanced, non-VIP peasant from the current
    /// gang. That pool is known at level load, so we can eagerly load
    /// each candidate's `.rhs` into the scriptor cache and the mid-tick
    /// spawn path can then use the cache-only `&SpriteScriptor`
    /// accessor.
    ///
    /// Safe to call repeatedly — `SpriteScriptor::load` short-circuits
    /// on cache hit, so re-preloading is a no-op beyond a few hashmap
    /// probes.
    pub(crate) fn preload_campaign_peasant_sprites(
        &mut self,
        assets: &mut crate::engine::LevelAssets,
    ) {
        let Some(campaign) = self.campaign.as_ref() else {
            return;
        };
        let bank_signature = assets.bank_signature;
        // Snapshot the profiles we need so we can drop the campaign
        // borrow before mutating assets.
        let profiles: Vec<(String, String)> = campaign
            .gang_indices
            .iter()
            .filter_map(|&gi| {
                let desc = campaign.characters.get(gi)?;
                let cpi = desc.character_profile_idx?;
                let profile = assets.profile_manager.get_character(cpi)?;
                if profile.vip {
                    return None;
                }
                Some((profile.filename.clone(), profile.profile_name.clone()))
            })
            .collect();
        for (filename, profile_name) in profiles {
            let mut sprite = crate::sprite::Sprite::default();
            if let Err(e) = sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                crate::sprite_script::FrameKind::Character,
                "Data/Characters",
                &filename,
                &profile_name,
                bank_signature,
                Some(self.weather.ambiance.to_sprite_ambiance()),
            ) {
                tracing::warn!(
                    "Failed to preload reinforcement sprite '{filename}' / '{profile_name}': {e}",
                );
            }
        }
    }

    // ─── Level loading ───────────────────────────────────────────

    /// Load a level from proto-level + mission files.
    ///
    /// This reads chunk-based binary files: the proto-level contains
    /// geometry (motion, sight, patches, etc.) and the mission file
    /// contains actors, scripts, and gameplay data.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn initialize_from_mission(
        &mut self,
        assets: &mut LevelAssets,
        pending: &mut PendingLevelData,
        mission_name: &str,
        proto_level_name: &str,
        mut loaded: crate::level_data::LoadedLevel,
        level_directory: &str,
        bg_pixel_dims: (f32, f32),
        progress: &mut dyn FnMut(f32),
    ) -> Result<(), EngineError> {
        self.script_globals.clear();
        self.mission_stat.reset();
        self.short_briefings.clear();

        self.campaign
            .as_ref()
            .expect("Campaign must be set before initialize_from_mission");
        let profiles = assets.profile_manager.clone();

        // The SIGHT chunk lists which CHUNK_MATERIAL sectors participate
        // in spatial material queries — only those are registered into
        // the fast-grid's per-block SECTOR_SOUND buckets at layer 0.
        // Material sectors present in CHUNK_MATERIAL but absent from
        // this list exist only as per-obstacle `material_indices`
        // references and are invisible to `GetSectors(SECTOR_SOUND)`
        // callers (footstep lookup, projectile water/hole impact
        // detection).  Filter the raw list so `MaterialSectors::material_at`
        // and `WaterZones` see the same subset.
        //
        // Empty `sight_material_indices` means the level has no SIGHT
        // chunk at all (test fixtures, or broken data) — preserve the
        // original pre-filter behaviour of including every material
        // sector rather than silently blanking material lookup.
        let filtered_material_sectors: Vec<crate::level_data::RawMaterialSector> =
            if loaded.proto.sight_material_indices.is_empty() {
                loaded.proto.material_sectors.clone()
            } else {
                loaded
                    .proto
                    .sight_material_indices
                    .iter()
                    .filter_map(|&idx| {
                        loaded
                            .proto
                            .material_sectors
                            .get(idx as usize)
                            .cloned()
                            .or_else(|| {
                                tracing::error!(
                                    "SIGHT chunk references material sector index {idx} but only \
                                 {} material sectors exist — dropping reference",
                                    loaded.proto.material_sectors.len()
                                );
                                None
                            })
                    })
                    .collect()
            };
        tracing::debug!(
            "SIGHT material-list gate: {} / {} material sectors active for spatial lookup",
            filtered_material_sectors.len(),
            loaded.proto.material_sectors.len()
        );

        // Water/hole zones for projectile splash detection. Material
        // WATER/HOLE sectors used by projectile splash detection go
        // through `GetSectors(SECTOR_SOUND)`, so the filter above applies.
        assets.water_zones =
            crate::water_zones::WaterZones::build_from_raw(&filtered_material_sectors);

        // SECTOR_SOUND registry for footstep material lookup.
        // Used by `set_obstacle_and_material` when no projection-area
        // obstacle is available.
        let default_material_code = loaded
            .proto
            .misc
            .as_ref()
            .map(|m| m.default_material)
            .unwrap_or(0);
        assets.material_sectors = crate::material_sectors::MaterialSectors::build_from_raw(
            &filtered_material_sectors,
            default_material_code,
        );

        // Build LINE_SOUND grid lines for every SIGHT-listed material
        // polygon: for every material index in the SIGHT chunk's list,
        // build a non-motion LINE_SOUND line per polygon edge at
        // layer 0.  Without this, the actor-side `CheckForLineCrossing`
        // LINE_SOUND arm has nothing to detect and cross-zone material
        // refresh stalls — actors keep the material from their last
        // elevation crossing or door-pass even after walking onto a
        // different sound-material polygon.
        //
        // The polygon goes to both layers (so mouse picking and sight
        // queries see it) but lines only go to layer 0 (sound
        // boundaries are layer-flat in the shipped data).  We only
        // need the lines for the per-tick crossing dispatch — the
        // polygon containment scan still runs through `MaterialSectors`.
        for ms in &assets.material_sectors.sectors {
            if ms.points.len() < 2 {
                continue;
            }
            self.fast_grid.add_sector_lines_for_sound(
                0, // layer 0
                &ms.points, true, // material polygons are always active in shipped levels
            );
        }
        if !assets.material_sectors.sectors.is_empty() {
            tracing::debug!(
                "Registered LINE_SOUND grid lines for {} SIGHT-listed material polygons",
                assets.material_sectors.sectors.len()
            );
        }

        // Warn when the mission header's control CRC differs from the
        // proto-level misc chunk's CRC — a cheap sanity check that the
        // mission file matches the proto-level it was authored against.
        if let Some(ref misc) = loaded.proto.misc
            && loaded.mission.header.control_crc != misc.control_crc
        {
            tracing::warn!(
                "Proto/mission CRC mismatch: proto misc control_crc=0x{:08X}, \
                 mission header control_crc=0x{:08X} — proto-level and mission \
                 file may be mismatched",
                misc.control_crc,
                loaded.mission.header.control_crc,
            );
        }

        // Apply mission header
        self.weather.ambiance = Ambiance::from_raw(loaded.mission.header.ambiance);
        // Install the initial view-polygon radius from the ambiance:
        // DAY / ATTACK / CUSTOM_1..4 → 400, FOG / NIGHT → 300.  Without
        // this seed, Fog/Night missions whose StartUp script does not
        // call `SetViewRadius(300)` would run with NPCs whose view
        // radius falls back to DEFAULT_VIEW_RADIUS (400) in the AI
        // vision path, detecting PCs from further away than in the
        // original game.  Script opcodes (engine/script.rs
        // `SetViewRadius`) can still overwrite this.
        self.standard_view_polygon_radius = self.weather.ambiance.default_view_polygon_radius();
        self.mission.map_name = loaded.mission.header.map_filename.clone();
        assets.script_hiking_path_count = loaded.mission.hiking_paths.len();

        // Set building count for script handle validation.
        // Only count actual Building entries, not StandaloneDoors.
        assets.script_building_count = loaded
            .proto
            .buildings
            .iter()
            .filter(|e| matches!(e, crate::level_data::RawBuildingEntry::Building { .. }))
            .count();

        // Set script location count and extract positions (points + lines + sectors).
        //
        // Script objects are laid out `[points ...] [lines ...] [sectors ...]`
        // and `GetLocationScript` indexes into the combined array directly.
        // Preserve that layout literally — including the empty lines slab — so
        // the index space matches the original.  `lines` is empty on every
        // shipped mission today (the only line-creation site was a dead branch
        // for an old level format); if a future stream version ever
        // re-introduces lines, the index will shift correctly without a code
        // change here.
        if let Some(ref so) = loaded.mission.script_objects {
            assets.script_location_count = so.points.len() + so.lines.len() + so.sectors.len();
            assets.script_point_count = so.points.len();
            assets.script_location_positions.clear();
            assets.script_location_layers.clear();
            assets.script_location_sectors.clear();
            // Points come first in the combined index.
            for pt in &so.points {
                assets
                    .script_location_positions
                    .push((pt.x as f32, pt.y as f32));
                assets.script_location_layers.push(pt.layer);
                assets.script_location_sectors.push(pt.sector);
            }
            // Lines slot into the middle of the index space; midpoint is the
            // natural representative position.  Empty in shipped data — see
            // `RawScriptLine` for the dead-branch rationale.
            for line in &so.lines {
                let mx = (line.x1 as f32 + line.x2 as f32) * 0.5;
                let my = (line.y1 as f32 + line.y2 as f32) * 0.5;
                assets.script_location_positions.push((mx, my));
                assets.script_location_layers.push(line.layer);
                assets.script_location_sectors.push(line.sector);
            }
            // Sectors follow; use polygon centroid as their position.
            for sec in &so.sectors {
                let (cx, cy) = if sec.polygon.points.is_empty() {
                    (0.0, 0.0)
                } else {
                    let n = sec.polygon.points.len() as f32;
                    let sum_x: f32 = sec.polygon.points.iter().map(|p| p.0 as f32).sum();
                    let sum_y: f32 = sec.polygon.points.iter().map(|p| p.1 as f32).sum();
                    (sum_x / n, sum_y / n)
                };
                assets.script_location_positions.push((cx, cy));
                assets.script_location_layers.push(sec.layer);
                assets.script_location_sectors.push(sec.sector_ref);
            }

            // Register script zone sectors on the fast-find grid so we can
            // do point-in-polygon occupant checks during gameplay.
            assets.script_zone_grid_indices.clear();
            self.script_zone_data.clear();
            for sec in &so.sectors {
                // Nudge every polygon vertex by `Y += 0.000348367f` to
                // avoid integer-aligned vertices confusing point-in-polygon
                // tests against actor positions on integer Y boundaries.
                let pts: Vec<MapPoint> = sec
                    .polygon
                    .points
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x as f32, y as f32 + 0.000348367))
                    .collect();
                let mut bbox = MapBBox::new();
                for &p in &pts {
                    bbox.expand_point(p);
                }

                // When scripts are disabled (`-NOSCRIPT`), the sector is
                // flagged unassociated even if a script class is named.
                let scripts_enabled = crate::engine::GlobalOptions::global()
                    .as_ref()
                    .map(|o| o.script_enabled)
                    .unwrap_or(true);
                let mut script_data = crate::sector::ScriptSectorData::new();
                script_data.script_associated = sec.script_class.is_some() && scripts_enabled;
                script_data.script_class_name = sec.script_class.clone();

                // Script sectors default to `CROSS | SCRIPT`; the CROSS bit
                // is cleared when the sector has no script class bound.
                let mut sector_type = crate::sector::SectorType::SCRIPT;
                if script_data.script_associated {
                    sector_type |= crate::sector::SectorType::CROSS;
                }

                // We keep the grid registration unconditionally to preserve
                // the 1:1 index mapping between `script_zone_grid_indices`
                // and the script sector index space (a sparse form would
                // ripple through `tick_zone_occupants` and zone-script
                // handle math).  `GridSector::contains_point` already
                // returns `false` for polygons with fewer than 3 vertices,
                // so a degenerate sector is effectively empty — but we
                // still surface the authoring error loudly.
                if pts.len() < 3 {
                    tracing::error!(
                        "Script sector (class {:?}, layer {}) has only {} polygon \
                         points — containment tests will never match (need >= 3)",
                        sec.script_class,
                        sec.layer,
                        pts.len(),
                    );
                }

                let grid_idx = self.fast_grid.add_sector(
                    crate::fast_find_grid::GridSector {
                        points: pts,
                        bounding_box: bbox,
                        sector_type,
                        layer: sec.layer,
                        sector_number: crate::sector::SectorNumber::new(-1), // script sectors don't have proto sector numbers
                        door_index: None,
                        lift_type: None,
                        lift_direction: 0,
                        force_crouched: false,
                        building_index: None,
                        low_exit_point: None,
                        high_exit_point: None,
                        lowest_door_index: None,
                        jump_line_indices: Vec::new(),
                        gate_indices: Vec::new(),
                        underlying_sector: None,
                    },
                    sec.layer,
                );
                assets.script_zone_grid_indices.push(grid_idx);
                let zone_idx = self.script_zone_data.len();
                self.script_zone_data.push(script_data);

                // Each polygon edge becomes a `LINE_SCRIPT | LINE_CROSS`
                // line carrying a back-pointer to the owning script zone
                // so the actor zone-crossing dispatch in `engine::script`
                // can fire on cross.
                if let Ok(zone_idx_u16) = u16::try_from(zone_idx) {
                    self.fast_grid.add_sector_lines_for_script(
                        grid_idx,
                        sec.layer,
                        zone_idx_u16,
                        true, // script sectors default to active
                    );
                } else {
                    tracing::warn!(
                        "Script zone index {} exceeds u16::MAX; skipping LINE_SCRIPT wiring",
                        zone_idx
                    );
                }
            }
            if !assets.script_zone_grid_indices.is_empty() {
                tracing::info!(
                    "Registered {} script zone sectors on grid",
                    assets.script_zone_grid_indices.len()
                );
            }
        }

        // Store hiking paths for patrol route lookups by AI.
        assets.script_hiking_path_count = loaded.mission.hiking_paths.len();
        assets.hiking_paths = std::sync::Arc::new(std::mem::take(&mut loaded.mission.hiking_paths));

        // Build the global SeekPoint / AmbushPoint / Archery arrays from
        // raw tactic data: reset the existing lists, then fan out to the
        // per-sub-chunk installers.  Reinforcement doors are handled
        // further below, after `populate_game_host_from_level` has created
        // the proto-level doors those entries share a table with.
        self.ai_global.reset_seek_points();
        self.ai_global.reset_ambush_points();
        if let Some(ref tactic) = loaded.mission.tactic_data {
            for raw in &tactic.seek_points {
                let dir = crate::ai::SeekPointDirection {
                    position: crate::ai::Position {
                        x: raw.x as f32,
                        y: raw.y as f32,
                        sector: crate::position_interface::SectorHandle::new(raw.sector),
                        level: raw.level,
                    },
                    direction: raw.direction,
                };
                self.ai_global.add_seek_point_direction(&dir);
            }
            tracing::debug!(
                "Loaded {} raw seek-point directions → {} unified seek points",
                tactic.seek_points.len(),
                self.ai_global.seek_points.len(),
            );

            // Install ambush points.
            // `position_3d` and `id` get fixed up later by the AI-init
            // loop at `engine/ai.rs`.
            for raw in &tactic.ambush_points {
                self.ai_global.ambush_points.push(crate::ai::AmbushPoint {
                    position: crate::ai::Position {
                        x: raw.x as f32,
                        y: raw.y as f32,
                        sector: crate::position_interface::SectorHandle::new(raw.sector),
                        level: raw.level,
                    },
                    direction: 0,
                    position_3d: crate::coordinates::WorldPoint3D::default(),
                    id: 0,
                });
            }
            if !tactic.ambush_points.is_empty() {
                tracing::debug!(
                    "Loaded {} ambush points into AiGlobalState",
                    tactic.ambush_points.len(),
                );
            }

            // Wire archery sectors into AiGlobalState.
            // Archery sectors are populated during InitAI from tactic data.
            self.ai_global.reset_archery_sectors();
            for raw in &tactic.archery_sectors {
                // Resolve the referenced sector through `sector_number_map`
                // so we can read its layer.
                let sector_layer = self
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&crate::sector::SectorNumber::new(raw.sector_ref as i16))
                    .and_then(|&idx| self.fast_grid.level.sectors.get(idx))
                    .map(|gs| gs.layer)
                    .unwrap_or(0);
                let mut index_first_shooting: Option<crate::sector::ArcheryPointIdx> = None;
                let mut index_last_shooting: Option<crate::sector::ArcheryPointIdx> = None;
                let mut num_shooting: u16 = 0;
                let n_points = raw.points.len();
                let points: Vec<crate::ai::PointArchery> = raw
                    .points
                    .iter()
                    .enumerate()
                    .map(|(i, rp)| {
                        if rp.is_shooting_point {
                            let idx = crate::sector::ArcheryPointIdx(i as u16);
                            if index_first_shooting.is_none_or(|first| idx < first) {
                                index_first_shooting = Some(idx);
                            }
                            if index_last_shooting.is_none_or(|last| idx > last) {
                                index_last_shooting = Some(idx);
                            }
                            num_shooting += 1;
                            // A shooting-point at either end of the way is
                            // a fatal authoring error.  Surface it as a
                            // loud runtime error so malformed ARCH/NLIP
                            // chunks are caught instead of silently
                            // producing a one-sided way.
                            if n_points >= 2 && (i == 0 || i + 1 == n_points) {
                                tracing::error!(
                                    "Archery sector {}: shooting point at way endpoint \
                                     (index {} of {}) — way is malformed",
                                    raw.sector_ref,
                                    i,
                                    n_points,
                                );
                            }
                        }
                        // Each waypoint carries the layer of the motion-area
                        // sector it references, which can differ from the
                        // archery sector's own layer.  Resolve each point's
                        // sector through `sector_number_map` to get the
                        // right layer.
                        let point_layer = self
                            .fast_grid
                            .level
                            .sector_number_map
                            .get(&crate::sector::SectorNumber::new(rp.sector as i16))
                            .and_then(|&idx| self.fast_grid.level.sectors.get(idx))
                            .map(|gs| gs.layer)
                            .unwrap_or(sector_layer);
                        crate::ai::PointArchery {
                            position: crate::ai::Position {
                                x: rp.x as f32,
                                y: rp.y as f32,
                                sector: crate::position_interface::SectorHandle::new(rp.sector),
                                level: point_layer,
                            },
                            direction: rp.direction,
                            is_shooting_point: rp.is_shooting_point,
                            sector_index: crate::sector::SectorNumber::new(rp.sector as i16),
                            owner: None,
                        }
                    })
                    .collect();
                // An archery way must have at least 3 points.
                if n_points < 3 {
                    tracing::error!(
                        "Archery sector {}: way has only {} points (need >= 3)",
                        raw.sector_ref,
                        n_points,
                    );
                }
                let polygon: Vec<(f32, f32)> = raw
                    .polygon
                    .points
                    .iter()
                    .map(|&(x, y)| (x as f32, y as f32))
                    .collect();
                self.ai_global
                    .archery_sectors
                    .push(crate::ai::SectorArchery {
                        points,
                        polygon,
                        layer: sector_layer,
                        index_first_shooting_point: index_first_shooting,
                        index_last_shooting_point: index_last_shooting,
                        num_shooting_points: num_shooting,
                        num_owners: 0,
                    });
            }
            if !tactic.archery_sectors.is_empty() {
                tracing::debug!(
                    "Loaded {} archery sectors into AiGlobalState",
                    tactic.archery_sectors.len(),
                );
            }
        }

        // Mobile entities (carts/trains/ships) are a Spellbound engine
        // leftover that no shipped Robin Hood mission uses — the runtime
        // was never ported and the entity types were deleted.  Tracked
        // in the deferred parity notes under "Spellbound engine leftovers".

        // Convert raw sight obstacles into SightObstacle instances for AI
        // line-of-sight checks.
        //
        // Static (load-time) obstacles live in `LevelAssets::static_sight_obstacles`
        // (Arc-shared so per-frame `EngineInner::clone` is a refcount bump).
        // Per-frame dynamic obstacles (shields) get appended to
        // `EngineInner::dynamic_sight_obstacles` later by `update_shield_obstacles`.
        //
        // Per-obstacle material sub-sectors index into the **unfiltered**
        // CHUNK_MATERIAL list, which holds every CHUNK_MATERIAL entry
        // regardless of SIGHT-list inclusion.  The SIGHT filter only
        // gates the global SECTOR_SOUND fast-find registry (already
        // applied above for `assets.material_sectors`).
        let raw_material_default = crate::element::GameMaterial::from_u32(default_material_code);
        let all_material_sectors = &loaded.proto.material_sectors;
        let static_obstacles: Vec<crate::sight_obstacle::SightObstacle> = loaded
            .proto
            .sight_obstacles
            .iter()
            .enumerate()
            .map(|(idx, raw)| {
                use crate::sight_obstacle::{
                    ObstaclePoint, SIGHTOBSTACLE_MOUSE, SIGHTOBSTACLE_OPAQUE,
                    SIGHTOBSTACLE_PROJECTION_AREA, SIGHTOBSTACLE_SHOW_SHADOW_POLYGON,
                    SIGHTOBSTACLE_SOLID, SightObstacle,
                };
                let mut flags: u32 = 0;
                if raw.opaque {
                    flags |= SIGHTOBSTACLE_OPAQUE;
                }
                if raw.solid {
                    flags |= SIGHTOBSTACLE_SOLID;
                }
                // MOUSE is only set when SOLID.
                if raw.solid && raw.mouse {
                    flags |= SIGHTOBSTACLE_MOUSE;
                }
                if raw.show_shadow_polygon {
                    flags |= SIGHTOBSTACLE_SHOW_SHADOW_POLYGON;
                }
                if raw.projection_area.is_some() {
                    flags |= SIGHTOBSTACLE_PROJECTION_AREA;
                }

                let mut obs = SightObstacle::new(idx as u32, flags);
                obs.obstacle_points = raw
                    .points
                    .iter()
                    .map(|p| ObstaclePoint {
                        x: p.x,
                        y: p.y,
                        z_top: p.z_top,
                        z_bottom: p.z_bottom,
                    })
                    .collect();
                obs.material = raw.default_material;
                // Build per-obstacle material sub-sectors from the
                // unfiltered CHUNK_MATERIAL list.  Drives projectile
                // material determination on heterogeneous obstacle
                // surfaces (e.g. stone inlay on a wooden platform).
                obs.material_sectors = raw
                    .material_indices
                    .iter()
                    .filter_map(|&mi| {
                        let r = all_material_sectors.get(mi as usize).or_else(|| {
                            tracing::error!(
                                "SightObstacle {idx} references material sector {mi} \
                                 but only {} exist — dropping reference",
                                all_material_sectors.len()
                            );
                            None
                        })?;
                        crate::material_sectors::MaterialSector::from_raw(r, raw_material_default)
                    })
                    .collect();
                if let Some((sector, layer)) = raw.projection_area {
                    obs.sector = sector;
                    obs.layer = layer;
                }
                // Copy each referenced material sector from the global
                // material-sector list onto the obstacle.  We resolve
                // indices into clones of the polygon data so subsequent
                // reads (e.g. branches 2 & 3 of `DetermineWaterHole`)
                // don't need to chase a separate global table at the
                // call site.  Indices that fall outside the proto
                // material-sector array are dropped with a warning
                // rather than panicking — we don't want to crash the
                // renderer over a bad asset reference, but the issue
                // should still surface.
                obs.material_sectors =
                    raw.material_indices
                        .iter()
                        .filter_map(|&idx| {
                            let raw_sector = loaded
                                .proto
                                .material_sectors
                                .get(idx as usize)
                                .or_else(|| {
                                    tracing::warn!(
                                        "Sight obstacle {} references material sector {} but \
                                     only {} material sectors exist — dropping reference",
                                        idx,
                                        idx,
                                        loaded.proto.material_sectors.len()
                                    );
                                    None
                                })?;
                            if raw_sector.polygon.points.len() < 3 {
                                return None;
                            }
                            let points: Vec<MapPoint> = raw_sector
                                .polygon
                                .points
                                .iter()
                                .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                                .collect();
                            let mut bbox = crate::coordinates::MapBBox::new();
                            for &p in &points {
                                bbox.expand_point(p);
                            }
                            // Same material-code → GameMaterial mapping
                            // as `MaterialSectors::build_from_raw` (clamp
                            // out-of-range / LIGHT_SHADOW to default).
                            const N_MATERIALS: u32 = 9;
                            let code = raw_sector.material as u32;
                            let material = if code >= N_MATERIALS {
                                crate::element::GameMaterial::from_u32(default_material_code)
                            } else {
                                crate::element::GameMaterial::from_u32(code)
                            };
                            Some(crate::material_sectors::MaterialSector {
                                points,
                                bounding_box: bbox,
                                material,
                            })
                        })
                        .collect();
                // Capture vertices 0/1/2 as (point3, point1, point2) and
                // seed the top/bottom planes from (point1, point2, point3).
                // Orientation flip is skipped because `compute_plane_z` is
                // symmetric in point order.
                if obs.obstacle_points.len() >= 3 {
                    let p0 = &obs.obstacle_points[0];
                    let p1 = &obs.obstacle_points[1];
                    let p2 = &obs.obstacle_points[2];
                    obs.top_plane_points = [
                        [p1.x, p1.y, p1.z_top],
                        [p2.x, p2.y, p2.z_top],
                        [p0.x, p0.y, p0.z_top],
                    ];
                    obs.bottom_plane_points = [
                        [p1.x, p1.y, p1.z_bottom],
                        [p2.x, p2.y, p2.z_bottom],
                        [p0.x, p0.y, p0.z_bottom],
                    ];
                }
                obs.rebuild_geometry();
                obs
            })
            .collect();
        let n = static_obstacles.len();
        self.dynamic_sight_obstacles.clear();
        self.static_sight_obstacle_active = vec![true; n];
        assets.static_sight_obstacles = std::sync::Arc::new(static_obstacles);
        tracing::info!("Loaded {} sight obstacles for AI line-of-sight", n);
        progress(1.0);

        // ── Sound sources ──
        // Convert raw sound sources from the proto level into the SoundManager's
        // source manager, filtering by the current ambiance bitmask.
        {
            use crate::sound_geometry::SoundSourceAltitude;
            use crate::sound_source::{SoundSource, SoundSourceKind};
            use std::collections::BTreeSet;

            let ambiance_mask = self.weather.ambiance.to_bitmask();
            let mut required_ids = BTreeSet::new();

            for raw in &loaded.proto.sound_sources {
                if (raw.ambience_filter & ambiance_mask) == 0 {
                    // Source not in this ambiance — push None to preserve indices
                    self.sound_sim.sources.sources_push_none();
                    continue;
                }

                let source_kind = SoundSourceKind::from_u8(raw.source_kind)
                    .unwrap_or_else(|| panic!("Invalid sound source kind: {}", raw.source_kind));

                let (min_delay, max_delay, delay_stepping) =
                    if let Some((min, max, step)) = raw.delayed_params {
                        (min, max, step + 1) // delay_stepping is pre-incremented
                    } else {
                        (0, 0, 1)
                    };

                let shape: Vec<MapPoint> = raw
                    .polyline
                    .as_ref()
                    .map(|pts| {
                        pts.iter()
                            .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                            .collect()
                    })
                    .unwrap_or_default();

                // Scale volumes from 0–100 range to 0–255
                let inner_volume = raw
                    .inner_volume
                    .map(|v| {
                        let clamped = v.min(100);
                        (clamped as f32 * 2.55) as u16
                    })
                    .unwrap_or(0);
                let outer_volume = raw
                    .outer_volume
                    .map(|v| {
                        let clamped = v.min(100);
                        (clamped as f32 * 2.55) as u16
                    })
                    .unwrap_or(0);

                let altitude = match raw.altitude {
                    0 => SoundSourceAltitude::Ground,
                    1 => SoundSourceAltitude::Middle,
                    2 => SoundSourceAltitude::Top,
                    3 => SoundSourceAltitude::NoAltitude,
                    _ => panic!("Invalid sound source altitude: {}", raw.altitude),
                };

                let source = SoundSource {
                    ambiences: raw.ambience_filter,
                    source_kind,
                    id: raw.id as u32,
                    is_global: raw.global,
                    inner_distance: raw.inner_distance.unwrap_or(0),
                    outer_distance: raw.outer_distance.unwrap_or(0),
                    noise_covering_distance: raw.noise_covering_distance.unwrap_or(0),
                    inner_volume,
                    outer_volume,
                    shape,
                    altitude,
                    min_delay,
                    max_delay,
                    delay_stepping,
                    timer: 0,
                    active: raw.active,
                };

                required_ids.insert(source.id);
                self.sound_sim.sources.sources_push_some(source);
            }

            tracing::info!(
                "Loaded {} sound sources ({} active, {} required samples)",
                loaded.proto.sound_sources.len(),
                self.sound_sim.sources.iter_active().count(),
                required_ids.len(),
            );

            // Store on level assets for host-side `setup_mission_audio`
            // to populate the sound-cache source map.
            assets.sound_source_required_ids = required_ids;
        }
        progress(1.0);

        // Rewire building-door sector_in/layer_in to point at the empty
        // BUILDING grid sectors that `initialize_motion_from_level_data` will
        // create later.  This has to run *before* `populate_game_host_from_level`
        // so the `game_host.doors` list stores the rewritten values.  The matching
        // grid sectors are registered later, in the motion-init pass, using
        // the sector numbers we stash on constructor-local pending data.
        self.rewire_building_doors(
            pending,
            &mut loaded.proto.buildings,
            loaded.proto.motion_data.as_ref(),
        );

        // Store motion data for processing when the background bitmap
        // is applied (grid sector registration needs map dimensions).
        pending.motion_data = loaded.proto.motion_data.take();

        // Pre-load *only* the move-box half-diagonal table from the
        // motion-data proto stream, so the soldier / civilian / PC
        // spawn blocks below can size each actor's `move_box` from
        // the real pathfinder profile instead of the `(-1,-1,1,1)`
        // fallback.  The rest of the pathfinder graph (sectors,
        // obstacles, links) still loads later in
        // `initialize_motion_from_level_data` because sector
        // registration needs `map_bbox`, which isn't known until the
        // background bitmap has been decoded.
        // `load_from_proto_stream` detects the already-populated table
        // and skips re-pushing.
        if let Some(ref motion_data) = pending.motion_data
            && !motion_data.graph_bytes.is_empty()
            && let Err(e) = std::sync::Arc::make_mut(&mut assets.pathfinder_graph)
                .preload_half_diagonals_from_proto(&mut self.fast_grid, &motion_data.graph_bytes)
        {
            tracing::error!(
                "Failed to pre-load pathfinder half-diagonals (soldier move_boxes will fall back): {e}"
            );
        }
        // Stash raw masks; converted into RuntimeMask and pushed into the
        // fast grid once layers are allocated in
        // `initialize_motion_from_level_data`.
        pending.masks = std::mem::take(&mut loaded.proto.masks);
        // Stash lift proto data alongside motion data for sector fixup.
        // Clone rather than take — populate_game_host_from_level still needs them.
        pending.lifts = loaded.proto.lifts.clone();
        // Stash elevation (bond) lines so `initialize_motion_from_level_data`
        // can register them into the fast grid once layers are allocated.
        pending.elevation_lines = std::mem::take(&mut loaded.proto.elevation_lines);
        // Stash jump-zone + jump-line-pair data for post-sector processing
        // in `load_jump_lines_from_proto`.
        pending.jump_zones = std::mem::take(&mut loaded.proto.jump_zones);
        pending.jump_line_pairs = std::mem::take(&mut loaded.proto.jump_line_pairs);
        // Stash light/shadow sectors so `initialize_motion_from_level_data`
        // can register them into the grid once layers are allocated and
        // sector numbers have been assigned to the motion / lift / building
        // sectors.
        pending.light_sectors = std::mem::take(&mut loaded.proto.light_sectors);

        // Load order (ProtoStream → MissionStream): size the grid and
        // register every motion sector / lift / mask / elevation-line
        // BEFORE any mission entity spawns.  The beam-me / soldier /
        // civilian sector validations all assume the fast-grid sector
        // lookup is populated at this point.  Deferring sector
        // registration to after PC spawn would make every beam-me
        // sector check return "no sector".
        self.set_level_size(bg_pixel_dims.0, bg_pixel_dims.1);
        self.consume_pending_motion_data(assets, pending);

        // Set forest_level from proto misc — must happen before entity
        // spawning uses it to decide CHARACTER vs CHARACTER_BLIPPED.
        self.weather.is_forest_level = loaded.proto.misc.as_ref().is_some_and(|m| m.forest_level);

        // Character sprite loading parameters
        let char_base_dir = "Data/Characters";
        let bank_signature = assets.bank_signature;
        let frame_kind = if self.weather.is_forest_level {
            crate::sprite_script::FrameKind::Character
        } else {
            crate::sprite_script::FrameKind::CharacterBlipped
        };

        // ── Entity spawn order ──
        //
        // Elements are added to the script-elements array in the order
        // they appear in the proto + mission files. Script handles carry
        // tagged 0-based indices into this array, so getting the order
        // right is essential — otherwise script natives like `Deactivate(GetActorScript(N))`
        // hit the wrong entity, leaving initially-hidden enemies/scrolls/FX
        // visible at mission start.
        //
        // Load order:
        //   1. Proto FX animations  (loaded before mission file)
        //   2. Mission ELEMENT chunk sub-chunks in file order:
        //        BETE animals (skipped) → GOOD beam-mes (no script entry) →
        //        CIVI civilians → PRIS PCs-to-rescue → EVIL soldiers → TGET targets
        //   3. BONU bonuses
        //   4. PARC scrolls
        //   5. GUYS tenants  (not ported as entities — see note below;
        //                      `InitOccupant` consumes GUYS already)
        //   6. PCs from beam-mes  (one slot per beam-me, NULL if unfilled)
        //
        // Some types (PRIS, GUYS) are not yet spawned; we push None placeholders
        // for them so the script-position-to-entity-index mapping stays aligned.

        // Spawn patch FX entities (door/trap overlay animations).
        // Patches are loaded from the PATCH chunk before the ANIMATION
        // chunk, so patch FX entities appear first in the element array.
        {
            let anim_base_dir = "Data/Animations";
            let sprite_ambiance = Some(self.weather.ambiance.to_sprite_ambiance());
            let bank_signature = assets.bank_signature;
            let mut patch_entity_handles: Vec<Option<i32>> = Vec::new();

            for (patch_idx, raw) in loaded.proto.patches.iter().enumerate() {
                let fname = &raw.element_fx.sprite.frame_profile_name;
                let profile = &raw.element_fx.sprite.profile_name;

                if fname.is_empty() {
                    patch_entity_handles.push(None);
                    continue;
                }

                let mut sprite = crate::sprite::Sprite::default();
                match crate::sprite_script::SpriteScriptor::resolve_rhs_path(
                    crate::sprite_script::FrameKind::Animation,
                    anim_base_dir,
                    fname,
                    sprite_ambiance,
                ) {
                    Ok(path) => {
                        let cache_key = format!("{fname}/{profile}");
                        match assets.sprite_scriptor_mut().load(&path, profile, &cache_key, crate::sprite_script::FrameKind::Animation, |file| {
                            let mut sig = 0u32;
                            file.serialize_u32(&mut sig)
                                .map_err(|e| format!("read signature: {e}"))?;
                            if sig != bank_signature {
                                return Err(format!(
                                    "bank signature mismatch: file {sig:#x} != bank {bank_signature:#x}"
                                ));
                            }
                            Ok(())
                        }) {
                            Ok(info) => {
                                sprite.scripts = info.scripts.clone();
                                sprite.conversion = info.conversion.clone();
                                sprite.frame_profile_name = fname.clone();
                                sprite.profile_cache_key = cache_key;
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to load sprite for patch {patch_idx} animation '{fname}' profile '{profile}': {e}"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to resolve RHS path for patch {patch_idx} animation '{fname}': {e}"
                        );
                    }
                }

                // Determine initial active state from start_animation_valid.
                let initially_active = raw.start_animation_valid;
                sprite.apply_placement(
                    MapPoint::new(
                        raw.element_fx.sprite.position_x as f32,
                        raw.element_fx.sprite.position_y as f32,
                    ),
                    0,
                    None,
                    0,
                    crate::element::GameMaterial::default(),
                    None,
                    None,
                );

                // When `start_animation_valid`, park the FX element on the
                // initial animation row.  Without this, the patch starts on
                // `current_row=0` which only matches PATCH_INITIAL by
                // coincidence (depends on the sprite's conversion table);
                // patches whose conversion maps PATCH_INITIAL to a non-zero
                // row would render the wrong animation until the next
                // StartAnimation effect fires.
                if initially_active {
                    if let Some(row) = sprite.row_for_action(crate::order::OrderType::PATCH_INITIAL)
                    {
                        sprite.current_row = row;
                    }
                    // Reset the sprite frame to 0.
                    sprite.current_frame = 0;
                    sprite.frame_count = 0;
                }
                let entity = crate::element::Entity::Fx(crate::element::ElementFx {
                    element: crate::element::ElementData {
                        kind: crate::element::ElementKind::Fx,
                        active: initially_active,
                        sprite,
                        ..Default::default()
                    },
                    fx: crate::element::FxData {
                        restore_background: raw.integrate_in_background,
                        force_display: raw.element_fx.force_display,
                        animation: crate::order::OrderType::NonanimationEnd,
                        display_polyline: raw
                            .element_fx
                            .display_polyline
                            .iter()
                            .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                            .collect(),
                        patch_index: crate::patch::PatchIndex::new(patch_idx as u32),
                        // `rendering_properties = (blit_type != 0) ? NeedShadow : Blocky`.
                        rendering_properties: if raw.element_fx.blit_type != 0 {
                            crate::element::RenderingProperties::NeedShadow
                        } else {
                            crate::element::RenderingProperties::Blocky
                        },
                    },
                });
                let id = self.add_entity(entity);
                let handle = crate::natives::GameHost::actor_handle(id);
                patch_entity_handles.push(Some(handle));
                tracing::trace!(
                    "Spawned patch {patch_idx} FX entity: id={:?}, handle={handle}, sprite='{fname}'",
                    id,
                );
            }

            // Store the mapping for later GameHost population.
            // Will be transferred in populate_game_host_from_level below.
            assets.patch_entity_handles = patch_entity_handles;

            tracing::info!("Spawned {} patch FX entities", loaded.proto.patches.len(),);
        }
        progress(1.0);

        // Spawn proto-level FX animations (water, flags, decorations, etc.)
        {
            let anim_base_dir = "Data/Animations";
            let sprite_ambiance = Some(self.weather.ambiance.to_sprite_ambiance());
            let bank_signature = assets.bank_signature;

            for raw in &loaded.proto.animations {
                let fname = &raw.sprite.frame_profile_name;
                let profile = &raw.sprite.profile_name;

                // Resolve .rhs path and load sprite scripts
                let mut sprite = crate::sprite::Sprite::default();
                match crate::sprite_script::SpriteScriptor::resolve_rhs_path(
                    crate::sprite_script::FrameKind::Animation,
                    anim_base_dir,
                    fname,
                    sprite_ambiance,
                ) {
                    Ok(path) => {
                        let cache_key = format!("{fname}/{profile}");
                        match assets.sprite_scriptor_mut().load(&path, profile, &cache_key, crate::sprite_script::FrameKind::Animation, |file| {
                            let mut sig = 0u32;
                            file.serialize_u32(&mut sig)
                                .map_err(|e| format!("read signature: {e}"))?;
                            if sig != bank_signature {
                                return Err(format!(
                                    "bank signature mismatch: file {sig:#x} != bank {bank_signature:#x}"
                                ));
                            }
                            Ok(())
                        }) {
                            Ok(info) => {
                                sprite.scripts = info.scripts.clone();
                                sprite.conversion = info.conversion.clone();
                                sprite.frame_profile_name = fname.clone();
                                sprite.profile_cache_key = cache_key;
                            }
                            Err(e) => {
                                tracing::error!("Failed to load sprite scripts for animation '{fname}' profile '{profile}': {e}");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to resolve animation RHS path for '{fname}': {e}");
                    }
                }
                sprite.apply_placement(
                    MapPoint::new(raw.sprite.position_x as f32, raw.sprite.position_y as f32),
                    0,
                    None,
                    0,
                    crate::element::GameMaterial::default(),
                    None,
                    None,
                );
                let entity = Entity::Fx(crate::element::ElementFx {
                    element: crate::element::ElementData {
                        kind: crate::element::ElementKind::Fx,
                        active: raw.active,
                        sprite,
                        ..Default::default()
                    },
                    fx: crate::element::FxData {
                        restore_background: false,
                        force_display: raw.force_display,
                        animation: crate::order::OrderType::NonanimationEnd,
                        display_polyline: raw
                            .display_polyline
                            .iter()
                            .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                            .collect(),
                        patch_index: None, // background animations aren't patches
                        // `rendering_properties = (blit_type != 0) ? NeedShadow : Blocky`.
                        rendering_properties: if raw.blit_type != 0 {
                            crate::element::RenderingProperties::NeedShadow
                        } else {
                            crate::element::RenderingProperties::Blocky
                        },
                    },
                });
                // Proto-level background animations always load at
                // layer 0 / Z 0, so the background animation accessor
                // derives them directly from the entity store.
                let _ = self.add_entity(entity);
            }

            tracing::info!(
                "Spawned {} proto-level animations ({} with sprites)",
                loaded.proto.animations.len(),
                self.bg_animation_ids().len(),
            );
        }

        progress(1.0);

        // Every freshly-constructed NPC (soldier or civilian, regardless
        // of camp) seeds `invulnerable` from the global highlander2
        // option.  The launcher sets the global from `-highlander2`
        // cmdline and never clears it again, so any NPC spawned
        // post-startup is born invulnerable when the cheat is on.
        let highlander2 = crate::engine::GlobalOptions::global()
            .as_ref()
            .map(|o| o.highlander2)
            .unwrap_or(false);

        // Spawn civilians (CIVI sub-chunk, before soldiers in the ELEMENT chunk)
        for raw in &loaded.mission.civilians {
            let mut sprite = crate::sprite::Sprite::default();
            let civ_profile = profiles.get_civilian(raw.profile_number);

            if let Some(profile) = civ_profile {
                if let Err(e) = sprite.load_frame_info(
                    assets.sprite_scriptor_mut(),
                    frame_kind,
                    char_base_dir,
                    &profile.filename,
                    &profile.profile_name,
                    bank_signature,
                    Some(self.weather.ambiance.to_sprite_ambiance()),
                ) {
                    tracing::error!(
                        "Failed to load sprite for civilian profile {}: {e}",
                        raw.profile_number,
                    );
                }
            } else {
                tracing::error!("Civilian profile {} not found", raw.profile_number);
            }

            let (cached_camp, cached_civilian_type) = match civ_profile {
                Some(p) => (
                    if p.attitude == crate::profiles::Attitude::Hostile {
                        crate::element::Camp::Lacklandists
                    } else {
                        crate::element::Camp::Royalists
                    },
                    p.civilian_type,
                ),
                None => (
                    crate::element::Camp::Error,
                    crate::profiles::CivilianType::default(),
                ),
            };

            let mut ai = crate::ai_friendly::FriendlyAi::default();
            ai.base.path_id = crate::ai::PathId::new(raw.path_id);
            ai.base.initial_action = raw.action;

            // Civilians hardcode pathfinder index 0 and take the move box
            // from the grid's slot 0.
            let civ_half_diag = self.fast_grid.try_move_box_half_diagonal(0);
            sprite.position_iface.configure_for_actor(
                0,
                civ_half_diag,
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
            );
            sprite.apply_placement(
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
                raw.layer,
                crate::position_interface::SectorHandle::new(raw.sector),
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::from_u32(raw.material),
                crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );
            let entity = Entity::Civilian(crate::element::ActorCivilian {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ActorCivilian,
                    // Civilians are also blipped on non-forest levels,
                    // same as soldiers.
                    blipped: !self.weather.is_forest_level,
                    posture: crate::element::Posture::Upright,
                    sprite,
                    ..Default::default()
                },
                actor: crate::element::ActorData {
                    script_class: raw.script_class.clone().unwrap_or_default(),
                    ..Default::default()
                },
                human: crate::element::HumanData {
                    invulnerable: highlander2,
                    ..Default::default()
                },
                npc: crate::element::NpcData {
                    money: raw.money,
                    ai_brain: crate::element::AiBrain::Friendly(Box::new(ai)),
                    ..Default::default()
                },
                civilian: crate::element::CivilianData {
                    civilian_profile_index: crate::profiles::CivilianProfileIdx(raw.profile_number),
                    cached_camp,
                    cached_civilian_type,
                    beggar_scroll_sets: raw.beggar_scroll_sets.clone(),
                    ..Default::default()
                },
            });
            self.add_entity(entity);
            // Civilians contribute to the same level-money pool the
            // debriefing screen surfaces, even though the pool is named
            // "soldier_money".
            self.mission_stat.soldier_money += raw.money;
        }

        // PRIS sub-chunk: PCs to rescue.
        // These are full PC actors that the player must rescue during
        // the mission; spawned playable=false so they're NPCs until
        // rescued.
        for raw in &loaded.mission.pcs_to_rescue {
            let char_profile = profiles.get_character(raw.profile_index);

            let mut sprite = crate::sprite::Sprite::default();
            if let Some(profile) = char_profile {
                if let Err(e) = sprite.load_frame_info(
                    assets.sprite_scriptor_mut(),
                    crate::sprite_script::FrameKind::Character,
                    char_base_dir,
                    &profile.filename,
                    &profile.profile_name,
                    bank_signature,
                    Some(self.weather.ambiance.to_sprite_ambiance()),
                ) {
                    tracing::error!(
                        "Failed to load sprite for rescue PC profile {}: {e}",
                        raw.profile_index,
                    );
                }
            } else {
                tracing::error!("Rescue PC profile {} not found", raw.profile_index);
            }

            let kind = char_profile.and_then(|p| {
                crate::character_kind::CharacterKind::from_profile(&p.filename, &p.profile_name)
            });
            let is_robin = kind.is_some_and(|k| k.is_robin());
            let (has_lockpick, has_climb, has_jump) = char_profile
                .map(crate::element::PcData::movement_auth_from_profile)
                .unwrap_or((false, false, false));

            // Set the sprite's move box + pathfinder index from the
            // character profile right after `LoadFrameInfo`, the same
            // way the beam-me path does, so anti-collision has a valid
            // bbox on the very first tick.  Falls back cleanly when the
            // profile lookup missed — `configure_for_actor` is a no-op
            // with a zero half-diagonal in that case.
            let (pc_pathfinder_idx, pc_half_diag) = match char_profile {
                Some(profile) => (
                    profile.pathfinder_index,
                    self.fast_grid
                        .try_move_box_half_diagonal(profile.pathfinder_index as usize),
                ),
                None => (0, None),
            };
            let initial_position = MapPoint::new(raw.position_x as f32, raw.position_y as f32);
            sprite.position_iface.configure_for_actor(
                pc_pathfinder_idx,
                pc_half_diag,
                initial_position,
            );
            // The sector must be both motion and area; warn instead of
            // asserting so a corrupt mission file still loads.
            let sector_motion_area = self
                .fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(raw.sector as i16))
                .and_then(|&idx| self.fast_grid.level.sectors.get(idx))
                .map(|gs| gs.sector_type.is_motion() && gs.sector_type.is_area())
                .unwrap_or(false);
            if !sector_motion_area {
                tracing::warn!(
                    "Rescue PC profile {} at ({},{}) sector {} is not a motion+area sector",
                    raw.profile_index,
                    raw.position_x,
                    raw.position_y,
                    raw.sector,
                );
            }
            // Validate the rescue PC's obstacle: it must be a projection
            // area and the map position must be inside the obstacle's
            // screen box.  Warn instead of aborting so a corrupt mission
            // stream still loads.
            if raw.obstacle_index != 0xFFFF {
                match assets
                    .static_sight_obstacles
                    .get(raw.obstacle_index as usize)
                {
                    None => tracing::warn!(
                        "Rescue PC profile {} references out-of-range obstacle {}",
                        raw.profile_index,
                        raw.obstacle_index,
                    ),
                    Some(obs) => {
                        if !obs.is_projection_area() {
                            tracing::warn!(
                                "Rescue PC profile {} at ({},{}) not lying on projection area (obstacle {})",
                                raw.profile_index,
                                raw.position_x,
                                raw.position_y,
                                raw.obstacle_index,
                            );
                        }
                        if !obs.box_projection.contains_point(initial_position) {
                            tracing::warn!(
                                "Rescue PC profile {} at ({},{}) map position not lying in projection area screen box (obstacle {})",
                                raw.profile_index,
                                raw.position_x,
                                raw.position_y,
                                raw.obstacle_index,
                            );
                        }
                    }
                }
            }
            // `sprite.center` (loaded from the sprite info via
            // `load_frame_info` above) is the authoritative C++ sprite
            // anchor used by rendering and gameplay hotspot lookups.
            sprite.apply_placement(
                initial_position,
                raw.layer,
                crate::position_interface::SectorHandle::new(raw.sector),
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::from_u32(raw.material),
                crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );
            // Seed old position fields so `is_moving()` is false on the
            // first post-spawn tick.
            let current_position = sprite.position_iface.get_position();
            sprite.position_iface.set_old_position(current_position);
            sprite.position_iface.set_old_map_position(initial_position);
            // Display order is computed by the host-side
            // `compute_display_order` pass (`engine/display_state.rs`)
            // that runs before render and input hit-test on every tick
            // — the rescue PC is picked up automatically before any
            // consumer reads display order.

            // Map the PRIS chunk's authored animation to the starting
            // (posture, action_state) pair.  Without this, rescue PCs
            // always spawn in UPRIGHT/WAITING regardless of the
            // level-authored action.
            let (initial_posture, initial_action_state) = map_pc_initial_action(raw.action);

            // Create or reuse the campaign `PcDescription` before
            // spawning the entity so PcData can point at the same
            // dynamic status block the C++ PC's mpStatus referenced.
            //   * Non-VIP   → always a fresh description with full pockets.
            //   * VIP, none → same as non-VIP.
            //   * VIP, dup  → reuse the existing description and heal to
            //                 `LIFEPOINTS_PC` (guest-star return).
            // Previously this was deferred to `rescue_pc_by_profile_name`,
            // which left the reuse + heal branch unreachable for guest
            // stars and meant rescue PCs were not flagged `instanced` at
            // spawn.
            let profile_idx = crate::profiles::CharacterProfileIdx(raw.profile_index);
            let difficulty = crate::player_profile::DifficultyLevel::current();
            let char_idx = {
                let profile = char_profile
                    .unwrap_or_else(|| panic!("rescue PC profile {} not found", raw.profile_index));
                let campaign = self
                    .campaign
                    .as_mut()
                    .expect("Campaign must be set before spawning rescue PCs");
                let existing = campaign.get_character_by_profile(profile_idx);
                let char_idx = match (profile.vip, existing) {
                    (true, Some(idx)) => {
                        if let Some(desc) = campaign.characters.get_mut(idx) {
                            desc.status.life_points = crate::pc_status::LIFEPOINTS_PC;
                        }
                        idx
                    }
                    _ => {
                        let desc = crate::campaign::PcDescription {
                            character_profile_idx: Some(profile_idx),
                            instanced: false,
                            status: crate::pc_status::PcStatus::from_profile(
                                profile, true, difficulty,
                            ),
                        };
                        campaign.add_to_characters(desc, &profiles)
                    }
                };
                if let Some(desc) = campaign.characters.get_mut(char_idx) {
                    desc.instanced = true;
                }
                char_idx
            };
            let list_index = u8::try_from(char_idx)
                .expect("campaign character index must fit in PcData::list_index");

            let entity = Entity::Pc(crate::element::ActorPc {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ActorPc,
                    sprite,
                    posture: initial_posture,
                    ..Default::default()
                },
                actor: crate::element::ActorData {
                    script_class: raw.script_class.clone().unwrap_or_default(),
                    action_state: initial_action_state,
                    ..Default::default()
                },
                human: crate::element::HumanData {
                    time_hulk: crate::element::HULK_LENGTH,
                    ..Default::default()
                },
                pc: crate::element::PcData {
                    robin: is_robin,
                    profile_index: profile_idx,
                    list_index,
                    kind,
                    has_lockpick,
                    has_climb,
                    has_jump,
                    // This PC is a rescue target, not initially
                    // controllable.  The portrait bar will pick this up
                    // via entity state sync.
                    playable: false,
                    ..Default::default()
                },
            });
            self.add_entity(entity);
            // The low-priority idle order is enqueued by the post-spawn
            // `ensure_wait_element` loop further below, which iterates
            // every actor (rescue PCs included) before the first tick.
        }

        progress(1.0);

        // Spawn soldiers (EVIL sub-chunk)
        for raw in &loaded.mission.soldiers {
            let mut sprite = crate::sprite::Sprite::default();
            let soldier_profile = profiles.get_soldier(raw.profile_number);

            if let Some(profile) = soldier_profile {
                sprite
                    .load_frame_info(
                        assets.sprite_scriptor_mut(),
                        frame_kind,
                        char_base_dir,
                        &profile.filename,
                        &profile.profile_name,
                        bank_signature,
                        Some(self.weather.ambiance.to_sprite_ambiance()),
                    )
                    .map_err(|e| EngineError::ProfileSpriteLoadFailed {
                        kind: "soldier",
                        profile_id: raw.profile_number,
                        reason: e.to_string(),
                    })?;
            } else {
                tracing::error!("Soldier profile {} not found", raw.profile_number);
            }

            let (mut cached_max_lp, cached_camp) = match soldier_profile {
                Some(p) => (
                    p.life_point as i16,
                    if p.hostile {
                        crate::element::Camp::Lacklandists
                    } else {
                        crate::element::Camp::Royalists
                    },
                ),
                None => (100, crate::element::Camp::Error),
            };

            // Modify life points for Lacklandist (enemy) soldiers based
            // on difficulty level.  VIPs are excluded from the modifier.
            // We scale cached_max_lp itself so both cached_max_life_points
            // and initial life_points start at the difficulty-adjusted
            // value.
            if cached_camp == crate::element::Camp::Lacklandists
                && !soldier_profile.map(|p| p.vip).unwrap_or(false)
            {
                let diff = crate::player_profile::DifficultyLevel::current();
                cached_max_lp = diff.modify_capacity(
                    cached_max_lp as u16,
                    crate::player_profile::difficulty_params::EASY_ENEMY_LIFEPOINTS,
                    crate::player_profile::difficulty_params::HARD_ENEMY_LIFEPOINTS,
                    10000,
                ) as i16;
            }

            // drunk_level must fit in u8.
            assert!(
                raw.drunk_level < 256,
                "soldier drunk_level out of range: {}",
                raw.drunk_level
            );
            // company_number must fit in u16.
            assert!(
                raw.company_number < 0x10000,
                "soldier company_number out of range: {}",
                raw.company_number
            );

            // Build the AI controller now so init_ai picks it up later.
            // path_id / alert_path_id / initial_action / blood_alcohol
            // all live on the AI base.
            let mut ai = crate::ai_enemy::EnemyAi::new(0);
            ai.base.path_id = crate::ai::PathId::new(raw.path_id);
            ai.base.alert_path_id = crate::ai::PathId::new(raw.alert_path_id);
            ai.base.initial_action = raw.action;
            ai.base.blood_alcohol = raw.drunk_level as u8;
            // company_number is u16, range asserted above.
            ai.company_number = raw.company_number as u16;
            ai.tower_guard = raw.tower_guard;
            // Copy courage from soldier profile for the approach logic.
            // Also pull the soldier's sword range from the HtH weapon
            // profile's distance[Default] entry.
            if let Some(p) = soldier_profile {
                ai.soldier_profile_courage = p.courage;
                ai.soldier_profile_iq = p.intelligence;
                ai.soldier_profile_shooting = p.shooting;
                ai.soldier_profile_pride = p.pride;
                ai.soldier_profile_rank = p.rank;
                ai.soldier_profile_initiative = p.initiative;
                ai.soldier_profile_beer = p.beer;
                ai.soldier_profile_money = p.money;
                ai.soldier_profile_apple = p.apple;
                ai.soldier_profile_whistle = p.whistle;
                ai.soldier_profile_duty = p.duty;
                ai.soldier_profile_endurance = p.endurance;
                ai.is_vip = p.vip;
                ai.hth_weapon_id = p.hth_weapon_id;
                if let Some(weapon) = profiles.get_hth_weapon(p.hth_weapon_id) {
                    ai.sword_range =
                        weapon.distance[crate::weapons::WeaponDistance::Default as usize];
                    ai.sword_is_charge_weapon = weapon.charge;
                }
            }

            // Set the sprite's move box + pathfinder index from the
            // soldier profile right after `LoadFrameInfo`.
            let soldier_pathfinder_idx = soldier_profile.map(|p| p.pathfinder_index).unwrap_or(0);
            let soldier_half_diag = self
                .fast_grid
                .try_move_box_half_diagonal(soldier_pathfinder_idx as usize);
            if soldier_half_diag.is_none() {
                tracing::warn!(
                    pf_idx = soldier_pathfinder_idx,
                    table_len = self.fast_grid.level.move_box_half_diagonals.len(),
                    profile = raw.profile_number,
                    "BUG: soldier spawn: half-diag table empty → move_box falls back to \
                     (-1,-1,1,1); pathfinder proto loads after spawn_soldier and baked \
                     move_box breaks TestIfPathIsFine / anti-collision"
                );
            }
            sprite.position_iface.configure_for_actor(
                soldier_pathfinder_idx,
                soldier_half_diag,
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
            );
            sprite.apply_placement(
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
                raw.layer,
                crate::position_interface::SectorHandle::new(raw.sector),
                // Apply initial facing from level data (0-15 sector).
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::from_u32(raw.material),
                crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );

            let entity = Entity::Soldier(crate::element::ActorSoldier {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ActorSoldier,
                    // Non-forest levels start soldiers as blipped shadows that
                    // get revealed by proximity detection (SeesBlip) or the
                    // Listen ability.
                    blipped: !self.weather.is_forest_level,
                    // Default posture is Upright.  Without an explicit
                    // initializer posture defaults to `Undefined`, which
                    // stranded freshly-spawned soldiers because the
                    // `Command::Wait` fallback in tick.rs only maps known
                    // postures to idle animations — Undefined returned None
                    // and no bored animation got pushed.
                    posture: crate::element::Posture::Upright,
                    sprite,
                    ..Default::default()
                },
                actor: crate::element::ActorData {
                    // Record the script class name here; per-actor
                    // Initialize() is dispatched by initialize_mission_script.
                    script_class: raw.script_class.clone().unwrap_or_default(),
                    ..Default::default()
                },
                human: crate::element::HumanData {
                    invulnerable: highlander2,
                    ..Default::default()
                },
                npc: crate::element::NpcData {
                    // cached_max_lp was already difficulty-scaled above.
                    life_points: cached_max_lp,
                    money: raw.money,
                    ai_brain: crate::element::AiBrain::Enemy(Box::new(ai)),
                    ..Default::default()
                },
                soldier: crate::element::SoldierData {
                    soldier_profile_index: crate::profiles::SoldierProfileIdx(raw.profile_number),
                    cached_max_life_points: cached_max_lp,
                    cached_camp,
                    // Seed the cached rider flag from the profile at spawn,
                    // same pattern as `ai.is_vip = p.vip` above.
                    rider: soldier_profile.map(|p| p.rider).unwrap_or(false),
                    ..Default::default()
                },
            });
            let eid = self.add_entity(entity);
            // AiController was built with `EnemyAi::new(0)` above because
            // the entity id isn't known until `add_entity` returns — backfill
            // `ai.base.me` and `owner_entity_id` so trace logs, filter-event
            // dispatch, and any `self.me`/`self.owner_entity_id` reads see
            // the real id instead of 0.
            if let Some(e) = self.entities.get_mut(eid)
                && let Some(ai) = e.ai_controller_mut()
            {
                ai.me = eid.index();
                ai.owner_entity_id = Some(eid);
            }
            tracing::trace!(
                eid = eid.index(),
                path_id = raw.path_id,
                action = raw.action,
                "spawn soldier"
            );
            // Track soldier load-order → EntityId for patrol ID resolution.
            assets.all_soldier_entity_ids.push(eid);
            assets
                .soldier_subordinate_ids
                .push(raw.subordinate_ids.clone());
            // Every soldier contributes its money to the level pool, and
            // hostile (Lacklandists) soldiers increment the total soldier
            // count used by debriefing / campaign-stat sync.  Without this,
            // the "Level money" and "enemies encountered" rows on the
            // debriefing screen stay at 0 — and the `money` console cheat
            // miscomputes the delta.
            self.mission_stat.soldier_money += raw.money;
            if cached_camp == crate::element::Camp::Lacklandists {
                self.mission_stat.total_soldier_count += 1;
            }
        }

        progress(1.0);

        // Spawn targets.
        //
        // Each target stores its own RHS file name and profile in the
        // mission stream, loaded as an animation (from
        // `Data/Animations/<ambiance>/`), then `ForceAnimation(action,
        // direction)` is applied.
        let anim_base_dir = "Data/Animations";
        let sprite_ambiance = Some(self.weather.ambiance.to_sprite_ambiance());
        for raw in &loaded.mission.targets {
            let mut sprite = crate::sprite::Sprite::default();

            // Animations resolve through the ambiance-specific subdirectory,
            // so we go via `resolve_rhs_path` + `sprite_scriptor.load` rather
            // than the simpler `load_frame_info` helper (which assumes a
            // fixed `{base_dir}/{file}.rhs` layout).
            match crate::sprite_script::SpriteScriptor::resolve_rhs_path(
                crate::sprite_script::FrameKind::Animation,
                anim_base_dir,
                &raw.filename,
                sprite_ambiance,
            ) {
                Ok(path) => {
                    let cache_key = format!("{}/{}", raw.filename, raw.profile_name);
                    match assets.sprite_scriptor_mut().load(&path, &raw.profile_name, &cache_key, crate::sprite_script::FrameKind::Animation, |file| {
                        let mut sig = 0u32;
                        file.serialize_u32(&mut sig)
                            .map_err(|e| format!("read signature: {e}"))?;
                        if sig != bank_signature {
                            return Err(format!(
                                "bank signature mismatch: file {sig:#x} != bank {bank_signature:#x}"
                            ));
                        }
                        Ok(())
                    }) {
                        Ok(info) => {
                            sprite.scripts = info.scripts.clone();
                            sprite.conversion = info.conversion.clone();
                            sprite.center = info.center;
                            sprite.frame_profile_name = raw.filename.clone();
                            sprite.profile_cache_key = cache_key;
                            let w = info.size.x as u16;
                            let h = info.size.y as u16;
                            if w > sprite.current_width {
                                sprite.current_width = w;
                            }
                            if h > sprite.current_height {
                                sprite.current_height = h;
                            }

                            // Apply `ForceAnimation(action, direction)`.
                            match crate::order::OrderType::try_from(raw.action) {
                                Ok(anim) => sprite.force_animation(anim, raw.direction as u16),
                                Err(_) => tracing::error!(
                                    "Target action {} is not a valid OrderType — animation not forced",
                                    raw.action,
                                ),
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to load target sprite scripts for '{}' profile '{}': {e}",
                                raw.filename,
                                raw.profile_name,
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to resolve target animation RHS path for '{}': {e}",
                        raw.filename,
                    );
                }
            }

            // Rendering properties come from the blit type byte:
            //   0      → Blocky
            //   non-0  → NeedShadow
            let rendering_properties = if raw.blit_type != 0 {
                crate::element::RenderingProperties::NeedShadow
            } else {
                crate::element::RenderingProperties::Blocky
            };
            sprite.apply_placement(
                // First set the map position to the raw sprite position
                // so the plane projection can derive a baseline 3D
                // location. The map is later overwritten with the action
                // point (see below).
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
                raw.layer,
                crate::position_interface::SectorHandle::new(raw.sector),
                // Apply initial facing from level data (0-15 sector).
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::default(),
                crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );

            // When the authored Z is non-negative, override the
            // plane-derived 3D with an explicit lift.  Elevated targets
            // (wall-mounted levers, second-storey apple shelves) render
            // at the authored height and feed the correct Z into bow-aim
            // / 3D hit queries.  Negative `position_z` means "no explicit
            // lift; keep the plane projection computed above".
            if raw.position_z >= 0 {
                sprite
                    .position_iface
                    .set_position(crate::coordinates::WorldPoint3D {
                        x: raw.position_x as f32,
                        y: raw.position_y as f32 + raw.position_z as f32,
                        z: raw.position_z as f32,
                    });
            }
            // Capture the current 3D position into `set_old_position`
            // so sprite-motion diffs across the first tick see a stable
            // baseline rather than the pre-placement zero.
            let current_position = sprite.position_iface.get_position();
            sprite.position_iface.set_old_position(current_position);

            // Overwrite the map position with the action point *without*
            // touching the 3D elevation we just set.  The target renders
            // at the sprite position but its action point — the spot the
            // PC walks to when interacting — lives at action_position.
            let action_point =
                MapPoint::new(raw.action_position_x as f32, raw.action_position_y as f32);
            sprite
                .position_iface
                .set_map_position_preserving_3d(action_point);
            sprite.position_iface.set_old_map_position(action_point);
            let entity = Entity::Target(crate::element::ElementTarget {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::Target,
                    sprite,
                    ..Default::default()
                },
                fx: crate::element::FxData {
                    // Targets are primary gameplay elements and are always
                    // drawn regardless of FX display options.
                    force_display: true,
                    ..Default::default()
                },
                target: crate::element::TargetData {
                    action_filter: crate::element::TargetFilter::from_bits_truncate(
                        raw.action_filter,
                    ),
                    action_position: MapPoint::new(
                        raw.action_position_x as f32,
                        raw.action_position_y as f32,
                    ),
                    action_sector: raw.action_sector,
                    action_layer: raw.action_layer,
                    position_z: raw.position_z,
                    sprite_filename: raw.filename.clone(),
                    sprite_profile_name: raw.profile_name.clone(),
                    display_polyline: raw
                        .polyline
                        .iter()
                        .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                        .collect(),
                    rendering_properties,
                    // Per-target script class name. Empty string = no
                    // script, matching proto default.
                    script_class: raw.script_class.clone().unwrap_or_default(),
                    ..Default::default()
                },
            });
            self.add_entity(entity);
        }

        progress(1.0);

        // Preload accessory sprite prototypes (arrow/stone/apple/net/
        // wasp/purse/coin/ale/cape) — one master sprite per accessory
        // type. We lazy-preload per mission since sprite banks differ
        // across levels.
        Self::preload_accessory_sprite_prototypes(assets);
        // Preload sprites for sim-tick spawn paths so they can hit the
        // scriptor cache through `&LevelAssets` — enforces the "engine
        // mutation only during perform_hourglass" invariant.
        self.preload_scroll_amulet_sprite(assets);
        self.preload_campaign_peasant_sprites(assets);

        // Spawn bonuses.
        //
        // Each bonus type has its own RHS file and profile name living
        // next to the character sprites; the bonus is constructed from
        // the corresponding pre-loaded master sprite.
        for raw in &loaded.mission.bonuses {
            let (sprite_file, profile_name, object_type) =
                match bonus_type_to_sprite_asset(raw.bonus_type) {
                    Some(t) => t,
                    None => {
                        tracing::error!(
                            "Unknown bonus type {} in mission file — skipping",
                            raw.bonus_type,
                        );
                        continue;
                    }
                };

            // Decode the bonus type to get the associated player action.
            let bonus_kind = crate::element::BonusItemType::from_u16(raw.bonus_type);
            let associated_action = bonus_kind.to_action();

            // SetQuantity:
            //   * Ransom maps 1..=5 to 100/500/1000/2500/5000.
            //   * Blazon keeps the raw quantity and forces the animation row
            //     to BonusOne..BonusFive.
            //   * Everything else stores the raw quantity as-is.
            // The level stream holds the 1..=5 ordinal.
            let (stored_quantity, blazon_anim) = match bonus_kind {
                crate::element::BonusItemType::Ransom => {
                    let real = match raw.quantity {
                        1 => 100,
                        2 => 500,
                        3 => 1000,
                        4 => 2500,
                        5 => 5000,
                        q => {
                            tracing::error!(
                                "Ransom quantity {q} out of range [1,5]; using raw value",
                            );
                            q
                        }
                    };
                    (real, None)
                }
                crate::element::BonusItemType::Blazon => {
                    let anim = match raw.quantity {
                        1 => Some(crate::order::OrderType::BonusOne),
                        2 => Some(crate::order::OrderType::BonusTwo),
                        3 => Some(crate::order::OrderType::BonusThree),
                        4 => Some(crate::order::OrderType::BonusFour),
                        5 => Some(crate::order::OrderType::BonusFive),
                        q => {
                            tracing::error!(
                                "Blazon quantity {q} out of range [1,5]; leaving default animation",
                            );
                            None
                        }
                    };
                    (raw.quantity, anim)
                }
                _ => (raw.quantity, None),
            };

            let mut sprite = crate::sprite::Sprite::default();
            if let Err(e) = sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                crate::sprite_script::FrameKind::Object,
                char_base_dir,
                sprite_file,
                profile_name,
                bank_signature,
                Some(self.weather.ambiance.to_sprite_ambiance()),
            ) {
                tracing::error!(
                    "Failed to load bonus sprite '{sprite_file}' profile '{profile_name}': {e}",
                );
            } else {
                if let Some(anim) = blazon_anim {
                    // Blazons display the row for their current quantity:
                    // force_animation with the BonusOne..BonusFive row.
                    // Direction comes from the level data (applied below on
                    // the ElementData); the direction is set AFTER
                    // constructing the bonus, so we pass 0 here.
                    sprite.force_animation(anim, 0);
                }
                // Force a random sprite frame *after* the quantity-driven
                // animation row, so every bonus (including Blazons whose
                // animation row was just forced) ends up on a random frame
                // within its current row.  Sequencing must be
                // force_animation → force_random_sprite_frame because
                // `force_animation` resets `current_frame` to 0.
                sprite.force_random_sprite_frame(&mut self.rng);
            }
            sprite.apply_placement(
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
                raw.layer,
                crate::position_interface::SectorHandle::new(raw.sector),
                // Apply initial facing from level data (0-15 sector).
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::default(),
                crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );
            let entity = Entity::Bonus(crate::element::ElementBonus {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ObjectBonus,
                    // Bonuses are blipped on non-forest levels.
                    blipped: !self.weather.is_forest_level,
                    sprite,
                    ..Default::default()
                },
                object: crate::element::ObjectData {
                    quantity: stored_quantity,
                    object_type,
                    associated_action,
                    ..Default::default()
                },
            });
            self.add_entity(entity);
            // Each RANSOM bonus feeds its mapped value
            // (100/500/1000/2500/5000) into `bonus_money`. Without this,
            // the "Level money" debriefing row and the `money` console
            // cheat always see 0 for bonus money.
            if matches!(bonus_kind, crate::element::BonusItemType::Ransom) {
                self.mission_stat.bonus_money += stored_quantity as u32;
            }
        }

        progress(1.0);

        // Spawn scrolls (PARC chunk).
        //
        // Each scroll uses the "BONUS_Parchment" / "BONUS Parchemin"
        // sprite pair.
        //
        // `force_visible` on the raw scroll is only used once during init
        // (`if force_visible → SetStatus(Visible)`). The scroll status
        // lives on `GameHost::scroll_status`, and the host isn't
        // constructed until `load_mission_script` runs below. Collect
        // the handles here and flush them afterwards.
        let mut force_visible_scroll_ids: Vec<crate::element::EntityId> = Vec::new();
        // Reset the scroll-id → EntityId map (repopulated per-level).
        // Reserved capacity matches the PARC chunk count exactly.
        assets.scroll_entity_ids.clear();
        assets
            .scroll_entity_ids
            .reserve(loaded.mission.scrolls.len());
        for raw in &loaded.mission.scrolls {
            let mut sprite = crate::sprite::Sprite::default();
            if let Err(e) = sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                crate::sprite_script::FrameKind::Object,
                char_base_dir,
                "BONUS_Parchment",
                "BONUS Parchemin",
                bank_signature,
                Some(self.weather.ambiance.to_sprite_ambiance()),
            ) {
                tracing::error!("Failed to load scroll sprite: {e}");
            } else {
                let anim = if raw.tutorial {
                    crate::order::OrderType::BonusTwo
                } else {
                    crate::order::OrderType::BonusOne
                };
                sprite.force_animation(anim, raw.direction as u16);
                // Random sprite frame is picked later by
                // `EngineInner::initialize_all_scrolls` at mission start
                // (not at load).
            }

            // The mission stream's `action` field is ignored — the
            // initial animation is overridden based on the tutorial
            // flag, applied by `force_animation` above.
            let _ = raw.action;

            // SetActive(true) when `is_to_be_replaced_by_amulet`
            // (Easy + presence[Easy]==false, so the scroll spawns in
            // place to later morph into an amulet), else
            // SetActive(presence[difficulty]).  Skipping this left every
            // scroll active on Medium/Hard even when its presence flag
            // was cleared, which the render / focus paths would then
            // happily expose.
            let difficulty = crate::player_profile::DifficultyLevel::current();
            let difficulty_idx = difficulty as usize;
            let is_to_be_replaced_by_amulet =
                difficulty == crate::player_profile::DifficultyLevel::Easy && !raw.presence[0];
            let scroll_active = if is_to_be_replaced_by_amulet {
                true
            } else {
                raw.presence.get(difficulty_idx).copied().unwrap_or(false)
            };
            sprite.apply_placement(
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
                raw.layer,
                crate::position_interface::SectorHandle::new(raw.sector),
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::default(),
                crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::new(raw.obstacle_index),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );
            let entity = Entity::Scroll(crate::element::ElementScroll {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ObjectScroll,
                    sprite,
                    active: scroll_active,
                    // Stamp `CUSTOM_DOT_INVISIBLE` (= 0) at construction.
                    // Without this the scroll's default
                    // `custom_minimap_dot = 1` leaks into
                    // `minimap::classify_default` and pre-reveal scrolls
                    // would paint a minimap dot before the PC even talks
                    // to a beggar.
                    custom_minimap_dot: 0,
                    ..Default::default()
                },
                object: crate::element::ObjectData {
                    object_type: crate::element::ObjectType::Scroll,
                    ..Default::default()
                },
                presence: raw.presence,
                tutorial: raw.tutorial,
                script_class: raw.script_class.clone().unwrap_or_default(),
                script_hourglass_timeout: 0,
            });
            let scroll_eid = self.add_entity(entity);
            assets.scroll_entity_ids.push(scroll_eid);

            // `force_visible` flips status to Visible.  The scroll status
            // lives on `GameHost::scroll_status`, keyed by actor handle.
            // Capture ids here and flush them once the
            // mission script (and its host) is loaded below.
            if raw.force_visible {
                force_visible_scroll_ids.push(scroll_eid);
            }
        }

        // NOTE: GUYS (tenants) chunk does NOT add entities to the script
        // elements array.  Tenants just register existing entities as
        // building occupants; `InitOccupant` runs after
        // `populate_game_host_from_level` so it can operate on fully-
        // initialised entities.

        progress(1.0);

        // ── Spawn PCs at beam-me points ─────────────────────────────
        // We split this into two phases to satisfy the borrow checker:
        // Phase A computes assignments from campaign data (borrows self.campaign),
        // Phase B creates entities (borrows assets.sprite_scriptor, self.entities).
        // (char_idx, profile_idx, beam_me_idx, sherwood_returner)
        let pc_spawn_plan: Vec<(usize, crate::profiles::CharacterProfileIdx, usize, bool)>;
        let is_sherwood;
        {
            let campaign = self
                .campaign
                .as_mut()
                .expect("Campaign must be set before spawning PCs");

            // Determine Sherwood-camp flag up front — the Sherwood
            // branch needs to run between Phase 1 and Phase 2, and the
            // final post-spawn `ResetMissionTeam` call reads the same
            // flag.
            is_sherwood = campaign.current_mission_idx.is_some_and(|idx| {
                campaign
                    .missions
                    .get(idx)
                    .and_then(|m| m.profile_idx)
                    .and_then(|pi| profiles.missions.get(pi as usize))
                    .is_some_and(|p| p.location == crate::profiles::MissionLocation::Sherwood)
            });

            // Reset instanced flags for all gang members
            for &gi in &campaign.gang_indices {
                if let Some(desc) = campaign.characters.get_mut(gi) {
                    desc.instanced = false;
                }
            }

            // Build mission team list: (char_idx, profile_idx)
            let team: Vec<(usize, crate::profiles::CharacterProfileIdx)> = campaign
                .mission_team_indices
                .iter()
                .filter_map(|&char_idx| {
                    let profile_idx = campaign.characters.get(char_idx)?.character_profile_idx?;
                    Some((char_idx, profile_idx))
                })
                .collect();

            // Hackable overlays can add extra PCs without patching the
            // binary mission file.  Preserve the original beam-me slots
            // and synthesize nearby free slots up to the engine's normal
            // five-character mission cap so overlay-only characters can
            // actually appear in demo missions.
            const MAX_NUMBER_OF_CHARACTER: usize = 5;
            if !is_sherwood
                && team.len() > loaded.mission.beam_mes.len()
                && loaded.mission.beam_mes.len() < MAX_NUMBER_OF_CHARACTER
                && let Some(template) = loaded.mission.beam_mes.last().cloned()
            {
                let original_len = loaded.mission.beam_mes.len();
                let target_len = team.len().min(MAX_NUMBER_OF_CHARACTER);
                for i in original_len..target_len {
                    let mut beam_me = template.clone();
                    let offset = (i - original_len + 1) as f32;
                    beam_me.position.x += 28.0 * offset;
                    beam_me.index = i as u16;
                    beam_me.required_pc = 0;
                    beam_me.script = None;
                    loaded.mission.beam_mes.push(beam_me);
                }
                tracing::info!(
                    "Synthesized {} overlay beam-me slot(s) for {} mission team members",
                    loaded.mission.beam_mes.len() - original_len,
                    team.len()
                );
            }

            // Snapshot each team member's remembered Sherwood slot now so
            // that later mutations (Phase 1 or the Phase-B write-back of
            // `beam_me_index_in_sherwood`) don't race with Sherwood-branch
            // placement below.
            let team_remembered_sherwood_slot: Vec<i16> = team
                .iter()
                .map(|&(char_idx, _)| {
                    campaign
                        .characters
                        .get(char_idx)
                        .map(|d| d.status.beam_me_index_in_sherwood)
                        .unwrap_or(-1)
                })
                .collect();

            let mut assignments: Vec<Option<usize>> = vec![None; loaded.mission.beam_mes.len()];
            let mut instanced = vec![false; team.len()];
            // `sherwood_placed[ti]` is true when team-member `ti` was
            // seated by the Sherwood branch (not Phase 1 / Phase 2).
            // Flows through to the spawn plan so Phase B can call
            // `randomize_position` on those PCs.
            let mut sherwood_placed = vec![false; team.len()];

            // Phase 1: Handle required characters.
            for (bm_idx, beam_me) in loaded.mission.beam_mes.iter().enumerate() {
                if beam_me.required_pc == 0 {
                    continue;
                }
                let required_names: &[&str] = match beam_me.required_pc {
                    1 => &["Frere Tuck"],
                    2 => &["Lady Marianne"],
                    3 => &["Petit Jean"],
                    4 => &["Robin des bois", "Robin des villes"],
                    5 => &["Stutely"],
                    6 => &["Will Ecarlate"],
                    _ => {
                        tracing::error!(
                            "Unknown required_pc value {} at beam-me {}",
                            beam_me.required_pc,
                            bm_idx,
                        );
                        continue;
                    }
                };
                let found = team.iter().enumerate().find(|(ti, (_, pidx))| {
                    if instanced[*ti] {
                        return false;
                    }
                    // Case-insensitive name match against the character
                    // profile name.
                    profiles.get_character(*pidx).is_some_and(|p| {
                        required_names
                            .iter()
                            .any(|n| p.profile_name.eq_ignore_ascii_case(n))
                    })
                });
                if let Some((ti, _)) = found {
                    instanced[ti] = true;
                    assignments[bm_idx] = Some(ti);
                } else {
                    tracing::error!(
                        "Beam-me {} requires character type {} but no match in mission team",
                        bm_idx,
                        beam_me.required_pc,
                    );
                }
            }

            // Sherwood branch: when the current mission is the Sherwood
            // camp, seat every mission-team member whose remembered
            // `beam_me_index_in_sherwood` recalls a slot from the
            // previous Sherwood stay back at that slot, then permute the
            // unused beam-mes 100 times so the remaining free-for-all
            // slots Phase 2 assigns to are randomised.  The per-PC
            // position jitter + random facing (`randomize_position`)
            // happens in Phase B after each sherwood-returner is
            // actually spawned.
            if is_sherwood {
                for ti in 0..team.len() {
                    if instanced[ti] {
                        // Phase 1 already seated this member by required
                        // name; skip to avoid double-spawning.
                        continue;
                    }
                    let remembered = team_remembered_sherwood_slot[ti];
                    if remembered < 0 {
                        continue;
                    }
                    let bm_idx = remembered as usize;
                    if bm_idx >= loaded.mission.beam_mes.len() {
                        tracing::warn!(
                            "Sherwood-return PC (team_idx {ti}) remembered beam_me {remembered} \
                             but only {} slots exist — ignoring",
                            loaded.mission.beam_mes.len(),
                        );
                        continue;
                    }
                    if assignments[bm_idx].is_some() {
                        tracing::warn!(
                            "Sherwood-return PC (team_idx {ti}) wanted beam_me {bm_idx} which is \
                             already assigned — skipping",
                        );
                        continue;
                    }
                    assignments[bm_idx] = Some(ti);
                    instanced[ti] = true;
                    sherwood_placed[ti] = true;
                }

                // Shuffle the free beam-mes 100 times.  Both the
                // `BeamMe` vector and the parallel `assignments` vector
                // are swapped together so that the "used" flag attached
                // to an occupied slot travels with its beam-me (a PC
                // Sherwood-placed above was baked at the slot's
                // original beam-me; we preserve that identity rather
                // than the slot index).  We pull from the deterministic
                // sim RNG so replay / rollback stay reproducible.
                let n = loaded.mission.beam_mes.len();
                if n > 0 {
                    for _ in 0..100 {
                        let a = crate::sim_rng::usize(0..n);
                        let b = crate::sim_rng::usize(0..n);
                        if a != b {
                            loaded.mission.beam_mes.swap(a, b);
                            assignments.swap(a, b);
                        }
                    }
                }
            }

            // Phase 2: Fill remaining beam-me slots, honoring the
            // per-beam-me action requirements.
            //
            //   1. Build the list of "available" team members (those not
            //      already instanced by Phase 1).  For non-Sherwood
            //      missions the list is capped at MAX_NUMBER_OF_CHARACTER
            //      (= 5; extra mission-team members stay behind as
            //      reservists for this mission).
            //   2. Iterate until every slot is filled or no candidates
            //      remain.  In each pass: for each unsolved beam-me,
            //      collect the team members valid for the slot (action-
            //      capability intersection with the beam-me's
            //      `action_required` flags).  Assign when there is
            //      exactly one candidate, or, once `force_decision`
            //      kicks in, when there is at least one.
            //   3. Dead end (no slot solved this iteration): first flip
            //      `force_decision` to let multi-candidate slots
            //      resolve.  If that still fails, fall through to a
            //      brute force pass that fills each remaining slot with
            //      whichever character is left even if the action
            //      requirements don't match — better than leaving the
            //      slot empty.
            let mut available: Vec<usize> = (0..team.len()).filter(|&i| !instanced[i]).collect();
            if !is_sherwood && available.len() > MAX_NUMBER_OF_CHARACTER {
                available.truncate(MAX_NUMBER_OF_CHARACTER);
            }

            let slot_valid_for = |beam_me: &crate::level_data::BeamMe,
                                  profile_idx: crate::profiles::CharacterProfileIdx|
             -> bool {
                let Some(profile) = profiles.get_character(profile_idx) else {
                    return false;
                };
                use crate::profiles::Action;
                let mut archer = false;
                let mut lever_main = false;
                let mut lockpicker_main = false;
                let mut stuner = false;
                let mut eater = false;
                for a in &profile.actions {
                    match *a {
                        Action::Bow => archer = true,
                        Action::Lever => lever_main = true,
                        Action::Lockpick => lockpicker_main = true,
                        Action::Hit | Action::HitHard => stuner = true,
                        Action::Eat | Action::Guzzle => eater = true,
                        _ => {}
                    }
                }
                let mut carrier = false;
                let mut climber = false;
                let mut jumper = false;
                let mut tailor = false;
                let mut searcher = false;
                let mut lever_ctx = false;
                let mut lockpicker_ctx = false;
                for a in &profile.contextual_actions {
                    match *a {
                        Action::FarmerCarry | Action::LittleJohnCarry => carrier = true,
                        Action::Climb => climber = true,
                        Action::Jump => jumper = true,
                        Action::Tie => tailor = true,
                        Action::Search => searcher = true,
                        Action::Lockpick => lockpicker_ctx = true,
                        Action::Lever => lever_ctx = true,
                        _ => {}
                    }
                }
                let req = &beam_me.action_required;
                if req.archery && !archer {
                    return false;
                }
                if req.carry && !carrier {
                    return false;
                }
                if req.climb && !climber {
                    return false;
                }
                if req.jump && !jumper {
                    return false;
                }
                if req.lever && !(lever_main || lever_ctx) {
                    return false;
                }
                if req.lockpick && !(lockpicker_main || lockpicker_ctx) {
                    return false;
                }
                if req.stun && !stuner {
                    return false;
                }
                if req.tie && !tailor {
                    return false;
                }
                if req.eat && !eater {
                    return false;
                }
                if req.search && !searcher {
                    return false;
                }
                true
            };

            let mut force_decision = false;
            while assignments.iter().any(|a| a.is_none()) && !available.is_empty() {
                let mut solved_this_pass = false;
                for (bm_idx, beam_me) in loaded.mission.beam_mes.iter().enumerate() {
                    if assignments[bm_idx].is_some() {
                        continue;
                    }
                    let candidates: Vec<usize> = available
                        .iter()
                        .copied()
                        .filter(|&ti| slot_valid_for(beam_me, team[ti].1))
                        .collect();
                    let pick =
                        if candidates.len() == 1 || (force_decision && !candidates.is_empty()) {
                            Some(candidates[0])
                        } else {
                            None
                        };
                    if let Some(ti) = pick {
                        assignments[bm_idx] = Some(ti);
                        instanced[ti] = true;
                        available.retain(|&x| x != ti);
                        solved_this_pass = true;
                        // Reset force_decision on success so the next
                        // pass re-prefers single-candidate slots.
                        force_decision = false;
                        if available.is_empty() {
                            break;
                        }
                    }
                }
                if !solved_this_pass {
                    if !force_decision {
                        force_decision = true;
                        continue;
                    }
                    // Brute-force fill for slots with no valid candidate.
                    for (bm_idx, _) in loaded.mission.beam_mes.iter().enumerate() {
                        if assignments[bm_idx].is_some() || available.is_empty() {
                            continue;
                        }
                        let ti = available.remove(0);
                        assignments[bm_idx] = Some(ti);
                        instanced[ti] = true;
                    }
                    break;
                }
            }

            // Collect the spawn plan and mark instanced in campaign.
            pc_spawn_plan = loaded
                .mission
                .beam_mes
                .iter()
                .enumerate()
                .filter_map(|(bm_idx, _)| {
                    let ti = assignments[bm_idx]?;
                    let (char_idx, profile_idx) = team[ti];
                    // Mark character as instanced in the campaign
                    if let Some(desc) = campaign.characters.get_mut(char_idx) {
                        desc.instanced = true;
                    }
                    Some((char_idx, profile_idx, bm_idx, sherwood_placed[ti]))
                })
                .collect();
        }

        // Phase B: Create entities (no longer borrowing self.campaign).
        // Add one script entry per beam-me: the PC if assigned, or None
        // to keep script entity indices aligned.
        let mut pc_count = 0u32;
        for (bm_idx, beam_me) in loaded.mission.beam_mes.iter().enumerate() {
            // Find the spawn plan entry for this beam-me, if any
            let plan_entry = pc_spawn_plan.iter().find(|&&(_, _, bi, _)| bi == bm_idx);

            if let Some(&(char_idx, mut profile_idx, _, sherwood_returner)) = plan_entry {
                // A "Robin des villes" PC in a forest level is rewritten
                // to "Robin des bois", and vice-versa in a town level.
                // Swap both `profile_idx` and the campaign character's
                // stored `character_profile_idx`.
                {
                    use crate::character_kind::CharacterKind;
                    let want_town = !self.weather.is_forest_level;
                    let current_kind = profiles
                        .get_character(profile_idx)
                        .and_then(|p| CharacterKind::from_profile(&p.filename, &p.profile_name));
                    if let Some(CharacterKind::RobinHood { is_town }) = current_kind
                        && is_town != want_town
                    {
                        let target_kind = CharacterKind::RobinHood { is_town: want_town };
                        if let Some(new_idx) = profiles.characters.iter().position(|p| {
                            CharacterKind::from_profile(&p.filename, &p.profile_name)
                                == Some(target_kind)
                        }) {
                            let new_profile_idx =
                                crate::profiles::CharacterProfileIdx(new_idx as u32);
                            profile_idx = new_profile_idx;
                            if let Some(campaign) = self.campaign.as_mut()
                                && let Some(desc) = campaign.characters.get_mut(char_idx)
                            {
                                desc.character_profile_idx = Some(new_profile_idx);
                            }
                        } else {
                            tracing::warn!(
                                "Robin forest/town swap: profile '{:?}' not found; keeping {:?}",
                                target_kind,
                                profile_idx,
                            );
                        }
                    }
                }
                let profile = profiles
                    .get_character(profile_idx)
                    .expect("Character profile must exist for mission team member");

                // PCs always use the Character frame kind, unlike NPCs
                // which use the level's frame_kind (Character vs CharacterBlipped).
                let mut sprite = crate::sprite::Sprite::default();
                if let Err(e) = sprite.load_frame_info(
                    assets.sprite_scriptor_mut(),
                    crate::sprite_script::FrameKind::Character,
                    char_base_dir,
                    &profile.filename,
                    &profile.profile_name,
                    bank_signature,
                    Some(self.weather.ambiance.to_sprite_ambiance()),
                ) {
                    tracing::error!(
                        "Failed to load sprite for PC '{}' (profile {}): {e}",
                        profile.profile_name,
                        profile_idx,
                    );
                }
                // Load the alternate profile track when the character
                // profile flags `valid_alternative_profile`.  Used for
                // disguise / variant animations.
                if profile.valid_alternative_profile
                    && !profile.alternative_profile_name.is_empty()
                    && let Err(e) = sprite.load_alternate_profile(
                        assets.sprite_scriptor_mut(),
                        crate::sprite_script::FrameKind::Character,
                        char_base_dir,
                        &profile.filename,
                        &profile.alternative_profile_name,
                        bank_signature,
                        None,
                    )
                {
                    tracing::error!(
                        "Failed to load alternate sprite profile '{}' for PC '{}' (profile {}): {e}",
                        profile.alternative_profile_name,
                        profile.profile_name,
                        profile_idx,
                    );
                }

                let kind = crate::character_kind::CharacterKind::from_profile(
                    &profile.filename,
                    &profile.profile_name,
                );
                let is_robin = kind.is_some_and(|k| k.is_robin());
                let (has_lockpick, has_climb, has_jump) =
                    crate::element::PcData::movement_auth_from_profile(profile);

                // Map the beam-me's initial animation to a (posture,
                // action_state) pair. Apply it up front so the PC starts
                // in the correct pose. The HIDDEN titbit for Spy/Tree
                // postures is added by the regular titbit sync pass once
                // it sees a hidden posture, so we don't add it manually
                // here.
                let (initial_posture, initial_action_state) = map_pc_initial_action(beam_me.action);
                let list_index = u8::try_from(char_idx)
                    .expect("campaign character index must fit in PcData::list_index");

                // Set the sprite's move box + pathfinder index from the
                // character profile right after `LoadFrameInfo` so
                // anti-collision has a valid bbox on the very first tick.
                let pc_pathfinder_idx = profile.pathfinder_index;
                let pc_half_diag = self
                    .fast_grid
                    .try_move_box_half_diagonal(pc_pathfinder_idx as usize);
                sprite.position_iface.configure_for_actor(
                    pc_pathfinder_idx,
                    pc_half_diag,
                    beam_me.position,
                );
                // Validate every beam-me's layer range and sector
                // motion/area bits.  Warn instead of asserting so a
                // corrupt mission file still loads (the existing
                // motion/area lookup will collapse the beam-me into the
                // fallback path downstream).
                if beam_me.layer > self.fast_grid.level.special_layer {
                    tracing::warn!(
                        "Beam-me {} at ({},{}) lies on out-of-range layer {} (special_layer={})",
                        bm_idx,
                        beam_me.position.x,
                        beam_me.position.y,
                        beam_me.layer,
                        self.fast_grid.level.special_layer,
                    );
                }
                let sector_motion_area = self
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&crate::sector::SectorNumber::new(beam_me.sector as i16))
                    .and_then(|&idx| self.fast_grid.level.sectors.get(idx))
                    .map(|gs| gs.sector_type.is_motion() && gs.sector_type.is_area())
                    .unwrap_or(false);
                if !sector_motion_area {
                    tracing::warn!(
                        "Beam-me {} at ({},{}) sector {} is not a motion+area sector",
                        bm_idx,
                        beam_me.position.x,
                        beam_me.position.y,
                        beam_me.sector,
                    );
                }
                // Out-of-range material silently falls back to the grid
                // default material.
                let material = crate::element::GameMaterial::from_u32_with_default(
                    beam_me.material,
                    assets.material_sectors.default_material,
                );
                // Validate that the beam-me's obstacle index is a
                // projection area and the beam-me position is inside its
                // screen box.  We warn so a corrupt mission still loads.
                if beam_me.projection_area != 0xFFFF {
                    match assets
                        .static_sight_obstacles
                        .get(beam_me.projection_area as usize)
                    {
                        None => tracing::warn!(
                            "Beam-me {} references out-of-range projection area {}",
                            bm_idx,
                            beam_me.projection_area,
                        ),
                        Some(obs) => {
                            if !obs.is_projection_area() {
                                tracing::warn!(
                                    "Beam-me {} at ({},{}) not lying on projection area (obstacle {})",
                                    bm_idx,
                                    beam_me.position.x,
                                    beam_me.position.y,
                                    beam_me.projection_area,
                                );
                            }
                            if !obs.box_projection.contains_point(beam_me.position) {
                                tracing::warn!(
                                    "Beam-me {} at ({},{}) map position not lying in projection area screen box (obstacle {})",
                                    bm_idx,
                                    beam_me.position.x,
                                    beam_me.position.y,
                                    beam_me.projection_area,
                                );
                            }
                        }
                    }
                }
                sprite.apply_placement(
                    beam_me.position,
                    beam_me.layer,
                    crate::position_interface::SectorHandle::new(beam_me.sector),
                    // Apply initial facing from the beam-me point (0-15 sector).
                    (beam_me.direction & 15) as i16,
                    material,
                    crate::position_interface::ObstacleHandle::new(beam_me.projection_area),
                    crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                        crate::position_interface::ObstacleHandle::new(beam_me.projection_area),
                        assets.static_sight_obstacles.as_slice(),
                    ),
                );
                // Seed old position fields so `is_moving()` is false on
                // the first post-spawn tick (matches the Target spawn
                // path above).
                let current_position = sprite.position_iface.get_position();
                sprite.position_iface.set_old_position(current_position);
                sprite.position_iface.set_old_map_position(beam_me.position);
                // Seed `disabled_actions` from per-slot ammo /
                // purse-ransom checks so a slot whose counter is empty
                // (or whose purse threshold isn't met) starts greyed out
                // instead of waiting for the first runtime ammo update.
                let disabled_actions: Vec<bool> = {
                    let pc_status_opt = self
                        .campaign
                        .as_ref()
                        .and_then(|c| c.characters.get(char_idx))
                        .map(|d| &d.status);
                    let ransom = self
                        .campaign
                        .as_ref()
                        .map(|c| c.get_value(crate::campaign::CampaignValue::Ransom))
                        .unwrap_or(0);
                    let purse_threshold = crate::inventory::COINS_PER_PURSE as i32
                        * crate::inventory::COIN_VALUE as i32;
                    (0..crate::profiles::NUMBER_OF_PC_ACTIONS)
                        .map(|slot| {
                            let action = profile.actions[slot];
                            if action == crate::profiles::Action::NoAction {
                                return false;
                            }
                            let ammo_empty = crate::inventory::action_uses_ammo(action)
                                && pc_status_opt.map(|s| s.get_ammo(action)).unwrap_or(0) == 0;
                            let purse_underfunded = action == crate::profiles::Action::Purse
                                && ransom < purse_threshold;
                            ammo_empty || purse_underfunded
                        })
                        .collect()
                };
                let entity = Entity::Pc(crate::element::ActorPc {
                    element: crate::element::ElementData {
                        kind: crate::element::ElementKind::ActorPc,
                        sprite,
                        // Initial posture from `InitializeAction`.
                        posture: initial_posture,
                        ..Default::default()
                    },
                    actor: crate::element::ActorData {
                        // The per-actor Initialize() dispatch in
                        // `EngineInner::initialize_mission_script` picks
                        // up this string and creates a persistent
                        // ScriptInstance via `MissionScript::bind_actor`.
                        script_class: beam_me.script.clone().unwrap_or_default(),
                        // Action state from `InitializeAction`.
                        action_state: initial_action_state,
                        ..Default::default()
                    },
                    human: crate::element::HumanData {
                        time_hulk: crate::element::HULK_LENGTH,
                        ..Default::default()
                    },
                    pc: crate::element::PcData {
                        robin: is_robin,
                        profile_index: profile_idx,
                        list_index,
                        kind,
                        has_lockpick,
                        has_climb,
                        has_jump,
                        beam_me_index: beam_me.index as i16,
                        disabled_actions,
                        disabled_actions_temp: vec![false; crate::profiles::NUMBER_OF_PC_ACTIONS],
                        // Kept for save/restore parity.
                        initial_action: beam_me.action,
                        ..Default::default()
                    },
                });
                let spawned_eid = self.add_entity(entity);
                pc_count += 1;

                // When the mission is the Sherwood camp, remember the
                // beam-me slot this PC landed on so that a later
                // Sherwood visit can restore the same position;
                // otherwise clear the slot ("goes out of Sherwood =>
                // looses his place").  The write happens here in
                // Phase B because this is the only point we have both
                // the post-shuffle beam-me and the char_idx in scope
                // with a live mut-borrow path to `campaign.characters`.
                if let Some(campaign) = self.campaign.as_mut()
                    && let Some(desc) = campaign.characters.get_mut(char_idx)
                {
                    desc.status.beam_me_index_in_sherwood = if is_sherwood {
                        beam_me.index as i16
                    } else {
                        -1
                    };
                }

                // Sherwood returners get their position + facing
                // jittered by `randomize_position`.  Non-returners keep
                // the beam-me's exact position/facing.
                if sherwood_returner {
                    self.randomize_position(spawned_eid);
                }
            } else {
                // No PC for this beam-me — push None to keep script indices aligned.
                self.entities.push(None);
            }
        }
        tracing::info!("Spawned {} PCs at beam-me positions", pc_count);

        // Sherwood camp built — clear the mission team so that UI paths
        // that read `mission_team_indices` while the player is back in
        // Sherwood don't see the team from whichever mission we just
        // finished.
        if is_sherwood && let Some(campaign) = self.campaign.as_mut() {
            campaign.reset_mission_team();
        }

        // Every spawned actor needs a low-priority wait element so the
        // animation driver's `current_order_for_actor` lookup always
        // returns a posture-appropriate idle order.  This is invoked
        // whenever an actor has no current order — in practice at the
        // first Execute tick after spawn.
        let actor_ids_snapshot: Vec<_> = self.entities.actors().map(|(id, _)| id.into()).collect();
        for actor_id in actor_ids_snapshot {
            self.ensure_wait_element(actor_id);
        }

        // Remaining load-time subsystems (motion grid, sight obstacles,
        // background map, patrol paths, tactical info, mobile elements,
        // tenants) are loaded from other code paths.

        // Set night color based on ambiance — pack via draw_manager.
        let (r, g, b) = self.weather.ambiance.night_color_rgb();
        let _ = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
        // EngineInner format is always RGB565. Host can derive 15-bit packing
        // at render time if its display needs it.
        self.weather.night_color = robin_util::color::rgb565(r, g, b);

        tracing::info!(
            "EngineInner: initialized from mission '{}' / proto '{}' — \
             {} soldiers, {} civilians, {} targets, {} bonuses, {} beam-mes",
            mission_name,
            proto_level_name,
            loaded.mission.soldiers.len(),
            loaded.mission.civilians.len(),
            loaded.mission.targets.len(),
            loaded.mission.bonuses.len(),
            loaded.mission.beam_mes.len(),
        );

        // Load the mission script (.scb bytecode).
        let scb_path = format!("{}/{}.scb", level_directory, mission_name);
        self.load_mission_script(assets, std::path::Path::new(&scb_path));

        // Flush `force_visible` scroll visibility, calling SetStatus
        // (Visible) for each captured scroll.  Route through
        // `set_scroll_status` so the scroll's `custom_minimap_dot` is
        // refreshed alongside the status.
        if !force_visible_scroll_ids.is_empty() {
            let count = force_visible_scroll_ids.len();
            for eid in force_visible_scroll_ids {
                self.set_scroll_status(eid, ScrollStatus::Visible);
            }
            tracing::info!("Applied force_visible to {count} scroll(s)");
        }

        // Populate the script host with level data (doors, patches, entity
        // state, PC auth bits) so that native functions can operate on real
        // game objects.
        self.populate_game_host_from_level(assets, pending, &loaded);

        // Install mission-defined reinforcement doors: construct one
        // `Door(Reinforcement)` per REIN entry and insert it into the
        // gate-graph table.  The Rust port keeps a single
        // `game_host.doors` list plus a filtered
        // `ai_global.reinforcement_doors` cache built below.  This has
        // to run after `populate_game_host_from_level` so that
        // `game_host.doors` exists, and before the cache filter so
        // `ai_global.reinforcement_doors` picks up these entries
        // alongside any proto-level doors with
        // `door_type == Reinforcement`.
        if let Some(ref tactic) = loaded.mission.tactic_data
            && !tactic.reinforcement_points.is_empty()
        {
            let map_bbox = self.fast_grid.level.map_bbox;
            let special_layer = self.fast_grid.level.special_layer;
            // The out-of-map sector is sentinel #-1.
            let sector_out_of_map = crate::sector::SectorNumber::new(-1);
            let mut installed = 0usize;
            if let Some(script) = self.mission_script.as_mut()
                && let Some(game_host) = script.game_host_mut()
            {
                for raw in &tactic.reinforcement_points {
                    // The referenced sector must be motion+area.  We use
                    // a soft error so the load path doesn't bail on a
                    // corrupt mission file, while still flagging the
                    // issue loudly.
                    let sector_ok = self
                        .fast_grid
                        .level
                        .sector_number_map
                        .get(&crate::sector::SectorNumber::new(raw.sector as i16))
                        .and_then(|&idx| self.fast_grid.level.sectors.get(idx))
                        .map(|gs| gs.sector_type.is_motion() && gs.sector_type.is_area())
                        .unwrap_or(false);
                    if !sector_ok {
                        tracing::error!(
                            "Reinforcement point ({}, {}) references non-motion-area sector {} \
                             — skipping",
                            raw.x,
                            raw.y,
                            raw.sector,
                        );
                        continue;
                    }

                    let inside = MapPoint::new(raw.x as f32, raw.y as f32);
                    let (border, outside) = crate::natives::compute_border_point_bbox(
                        map_bbox,
                        (inside.x, inside.y),
                        raw.direction as i16,
                    );
                    let border = MapPoint::new(border.0, border.1);
                    let outside = MapPoint::new(outside.0, outside.1);

                    // Reinforcement doors get 4× WalkingUpright actions
                    // by default.
                    let (act_d1, act_d2, act_i1, act_i2) =
                        crate::gate::Door::default_actions_for_type(
                            crate::gate::DoorType::Reinforcement,
                        );

                    game_host.doors.push(crate::gate::Door {
                        gate_type: crate::gate::GateType::Door,
                        door_type: crate::gate::DoorType::Reinforcement,
                        point_in: inside,
                        point_mid: border,
                        point_out: outside,
                        layer_in: raw.layer,
                        layer_out: special_layer,
                        sector_in: crate::sector::SectorNumber::new(raw.sector as i16),
                        sector_out: sector_out_of_map,
                        action_direct_1: act_d1,
                        action_direct_2: act_d2,
                        action_indirect_1: act_i1,
                        action_indirect_2: act_i2,
                        ..Default::default()
                    });
                    // AdaptPoints is a no-op for `Reinforcement` doors
                    // (only BuildingTrap / LiftHigh[Crenel] on wall lifts
                    // shift `point_in`), but penalty still has to be
                    // computed so A* gate-graph routing through these
                    // out-of-map doors has a finite cost.
                    if let Some(door) = game_host.doors.last_mut() {
                        door.compute_door_penalty();
                    }
                    installed += 1;
                }
                // Rebuild gate-link connectivity so the new reinforcement
                // doors are routed through by `find_path_gates`.
                if installed > 0 {
                    crate::gate::build_gate_links(&mut game_host.doors);
                }
            }
            if installed > 0 {
                tracing::debug!(
                    "Installed {installed} mission-REIN reinforcement doors into game_host.doors",
                );
            }
        }

        // Drain proto-stream jump-gate Door specs into `game_host.doors`.
        // The `consume_pending_motion_data` pass that produced these specs
        // ran before `populate_game_host_from_level` (so beam-me sector
        // checks see a populated grid), so the Door push has to happen
        // here once `game_host` exists.  Rebuilds gate-link connectivity
        // last so the new jump gates (and any prior REIN doors above) are
        // routed through by `find_path_gates`.
        self.register_pending_jump_gates(pending);

        // ── InitOccupant ──
        // Each tenant is (1) validated as a human (warn otherwise),
        // (2) moved to the grid's lift-layer and the building's sector,
        // (3) repositioned onto the building's first door's `point_in`,
        // and (4) set inactive so it starts "inside" the building.
        //
        // `mission.building_tenants[i]` corresponds to the `i`-th
        // `RawBuildingEntry::Building` (StandaloneDoors entries are not
        // buildings).
        // Pre-pass: collect the first door's point_in + the sector handle
        // per building index, then apply to each tenant.  Sector numbers
        // were allocated by `rewire_building_doors` into
        // constructor-local pending data.
        //
        // `lift_layer()` is `special_layer - 1`; skip the whole pass when
        // the grid hasn't been sized yet (empty fixtures, tests).
        let lift_layer = if self.fast_grid.level.special_layer > 0 {
            self.fast_grid.lift_layer()
        } else {
            0
        };
        let building_first_door_info: Vec<(MapPoint, u16)> = loaded
            .proto
            .buildings
            .iter()
            .filter_map(|entry| match entry {
                crate::level_data::RawBuildingEntry::Building { doors } => doors.first(),
                _ => None,
            })
            .map(|door| {
                (
                    MapPoint::new(door.point_in.0 as f32, door.point_in.1 as f32),
                    door.sector_in,
                )
            })
            .collect();
        if loaded.mission.building_tenants.len() != building_first_door_info.len() {
            tracing::warn!(
                "Building tenant count {} does not match building count {} — \
                 mission file and proto-level may be mismatched",
                loaded.mission.building_tenants.len(),
                building_first_door_info.len(),
            );
        }
        for (bld_idx, tenants) in loaded.mission.building_tenants.iter().enumerate() {
            let first_door = building_first_door_info.get(bld_idx).copied();
            for &elem_idx in &tenants.tenant_element_indices {
                let Some(entity_id) = self.entities.id_at_index(u32::from(elem_idx)) else {
                    continue;
                };
                let Some(entity) = self.entities.get_mut(entity_id) else {
                    continue;
                };
                // Only humans participate in InitOccupant; warn and skip
                // otherwise.  Keep the entity untouched in the non-human
                // case so we don't corrupt whatever sits at that slot.
                if !entity.is_human() {
                    tracing::warn!(
                        "Building {} tenant element #{} is not a human — \
                         skipping InitOccupant",
                        bld_idx,
                        elem_idx,
                    );
                    continue;
                }
                let elem = entity.element_data_mut();
                elem.active = false;
                if let Some((point_in, sector_in)) = first_door {
                    // Change layer to lift_layer + sector to the
                    // building's first sector, then snap the position to
                    // PointIn.  Write through the sprite's
                    // `PositionInterface` so the move-box and pathfinder
                    // caches see the teleport.
                    let pi = &mut elem.sprite.position_iface;
                    pi.set_map_position(point_in);
                    if let Some(layer) = crate::position_interface::Layer::new(lift_layer) {
                        pi.set_layer(layer);
                    }
                    pi.set_sector(crate::position_interface::SectorHandle::new(sector_in));
                }
            }
        }

        // Cache door geometry for `FindDoorEnemyCouldBeBehind`, which
        // walks the door list owned by the building/sector graph.
        let door_infos: Vec<crate::ai::DoorSeekInfo> = self
            .mission_script
            .as_mut()
            .and_then(|s| s.game_host_mut())
            .map(|game_host| {
                game_host
                    .doors
                    .iter()
                    .enumerate()
                    .map(|(idx, door)| {
                        // Approximation of "is the soldier authorized to
                        // enter through this door?": we check only the
                        // static villain lock + active flag; the building
                        // capacity and rider checks are dynamic and not
                        // modelled here. `find_door_enemy_could_be_behind`
                        // is only called from soldier seek paths and
                        // building doors are the only relevant door type,
                        // so the approximation is fine.
                        let npc_villain_authorized_direct =
                            matches!(door.door_type, crate::gate::DoorType::Building)
                                && door.active
                                && !door.locked_npc_villain;
                        crate::ai::DoorSeekInfo {
                            door_index: crate::gate::DoorIndex(idx as u32),
                            door_type: door.door_type,
                            point_out: door.point_out,
                            position_in: crate::ai::Position {
                                x: door.point_in.x,
                                y: door.point_in.y,
                                sector: crate::position_interface::SectorHandle::new(u16::from(
                                    door.sector_in,
                                )),
                                level: door.layer_in,
                            },
                            sector_out: u16::from(door.sector_out),
                            sector_in: u16::from(door.sector_in),
                            layer_out: door.layer_out,
                            npc_villain_authorized_direct,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.ai_global.door_seek_infos = door_infos;
        tracing::debug!(
            "Cached {} door seek infos for FindDoorEnemyCouldBeBehind",
            self.ai_global.door_seek_infos.len(),
        );

        // Populate reinforcement door info for MerryManForestCassos.
        self.ai_global.reinforcement_doors = self
            .mission_script
            .as_mut()
            .and_then(|s| s.game_host_mut())
            .map(|game_host| {
                game_host
                    .doors
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.door_type == crate::gate::DoorType::Reinforcement)
                    .map(|(idx, d)| crate::ai::ReinforcementDoorInfo {
                        position_in: crate::ai::Position {
                            x: d.point_in.x,
                            y: d.point_in.y,
                            sector: crate::position_interface::SectorHandle::new(u16::from(
                                d.sector_in,
                            )),
                            level: d.layer_in,
                        },
                        door_index: crate::gate::DoorIndex(idx as u32),
                        point_out: d.point_out,
                        point_in: d.point_in,
                        point_mid: d.point_mid,
                        layer_out: d.layer_out,
                        sector_out: crate::position_interface::SectorHandle::new(u16::from(
                            d.sector_out,
                        )),
                    })
                    .collect()
            })
            .unwrap_or_default();
        tracing::debug!(
            "Cached {} reinforcement doors for MerryManForestCassos",
            self.ai_global.reinforcement_doors.len(),
        );

        // Sort portrait order by character priority (descending — highest
        // priority = leftmost slot). Done here so the portrait bar, the
        // keyboard 1-5 shortcuts, and `select_by_portrait_index` all see
        // the priority-sorted order.
        self.sort_pc_ids_by_priority(assets);

        // Auto-select the highest-priority playable PC (Robin) into
        // canonical seat 0. Local host viewport centering happens outside
        // the engine after level load.
        self.select_highest_priority_pc(assets, 0);

        Ok(())
    }

    // ─── Ambiance / sprite variants ────────────────────────────────

    /// Return the sprite variant matching the current ambiance.
    pub fn default_variant(&self) -> crate::sprite_variant::SpriteVariant {
        use crate::sprite_variant::SpriteVariant;
        // Force Day regardless of ambiance when the fog-sprites-crash
        // workaround is enabled.
        if let Some(opts) = crate::engine::GlobalOptions::global().as_ref()
            && opts.bypass_fog_sprites_crash
        {
            return SpriteVariant::Day;
        }
        match self.weather.ambiance {
            Ambiance::Fog => SpriteVariant::Fog,
            Ambiance::Night => SpriteVariant::Night,
            _ => SpriteVariant::Day,
        }
    }

    /// Resolve the sprite variant to use when rendering `entity` this frame.
    ///
    /// PCs/NPCs always pick up the default variant; objects/projectiles only
    /// do so when their object type has an ambiance variant.  Animals, FX,
    /// targets, bonuses, scrolls, and mobiles always render as Day.
    pub fn resolve_render_variant(
        &self,
        entity: &crate::element::Entity,
    ) -> crate::sprite_variant::SpriteVariant {
        use crate::element::Entity;
        use crate::sprite_variant::SpriteVariant;

        let apply_ambiance = match entity {
            // PCs/NPCs always pick up the default variant.
            Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_) => true,
            // Objects/projectiles only pick up the variant when their type
            // has an ambiance variant.
            Entity::Projectile(p) => p.object.object_type.has_ambiance_variant(),
            Entity::Net(n) => n.object.object_type.has_ambiance_variant(),
            Entity::Bonus(b) => b.object.object_type.has_ambiance_variant(),
            // Scroll, Animal, Fx, FxMasked, Target, Mobile: variant stays
            // at its default (Day).
            _ => false,
        };
        if !apply_ambiance {
            return SpriteVariant::Day;
        }

        let default = self.default_variant();
        if !matches!(default, SpriteVariant::Fog | SpriteVariant::Night) {
            return default;
        }

        // Shadow-sector fallback: revert Fog/Night → Day when the entity
        // stands in a `SECTOR_SHADOW` (light-at-night) polygon.
        let elem = entity.element_data();
        if elem.layer() != 0xFFFF
            && self
                .fast_grid
                .is_in_shadow_sector(elem.position_map(), elem.layer())
        {
            SpriteVariant::Day
        } else {
            default
        }
    }

    // NOTE: `initialize_sprite_variants` was moved to robin_rs
    // (`level_loading_host::initialize_sprite_variants`) as part of
    // the engine carve-out (Decision 1): it only manipulates the host-
    // side `FrameHolder` (variant dictionaries, global shadow values)
    // which the engine must no longer reference. After level load: Day
    // drops night+fog dictionaries (shadow=40, blip=60); Fog drops
    // night and generates fog dictionaries (shadow=10, blip=40); Night
    // drops fog and generates night dictionaries (shadow=40, blip=60).

    /// Pre-pass that rewrites every `Building`-entry door's `sector_in`
    /// / `layer_in` to point at the empty `BUILDING` grid sector the
    /// motion-init pass will allocate for that building, and records
    /// the allocated sector numbers on constructor-local pending data.
    ///
    /// Two-step building load: create one empty-polygon sector per
    /// `Building` entry on the lift layer (type
    /// `MOTION | AREA | BUILDING`), then route every door inside that
    /// building's `sector_in` to point at the building's sector — the
    /// door's stream-read sector is discarded in favour of the
    /// building's.
    ///
    /// We can't defer the rewrite until `initialize_motion_from_level_data`
    /// runs, because the load pipeline calls `populate_game_host_from_level`
    /// first (to build `game_host.doors` + `ai_global.door_seek_infos`).  So this
    /// pre-pass runs during the initial load, right after the level file is
    /// parsed, computes the same sector number each building would get in
    /// the motion-init pass, and patches the raw doors in place.  The motion
    /// pass later creates the matching grid sectors using the stashed numbers
    /// (with a `debug_assert_eq!` that catches any drift between the two
    /// allocators).
    fn rewire_building_doors(
        &mut self,
        pending: &mut PendingLevelData,
        buildings: &mut [crate::level_data::RawBuildingEntry],
        motion_data: Option<&crate::level_data::RawMotionData>,
    ) {
        pending.building_sector_numbers.clear();

        // Count motion areas + obstacles in proto order — must match the
        // allocation loop in `initialize_motion_from_level_data`.
        let Some(md) = motion_data else {
            // No motion data ⇒ no grid sectors ⇒ nothing sensible to rewire.
            // Leave building doors alone; they'll keep their raw (wrong) values,
            // but without motion data nothing else in the engine can use them
            // anyway.
            return;
        };
        let mut next_sector: i16 = 0;
        for layer_areas in &md.layers {
            for area in layer_areas {
                next_sector += 1; // area polygon
                next_sector += area.obstacles.len() as i16; // obstacles
            }
        }

        // `lift_layer = special_layer - 1 = motion_data.layers.len()`.
        // Computing it from the raw motion data keeps this pass
        // independent of `fast_grid` init order.
        let building_lift_layer = md.layers.len() as u16;

        // Allocate one sector per Building entry, in proto order, and rewrite
        // each of its doors in place.  StandaloneDoors entries are left alone:
        // their `sector_in` already points at a real motion area.
        for entry in buildings.iter_mut() {
            let crate::level_data::RawBuildingEntry::Building { doors } = entry else {
                continue;
            };
            let sn = next_sector;
            next_sector += 1;
            pending.building_sector_numbers.push(sn);
            for door in doors.iter_mut() {
                door.sector_in = sn as u16;
                door.layer_in = building_lift_layer;
            }
        }
    }

    /// Initialize the fast find grid and pathfinder from motion data loaded from the proto level.
    ///
    /// Must be called after `load_background_map` sets `cutscene_camera.level_size`.
    pub(crate) fn initialize_motion_from_level_data(
        &mut self,
        assets: &mut LevelAssets,
        pending: &mut PendingLevelData,
        motion_data: &crate::level_data::RawMotionData,
        lifts: &[crate::level_data::RawLift],
    ) {
        let level_w = self.cutscene_camera.level_size.x as u16;
        let level_h = self.cutscene_camera.level_size.y as u16;

        // Size the grid from map dimensions.
        let grid_w = level_w / 64;
        let grid_h = level_h / 64;
        self.fast_grid.size_map(grid_w, grid_h);
        self.fast_grid
            .allocate_layers(motion_data.layers.len() as u16);

        // Register the already-loaded sight obstacles with the grid so
        // per-cell queries (`get_obstacle_indices`) can restrict the
        // 3D raycast scan to overlapping obstacles.
        // Snapshot (idx, layer, box_ground) before mutating fast_grid —
        // `self.sight_obstacles(assets)` borrows engine immutably while
        // `add_obstacle_index` needs `&mut self.fast_grid`.
        let obstacle_metadata: Vec<(u32, u16, crate::coordinates::GroundBBox)> = self
            .sight_obstacles(assets)
            .iter_indexed()
            .map(|(idx, obs)| (idx, obs.layer, obs.box_ground))
            .collect();
        for (obs_idx, layer, box_ground) in obstacle_metadata {
            if let Some(idx) = crate::sight_obstacle::SightObstacleIndex::new(obs_idx) {
                self.fast_grid.add_obstacle_index(idx, layer, &box_ground);
            }
        }

        // Drain raw masks stashed by `initialize_from_mission` and push the
        // decoded `RuntimeMask`s into the grid.  Masks are pushed just
        // after the grid is sized.
        let raw_masks = std::mem::take(&mut pending.masks);
        let raw_count = raw_masks.len();
        let mut added = 0usize;
        for raw in raw_masks {
            if let Some(mask) = crate::mask::RuntimeMask::from_raw(&raw) {
                self.fast_grid.add_mask(mask);
                added += 1;
            }
        }
        if raw_count > 0 {
            tracing::debug!(
                "Loaded {} masks into fast grid ({} skipped)",
                added,
                raw_count - added,
            );
        }

        // ── Elevation (bond) lines → grid lines ──
        //
        // Each bond line separates two adjacent sight obstacles on the
        // same layer. Register them as non-motion `GridLine`s tagged
        // `is_elevation = true` so the per-tick line-cross query in
        // `tick_entity_movement` can detect when an actor walks over
        // one and dispatch `cross_elevation_line`.
        //
        // The proto stores `right_obstacle_index` before `left`.
        let elev_raw = std::mem::take(&mut pending.elevation_lines);
        let num_obstacles = self.sight_obstacles(assets).len();
        let mut elev_added = 0usize;
        let mut elev_skipped_layer = 0usize;
        for raw in &elev_raw {
            let to_idx = |i: u16| -> Option<u16> {
                // 0xFFFF is the sentinel for "no obstacle" in the proto.
                if i == 0xFFFF {
                    None
                } else if (i as usize) < num_obstacles {
                    Some(i)
                } else {
                    None
                }
            };
            let left = to_idx(raw.left_obstacle_index);
            let right = to_idx(raw.right_obstacle_index);
            let line = crate::fast_find_grid::GridLine::new_elevation(
                MapPoint::new(raw.point_a.0 as f32, raw.point_a.1 as f32),
                MapPoint::new(raw.point_b.0 as f32, raw.point_b.1 as f32),
                left,
                right,
            );
            if (raw.layer as usize) >= self.fast_grid.level.layers.len() {
                elev_skipped_layer += 1;
                continue;
            }
            self.fast_grid.add_line(line, raw.layer);
            elev_added += 1;
        }
        if !elev_raw.is_empty() {
            tracing::debug!(
                "Loaded {} elevation lines into fast grid ({} skipped for bad layer)",
                elev_added,
                elev_skipped_layer,
            );
        }

        // ── Part 1: Motion obstacles → grid lines + pathfinder move_layers ──
        for (layer_idx, layer_areas) in motion_data.layers.iter().enumerate() {
            let mut move_areas = Vec::new();
            let mut alt_move_areas = Vec::new();

            for area in layer_areas {
                // Add motion area polygon lines to the grid: the
                // perimeter is both motion-blocking and repulsive, so
                // anti-collision pushes actors off walls instead of
                // letting them scrape along.
                let poly = &area.polygon;
                for i in 0..poly.points.len() {
                    let (x1, y1) = poly.points[i];
                    let (x2, y2) = poly.points[(i + 1) % poly.points.len()];
                    let mut line = crate::fast_find_grid::GridLine::new(
                        MapPoint::new(x1 as f32, y1 as f32),
                        MapPoint::new(x2 as f32, y2 as f32),
                        true, // is_motion
                    );
                    line.initialize_motion_normal(true);
                    line.set_repulsive(true);
                    self.fast_grid.add_line(line, layer_idx as u16);
                }
                // Emit cone-limited repulsive points at inward corners
                // of the motion area.  `det(v1, v2) < 0` marks an
                // inward corner (the wedge faces *into* the walkable
                // area, pushing actors away from the pinch point).
                let n = poly.points.len();
                if n > 2 {
                    for i in 0..n {
                        let (ax, ay) = poly.points[i];
                        let (bx, by) = poly.points[(i + 1) % n];
                        let (cx, cy) = poly.points[(i + 2) % n];
                        let v1 = (bx as f32 - ax as f32, by as f32 - ay as f32);
                        let v2 = (cx as f32 - bx as f32, cy as f32 - by as f32);
                        let det = v1.0 * v2.1 - v1.1 * v2.0;
                        if det < 0.0 {
                            // `GetNormal(false)` → (y, -x).
                            let limit_left = crate::coordinates::MapVec::new(v1.1, -v1.0);
                            let limit_right = crate::coordinates::MapVec::new(v2.1, -v2.0);
                            let is_concave =
                                crate::geo2d::cross(limit_left.to_geo(), limit_right.to_geo())
                                    < 0.0;
                            self.fast_grid.level_mut().level_repulsive_points.push(
                                crate::fast_find_grid::LevelRepulsivePoint {
                                    position: MapPoint::new(bx as f32, by as f32),
                                    layer: layer_idx as u16,
                                    limit_left,
                                    limit_right,
                                    is_concave,
                                },
                            );
                        }
                    }
                }

                // Build skeleton segments
                let skeleton: Vec<geo::Line<f32>> = area
                    .skeleton_segments
                    .iter()
                    .map(|&((x1, y1), (x2, y2))| {
                        crate::geo2d::segment(
                            crate::geo2d::pt(x1 as f32, y1 as f32),
                            crate::geo2d::pt(x2 as f32, y2 as f32),
                        )
                    })
                    .collect();

                // Add motion obstacle lines to the grid and build per-obstacle
                // metadata (state_id + bbox + polygon + grid-line indices) for
                // runtime state swaps.  The grid-line indices feed the reverse
                // mapping used by `SetLineForMotionSectorActive` so a
                // state transition can flip each perimeter line's
                // `line_active` flag without rescanning the layer.
                let mut obstacles = Vec::new();
                for obstacle in &area.obstacles {
                    let obs_poly = &obstacle.polygon;
                    let mut bbox = crate::coordinates::MapBBox::new();
                    let mut poly_pts: Vec<MapPoint> = Vec::with_capacity(obs_poly.points.len());
                    let mut line_indices: Vec<crate::fast_find_grid::LineIndex> =
                        Vec::with_capacity(obs_poly.points.len());
                    for i in 0..obs_poly.points.len() {
                        let (x1, y1) = obs_poly.points[i];
                        let (x2, y2) = obs_poly.points[(i + 1) % obs_poly.points.len()];
                        let mut line = crate::fast_find_grid::GridLine::new(
                            MapPoint::new(x1 as f32, y1 as f32),
                            MapPoint::new(x2 as f32, y2 as f32),
                            true,
                        );
                        line.initialize_motion_normal(false);
                        line.set_repulsive(true);
                        let line_idx = self.fast_grid.add_line(line, layer_idx as u16);
                        line_indices.push(line_idx);
                        let p = MapPoint::new(x1 as f32, y1 as f32);
                        bbox.expand_point(p);
                        poly_pts.push(p);
                    }
                    // Obstacle-corner repulsive points: `det(v1, v2) > 0`
                    // marks the convex outward corners of the obstacle
                    // polygon.
                    let on = obs_poly.points.len();
                    if on > 2 {
                        for i in 0..on {
                            let (ax, ay) = obs_poly.points[i];
                            let (ox, oy) = obs_poly.points[(i + 1) % on];
                            let (cx, cy) = obs_poly.points[(i + 2) % on];
                            let v1 = (ox as f32 - ax as f32, oy as f32 - ay as f32);
                            let v2 = (cx as f32 - ox as f32, cy as f32 - oy as f32);
                            let det = v1.0 * v2.1 - v1.1 * v2.0;
                            if det > 0.0 {
                                let limit_left = crate::coordinates::MapVec::new(v1.1, -v1.0);
                                let limit_right = crate::coordinates::MapVec::new(v2.1, -v2.0);
                                let is_concave =
                                    crate::geo2d::cross(limit_left.to_geo(), limit_right.to_geo())
                                        < 0.0;
                                self.fast_grid.level_mut().level_repulsive_points.push(
                                    crate::fast_find_grid::LevelRepulsivePoint {
                                        position: MapPoint::new(ox as f32, oy as f32),
                                        layer: layer_idx as u16,
                                        limit_left,
                                        limit_right,
                                        is_concave,
                                    },
                                );
                            }
                        }
                    }
                    obstacles.push(crate::pathfinder::MotionObstacle {
                        state_id: obstacle.state_id,
                        // Active by default; pathfinder::initialize() will
                        // flip this in line with the default state mask.
                        active: true,
                        bounding_box: bbox,
                        polygon: poly_pts,
                        grid_line_indices: line_indices,
                    });
                }

                // Store polygon vertices for point-in-area hit-testing.
                let polygon_pts: Vec<MapPoint> = area
                    .polygon
                    .points
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                    .collect();

                move_areas.push(crate::pathfinder::MotionArea {
                    skeleton,
                    polygon: polygon_pts,
                    motion_obstacles: obstacles,
                });
                alt_move_areas.push(crate::pathfinder::MotionArea {
                    skeleton: Vec::new(),
                    polygon: Vec::new(),
                    motion_obstacles: Vec::new(),
                });
            }

            let graph = std::sync::Arc::make_mut(&mut assets.pathfinder_graph);
            let static_data = graph.static_mut();
            static_data.move_layers.push(move_areas);
            static_data.alternative_move_layers.push(alt_move_areas);
        }

        // ── Part 2: Pathfinder graph ──
        if !motion_data.graph_bytes.is_empty()
            && let Err(e) = std::sync::Arc::make_mut(&mut assets.pathfinder_graph)
                .load_from_proto_stream(&mut self.fast_grid, &motion_data.graph_bytes)
        {
            tracing::error!("Failed to load pathfinder graph: {e}");
        }

        // ── Part 3: Build sector conversion table ──
        std::sync::Arc::make_mut(&mut assets.pathfinder_graph).build_sector_conversion();

        // ── Part 4: Initialize pathfinder obstacle states ──
        // Must happen after graph is loaded, not during engine.initialize() which
        // runs before load_background_map processes the motion data.
        self.pathfinder
            .initialize_from_graph(assets.pathfinder_graph.as_ref());

        // ── Part 5: Register sectors in grid blocks ──
        //
        // Each motion area polygon becomes a MOTION | AREA | MOUSE
        // sector in the grid, and each obstacle becomes MOTION (without
        // AREA).  This enables GetSector/GetSectorScreen spatial queries.
        {
            use crate::sector::SectorType;

            let mut sector_number = crate::sector::SectorNumber::new(0);
            let mut area_flat_idx: u16 = 0;

            for (layer_idx, layer_areas) in motion_data.layers.iter().enumerate() {
                for area in layer_areas {
                    // Register the walkable area polygon.
                    // ForceCrouched when flags != 0.
                    let force_crouched = area.flags != 0;
                    let mut area_type = SectorType::MOTION | SectorType::AREA | SectorType::MOUSE;
                    if area.is_lift {
                        area_type |= SectorType::LIFT;
                    }

                    let pts: Vec<_> = area
                        .polygon
                        .points
                        .iter()
                        .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                        .collect();
                    let mut bbox = MapBBox::new();
                    for &p in &pts {
                        bbox.expand_point(p);
                    }

                    self.fast_grid.add_sector(
                        crate::fast_find_grid::GridSector {
                            points: pts,
                            bounding_box: bbox,
                            sector_type: area_type,
                            layer: layer_idx as u16,
                            sector_number,
                            door_index: None,
                            lift_type: None,
                            lift_direction: 0,
                            force_crouched,
                            building_index: None,
                            low_exit_point: None,
                            high_exit_point: None,
                            lowest_door_index: None,
                            jump_line_indices: Vec::new(),
                            gate_indices: Vec::new(),
                            underlying_sector: None,
                        },
                        layer_idx as u16,
                    );
                    sector_number += 1;
                    area_flat_idx += 1;

                    // Register each obstacle polygon (MOTION without AREA)
                    for obstacle in &area.obstacles {
                        let obs_pts: Vec<_> = obstacle
                            .polygon
                            .points
                            .iter()
                            .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                            .collect();
                        let mut obs_bbox = MapBBox::new();
                        for &p in &obs_pts {
                            obs_bbox.expand_point(p);
                        }

                        self.fast_grid.add_sector(
                            crate::fast_find_grid::GridSector {
                                points: obs_pts,
                                bounding_box: obs_bbox,
                                sector_type: SectorType::MOTION,
                                layer: layer_idx as u16,
                                sector_number,
                                door_index: None,
                                lift_type: None,
                                lift_direction: 0,
                                force_crouched: false,
                                building_index: None,
                                low_exit_point: None,
                                high_exit_point: None,
                                lowest_door_index: None,
                                jump_line_indices: Vec::new(),
                                gate_indices: Vec::new(),
                                underlying_sector: None,
                            },
                            layer_idx as u16,
                        );
                        sector_number += 1;
                    }
                }
            }

            // ── Apply lift_type from RawLift data to grid sectors ──
            //
            // The `uwSector` word in CHUNK_LIFT is a sector_number, not
            // a motion-area-flat index — obstacles interleave with areas
            // in the sector array, so the two indices diverge once any
            // motion area has obstacles.  The
            // `RawLift::motion_area_index` field name is a misnomer kept
            // for compatibility.
            //
            // Look up by sector_number directly and set lift_type on the
            // GridSector. legacy implementation reads the lift's associated click sector
            // (`RHSectorLift::InitializeFromProtoStream`) but the two
            // `AddSector(pSectorAssociated, ...)` calls are commented out,
            // so the proto click_sector is not a live mouse-pick sector.
            // The real lift motion polygon remains mouse-enabled from
            // CHUNK_MOTION and is the only clickable lift geometry.
            for lift in lifts {
                let sn = crate::sector::SectorNumber::new(lift.motion_area_index as i16);
                let Some(&lift_grid_idx) = self.fast_grid.level.sector_number_map.get(&sn) else {
                    continue;
                };
                {
                    let level = self.fast_grid.level_mut();
                    if let Some(gs) = level.sectors.get_mut(lift_grid_idx) {
                        // The motion loader must already have marked the
                        // area as a lift.  Panic so malformed level data
                        // surfaces at load time (per project "No fake
                        // data" rule).
                        if !gs.sector_type.is_lift() {
                            panic!(
                                "Illegal lift sector: sector_number {} is not flagged as a lift — \
                                 CHUNK_LIFT references a sector that CHUNK_MOTION didn't mark is_lift=true",
                                i16::from(sn)
                            );
                        }
                        // Intercept `LiftType::Normal` (value 0), warn,
                        // and force `LiftType::Stairs`: default lifts
                        // are no longer supported, levels must use a
                        // simple motion area with "square"-doors. No
                        // live lift sector should end up as
                        // `LiftType::Normal`.
                        let mut raw_lift_type = crate::sector::LiftType::from_u8(lift.lift_type);
                        if raw_lift_type == crate::sector::LiftType::Normal {
                            tracing::warn!(
                                sector_number = i16::from(sn),
                                "default lifts are no longer supported, please use a simple \
                                 motion area with \"square\"-doors ! (forcing LIFT_STAIRS)"
                            );
                            raw_lift_type = crate::sector::LiftType::Stairs;
                        }
                        gs.lift_type = Some(raw_lift_type);
                        gs.lift_direction = lift.direction;
                        // `LIFT` bit is already set by the motion loader;
                        // OR again is a no-op but kept for symmetry.
                        gs.sector_type |= SectorType::LIFT;
                    }
                }
            }

            // ── Building sectors ──
            //
            // Every `Building` entry in the proto stream gets its own
            // sector with no polygon geometry, flagged
            // `MOTION | AREA | BUILDING`, living on the lift layer.
            // Doors inside the building have their `sector_in` rewritten
            // to point at the building sector, so a PC walking through
            // the door ends up with its sector pointer aimed at the
            // building sector.
            //
            // Sector number allocation and door `sector_in` rewrites both
            // already happened in `rewire_building_doors` during the initial
            // level load, so that the earlier `populate_game_host_from_level`
            // call sees the correct values.  Here we just walk the stashed
            // list and register the matching empty grid sectors — the
            // `debug_assert_eq!` catches any drift between the two passes.
            let building_lift_layer = self.fast_grid.lift_layer();
            let allocated = std::mem::take(&mut pending.building_sector_numbers);
            for (bld_idx, sn) in allocated.iter().copied().enumerate() {
                let sn_wrapped = crate::sector::SectorNumber::new(sn);
                debug_assert_eq!(
                    sn_wrapped, sector_number,
                    "building sector allocation drifted from motion layout \
                     — `rewire_building_doors` and \
                     `initialize_motion_from_level_data` must agree on the \
                     area/obstacle count"
                );
                self.fast_grid.add_sector(
                    crate::fast_find_grid::GridSector {
                        points: Vec::new(),
                        bounding_box: crate::coordinates::MapBBox::new(),
                        sector_type: SectorType::MOTION | SectorType::AREA | SectorType::BUILDING,
                        layer: building_lift_layer,
                        sector_number: sn_wrapped,
                        door_index: None,
                        lift_type: None,
                        lift_direction: 0,
                        force_crouched: false,
                        building_index: crate::sector::BuildingIdx::new(bld_idx as u16),
                        low_exit_point: None,
                        high_exit_point: None,
                        lowest_door_index: None,
                        jump_line_indices: Vec::new(),
                        gate_indices: Vec::new(),
                        underlying_sector: None,
                    },
                    building_lift_layer,
                );
                sector_number += 1;
            }

            // ── Light / shadow sectors ──
            //
            // Each raw light sector becomes a `SectorType::SHADOW` grid
            // sector on its own layer iff its ambience bitmask overlaps
            // the mission's ambience.  Sectors whose ambience bit is
            // clear are dropped.  `is_in_shadow_sector` queries these to
            // suppress the fog/night sprite variant when an actor stands
            // inside a torch-lit polygon.
            let ambiance_mask = self.weather.ambiance.to_bitmask();
            let raw_light_sectors = std::mem::take(&mut pending.light_sectors);
            let mut light_added = 0usize;
            let mut light_skipped_ambience = 0usize;
            let mut light_skipped_layer = 0usize;
            let mut light_skipped_polygon = 0usize;
            for raw in raw_light_sectors {
                if (raw.ambience & ambiance_mask) == 0 {
                    light_skipped_ambience += 1;
                    continue;
                }
                if (raw.layer as usize) >= self.fast_grid.level.layers.len() {
                    light_skipped_layer += 1;
                    continue;
                }
                if raw.polygon.points.len() < 3 {
                    light_skipped_polygon += 1;
                    continue;
                }
                let pts: Vec<_> = raw
                    .polygon
                    .points
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                    .collect();
                let mut bbox = MapBBox::new();
                for &p in &pts {
                    bbox.expand_point(p);
                }
                self.fast_grid.add_sector(
                    crate::fast_find_grid::GridSector {
                        points: pts,
                        bounding_box: bbox,
                        sector_type: SectorType::SHADOW,
                        layer: raw.layer,
                        sector_number,
                        door_index: None,
                        lift_type: None,
                        lift_direction: 0,
                        force_crouched: false,
                        building_index: None,
                        low_exit_point: None,
                        high_exit_point: None,
                        lowest_door_index: None,
                        jump_line_indices: Vec::new(),
                        gate_indices: Vec::new(),
                        underlying_sector: None,
                    },
                    raw.layer,
                );
                sector_number += 1;
                light_added += 1;
            }
            if light_added + light_skipped_ambience + light_skipped_layer + light_skipped_polygon
                > 0
            {
                tracing::debug!(
                    "Loaded {} shadow sectors ({} filtered by ambience, {} bad layer, {} degenerate polygon)",
                    light_added,
                    light_skipped_ambience,
                    light_skipped_layer,
                    light_skipped_polygon,
                );
            }

            // ── Shadow centroid + radius post-load (NIGHT/FOG only) ──
            //
            // For each SHADOW sector, when ambience is NIGHT or FOG,
            // compute the polygon centroid in 2D, look up the enclosing
            // `SECTOR_PLANE` to read its top-plane coefficients, and
            // derive the 3D centroid + average radius.
            //
            // Stored as a parallel `HashMap` keyed by `GridSector` index
            // so we don't bloat every (mostly non-shadow) GridSector with
            // an `Option<ShadowData>`.  Consumed by the night/fog branch
            // of `ai_vision::compute_view_radius`.
            let is_night_or_fog = matches!(
                self.weather.ambiance,
                crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
            );
            if is_night_or_fog && light_added > 0 {
                // Snapshot (idx, points, layer) without holding the
                // immutable borrow during the obstacle lookup pass.
                let shadow_inputs: Vec<(u32, Vec<MapPoint>, u16)> = self
                    .fast_grid
                    .level
                    .sectors
                    .iter()
                    .enumerate()
                    .filter(|(_, gs)| gs.sector_type.is_shadow())
                    .map(|(i, gs)| (i as u32, gs.points.clone(), gs.layer))
                    .collect();
                for (sector_idx, points, layer) in shadow_inputs {
                    let mut shadow = crate::sector::ShadowData::default();
                    shadow.initialize_2d(&points);

                    // Inline projection-area lookup.  Every plane sector
                    // wraps exactly one projection-area obstacle, so
                    // iterating obstacles gives the same answer as
                    // walking SECTOR_PLANE GridSectors and following
                    // their owning sight obstacle.
                    let bary = shadow.barycentre_2d;
                    let mut found_top_plane: Option<[[f32; 3]; 3]> = None;
                    for (oi, obs) in self.sight_obstacles(assets).iter_indexed() {
                        if !obs.is_projection_area() {
                            continue;
                        }
                        if obs.layer != layer {
                            continue;
                        }
                        if !obs.box_projection.contains_point(bary) {
                            continue;
                        }
                        if !obs.contains_point_projection(bary) {
                            continue;
                        }
                        found_top_plane = Some(obs.top_plane_points);
                        let _ = oi; // index unused beyond verification
                        break;
                    }
                    shadow.initialize_3d(found_top_plane.as_ref());

                    self.fast_grid
                        .level_mut()
                        .shadow_data
                        .insert(sector_idx, shadow);
                }
                tracing::debug!(
                    "Initialized shadow centroid data for {} sectors (NIGHT/FOG ambience)",
                    self.fast_grid.level.shadow_data.len(),
                );
            }

            tracing::info!(
                "Registered {} grid sectors ({} motion areas + obstacles, {} area-only)",
                self.fast_grid.level.sectors.len(),
                i16::from(sector_number),
                area_flat_idx,
            );
        }

        tracing::info!(
            "Motion initialized: {} layers, {} grid lines, {} path nodes, {} path links, {} pf sectors",
            motion_data.layers.len(),
            self.fast_grid.level.lines.len(),
            assets.pathfinder_graph.nodes.len(),
            assets.pathfinder_graph.static_data.links.len(),
            assets.pathfinder_graph.static_data.sector_conversion.len(),
        );

        // ── Jump zones + jump line pairs ──
        //
        // Must run after all motion-area sectors are registered so
        // `sector_number_map` lookups succeed for each jump zone's
        // sector number.
        self.load_jump_lines_from_proto(pending);
    }

    /// Minimum fall (negative jump height) before a jump line requires a
    /// helper.  A line's `jump_height = associated.z_a - this.z_a`, so a
    /// value below this threshold means the landing spot is at least 40
    /// units below the take-off.
    const JUMP_HEIGHT_HELPER_THRESHOLD: f32 = -40.0;

    /// Build runtime `JumpLine` entries from the stashed JZ/PPPP
    /// proto data and link them to their home sectors.
    ///
    /// Flow:
    /// 1. Read all jump zones (polygon + associated motion-area
    ///    sector number + layer).  Each line pair stores a
    ///    `jump_zone_index` pointing into this list.
    /// 2. Read each line pair.  For each pair `(line1, line2)`:
    ///    * Link them to each other.
    ///    * Give `line1` a home sector equal to `line2`'s jump
    ///      zone's sector (and symmetrically for `line2`).  This
    ///      places each line on the sector that its paired line
    ///      jumps *into*.
    ///    * Add `line1` to that sector's `jump_line_indices`
    ///      (and `line2` to the other).
    ///
    /// We skip jump-sector registration (polygon sectors for
    /// landing-spot lookup) because the table-swordfight path only
    /// needs the line endpoints and the sector linkage.
    pub(crate) fn load_jump_lines_from_proto(&mut self, pending: &mut PendingLevelData) {
        let jump_zones = std::mem::take(&mut pending.jump_zones);
        let line_pairs = std::mem::take(&mut pending.jump_line_pairs);
        if line_pairs.is_empty() {
            return;
        }

        // Resolve each jump zone's sector number to a grid-sector
        // index.  `RawJumpZone.sector` stores a sector number which
        // maps via `sector_number_map` to the flat sector index.
        let zone_sector: Vec<Option<u32>> = jump_zones
            .iter()
            .map(|z| {
                self.fast_grid
                    .level
                    .sector_number_map
                    .get(&crate::sector::SectorNumber::new(z.sector as i16))
                    .map(|&idx| idx as u32)
            })
            .collect();
        let zone_layer: Vec<u16> = jump_zones.iter().map(|z| z.layer).collect();

        let num_zones = jump_zones.len();
        let mut loaded_pairs = 0usize;
        // Count referenced jump lines per zone so we can fail loudly
        // when a zone has no jump line referencing it.
        let mut zone_line_counts = vec![0u32; num_zones];

        for pair in line_pairs {
            let z1 = pair.line1.jump_zone_index as usize;
            let z2 = pair.line2.jump_zone_index as usize;
            if z1 >= num_zones || z2 >= num_zones {
                tracing::warn!(
                    "Jump line pair references invalid zone index ({z1}, {z2}) / {num_zones}"
                );
                continue;
            }
            zone_line_counts[z1] += 1;
            zone_line_counts[z2] += 1;

            // Each line's home is its *paired* line's jump zone.
            let Some(sec1) = zone_sector[z2] else {
                tracing::warn!(
                    "Jump line pair zone {z2} has unresolved sector ({})",
                    jump_zones[z2].sector
                );
                continue;
            };
            let Some(sec2) = zone_sector[z1] else {
                tracing::warn!(
                    "Jump line pair zone {z1} has unresolved sector ({})",
                    jump_zones[z1].sector
                );
                continue;
            };
            let layer1 = zone_layer[z2];
            let layer2 = zone_layer[z1];

            let mut jl1 = crate::jump_line::JumpLine::new(
                crate::coordinates::map_pt(
                    pair.line1.point_a.0 as f32,
                    pair.line1.point_a.1 as f32,
                ),
                crate::coordinates::map_pt(
                    pair.line1.point_b.0 as f32,
                    pair.line1.point_b.1 as f32,
                ),
                pair.line1.point_a.2 as f32,
                pair.line1.point_b.2 as f32,
            );
            jl1.layer = layer1;
            jl1.sector_index = crate::fast_find_grid::SectorIndex::new(sec1);
            jl1.long_jump_forced = pair.jump_long;

            let mut jl2 = crate::jump_line::JumpLine::new(
                crate::coordinates::map_pt(
                    pair.line2.point_a.0 as f32,
                    pair.line2.point_a.1 as f32,
                ),
                crate::coordinates::map_pt(
                    pair.line2.point_b.0 as f32,
                    pair.line2.point_b.1 as f32,
                ),
                pair.line2.point_a.2 as f32,
                pair.line2.point_b.2 as f32,
            );
            jl2.layer = layer2;
            jl2.sector_index = crate::fast_find_grid::SectorIndex::new(sec2);
            jl2.long_jump_forced = pair.jump_long;

            // A line only requires a helper when *both* the drop from
            // its paired line exceeds `JUMP_HEIGHT_HELPER_THRESHOLD` (the
            // paired line is at least 40 units *below* the line's own
            // elevation) *and* either the line's own jump zone or the
            // paired zone has `helper_needed` set.  Using the zone's
            // flag directly forces a helper for shallow drops and misses
            // deep drops where only the paired zone is flagged.
            let either_zone_helper = jump_zones[z1].helper_needed || jump_zones[z2].helper_needed;
            // For each line, `jump_height = associated.z_a - this.z_a`.
            let jh1 = jl2.z_a - jl1.z_a;
            let jh2 = jl1.z_a - jl2.z_a;
            jl1.helper_needed = jh1 < Self::JUMP_HEIGHT_HELPER_THRESHOLD && either_zone_helper;
            jl2.helper_needed = jh2 < Self::JUMP_HEIGHT_HELPER_THRESHOLD && either_zone_helper;

            // Push both lines and cross-link their associated indices.
            let idx1 = self.fast_grid.level.jump_lines.len() as u32;
            let idx2 = idx1 + 1;
            jl1.associated_line_index = Some(idx2);
            jl2.associated_line_index = Some(idx1);
            // Remember the line geometry we need for the jump gate
            // below; we have to clone before moving the lines into
            // `fast_grid.level.jump_lines`.
            let jl1_mid = jl1.get_middle_point();
            let jl2_mid = jl2.get_middle_point();
            let jl1_layer = jl1.layer;
            let jl2_layer = jl2.layer;
            let jl1_helper_needed = jl1.helper_needed;
            let jl2_helper_needed = jl2.helper_needed;
            {
                let level = self.fast_grid.level_mut();
                level.jump_lines.push(jl1);
                level.jump_lines.push(jl2);

                // Register line indices on their home sectors so
                // `GetNearestJumpLine` can iterate without a global scan.
                if let Some(gs) = level.sectors.get_mut(sec1 as usize)
                    && let Some(idx) = crate::jump_line::JumpLineIndex::new(idx1)
                {
                    gs.jump_line_indices.push(idx);
                }
                if let Some(gs) = level.sectors.get_mut(sec2 as usize)
                    && let Some(idx) = crate::jump_line::JumpLineIndex::new(idx2)
                {
                    gs.jump_line_indices.push(idx);
                }
            }

            // Resolve the sectors' `sector_number` so the jump-gate Door
            // can reference them by the same IDs the rest of the door
            // table uses (sector_out / sector_in are sector numbers, not
            // grid-flat indices).
            let sector_num_out = self
                .fast_grid
                .level
                .sectors
                .get(sec2 as usize)
                .map(|s| s.sector_number);
            let sector_num_in = self
                .fast_grid
                .level
                .sectors
                .get(sec1 as usize)
                .map(|s| s.sector_number);

            // Stash the jump-gate Door spec for later push into
            // `game_host.doors`: compute the midpoint of each line as
            // the in/out point and use each line's home sector as the
            // in/out sector.
            //
            // We can't push directly here: the proto-stream phase
            // (`consume_pending_motion_data`) now runs before
            // `load_mission_script` / `populate_game_host_from_level`
            // so that beam-me / soldier sector-motion-area validations
            // see a populated grid (PROTO → MISSION load order).
            // `register_pending_jump_gates` drains this stash + rebuilds
            // gate links once `game_host` exists.
            if let (Some(num_out), Some(num_in)) = (sector_num_out, sector_num_in)
                && num_out.is_valid()
                && num_in.is_valid()
            {
                // Penalty: `||pt_in - pt_out|| + PENALTY_JUMP`.
                let pdx = jl1_mid.x - jl2_mid.x;
                let pdy = jl1_mid.y - jl2_mid.y;
                let penalty = (pdx * pdx + pdy * pdy).sqrt() + crate::gate::PENALTY_JUMP;

                pending
                    .jump_gate_specs
                    .push(crate::engine::types::PendingJumpGate {
                        point_out: jl2_mid,
                        point_in: jl1_mid,
                        layer_out: jl2_layer,
                        layer_in: jl1_layer,
                        sector_out: num_out,
                        sector_in: num_in,
                        jump_line_out: idx2,
                        jump_line_in: idx1,
                        // Cache each destination line's `helper_needed`
                        // flag so `Door::is_actor_authorized` can answer
                        // its destination-line branch without reading
                        // back into `fast_grid`.
                        jump_line_in_helper_needed: jl1_helper_needed,
                        jump_line_out_helper_needed: jl2_helper_needed,
                        penalty,
                    });
            } else {
                tracing::warn!(
                    "Jump line pair ({z1}/{z2}) failed to resolve sector numbers; \
                     skipping jump-gate registration"
                );
            }

            loaded_pairs += 1;
        }

        if loaded_pairs > 0 {
            tracing::debug!(
                "Loaded {} jump line pair(s) into fast grid ({} jump lines total)",
                loaded_pairs,
                self.fast_grid.level.jump_lines.len(),
            );
        }

        // ── Jump sector registration ──
        // Each jump zone becomes a `MOUSE | JUMP` grid sector so cursor
        // hit-tests can land on them; the `underlying_sector` link lets
        // `update_mouse` recurse into the motion area beneath when no
        // jump line resolves.
        let mut registered = 0usize;
        for (zi, zone) in jump_zones.iter().enumerate() {
            // Log instead of aborting when a zone has no jump line —
            // the zone still gets registered below so cursor hit-tests
            // are consistent, but the mismatch is loud enough to catch
            // authoring errors.
            if zone_line_counts[zi] == 0 {
                tracing::error!(
                    "Jump zone {} has no jump line referencing it (uwSector={}, layer={})",
                    zi,
                    zone.sector,
                    zone.layer,
                );
            }
            if zone.polygon.points.is_empty() {
                continue;
            }
            let points: Vec<MapPoint> = zone
                .polygon
                .points
                .iter()
                .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                .collect();
            let mut bbox = MapBBox::new();
            for &p in &points {
                bbox.expand_point(p);
            }
            let gs = crate::fast_find_grid::GridSector {
                points,
                bounding_box: bbox,
                sector_type: crate::sector::SectorType::MOUSE | crate::sector::SectorType::JUMP,
                layer: zone.layer,
                sector_number: crate::sector::SectorNumber::new(-1),
                door_index: None,
                lift_type: None,
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: zone_sector[zi]
                    .and_then(crate::fast_find_grid::SectorIndex::new),
            };
            self.fast_grid.add_sector(gs, zone.layer);
            registered += 1;
        }
        if registered > 0 {
            tracing::debug!("Registered {} jump sectors in fast grid", registered);
        }
    }

    /// Drain constructor-local pending jump-gate specs and push each entry as a
    /// `Door` (gate_type=Jump) into `game_host.doors`, then rebuild
    /// gate-link connectivity.  Must run after
    /// `populate_game_host_from_level` so `game_host` exists, and after
    /// every other proto/mission door has been registered so the
    /// gate-link rebuild sees the complete door table in one pass.
    pub(crate) fn register_pending_jump_gates(&mut self, pending: &mut PendingLevelData) {
        let specs = std::mem::take(&mut pending.jump_gate_specs);
        if specs.is_empty() {
            return;
        }
        let Some(game_host) = self.mission_script.as_mut().and_then(|s| s.game_host_mut()) else {
            tracing::warn!(
                "register_pending_jump_gates: no game_host — {} jump-gate Door(s) dropped",
                specs.len(),
            );
            return;
        };
        let count = specs.len();
        for spec in specs {
            game_host.doors.push(crate::gate::Door {
                gate_type: crate::gate::GateType::Jump,
                door_type: crate::gate::DoorType::Default,
                point_out: spec.point_out,
                point_in: spec.point_in,
                point_mid: MapPoint::new(
                    (spec.point_in.x + spec.point_out.x) * 0.5,
                    (spec.point_in.y + spec.point_out.y) * 0.5,
                ),
                layer_out: spec.layer_out,
                layer_in: spec.layer_in,
                sector_out: spec.sector_out,
                sector_in: spec.sector_in,
                jump_line_out: Some(spec.jump_line_out),
                jump_line_in: Some(spec.jump_line_in),
                jump_line_in_helper_needed: spec.jump_line_in_helper_needed,
                jump_line_out_helper_needed: spec.jump_line_out_helper_needed,
                penalty: spec.penalty,
                ..Default::default()
            });
        }
        crate::gate::build_gate_links(&mut game_host.doors);
        tracing::debug!("Registered {count} jump-gate Door(s) into game_host.doors");
    }

    /// Resolve each patch's old/new mask refs (layer + per-layer index)
    /// to flat `fast_grid.level.masks` indices, and flip each patch's new masks
    /// dormant so the patch starts in its "old" state.
    ///
    /// Must run after `initialize_motion_from_level_data` registers every
    /// mask in the grid.  Patches themselves are constructed earlier in
    /// `populate_game_host_from_level`, and their raw mask refs are stashed
    /// in constructor-local pending data keyed by patch index.
    pub(crate) fn resolve_patch_mask_refs(&mut self, pending: &mut PendingLevelData) {
        let refs = std::mem::take(&mut pending.patch_mask_refs);
        if refs.is_empty() {
            return;
        }
        let Some(game_host) = self.mission_script.as_mut().and_then(|s| s.game_host_mut()) else {
            return;
        };
        let mut missing_old = 0u32;
        let mut missing_new = 0u32;
        let mut missing_values: std::collections::BTreeSet<(u16, u16)> =
            std::collections::BTreeSet::new();
        for (patch_idx, (old_refs, new_refs)) in refs.iter().enumerate() {
            let Some(patch) = game_host.patches.get_mut(patch_idx) else {
                continue;
            };
            for mref in old_refs {
                let resolved = self
                    .fast_grid
                    .level
                    .layers
                    .get(mref.layer as usize)
                    .and_then(|l| l.mask_indices.get(mref.index as usize).copied());
                match resolved {
                    Some(idx) => patch.old_mask_indices.push(idx),
                    None => {
                        missing_old += 1;
                        missing_values.insert((mref.layer, mref.index));
                    }
                }
            }
            for mref in new_refs {
                let resolved = self
                    .fast_grid
                    .level
                    .layers
                    .get(mref.layer as usize)
                    .and_then(|l| l.mask_indices.get(mref.index as usize).copied());
                match resolved {
                    Some(idx) => patch.new_mask_indices.push(idx),
                    None => {
                        missing_new += 1;
                        missing_values.insert((mref.layer, mref.index));
                    }
                }
            }
            // Patch starts in its "old" state: old masks active, new masks
            // dormant.  `initially_active` in the proto doesn't affect
            // masks — the flip happens via `PatchEffect::SwapObjects`.
            for &idx in &patch.new_mask_indices {
                self.fast_grid.set_mask_active(idx, false);
            }
        }
        if missing_old > 0 || missing_new > 0 {
            tracing::warn!(
                "resolve_patch_mask_refs: {missing_old} old + {missing_new} new mask refs out of range (missing (layer,index)={:?})",
                missing_values,
            );
        }
    }

    /// Resolve each door's two endpoint sector numbers to their grid
    /// sectors and record the door index in each sector's
    /// `gate_indices`.
    ///
    /// Must run after `initialize_motion_from_level_data`, which is what
    /// populates `sector_number_map`.  Doors themselves are loaded earlier
    /// by `populate_game_host_from_level`.
    pub(crate) fn populate_sector_gates_from_doors(&mut self) {
        let door_count = self
            .mission_script
            .as_ref()
            .and_then(|s| s.game_host())
            .map(|h| h.doors.len())
            .unwrap_or(0);
        if door_count == 0 {
            return;
        }

        // Snapshot door endpoints so we can borrow `self.fast_grid` mutably
        // without also holding a reference into the script host.
        let endpoints: Vec<(u32, crate::sector::SectorNumber)> = {
            let game_host = self
                .mission_script
                .as_ref()
                .and_then(|s| s.game_host())
                .expect("door_count > 0 implies game host present");
            game_host
                .doors
                .iter()
                .enumerate()
                .flat_map(|(idx, door)| {
                    [(idx as u32, door.sector_out), (idx as u32, door.sector_in)]
                })
                .collect()
        };

        let mut missing = 0u32;
        let mut missing_values: std::collections::BTreeSet<i16> = std::collections::BTreeSet::new();
        for (door_idx, sector_number) in &endpoints {
            let Some(&grid_idx) = self.fast_grid.level.sector_number_map.get(sector_number) else {
                missing += 1;
                missing_values.insert(i16::from(*sector_number));
                continue;
            };
            let level = self.fast_grid.level_mut();
            if let Some(gs) = level.sectors.get_mut(grid_idx)
                && gs.sector_type.is_motion()
                && gs.sector_type.is_area()
            {
                gs.gate_indices
                    .push(crate::gate::DoorIndex::from(*door_idx));
            }
        }
        if missing > 0 {
            tracing::warn!(
                "populate_sector_gates_from_doors: {missing}/{} door endpoints referenced unknown sector numbers (missing values={:?})",
                endpoints.len(),
                missing_values,
            );
        }
    }

    // ─── GameHost ↔ EngineInner wiring ─────────────────────────────────

    /// Populate the script host with doors, patches, entity state and
    /// PC authorisation bits from the loaded level data.  Called once,
    /// right after the mission script is loaded.
    fn populate_game_host_from_level(
        &mut self,
        assets: &LevelAssets,
        pending: &mut PendingLevelData,
        loaded: &crate::level_data::LoadedLevel,
    ) {
        let script = match self.mission_script.as_mut() {
            Some(s) => s,
            None => return,
        };
        let game_host = match script.game_host_mut() {
            Some(h) => h,
            None => return,
        };

        // ── Doors ──
        // Collect every RawDoor from buildings / standalone-door entries.
        //
        // For `Building` entries we also record the resulting door handles
        // in `building_gates[bld_idx]` so script natives (PutActorInBuilding,
        // SetBuildingActive) can find the first gate of a given building.
        // The building's gates are exactly the doors declared inside its
        // proto entry.
        let mut bld_idx: usize = 0;
        for entry in &loaded.proto.buildings {
            let (raw_doors, is_building) = match entry {
                crate::level_data::RawBuildingEntry::Building { doors } => (doors, true),
                crate::level_data::RawBuildingEntry::StandaloneDoors { doors } => (doors, false),
            };
            let first_handle =
                crate::natives::GameHost::door_handle_from_index(game_host.doors.len());
            for raw in raw_doors {
                // Standalone (non-building) doors must be Default (0),
                // Gate (3), or Trap (7).
                if !is_building && !matches!(raw.door_type, 0 | 3 | 7) {
                    panic!(
                        "Illegal standalone door type {} at ({}, {}): must be DEFAULT, GATE, or TRAP",
                        raw.door_type, raw.point_mid.0, raw.point_mid.1
                    );
                }
                let door_type = match raw.door_type {
                    1 => crate::gate::DoorType::Building,
                    2 => crate::gate::DoorType::BuildingTrap,
                    3 => crate::gate::DoorType::Gate,
                    4 => crate::gate::DoorType::LiftHigh,
                    5 => crate::gate::DoorType::LiftLow,
                    6 => crate::gate::DoorType::LiftHighCrenel,
                    7 => crate::gate::DoorType::Trap,
                    8 => crate::gate::DoorType::Reinforcement,
                    _ => crate::gate::DoorType::Default,
                };
                let (act_d1, act_d2, act_i1, act_i2) =
                    crate::gate::Door::default_actions_for_type(door_type);
                game_host.doors.push(crate::gate::Door {
                    gate_type: crate::gate::GateType::Door,
                    active: raw.active,
                    door_type,
                    locked_pc: raw.locked_pc,
                    locked_npc_villain: raw.locked_npc_villain,
                    locked_npc_civilian: raw.locked_npc_civilian,
                    unlockable: raw.unlockable,
                    locked_pc_after_patch: raw.locked_pc_after_patch,
                    locked_npc_villain_after_patch: raw.locked_npc_villain_after_patch,
                    locked_npc_civilian_after_patch: raw.locked_npc_civilian_after_patch,
                    unlockable_after_patch: raw.unlockable_after_patch,
                    special_authorisation_pc: false,
                    authorised_pc_direct: 0,
                    authorised_pc_indirect: 0,
                    point_out: MapPoint::new(raw.point_out.0 as f32, raw.point_out.1 as f32),
                    point_in: MapPoint::new(raw.point_in.0 as f32, raw.point_in.1 as f32),
                    point_mid: MapPoint::new(raw.point_mid.0 as f32, raw.point_mid.1 as f32),
                    layer_out: raw.layer_out,
                    layer_in: raw.layer_in,
                    sector_out: crate::sector::SectorNumber::new(raw.sector_out as i16),
                    sector_in: crate::sector::SectorNumber::new(raw.sector_in as i16),
                    gate_links: Vec::new(),
                    click_polygon: raw
                        .door_sector
                        .points
                        .iter()
                        .map(|&(x, y)| (x as f32, y as f32))
                        .collect(),
                    click_bbox: crate::coordinates::MapBBox::new(),
                    penalty: 0.0,
                    patch_index: None,
                    gate_state: crate::gate::GateState::default(),
                    jump_line_out: None,
                    jump_line_in: None,
                    jump_line_in_helper_needed: false,
                    jump_line_out_helper_needed: false,
                    action_direct_1: act_d1,
                    action_direct_2: act_d2,
                    action_indirect_1: act_i1,
                    action_indirect_2: act_i2,
                });
                if let Some(door) = game_host.doors.last_mut() {
                    // Apply the `adapt_points` shift before computing the
                    // penalty: building-trap / wall-lift entries have their
                    // `point_in` offset from `point_mid`.  Non-lift building
                    // / standalone doors never hit a wall-lift branch, so
                    // `lift_wall = false` is correct.
                    door.adapt_points(false);
                    // `penalty = |point_in - point_out| + PENALTY_{BUILDING|DEFAULT}`.
                    // Must run after `adapt_points`.
                    door.compute_door_penalty();
                    door.rebuild_click_bbox();
                }
            }
            if is_building {
                let last_handle = game_host.doors.len() as i32;
                let gates: Vec<i32> = (first_handle..=last_handle).collect();
                if bld_idx >= game_host.building_gates.len() {
                    game_host.building_gates.resize(bld_idx + 1, Vec::new());
                }
                game_host.building_gates[bld_idx] = gates;
                bld_idx += 1;
            }
        }

        // Also collect doors from lifts.
        for lift in &loaded.proto.lifts {
            // `adapt_points` guards its LiftHigh / LiftHighCrenel arms
            // on whether the host lift is a wall lift. We already know
            // the hosting lift's type from the proto stream, so we
            // forward that bit to the Door method rather than looking it
            // up from the grid later.
            let lift_wall =
                crate::sector::LiftType::from_u8(lift.lift_type) == crate::sector::LiftType::Wall;
            for raw in &lift.doors {
                let door_type = match raw.door_type {
                    1 => crate::gate::DoorType::Building,
                    4 => crate::gate::DoorType::LiftHigh,
                    5 => crate::gate::DoorType::LiftLow,
                    6 => crate::gate::DoorType::LiftHighCrenel,
                    _ => crate::gate::DoorType::Default,
                };
                let (act_d1, act_d2, act_i1, act_i2) =
                    crate::gate::Door::default_actions_for_type(door_type);
                game_host.doors.push(crate::gate::Door {
                    gate_type: crate::gate::GateType::Door,
                    active: raw.active,
                    door_type,
                    locked_pc: raw.locked_pc,
                    locked_npc_villain: raw.locked_npc_villain,
                    locked_npc_civilian: raw.locked_npc_civilian,
                    unlockable: raw.unlockable,
                    locked_pc_after_patch: raw.locked_pc_after_patch,
                    locked_npc_villain_after_patch: raw.locked_npc_villain_after_patch,
                    locked_npc_civilian_after_patch: raw.locked_npc_civilian_after_patch,
                    unlockable_after_patch: raw.unlockable_after_patch,
                    point_out: MapPoint::new(raw.point_out.0 as f32, raw.point_out.1 as f32),
                    point_in: MapPoint::new(raw.point_in.0 as f32, raw.point_in.1 as f32),
                    point_mid: MapPoint::new(raw.point_mid.0 as f32, raw.point_mid.1 as f32),
                    layer_out: raw.layer_out,
                    layer_in: raw.layer_in,
                    sector_out: crate::sector::SectorNumber::new(raw.sector_out as i16),
                    sector_in: crate::sector::SectorNumber::new(raw.sector_in as i16),
                    click_polygon: raw
                        .door_sector
                        .points
                        .iter()
                        .map(|&(x, y)| (x as f32, y as f32))
                        .collect(),
                    click_bbox: crate::coordinates::MapBBox::new(),
                    action_direct_1: act_d1,
                    action_direct_2: act_d2,
                    action_indirect_1: act_i1,
                    action_indirect_2: act_i2,
                    ..Default::default()
                });
                if let Some(door) = game_host.doors.last_mut() {
                    // Order: `adapt_points` then penalty.  LiftHigh /
                    // LiftHighCrenel doors on wall lifts get their
                    // `point_in` nudged toward `point_mid`; other lift
                    // types leave `point_in` alone.
                    door.adapt_points(lift_wall);
                    door.compute_door_penalty();
                    door.rebuild_click_bbox();
                }
            }
        }

        // Build gate links: connect doors that share a sector.
        // Jump gates are appended later by `load_jump_lines_from_proto`,
        // which re-invokes `build_gate_links` to cover them too.
        crate::gate::build_gate_links(&mut game_host.doors);
        let total_links: usize = game_host.doors.iter().map(|d| d.gate_links.len()).sum();
        tracing::info!(
            "Built gate connectivity graph: {} doors, {} links",
            game_host.doors.len(),
            total_links,
        );

        // Note: motion-area sector `gate_indices` are populated later, in
        // `populate_sector_gates_from_doors`, which runs after
        // `initialize_motion_from_level_data` has registered the sectors
        // in `sector_number_map`.

        // Register door click polygons as grid sectors (DOOR | MOUSE)
        // so GetSector/GetSectorScreen can find them during click hit-testing.
        {
            use crate::sector::SectorType;
            // Register the door's clickable polygon on
            // `max(layer_out, layer_in)`, but exclude the grid's
            // `special_layer` from the bump — keeping the door reachable
            // from the higher of the two real layers without accidentally
            // landing it on the out-of-map layer.
            let special_layer = self.fast_grid.level.special_layer;
            let mut door_sectors_registered = 0u32;
            for (door_idx, door) in game_host.doors.iter().enumerate() {
                if door.click_polygon.is_empty() {
                    continue;
                }
                let pts: Vec<_> = door
                    .click_polygon
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x, y))
                    .collect();
                let bbox = door.click_bbox;
                // Start with layer_out; bump to layer_in iff it's
                // strictly higher AND not the special (out-of-map) layer.
                let mut layer = door.layer_out;
                if door.layer_in > layer && door.layer_in != special_layer {
                    layer = door.layer_in;
                }

                let door_active = door.active;
                let idx = self.fast_grid.add_sector(
                    crate::fast_find_grid::GridSector { points: pts,
                    bounding_box: bbox,
                    sector_type: SectorType::DOOR | SectorType::MOUSE,
                    layer,
                    sector_number: crate::sector::SectorNumber::new(-1), /* Doors don't have motion sector numbers */
                    door_index: Some(door_idx as u32),
                    lift_type: None,
                    lift_direction: 0,
                    force_crouched: false,
                    building_index: None,
                    low_exit_point: None,
                    high_exit_point: None,
                    lowest_door_index: None, jump_line_indices: Vec::new(),
                    gate_indices: Vec::new(),
                    underlying_sector: None,
                },
                    layer,
                );
                self.fast_grid.set_sector_active(idx, door_active);
                door_sectors_registered += 1;
            }
            tracing::info!(
                "Registered {} door click sectors in grid",
                door_sectors_registered,
            );
        }

        tracing::info!(
            "GameHost: populated {} doors from level data",
            game_host.doors.len(),
        );

        // The legacy implementation lift sectors expose high/low exit points for
        // DetermineMovementAnimation. Populate the Rust cache after the
        // GameHost door table is fully loaded; an earlier motion-sector pass
        // may run before these doors exist.
        for gs in &mut self.fast_grid.level_mut().sectors {
            if gs.sector_type.is_lift() || gs.lift_type.is_some() {
                gs.low_exit_point = None;
                gs.high_exit_point = None;
                gs.lowest_door_index = None;
            }
        }
        let mut lift_endpoints_cached = 0usize;
        let mut lift_endpoints_partial = 0usize;
        for (door_idx, door) in game_host.doors.iter().enumerate() {
            for sector_number in [door.sector_out, door.sector_in] {
                let Some(&grid_idx) = self.fast_grid.level.sector_number_map.get(&sector_number)
                else {
                    continue;
                };
                let Some(gs) = self.fast_grid.level_mut().sectors.get_mut(grid_idx) else {
                    continue;
                };
                if !(gs.sector_type.is_lift() || gs.lift_type.is_some()) {
                    continue;
                }
                let door_idx = door_idx as u32;
                let lowest = gs
                    .lowest_door_index
                    .and_then(|prev| game_host.doors.get(prev as usize))
                    .map(|prev| prev.point_in.y)
                    .is_none_or(|prev_y| door.point_in.y > prev_y);
                if lowest {
                    gs.low_exit_point = Some(door.point_in);
                    gs.lowest_door_index = Some(door_idx);
                }
                let highest = gs
                    .high_exit_point
                    .map(|prev| prev.y)
                    .is_none_or(|prev_y| door.point_in.y < prev_y);
                if highest {
                    gs.high_exit_point = Some(door.point_in);
                }
            }
        }
        for gs in &self.fast_grid.level.sectors {
            if !(gs.sector_type.is_lift() || gs.lift_type.is_some()) {
                continue;
            }
            match (gs.low_exit_point, gs.high_exit_point) {
                (Some(_), Some(_)) => lift_endpoints_cached += 1,
                (Some(_), None) | (None, Some(_)) => {
                    lift_endpoints_partial += 1;
                    if matches!(
                        gs.lift_type,
                        Some(crate::sector::LiftType::Wall | crate::sector::LiftType::Ladder)
                    ) {
                        panic!(
                            "Lift sector {} ({:?}) missing high/low exit points after door load",
                            gs.sector_number, gs.lift_type
                        );
                    }
                }
                (None, None) => {
                    if matches!(
                        gs.lift_type,
                        Some(crate::sector::LiftType::Wall | crate::sector::LiftType::Ladder)
                    ) {
                        panic!(
                            "Lift sector {} ({:?}) missing high/low exit points after door load",
                            gs.sector_number, gs.lift_type
                        );
                    }
                }
            }
        }
        tracing::debug!(
            "Loaded lift exit points after door load: {} sectors fully resolved, {} partial",
            lift_endpoints_cached,
            lift_endpoints_partial,
        );

        // ── Patches ──
        for (patch_idx, raw) in loaded.proto.patches.iter().enumerate() {
            // Copy sight obstacle indices directly — they index into
            // EngineInner::sight_obstacles which is loaded from the same proto.
            let old_sight: Vec<crate::sight_obstacle::SightObstacleIndex> = raw
                .old_sight_obstacles
                .iter()
                .filter_map(|&i| crate::sight_obstacle::SightObstacleIndex::new(u32::from(i)))
                .collect();
            let new_sight: Vec<crate::sight_obstacle::SightObstacleIndex> = raw
                .new_sight_obstacles
                .iter()
                .filter_map(|&i| crate::sight_obstacle::SightObstacleIndex::new(u32::from(i)))
                .collect();

            // Register old/new sector polygons in the FastFindGrid.
            let mut old_sector_indices: Vec<u32> = Vec::new();
            let mut new_sector_indices: Vec<u32> = Vec::new();

            // Helper: register a SectorPolygon as a GridSector if non-empty.
            let register_sector = |grid: &mut crate::fast_find_grid::FastFindGrid,
                                   poly: &crate::level_data::SectorPolygon,
                                   sector_type: crate::sector::SectorType,
                                   active: bool,
                                   layer: u16|
             -> Option<u32> {
                if poly.points.is_empty() {
                    return None;
                }
                let points: Vec<MapPoint> = poly
                    .points
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                    .collect();
                let mut bbox = MapBBox::new();
                for &p in &points {
                    bbox.expand_point(p);
                }
                let gs = crate::fast_find_grid::GridSector {
                    points,
                    bounding_box: bbox,
                    sector_type,
                    layer,
                    sector_number: crate::sector::SectorNumber::new(-1), /* Patch sectors don't have motion sector numbers */
                    door_index: None,
                    lift_type: None,
                    lift_direction: 0,
                    force_crouched: false,
                    building_index: None,
                    low_exit_point: None,
                    high_exit_point: None,
                    lowest_door_index: None,
                    jump_line_indices: Vec::new(),
                    gate_indices: Vec::new(),
                    underlying_sector: None,
                };
                let idx = grid.add_sector(gs, layer);
                grid.set_sector_active(idx, active);
                Some(idx)
            };

            // Mirrors C++ `mFastGrid.AddPatch(pPatch, pPatch->GetLayer(), …)`
            // — the late `READ(muwLayer)` at the end of `RHPatch::Initialize`
            // clobbers the mid-stream read.
            let patch_layer = raw.final_layer;
            let mouse_patch = crate::sector::SectorType::MOUSE | crate::sector::SectorType::PATCH;
            let mouse_motion = crate::sector::SectorType::MOUSE | crate::sector::SectorType::MOTION;

            // Old sectors: active = true (visible before patch fires)
            if let Some(idx) = register_sector(
                &mut self.fast_grid,
                &raw.old_mouse_sector,
                mouse_patch,
                true,
                patch_layer,
            ) {
                old_sector_indices.push(idx);
            }
            if let Some(idx) = register_sector(
                &mut self.fast_grid,
                &raw.old_masking_sector,
                mouse_motion,
                true,
                patch_layer,
            ) {
                old_sector_indices.push(idx);
            }
            // New sectors: active = false (hidden until patch fires)
            if let Some(idx) = register_sector(
                &mut self.fast_grid,
                &raw.new_mouse_sector,
                mouse_patch,
                false,
                patch_layer,
            ) {
                new_sector_indices.push(idx);
            }
            if let Some(idx) = register_sector(
                &mut self.fast_grid,
                &raw.new_masking_sector,
                mouse_motion,
                false,
                patch_layer,
            ) {
                new_sector_indices.push(idx);
            }

            // ── Apply / NoApply sectors ──
            //
            //   - `apply_sector`    = CROSS | PATCH | APPLY, always
            //     active when non-empty
            //   - `no_apply_sector` = CROSS | PATCH, always active when
            //     non-empty
            // Both get registered with `AddSector` + `AddSectorLines`
            // because they are cross-sectors.  The resulting
            // LINE_PATCH | LINE_CROSS boundary segments feed the per-PC
            // patch auto-trigger.
            let cross_patch_apply = crate::sector::SectorType::CROSS
                | crate::sector::SectorType::PATCH
                | crate::sector::SectorType::APPLY;
            let cross_patch = crate::sector::SectorType::CROSS | crate::sector::SectorType::PATCH;

            // Register the apply polygon as a cross-sector + build its
            // LINE_PATCH boundary lines carrying this patch's index.
            let apply_sector_idx = register_sector(
                &mut self.fast_grid,
                &raw.apply_sector,
                cross_patch_apply,
                true,
                patch_layer,
            );
            if let Some(idx) = apply_sector_idx
                && let Some(patch_index) = crate::patch::PatchIndex::new(patch_idx as u32)
            {
                self.fast_grid.add_sector_lines_for_patch(
                    idx,
                    patch_layer,
                    patch_index,
                    true, // apply sector is always active
                );
            }

            // Register the no-apply polygon identically, minus the APPLY bit.
            if let Some(idx) = register_sector(
                &mut self.fast_grid,
                &raw.no_apply_sector,
                cross_patch,
                true,
                patch_layer,
            ) && let Some(patch_index) = crate::patch::PatchIndex::new(patch_idx as u32)
            {
                self.fast_grid.add_sector_lines_for_patch(
                    idx,
                    patch_layer,
                    patch_index,
                    true, // no-apply sector is always active
                );
            }

            // Old/new line indices are populated at runtime when a patch
            // is triggered — they don't appear in the proto stream, so we
            // start empty.
            let old_line_indices: Vec<crate::fast_find_grid::LineIndex> = Vec::new();
            let new_line_indices: Vec<crate::fast_find_grid::LineIndex> = Vec::new();

            // Mask index resolution is deferred: at this point
            // `initialize_motion_from_level_data` hasn't run yet, so
            // `fast_grid.level.layers[L].mask_indices` is still empty.  Stash the
            // raw refs keyed by patch index and resolve them in
            // `resolve_patch_mask_refs`, which also flips `new_masks` dormant.
            pending
                .patch_mask_refs
                .push((raw.old_masks.clone(), raw.new_masks.clone()));
            let old_mask_indices: Vec<crate::mask::MaskIndex> = Vec::new();
            let new_mask_indices: Vec<crate::mask::MaskIndex> = Vec::new();

            game_host.patches.push(crate::patch::Patch {
                active: raw.active,
                // `initially_active` is unconditionally overridden to
                // `true` (a debug-leftover line, but it is what the
                // shipped binary does), so script-driven `ForceReset`
                // re-activates the patch as the game expects.
                initially_active: true,
                definitive: raw.definitive,
                animated: true, // default
                door_triggered: raw.door_triggered,
                triggers_door: raw.triggers_door,
                integrate_in_background: raw.integrate_in_background,
                animation_flags: crate::patch::AnimationFlags {
                    start_valid: raw.start_animation_valid,
                    transition_valid: raw.transition_animation_valid,
                    end_valid: raw.end_animation_valid,
                },
                use_changing_obstacles: raw.pathfinder_changing_obstacles != 0,
                pathfinder_layer: raw.pathfinder_layer.unwrap_or(0),
                pathfinder_sector: raw.pathfinder_sector.unwrap_or(0),
                pathfinder_changing_obstacles: raw.pathfinder_changing_obstacles,
                // C++ reads muwLayer twice (mid-stream and again at end of
                // patch); the late `final_layer` clobbers the early read,
                // so that's the authoritative value used by `AddPatch`
                // and `mpSelectedPatch->GetLayer()`.
                layer: raw.final_layer,
                sector: raw.sector,
                waypoint: MapPoint::new(raw.waypoint.0 as f32, raw.waypoint.1 as f32),
                old_sight_obstacle_indices: old_sight,
                new_sight_obstacle_indices: new_sight,
                old_sector_indices,
                new_sector_indices,
                old_line_indices,
                new_line_indices,
                old_mask_indices,
                new_mask_indices,
                apply_sector_index: apply_sector_idx,
                ..Default::default()
            });
        }

        // Wire door↔patch connections.  In C++ (`RHpatch.cpp:300-308`)
        // the two relationships are *mutually exclusive* — a patch's
        // door list is either consumed as door→patch links (every door
        // gets `mpPatch = this`) when `mbDoorTriggered`, or as the
        // patch→door swap-rights list (`maDoors`) when `mbTriggersDoor`.
        // Mirror that here: gate `Door::patch_index` on `door_triggered`
        // and `Patch::door_indices` on `triggers_door`.
        let mut door_triggered_count = 0_usize;
        let mut triggers_door_count = 0_usize;
        for (patch_idx, raw) in loaded.proto.patches.iter().enumerate() {
            let patch_door_indices: Vec<u32> = raw
                .door_indices
                .iter()
                .filter_map(|&raw_idx| {
                    let idx = raw_idx as u32;
                    if (idx as usize) < game_host.doors.len() {
                        Some(idx)
                    } else {
                        tracing::warn!(
                            "Patch {}: door_index {} out of range (have {} doors)",
                            patch_idx,
                            idx,
                            game_host.doors.len()
                        );
                        None
                    }
                })
                .collect();
            if !patch_door_indices.is_empty() && !raw.door_triggered && !raw.triggers_door {
                tracing::warn!(
                    "Patch {} lists {} door(s) but neither door_triggered nor \
                     triggers_door is set — wiring will be skipped",
                    patch_idx,
                    patch_door_indices.len()
                );
            }
            if raw.door_triggered {
                for &door_idx in &patch_door_indices {
                    game_host.doors[door_idx as usize].patch_index =
                        crate::patch::PatchIndex::new(patch_idx as u32);
                }
                if !patch_door_indices.is_empty() {
                    door_triggered_count += 1;
                }
            }
            if raw.triggers_door
                && let Some(patch) = game_host.patches.get_mut(patch_idx)
            {
                let n = patch_door_indices.len();
                patch.door_indices = patch_door_indices;
                if n > 0 {
                    triggers_door_count += 1;
                }
            }
        }
        tracing::debug!(
            door_triggered_patches = door_triggered_count,
            triggers_door_patches = triggers_door_count,
            "patch↔door wiring complete"
        );

        // ── Patch animation entities ──
        // Transfer the entity handle mapping computed during entity spawning.
        game_host.patch_animation_entities = assets.patch_entity_handles.clone();
        tracing::info!(
            "GameHost: populated {} patches from level data ({} with FX entities)",
            game_host.patches.len(),
            game_host
                .patch_animation_entities
                .iter()
                .filter(|h| h.is_some())
                .count(),
        );

        // ── Building occupants from tenant data ──
        for (bld_idx, tenants) in loaded.mission.building_tenants.iter().enumerate() {
            if bld_idx >= game_host.building_occupants.len() {
                game_host.building_occupants.resize(bld_idx + 1, Vec::new());
            }
            // Parallel array: propagate the `arrow_reserve` flag off the
            // same tenant chunk so `initialize_buildings` can copy it
            // into `ai::House::arrow_reserve`.  Consumer: AI's
            // `FleeingRunForArrowReserves` substate.
            if bld_idx >= game_host.arrow_reserves.len() {
                game_host.arrow_reserves.resize(bld_idx + 1, false);
            }
            game_host.arrow_reserves[bld_idx] = tenants.arrow_reserve;
            for &elem_idx in &tenants.tenant_element_indices {
                let actor_h =
                    crate::natives::GameHost::actor_handle_from_index(usize::from(elem_idx));
                game_host.building_occupants[bld_idx].push(actor_h);
                let bld_h = crate::natives::GameHost::building_handle_from_index(bld_idx);
                game_host.actor_building.insert(actor_h, bld_h);
            }
        }

        // ── Entity active state (FX animations) ──
        // Populated here once; kept in sync by sync_game_host_post_script.
        self.refresh_game_host_entity_state();

        // ── PC auth bits ──
        self.refresh_game_host_pc_auth_bits();
    }

    /// Harvest the Sherwood engine state into the campaign's production
    /// sectors just before exiting Sherwood: for every sector, capture
    /// amount from engine bonuses and occupants from script-zone
    /// membership.  Invoked at mission start.
    pub(crate) fn harvest_production_sector_state(&mut self, assets: &LevelAssets) {
        // Build a (production_type → occupants Vec) map by walking every
        // script zone that carries a production type.  Capturing occupants
        // here keeps the borrow of `campaign` short (we only update it
        // after all engine reads are done).
        let mut per_sector_occupants: std::collections::HashMap<
            crate::sector_production::Type,
            Vec<crate::sector_production::Occupant>,
        > = std::collections::HashMap::new();

        for (zone_idx, zone) in self.script_zone_data.iter().enumerate() {
            let prod_type = zone.production_sector_type;
            if prod_type == crate::sector_production::Type::Unknown
                || prod_type == crate::sector_production::Type::Relic
            {
                // RELIC sectors don't accept occupants; UNKNOWN has no
                // associated sector.
                continue;
            }

            // TRAIN_BOW additionally requires the PC to own Action::Bow.
            let train_bow_filter = prod_type == crate::sector_production::Type::TrainBow;

            let captured = per_sector_occupants.entry(prod_type).or_default();

            for &elem_idx in &zone.occupant_indices {
                let slot = self.entities.get(elem_idx);
                let Some(entity) = slot else { continue };
                let crate::element::Entity::Pc(pc) = entity else {
                    continue;
                };

                let profile_idx = pc.pc.profile_index;

                if train_bow_filter {
                    let Some(_campaign) = self.campaign.as_ref() else {
                        continue;
                    };
                    let Some(profile) = assets.profile_manager.get_character(profile_idx) else {
                        panic!(
                            "TRAIN_BOW occupant profile {profile_idx} missing from ProfileManager"
                        );
                    };
                    if !profile.has_action(crate::profiles::Action::Bow) {
                        continue;
                    }
                }

                // Find the PcDescription index (position in campaign.characters).
                let Some(campaign) = self.campaign.as_ref() else {
                    continue;
                };
                let Some(pc_description_idx) = campaign
                    .characters
                    .iter()
                    .position(|desc| desc.character_profile_idx == Some(profile_idx))
                else {
                    // The zone contained a PC whose profile isn't in the
                    // campaign's PcDescription list — that's a state-
                    // consistency bug, not a fallback case.
                    panic!(
                        "production sector PC profile {profile_idx} has no PcDescription in campaign (handle={elem_idx}, zone={zone_idx})"
                    );
                };

                let obstacle = pc.element.obstacle_index().map(u16::from).unwrap_or(0xFFFF);

                captured.push(crate::sector_production::Occupant {
                    pc_description_idx,
                    x: pc.element.position_map().x,
                    y: pc.element.position_map().y,
                    obstacle,
                });
            }
        }

        // Now that engine reads are done, write into the campaign sectors:
        // amount harvest (from entities) + occupants (from zones above).
        let entities_snapshot = &self.entities;
        let Some(campaign) = self.campaign.as_mut() else {
            return;
        };
        for sector in &mut campaign.production_sectors {
            // Bonus-amount branch runs for MAKE_* / any sector with an
            // associated action; no-op for TRAIN/HEAL/RELIC.
            sector.get_amount_from_current_mission(entities_snapshot);

            // Replace occupants with the fresh snapshot: sectors
            // whose type has no zone in the new map (or whose zone
            // produced no valid PCs) drop their previous occupants
            // instead of keeping them across mission cycles.
            //
            // Keying is still per-prod_type rather than per-sector
            // identity; stock data only ever has one zone per
            // production type so multiple SectorProductions sharing a
            // type all see the same captured set.  If future data
            // exposes multiple zones per type, switch this to a
            // per-sector zone index.
            sector.occupants.clear();
            if let Some(new_occupants) = per_sector_occupants.get(&sector.prod_type) {
                sector.occupants = new_occupants.clone();
            }

            // Clear `production_points` after the per-sector capture so
            // the next Sherwood load's `apply_production_registrations`
            // (which re-runs the script's `AddProductionPoint` opcodes)
            // doesn't accumulate duplicate points across visits — without
            // this, `plan_bonus_spawns` iterates more points than exist
            // in the level and raises `max_amount_reached` prematurely
            // while spawning duplicate-position bonuses.
            sector.production_points.clear();
        }
    }

    /// Apply stored production-sector state to the live Sherwood engine:
    /// finalize amounts (UpdateAmount), spawn MAKE_* bonuses at production
    /// points, spawn collected RELIC items, restore occupant PCs to their
    /// recorded positions, apply training XP (TRAIN_BOW / TRAIN_HAND_TO_HAND)
    /// and heal occupants (HEAL) for won missions.  Invoked at Sherwood
    /// entry.
    ///
    /// Must be called AFTER `apply_production_registrations` so production
    /// points are populated, and only when the current level is Sherwood.
    /// Called from `Engine::new` when Sherwood is the current mission.
    pub(crate) fn apply_production_sector_data(&mut self, assets: &mut LevelAssets) {
        use crate::sector_production::Type as PT;

        // Resolve last-mission info — drives UpdateAmount/Experience/LifePoints.
        let (last_won, last_length) = {
            let Some(campaign) = self.campaign.as_ref() else {
                return;
            };
            match campaign.last_mission_idx {
                Some(idx) => {
                    let mission = &campaign.missions[idx];
                    (
                        mission.status == crate::mission::MissionStatus::Won,
                        mission.profile(&assets.profile_manager).length,
                    )
                }
                None => (false, 0),
            }
        };

        // Resolve per-sector specialist flags + run UpdateAmount (adds
        // produced_amount into amount).  Collect a per-sector "plan" of
        // follow-up actions (spawn bonuses, spawn relics, restore occupants,
        // add XP, heal), along with the script-zone layer/sector needed to
        // position restored occupants.
        let points_count = assets
            .script_location_positions
            .len()
            .saturating_sub(self.script_zone_data.len());

        struct SectorPlan {
            prod_type: PT,
            points: Vec<crate::sector_production::Point>,
            bonus_spawns: Vec<(usize, u16)>, // (point_idx, quantity)
            occupants: Vec<crate::sector_production::Occupant>,
            zone_layer: Option<u16>,
            zone_sector: Option<u16>,
            // Applied-in-engine post-mission PC updates
            xp_gain: u16,   // TRAIN_* only
            heal_gain: u16, // HEAL only
        }

        let mut plans: Vec<SectorPlan> = Vec::new();

        // Build a zone-type → (layer, sector) map for attaching sectors.
        let mut zone_location: std::collections::HashMap<PT, (u16, u16)> =
            std::collections::HashMap::new();
        for (zone_idx, zone) in self.script_zone_data.iter().enumerate() {
            let pt = zone.production_sector_type;
            if pt == PT::Unknown {
                continue;
            }
            let loc_handle_idx = points_count + zone_idx; // 0-based index into script_location_*
            if let (Some(&layer), Some(&sector)) = (
                assets.script_location_layers.get(loc_handle_idx),
                assets.script_location_sectors.get(loc_handle_idx),
            ) {
                zone_location.entry(pt).or_insert((layer, sector));
            }
        }

        // Finalize amounts + gather plan data.
        {
            let Some(campaign) = self.campaign.as_mut() else {
                return;
            };
            // snapshot specialist resolution before mutating occupants — it
            // reads the *current* occupants (captured just before mission).
            let has_specialist: Vec<bool> = campaign
                .production_sectors
                .iter()
                .map(|s| {
                    if s.associated_action().is_some()
                        || matches!(s.prod_type, PT::TrainBow | PT::TrainHandToHand | PT::Heal)
                    {
                        Self::sector_has_specialist_cached(campaign, &assets.profile_manager, s)
                    } else {
                        false
                    }
                })
                .collect();

            for (sector, &specialist) in campaign
                .production_sectors
                .iter_mut()
                .zip(has_specialist.iter())
            {
                match sector.prod_type {
                    PT::MakeArrow
                    | PT::MakePurse
                    | PT::MakeStone
                    | PT::MakeApple
                    | PT::MakeAle
                    | PT::MakeLamblegg
                    | PT::MakePlant
                    | PT::MakeNet
                    | PT::MakeWaspNest => {
                        if last_won {
                            sector.update_amount(last_length, specialist);
                        }
                        let bonus_spawns = sector.plan_bonus_spawns();
                        let (layer, sector_idx) =
                            zone_location.get(&sector.prod_type).copied().unzip();
                        plans.push(SectorPlan {
                            prod_type: sector.prod_type,
                            points: sector.production_points.clone(),
                            bonus_spawns,
                            occupants: sector.occupants.clone(),
                            zone_layer: layer,
                            zone_sector: sector_idx,
                            xp_gain: 0,
                            heal_gain: 0,
                        });
                    }
                    PT::TrainBow | PT::TrainHandToHand => {
                        let xp = if last_won {
                            // UpdateExperience:
                            //   training_speed = speed / 100.0
                            //   super = specialist ? 2.0 : 1.0
                            //   gain = super * training_speed * length
                            let training = (sector.speed as f32) / 100.0;
                            let super_mul = if specialist { 2.0 } else { 1.0 };
                            (super_mul * training * last_length as f32) as u16
                        } else {
                            0
                        };
                        let (layer, sector_idx) =
                            zone_location.get(&sector.prod_type).copied().unzip();
                        plans.push(SectorPlan {
                            prod_type: sector.prod_type,
                            points: Vec::new(),
                            bonus_spawns: Vec::new(),
                            occupants: sector.occupants.clone(),
                            zone_layer: layer,
                            zone_sector: sector_idx,
                            xp_gain: xp,
                            heal_gain: 0,
                        });
                    }
                    PT::Heal => {
                        let heal = if last_won {
                            // UpdateLifePoints:
                            //   healing = speed / 100.0
                            //   super = specialist ? 1.5 : 1.0
                            //   amt = super * healing * length
                            let healing = (sector.speed as f32) / 100.0;
                            let super_mul = if specialist { 1.5 } else { 1.0 };
                            (super_mul * healing * last_length as f32) as u16
                        } else {
                            0
                        };
                        let (layer, sector_idx) =
                            zone_location.get(&sector.prod_type).copied().unzip();
                        plans.push(SectorPlan {
                            prod_type: sector.prod_type,
                            points: Vec::new(),
                            bonus_spawns: Vec::new(),
                            occupants: sector.occupants.clone(),
                            zone_layer: layer,
                            zone_sector: sector_idx,
                            xp_gain: 0,
                            heal_gain: heal,
                        });
                    }
                    PT::Relic => {
                        let (layer, sector_idx) = zone_location.get(&PT::Relic).copied().unzip();
                        plans.push(SectorPlan {
                            prod_type: PT::Relic,
                            points: sector.production_points.clone(),
                            bonus_spawns: Vec::new(), // relics handled separately
                            occupants: Vec::new(),
                            zone_layer: layer,
                            zone_sector: sector_idx,
                            xp_gain: 0,
                            heal_gain: 0,
                        });
                    }
                    PT::Unknown => {}
                }
            }
        }

        // Snapshot collected relics for the RELIC branch.
        let collected_relics: Vec<u32> = self
            .campaign
            .as_ref()
            .map(|c| c.collected_relics.clone())
            .unwrap_or_default();

        let char_base_dir = "Data/Characters";
        let bank_signature = assets.bank_signature;
        // Sherwood production-spawned bonuses are never blipped on the
        // minimap, even on non-forest maps.
        let blipped = false;

        for plan in &plans {
            // ── MAKE_*: spawn bonus entities at production points ──
            if let Some(raw_bonus) = plan.prod_type.bonus_raw_type()
                && let Some((sprite_file, profile_name, object_type)) =
                    bonus_type_to_sprite_asset(raw_bonus)
            {
                let bonus_kind = crate::element::BonusItemType::from_u16(raw_bonus);
                let associated_action = bonus_kind.to_action();
                for &(point_idx, quantity) in &plan.bonus_spawns {
                    let Some(point) = plan.points.get(point_idx) else {
                        continue;
                    };
                    let mut sprite = crate::sprite::Sprite::default();
                    if let Err(e) = sprite.load_frame_info(
                        assets.sprite_scriptor_mut(),
                        crate::sprite_script::FrameKind::Object,
                        char_base_dir,
                        sprite_file,
                        profile_name,
                        bank_signature,
                        Some(self.weather.ambiance.to_sprite_ambiance()),
                    ) {
                        tracing::error!(
                            "Sherwood production bonus sprite '{sprite_file}' / '{profile_name}' failed: {e}"
                        );
                        continue;
                    }
                    sprite.force_random_sprite_frame(&mut self.rng);
                    sprite.apply_placement(
                        MapPoint::new(point.x, point.y),
                        point.layer,
                        crate::position_interface::SectorHandle::new(point.sector),
                        0,
                        crate::element::GameMaterial::default(),
                        crate::position_interface::ObstacleHandle::new(point.obstacle),
                        crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                            crate::position_interface::ObstacleHandle::new(point.obstacle),
                            assets.static_sight_obstacles.as_slice(),
                        ),
                    );
                    let entity = crate::element::Entity::Bonus(crate::element::ElementBonus {
                        element: crate::element::ElementData {
                            kind: crate::element::ElementKind::ObjectBonus,
                            blipped,
                            sprite,
                            ..Default::default()
                        },
                        object: crate::element::ObjectData {
                            quantity,
                            object_type,
                            associated_action,
                            ..Default::default()
                        },
                    });
                    self.add_entity(entity);
                }
            }

            // ── RELIC: spawn one bonus entity per collected relic, one per point ──
            if plan.prod_type == PT::Relic {
                for (relic_idx, &relic_raw) in collected_relics.iter().enumerate() {
                    let Some(point) = plan.points.get(relic_idx) else {
                        break;
                    };
                    let Some((sprite_file, profile_name, object_type)) =
                        bonus_type_to_sprite_asset(relic_raw as u16)
                    else {
                        tracing::warn!(
                            "Unknown relic raw type {relic_raw} — cannot resolve sprite"
                        );
                        continue;
                    };
                    let bonus_kind = crate::element::BonusItemType::from_u16(relic_raw as u16);
                    let associated_action = bonus_kind.to_action();
                    let mut sprite = crate::sprite::Sprite::default();
                    if let Err(e) = sprite.load_frame_info(
                        assets.sprite_scriptor_mut(),
                        crate::sprite_script::FrameKind::Object,
                        char_base_dir,
                        sprite_file,
                        profile_name,
                        bank_signature,
                        Some(self.weather.ambiance.to_sprite_ambiance()),
                    ) {
                        tracing::error!(
                            "Sherwood relic sprite '{sprite_file}' / '{profile_name}' failed: {e}"
                        );
                        continue;
                    }
                    sprite.force_random_sprite_frame(&mut self.rng);
                    sprite.apply_placement(
                        MapPoint::new(point.x, point.y),
                        point.layer,
                        crate::position_interface::SectorHandle::new(point.sector),
                        0,
                        crate::element::GameMaterial::default(),
                        crate::position_interface::ObstacleHandle::new(point.obstacle),
                        crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                            crate::position_interface::ObstacleHandle::new(point.obstacle),
                            assets.static_sight_obstacles.as_slice(),
                        ),
                    );
                    let entity = crate::element::Entity::Bonus(crate::element::ElementBonus {
                        element: crate::element::ElementData {
                            kind: crate::element::ElementKind::ObjectBonus,
                            blipped,
                            sprite,
                            ..Default::default()
                        },
                        object: crate::element::ObjectData {
                            quantity: 1,
                            object_type,
                            associated_action,
                            ..Default::default()
                        },
                    });
                    self.add_entity(entity);
                }
            }

            // ── Occupant restore + work-icon set + XP/heal ──
            for occupant in &plan.occupants {
                // Resolve the PC description → profile_index → find live entity.
                let profile_idx = {
                    let Some(campaign) = self.campaign.as_ref() else {
                        continue;
                    };
                    let Some(desc) = campaign.characters.get(occupant.pc_description_idx) else {
                        panic!(
                            "occupant pc_description_idx {} out of range",
                            occupant.pc_description_idx
                        );
                    };
                    match desc.character_profile_idx {
                        Some(p) => p,
                        None => continue, // unlinked description — skip
                    }
                };

                let Some(pc_id) = self
                    .entities
                    .pcs()
                    .find_map(|(id, pc)| (pc.pc.profile_index == profile_idx).then_some(id))
                else {
                    // PC isn't present in this Sherwood load (e.g. not yet
                    // rescued, or lost) — skip silently.
                    continue;
                };
                let entity_id = EntityId::Pc(pc_id);

                let (Some(layer), Some(sector_idx)) = (plan.zone_layer, plan.zone_sector) else {
                    // No script-zone for this production type — cannot
                    // position.  Leave the PC where it is.
                    continue;
                };

                // Teleport, then refresh obstacle + plane + material via
                // the shared `set_obstacle_and_material` helper: with an
                // obstacle, pull the obstacle's top-plane and material;
                // without one, clear the plane and resolve the footstep
                // material from the SECTOR_SOUND polygons at the current
                // map position (or `default_material` when none contain
                // the point).
                if let Some(crate::element::Entity::Pc(pc)) = self.entities.get_mut(entity_id) {
                    pc.element
                        .set_position_map(MapPoint::new(occupant.x, occupant.y));
                    pc.element.set_layer(layer);
                    pc.element
                        .set_sector(crate::position_interface::SectorHandle::new(sector_idx));
                    {
                        let pi = &mut pc.element.sprite.position_iface;
                        pi.set_map_position(MapPoint::new(occupant.x, occupant.y));
                    }
                }
                // Release the `entities` borrow before calling helpers
                // that take `&mut self`.
                let occupant_obstacle_opt = if occupant.obstacle == 0xFFFF {
                    None
                } else {
                    Some(occupant.obstacle)
                };
                self.set_obstacle_and_material(assets, entity_id, occupant_obstacle_opt);
                if let Some(crate::element::Entity::Pc(pc)) = self.entities.get_mut(entity_id) {
                    pc.element.update_grid_cell();
                }

                // Set the work icon for the production type.
                let pt = plan.prod_type;
                self.apply_production_work_icon(entity_id, pt, true);

                // XP / heal on the live PC.
                if plan.xp_gain > 0 {
                    let skill = if plan.prod_type == PT::TrainBow {
                        crate::pc_status::SkillName::Bow
                    } else {
                        crate::pc_status::SkillName::HandToHand
                    };
                    // TRAIN_BOW's occupant filter already ensured
                    // Action::Bow at harvest time — no re-check here.
                    //
                    // Sherwood training uses `human_status.add_experience`
                    // directly rather than `campaign.add_pc_experience`,
                    // which deliberately bypasses the campaign-score
                    // bonus that the PC override awards when capacity
                    // crosses a 100-XP boundary.  Going through
                    // `add_pc_experience` would over-credit Sherwood
                    // training by 100 Score per skill-capacity threshold
                    // crossed.
                    if let Some(campaign) = self.campaign.as_mut()
                        && let Some(desc) = campaign.characters.get_mut(occupant.pc_description_idx)
                    {
                        desc.status
                            .human_status
                            .add_experience(skill, plan.xp_gain as u32);
                    }
                }
                if plan.heal_gain > 0 {
                    let amount = plan.heal_gain.min(i16::MAX as u16) as i16;
                    if let Some(crate::element::Entity::Pc(pc)) = self.entities.get_mut(entity_id) {
                        crate::pc_status::heal(&mut pc.pc.life_points, amount, false);
                    }
                    if let Some(campaign) = self.campaign.as_mut()
                        && let Some(desc) = campaign.characters.get_mut(occupant.pc_description_idx)
                    {
                        crate::pc_status::heal(&mut desc.status.life_points, amount, false);
                    }
                }
            }
        }

        // After teleporting occupants to their recorded positions,
        // rebuild zone membership against the new positions without
        // firing any `EnterZone` scripts.  Without this,
        // `initialize_zone_occupants` (which ran earlier against
        // pre-teleport positions) has left stale entries that the
        // per-frame `tick_zone_occupants` would reconcile by
        // dispatching spurious `ExitZone` / `EnterZone` events.
        self.refresh_zone_occupants_silent(assets);
    }

    /// Free-function variant of `Campaign::sector_has_specialist` that takes
    /// the campaign by shared ref.  Exists so we can call it while holding
    /// a `&mut Vec<SectorProduction>` borrow at the same time.
    fn sector_has_specialist_cached(
        campaign: &crate::campaign::Campaign,
        profiles: &crate::profiles::ProfileManager,
        sector: &crate::sector_production::SectorProduction,
    ) -> bool {
        campaign.sector_has_specialist(sector, profiles)
    }
}

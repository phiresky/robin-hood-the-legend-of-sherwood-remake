//! Phase two: resolve deferred submitted references against approved static content.
//! Only successful exhaustion of this phase constructs the opaque approval owner.
use super::*;

/// Exact public sector identity accepted for a persisted Sherwood production
/// point. The pair must come from the manifest-approved mounted Sherwood map,
/// not from submitted replay state.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ReplayCampaignProductionPointTopology {
    pub map_layer: u16,
    pub sector: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayCampaignApprovedMapPoint {
    pub x: f32,
    pub y: f32,
}

/// One ordered authored static obstacle. Runtime-created dynamic obstacles
/// are intentionally absent because their indices are not durable campaign
/// identities.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayCampaignApprovedStaticObstacle {
    pub projection_topology: Option<ReplayCampaignProductionPointTopology>,
    pub projected_polygon: Vec<ReplayCampaignApprovedMapPoint>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayCampaignApprovedScriptZone {
    /// Canonical production slot registered by the approved Sherwood script,
    /// or `None` when this authored zone is unrelated to production.
    pub production_slot: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayCampaignApprovedBeamMe {
    pub map_layer: u16,
    pub sector: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayCampaignApprovedSector {
    pub topology: ReplayCampaignProductionPointTopology,
    /// Boundary-inclusive map-space polygon for exact point containment.
    pub polygon: Vec<ReplayCampaignApprovedMapPoint>,
}

/// Ordered, read-only identities derived from manifest-approved mounted
/// Sherwood content. The verifier adapter must retain source order exactly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayCampaignApprovedContentMetadata {
    pub static_sight_obstacles: Vec<ReplayCampaignApprovedStaticObstacle>,
    pub script_zones: Vec<ReplayCampaignApprovedScriptZone>,
    pub beam_mes: Vec<ReplayCampaignApprovedBeamMe>,
    pub production_sectors: Vec<ReplayCampaignApprovedSector>,
}

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
pub enum ReplayCampaignMetadataDerivationError {
    #[error("approved Sherwood layer index {index} does not fit the replay topology schema")]
    LayerIndexOverflow { index: usize },
    #[error("approved Sherwood public motion-sector numbering overflowed")]
    SectorNumberOverflow,
    #[error(
        "approved Sherwood production slot {production_slot} registered absent script zone {zone_index}; raw zone count is {zone_count}"
    )]
    ProductionZoneOutOfRange {
        production_slot: usize,
        zone_index: usize,
        zone_count: usize,
    },
    #[error(
        "approved Sherwood script zone {zone_index} is registered by production slots {first_slot} and {duplicate_slot}"
    )]
    DuplicateProductionZone {
        zone_index: usize,
        first_slot: usize,
        duplicate_slot: usize,
    },
}

/// Derive the exact phase-two catalog from the manifest-approved raw
/// Sherwood level plus a separately initialized, trusted Sherwood campaign.
///
/// Script-zone production registration is executable authored behavior, not
/// present in `.rhm` geometry. The caller therefore boots one clean reference
/// Engine from the same approved assets and passes its post-Initialize
/// campaign here. Submitted campaign bytes are not involved in this step.
pub fn derive_replay_campaign_approved_content_metadata(
    loaded: &robin_engine::level_data::LoadedLevel,
    initialized_sherwood_campaign: &Campaign,
) -> Result<ReplayCampaignApprovedContentMetadata, ReplayCampaignMetadataDerivationError> {
    let static_sight_obstacles = loaded
        .proto
        .sight_obstacles
        .iter()
        .map(|obstacle| ReplayCampaignApprovedStaticObstacle {
            projection_topology: obstacle.projection_area.map(|(sector, map_layer)| {
                ReplayCampaignProductionPointTopology { map_layer, sector }
            }),
            projected_polygon: obstacle
                .points
                .iter()
                .map(|point| ReplayCampaignApprovedMapPoint {
                    x: point.x,
                    y: point.y,
                })
                .collect(),
        })
        .collect();

    let script_zone_count = loaded
        .mission
        .script_objects
        .as_ref()
        .map_or(0, |objects| objects.sectors.len());
    let mut script_zones = vec![
        ReplayCampaignApprovedScriptZone {
            production_slot: None,
        };
        script_zone_count
    ];
    for (production_slot, production) in initialized_sherwood_campaign
        .production_sectors
        .iter()
        .enumerate()
    {
        let Some(zone_index) = production.script_zone else {
            continue;
        };
        let Some(zone) = script_zones.get_mut(zone_index) else {
            return Err(
                ReplayCampaignMetadataDerivationError::ProductionZoneOutOfRange {
                    production_slot,
                    zone_index,
                    zone_count: script_zone_count,
                },
            );
        };
        if let Some(first_slot) = zone.production_slot.replace(production_slot) {
            return Err(
                ReplayCampaignMetadataDerivationError::DuplicateProductionZone {
                    zone_index,
                    first_slot,
                    duplicate_slot: production_slot,
                },
            );
        }
    }

    let beam_mes = loaded
        .mission
        .beam_mes
        .iter()
        .map(|beam_me| ReplayCampaignApprovedBeamMe {
            map_layer: beam_me.layer,
            sector: beam_me.sector,
        })
        .collect();

    let mut production_sectors = Vec::new();
    let mut public_sector = 0_u16;
    if let Some(motion) = &loaded.proto.motion_data {
        for (layer_index, areas) in motion.layers.iter().enumerate() {
            let map_layer = u16::try_from(layer_index).map_err(|_| {
                ReplayCampaignMetadataDerivationError::LayerIndexOverflow { index: layer_index }
            })?;
            for area in areas {
                production_sectors.push(ReplayCampaignApprovedSector {
                    topology: ReplayCampaignProductionPointTopology {
                        map_layer,
                        sector: public_sector,
                    },
                    polygon: area
                        .polygon
                        .points
                        .iter()
                        .map(|&(x, y)| ReplayCampaignApprovedMapPoint {
                            x: f32::from(x),
                            y: f32::from(y),
                        })
                        .collect(),
                });
                public_sector = public_sector
                    .checked_add(1)
                    .ok_or(ReplayCampaignMetadataDerivationError::SectorNumberOverflow)?;
                for obstacle in &area.obstacles {
                    production_sectors.push(ReplayCampaignApprovedSector {
                        topology: ReplayCampaignProductionPointTopology {
                            map_layer,
                            sector: public_sector,
                        },
                        polygon: obstacle
                            .polygon
                            .points
                            .iter()
                            .map(|&(x, y)| ReplayCampaignApprovedMapPoint {
                                x: f32::from(x),
                                y: f32::from(y),
                            })
                            .collect(),
                    });
                    public_sector = public_sector
                        .checked_add(1)
                        .ok_or(ReplayCampaignMetadataDerivationError::SectorNumberOverflow)?;
                }
            }
        }
    }

    Ok(ReplayCampaignApprovedContentMetadata {
        static_sight_obstacles,
        script_zones,
        beam_mes,
        production_sectors,
    })
}

/// Resolver supplied by the verifier worker's manifest-validated, read-only
/// mounted content catalog.
///
/// This trait is a data adapter, not an attestation boundary: a public Rust
/// implementation is forgeable. The disposable worker, pinned content
/// manifest, and read-only mount establish trust. API/supervisor processes
/// must not implement this from submitter-controlled metadata.
pub trait ReplayCampaignApprovedContentResolver {
    fn approved_content_identity(&self) -> ReplayCampaignApprovedContentIdentity;

    fn sherwood_campaign_metadata(&self) -> Option<&ReplayCampaignApprovedContentMetadata>;
}

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
pub enum ReplayCampaignContentValidationError {
    #[error("approved mounted content identity has an all-zero {field} digest")]
    InvalidApprovedContentIdentity { field: String },
    #[error("approved mounted content has no Sherwood campaign metadata")]
    MissingSherwoodMetadata,
    #[error(
        "approved Sherwood metadata collection {collection} has {observed} entries, limit {limit}"
    )]
    ApprovedMetadataLimit {
        collection: String,
        observed: usize,
        limit: usize,
    },
    #[error("approved Sherwood metadata collection {collection} overflows its aggregate count")]
    ApprovedMetadataCountOverflow { collection: String },
    #[error("approved-content polygon containment work would reach {attempted}, limit {limit}")]
    ApprovedContainmentWorkLimit { attempted: usize, limit: usize },
    #[error("approved Sherwood {collection}[{index}] polygon is invalid: {reason}")]
    ApprovedPolygonInvalid {
        collection: String,
        index: usize,
        reason: String,
    },
    #[error("approved Sherwood metadata repeats production topology {topology:?}")]
    DuplicateApprovedProductionTopology {
        topology: ReplayCampaignProductionPointTopology,
    },
    #[error(
        "approved Sherwood script zones {first_zone} and {duplicate_zone} both register production slot {production_slot}"
    )]
    DuplicateApprovedProductionScriptSlot {
        production_slot: usize,
        first_zone: usize,
        duplicate_zone: usize,
    },
    #[error(
        "approved Sherwood script zone {zone_index} registers invalid production slot {production_slot}"
    )]
    ApprovedProductionScriptSlotOutOfRange {
        zone_index: usize,
        production_slot: usize,
    },
    #[error(
        "campaign static sight-obstacle reference {reference:?} is outside approved authored total {approved_count}"
    )]
    StaticSightObstacleOutOfRange {
        reference: DeferredSightObstacleReference,
        approved_count: usize,
    },
    #[error(
        "campaign production-point obstacle {reference:?} has approved projection {actual:?}, expected {expected:?}"
    )]
    ProductionPointObstacleTopologyMismatch {
        reference: DeferredSightObstacleReference,
        expected: ReplayCampaignProductionPointTopology,
        actual: Option<ReplayCampaignProductionPointTopology>,
    },
    #[error("campaign production-point obstacle {reference:?} does not contain its stored point")]
    ProductionPointOutsideObstacle {
        reference: DeferredSightObstacleReference,
    },
    #[error("campaign production-occupant obstacle {reference:?} is not a projection area")]
    ProductionOccupantObstacleHasNoProjection {
        reference: DeferredSightObstacleReference,
    },
    #[error(
        "campaign production-occupant obstacle {reference:?} does not contain its stored point"
    )]
    ProductionOccupantOutsideObstacle {
        reference: DeferredSightObstacleReference,
    },
    #[error(
        "campaign production script-zone reference {reference:?} is outside approved total {approved_count}"
    )]
    ScriptZoneOutOfRange {
        reference: DeferredProductionScriptZoneReference,
        approved_count: usize,
    },
    #[error(
        "campaign production script-zone reference {reference:?} is registered for approved slot {actual_slot:?}"
    )]
    ScriptZoneProductionMismatch {
        reference: DeferredProductionScriptZoneReference,
        actual_slot: Option<usize>,
    },
    #[error("campaign production topology reference {reference:?} is absent from approved content")]
    ProductionPointTopologyMissing {
        reference: DeferredProductionPointTopologyReference,
    },
    #[error(
        "campaign production topology reference {reference:?} does not contain its stored point"
    )]
    ProductionPointOutsideSector {
        reference: DeferredProductionPointTopologyReference,
    },
    #[error(
        "campaign Sherwood beam-me reference {reference:?} is outside approved total {approved_count}"
    )]
    BeamMeOutOfRange {
        reference: DeferredSherwoodBeamMeReference,
        approved_count: usize,
    },
    #[error(
        "campaign deferred sight-obstacle reference {reference:?} requires a missing restart snapshot"
    )]
    DeferredSightSnapshotMissing {
        reference: DeferredSightObstacleReference,
    },
    #[error(
        "campaign deferred sight-obstacle reference {reference:?} uses production slot outside {sector_count} sectors"
    )]
    DeferredSightProductionSectorOutOfRange {
        reference: DeferredSightObstacleReference,
        sector_count: usize,
    },
    #[error(
        "campaign deferred sight-obstacle reference {reference:?} uses source index outside {source_count} entries"
    )]
    DeferredSightSourceOutOfRange {
        reference: DeferredSightObstacleReference,
        source_count: usize,
    },
    #[error(
        "campaign deferred production-topology reference {reference:?} requires a missing restart snapshot"
    )]
    DeferredTopologySnapshotMissing {
        reference: DeferredProductionPointTopologyReference,
    },
    #[error(
        "campaign deferred production-topology reference {reference:?} uses production slot outside {sector_count} sectors"
    )]
    DeferredTopologyProductionSectorOutOfRange {
        reference: DeferredProductionPointTopologyReference,
        sector_count: usize,
    },
    #[error(
        "campaign deferred production-topology reference {reference:?} uses point index outside {point_count} points"
    )]
    DeferredTopologyPointOutOfRange {
        reference: DeferredProductionPointTopologyReference,
        point_count: usize,
    },
}

/// Validate every deferred identity against concrete approved mounted content
/// and mint the opaque process-local admission token.
const MAX_APPROVED_CONTAINMENT_WORK: usize = 16_000_000;

pub fn validate_replay_campaign_approved_content<R>(
    validated: ValidatedReplayCampaign,
    resolver: &R,
) -> Result<ApprovedReplayCampaignContent, ReplayCampaignContentValidationError>
where
    R: ReplayCampaignApprovedContentResolver + ?Sized,
{
    validate_replay_campaign_approved_content_with_work_limit(
        validated,
        resolver,
        MAX_APPROVED_CONTAINMENT_WORK,
    )
}

pub(super) fn validate_replay_campaign_approved_content_with_work_limit<R>(
    validated: ValidatedReplayCampaign,
    resolver: &R,
    max_containment_work: usize,
) -> Result<ApprovedReplayCampaignContent, ReplayCampaignContentValidationError>
where
    R: ReplayCampaignApprovedContentResolver + ?Sized,
{
    let approved_identity = resolver.approved_content_identity();
    validate_approved_content_identity(approved_identity)?;
    if validated.deferred_content_checks.is_empty() {
        return Ok(ApprovedReplayCampaignContent {
            validated,
            approved_identity,
            _seal: ApprovedReplayCampaignContentSeal,
        });
    }
    let metadata = resolver
        .sherwood_campaign_metadata()
        .ok_or(ReplayCampaignContentValidationError::MissingSherwoodMetadata)?;

    const MAX_APPROVED_RECORDS: usize = 1_000_000;
    const MAX_APPROVED_POLYGON_POINTS: usize = 4_000_000;
    check_approved_metadata_limit(
        "static_sight_obstacles",
        metadata.static_sight_obstacles.len(),
        MAX_APPROVED_RECORDS,
    )?;
    check_approved_metadata_limit(
        "script_zones",
        metadata.script_zones.len(),
        MAX_APPROVED_RECORDS,
    )?;
    check_approved_metadata_limit("beam_mes", metadata.beam_mes.len(), MAX_APPROVED_RECORDS)?;
    check_approved_metadata_limit(
        "production_sectors",
        metadata.production_sectors.len(),
        MAX_APPROVED_RECORDS,
    )?;

    let total_polygon_points = metadata
        .static_sight_obstacles
        .iter()
        .map(|obstacle| obstacle.projected_polygon.len())
        .chain(
            metadata
                .production_sectors
                .iter()
                .map(|sector| sector.polygon.len()),
        )
        .try_fold(0usize, |total, points| {
            total.checked_add(points).ok_or_else(|| {
                ReplayCampaignContentValidationError::ApprovedMetadataCountOverflow {
                    collection: "polygon_points".to_owned(),
                }
            })
        })?;
    check_approved_metadata_limit(
        "polygon_points",
        total_polygon_points,
        MAX_APPROVED_POLYGON_POINTS,
    )?;

    for (index, obstacle) in metadata.static_sight_obstacles.iter().enumerate() {
        if obstacle.projection_topology.is_some() {
            validate_approved_polygon(
                "static_sight_obstacles",
                index,
                &obstacle.projected_polygon,
            )?;
        }
    }
    let mut sectors_by_topology = BTreeMap::new();
    for (index, sector) in metadata.production_sectors.iter().enumerate() {
        validate_approved_polygon("production_sectors", index, &sector.polygon)?;
        if sectors_by_topology
            .insert(sector.topology, &sector.polygon)
            .is_some()
        {
            return Err(
                ReplayCampaignContentValidationError::DuplicateApprovedProductionTopology {
                    topology: sector.topology,
                },
            );
        }
    }
    let mut production_zones = BTreeMap::new();
    for (zone_index, zone) in metadata.script_zones.iter().enumerate() {
        if let Some(production_slot) = zone.production_slot {
            if production_slot >= CANONICAL_PRODUCTION_TYPES.len() {
                return Err(
                    ReplayCampaignContentValidationError::ApprovedProductionScriptSlotOutOfRange {
                        zone_index,
                        production_slot,
                    },
                );
            }
            if let Some(first_zone) = production_zones.insert(production_slot, zone_index) {
                return Err(
                    ReplayCampaignContentValidationError::DuplicateApprovedProductionScriptSlot {
                        production_slot,
                        first_zone,
                        duplicate_zone: zone_index,
                    },
                );
            }
        }
    }

    let mut containment_work = 0usize;
    for reference in &validated.deferred_content_checks.sight_obstacle_references {
        let Some(obstacle) = metadata
            .static_sight_obstacles
            .get(reference.obstacle_index as usize)
        else {
            return Err(
                ReplayCampaignContentValidationError::StaticSightObstacleOutOfRange {
                    reference: reference.clone(),
                    approved_count: metadata.static_sight_obstacles.len(),
                },
            );
        };
        if reference.source == ProductionObstacleSource::Point {
            let point = replay_production_point(&validated.campaign, reference)?;
            let expected = ReplayCampaignProductionPointTopology {
                map_layer: point.layer,
                sector: point.sector,
            };
            if obstacle.projection_topology != Some(expected) {
                return Err(
                    ReplayCampaignContentValidationError::ProductionPointObstacleTopologyMismatch {
                        reference: reference.clone(),
                        expected,
                        actual: obstacle.projection_topology,
                    },
                );
            }
            charge_approved_containment_work(
                &mut containment_work,
                obstacle.projected_polygon.len(),
                max_containment_work,
            )?;
            if !approved_polygon_contains(&obstacle.projected_polygon, point.x, point.y) {
                return Err(
                    ReplayCampaignContentValidationError::ProductionPointOutsideObstacle {
                        reference: reference.clone(),
                    },
                );
            }
        } else {
            if obstacle.projection_topology.is_none() {
                return Err(
                    ReplayCampaignContentValidationError::ProductionOccupantObstacleHasNoProjection {
                        reference: reference.clone(),
                    },
                );
            }
            let occupant = replay_production_occupant(&validated.campaign, reference)?;
            charge_approved_containment_work(
                &mut containment_work,
                obstacle.projected_polygon.len(),
                max_containment_work,
            )?;
            if !approved_polygon_contains(&obstacle.projected_polygon, occupant.x, occupant.y) {
                return Err(
                    ReplayCampaignContentValidationError::ProductionOccupantOutsideObstacle {
                        reference: reference.clone(),
                    },
                );
            }
        }
    }
    for reference in &validated
        .deferred_content_checks
        .production_script_zone_references
    {
        let Some(zone) = metadata.script_zones.get(reference.script_zone_index) else {
            return Err(ReplayCampaignContentValidationError::ScriptZoneOutOfRange {
                reference: reference.clone(),
                approved_count: metadata.script_zones.len(),
            });
        };
        if zone.production_slot != Some(reference.production_slot) {
            return Err(
                ReplayCampaignContentValidationError::ScriptZoneProductionMismatch {
                    reference: reference.clone(),
                    actual_slot: zone.production_slot,
                },
            );
        }
    }
    for reference in &validated
        .deferred_content_checks
        .production_point_topology_references
    {
        let topology = ReplayCampaignProductionPointTopology {
            map_layer: reference.map_layer,
            sector: reference.sector,
        };
        let Some(polygon) = sectors_by_topology.get(&topology) else {
            return Err(
                ReplayCampaignContentValidationError::ProductionPointTopologyMissing {
                    reference: reference.clone(),
                },
            );
        };
        let point = replay_production_point_from_topology(&validated.campaign, reference)?;
        charge_approved_containment_work(
            &mut containment_work,
            polygon.len(),
            max_containment_work,
        )?;
        if !approved_polygon_contains(polygon, point.x, point.y) {
            return Err(
                ReplayCampaignContentValidationError::ProductionPointOutsideSector {
                    reference: reference.clone(),
                },
            );
        }
    }
    for reference in &validated
        .deferred_content_checks
        .sherwood_beam_me_references
    {
        if reference.beam_me_index as usize >= metadata.beam_mes.len() {
            return Err(ReplayCampaignContentValidationError::BeamMeOutOfRange {
                reference: reference.clone(),
                approved_count: metadata.beam_mes.len(),
            });
        }
    }

    Ok(ApprovedReplayCampaignContent {
        validated,
        approved_identity,
        _seal: ApprovedReplayCampaignContentSeal,
    })
}

fn validate_approved_content_identity(
    identity: ReplayCampaignApprovedContentIdentity,
) -> Result<(), ReplayCampaignContentValidationError> {
    if identity.build_manifest_sha256 == [0; 32] {
        return Err(
            ReplayCampaignContentValidationError::InvalidApprovedContentIdentity {
                field: "build_manifest_sha256".to_owned(),
            },
        );
    }
    if identity.content_manifest_sha256 == [0; 32] {
        return Err(
            ReplayCampaignContentValidationError::InvalidApprovedContentIdentity {
                field: "content_manifest_sha256".to_owned(),
            },
        );
    }
    Ok(())
}

fn charge_approved_containment_work(
    completed: &mut usize,
    requested: usize,
    limit: usize,
) -> Result<(), ReplayCampaignContentValidationError> {
    let attempted = completed.checked_add(requested).ok_or(
        ReplayCampaignContentValidationError::ApprovedContainmentWorkLimit {
            attempted: usize::MAX,
            limit,
        },
    )?;
    if attempted > limit {
        return Err(
            ReplayCampaignContentValidationError::ApprovedContainmentWorkLimit { attempted, limit },
        );
    }
    *completed = attempted;
    Ok(())
}

fn check_approved_metadata_limit(
    collection: &str,
    observed: usize,
    limit: usize,
) -> Result<(), ReplayCampaignContentValidationError> {
    if observed > limit {
        return Err(
            ReplayCampaignContentValidationError::ApprovedMetadataLimit {
                collection: collection.to_owned(),
                observed,
                limit,
            },
        );
    }
    Ok(())
}

fn validate_approved_polygon(
    collection: &str,
    index: usize,
    polygon: &[ReplayCampaignApprovedMapPoint],
) -> Result<(), ReplayCampaignContentValidationError> {
    if polygon.len() < 3 {
        return Err(
            ReplayCampaignContentValidationError::ApprovedPolygonInvalid {
                collection: collection.to_owned(),
                index,
                reason: "polygon has fewer than three vertices".to_owned(),
            },
        );
    }
    if polygon
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return Err(
            ReplayCampaignContentValidationError::ApprovedPolygonInvalid {
                collection: collection.to_owned(),
                index,
                reason: "polygon has a non-finite coordinate".to_owned(),
            },
        );
    }
    Ok(())
}

fn replay_production_point<'a>(
    campaign: &'a Campaign,
    reference: &DeferredSightObstacleReference,
) -> Result<&'a robin_engine::sector_production::Point, ReplayCampaignContentValidationError> {
    let sectors = match reference.layer {
        CampaignLayer::Current => &campaign.production_sectors,
        CampaignLayer::PreMissionSnapshot => {
            &campaign
                .pre_mission_snapshot
                .as_ref()
                .ok_or_else(|| {
                    ReplayCampaignContentValidationError::DeferredSightSnapshotMissing {
                        reference: reference.clone(),
                    }
                })?
                .production_sectors
        }
        CampaignLayer::PracticeReturnSnapshot => {
            campaign
                .practice_return_snapshot
                .as_ref()
                .ok_or_else(|| {
                    ReplayCampaignContentValidationError::DeferredSightSnapshotMissing {
                        reference: reference.clone(),
                    }
                })?
                .validation_view()
                .production_sectors
        }
    };
    let sector = sectors.get(reference.production_slot).ok_or_else(|| {
        ReplayCampaignContentValidationError::DeferredSightProductionSectorOutOfRange {
            reference: reference.clone(),
            sector_count: sectors.len(),
        }
    })?;
    sector
        .production_points
        .get(reference.source_index)
        .ok_or_else(
            || ReplayCampaignContentValidationError::DeferredSightSourceOutOfRange {
                reference: reference.clone(),
                source_count: sector.production_points.len(),
            },
        )
}

fn replay_production_point_from_topology<'a>(
    campaign: &'a Campaign,
    reference: &DeferredProductionPointTopologyReference,
) -> Result<&'a robin_engine::sector_production::Point, ReplayCampaignContentValidationError> {
    let sectors = match reference.layer {
        CampaignLayer::Current => &campaign.production_sectors,
        CampaignLayer::PreMissionSnapshot => {
            &campaign
                .pre_mission_snapshot
                .as_ref()
                .ok_or_else(|| {
                    ReplayCampaignContentValidationError::DeferredTopologySnapshotMissing {
                        reference: reference.clone(),
                    }
                })?
                .production_sectors
        }
        CampaignLayer::PracticeReturnSnapshot => {
            campaign
                .practice_return_snapshot
                .as_ref()
                .ok_or_else(|| {
                    ReplayCampaignContentValidationError::DeferredTopologySnapshotMissing {
                        reference: reference.clone(),
                    }
                })?
                .validation_view()
                .production_sectors
        }
    };
    let sector = sectors.get(reference.production_slot).ok_or_else(|| {
        ReplayCampaignContentValidationError::DeferredTopologyProductionSectorOutOfRange {
            reference: reference.clone(),
            sector_count: sectors.len(),
        }
    })?;
    sector
        .production_points
        .get(reference.point_index)
        .ok_or_else(
            || ReplayCampaignContentValidationError::DeferredTopologyPointOutOfRange {
                reference: reference.clone(),
                point_count: sector.production_points.len(),
            },
        )
}

fn replay_production_occupant<'a>(
    campaign: &'a Campaign,
    reference: &DeferredSightObstacleReference,
) -> Result<&'a robin_engine::sector_production::Occupant, ReplayCampaignContentValidationError> {
    let sectors = match reference.layer {
        CampaignLayer::Current => &campaign.production_sectors,
        CampaignLayer::PreMissionSnapshot => {
            &campaign
                .pre_mission_snapshot
                .as_ref()
                .ok_or_else(|| {
                    ReplayCampaignContentValidationError::DeferredSightSnapshotMissing {
                        reference: reference.clone(),
                    }
                })?
                .production_sectors
        }
        CampaignLayer::PracticeReturnSnapshot => {
            campaign
                .practice_return_snapshot
                .as_ref()
                .ok_or_else(|| {
                    ReplayCampaignContentValidationError::DeferredSightSnapshotMissing {
                        reference: reference.clone(),
                    }
                })?
                .validation_view()
                .production_sectors
        }
    };
    let sector = sectors.get(reference.production_slot).ok_or_else(|| {
        ReplayCampaignContentValidationError::DeferredSightProductionSectorOutOfRange {
            reference: reference.clone(),
            sector_count: sectors.len(),
        }
    })?;
    sector.occupants.get(reference.source_index).ok_or_else(|| {
        ReplayCampaignContentValidationError::DeferredSightSourceOutOfRange {
            reference: reference.clone(),
            source_count: sector.occupants.len(),
        }
    })
}

fn approved_polygon_contains(polygon: &[ReplayCampaignApprovedMapPoint], x: f32, y: f32) -> bool {
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for &current in polygon {
        let cross = (x - previous.x) * (current.y - previous.y)
            - (y - previous.y) * (current.x - previous.x);
        let on_edge = cross.abs() <= f32::EPSILON
            && x >= previous.x.min(current.x)
            && x <= previous.x.max(current.x)
            && y >= previous.y.min(current.y)
            && y <= previous.y.max(current.y);
        if on_edge {
            return true;
        }
        if (current.y > y) != (previous.y > y) {
            let edge_x =
                (previous.x - current.x) * (y - current.y) / (previous.y - current.y) + current.x;
            if x < edge_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

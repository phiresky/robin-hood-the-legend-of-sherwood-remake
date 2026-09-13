use crate::bow_shot::{SpawnArrowParams, spawn_arrow};
use crate::coordinates::{MapPoint, WorldPoint3D, WorldVec3D};
use crate::element::{EntityId, ObjectType, TrajectoryPoint};
use crate::entity_id::{PcId, SoldierId};

#[test]
fn accessory_sprite_keeps_fresh_instance_order_sentinel() {
    let mut assets = super::LevelAssets::new();
    assets
        .accessory_sprite_prototypes
        .insert(ObjectType::Arrow, crate::sprite::Sprite::default());
    let arrow = spawn_arrow(SpawnArrowParams {
        shooter: EntityId::Pc(PcId(0)),
        bow_point: WorldPoint3D::new(0.0, 0.0, 10.0),
        trajectory_origin: MapPoint::ZERO,
        target: EntityId::Soldier(SoldierId(1)),
        target_pos: MapPoint::new(8.0, 0.0),
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D::new(8.0, 0.0, 10.0),
            time: 2,
        }],
        damage: 1,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D::new(4.0, 0.0, 0.0),
    });
    let mut engine = super::EngineInner::new();
    let arrow_id = engine.add_test_entity(arrow);

    engine.attach_accessory_sprite(&assets, arrow_id);

    assert_eq!(
        engine
            .get_entity(arrow_id)
            .unwrap()
            .sprite()
            .last_processed_order_id,
        u32::from(u16::MAX) + 1
    );
}

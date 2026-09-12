//! Shared host/input fixtures built through the explicit engine test API.
use super::Host;
use robin_engine::{
    campaign::Campaign,
    coordinates::MapPoint,
    element::{
        self as engine_element, ActorData, ActorPc, ElementData, ElementKind, EntityId, HumanData,
        PcData, Posture,
    },
    engine::{Engine, LevelAssets},
};

pub(crate) fn fixture() -> (Engine, LevelAssets, Host) {
    let mut assets = LevelAssets::new();
    let engine =
        Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets).expect("test engine");
    let host = Host::scratch(800.0, 600.0);
    (engine, assets, host)
}

pub(crate) fn add_pc_with_status(
    engine: &mut Engine,
    x: f32,
    y: f32,
    posture: Posture,
    active: bool,
    life_points: i16,
) -> EntityId {
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(posture);
        initial_element.kind = ElementKind::ActorPc;
        initial_element.active = active;
        initial_element
    };
    element.set_position_map(MapPoint::new(x, y));
    // Settle the position interface so the freshly-placed PC does
    // not read as in motion (position != old-position after a raw
    // position write).
    element.sprite.position_iface.settle_current_position();
    engine.test_add_entity(engine_element::Entity::Pc(ActorPc {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData {
            life_points,
            playable: true,
            ..Default::default()
        },
    }))
}

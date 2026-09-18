//! Test helpers shared by the combat, movement, AI and bow-shot suites.

use crate::element::{ElementData, ElementKind};

/// Default element data with only `kind` and `active` set.
pub(crate) fn test_element(kind: ElementKind, active: bool) -> ElementData {
    let mut element = ElementData::default();
    element.kind = kind;
    element.active = active;
    element
}

/// Mission holding an empty `StartUp` class plus `class_name`, whose only
/// function is a three-parameter `FilterAIEvent` made of `quads`.
pub(crate) fn filter_ai_event_mission(
    source_file: &str,
    class_name: &str,
    size_of_temporary: i32,
    quads: Vec<crate::vm::Quad>,
) -> crate::engine::types::MissionScript {
    use crate::scb::{ClassEntry, Function, ScbFile};
    crate::engine::types::MissionScript::from_scb(ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![
            super::asm::empty_startup_class(source_file.into()),
            ClassEntry {
                source_file: source_file.into(),
                class_name: class_name.into(),
                functions: vec![Function {
                    name: "FilterAIEvent".into(),
                    num_parameters: 3,
                    size_of_return_value: 4,
                    size_of_parameters: 12,
                    size_of_temporary,
                    ..Default::default()
                }],
                quads,
                ..Default::default()
            },
        ],
    })
    .expect("FilterAIEvent test mission must load")
}

/// Put `actor` inside a fresh door at `point` by starting a `PassDoor`
/// movement through it.
pub(crate) fn enter_test_door(
    engine: &mut crate::engine::EngineInner,
    actor: crate::entity_id::EntityId,
    point: crate::coordinates::MapPoint,
) {
    let sector = engine.sector_of(actor).unwrap();
    let door = crate::gate::DoorIndex::new(engine.script_domains.interactables.doors.len() as u32)
        .unwrap();
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            point_in: point,
            point_out: point,
            sector_in: crate::sector::SectorNumber::new(1),
            sector_out: crate::sector::SectorNumber::new(1),
            sector_in_index: sector.arena_index(),
            sector_out_index: sector.arena_index(),
            ..Default::default()
        });
    let mut element = crate::sequence::SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(actor),
        crate::order::OrderType::WalkingUpright,
    );
    let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut element.data
    else {
        unreachable!()
    };
    *gate_id = Some(door);
    *direction = 1;
    let sequence = engine.orders.sequence_manager.insert_element(element);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence);
    engine.select_sequence_element(actor, Some((sequence, 0)));
    engine.t_element_in_progress(&crate::engine::LevelAssets::new(), sequence, 0);
}

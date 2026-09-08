//! Execute independent engine owners against colliding logical resource names.
use super::*;
use crate::coordinates::{SpriteAnchor, SpriteFrameOffset, SpriteSize};
use crate::element::{ElementData, ElementFx, ElementKind, Entity, FxData};
use crate::interp::{HostFunctions, NativeCallOutcome, NativeStack, StopReason};
use crate::scb::{ClassEntry, Function, Quad, ScbFile};
use crate::script_manager::ScriptProgram;
use crate::sprite::Sprite;
use crate::sprite_script::{
    FrameKind, MissionResourceEnvironment, NONANIMATION_END, SpriteInfo, SpriteScript,
    SpriteScriptor,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};

fn program(value: i32) -> Arc<ScriptProgram> {
    let mut begin = [0; 8];
    begin[2..4].copy_from_slice(&1u16.to_le_bytes());
    let mut constant = [0; 8];
    constant[0..2].copy_from_slice(&0xC000u16.to_le_bytes());
    constant[4..8].copy_from_slice(&value.to_le_bytes());
    let mut returned = [0; 8];
    returned[0..2].copy_from_slice(&0xC000u16.to_le_bytes());
    Arc::new(
        ScriptProgram::from_scb(ScbFile {
            version: 1.5,
            classes: vec![ClassEntry {
                source_file: "same.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: vec![],
                functions: vec![Function {
                    name: "Value".into(),
                    address: 0,
                    num_parameters: 0,
                    size_of_return_value: 4,
                    size_of_parameters: 0,
                    size_of_volatile: 4,
                    size_of_temporary: 0,
                }],
                quads: vec![
                    Quad {
                        operation: 3,
                        operands: begin,
                    },
                    Quad {
                        operation: 19,
                        operands: constant,
                    },
                    Quad {
                        operation: 7,
                        operands: returned,
                    },
                ],
            }],
        })
        .unwrap(),
    )
}

fn resources(first_frame: u32, value: i32) -> Arc<MissionResourceEnvironment> {
    let info = SpriteInfo {
        scripts: Arc::new(vec![SpriteScript {
            action_id: 0,
            action_done: 2,
            frame_ids: vec![first_frame, first_frame + 1, first_frame + 2],
            delays: vec![0; 3],
            distances: vec![0; 3],
            offsets: vec![SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
            ..SpriteScript::default()
        }]),
        conversion: Arc::new(vec![0; NONANIMATION_END]),
        size: SpriteSize::new(12.0, 8.0),
        center: SpriteAnchor::ZERO,
    };
    let profiles = vec![("Same".into(), info)];
    Arc::new(
        MissionResourceEnvironment::default()
            .with_parsed_rhs([("Animations/Day/Same.rhs", 77, profiles.as_slice())])
            .unwrap()
            .with_programs(BTreeMap::from([("same".into(), program(value))])),
    )
}

struct NoNatives;
impl HostFunctions for NoNatives {
    fn call(&mut self, index: u32, _: &mut NativeStack) -> NativeCallOutcome {
        panic!("fixture unexpectedly called native {index}");
    }
}

#[test]
fn two_engines_execute_different_rhs_and_bytecode_concurrently() {
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [(10, 111), (100, 222)]
        .into_iter()
        .map(|(first_frame, value)| {
            let barrier = barrier.clone();
            let resources = resources(first_frame, value);
            std::thread::spawn(move || {
                let mut assets = LevelAssets::new();
                assets.sprite_scriptor =
                    Arc::new(SpriteScriptor::with_resources(resources.clone()));
                assets.scripts.mission_programs = Arc::new(resources.programs().clone());
                assets.bank_signature = 77;
                let mut engine = EngineInner::new();
                engine
                    .load_mission_script(&assets, std::path::Path::new("Data/Levels/same.scb"))
                    .unwrap();
                barrier.wait();
                let mut sprite = Sprite::default();
                sprite
                    .load_frame_info(
                        assets.sprite_scriptor_mut(),
                        FrameKind::Animation,
                        "Data/Animations",
                        "Same",
                        "Same",
                        77,
                        None,
                    )
                    .unwrap();
                sprite.current_row = 0;
                let owner = engine.add_entity(Entity::Fx(ElementFx {
                    element: {
                        let mut initial_element = ElementData::default();
                        initial_element.kind = ElementKind::Fx;
                        initial_element.active = true;
                        initial_element.sprite = sprite;
                        initial_element
                    },
                    fx: FxData::default(),
                }));
                let rollback_assets = assets.clone();
                drop(resources);
                let sim = crate::sim_rng::SimulationContext::with_seed(17);
                let mut seen = std::collections::BTreeSet::new();
                for _ in 0..20 {
                    barrier.wait();
                    engine.tick_static_entity_hourglass_for(&sim, &rollback_assets, owner);
                    let sprite = engine.get_entity(owner).unwrap().sprite();
                    let frame = sprite.bank_id_for(sprite.current_row, sprite.current_frame);
                    assert!((first_frame..first_frame + 3).contains(&frame));
                    seen.insert(frame);
                    let mission = engine.scripts.mission.as_mut().unwrap();
                    let mut activation = mission
                        .instance
                        .begin_activation(&mission.manager, "Value", &[])
                        .unwrap();
                    let stop = mission.instance.poll_activation_with_host(
                        &mut mission.manager,
                        &mut activation,
                        100,
                        "Value",
                        &mut NoNatives,
                    );
                    assert!(matches!(stop, StopReason::ReturnedValue(result) if result == value));
                }
                assert_eq!(
                    seen.len(),
                    3,
                    "real engine FX hourglass must advance frames"
                );
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn missing_prepared_program_is_an_error_not_a_cross_engine_fallback() {
    let mut first_assets = LevelAssets::new();
    first_assets.scripts.mission_programs = Arc::new(resources(10, 111).programs().clone());
    let mut first = EngineInner::new();
    first
        .load_mission_script(&first_assets, std::path::Path::new("same.scb"))
        .unwrap();
    let mut second = EngineInner::new();
    let error = second
        .load_mission_script(&LevelAssets::new(), std::path::Path::new("same.scb"))
        .unwrap_err();
    assert!(error.contains("required mission script"));
    assert!(second.scripts.mission.is_none());
    assert!(first.scripts.mission.is_some());
}

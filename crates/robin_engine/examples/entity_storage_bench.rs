//! Compare entity storage using real saved state, without changing runtime storage.
//!
//! Build: cargo build --release -p robin_engine --example entity_storage_bench
//! Run: target/release/examples/entity_storage_bench SAVE.json [SAVE.json ...]
//!
//! Uses the system allocator without allocation instrumentation. Loading and
//! conversions are outside timed regions. Times include nested entity data.
//! Uses thread CPU time to exclude time spent waiting for the scheduler.
//! This is a storage microbenchmark, not a simulation or replay benchmark.

use cpu_time::ThreadTime as Instant;
use robin_engine::element::*;
use serde::{Deserialize, Serialize};
use std::hint::black_box;

const SAMPLES: usize = 11;
const COPIES: usize = 512;
const RETAINED: usize = 16;
const SCANS: usize = 10_000;

#[derive(Clone, Serialize, Deserialize)]
enum BoxedEntity {
    Pc(Box<ActorPc>),
    Soldier(Box<ActorSoldier>),
    Civilian(Box<ActorCivilian>),
    Fx(Box<ElementFx>),
    Target(Box<ElementTarget>),
    Bonus(Box<ElementBonus>),
    Scroll(Box<ElementScroll>),
    Projectile(Box<ElementProjectile>),
    Net(Box<ElementNet>),
}

impl From<Entity> for BoxedEntity {
    fn from(entity: Entity) -> Self {
        match entity {
            Entity::Pc(value) => Self::Pc(Box::new(value)),
            Entity::Soldier(value) => Self::Soldier(Box::new(value)),
            Entity::Civilian(value) => Self::Civilian(Box::new(value)),
            Entity::Fx(value) => Self::Fx(Box::new(value)),
            Entity::Target(value) => Self::Target(Box::new(value)),
            Entity::Bonus(value) => Self::Bonus(Box::new(value)),
            Entity::Scroll(value) => Self::Scroll(Box::new(value)),
            Entity::Projectile(value) => Self::Projectile(Box::new(value)),
            Entity::Net(value) => Self::Net(Box::new(value)),
        }
    }
}

impl BoxedEntity {
    fn element(&self) -> &ElementData {
        match self {
            Self::Pc(value) => &value.element,
            Self::Soldier(value) => &value.element,
            Self::Civilian(value) => &value.element,
            Self::Fx(value) => &value.element,
            Self::Target(value) => &value.element,
            Self::Bonus(value) => &value.element,
            Self::Scroll(value) => &value.element,
            Self::Projectile(value) => &value.element,
            Self::Net(value) => &value.element,
        }
    }
}

/// Preserve global slot order while payloads live in type-specific vectors.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum PoolSlot {
    Pc(u32),
    Soldier(u32),
    Civilian(u32),
    Fx(u32),
    Target(u32),
    Bonus(u32),
    Scroll(u32),
    Projectile(u32),
    Net(u32),
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct PooledEntities {
    slots: Vec<Option<PoolSlot>>,
    pcs: Vec<ActorPc>,
    soldiers: Vec<ActorSoldier>,
    civilians: Vec<ActorCivilian>,
    effects: Vec<ElementFx>,
    targets: Vec<ElementTarget>,
    bonuses: Vec<ElementBonus>,
    scrolls: Vec<ElementScroll>,
    projectiles: Vec<ElementProjectile>,
    nets: Vec<ElementNet>,
}

impl PooledEntities {
    fn from_entities(entities: Vec<Option<Entity>>) -> Self {
        let mut pool = Self::default();
        for entity in entities {
            let slot = entity.map(|entity| match entity {
                Entity::Pc(value) => {
                    let index = u32::try_from(pool.pcs.len()).unwrap();
                    pool.pcs.push(value);
                    PoolSlot::Pc(index)
                }
                Entity::Soldier(value) => {
                    let index = u32::try_from(pool.soldiers.len()).unwrap();
                    pool.soldiers.push(value);
                    PoolSlot::Soldier(index)
                }
                Entity::Civilian(value) => {
                    let index = u32::try_from(pool.civilians.len()).unwrap();
                    pool.civilians.push(value);
                    PoolSlot::Civilian(index)
                }
                Entity::Fx(value) => {
                    let index = u32::try_from(pool.effects.len()).unwrap();
                    pool.effects.push(value);
                    PoolSlot::Fx(index)
                }
                Entity::Target(value) => {
                    let index = u32::try_from(pool.targets.len()).unwrap();
                    pool.targets.push(value);
                    PoolSlot::Target(index)
                }
                Entity::Bonus(value) => {
                    let index = u32::try_from(pool.bonuses.len()).unwrap();
                    pool.bonuses.push(value);
                    PoolSlot::Bonus(index)
                }
                Entity::Scroll(value) => {
                    let index = u32::try_from(pool.scrolls.len()).unwrap();
                    pool.scrolls.push(value);
                    PoolSlot::Scroll(index)
                }
                Entity::Projectile(value) => {
                    let index = u32::try_from(pool.projectiles.len()).unwrap();
                    pool.projectiles.push(value);
                    PoolSlot::Projectile(index)
                }
                Entity::Net(value) => {
                    let index = u32::try_from(pool.nets.len()).unwrap();
                    pool.nets.push(value);
                    PoolSlot::Net(index)
                }
            });
            pool.slots.push(slot);
        }
        pool
    }

    fn element(&self, slot: PoolSlot) -> &ElementData {
        match slot {
            PoolSlot::Pc(index) => &self.pcs[index as usize].element,
            PoolSlot::Soldier(index) => &self.soldiers[index as usize].element,
            PoolSlot::Civilian(index) => &self.civilians[index as usize].element,
            PoolSlot::Fx(index) => &self.effects[index as usize].element,
            PoolSlot::Target(index) => &self.targets[index as usize].element,
            PoolSlot::Bonus(index) => &self.bonuses[index as usize].element,
            PoolSlot::Scroll(index) => &self.scrolls[index as usize].element,
            PoolSlot::Projectile(index) => &self.projectiles[index as usize].element,
            PoolSlot::Net(index) => &self.nets[index as usize].element,
        }
    }

    fn to_entities(&self) -> Vec<Option<Entity>> {
        self.slots
            .iter()
            .map(|slot| {
                slot.map(|slot| match slot {
                    PoolSlot::Pc(index) => Entity::Pc(self.pcs[index as usize].clone()),
                    PoolSlot::Soldier(index) => {
                        Entity::Soldier(self.soldiers[index as usize].clone())
                    }
                    PoolSlot::Civilian(index) => {
                        Entity::Civilian(self.civilians[index as usize].clone())
                    }
                    PoolSlot::Fx(index) => Entity::Fx(self.effects[index as usize].clone()),
                    PoolSlot::Target(index) => Entity::Target(self.targets[index as usize].clone()),
                    PoolSlot::Bonus(index) => Entity::Bonus(self.bonuses[index as usize].clone()),
                    PoolSlot::Scroll(index) => Entity::Scroll(self.scrolls[index as usize].clone()),
                    PoolSlot::Projectile(index) => {
                        Entity::Projectile(self.projectiles[index as usize].clone())
                    }
                    PoolSlot::Net(index) => Entity::Net(self.nets[index as usize].clone()),
                })
            })
            .collect()
    }
}

trait Scan {
    fn scan(&self) -> f64;
}

fn position_sum(element: &ElementData) -> f64 {
    let position = element.position_map();
    f64::from(position.x) + f64::from(position.y) + f64::from(u8::from(element.active))
}

impl Scan for Vec<Option<Entity>> {
    fn scan(&self) -> f64 {
        self.iter()
            .flatten()
            .map(|entity| position_sum(entity.element_data()))
            .sum()
    }
}

impl Scan for Vec<Option<BoxedEntity>> {
    fn scan(&self) -> f64 {
        self.iter()
            .flatten()
            .map(|entity| position_sum(entity.element()))
            .sum()
    }
}

impl Scan for PooledEntities {
    fn scan(&self) -> f64 {
        self.slots
            .iter()
            .flatten()
            .map(|slot| position_sum(self.element(*slot)))
            .sum()
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Samples {
    clone_drop_us: Vec<f64>,
    retained_clone_us: Vec<f64>,
    retained_drop_us: Vec<f64>,
    scan_us: Vec<f64>,
}

fn sample<T: Clone + Scan>(value: &T, samples: &mut Samples) {
    let start = Instant::now();
    for _ in 0..COPIES {
        drop(black_box(black_box(value).clone()));
    }
    samples
        .clone_drop_us
        .push(start.elapsed().as_secs_f64() * 1e6 / COPIES as f64);

    let mut retained = Vec::with_capacity(RETAINED);
    let mut clone_seconds = 0.0;
    let mut drop_seconds = 0.0;
    for _ in 0..COPIES / RETAINED {
        let start = Instant::now();
        for _ in 0..RETAINED {
            retained.push(black_box(value).clone());
        }
        clone_seconds += start.elapsed().as_secs_f64();
        black_box(&retained);
        let start = Instant::now();
        retained.clear();
        drop_seconds += start.elapsed().as_secs_f64();
    }
    samples
        .retained_clone_us
        .push(clone_seconds * 1e6 / COPIES as f64);
    samples
        .retained_drop_us
        .push(drop_seconds * 1e6 / COPIES as f64);

    let start = Instant::now();
    for _ in 0..SCANS {
        black_box(black_box(value).scan());
    }
    samples
        .scan_us
        .push(start.elapsed().as_secs_f64() * 1e6 / SCANS as f64);
}

fn summarize(values: &[f64]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    serde_json::json!({
        "median": sorted[sorted.len()/2],
        "min": sorted[0],
        "max": sorted[sorted.len()-1],
    })
}

impl Samples {
    fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "clone_drop_us": summarize(&self.clone_drop_us),
            "retained_clone_us": summarize(&self.retained_clone_us),
            "retained_drop_us": summarize(&self.retained_drop_us),
            "scan_us": summarize(&self.scan_us),
            "raw": self,
        })
    }
}

fn main() {
    assert!(
        !cfg!(debug_assertions),
        "build and run this benchmark in release mode"
    );
    let paths = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(
        !paths.is_empty(),
        "provide at least one saved game JSON path"
    );
    for path in paths {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read save")).expect("parse save");
        let entities: Vec<Option<Entity>> =
            serde_json::from_value(value["engine"]["world"]["entities"].take())
                .expect("decode entities");
        drop(value);
        let boxed = entities
            .iter()
            .cloned()
            .map(|entity| entity.map(Into::into))
            .collect::<Vec<Option<BoxedEntity>>>();
        let pooled = PooledEntities::from_entities(entities.clone());
        // Compare all stored values and slot order before measuring.
        let expected = serde_json::to_value(&entities).unwrap();
        assert_eq!(serde_json::to_value(&boxed).unwrap(), expected);
        assert_eq!(
            serde_json::to_value(pooled.to_entities()).unwrap(),
            expected
        );
        assert_eq!(entities.scan(), boxed.scan());
        assert_eq!(entities.scan(), pooled.scan());
        drop(expected);

        let mut inline_samples = Samples::default();
        let mut boxed_samples = Samples::default();
        let mut pooled_samples = Samples::default();
        // One complete untabulated warmup for every representation.
        sample(&entities, &mut Samples::default());
        sample(&boxed, &mut Samples::default());
        sample(&pooled, &mut Samples::default());
        for round in 0..SAMPLES {
            // Rotate order to reduce systematic temperature/frequency bias.
            for offset in 0..3 {
                match (round + offset) % 3 {
                    0 => sample(&entities, &mut inline_samples),
                    1 => sample(&boxed, &mut boxed_samples),
                    _ => sample(&pooled, &mut pooled_samples),
                }
            }
        }
        println!(
            "{}",
            serde_json::json!({
                "save": path,
                "occupied": entities.iter().flatten().count(),
                "slots": entities.len(),
            "samples": SAMPLES,
            "clock": "thread_cpu_time",
                "copies_per_sample": COPIES,
                "retained_batch": RETAINED,
                "scans_per_sample": SCANS,
                "slot_bytes": {
                    "inline": std::mem::size_of::<Option<Entity>>(),
                    "boxed": std::mem::size_of::<Option<BoxedEntity>>(),
                    "pooled": std::mem::size_of::<Option<PoolSlot>>(),
                },
                "inline": inline_samples.summary(),
                "boxed": boxed_samples.summary(),
                "pooled": pooled_samples.summary(),
            })
        );
    }
}

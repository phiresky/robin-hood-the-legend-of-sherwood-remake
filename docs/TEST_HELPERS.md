# Test helper cheat-sheet

Helpers that replace test ceremony. Each entry gives a grep-able "before"
pattern (whitespace-insensitive: rustfmt usually splits the chains over
several lines, so search with `rg -U` or strip whitespace first).

Locations:

- `crates/robin_engine/src/engine/test_support/shortcuts.rs` - entity queries and entry-point adapters (`impl EngineInner`, crate-private, `#[cfg(test)]`)
- `crates/robin_engine/src/engine/test_support/pair_fixture.rs` - `PairFixture`
- `crates/robin_engine/src/engine/test_support.rs`, `test_support/actors.rs`, `test_support/asm.rs` - pre-existing: `TestActor`, `make_test_*`, `square_sector`, `ensure_ordinary_sector`, `empty_mission_script`, `apply_command`, `test_runtime_assets`
- `crates/robin_engine/src/test_support.rs` - public `robin_engine::test_support` for other crates (feature `test-support`)

All query helpers panic with the entity id and missing component; none
returns a fabricated value. Every `id` parameter is `impl Into<EntityId>`.

## 1. Entity queries (`engine.<helper>(id)`)

| Helper | Replaces (before) |
|---|---|
| `engine.ent(id)` / `ent_mut(id)` | `engine.get_entity(id).unwrap()` / `engine.get_entity_mut(id).unwrap()` |
| `engine.elem(id)` / `elem_mut(id)` | `.get_entity(id).unwrap().element_data()` / `.get_entity_mut(id).unwrap().element_data_mut()` |
| `engine.actor(id)` / `actor_mut(id)` | `.get_entity(id).unwrap().actor_data().unwrap()` / `..._mut` |
| `engine.human(id)` / `human_mut(id)` | `.get_entity(id).unwrap().human_data().unwrap()` / `..._mut` |
| `engine.pc(id)` / `pc_mut(id)` | `.get_entity(id).unwrap().pc_data().unwrap()` / `..._mut` |
| `engine.npc(id)` / `npc_mut(id)` | `.get_entity(id).unwrap().npc_data().unwrap()` / `..._mut` |
| `engine.enemy(id)` / `enemy_mut(id)` | `.get_entity(id).unwrap().enemy_ai().unwrap()` / `.enemy_ai_mut().unwrap()` |
| `engine.ai_ctrl(id)` / `ai_ctrl_mut(id)` | `.get_entity(id).unwrap().ai_controller().unwrap()` / `..._mut` |
| `engine.motion_state_of(id)` | `.actor_data().unwrap().continuation.motion_state` |
| `engine.action_state_of(id)` | `.actor_data().unwrap().action_state` (read) |
| `engine.set_action_state_of(id, s)` | `.actor_data_mut().unwrap().action_state = s` |
| `engine.pos_of(id)` | `.element_data().position()` |
| `engine.map_pos_of(id)` | `.element_data().position_map()` |
| `engine.direction_of(id)` | `.element_data().direction()` |
| `engine.posture_of(id)` | `.get_entity(id).unwrap().posture()` / `.element_data().posture()` |
| `engine.sector_of(id)` | `.element_data().sector()` |
| `engine.place(id, p)` | `.element_data_mut().set_position(p)` |
| `engine.place_map(id, p)` | `.element_data_mut().set_position_map(p)` |
| `engine.face(id, d)` | `.element_data_mut().set_direction_instantly(d)` |
| `engine.set_active(id, b)` | `.element_data_mut().active = b` |

Anything deeper composes from the component accessors, e.g.
`engine.human_mut(id).opponents.push(x)`, `engine.pc(id).current_action`,
`engine.npc(id).life_points`, `engine.elem(id).sprite.current_frame`,
`engine.enemy_mut(id).hth_weapon_id`.

Before:

```rust
assert_eq!(
    engine
        .get_entity(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .continuation
        .motion_state,
    MotionState::Stopped
);
```

After:

```rust
assert_eq!(engine.motion_state_of(owner), MotionState::Stopped);
```

The same chains written with `.expect("...")` instead of `.unwrap()` are
covered too. Do not convert chains whose receiver is not an `EngineInner`
(e.g. the `Engine` facade or a view).

## 2. Entry-point adapters (`engine.t_<name>(&assets, ...)`)

Each fixes `sim` to `crate::sim_rng::test_context()` and passes a throwaway
`&mut Vec::new()` active-script list. Only convert call sites that pass
exactly those boilerplate values - keep the wide call when the test holds a
real `sim` or inspects `active_scripts`.

| Helper | Replaces (before) |
|---|---|
| `t_element_in_progress(&assets, seq, idx)` | `.element_in_progress(&crate::sim_rng::test_context(), &assets, &mut Vec::new(), seq, idx)` |
| `t_launch_element(&assets, elem)` | `.launch_element(&crate::sim_rng::test_context(), &assets, elem)` |
| `t_launch_sequence(&assets, seq)` | `.launch_sequence(&crate::sim_rng::test_context(), &assets, seq)` |
| `t_element_terminated(&assets, seq, idx)` | `.element_terminated(&crate::sim_rng::test_context(), &assets, &mut Vec::new(), seq, idx)` |
| `t_element_interrupted(&assets, seq, idx, flags)` | `.element_interrupted(&crate::sim_rng::test_context(), &assets, &mut Vec::new(), seq, idx, flags)` |
| `t_postpone_element(&assets, seq, idx)` | `.postpone_element(&crate::sim_rng::test_context(), &assets, &mut Vec::new(), seq, idx)` |
| `t_instruct_owner(&assets, owner, seq, idx)` | `.instruct_owner(&crate::sim_rng::test_context(), &assets, &mut Vec::new(), owner, seq, idx)` |
| `t_tick_actor_owner_envelopes(&assets)` | `.tick_actor_owner_envelopes(&crate::sim_rng::test_context(), &assets)` |
| `t_hourglass_phase_sequences(&assets)` | `.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut HostDisplayState::default(), &assets)` |
| `t_launch_in_progress(&assets, elem)` | `let s = engine.launch_element(...); engine.element_in_progress(..., s, 0);` pair |

Many files bind `let sim = crate::sim_rng::test_context();` first and pass
`&sim`; those are the same pattern (drop the binding if it becomes unused).

Before:

```rust
let seq = engine.launch_element(&crate::sim_rng::test_context(), &assets, wait);
engine.element_in_progress(
    &crate::sim_rng::test_context(),
    &assets,
    &mut Vec::new(),
    seq,
    0,
);
```

After:

```rust
let seq = engine.t_launch_in_progress(&assets, wait);
```

Pre-existing, same family: `engine.apply_command(&sim, &mut display, &mut input, &assets, &cmd)`
and `engine.test_runtime_assets()` (replaces `let mut assets = LevelAssets::new(); complete_test_runtime_fixture(&mut engine, &mut assets);`).

## 3. `PairFixture` - the two-actor scenario

Replaces hand-rolled `fn fixture() -> (EngineInner, LevelAssets, EntityId, EntityId)`
bodies made of `size_map` -> `allocate_layers(1)` -> `add_sector(square_sector(1, 0, ..))`
-> `add_test_entity` x2 -> `set_position` -> `set_sector_topology` ->
`complete_test_runtime_fixture` -> `frame_counter = 100`.

```rust
use crate::engine::test_support::pair_fixture::PairFixture;

fn fixture() -> (EngineInner, LevelAssets, EntityId, EntityId) {
    PairFixture::new(make_test_ai_soldier(Camp::Lacklandists), make_test_pc(Posture::Upright))
        .grid(256, 256)                     // default 128x128
        .extent(4000.0)                     // default 2000.0
        .on_row(100.0, 200.0, 100.0)        // default; or .positions(a, b)
        .action_state(ActionState::Waiting) // both actors; optional
        .move_box(MoveBox::from_coords(-4.0, -4.0, 4.0, 4.0)) // optional
        .direction(4)                       // optional
        .active(true)                       // optional
        .mission_script("observation.scs")  // empty_mission_script; optional
        .frame_counter(100)                 // default 100
        .think_first()                      // enter_ai_think_frame(first); optional
        .build()
        .into_tuple()
}
```

`build()` returns `PairScene { engine, assets, first, second, sector, sector_index }`
when the test also needs the sector handle. Per-actor setup that differs
between the two actors (life points, AI fields, profiles) stays in the
calling fixture: configure the `Entity` before passing it in, or mutate
through the section 1 helpers after `build()`.
Example conversion: `crates/robin_engine/src/engine/ai/enemy_observation/tests.rs`.

## 4. `Default` on exhaustively-spelled production types

`..Default::default()` is now available on `scb::Function`,
`scb::ClassEntry`, `scb::ScbFile`, `fast_find_grid::GridSector`,
`sector::SectorType` (`SpriteScript` already had it).

Before (grep `underlying_sector: None,` / `size_of_temporary: 0,`):

```rust
GridSector {
    points, bounding_box, sector_type, layer, sector_number,
    door_index: None, lift_type: None, lift_direction: 0, force_crouched: false,
    building_index: None, low_exit_point: None, high_exit_point: None,
    lowest_door_index: None, jump_line_indices: Vec::new(), gate_indices: Vec::new(),
    underlying_sector: None,
}
```

After: keep only the meaningful fields, then `..Default::default()`. For a
plain walkable square prefer `square_sector(number, layer, min, max)`.

`SpawnArrowParams` has no `Default` (its entity ids have no harmless zero
value). Tests use `SpawnArrowParams::test_flat(shooter, target, bow_point, target_pos)`
(30 damage, layer 0, one 10-frame segment at bow height, velocity +X) with
struct-update for the fields the test cares about:

```rust
spawn_arrow(SpawnArrowParams { damage: 5, ..SpawnArrowParams::test_flat(shooter, target, bow, target_pos) })
```

## 5. Table-driven tests (`rstest`)

`rstest` is a workspace dev-dependency (robin_engine, robin_rs, robin_lua).
Collapse families of tests that differ only in input/expected value:

```rust
#[rstest::rstest]
#[case::hero_dead(ElementDotInfo { is_human: true, is_pc: true, is_dead: true, ..default_info() }, Some(DotType::DeadHero))]
#[case::civilian(ElementDotInfo { is_human: true, is_civilian: true, ..default_info() }, Some(DotType::Civilian))]
fn classify(#[case] info: ElementDotInfo, #[case] expected: Option<DotType>) {
    assert_eq!(classify_element_dot(&info), expected);
}
```

Name every case (`#[case::name(..)]`) so failures and `cargo test <filter>`
stay as specific as the old one-fn-per-case names (`classify::case_02_hero_dead`).
Example: `crates/robin_engine/src/minimap.rs` (`classify`).

## 6. Cross-crate engine fixtures

Add to the consuming crate's `[dev-dependencies]` only:

```toml
robin_engine = { workspace = true, features = ["test-support"] }
```

| Helper (`robin_engine::test_support::`) | Replaces (before) |
|---|---|
| `fresh_engine()` | local `fn fresh_engine()` / `fn fixture_engine` wrapping `Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets)` |
| `fresh_engine_sized(w, h)` | same with another screen size |
| `seeded_engine(seed)` | `Engine::new_for_test_with_simulation(800.0, 600.0, Campaign::default(), assets, seed, SimConfig { script_enabled: false, ..})` |

All return `(Engine, LevelAssets)`. Examples:
`crates/robin_rs/src/gamepad/tests.rs`, `crates/robin_rs/src/savegame/tests.rs`.
The feature must never appear in a `[dependencies]` entry; resolver 2 keeps
dev-dependency features out of `cargo build -p robin_rs --bin robin`.

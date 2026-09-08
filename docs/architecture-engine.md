# Engine mutation authority

## Implemented

`ElementData::posture` is private. Reads use `posture()`, including external
client and parity consumers. Since a private field makes Rust struct-update
syntax unavailable outside the defining module, the existing default-based
element initializers now construct a default/initial-posture element and assign
the other public fields explicitly. No field was reordered, removed, or added
to the simulation, save projection, or existing wire/hash derives.

The mutation APIs distinguish three existing behaviors rather than silently
combining them:

- `set_posture`: requested live transition, retaining the corpse guard and
  synchronizing the embedded position interface.
- Crate-private `publish_order_posture`: unconditional logical-only publication
  by an executing animation/order/damage barrier. Existing frozen runner, bow,
  eager door and lethal damage writes intentionally retain this behavior; adding
  the corpse guard or synchronizing the sprite here would change simulation
  state. This is not exposed to client crates.
- `from_initial_posture` and crate-private `restore_v48_position_and_posture`:
  explicit construction and legacy adoption. Native save projection restoration
  continues to restore logical and sprite posture independently. Existing tied
  human release remains an invariant-checked entity operation inside the owner.

The terminal movement-order advancement operation is split into a sequence-only
capability module (`engine/movement/order_advancement.rs`) and the root
coordinator. Capture checks the current selection, preparation invalidates the
outgoing retained goals and snapshots the live following chain, then the
coordinator synchronously calls `do_next_order`. Classification re-reads the
captured element after callbacks. The capability module receives only
`SequenceManager`, so it cannot inspect or mutate actors, scripts, feedback,
spatial state, or RNG. Diagnostic timing and following-chain traversal order are
unchanged.

## Regression coverage

New tests cover initialization/order publication wire bytes and state hashes,
independent logical/sprite posture save round trips, explicit v48 restoration,
and movement preparation versus synchronous advancement/classification. Existing
corpse guard, posture transition, callback ownership, lazy door allocation,
movement and snapshot tests remain the broad acceptance suite.

Final combined validation at `43ab1f43280ac11ea305a72886109ce2e8e8eda6`:
`CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test --locked -p robin_engine -p robin_lua`
passed. Engine: 4,461 unit tests, 15 integration tests, and 22 doctests passed;
four unit tests and one doctest retained their existing ignores. The new
`ElementData::posture` private-field compile-fail doctest passed explicitly.
Lua: 12 unit tests and 23 integration tests passed. Client validation belongs
to the integrated client acceptance lanes, not this engine-only result.

TODO: continue narrowing broader engine domain privacy incrementally; this
change does not claim every `EngineInner` subsystem has restricted mutation
authority.

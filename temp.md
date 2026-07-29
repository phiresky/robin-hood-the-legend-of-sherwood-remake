# Agent prompt: audit replay recording completeness

Audit this repository for authoritative game state or nondeterministic inputs that
the current schema-9 parity recording does not capture, but that may be required
to replay an arbitrary mission-start session deterministically in the Rust
rewrite.

Repository context:

- The original C/C++ implementation is in `./original-code`; always use it as
  the behavioral authority.
- The Rust rewrite is primarily in `crates/robin_engine` and
  `crates/robin_rs`.
- Current recordings are JSONL files named `*.rhrec.jsonl`.
- They already include mission/campaign start state, resolved player commands,
  a global ordered RNG-draw stream, and broad per-frame parity state.
- Mission-start replay is the required scope. Loading a recording from the
  middle of a mission is not required.
- IDs need not be numerically identical if entities can be mapped
  isomorphically.
- Synchronous pathfinding is intentional.
- Compatibility with recordings older than schema 9 is not required.

Do a read-only audit. Do not edit code, regenerate recordings, or redesign the
replay system. Find concrete potential omissions by tracing the original
recording writer, Rust deserializer/replay runner, and both engines' sources of
state and nondeterminism.

In particular, inspect:

1. Every source of nondeterminism besides the recorded simulation RNG stream:
   other PRNG instances, random library calls, clocks/timers, frame duration,
   thread completion/order, async pathfinding, filesystem iteration, hash-map
   iteration where order affects behavior, pointer/address ordering, uninitialized
   data, floating-point environment, locale, and platform APIs.
2. Mission/campaign initialization state consumed before or during the first
   replayed frame: difficulty, campaign variables, mission/script globals,
   objectives, inventory, roster/character availability, upgrades, statistics,
   save-derived flags, and data-directory/build identity.
3. Resolved gameplay inputs or external events not represented by the current
   command log: selection/group state, pause/speed changes, UI-confirmed targets,
   camera-dependent commands, cheats/debug actions, console/script injection,
   window focus, and network input.
4. Stateful subsystems whose future behavior cannot be reconstructed from the
   recorded mission start plus commands and RNG draws, especially script VM
   state, sequence/order queues, AI scheduling, pathfinder caches, visibility
   state, animation timing, audio callbacks, and deferred event queues.
5. Existing parity dump fields that are diagnostic only versus fields actually
   needed as replay input. Do not recommend recording derived per-frame engine
   state merely to mask an implementation divergence.

For every candidate, report:

- exact C/C++ and Rust source locations;
- how the value enters gameplay and why it can affect future authoritative
  state;
- whether schema 9 already records or deterministically reconstructs it;
- a minimal scenario that would expose the omission;
- confidence: confirmed omission, likely risk, or speculative;
- the smallest additive recording change, only if one is genuinely needed.

Explicitly separate:

- recording-format gaps;
- original-engine determinism bugs that should instead be fixed at the source;
- Rust parity bugs that should be fixed without changing the format;
- diagnostic dump improvements that are useful but not replay inputs.

Prioritize confirmed and likely issues. End with a concise recommendation:
“no schema change needed”, “additive schema fields advisable”, or “new schema
required”, with justification. Do not infer a format change solely because the
two engines currently diverge.

## Deferred audit findings

The July 2026 read-only audit found the following issues outside the implemented
configuration and determinism fixes. They are intentionally deferred:

- Authoritative console and developer actions bypass the schema-9 command log.
  Either add typed resolved commands for supported actions or reject a trace as
  non-replayable when one occurs. Do not capture the resulting world state.
- The trace has no executable/data identity. Add a build identifier and content
  fingerprints for the profile CPF, mission/proto data, SCB, and authoritative
  sound metadata so the replay runner can reject a mismatched data directory.
- The C++ writer emits schema-9 command variants that
  `original_parity_replay.rs` does not deserialize or apply, including
  select-all, single-PC unselect, action-index/cancel, modifier state, macros,
  drop-ale, box-unselect, shield, and teleport commands. This is a Rust parity
  runner bug and does not require a recording-format change.
- Rust's case-insensitive path resolution selects the first matching
  `read_dir` entry. Reject case-folded duplicate names or choose a stable sorted
  match; this is a loader determinism fix, not replay input.
- Build/ABI and floating-point-environment metadata may be useful for rejecting
  unsupported replay hosts. Per-frame floating-point results must remain
  diagnostics rather than replay inputs.

Broad entity state, selected-PC snapshots, visibility queries, motion-grid
changes, and path events remain diagnostic comparisons. They must not be fed
back into the Rust simulation to conceal parity divergences.

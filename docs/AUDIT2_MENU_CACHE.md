# Menu cache identity and optional asset lookup

Implemented in `audit2-menu-cache`.

## Changes

- Lazy menu images use a structured `(resource source, generation, resource ID,
  subpicture index)` key, replacing arithmetic aliases and source-blind IDs.
- Resource managers allocate fresh process-local source identities on construction,
  clone, and deserialization. Runtime identity is skipped by serde and bitcode;
  persisted resource bytes retain their existing layout.
- Attach (including partial failure), merge, reader rebinding, picture dismissal,
  and shipping re-encoding advance the generation. Lazy decoding and recovery do
  not: they populate the current view rather than repeatedly invalidating it.
- `find_picture` / `find_pictures` return `Result<Option<_>>`: absent resources and
  sparse/out-of-range frames are distinct from decode and recovery failures.
- Optional menu APIs preserve their `Option` contract but log failed resources.
  Local-first external lookup falls through only on absence, never corruption.
  Sprite-pack loading also reports failures and preserves sparse frame positions.
- Old-generation GPU owners remain alive until menu retirement so already queued
  draws stay valid; new lookups cannot reuse old-generation entries.

## Verification

Added CPU regressions for source/clone/serde/bitcode identity, mutation invalidation,
arithmetic key collisions, sparse slots, malformed encoded pictures, and failed
recovery. Extended the existing named GPU ownership gate with source separation,
rebinding invalidation, local corruption versus external fallback, and retirement.

`cargo fmt --all` and `git diff --check` run locally. No compilation or test success
is claimed: the coordinator requested combined validation to avoid parallel cold
builds. Required integration suites: `cargo test -p robin_assets`, affected
`robin_rs` menu tests, and the named Vulkan GPU ownership gate. The existing
resource wire-contract test must continue passing.

## Remaining tradeoff

Invalidation is manager-wide, intentionally conservative. Repeated hot reloads
retain old lazy surfaces until the menu retires; promptly reclaiming those would
require a renderer-aware retirement boundary rather than discarding owners at
lookup time. TODO: consider per-resource generations only if measured reload
workloads justify the additional bookkeeping.

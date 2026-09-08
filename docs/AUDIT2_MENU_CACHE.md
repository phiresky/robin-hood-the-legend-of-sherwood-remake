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

Formatting and whitespace checks passed. Coordinated acceptance passed: asset
library 142 tests plus unchanged resource wire contracts (`9f0b52c2a`); default
client 1578 tests (`f5f531c7`); pure assets 76 tests plus five fixture tests without
engine adapters (`07520a632`); resolved dependency boundary; and the named Vulkan
GPU execution gate (`07520a632`). The GPU fixture initially failed before cache
assertions because `from_value` rejected a flattened integer-map representation.
Test-only `3ddb65de6` uses real SRES/PIC loading and the public shipping encoder;
source/clone/rebind/corruption/retirement assertions all passed on rerun.

Additional native GL execution could not obtain an adapter because amd64
EGL/GLES libraries are absent. It is not reported green. Browser protocol/audio
24/24 passed, but is not full-game browser GPU coverage. Full provenance and
limits: [final acceptance](AUDIT2_PLAN.md#final-acceptance).

## Remaining tradeoff

Invalidation is manager-wide, intentionally conservative. Repeated hot reloads
retain old lazy surfaces until the menu retires; promptly reclaiming those would
require a renderer-aware retirement boundary rather than discarding owners at
lookup time. TODO: consider per-resource generations only if measured reload
workloads justify the additional bookkeeping.

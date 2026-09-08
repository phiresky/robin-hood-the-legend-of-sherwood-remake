# Historical parity schema isolation

## Change

The current JSON/native-v68 schema remains in `original_parity_replay.rs`.
All existing version-specific wire declarations and conversion implementations
now live together in three private sibling modules:

- `v66.rs`: retained v66 headers, commands, actor/human/AI snapshots, frames,
  engine-state diagnostics, record envelopes, and compatibility conversions.
- `v67.rs`: original v67 vector transient headers, boolean increment-validity
  element snapshots, frames, record envelopes, and conversions.
- `v67_late.rs`: the accidental late-v67 optional transient header and record
  envelope. This remains separate even where it has the same shape as v68.

Only the historical header and record envelopes are imported into the parent.
Storage continues to choose the exact decoder generation before returning
current data. The early-v67-first/late-v67-fallback order is unchanged, as are
the contextual increment-validity reconstruction and all missing-value rules.
Reverse header conversions remain test-only. Production still writes v68.

No field or variant was added, deleted, reordered, or changed in type; no native
version or codec dependency changed. Historical bitcode derives are retained
because these are existing authoritative artifact layouts, not new serializers.
Serde and bitcode annotations were moved unchanged. Visibility changes only
allow access within the existing private parity implementation boundary.

## Fixed golden evidence

`historical_golden.rs` contains fixed hex bytes, not data encoded by the decoder
under test. They were emitted from declarations read with `git show` from
pre-refactor commit `53e36bdc46542b51a5358674e663961de9f25d1b` using bitcode
**0.6.9**, the repository's pinned parity codec. A small independent generator
copied those declarations transitively, preserving all wire fields and variants;
it omitted serde-only attributes/derives because it did not parse JSON. It did
not depend on or compile the engine or the refactored parity modules.

Fixture construction starts with that commit's `minimal_test_native_header`
and source fingerprint `frozen-before-audit2`, with these overrides:

- RNG seed `0x123456789abcdef0`, frame `321`, random input seed
  `Some(0xfedcba98)`.
- Two initial NPC transient entries `(creation_order, maximal_visibility)`:
  `(93, 1234)` and `(107, 5678)`.
- One motion layer `7`, line `31`, endpoints `(1, -2)` and `(3, 4)`, mask
  `0x1234`, sector `-7`, active `true`.
- RNG prefix values `[9, 81, 729]`.
- After the existing test-only conversion to v66, its historical-only
  `authoritative_state` is set to `Some("retained-v66-only")`.

The command-vector fixture contains `SelectAllPcs`, `SetLockAlt(true)`,
`SelectActionIndex(0x12345678)`, then two `SwordStrike` commands with respectively
absent and `Some(3.5)` seek distances. Both use PC 17, soldier 93, original command
24/name `sword`, and `with_seek = true`. These fixed bytes detect variant-order
changes and exercise the v66-only optional-to-NaN conversion.

The generator and its normal, isolated Cargo target are retained at
`/tmp/audit2-parity-golden.i0wVlg` for this session. This temporary path is not a
test dependency. The committed hex values and construction description are the
durable regression evidence; do not silently regenerate them after a schema edit.

Four new tests check production historical header dispatch and late-layout
selection, decoded nonzero nested values and vector order, exact re-encoding of
fixed historical bytes, command conversion, and rejection of truncation and an
unsupported version. Existing generated-fixture, standalone conversion,
reblocking, and legacy semantic tests remain in place.

## Validation and remaining boundaries

- `cargo fmt --all`: passed.
- `git diff --check`: passed.
- Independent bitcode-only fixture generator: build and execution passed.
- Coordinated explicit `robin_parity` suite passed at `9f0b52c2a`: 155 library
  tests, dependency-closure integration and doctests. All four frozen-byte tests
  passed. Independent comparison matched all 42 moved declarations/conversions.
- Optional `--features client` check passed at `d224ecdae` after `5ad9693fd`
  corrected a pre-existing render consumer to borrow `host.frontend.viewport`.
  The base already had this API mismatch; no historical schema changed.
- Fresh native game replay/save-load EOF acceptance passed; this does not claim
  execution of original licensed parity corpora. Exact provenance and limits:
  [final acceptance](AUDIT2_PLAN.md#final-acceptance).

This change isolates native wire generations, not every behavioral repair for
old Original captures. Simulation compatibility helpers stay with their callers;
their existing evidence-specific admission rules have not been generalized.
Shared unchanged child types are still shared with current v68 layouts, including
late-v67's frame type. They are transitively frozen for their native versions.
TODO: whenever a future native version changes a shared child, freeze that exact
child at the historical boundary before modifying the current schema. Wholesale
duplication now would add conversion risk without changing the current format.

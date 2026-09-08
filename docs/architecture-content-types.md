# Shared content contracts and the format-only asset boundary

## Implemented

- New `robin_content` owns the coherent sprite-content contracts:
  `SpriteVariant` and `PixelOpacityLookup`, including the canonical opacity
  fingerprint algorithm. The engine's original public paths remain reexports.
- The leaf crate normally depends only on Serde and SHA-256. Its explicit
  `simulation-codecs` feature preserves existing state-hash and snapshot
  implementations for engine consumers; no serialization format was changed.
- `robin_assets` defaults to `engine-adapters`, preserving existing game/tool
  APIs. Disabling defaults removes the optional simulation dependency.
- Format-only builds retain picture, packed sprite, sprite grid, RLE/JXL and
  legacy frame-bank codecs. Legacy frame loading now has an explicit independent
  entry point; shipping loading delegates to it only when no shipping bank exists.
- Actor names, VM decompile/disassemble/SCB, shipping containers and resource
  managers remain adapter APIs. Their dependencies include actual simulation
  profiles/scripts/level structures, not merely small data types.
- Adapter-dependent integration tests and examples declare required features.
  Codec tests still run without adapters. The WASM benchmark retains its default
  shipping-mission exports and additionally provides a raw VQ decode endpoint
  usable in no-default-feature builds.

## Dependency measurement

On native Linux, with the checked-in lockfile, unique package entries from
`cargo tree --locked -p robin_assets --edges normal --prefix none --format '{p}'`:

| Selection | Reachable packages including root |
| --- | ---: |
| Default, backward-compatible adapters | 144 |
| `--no-default-features` codecs | 105 |

The 39-package difference includes `robin_engine`, `robin_run_protocol`,
the engine-only overlay version, native-enum tooling, signing dependencies,
and simulation serialization helpers. The default graph intentionally retains
simulation; this is a real selectable boundary, not a claim that every asset
adapter is independent. Package counts are not compile-time measurements.

## Validation

- `cargo test --locked -p robin_content -p robin_assets --no-default-features`:
  3 leaf contract tests, 76 asset unit tests and 5 fixture-resolver tests passed;
  3 original-data asset tests remain explicitly ignored. This exercises the
  optimized frame-holder opacity fingerprint against the shared canonical
  implementation, plus raw sprite and picture codec regression suites.
- `cargo check --locked -p robin_assets --no-default-features --example
  wasm_decode_bench --target wasm32-unknown-unknown --profile wasm-dev` passed
  at `751d7b1bd` (5m31s cold). This checks the real raw VQ JS export without
  linking simulation; it is a compile check, not a browser timing measurement.

- `CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test --locked -p robin_content
  --features simulation-codecs -p robin_assets -p robin_engine` passed:
  default asset unit/integration suites, all 4 leaf contract tests including
  unchanged snapshot bytes and state hashes, 4,449 engine unit tests (4 ignored),
  15 engine integration tests, and 21 engine doctests (1 ignored).
- The codec-only suite was rerun after the final source commit
  `751d7b1bd` and passed again. `cargo fmt --all` and `git diff --check`
  passed. Existing unrelated engine/data-I/O warnings were left unchanged.

No game execution, browser benchmark timing, or full client build is claimed
by this isolated lane; those belong to combined integration acceptance.

## Deliberately not relocated

Coordinates are intertwined with geometry, hashing and legacy I/O; shipping
mission containers include full simulation level and profile types. Moving
these wholesale would conceal rather than simplify the dependency. A future
physical adapter-crate split can build on the enforced feature boundary if
independent shipping-container codecs become a concrete requirement.

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

TODO: Record explicit content, codec-only, default asset, engine compatibility,
and WASM example results once the cold worktree builds finish.

## Deliberately not relocated

Coordinates are intertwined with geometry, hashing and legacy I/O; shipping
mission containers include full simulation level and profile types. Moving
these wholesale would conceal rather than simplify the dependency. A future
physical adapter-crate split can build on the enforced feature boundary if
independent shipping-container codecs become a concrete requirement.

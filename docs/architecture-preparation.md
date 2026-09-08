# Prepared mission resource boundaries

## Implemented

`LevelAssets` now explicitly separates navigation geometry/topology,
environment geometry, deterministic audio inputs, and process-local
attachments. Existing scripts and entity identity groups remain intact.
Construction-only sprite prototypes, profiles, and deterministic localized
names remain simulation inputs, not presentation-only data. Decoded terrain
bitmaps and interactive uploads already belong to `LoadedCpuMission` /
`LoadedInteractiveResources`, outside the engine assets; they are not moved
back into the new groups.

Every engine/host/verifier/parity consumer now names the owning domain. Arc
sharing, authored vector ordering, sparse topology `None` entries, and
snapshot restoration of live callbacks are unchanged. `LevelAssets::new`
retains its previous initialization values. No serialized engine field or
state-hash domain changed.

Audio timing tables are no longer independently writable by host crates.
`LevelAudioAssets::publish_timing` validates concrete sample identities and
positive duration values before atomically replacing all three tables.
Failed publication preserves the old tables. Interactive, headless and replay
use the same CPU mission loader; ranked verification and headless parity also
use this publication API. Preparation remains independent of sound playback
being enabled.

Missing source timings are deliberately not synthesized or made a new
unranked load failure. Missing speech remains `None`, absent source duration
entries remain absent, and authored random gaps/variant ordering are retained.
The normal loader warns when metadata cannot satisfy ranked completeness.
`validate_ranked_timing` centralizes exactly the existing strict policy:
nonempty speech catalog, timing for every listed variant, and every required
source ID present. The existing projection/admission boundary consumes that
result. Empty synthetic levels remain supported.

The content projection still exhaustively destructures every field, now also
inside each group. Its serialized payload structs, field order, component
order and values are unchanged. Domain grouping must not change replay,
official projection or save identity.

## Authority and remaining scope

Serde for process-local attachments deliberately drops executable/opacity
implementations. Decoding an audio group likewise does not confer mission
preparation or ranked authority: `PreparedMissionInputs` still seals the
actual engine inputs and ranked admission still checks completeness.

Navigation/environment fields remain mutable during mission construction;
the existing `PreparedMissionInputs` boundary seals them before execution.
This refactor does not impose a new typestate on synthetic fixture builders,
change legacy save topology, or introduce a second engine resource store.

TODO: where future loading stages need fewer inputs, pass the relevant domain
borrow instead of all of `LevelAssets`; do not duplicate resource state in a
new service container.

## Validation

Added focused engine tests for optional versus required timing, malformed
concrete metadata, atomic publication failure, empty synthetic preparation,
deserialized missing metadata, attachment absence, and Arc sharing across
snapshots. Existing projection/speech and load/snapshot tests cover the
unchanged deterministic consumers.

Commands/results will be recorded after the isolated package suites finish.

# Explicit mission preparation stages

## Scope

The former 17-argument `load_level_and_sprite_bank` no longer hides engine
construction, ranked admission, and presentation attachment inside one loader.
Both interactive and true-headless bootstrap now explicitly advance:

1. `prepare_mission` returns an owning `PreparedMission`: decoded mission,
   complete deterministic assets, campaign, launch authority, and outstanding
   presentation work.
2. `PreparedMission::construct_engine` consumes those inputs, snapshots the
   pre-construction campaign for replay, performs the preserving engine
   constructor, and consumes ranked admission. It returns `ConstructedMission`.
3. `ConstructedMission::attach_presentation` resolves the caller's terrain join
   policy, applies optional legacy display state, initializes the viewport,
   and verifies the published sprite generation before returning
   `LoadedMissionCore` to existing frontend bootstrap.

Private fields and consuming methods make these runtime stages linear: callers
cannot publish a core before construction or reuse a stage to run startup twice.
Serde emits diagnostic stage labels and rejects deserialization; it does not
recreate live engines, terrain jobs, or ranked authority.

Loading feedback is a separate borrowed channel. Interface metadata groups the
ground mark, titbit rows, and minimap placement inputs. Launch setup groups the
deterministic seed/options with ranked admission. These are not a single bag of
the old positional arguments.

## Preserved ordering and failure behavior

- Terrain work starts before sprite-bank, script, and deterministic audio work.
  The single-threaded WASM fallback still decodes before dimension resolution.
- Deterministic audio metadata and sprite opacity are complete before engine
  preparation. Multiplayer Welcome seed/options retain their existing override.
- Replay campaign capture, construction, projection export, and ranked admission
  retain their original order; no extra RNG operation or callback is introduced.
- Construction leaves terrain work pending. Interactive attachment forwards it
  to the existing pre-upload join; true-headless attachment joins it before
  returning the runtime core.
- File/decode errors before construction still return the campaign. Ingestion
  failure returns the constructor-preserved campaign allocation. Post-engine
  terrain/legacy failures retain the original replay-campaign recovery policy.
- Legacy adoption, viewport setup, debug flags, and initial shadow-key checks
  remain after engine construction.

## Validation

Four new native unit tests invoke the same production construction/attachment
methods. They cover launch/config retention, replay snapshot retention, deferred
versus immediate terrain joins, exact campaign allocation recovery on ingestion
failure, explicit terrain failure, and non-restorable stage diagnostics.
They inject a `PreparedMission` fixture: they do **not** execute the complete
file/audio preparation path or establish multiplayer seed negotiation coverage.
Full datadir lifecycle and multiplayer acceptance are coordinated at integration.

At source commit `19b0da68a0bae7e29bdb68ae69a9832942001025`, the named
`browser-audio` gate passed both audio and audio+multiplayer WASM bin/test checks,
linked the audio+multiplayer test module, and ran all 24 real Chrome tests with
zero failures. Evidence: `/tmp/robin-lifecycle-gate-cd6g0qvi`.
The four new native tests are compiled in that gate but are not Chrome tests;
their execution belongs to the combined explicit native client suites.

Final combined source `09a2438b1` passed the full default and release-feature
library suites, executing the four stage tests, and the tools/projection-export
examples check. The final named browser gate again passed all 24 Chrome tests.
Both live multiplayer scenarios and all four native lifecycle/replay phases
passed against the final release-feature binary. See
[combined acceptance](REFACTOR_BOUNDARIES.md) for counts and artifact provenance.

## Remaining seams

TODO: preparation is still substantial. Its file/resource, terrain-dimension,
and deterministic-audio work can be decomposed further after the independent
performance changes land. The current change deliberately leaves their order
and nearby code stable rather than importing those unrelated changes.

The active performance work also changes terrain handoff and whether browse-only
construction needs a verification projection. Future merges must preserve that
policy inside the new preparation/construction boundaries, not restore the
monolithic loader or copy an older version of portrait ownership handling.

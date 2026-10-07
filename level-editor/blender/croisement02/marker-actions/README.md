# Marker visual sampling candidate

Private integration recipe; these files do not alter the application runtime.

Run from the repository root:

```sh
node --test level-editor/blender/croisement02/marker-actions/sampler.test.mjs
```

Tests require the existing local Crossings02 source evidence and marker export.
They check real frame delays, all 45 target identities/placements and all 66
exported image hashes. They do not prove browser decoding, native composition,
script execution or human animation behavior.

`createMarkerAdapter(profile, exported).sample(snapshot)` accepts mission and
target index, action, progression mode, active state, ambiance, and the number
of completed eligible updates since that action was reset. The runtime owner
must supply that count from the authoritative simulation: inactive periods and
global freeze contribute zero updates. Do not derive it from wall time, resource
load completion, render count, or mission age across an action reset. Replaying
the same snapshot has no side effects and no other animation's phase changes.

Zero completed updates preserves the reset sentinel. The first eligible update
selects frame 0 with counter 0. Thus presentation ticks 0/2/3/59/60 correspond
to completed update counts 1/3/4/60/61. Freeze-last stops on arrival at the final
frame; a one-frame freeze-last row retains its sentinel. `motion` is the status
at the sampled eligible update, not a newly emitted event. The caller must not
re-emit it on repeated, paused or inactive samples. Audio and gameplay command
events are intentionally not emitted by this visual adapter.

Apply each returned presentation as a unit: body, shadow, and placement must
all change together. Both exported images use the common `canvasBounds`.
`sourceOffset` describes the original frame and must not translate the already
composed images a second time. Hidden actions 210 and 211 preserve identity but
return null body and shadow. They are valid transparent art, not a missing asset.
Shadow coverage means destination darkening, with the selected ambiance's
retention factor; it is not blue paint or an opaque black surface.

The caller preserves authored visual x/y/z and projection support separately
from the target's interaction point. It also supplies validated target/script
identity and the native ordering/masking policy. This adapter deliberately does
not map script handles from scene child order or execute branch-dependent calls.

TODO: wire through the sole runtime owner's shared-clock and native-composition
adapter, then verify native-camera phase/shadow/hide/reset and mission switching
in the browser. Human directional shadow swapping remains a separate runtime
fix; this marker-only candidate does not change characters or infer their AI.

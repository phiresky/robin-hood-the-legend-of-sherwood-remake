# Source-clock component clips

`native-element-component-clips` is a separate reviewed preview contract. It
combines the existing morph-capable clip sampler with independent target owners
and retained native artwork. It does not change legacy motion validators.

The schema is in `shared/src/component-clips.ts`. Each component pins one model
binding and named clip, its disjoint mesh names, its source stages, visibility
phases, and its separately verified terminal subset. Components using the same
full clip model must use identical bindings; their `nodes` together cover that
model exactly. Each owner receives an isolated playback instance.

Each stage includes the exact `source_element_sha256` and `source_frames` copied
from the native contract. `frame_ticks` has one entry per source frame. Non-null
entries equal `clip_start_tick` plus source delays (`delay + 1`) since
`first_source_frame`; timing resampling is rejected. Every null entry requires
a `gaps` record naming that frame and its image hash, with reason
`component-absent`. This means reviewed component absence, not a transparent
whole sprite. Frame zero requires this explicit binding. Inactive targets do
not start their clips.

Family elements either own a component stage or belong to
`native_passthrough_element_ids`, never both. Owners may coexist, but a component
cannot have two visible non-gap owners simultaneously. Completed owners retain
their last source frame independently; a later cask owner cannot retire a frozen
body or wheel owner. Reset and replay use source-event batches. Shared-clock
preview seek/play is rejected for staged clips.

`visibility_phases` covers every tick through the component terminal. It enables
owned nodes and disables the others, supporting both exported scale-zero phases
and separate visibility manifests. An active node with three zero scale axes is
rejected; a plane with only one zero axis still counts as drawable. An unexpected
node cannot be hidden by omission: declared nodes cover the pinned model exactly.

A remainder frame uses `source-frame-components` and lists exactly the components
owned by that element/frame. The renderer samples their mapped ticks before
returning separate per-owner physical roots. Retained native and ambiguous
pixels keep their draw priority. Explicit native-only targets remain unsuppressed
in physical preview.

At load, each terminal is sampled and checked against its applied subset. Checks
include evaluated morph positions and normals, world placement, indices,
UV/color/tangent data, material groups and draw ranges, materials and texture
pixels/sampling. Retired meshes must be declared and disabled or zero on all
three axes. Missing active geometry, extra visible geometry, incorrect morphs
and changed materials fail. Active terminal subsets together cover the family's
applied model exactly. Runtime checks do not replace visual approval.

Image textures are compared through decoded pixels using `OffscreenCanvas`;
typed image data is supported directly. Unreadable pixels fail explicitly.
Skinning and material animation are outside this format.

Full physical playback samples elapsed source ticks, including ticks within a
held native image. Each owner's final source frame holds its mapped pose. Hybrid
remainder playback deliberately samples the exact source-frame start, keeping
physical surfaces aligned with retained pixels. These are separate sampling
paths; delayed images must not quantize full physical animation.

Asset export validation must also sample between authored transform keys,
including fractional ticks, and compare evaluated surfaces against the editable
animation. Key-pose agreement alone cannot detect interpolation excursions from
world-origin pivot cancellation. Prefer local rigid pivots with stable parent
transforms; inspect trajectory and bounds at quarter-key intervals before
approving a cart clip. Runtime finite checks are not a geometric parity proof.

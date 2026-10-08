# Explicit physical shadow binding

The private receiver candidate projects the selected source frame's keyed shadow
onto evaluated triangles. It does not alter the character artwork or advance a
clock. A physical shadow is not proof of the character body's ground contact.

The runtime boundary must supply:

- Stable actor identity, current frame resource, activity, pose and camera elevation.
- Explicit physical receiver ownership from the current placement. A rendered
  contact overlay or a nearby mesh is not sufficient authority.
- Evaluated world transforms and geometry revision for those receiver meshes.
- Exact source shadow alpha bounds, retaining the original full-frame UV mapping.

`evaluatePhysicalReceiver` excludes nearly vertical and downward shell faces.
`projectedTerrainReceivers` selects camera-ray footprint intersections without
silently resolving overlaps. `projectShadowReceivers` requires complete unique
coverage and retains separate vertices across discontinuities. Unsupported gaps
and overlapping owners remain errors, not a fallback to a four-corner plane.

The runtime owner should prepare geometry before changing either body or shadow;
`createReceiverActorFrameBinding` demonstrates atomic replacement and cleanup.
Resources decoded by the character loader stay borrowed. Cache invalidation must
include frame, pose, receiver membership, geometry and world-transform revisions.
Retirement, inactive actors and empty shadow frames remove the owned shadow.

The typed implementation now lives in `app/src/actor-shadow-receivers.ts` and
`app/src/actor-receiver-binding.ts`. The viewport exposes an explicit selection
for an editable character through the State Preview's **Shadow surface** control.
Its initial authority is the reviewed north woodland bank's five physical meshes,
bound to the model hash and current placement; the contact appearance is excluded.
Changing the receiver placement invalidates the cached projection. Missing or
ambiguous support retires the owned shadow and reports an error.

The production loader has GPU evidence for four authored directions from front
and reverse cameras, with current body and shadow frames checked together. A
normal-library-route editor proof also checks explicit receiver selection,
position changes, camera reversal, clearing, and playback/view controls. Its
copied runtime is bound to commit `5f394d36e`: 13 control assertions and four
independent full-frame pixel comparisons pass. The character dropdown retains
its stable identity when edits replace an actor record. Full-scene foliage
occludes the reverse-view character, so the separate unobstructed loader captures
remain the physical-shadow inspection evidence. Root review is still required.

Existing imported character shadows still use the older support path; editable
actors do not implicitly acquire receiver ownership. This scoped preview does
not simulate a mission or establish character-body ground contact. Later changes
to the live runtime are not implicitly accepted by a copied-runtime proof.

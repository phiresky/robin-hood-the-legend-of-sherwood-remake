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

The current candidate is not installed in the shared viewport. Existing imported
character shadows still have the older support path, and editable actors do not
implicitly acquire receiver ownership. A later adapter must surface unresolved
support, use the same current frame for body and shadow, and receive browser
verification after integration.

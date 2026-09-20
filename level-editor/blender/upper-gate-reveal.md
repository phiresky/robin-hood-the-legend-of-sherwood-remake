# Upper gate chamber projection

`derby_upper_gate_projection.json` is an explicit opt-in review for Derby
patch003. Copy this record into a layer manifest's
`projection_reviews["patch-003"]`. Existing manifests and frozen review sheets
keep their old receiver and visibility definitions. The new review checks exact
revealed-source and patch-alpha hashes before changing the partition.

The covered sprite disappears when the chamber patch applies; its mask state
changes independently of sight obstacles. The chamber patch has no sight-list
change. Its native mask transition replaces layer2 indices61/62, layer4 index5,
and layer3 index0 with layer2 indices63–68, layer3 index1, layer0 index108, and
layer4 index6. This does not authorize hiding an entire gatehouse wall mesh.

The reviewed geometry recipe splits source parts257 and263 into closed components:

| Component | Projection component ID | Revealed visibility |
| --- | --- | --- |
| Chamber facade cover | `upper-chamber-removable-cover` | Hidden |
| Upper merlons and lower parapet | `upper-chamber-retained-parapet` | Retained |
| West facade cover | `upper-chamber-west-removable-cover` | Hidden |
| West chamber wall | `upper-chamber-west-retained-wall` | Retained |

Each pair preserves its own canonical source_node and logical gatehouse asset. Their covered
union must match the prepartition surface and volume. Component selectors also
require `reveal_component_role` and `reveal_component_patch_id`; missing,
ambiguous, wrong-patch, or exterior-pass selectors fail instead of broadening the
exclusion to the whole source node. Projection never changes scene visibility.
Explicit `receiver_components` keeps the263 cover in the exterior projection and
its retained wall in the interior projection. Treating both as interior would
show the wood chamber through the closed facade. The renderer partitions actual
objects after applying these selectors, rather than requiring a source node to
have only one projection layer.

Revealed source-camera rays identify floor249, west chamber wall263, and stair265
in addition to terrace receivers252/253. Their revealed pixels are used only
inside the positive native patch alpha. Outside that region, the covered image
and its full occluder geometry remain authoritative. The removable cover is
excluded only from the revealed BVH; retained257 still blocks rays. The covered
fallback explicitly includes both reviewed cover components, even when a reveal
preview hides them. This prevents covered pixels outside the patch alpha from
passing through hidden covers and landing on interior surfaces.

Actor occlusion masks do not cover every visible floor pixel. In source-mask
manifests, the newly identified chamber receivers use positive authored patch003
alpha for their interior ownership override, with native scenery masks retained
for exterior fallback. Do not replace this with a bounding box or apply the
interior ownership mask to the entire building.

The shared bake, legacy preliminary projection, and reference renderer accept
`exclude_occluder_components`. Workspace review definitions emit this field only
for the opted-in patch. Frozen manifests compare it as ownership evidence and
reject changes; create a new review packet for the changed partition.

Run the regression with:

```sh
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/test_reveal_components.py
```

It tests the shared bake and preview, retained parapet occlusion, covered-source
fallback with visible and hidden covers, default compatibility, geometry
invariance, exact source binding, separate receivers sharing one canonical ID,
masked preview metadata, and the frozen-review guard. Runtime reveal visibility is separate from this
projection configuration.

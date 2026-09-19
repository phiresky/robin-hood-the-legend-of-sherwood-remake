"""Annotate projection receivers without equating sight blockers with removable walls.

Run annotate_layers(layers_json) after grouping. This only writes metadata; it
never hides geometry. Render/cutaway visibility is a separate authored decision.
The manifest preserves each patch's independent mask and sight state changes.
"""
import json
from pathlib import Path

# Reviewed interior geometry in the Derby catalog. Overlap alone is deliberately
# not an assignment: exterior walls and furniture can occupy the same pixels.
DERBY_INTERIORS = {
    "patch-000": [*range(223, 228), *range(229, 239)],
    "patch-001": [239, 240],
    "patch-002": [186, *range(241, 249)],
    "patch-003": [252, 253],
}


def projection_receivers(manifest):
    if manifest["map"].casefold() != "derby":
        raise ValueError("Interior receiver roles need an authored map-specific review")
    return {patch: [f"building-{n:03d}" for n in nodes]
            for patch, nodes in DERBY_INTERIORS.items()}


def annotate_layers(manifest_path):
    import bpy
    path = Path(manifest_path).resolve()
    manifest = json.loads(path.read_text())
    receivers = projection_receivers(manifest)
    working = bpy.data.collections.get(f"{manifest['map']} Working")
    if working is None:
        raise ValueError(f"Missing working collection for {manifest['map']}")
    objects = [o for o in working.all_objects if o.type == 'MESH' and o.get('source_node')]
    available = {o['source_node'] for o in objects}
    missing = {n for nodes in receivers.values() for n in nodes} - available
    if missing:
        raise ValueError(f"Missing authored interior receivers: {sorted(missing)}")
    working['reveal_manifest_path'] = str(path)
    annotated = []
    for obj in objects:
        node = obj['source_node']
        interior = [patch for patch, nodes in receivers.items() if node in nodes]
        candidates = [p['id'] for p in manifest['patches']
                      if any(c['source_node'] == node for c in p['coverage_candidates'])]
        before = [p['id'] for p in manifest['patches'] if node in p['sight_before']]
        after = [p['id'] for p in manifest['patches'] if node in p['sight_after']]
        obj['projection_layer'] = 'interior' if interior else 'exterior'
        obj['reveal_role'] = 'interior' if interior else ('ambiguous' if candidates else 'shared')
        obj['reveal_patch_ids'] = interior
        obj['reveal_candidate_patch_ids'] = candidates
        obj['sight_patch_before_ids'] = before
        obj['sight_patch_after_ids'] = after
        obj['projection_layer_manifest'] = str(path)
        if interior:
            annotated.append({'name': obj.name, 'source_node': node, 'patches': interior})
    bpy.context.scene['projection_layer_manifest'] = str(path)
    report = {'map': manifest['map'], 'interior_receivers': receivers,
              'annotated_meshes': annotated,
              'notes': 'Sight activation and rendered cutaway geometry are independent. Candidate overlap does not authorize removal.'}
    (path.parent / 'receiver-assignments.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def interior_preview(manifest_path, patch_id):
    """Temporary diagnostic view of reviewed receivers, restoring all visibility.

    Use `with interior_preview(path, 'patch-000'):` around render_views(...).
    This is an isolated furniture/interior inspection, not a proposed in-game
    cutaway: exterior candidate walls are intentionally not classified as removed.
    """
    from contextlib import contextmanager

    @contextmanager
    def preview():
        import bpy
        manifest = json.loads(Path(manifest_path).read_text())
        nodes = set(projection_receivers(manifest)[patch_id])
        # This upper slab encloses the dining hall; omit it only for inspection.
        if manifest['map'].casefold() == 'derby' and patch_id == 'patch-000':
            nodes.discard('building-228')
        working = bpy.data.collections[f"{manifest['map']} Working"]
        objects = [o for o in working.all_objects if o.type == 'MESH']
        visibility = [(o, o.hide_render) for o in objects]
        try:
            for obj, hidden in visibility:
                obj.hide_render = hidden or obj.get('source_node') not in nodes
            bpy.context.view_layer.update()
            yield nodes
        finally:
            for obj, hidden in visibility:
                obj.hide_render = hidden
            bpy.context.view_layer.update()
    return preview()

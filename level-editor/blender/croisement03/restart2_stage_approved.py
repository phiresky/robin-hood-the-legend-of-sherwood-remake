"""Privately integrate and export the five approved Croisement03 appearances."""
import hashlib
import json
import sys
from pathlib import Path

import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from render_slots import acquire, release
from texture_staging import verify_baked_geometry
from supplemental_parts import import_scenery_part
from import_reviewed_geometry import import_asset_geometry
from group_assets import reconcile_asset_groups
from export_editor import export_asset_library
from refinement_workspace import _geometry
from workspace_components import appearance_state
from review_evidence import sha


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def signature(obj, detail=False):
    appearance = appearance_state(obj)
    for material in appearance['materials']:
        if material is None:
            continue
        material.pop('name')
        for node in material.get('nodes', []):
            if 'image' in node:
                node['image'].pop('name')
    value = dict(world_vertices=[list(obj.matrix_world @ vertex.co) for vertex in obj.data.vertices],
                       faces=[list(face.vertices) for face in obj.data.polygons],
                       source_node=obj.get('source_node'), asset_group=obj.get('asset_group'),
                       visibility=[obj.hide_render, obj.hide_viewport], appearance=appearance)
    return value if detail else digest(value)


def main():
    root = ROOT / 'level-editor/work/croisement03-refinement/restart2/integration-round1'
    plan = json.loads((root / 'plan.json').read_text())
    for field in ['baseline', 'catalog', 'handoffs', 'source_level']:
        assert sha(Path(plan[field])) == plan[field + '_sha256']
    handoffs = json.loads(Path(plan['handoffs']).read_text())
    output = Path(plan['output'])
    assert not output.exists()
    acquire()
    try:
        checks, expected, expected_detail, import_drift = [], {}, {}, {}
        for item in handoffs:
            checks.append(verify_baked_geometry(item))
            meshes = [obj for obj in bpy.data.collections[item['collection_name']].all_objects
                      if obj.type == 'MESH' and obj.get('asset_group') == item['asset_id']]
            expected[item['asset_id']] = sorted(signature(obj) for obj in meshes)
            expected_detail[item['asset_id']] = [signature(obj, detail=True) for obj in meshes]
        bpy.ops.wm.open_mainfile(filepath=plan['baseline'])
        bpy.context.preferences.filepaths.save_version = 0
        bpy.context.window.scene = bpy.data.scenes['Croisement03 Refinement']
        collection = bpy.data.collections['Croisement03 Working']
        selected = {node for item in handoffs for node in item['source_nodes']}
        outside = [obj for obj in collection.all_objects if obj.type == 'MESH' and obj.get('source_node') not in selected]
        before = {obj.name: _geometry(obj, protect_appearance=True) for obj in outside}
        imports = []
        for item in handoffs:
            if item.get('new_scenery_part'):
                result = import_scenery_part(item, collection.name)
            else:
                result = import_asset_geometry(item['blend_path'], asset_id=item['asset_id'],
                         object_names=item['object_names'], collection_name=collection.name,
                         source_nodes=item['source_nodes'])
            imports.append(result)
        grouping = reconcile_asset_groups(plan['catalog'], preserve_objects=[obj for obj in outside if obj.get('source_node') != 'ground'])
        after = {obj.name: _geometry(obj, protect_appearance=True) for obj in outside}
        assert before == after, 'Unselected scene geometry/materials changed'
        for item in handoffs:
            meshes = [obj for obj in collection.all_objects if obj.type == 'MESH' and obj.get('asset_group') == item['asset_id']]
            actual = [signature(obj, detail=True) for obj in meshes]
            approved = expected_detail[item['asset_id']]
            valid = len(actual) == len(approved)
            drift = 0.0
            for original, imported in zip(approved, actual):
                a, b = dict(original), dict(imported)
                source_points, points = a.pop('world_vertices'), b.pop('world_vertices')
                valid &= a == b and len(source_points) == len(points)
                drift = max(drift, max((abs(x - y) for p, q in zip(source_points, points)
                                       for x, y in zip(p, q)), default=0.0))
            # Reparenting decomposes matrices to Blender float transforms. Keep
            # faces, ownership, UVs and shader/image bytes exact; permit at most
            # one thousandth of a source-pixel world unit in transformed points.
            import_drift[item['asset_id']] = drift
            if not valid or drift > 0.001:
                diagnostic = dict(asset=item['asset_id'], expected=approved, actual=actual)
                (root / ('import-difference-' + item['asset_id'] + '.json')).write_text(json.dumps(diagnostic) + '\n')
                raise ValueError('Imported mesh differs: ' + item['asset_id'])
        output.mkdir()
        bpy.ops.wm.save_as_mainfile(filepath=str(output / 'worker.blend'))
        catalog = json.loads(Path(plan['catalog']).read_text())
        exports = export_asset_library('Croisement03', output / '3d-assets', plan['source_level'],
                                      asset_ids=[item['asset_id'] for item in handoffs], catalog=catalog)
        for item in handoffs:
            assert all(sha(Path(path)) == digest for path, digest in item['protected_files'].items())
        report = dict(status='Private staging complete; browser and scene integration review pending',
                      publication_approved=False, plan_sha256=sha(root / 'plan.json'),
                      model_sha256=sha(output / 'worker.blend'), imports=imports, geometry_checks=checks,
                      grouping=grouping, unselected_meshes_preserved=len(before),
                      imported_faces_uv_materials_identical=True, maximum_world_coordinate_error=import_drift,
                      world_coordinate_error_limit=0.001, exports=exports,
                      limitations=['Full map remains largely unrefined.',
                                   'Bridge and fallen-log terrain/riverbed remain separate hypotheses, not complete scene ground.',
                                   'Fern receiving trunks and foreground plant integration remain unfinished.',
                                   'Five standalone appearances are approved; this is not a full-map completion claim.'])
        (output / 'stage-report.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(dict(output=str(output), assets=len(handoffs), unselected_meshes_preserved=len(before))))
    finally:
        release()


if __name__ == '__main__':
    main()

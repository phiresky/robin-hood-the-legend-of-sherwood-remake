"""Export a staged Lincoln publication: full map, standalone assets and deterministic metadata.

Run from the repository root after `publish_stage.py` (never promotes anything):

    blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/publish_export.py -- \
      --stage level-editor/work/needs-review/lincoln-refinement/publication-N/stage-vM \
      --tooling <frozen tooling snapshot directory> \
      --progress <collector gallery-progress.json> \
      --level datadirs/fullgame_gog_hackable/Data/Levels/Lincoln.rhp.json

The worker is opened read-only; its `lincoln …` scene and collection are renamed in memory to
the catalog's display map name so exported descriptors say `Lincoln`. Catalog-v2 component
splits are exported as separate selectable parts (`building-NNN--component-<name>`) by the
shared exporter. The standalone assets are exported first; the map is then written in the
library format (`lincoln.rhlos-map.json` + `map-assets/`) so map instances and palette entries
reference the same local catalog models (see refinement/PROCEDURE.md "Map publication format").
Framing (`size`, `camera`, `exportBounds`) is retained from the live library document.
Writes `lincoln.rhlos-map.json`, `map-assets/`, `assets/`, `publication-metadata.json`,
`effective-plan.json` (for `verify_staged_handoffs.py`) and `stage.json` (for
`verify_publication_assets.py`).
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage', type=Path, required=True)
    parser.add_argument('--tooling', type=Path, required=True)
    parser.add_argument('--progress', type=Path, required=True)
    parser.add_argument('--level', type=Path, required=True)
    args = parser.parse_args(argv)
    stage = args.stage.resolve(strict=True)
    integration = json.loads((stage / 'integration.json').read_text())
    if sha(stage / 'worker.blend') != integration['worker_sha256']:
        raise ValueError('Staged worker changed after integration')
    if sha(stage / 'catalog.json') != integration['staged_catalog_sha256']:
        raise ValueError('Staged catalog changed after integration')
    catalog = json.loads((stage / 'catalog.json').read_text())
    map_name = catalog['map']
    level = json.loads(args.level.read_text())
    progress = json.loads(args.progress.read_text())
    workspaces = {row['id']: Path(row['workspace']) for row in progress['assets'] if row['status'] == 'approved'}
    asset_ids = sorted(group['id'] for group in catalog['groups'])
    if set(asset_ids) - set(workspaces):
        raise ValueError('Publication contains unapproved assets: ' + repr(sorted(set(asset_ids) - set(workspaces))))
    plan = json.loads(Path(integration['plan']).read_text())
    approvals = {}
    for record in json.loads((Path.cwd() / plan['approvals']).read_text())['approvals']:
        if record['decision'] == 'approved' and record['scope'] in ('geometry', 'geometry-and-states'):
            approvals.setdefault(record['asset_id'], []).append(record)

    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from render_slots import acquire
    acquire()
    from freeze_tooling import select_tooling
    tooling = select_tooling(args.tooling)
    import bpy
    import numpy as np
    from export_editor import export_editor, export_asset_library
    from publication_contract import publication_parts, validate_export_records

    bpy.ops.wm.open_mainfile(filepath=str(stage / 'worker.blend'))
    # The map export writes glTF resources as separate files named after the images; baked atlas
    # names contain ' / '. Rename in memory only (the worker is never saved); published
    # resources are content-addressed, so names carry no meaning there.
    for image in bpy.data.images:
        if '/' in image.name:
            image.name = image.name.replace('/', '-')
    bpy.data.scenes['lincoln Refinement'].name = map_name + ' Refinement'
    working = bpy.data.collections['lincoln Working']
    working.name = map_name + ' Working'
    bpy.context.window.scene = bpy.data.scenes[map_name + ' Refinement']

    declared = publication_parts(catalog)
    meshes = [o for o in working.all_objects if o.type == 'MESH' and not o.hide_render]
    bindings = validate_export_records(catalog, [dict(name=o.name, source_node=o['source_node'],
        asset_group=o.get('asset_group'), projection_component=o.get('projection_component'))
        for o in meshes], None)

    # Deterministic per-asset metadata, computed from the exact meshes being exported.
    images = {}

    def image_sha(image):
        if image.name not in images:
            if not image.packed_file:
                raise ValueError('Unpacked publication image: ' + image.name)
            images[image.name] = hashlib.sha256(image.packed_file.data).hexdigest()
        return images[image.name]

    def mesh_hashes(obj):
        mesh = obj.data
        co = np.empty(len(mesh.vertices) * 3, dtype=np.float64)
        mesh.vertices.foreach_get('co', co)
        m = np.array(obj.matrix_world, dtype=np.float64)
        world = np.round(co.reshape(-1, 3) @ m[:3, :3].T + m[:3, 3], 4).astype(np.float64)
        loops = np.empty(len(mesh.loops), dtype=np.int64)
        mesh.loops.foreach_get('vertex_index', loops)
        totals = np.empty(len(mesh.polygons), dtype=np.int64)
        mesh.polygons.foreach_get('loop_total', totals)
        uv = hashlib.sha256()
        for layer in sorted(mesh.uv_layers, key=lambda layer: layer.name):
            data = np.empty(len(mesh.loops) * 2, dtype=np.float32)
            layer.data.foreach_get('uv', data)
            uv.update(layer.name.encode() + b'\0' + data.tobytes())
        return (hashlib.sha256(world.tobytes() + loops.tobytes() + totals.tobytes()).hexdigest(),
                uv.hexdigest(), len(mesh.vertices), len(mesh.polygons))

    def materials(obj):
        rows = []
        for material in obj.data.materials:
            if material is None:
                rows.append(None)
                continue
            textures = sorted({image_sha(node.image) for node in material.node_tree.nodes
                               if node.type == 'TEX_IMAGE' and node.image}) if material.use_nodes else []
            rows.append({'name': material.name, 'image_sha256': textures,
                         'generated_source_sha256': material.get('generated_source_sha256')})
        return rows

    def masks(asset_id, nodes):
        path = workspaces[asset_id] / 'source-masks.json'
        manifest = json.loads(path.read_text())
        rows = []
        for layer, projection in sorted(manifest['projections'].items()):
            for assignment in projection['assignments']:
                if assignment.get('source_node') in nodes:
                    rows.append({'layer': layer, 'source_node': assignment['source_node'],
                                 'mask_indices': assignment.get('mask_indices', []),
                                 'constraint_kind': assignment.get('constraint_kind')})
        rows.sort(key=lambda row: (row['layer'], row['source_node'], row['mask_indices']))
        return {'manifest': str(path), 'manifest_sha256': sha(path), 'assignments': rows}

    state_keys = ('reveal_material_states', 'reveal_hide_when_applied', 'reveal_show_when_applied',
                  'reveal_material_patch', 'drawbridge_patch_id', 'sight_patch_id', 'mission_patch_profile')
    groups = {group['id']: group for group in catalog['groups']}
    records = []
    for asset_id in asset_ids:
        owned = sorted((o for o in meshes if o.get('asset_group') == asset_id), key=lambda o: o.name)
        components = []
        geometry, uvs = hashlib.sha256(), hashlib.sha256()
        for obj in owned:
            geometry_sha, uv_sha, vertices, faces = mesh_hashes(obj)
            geometry.update(bindings[obj.name].encode() + geometry_sha.encode())
            uvs.update(bindings[obj.name].encode() + uv_sha.encode())
            states = {}
            for key in state_keys:
                value = obj.get(key)
                if value is not None:
                    states[key] = value.to_list() if hasattr(value, 'to_list') else (
                        value.to_dict() if hasattr(value, 'to_dict') else value)
            components.append({'name': obj.name, 'source_node': obj['source_node'],
                               'projection_component': obj.get('projection_component'),
                               'editor_part_node': bindings[obj.name], 'vertices': vertices, 'faces': faces,
                               'geometry_sha256': geometry_sha, 'uv_sha256': uv_sha,
                               'materials': materials(obj), 'states': states})
        nodes = sorted({o['source_node'] for o in owned})
        approval = approvals.get(asset_id, [])
        model = workspaces[asset_id] / 'model.blend'
        model_sha = sha(model)
        bound = [record for record in approval if record['model_sha256'] == model_sha]
        if not bound:
            raise ValueError('Approved model hash no longer matches its approval: ' + asset_id)
        record = bound[-1]
        records.append({
            'asset_id': asset_id, 'name': groups[asset_id]['name'],
            'source_nodes': nodes,
            'editor_parts': sorted(key for key, value in declared.items() if value['asset_id'] == asset_id),
            'split_parts': sorted(key for key, value in declared.items()
                                  if value['asset_id'] == asset_id and value['source_components']),
            'components': components,
            'masks': masks(asset_id, set(nodes)),
            'states': sorted({key for component in components for key in component['states']}),
            'geometry_sha256': geometry.hexdigest(), 'uv_sha256': uvs.hexdigest(),
            'approved_model': {'path': str(model), 'sha256': model_sha, 'workspace': str(workspaces[asset_id])},
            'approval': {key: record[key] for key in ('decision', 'scope', 'exact_text', 'model_sha256',
                                                      'modified_views_sha256', 'recorded_utc', 'evidence_directory')},
            'texture': ('global source reprojection plus generated fill of still-unknown texels'
                        if 'texture_combine' in integration else
                        'global source reprojection' if 'global_reprojection' in integration else
                        'approved source-projected materials'),
        })

    asset_report = export_asset_library(map_name, stage / 'assets', str(args.level.resolve()),
                                        asset_ids=asset_ids, catalog=catalog)
    live_document = Path.cwd() / 'level-editor/library/scenes' / (map_name.lower() + '.rhlos-map.json')
    map_settings = json.loads(live_document.read_text()) if live_document.exists() else {}
    map_report = export_editor(map_name, stage / (map_name.lower() + '.rhlos-map.json'), catalog=catalog,
                               level=level, map_settings=map_settings)
    index = json.loads((stage / 'assets/index.json').read_text())
    files = {entry['id']: {'descriptor_sha256': sha(stage / 'assets' / entry['descriptor']),
                           'model_sha256': sha(stage / 'assets' / entry['model'])} for entry in index['assets']}
    for record in records:
        record['files'] = files[record['asset_id']]
    generated = {}
    for obj in meshes:
        for material in obj.data.materials:
            if material and material.get('generated_source_sha256'):
                generated.setdefault(material['generated_source_sha256'], set()).add(material.name)
    ground = [o for o in meshes if o['source_node'] == 'ground']
    metadata = {'version': 1, 'map': map_name, 'stage': str(stage),
                'worker_sha256': integration['worker_sha256'],
                'catalog_sha256': integration['staged_catalog_sha256'],
                'source_catalog': integration['catalog'], 'source_catalog_sha256': integration['catalog_sha256'],
                'level': str(args.level.resolve()), 'level_sha256': sha(args.level),
                'tooling': tooling['snapshot_id'], 'tooling_directory': tooling['directory'],
                'map_scene': {'file': map_report['file'], 'sha256': sha(map_report['file']),
                              'library': map_report.get('library'),
                              'groups': map_report['assets'], 'parts': map_report['parts'],
                              'meshes': map_report['meshes']},
                'ground': [{'name': o.name, 'asset_group': o.get('asset_group'),
                            'materials': materials(o), 'geometry_sha256': mesh_hashes(o)[0],
                            'status': 'staged ground (lincoln-terrain catalog asset)'} for o in ground],
                'assets': records}
    (stage / 'publication-metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')

    effective = {'output': str(stage), 'baseline': integration['baseline'], 'collection_name': 'lincoln Working',
                 'map_name': map_name, 'catalog': str(stage / 'catalog.json'), 'hackable_map': str(args.level.resolve()),
                 'imports': [{'asset_id': item['asset_id'], 'blend_path': item['source_blend'],
                              'blend_sha256': item['model_sha256'],
                              'review_manifest': str(Path(item['source_blend']).parent / 'workspace.json'),
                              'source_nodes': item['canonical_parts']} for item in integration['imports']]}
    (stage / 'effective-plan.json').write_text(json.dumps(effective, indent=2) + '\n')
    report = {'plan': str(stage / 'effective-plan.json'), 'imports': integration['imports'],
              'canonical_parts': len({o['source_node'] for o in meshes if o['source_node'] != 'ground'}),
              'generated_materials': {key: sorted(value) for key, value in generated.items()},
              'map': map_report, 'assets': asset_report, 'metadata': str(stage / 'publication-metadata.json')}
    (stage / 'stage.json').write_text(json.dumps(report, indent=2, default=str) + '\n')
    print(json.dumps({'map': {k: map_report[k] for k in ('assets', 'parts', 'meshes')}, 'assets': asset_report}), flush=True)


if __name__ == '__main__':
    main(sys.argv[sys.argv.index('--') + 1:])

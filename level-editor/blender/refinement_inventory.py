"""Inventory a complete scene for a grouping agent; validate its reviewed catalog.

Run through Blender MCP against a copied scene. This module never reparents or
changes geometry. Reviewed ownership is applied separately with group_assets.
"""
import hashlib
import json
import math
from pathlib import Path

import bpy


def inventory(output_dir, *, collection_name, map_name, source_path, elevation_degrees=35,
              patch_manifest=None):
    output = Path(output_dir).resolve()
    output.mkdir(parents=True, exist_ok=False)
    collection = bpy.data.collections[collection_name]
    sine = math.sin(math.radians(elevation_degrees))
    cosine = math.cos(math.radians(elevation_degrees))
    records = []
    for obj in sorted(collection.all_objects, key=lambda o: o.name):
        if obj.type != 'MESH' or obj.hide_render:
            continue
        if not obj.get('source_node'):
            raise ValueError(f'Missing stable source_node: {obj.name}')
        points = [obj.matrix_world @ v.co for v in obj.data.vertices]
        if not points:
            raise ValueError(f'Empty mesh: {obj.name}')
        screen = [(p.x, -p.y*sine-p.z*cosine) for p in points]
        records.append({
            'object': obj.name, 'source_node': obj['source_node'],
            'group': obj.get('asset_group'), 'name': obj.get('asset_name'),
            'part_name': obj.get('part_name'),
            'vertices': len(points), 'faces': len(obj.data.polygons),
            'bounds_world': [[min(p[i] for p in points) for i in range(3)],
                             [max(p[i] for p in points) for i in range(3)]],
            'bounds_source_pixels': [min(p[0] for p in screen), min(p[1] for p in screen),
                                     max(p[0] for p in screen), max(p[1] for p in screen)],
            'projection_layer': obj.get('projection_layer'),
        })
    source = Path(source_path).resolve(strict=True)
    result = {'version': 1, 'map': map_name, 'collection': collection_name,
              'source_blend': bpy.data.filepath, 'source_image': str(source),
              'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
              'objects': records}
    if patch_manifest:
        patch_path = Path(patch_manifest).resolve(strict=True)
        patches = json.loads(patch_path.read_text())
        if patches['map'] != map_name:
            raise ValueError('Patch manifest belongs to a different map')
        result['patch_manifest'] = str(patch_path)
        result['patch_manifest_sha256'] = hashlib.sha256(patch_path.read_bytes()).hexdigest()
        result['patch_assets'] = [dict(patch, origin=origin)
                                  for origin, values in (('base', patches['patches']),
                                      ('mission', patches.get('mission_patches', [])))
                                  for patch in values]
    result['patch_inventory_status'] = ('supplied; requires visual/state grouping review'
                                        if patch_manifest else 'missing; static coverage only')
    (output/'inventory.json').write_text(json.dumps(result, indent=2)+'\n')
    (output/'INSTRUCTIONS.md').write_text('''# Scene grouping review

Inspect the entire source image and every object in inventory.json. Identify
logical assets from architecture and spatial context, not existing Group N names.
One building includes its roof, walls, attached stairs and building parts. Keep
parts independently addressable beneath that asset. Separate freestanding props;
retain interior/patch membership. Include terrain as an explicitly reviewed role.

Write catalog.json using the authored catalog schema: version 1, map, groups of
{id, name, parts: [{obstacle, name}]}. Every non-ground source_node must occur
exactly once, even when several mesh components share that source ID. Names must
describe objects. Do not infer missing ownership from nearest bounding boxes.
Write grouping-review.md explaining decisions and remaining ambiguities. Run
refinement_inventory.validate_catalog before preparing per-asset workspaces.
Geometry refinement starts only after all ownership ambiguities are resolved.
Review patch_assets as well: mission-owned drawbridges, mechanisms, animated
props and revealed interiors may be absent from static obstacle geometry. Record
their states, mission associations and logical ownership separately. Complete
static-part coverage alone does not establish complete scene coverage.
''')
    return {'inventory': str(output/'inventory.json'), 'mesh_components': len(records),
            'source_nodes': len({r['source_node'] for r in records})}


def validate_catalog(inventory_path, catalog_path):
    scene = json.loads(Path(inventory_path).read_text())
    catalog = json.loads(Path(catalog_path).read_text())
    if catalog.get('version') != 1 or catalog.get('map') != scene['map']:
        raise ValueError('Catalog version/map differs from scene inventory')
    expected = {r['source_node'] for r in scene['objects']} - {'ground'}
    owners = {}; ids = set(); names = set()
    for group in catalog['groups']:
        if not group['id'] or group['id'] in ids:
            raise ValueError('Missing/duplicate asset ID')
        ids.add(group['id'])
        name = group['name'].strip()
        if not name or name.casefold() in names or name.lower().startswith('group '):
            raise ValueError(f'Missing, duplicate or placeholder asset name: {name}')
        names.add(name.casefold())
        if not group['parts']:
            raise ValueError(f'Empty asset: {name}')
        for part in group['parts']:
            number = part['obstacle']
            if type(number) is not int or number < 0 or not part['name'].strip():
                raise ValueError(f'Invalid named source part: {part}')
            node = f'building-{number:03}'
            if node in owners:
                raise ValueError(f'Duplicate ownership: {node}')
            owners[node] = group['id']
    if set(owners) != expected:
        raise ValueError(f'Coverage mismatch: missing={sorted(expected-set(owners))}, extra={sorted(set(owners)-expected)}')
    return {'status': 'PASS', 'groups': len(ids), 'source_parts': len(owners),
            'terrain_review_required': any(r['source_node']=='ground' for r in scene['objects'])}

"""Trim York's buried reconstruction surfaces against the raised terrain."""
import hashlib
import json
import math
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT/'level-editor/work/active/york'
sys.path.insert(0, str(Path(__file__).parent))
from terrain_clip import area, clip, prism
from floor_contacts import load_contacts, extension, cutters as floor_cutters

# Reviewed terrain, ramps and raised lanes. Bridge decks and roof receivers are
# not ground: their undersides can be visible and must not become solid cutters.
SUPPORTS = {86, 87, 88, 90, 91, 92, 93, 96, 98, 106, 110}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    import bpy
    from mathutils import Matrix, Vector
    sys.path.insert(0, str(ROOT/'level-editor/refinement'))
    from render_slots import acquire
    acquire()
    source = OUT/'grouped/york-grouped.blend'
    output = OUT/'grounding/york-grounded.blend'
    if output.exists():
        raise FileExistsError(output)
    frozen = sha(source)
    bpy.ops.wm.open_mainfile(filepath=str(source))
    bpy.context.window.scene = bpy.data.scenes['york Refinement']
    working = bpy.data.collections['york Working']
    objects = [o for o in working.objects if o.type == 'MESH' and not o.hide_render]
    contacts = load_contacts()
    extensions = {}
    for identity, contact in contacts.items():
        positions = [tuple(o.matrix_world @ v.co) for o in objects
                     if o.get('asset_group') == identity for v in o.data.vertices]
        if not positions:
            raise ValueError('Missing floor-contact asset: '+identity)
        extensions[identity] = extension(identity, positions, contact)
    support_ids = {f'building-{i:03}' for i in SUPPORTS}
    cutters = []
    for obj in objects:
        if obj.get('source_node') not in support_ids:
            continue
        obj.data.calc_loop_triangles()
        for tri in obj.data.loop_triangles:
            points = [tuple(obj.matrix_world @ obj.data.vertices[i].co) for i in tri.vertices]
            cutter = prism(points, obj['source_node'])
            if cutter and cutter['hi'][2] > .1:
                cutters.append(cutter)
    originals = bpy.data.collections.new('york Grounding Originals')
    bpy.context.scene.collection.children.link(originals)
    originals.hide_render = True
    originals.hide_viewport = True
    changed, unaffected, removed_evidence = [], [], []
    for obj in objects:
        floor = extensions.get(obj.get('asset_group'))
        is_support = obj['source_node'] in support_ids
        if obj['source_node'] == 'ground' or (is_support and not floor):
            unaffected.append(obj.name)
            continue
        obj.data.calc_loop_triangles()
        uvs = list(obj.data.uv_layers)
        # A support may itself continue below its lower landing. Only its
        # reviewed floor may trim it; never subtract its own support volume.
        object_cutters = ([] if is_support else cutters) + (floor_cutters(floor) if floor else [])
        retained, removed, total = [], [], 0.0
        for tri in obj.data.loop_triangles:
            points = [tuple(obj.matrix_world @ obj.data.vertices[v].co) + tuple(
                x for layer in uvs for x in layer.data[loop].uv)
                for v, loop in zip(tri.vertices, tri.loops)]
            pieces, buried = clip(points, object_cutters)
            retained.extend((poly, tri.material_index) for poly in pieces)
            removed.extend((poly, support, tri.material_index) for poly, support in buried)
            total += area(points)
        removed_area = sum(area(p) for p, _, _ in removed)
        # Roundoff fragments alone do not justify changing a source mesh.
        if removed_area < .01:
            unaffected.append(obj.name)
            continue
        saved = obj.copy(); saved.data = obj.data.copy(); originals.objects.link(saved)
        saved.hide_render = True; saved.hide_viewport = True
        vertices, faces, corner_uvs, materials = [], [], [], []
        inv = obj.matrix_world.inverted()
        for poly, material in retained:
            for b, c in zip(poly[1:], poly[2:]):
                tri = [poly[0], b, c]
                if area(tri) <= 1e-7:
                    continue
                start = len(vertices)
                vertices.extend(tuple(inv @ Vector(p[:3])) for p in tri)
                corner_uvs.extend([p[3:] for p in tri])
                faces.append((start, start+1, start+2)); materials.append(material)
        mesh = bpy.data.meshes.new(obj.data.name+' / terrain clipped')
        mesh.from_pydata(vertices, [], faces)
        for material in obj.data.materials:
            mesh.materials.append(material)
        for face, material in zip(mesh.polygons, materials):
            face.material_index = material
        for i, layer in enumerate(uvs):
            target = mesh.uv_layers.new(name=layer.name)
            for j, values in enumerate(corner_uvs):
                target.data[j].uv = values[i*2:i*2+2]
        obj.data = mesh
        obj['refinement_recipe'] = 'york/ground_assets.py'
        supports = sorted({support for _, support, _ in removed})
        obj['support_floor_source_node'] = ','.join(supports)
        remaining_area = sum(area(poly) for poly, _ in retained)
        if abs(total-remaining_area-removed_area) > max(.01, total*1e-7):
            raise ValueError('Surface accounting mismatch: '+obj.name)
        record = {'object':obj.name, 'asset':obj['asset_group'], 'source_node':obj['source_node'],
                  'component':obj.get('projection_component'), 'supports':supports,
                  'before_area':total, 'removed_area':removed_area, 'retained_area':remaining_area,
                  'remaining_triangles':len(faces)}
        changed.append(record)
        removed_evidence.append({'object':obj.name,'asset':obj['asset_group'],
            'source_node':obj['source_node'], 'polygons':[{'vertices':p,'support':s,'material':m} for p,s,m in removed]})
    bpy.context.view_layer.update()
    anchors = {}
    for identity in sorted({r['asset'] for r in changed}):
        parts = [o for o in objects if o.get('asset_group') == identity]
        positions = [o.matrix_world @ v.co for o in parts for v in o.data.vertices]
        if not positions:
            raise ValueError('Whole asset buried; requires an explicit nonrendering review: '+identity)
        low = [min(p[k] for p in positions) for k in range(3)]
        high = [max(p[k] for p in positions) for k in range(3)]
        anchor = [round((low[0]+high[0])/2), round((low[1]+high[1])/2), max(0.0, low[2])]
        parent = next(o for o in working.objects if o.type == 'EMPTY' and o.get('asset_group') == identity)
        matrices = {o:o.matrix_world.copy() for o in parent.children}
        parent.matrix_world = Matrix.Translation(Vector(anchor))
        for obj, matrix in matrices.items():
            obj.matrix_world = matrix
        parent['asset_origin_scene'] = anchor
        anchors[identity] = anchor
    bpy.context.view_layer.update()
    if sha(source) != frozen:
        raise ValueError('Grouping input changed')
    output.parent.mkdir(exist_ok=True)
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(output))
    report = {'status':'PASS', 'input_sha256':frozen, 'output_sha256':sha(output),
        'recipe_sha256':sha(__file__), 'clip_recipe_sha256':sha(Path(__file__).with_name('terrain_clip.py')),
        'floor_contacts_sha256':sha(Path(__file__).with_name('floor-contacts.json')),
        'floor_extensions':extensions,
        'support_sources':sorted(support_ids), 'support_triangles':len(cutters),
        'assets_changed':len(anchors), 'meshes_changed':len(changed), 'meshes_unchanged':len(unaffected),
        'changes':changed, 'anchors':anchors, 'new_caps':False,
        'scope':'Remove surfaces inside reviewed terrain volumes and explicit asset floor continuations; retain exposed lower walls and all original meshes in a hidden reference collection.'}
    (output.parent/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    (output.parent/'removed-surfaces.json').write_text(json.dumps(removed_evidence,separators=(',',':'))+'\n')
    print(json.dumps({k:v for k,v in report.items() if k not in ('changes','anchors','floor_extensions')}))


if __name__ == '__main__':
    main()

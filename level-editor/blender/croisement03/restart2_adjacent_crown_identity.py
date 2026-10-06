"""Audit approved pre-fill and filled crowns independently of the wood export."""
import hashlib
import json
import sys
from pathlib import Path

import bpy
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from evidence_io import sha, write_json
from render_slots import acquire, release

B = ROOT / 'level-editor/work/croisement03-refinement/restart2'


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def value(socket):
    if not hasattr(socket, 'default_value'):
        return None
    v = socket.default_value
    if isinstance(v, (str, int, float, bool)):
        return v
    try:
        return list(v)
    except TypeError:
        return str(v)


def snapshot(path, tree):
    bpy.ops.wm.open_mainfile(filepath=str(path))
    obj = next(o for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == f'croisement03-arbre06-fragment-tree{tree}-provisional')
    mesh = obj.data
    materials = []
    for mat in mesh.materials:
        nodes = []
        for node in mat.node_tree.nodes:
            row = dict(name=node.name, type=node.type, inputs=[value(s) for s in node.inputs])
            for key in ('operation', 'blend_type', 'uv_map', 'interpolation', 'extension'):
                if hasattr(node, key):
                    row[key] = getattr(node, key)
            if node.type == 'TEX_IMAGE':
                im = node.image
                row['image'] = dict(size=list(im.size), colorspace=im.colorspace_settings.name,
                                    alpha_mode=im.alpha_mode,
                                    rgba_sha256=hashlib.sha256(np.asarray(im.pixels[:], np.float32).tobytes()).hexdigest())
            nodes.append(row)
        links = [(l.from_node.name, l.from_socket.identifier, l.to_node.name, l.to_socket.identifier) for l in mat.node_tree.links]
        materials.append(dict(nodes=nodes, links=links))
    return dict(
        positions=digest([list(v.co) for v in mesh.vertices]),
        topology=digest([(list(f.vertices), f.material_index, f.use_smooth) for f in mesh.polygons]),
        transform=digest([list(row) for row in obj.matrix_world]),
        normals=digest([list(n.vector) for n in mesh.corner_normals]),
        uv=digest([(u.name, u.active_render, [list(x.uv) for x in u.data]) for u in mesh.uv_layers]),
        colors=digest([(a.name, a.domain, a.data_type, [list(x.color) for x in a.data]) for a in mesh.color_attributes]),
        material_graph_and_rgba=digest(materials),
        object=obj.name, vertices=len(mesh.vertices), faces=len(mesh.polygons),
    )


def main():
    out = B / 'adjacent-crown-renderer-audit-v1'
    assert not out.exists()
    acquire()
    try:
        rows = []
        for tree, revision in ((12, 7), (14, 5)):
            before = B / f'tree{tree}-crownfragment-v{revision}/worker.blend'
            after = B / f'tree{tree}-approved-wood-texture-v1/source-restored-fill-v1/worker.blend'
            a, b = snapshot(before, tree), snapshot(after, tree)
            assert a == b, (tree, a, b)
            rows.append(dict(tree=tree, geometry_approved_sha256=sha(before), appearance_approved_sha256=sha(after),
                             crown_exact=True, crown=a, export_model_sha256=sha(B / f'tree{tree}-exact-export-v2/model.glb')))
        out.mkdir()
        write_json(out / 'identity.json', dict(status='PASS exact pre-fill crown identity', trees=rows,
            scope='Positions, topology, normals, transform, UV, color attributes, material graph and decoded RGBA are identical before and after wood fill. Browser versus Blender differences remain a separate renderer comparison, not corrected here.'))
    finally:
        release()


if __name__ == '__main__':
    main()

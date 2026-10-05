"""Apply private closed contour hulls and independently verify native rays."""
import json
import sys
from pathlib import Path

import bpy
import bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from refinement_workspace import _geometry
from render_slots import acquire, release
from tree_geometry import SIN, COS, RAY


def main():
    base = OUT / 'restart2-vegetation'; old = base / 'southwest-branches-v1/model.blend'
    recipe = base / 'southwest-convex-contour-research-v2/proposal.json'; data = json.loads(recipe.read_text())
    assert sha(old) == data['original_model_sha256']
    dest = base / 'southwest-convex-v3'; dest.mkdir(exist_ok=False)
    bpy.ops.wm.open_mainfile(filepath=str(old)); bpy.context.view_layer.update(); bpy.context.preferences.filepaths.save_version = 0
    obj = bpy.data.objects['Southwest stump root and branch tangle']
    protected = {o.name: _geometry(o, protect_appearance=True) for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type == 'MESH' and o != obj}
    materials = list(obj.data.materials); vertices = []; faces = []; vertex_parts = []
    for part in data['parts']:
        start = len(vertices); vertices.extend(part['vertices']); vertex_parts.extend([part] * len(part['vertices']))
        faces.extend(tuple(start + i for i in f) for f in part['triangles'])
    mesh = bpy.data.meshes.new('Southwest roots with bounded convex contours'); mesh.from_pydata(vertices, [], faces); mesh.update()
    bm = bmesh.new(); bm.from_mesh(mesh); bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    topology = dict(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges), degenerate_faces=sum(f.calc_area() < 1e-9 for f in bm.faces))
    assert topology == dict(nonmanifold_edges=0, degenerate_faces=0), topology
    bm.to_mesh(mesh); bm.free(); mesh.update()
    for material in materials: mesh.materials.append(material)
    native = mesh.uv_layers.new(name='Native front projection'); grain = mesh.uv_layers.new(name='Inferred board grain')
    for polygon in mesh.polygons:
        polygon.material_index = 0 if polygon.normal.dot(RAY) > .01 else 1
        for li in polygon.loop_indices:
            vi = mesh.loops[li].vertex_index; v = mesh.vertices[vi].co; part = vertex_parts[vi]
            original = np.asarray(part['original_vertices']); a = original[:12].mean(axis=0); b = original[12:24].mean(axis=0); axis = b-a; length = np.linalg.norm(axis); axis /= length
            helper = (0.,0.,1.) if abs(axis[2]) < .9 else (1.,0.,0.); u = np.cross(axis,helper); u /= np.linalg.norm(u); side = np.cross(axis,u); q = np.asarray(v)-a
            native.data[li].uv = (v.x/1792,1-(-v.y*SIN-v.z*COS)/1152)
            grain.data[li].uv = (np.arctan2(q@side,q@u)/np.pi,(q@axis)/14)
    obj.data = mesh; bpy.context.view_layer.update()
    proposal = json.loads((old.parent / 'proposal.json').read_text()); targets = [r['pixel'] for r in proposal['hits'] if r['object'] is None]
    tree = BVHTree.FromPolygons([obj.matrix_world @ v.co for v in mesh.vertices], [list(p.vertices) for p in mesh.polygons])
    missed = []
    for x,y in targets:
        if tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000,-RAY)[0] is None: missed.append([x,y])
    assert not missed, missed
    for name,digest in protected.items(): assert _geometry(bpy.data.objects[name],protect_appearance=True)==digest,name
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True)
    saved = sha(dest/'model.blend'); bpy.ops.wm.open_mainfile(filepath=str(dest/'model.blend'))
    for name,digest in protected.items(): assert _geometry(bpy.data.objects[name],protect_appearance=True)==digest,name
    assert sha(old)==data['original_model_sha256']
    write_json(dest/'evidence.json',dict(status='Private closed contour construction; actual8/source/neighbor review pending',model_sha256=saved,recipe=str(recipe),recipe_sha256=sha(recipe),original_model_sha256=data['original_model_sha256'],protected_appearance=protected,topology=topology,target_pixels=len(targets),independent_bvh_covered=len(targets),added_points=len(data['additions']),limitations=['Closed primitive volumes preserved; local projected contour placement and hidden depth inferred.','Source ownership unchanged;Additional outside-domain edge pixels disclosed in analytic projection receipt; two native-traced upper twigs added.','Actual material, full framing, ground and neighbor review still required.']))
    (dest/'recipe.py').write_text(Path(__file__).read_text())
    print(saved,len(targets),'native contour rays covered')


if __name__ == '__main__':
    acquire()
    try: main()
    finally: release()

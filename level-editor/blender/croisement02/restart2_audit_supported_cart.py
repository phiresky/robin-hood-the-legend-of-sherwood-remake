"""Reopen a private wreck and check finite support against the pinned receiver."""
import sys, json
from pathlib import Path
import bpy, bmesh, numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from scipy.spatial import ConvexHull
HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[1] / 'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json


def main():
    dest = OUT / 'restart2-state/south-cart-wreck-solid-v3'; manifest = json.loads((dest / 'manifest.json').read_text())
    model = dest / 'worker.blend'; assert sha(model) == manifest['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(model)); bpy.context.view_layer.update()
    rows = []; contacts = []; samples = []; total_mass = 0.; moment = np.zeros(3)
    for obj in bpy.context.scene.objects:
        if obj.type != 'MESH': continue
        bm = bmesh.new(); bm.from_mesh(obj.data); assert all(e.is_manifold for e in bm.edges), obj.name
        volume = bm.calc_volume(signed=True); assert volume > 0; bm.free()
        obj.data.calc_loop_triangles(); vertices = np.array([obj.matrix_world @ v.co for v in obj.data.vertices]); center = vertices.mean(axis=0)
        triangles = np.array([vertices[list(t.vertices)] - center for t in obj.data.loop_triangles])
        signed = np.einsum('ij,ij->i', triangles[:, 0], np.cross(triangles[:, 1], triangles[:, 2])) / 6
        mass = signed.sum(); centroid = center + (signed[:, None] * triangles.sum(axis=1) / 4).sum(axis=0) / mass
        total_mass += mass; moment += centroid * mass
        minimum = float(vertices[:, 2].min()); touching = vertices[vertices[:, 2] <= .05]
        contacts.extend(touching.tolist()); samples.extend(vertices.tolist())
        rows.append(dict(name=obj.name, closed_positive_volume=float(volume), minimum_z=minimum, near_plane_vertices=len(touching), uniform_density_centroid=centroid.tolist()))
    center = moment / total_mass; contact_points = np.unique(np.array(contacts)[:, :2], axis=0); hull = ConvexHull(contact_points)
    margins = -(hull.equations[:, :2] @ center[:2] + hull.equations[:, 2]); inside = bool(np.all(margins >= -1e-5))
    frozen = json.loads((OUT / 'restart2-textures/approved6-ground-scene-v1/assembly.json').read_text())['ground']
    ground = Path(frozen['model']); assert sha(ground) == frozen['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(ground)); bpy.context.view_layer.update()
    vertices = []; faces = []; objects = []
    for obj in bpy.context.scene.objects:
        if obj.type != 'MESH' or obj.hide_render: continue
        points = [obj.matrix_world @ v.co for v in obj.data.vertices]
        # The pinned floor is the unique mesh whose evaluated vertices all lie on Z0.
        if not points or max(abs(p.z) for p in points) > 1e-3: continue
        start = len(vertices); vertices.extend(points); faces.extend(tuple(start + i for i in p.vertices) for p in obj.data.polygons); objects.append(obj.name)
    assert objects, 'Pinned ground artifact contains no evaluated flat receiver'
    bvh = BVHTree.FromPolygons(vertices, faces); clearances = []; misses = 0
    for p in samples:
        hit = bvh.ray_cast(Vector((p[0], p[1], 1000)), Vector((0, 0, -1)), 2000)
        if hit[0] is None: misses += 1
        else: clearances.append(p[2] - hit[0].z)
    write_json(dest / 'reopened-support-audit.json', dict(status='PASS scoped finite ground support' if inside and misses == 0 and min(clearances) >= -.001 else 'HOLD ground or load support', model_sha256=manifest['model_sha256'], ground_model_sha256=frozen['model_sha256'], receiver_objects=objects, components=rows, sampled_vertices=len(samples), receiver_misses=misses, minimum_receiver_clearance=min(clearances), near_plane_tolerance=.05, support_hull=contact_points[hull.vertices].tolist(), uniform_density_center_of_mass=center.tolist(), center_inside_support_hull=inside, minimum_hull_margin=float(margins.min()), limitations=['Uniform-density rigid assembly check only; no friction, joint-strength or collapse simulation.', 'Native source coverage, broken fragments, appearance and temporal identities remain separate.']))


if __name__ == '__main__': main()

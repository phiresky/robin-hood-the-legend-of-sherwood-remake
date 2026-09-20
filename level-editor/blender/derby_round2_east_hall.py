"""Reconstruct East Hall's hanging southeast turret from its authored silhouette.

The roof and drum are bounded render surfaces, not ground-height volumes. Four
canonical parts remain selectable; concealed depth stays a conservative circular
continuation of the visible drum. Call refine(), then refresh projection layers.
"""
import math

import bmesh
import bpy
from mathutils import Vector

TAG = 'east-hall-hanging-turret-round2-v1'
SEGMENTS = 72


def _replace(obj, vertices, faces):
    mesh = bpy.data.meshes.new(obj.name + ' / bounded round turret')
    inverse = obj.matrix_world.inverted()
    mesh.from_pydata([inverse @ Vector(v) for v in vertices], [], faces)
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    invalid = sum(not e.is_manifold for e in bm.edges)
    degenerate = sum(f.calc_area() < 1e-8 for f in bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    if invalid or degenerate:
        raise ValueError(f'{obj.name}: {invalid} open edges, {degenerate} degenerate faces')
    material = bpy.data.materials.get('East Hall / unobserved turret')
    if material is None:
        material = bpy.data.materials.new('East Hall / unobserved turret')
        material.diffuse_color = (.24, .24, .24, 1)
    mesh.materials.append(material)
    uv = mesh.uv_layers.new(name='Turret fallback')
    for loop in uv.data:
        loop.uv = (.5, .5)
    old_bottom = min((obj.matrix_world @ v.co).z for v in obj.data.vertices)
    obj.data = mesh
    obj['east_hall_round2'] = TAG
    obj['projection_min_cosine'] = .15
    return dict(source_node=obj['source_node'], old_bottom=old_bottom,
                new_bottom=min(v[2] for v in vertices), faces=len(faces),
                nonmanifold_edges=invalid, degenerate_faces=degenerate)


def refine():
    working = bpy.data.collections['Derby Working']
    objects = {}
    for number in range(207, 211):
        node = f'building-{number:03d}'
        matches = [o for o in working.all_objects if o.type == 'MESH'
                   and o.get('source_node') == node and not o.hide_render]
        if len(matches) != 1:
            raise ValueError(f'Expected one visible {node}, found {len(matches)}')
        objects[number] = matches[0]
    if all(o.get('east_hall_round2') == TAG for o in objects.values()):
        return {'status': 'already-refined'}
    # The source silhouette bounds the drum at x1570..1636 and the roof at
    # x1568..1645. The final tapered stone courses terminate around source y1035.
    vertices, faces = [], []
    profile = [(431, 32, 1603), (360, 32, 1603),
               (356, 34, 1603), (351, 34, 1603), (347, 31, 1603),
               (333, 31, 1604), (315, 29, 1606), (302, 24, 1605),
               (290, 16, 1600),
               (283, 8, 1594)]
    for z, radius, cx in profile:
        vertices.extend((cx + radius * math.cos(i * 2 * math.pi / SEGMENTS),
                         -2199 + radius * math.sin(i * 2 * math.pi / SEGMENTS), z)
                        for i in range(SEGMENTS))
    for ring in range(len(profile) - 1):
        for i in range(SEGMENTS):
            j = (i + 1) % SEGMENTS
            a, b = ring * SEGMENTS, (ring + 1) * SEGMENTS
            faces.append((a+i, a+j, b+j, b+i))
    faces.extend([tuple(reversed(range(SEGMENTS))),
                  tuple(range((len(profile)-1)*SEGMENTS, len(profile)*SEGMENTS))])
    changes = [_replace(objects[207], vertices, faces)]
    # Radius contracts faster toward the tip; the upper narrow metal finial is
    # present in the silhouette and was absent from the low polygon volume.
    roof_profile = [(431, 38.5), (438, 36), (448, 29), (460, 23),
                    (474, 17), (489, 11), (500, 6), (510, 3),
                    (532, 1.2), (538, 2), (542, .7), (555, .35)]
    slices = SEGMENTS // 3
    for sector, number in enumerate((208, 209, 210)):
        vertices, faces = [], []
        for inside in (False, True):
            for z, radius in roof_profile:
                cx = 1606.5 + (z-431) / (555-431) * 4.5
                for i in range(slices+1):
                    angle = (sector*slices+i)*2*math.pi/SEGMENTS
                    vertices.append((cx+radius*math.cos(angle),
                                     -2201+radius*math.sin(angle), z-(1 if inside else 0)))
        layer = len(roof_profile)*(slices+1)
        for ring in range(len(roof_profile)-1):
            for i in range(slices):
                a=ring*(slices+1)+i
                faces.extend([(a,a+1,a+slices+2,a+slices+1),
                              (a+layer+slices+1,a+layer+slices+2,a+layer+1,a+layer)])
        for ring in (0,len(roof_profile)-1):
            for i in range(slices):
                a=ring*(slices+1)+i
                faces.append((a,a+layer,a+layer+1,a+1))
        for i in (0,slices):
            for ring in range(len(roof_profile)-1):
                a=ring*(slices+1)+i
                faces.append((a,a+slices+1,a+slices+1+layer,a+layer))
        changes.append(_replace(objects[number],vertices,faces))
    bpy.context.view_layer.update()
    return {'status':'refined','changes':changes,'radial_segments':SEGMENTS,
            'evidence':'Authored mask 150 and covered source image',
            'limitations':['Concealed depth is inferred; source silhouette is not a depth map.']}

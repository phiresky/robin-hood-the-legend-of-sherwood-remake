"""Restrained bowed barrel and shallow inset head for the northwest cottage yard."""
import math

import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-lower-northwest-cottage-barrel'
NODE = 'building-062'
TAG = 'northwest-barrel-round2-v1'


def profile():
    """A continuous stave envelope with two modest integrated binding hoops."""
    levels = {i * 13 / 16 for i in range(17)}
    bands = ((3.1, 4.0), (9.2, 10.1))
    for low, high in bands:
        levels.update((low - .18, low, high, high + .18))
    result = []
    for z in sorted(levels):
        t = z / 13
        radius = 6.6 * (.82 + .05 * t + .155 * math.sin(math.pi * t))
        radius += .14 * max((max(0., min(1., (z-low+.18)/.18,
                                                (high+.18-z)/.18))
                           for low, high in bands), default=0.)
        result.append((z, radius))
    # The tiny painted top establishes a rim and recess, not a deep cavity.
    result.extend(((13.12, 5.65), (13.12, 5.1), (12.85, 4.98), (12.05, 4.98)))
    return result


def refine(collection_name='Derby Working'):
    owned = [o for o in bpy.data.collections[collection_name].objects
             if o.type == 'MESH' and o.get('source_node') == NODE
             and o.get('asset_group') == ASSET and not o.hide_render]
    if len(owned) != 1:
        raise RuntimeError(f'Expected one active {ASSET} mesh, found {len(owned)}')
    obj = owned[0]
    center = Vector((499.2, -3238., 0.))
    inverse = obj.matrix_world.inverted()
    count = 48
    rings = profile()
    vertices = [inverse @ (center + Vector((radius * math.cos(i * math.tau/count),
                                            radius * math.sin(i * math.tau/count), z)))
                for z, radius in rings for i in range(count)]
    faces = [tuple(reversed(range(count)))]
    for row in range(len(rings)-1):
        for i in range(count):
            j = (i+1) % count
            faces.append((row*count+i, row*count+j,
                          (row+1)*count+j, (row+1)*count+i))
    faces.append(tuple((len(rings)-1)*count+i for i in range(count)))
    mesh = bpy.data.meshes.new(f'{ASSET} / bowed staves, hoops and recessed head')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges = sum(not edge.is_manifold for edge in bm.edges)
    bad_faces = sum(face.calc_area() < 1e-8 for face in bm.faces)
    if bad_edges or bad_faces:
        bm.free()
        bpy.data.meshes.remove(mesh)
        raise RuntimeError(f'Barrel validation failed: {bad_edges} edges, {bad_faces} faces')
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bm.to_mesh(mesh)
    bm.free()
    material = bpy.data.materials.new(f'{ASSET} / pending source reprojection')
    material.diffuse_color = (.25, .25, .25, 1.)
    mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    obj.data = mesh
    obj['refinement_recipe'] = TAG
    obj['part_name'] = 'Bowed yard barrel with binding hoops and recessed head'
    obj['refinement_top_interpretation'] = 'Shallow recessed head; lid versus open top unresolved in source'
    obj['todo'] = 'Reapply trusted source projection; unseen surfaces remain unobserved'
    bpy.context.view_layer.update()
    return {'asset': ASSET, 'source_node': NODE, 'object': obj.name,
            'vertices': len(mesh.vertices), 'faces': len(mesh.polygons),
            'nonmanifold_edges': bad_edges, 'degenerate_faces': bad_faces,
            'world_bounds': [[min((obj.matrix_world @ v.co)[axis] for v in mesh.vertices)
                              for axis in range(3)],
                             [max((obj.matrix_world @ v.co)[axis] for v in mesh.vertices)
                              for axis in range(3)]]}

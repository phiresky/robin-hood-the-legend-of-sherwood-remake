"""Source-supported wash-tub rim and binding relief, with an open interior."""
import math

import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-lower-northwest-wash-tub'
NODE = 'building-064'
TAG = 'northwest-tub-round2-v1'


def profile():
    """Return height/ellipse-scale pairs around a closed vessel cross section."""
    levels = {0., 5.6, 10.65}
    bands = ((1.35, 2.35), (8.25, 9.25))
    bevel = .16
    for low, high in bands:
        levels.update((low-bevel, low, high, high+bevel))
    result = []
    for z in sorted(levels):
        scale = .77 + .23*z/11.2
        # The painted bands are narrow bindings, not oversized separate rings.
        relief = .012 * max(max(0., min(1., (z-low+bevel)/bevel,
                                       (high+bevel-z)/bevel))
                            for low, high in bands)
        result.append((z, scale+relief))
    # Keep the established maximum height and footprint. Round the lip locally
    # while retaining the inner wall and floor; the source does not show water.
    result.extend(((11., .999), (11.2, .985), (11.2, .925),
                   (11., .906), (10.65, .9), (2.35, .70), (2., .69)))
    return result


def refine(collection_name='Derby Working'):
    owned = [obj for obj in bpy.data.collections[collection_name].objects
             if obj.type == 'MESH' and obj.get('source_node') == NODE
             and obj.get('asset_group') == ASSET and not obj.hide_render]
    if len(owned) != 1:
        raise RuntimeError(f'Expected one active {ASSET} mesh; found {len(owned)}')
    obj = owned[0]
    center = Vector((449.7, -3273.4, 0.))
    inverse = obj.matrix_world.inverted()
    count = 48
    rings = profile()
    vertices = [inverse @ (center + Vector((16*scale*math.cos(i*math.tau/count),
                                            19*scale*math.sin(i*math.tau/count), z)))
                for z, scale in rings for i in range(count)]
    faces = [tuple(reversed(range(count)))]
    for row in range(len(rings)-1):
        for i in range(count):
            j = (i+1) % count
            faces.append((row*count+i, row*count+j,
                          (row+1)*count+j, (row+1)*count+i))
    faces.append(tuple((len(rings)-1)*count+i for i in range(count)))
    mesh = bpy.data.meshes.new(f'{ASSET} / rounded rim and restrained binding hoops')
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
        raise RuntimeError(f'Tub validation failed: {bad_edges} edges, {bad_faces} faces')
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bm.to_mesh(mesh)
    bm.free()
    material = bpy.data.materials.new(f'{ASSET} / pending source reprojection')
    material.diffuse_color = (.25, .25, .25, 1.)
    mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    obj.data = mesh
    obj['refinement_recipe'] = TAG
    obj['part_name'] = 'Wooden wash tub with rounded rim and binding hoops'
    obj['refinement_interior_interpretation'] = 'Open dark interior; depth uncertain and no water inferred'
    obj['todo'] = 'Reapply trusted source projection; concealed surfaces remain unobserved'
    bpy.context.view_layer.update()
    return {'asset': ASSET, 'source_node': NODE, 'object': obj.name,
            'vertices': len(mesh.vertices), 'faces': len(mesh.polygons),
            'nonmanifold_edges': bad_edges, 'degenerate_faces': bad_faces,
            'world_bounds': [[min((obj.matrix_world @ v.co)[axis] for v in mesh.vertices)
                              for axis in range(3)],
                             [max((obj.matrix_world @ v.co)[axis] for v in mesh.vertices)
                              for axis in range(3)]],
            'profile': rings}

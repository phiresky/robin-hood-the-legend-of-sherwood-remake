"""Source-silhouette fitted round body, hoops and lid for the east yard barrel."""
import math
import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-lower-east-cottage-barrel'


def refine():
    candidates = [o for o in bpy.data.collections['Derby Working'].all_objects
                  if o.type == 'MESH' and not o.hide_render
                  and o.get('source_node') == 'building-051']
    if len(candidates) != 1:
        raise ValueError(f'Expected one visible barrel, found {len(candidates)}')
    obj = candidates[0]
    # Circular horizontal sections keep hidden views physically coherent. The
    # base-state silhouette and lid ellipse constrain these modest dimensions.
    center_x, ground_pixel_y = 922.6, 2016.3
    center_y = -ground_pixel_y / math.sin(math.radians(35))
    profile = [(0, 8.4), (.6, 8.7), (2, 9.1), (4, 9.8),
               (7, 10.6), (8.5, 10.9), (8.65, 11.15),
               (9.3, 11.25), (9.45, 11.02), (12, 11.45),
               (16, 11.75), (20, 11.55), (22.3, 11.18),
               (22.45, 11.42), (23.1, 11.3), (23.25, 11.04),
               (26, 10.3), (29, 9.55), (31.2, 9.05),
               (31.6, 8.95), (31.6, 8.4), (31.22, 8.4)]
    count = 64
    inverse = obj.matrix_world.inverted()
    vertices = [inverse @ Vector((center_x + radius * math.cos(i*math.tau/count),
                                  center_y + radius * math.sin(i*math.tau/count), z))
                for z, radius in profile for i in range(count)]
    faces = [tuple(reversed(range(count)))]
    for ring in range(len(profile)-1):
        for i in range(count):
            j = (i+1) % count
            faces.append((ring*count+i, ring*count+j,
                          (ring+1)*count+j, (ring+1)*count+i))
    faces.append(tuple(range((len(profile)-1)*count, len(profile)*count)))
    mesh = bpy.data.meshes.new('East cottage barrel / rounded staves and restrained hoops')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    # Fresh source projection is applied by the worker pipeline after geometry.
    material = bpy.data.materials.get('East cottage barrel / neutral evidence')
    if material is None:
        material = bpy.data.materials.new('East cottage barrel / neutral evidence')
        material.diffuse_color = (.34, .34, .34, 1)
    mesh.materials.append(material)
    mesh.uv_layers.new(name='Neutral fallback')
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    remaining = set(bm.verts)
    shells = 0
    while remaining:
        shells += 1
        queue = [remaining.pop()]
        while queue:
            current = queue.pop()
            for edge in current.link_edges:
                other = edge.other_vert(current)
                if other in remaining:
                    remaining.remove(other)
                    queue.append(other)
    validation = {'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
                  'degenerate_faces': sum(f.calc_area() < 1e-7 for f in bm.faces),
                  'connected_shells': shells, 'signed_volume': bm.calc_volume(signed=True),
                  'ground_z': 0.0}
    if validation['nonmanifold_edges'] or validation['degenerate_faces']:
        raise ValueError(validation)
    bm.to_mesh(mesh)
    bm.free()
    obj.data = mesh
    obj['round2_east_cottage_barrel'] = 'round-profile-hoops-recessed-cap-v1'
    obj['round2_mask_evidence'] = 'base layer 0, native mask 2, visually reviewed'
    return {'asset': ASSET, 'source_node': 'building-051',
            'vertices': len(mesh.vertices), 'faces': len(mesh.polygons),
            'validation': validation}

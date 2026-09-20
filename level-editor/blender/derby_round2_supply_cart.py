"""Coherent cart timbers and rounded cargo, awaiting source-view validation."""
import math

import bpy
import bmesh
from mathutils import Vector

from derby_asset_lower_east_supplies import Geometry

TAG = 'round2_supply_cart'
NODE = 'building-075'


def _replace(obj, geometry):
    mesh = bpy.data.meshes.new(obj.name + ' round2')
    mesh.from_pydata(geometry.vertices, [], geometry.faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    defects = {'nonmanifold': sum(not e.is_manifold for e in bm.edges),
               'degenerate': sum(f.calc_area() < 1e-7 for f in bm.faces)}
    bm.to_mesh(mesh)
    bm.free()
    if any(defects.values()):
        raise ValueError((obj.name, defects))
    # Geometry uses world coordinates; retain the existing parent and transform.
    mesh.transform(obj.matrix_world.inverted())
    material = bpy.data.materials.get('Derby cart source-review neutral')
    if material is None:
        material = bpy.data.materials.new('Derby cart source-review neutral')
        material.diffuse_color = (.35, .35, .35, 1)
    mesh.materials.append(material)
    mesh.uv_layers.new(name='Cart neutral fallback')
    obj.data = mesh
    obj[TAG] = True
    return {'name': obj.name, 'vertices': len(mesh.vertices),
            'faces': len(mesh.polygons), **defects}


def refine():
    """Replace only node 75 components; reproject through the worker afterward.

    The visible slatted end follows the cart frame, not individual mask pixels.
    Hidden wheel placement and shared axle remain conservative symmetry inferences.
    """
    collection = bpy.data.collections['Derby Working']
    owned = [o for o in collection.objects
             if o.type == 'MESH' and o.get('source_node') == NODE]
    current = [o for o in owned if not o.hide_render]
    if any(o.get(TAG) for o in current):
        raise ValueError('Start from the worker baseline before reapplying recipe')
    if len(current) != 4:
        raise ValueError('Expected exactly four inherited cart components')
    originals = [o for o in owned if o.hide_render and
                 not o.get('lower_east_supply_cart_refinement') and
                 len(o.data.vertices) >= 20]
    if len(originals) != 1:
        raise ValueError('Cannot uniquely recover original cart frame basis')
    source = originals[0]
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    back, front = (world[17]+world[18])/2, (world[16]+world[19])/2
    u = front-back
    u.z = 0
    u.normalize()
    v = world[18]-world[17]
    v.z = 0
    width = v.length
    v.normalize()
    up = Vector((0, 0, 1))
    pitched = (front-back).normalized()
    length = (front-back).length

    def point(s, t=0, dz=0):
        p = back.lerp(front, s)+v*t
        p.z += dz
        return p

    body = Geometry()
    for i in range(6):
        body.box(point((i+.5)/6, dz=-25), pitched*length/12,
                 v*(width/2-1), up*.8)
    for side in (-1, 1):
        for row in range(3):
            body.box(point(.5, side*(width/2-1), -21+row*8),
                     pitched*length/2, v*.8, up*3.6)
        for s in (.06, .5, .94):
            body.box(point(s, side*(width/2+.1), -10),
                     u*1.1, v*1.1, up*14)
    visible_end = 0 if back.x < front.x else 1
    for end in (0, 1):
        if end != visible_end:
            for row in range(3):
                body.box(point(end, dz=-21+row*8), u*.8,
                         v*(width/2-1), up*3.6)
            continue
        # Straight upper/lower rails and six upright members. Slat count is
        # approximate at native resolution; preserve the observed narrow gaps.
        for dz in (-24, -2):
            body.box(point(end, dz=dz), u*1, v*(width/2-1), up*1)
        for i in range(6):
            t = (i/5-.5)*(width-4)
            body.box(point(end, t, -13), u*.8,
                     v*((width-4)/5*.43), up*10.5)

    axle = point(.39)
    axle.z = 19
    body.box(axle, u*1.6, v*(width/2+3), up*1.6)
    objects = {}
    for label in ('Plank cargo body', 'Near spoked wheel', 'Far spoked wheel',
                  'Barrel cargo'):
        matches = [o for o in current if o.name.endswith(label)]
        if len(matches) != 1:
            raise ValueError(('Unexpected component names', label))
        objects[label] = matches[0]
    report = [_replace(objects['Plank cargo body'], body)]
    for side, label in ((1, 'Near'), (-1, 'Far')):
        wheel = Geometry()
        center = axle+v*side*(width/2+2)
        wheel.ring(center, u, up, v, 20, 17, 2.5, count=64)
        for i in range(12):
            phase = math.radians(16.75)+i*math.tau/12
            axis = u*math.cos(phase)+up*math.sin(phase)
            cross = v.cross(axis).normalized()
            wheel.box(center+axis*9, axis*8, cross*1.3, v*.8)
        wheel.box(center, u*2.2, up*2.2, v*2.1)
        report.append(_replace(objects[label+' spoked wheel'], wheel))

    # The inherited barrel base intersects the inclined deck. Preserve the
    # visible shoulder and lid; shorten only the hidden foot and seat it on a
    # tapered timber block. This concealed support is a structural inference.
    barrel = Geometry()
    center = Vector((992, -3198, 0))
    horizontal = front-back
    horizontal.z = 0
    along = (center-back).dot(horizontal)/horizontal.length_squared
    deck_center = back.lerp(front, along).z-24.2
    deck_slope = (front.z-back.z)/horizontal.length
    base_z = deck_center+abs(deck_slope)*8+.05
    if not 25 < base_z < 27:
        raise ValueError(('Unexpected barrel seating height', base_z))
    wedge = []
    for top in (False, True):
        for t in (-7, 7):
            for s in (-7, 7):
                p = center+u*s+v*t
                p.z = base_z if top else deck_center+deck_slope*s-.05
                wedge.append(p)
    barrel.add(wedge, [(0,2,3,1),(4,5,7,6),(0,1,5,4),
                       (2,6,7,3),(0,4,6,2),(1,3,7,5)])
    profile = ((base_z, 8), (27, 9.5), (36, 10), (45, 9.5), (49, 8))
    # Smooth the observed shoulder transitions using bounded cubic interpolation.
    slopes = []
    for i, (z, radius) in enumerate(profile):
        a, b = profile[max(0, i-1)], profile[min(len(profile)-1, i+1)]
        slopes.append((b[1]-a[1])/(b[0]-a[0]))
    rings = []
    for row in range(len(profile)-1):
        z0, r0 = profile[row]
        z1, r1 = profile[row+1]
        for step in range(4):
            t = step/4
            radius = ((2*t**3-3*t*t+1)*r0 + (t**3-2*t*t+t)*(z1-z0)*slopes[row]
                      + (-2*t**3+3*t*t)*r1 + (t**3-t*t)*(z1-z0)*slopes[row+1])
            rings.append((z0+(z1-z0)*t, max(min(r0,r1), min(max(r0,r1), radius))))
    rings.append(profile[-1])
    count = 64
    verts = []
    for z, r in rings:
        verts.extend(center+Vector((r*math.cos(i*math.tau/count),
                                    r*math.sin(i*math.tau/count), z))
                     for i in range(count))
    faces = [tuple(reversed(range(count))),
             tuple(range((len(rings)-1)*count, len(rings)*count))]
    for row in range(len(rings)-1):
        for i in range(count):
            j = (i+1) % count
            faces.append((row*count+i, row*count+j,
                          (row+1)*count+j, (row+1)*count+i))
    barrel.add(verts, faces)
    for z, r in ((28, 9.7), (43, 9.9)):
        barrel.ring(center+up*z, Vector((1, 0, 0)), Vector((0, 1, 0)),
                    up, r+.4, r-.2, 1.5, count=count)
    report.append(_replace(objects['Barrel cargo'], barrel))
    return {'components': report, 'slatted_end': visible_end,
            'frame_back': list(back), 'frame_front': list(front),
            'width': width, 'axle': list(axle),
            'barrel_base_z': base_z, 'deck_center_z': deck_center,
            'spokes': {'count': 12, 'phase_degrees': 16.75, 'half_width': 1.3},
            'pending': 'Validate native mask authority and far-wheel visibility'}

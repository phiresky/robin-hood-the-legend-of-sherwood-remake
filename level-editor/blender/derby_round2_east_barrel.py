"""Simple source-aligned lying cask, preserving its reviewed native silhouette."""
import math

import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-east-bailey-crate'


def refine():
    """Use a simple cask, with the head proportions anchored to the source art."""
    tag = 'east-bailey-simple-cask-r3'
    objects = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and not o.hide_render
               and o.get('source_node') == 'building-112']
    if len(objects) != 3 or any(o.get('asset_group') != ASSET for o in objects):
        raise ValueError('Expected owned barrel body and two hoops')
    if all(o.get('round2_east_barrel') == tag for o in objects):
        return {'asset': ASSET, 'status': 'already-refined'}
    body = next(o for o in objects if 'Bowed timber' in o.name)
    hoops = sorted([o for o in objects if o != body], key=lambda o: o.name)
    length, radius = 16.9859419168505, 11.343824807749744
    front_x, front_y, yaw = 1499.7860709859592, 1333.4975773274105, -0.5806568694363229
    axis = Vector((math.cos(yaw), math.sin(yaw), 0))
    side = Vector((-axis.y, axis.x, 0))
    elevation = math.radians(35)
    center = Vector((front_x, (-front_y-math.cos(elevation)*radius*1.015)
                     / math.sin(elevation), radius*1.015))
    profiles = [[(0,.93),(.2,.99),(.5,1),(.8,.99),(1,.93)]]
    profiles += [[(t-.025,.98),(t+.025,.98),(t+.025,1.015),(t-.025,1.015)]
                 for t in (.2,.8)]
    report = []
    for obj, profile in zip([body]+hoops, profiles):
        vertices = []
        inverse = obj.matrix_world.inverted()
        for t, r in profile:
            for i in range(16):
                angle = i*math.pi/8
                p = center + axis*((t-1)*length) + side*(radius*r*math.cos(angle))
                p.z += radius*r*math.sin(angle)
                vertices.append(inverse @ p)
        faces = [(j*16+i,j*16+(i+1)%16,(j+1)*16+(i+1)%16,(j+1)*16+i)
                 for j in range(len(profile)-1) for i in range(16)]
        if obj == body:
            faces += [tuple(reversed(range(16))), tuple(range(len(vertices)-16,len(vertices)))]
        else:
            faces += [((len(profile)-1)*16+i,(len(profile)-1)*16+(i+1)%16,(i+1)%16,i)
                      for i in range(16)]
        mesh = bpy.data.meshes.new(obj.name+' simplified')
        mesh.from_pydata(vertices, [], faces)
        mesh.materials.append(obj.data.materials[0])
        uv = mesh.uv_layers.new(name='UVMap')
        for item in uv.data:
            item.uv = (.5,.5)
        mesh.attributes.new('reprojection_fallback_material','INT','FACE')
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        defects = {'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
                   'degenerate_faces':sum(f.calc_area()<1e-8 for f in bm.faces)}
        if any(defects.values()):
            bm.free()
            raise ValueError(defects)
        bm.to_mesh(mesh)
        bm.free()
        obj.data = mesh
        obj['round2_east_barrel'] = tag
        report.append({'object':obj.name,'vertices':len(vertices),'faces':len(faces),**defects})
    bpy.context.view_layer.update()
    return {'asset':ASSET,'status':'refined','components':report,
            'profile':'16 radial sides; five body rings and flat heads; two simple closed hoops',
            'length':length,'radius':radius,'axis_degrees':math.degrees(yaw),
            'source_head_center':[front_x,front_y],'reviewed_mask':96}

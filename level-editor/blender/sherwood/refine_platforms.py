"""Partition three measured tree platforms into closed radial timber planks.

Retains the exported footprint, height and access cutouts. Board divisions are
an initial reconstruction of the radial arrangement visible in the Day map.
TODO: trace each irregular original board end and the underside timber joints.
"""

import math

import bmesh
import bpy
from mathutils import Vector

SIN = math.sin(math.radians(35))
COS = math.cos(math.radians(35))
NAME = '05 Detail pass - radial platform boards'
if NAME in bpy.data.collections:
    raise RuntimeError('Platform pass already exists')
collection = bpy.data.collections.new(NAME)
bpy.context.scene.collection.children.link(collection)
work = bpy.data.collections['01 Refinement - working copy']
material = bpy.data.materials['Sherwood measured Day projection']


def clip(polygon, center, angle, sign):
    direction = Vector((math.cos(angle),math.sin(angle),0))
    def distance(p):
        d = p-center
        return sign*(direction.x*d.y-direction.y*d.x)
    output = []
    for a,b in zip(polygon,polygon[1:]+polygon[:1]):
        da,db=distance(a),distance(b)
        if da>=-1e-7:
            output.append(a)
        if (da>0 and db<0) or (da<0 and db>0):
            output.append(a.lerp(b,da/(da-db)))
    return output


counts = {}
for index,cx,cy,count,thickness in [(86,425,409,68,5.7),(88,985,635,76,3.6),(89,193,485,80,3.6)]:
    source = next(o for o in work.objects if o.get('source_obstacle')==f'building-{index:03}')
    triangles = [[source.matrix_world@source.data.vertices[i].co for i in p.vertices]
                 for p in source.data.polygons if (source.matrix_world.to_3x3()@p.normal).z>0.8]
    if not triangles:
        raise RuntimeError(f'No upward faces in {source.name}')
    center = Vector((cx,-cy/SIN,0))
    made = 0
    for plank in range(count):
        start=(plank+0.025)*math.tau/count
        end=(plank+0.975)*math.tau/count
        bm=bmesh.new()
        for triangle in triangles:
            polygon=clip(clip(triangle,center,start,1),center,end,-1)
            if len(polygon)<3:
                continue
            verts=[bm.verts.new(p) for p in polygon]
            bm.faces.new(verts)
        if not bm.faces:
            bm.free()
            continue
        bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=0.001)
        bmesh.ops.dissolve_degenerate(bm,edges=list(bm.edges),dist=0.0001)
        if not bm.faces:
            bm.free()
            continue
        top_vertices=list(bm.verts)
        top_faces=list(bm.faces)
        boundaries=[e for e in bm.edges if e.is_boundary]
        bottom={v:bm.verts.new(v.co-Vector((0,0,thickness))) for v in top_vertices}
        for face in top_faces:
            bm.faces.new([bottom[v] for v in reversed(list(face.verts))])
        for edge in boundaries:
            a,b=edge.verts
            bm.faces.new((a,b,bottom[b],bottom[a]))
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        mesh=bpy.data.meshes.new(f'Platform {index:03} board {plank+1:02}')
        bm.to_mesh(mesh)
        bm.free()
        mesh.update()
        obj=bpy.data.objects.new(mesh.name,mesh)
        collection.objects.link(obj)
        mesh.materials.append(material)
        uv=mesh.uv_layers.new(name='Original map projection')
        top_z=max(v.co.z for v in mesh.vertices)
        for polygon in mesh.polygons:
            for loop_index in polygon.loop_indices:
                x,y,z=mesh.vertices[mesh.loops[loop_index].vertex_index].co
                sample_z=top_z if polygon.normal.z < -0.8 else z
                uv.data[loop_index].uv=(x/1920,1-(-y*SIN-sample_z*COS)/1088)
        obj['source_obstacle']=f'building-{index:03}'
        obj['inferred']='Individual radial board divisions; original platform outline retained'
        made+=1
    source.hide_render=True
    source.hide_set(True)
    source['replaced_by']=NAME
    counts[index]=made
bpy.context.view_layer.update()
result={'boards':counts,'collection':NAME}

"""Source-supported closed backing for the North Tower spire doorway."""
import math
import bpy,bmesh
from mathutils import Matrix

def refine():
    collection=bpy.data.collections['Derby Working']
    if any(o.get('north_spire_door_backing') for o in collection.objects):return {'status':'already-applied'}
    source=next(o for o in collection.objects if o.get('source_node')=='building-173' and not o.hide_render)
    # Door sits behind the existing arched opening. Depth is conservative inference;
    # width and crown follow the existing opening, not individual image texels.
    outline=[(-12,835),(12,835),(12,875)]
    outline.extend((12*math.cos(i*math.pi/16),875+12*math.sin(i*math.pi/16)) for i in range(1,17))
    n=len(outline);vertices=[(1003.1+x,y,z) for y in (-1569.9,-1568.4) for x,z in outline]
    faces=[tuple(range(n-1,-1,-1)),tuple(range(n,2*n))]
    faces.extend((i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n))
    mesh=bpy.data.meshes.new('North spire closed wooden door');mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free()
    obj=bpy.data.objects.new('Great Keep / North spire / recessed wooden door',mesh);collection.objects.link(obj)
    obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for k in ('source_node','source_obstacle','asset_group','asset_name'):obj[k]=source[k]
    obj['projection_component']='north-spire-wooden-door';obj['north_spire_door_backing']=True;obj['part_name']='North spire recessed wooden door'
    return {'changed_nodes':['building-173'],'new_component':obj.name,'faces':len(faces),'depth_inferred':True,'source_supported':'Closed wooden arched door visible within spire entrance','retained_geometry_unchanged':True}

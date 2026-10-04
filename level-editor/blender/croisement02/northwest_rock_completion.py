"""Complete northwest rock masses from native footprints and source crests."""
import bpy,bmesh
from mathutils import Vector,noise
from mathutils.bvhtree import BVHTree
from tree_geometry import replace_mesh,SIN,COS,RAY


def prism(original,name,points,top,bottom,radius=7):
    obj=original.copy();obj.data=original.data.copy();obj.name=name
    bpy.data.collections['Croisement02 Working'].objects.link(obj)
    vertices=[(x,-y/SIN,(bottom(x,y) if layer==0 else top(x,y) if callable(top) else top)/COS) for layer in [0,1] for x,y in points]
    n=len(points);faces=[tuple(reversed(range(n))),tuple(range(n,n*2))]
    faces.extend((i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n))
    replace_mesh(obj,vertices,faces,materials=list(original.data.materials))
    bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
    bevel=obj.modifiers.new('Rounded inferred rock edges','BEVEL');bevel.width=radius;bevel.segments=4;bevel.affect='EDGES'
    bpy.ops.object.modifier_apply(modifier=bevel.name)
    return obj


def build(original):
    world=[original.matrix_world @ v.co for v in original.data.vertices]
    tree=BVHTree.FromPolygons(world,[tuple(f.vertices) for f in original.data.polygons])
    node=original['source_node'];pieces=[]
    if node=='building-035':
        # Continue the back footprint north at the retained native crest height.
        # Unlike a projected shell this has a full-height rounded rock body.
        northern=[(-65,55),(-30,40),(35,40),(85,48),(112,72),(110,105),(75,120),(-35,120),(-78,95),(-85,72)]
        rear=[(100,55),(129,64),(150,93),(143,121),(136,138),(123,140),(108,125),(100,100)]
        pieces=[prism(original,'Northwest cliff / Northern return',northern,lambda x,y:min(92.934006,101.30-.09*y),lambda x,y:0,radius=12),
                prism(original,'Northwest cliff / Observed rear stone',rear,92.934006,lambda x,y:max(0,y-48))]
        profile=[(-85,-53),(112,-21),(150,0),(143,28),(136,45),(123,48),(108,48)]
    elif node=='building-036':
        foot=[(105,178),(116,180),(127,195),(121,213),(106,214),(101,200)]
        pieces=[prism(original,'Northwest cliff / Traced middle stone foot',foot,66.001,lambda x,y:max(0,y-165))]
        profile=[(105,112),(116,114),(127,129),(121,147),(106,148),(101,134)]
    else:raise ValueError('Only the observed northwest corrections are supported')
    bpy.ops.object.select_all(action='DESELECT');original.select_set(True)
    for obj in pieces:obj.select_set(True)
    bpy.context.view_layer.objects.active=original;bpy.ops.object.join()
    original.data.remesh_voxel_size=1.4;original.data.use_remesh_preserve_volume=True;bpy.ops.object.voxel_remesh()
    bm=bmesh.new();bm.from_mesh(original.data)
    for _ in range(10):bmesh.ops.smooth_vert(bm,verts=list(bm.verts),factor=.4,use_axis_x=True,use_axis_y=True,use_axis_z=True)
    bm.normal_update();inverse=original.matrix_world.inverted();normal_matrix=original.matrix_world.to_3x3().inverted().transposed();restored=0
    for vertex in bm.verts:
        point=original.matrix_world @ vertex.co;x=point.x;y=-point.y*SIN-point.z*COS
        hit=tree.ray_cast(Vector((x,-y/SIN,0))+RAY*5000,-RAY)[0]
        preserve_region=(y>=20 and x<100) if node=='building-035' else x<99
        if preserve_region and hit is not None and abs(point.dot(RAY)-hit.dot(RAY))<4:
            point+=RAY*(hit.dot(RAY)-point.dot(RAY));restored+=1
        elif hit is None or y<0:
            normal=(normal_matrix @ vertex.normal).normalized();point+=normal*noise.noise_vector(point*.065+Vector((9,3,7))).x*.55
        vertex.co=inverse @ point
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(original.data);bm.free()
    result=replace_mesh(original,[tuple(original.matrix_world @ v.co) for v in original.data.vertices],[tuple(f.vertices) for f in original.data.polygons],materials=list(original.data.materials))
    for face in original.data.polygons:face.use_smooth=True
    if result['nonmanifold_edges'] or result['degenerate_faces']:raise ValueError('Cliff completion must be a closed solid')
    result.update(source_node=node,source_profile=profile,restored_front_vertices=restored,
        method='Rounded native-coordinate rock masses unioned into surveyed relief; original interior front depths restored while obsolete cut-edge bevels blend into the completion',
        inference='Northern full-height return and buried rear geometry are inferred. Rear stone and middle foot are bounded by observed source crests; native maximum height is retained.')
    return result

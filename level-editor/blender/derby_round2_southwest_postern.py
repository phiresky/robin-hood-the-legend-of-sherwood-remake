"""Restore the postern scaffold from authored timber silhouettes.

Screen anchors use the covered map's native coordinates. Mask 67 describes the
lower landing frame and mask 69 the taller side frame. Heights come from the
existing landing joins; silhouettes alone do not supply hidden depth.
"""
import math
import bpy
import bmesh
from mathutils import Vector, Matrix

ASSET = 'derby-southwest-postern'
TAG = 'postern-authored-scaffold-round2-v1'


def _refine_scaffold():
    collection = bpy.data.collections['Derby Working']
    owned = [o for o in collection.objects if o.type == 'MESH'
             and o.get('asset_group') == ASSET and not o.hide_render]
    if any(o.get('round2_postern') == TAG for o in owned):
        return {'reused': True}
    parts = {o['source_node']: o for o in owned}
    ladder = parts['building-039']
    if len(ladder.data.vertices) != 176 or len(ladder.data.polygons) != 132:
        raise ValueError('Lower ladder topology changed; re-audit before stripping old braces')
    # The first sixteen cuboids are the two rails and fourteen rungs. The last
    # six were inferred braces occupying the wrong screen-space silhouette.
    mesh = ladder.data.copy()
    ladder.data = mesh
    bm = bmesh.new(); bm.from_mesh(mesh); bm.verts.ensure_lookup_table()
    bmesh.ops.delete(bm, geom=list(bm.verts)[128:], context='VERTS')
    bm.to_mesh(mesh); bm.free(); mesh.update()
    report = {'reused': False, 'removed_misplaced_beams': 6, 'frames': []}
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    # Endpoints traced along the native mask's narrow timber strips. Keep both
    # frames open so original wall pixels cannot be copied across empty space.
    frames = [
        ('building-033', 'Lower landing scaffold', 67,
         (560.5, 2370, 600.5, 2359, 124.521), [
            ((560.5,2370),(560.5,2476),1.7),
            ((600.5,2359),(600.5,2470),1.7),
            ((562,2411),(600,2360),2.1),
            ((561.5,2418),(600.5,2406),1.8),
            ((560.5,2463),(600.5,2459),1.8),
         ]),
        ('building-034', 'Upper landing side scaffold', 69,
         (609.5,2311,628.5,2293,179.456), [
            ((609.5,2312),(609.5,2457),2.1),
            ((628.5,2293),(628.5,2441),1.5),
            ((611.5,2311),(628.5,2337),1.8),
            ((609.5,2360),(626.5,2336),1.8),
            ((609.5,2410),(626.5,2333),1.8),
            ((609.5,2405),(627.5,2382),1.8),
            ((610.5,2410),(628.5,2434),2.0),
            ((609.5,2450),(628.5,2431),2.0),
         ]),
    ]
    for node, label, mask, plane, lines in frames:
        source = parts[node]
        x0,y0,x1,y1,z0 = plane
        def lift(p):
            x,y = p
            join_y = y0+(x-x0)/(x1-x0)*(y1-y0)
            z = z0+(join_y-y)/cosine
            return Vector((x, -(join_y+z0*cosine)/sine, z))
        vertices, faces = [], []
        for a,b,width in lines:
            a,b = lift(a),lift(b)
            direction = (b-a).normalized()
            face_normal = Vector((-(y1-y0)/(x1-x0)/sine,-1,0)).normalized()
            side = direction.cross(face_normal).normalized()*width/2
            depth = direction.cross(side).normalized()*width/2
            start = len(vertices)
            vertices.extend(p+s*side+t*depth for p in (a,b)
                            for s,t in ((-1,-1),(1,-1),(1,1),(-1,1)))
            faces.extend(tuple(start+i for i in f) for f in
                         ((0,3,2,1),(4,5,6,7),(0,1,5,4),
                          (1,2,6,5),(2,3,7,6),(3,0,4,7)))
        mesh = bpy.data.meshes.new(label)
        mesh.from_pydata(vertices,[],faces); mesh.update()
        bm=bmesh.new(); bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bad={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
             'degenerate_faces':sum(f.calc_area()<1e-8 for f in bm.faces)}
        bm.to_mesh(mesh);bm.free()
        if any(bad.values()): raise ValueError(bad)
        uv=mesh.uv_layers.new(name='UVMap')
        for loop in mesh.loops:
            p=mesh.vertices[loop.vertex_index].co
            uv.data[loop.index].uv=(p.x/1920,1+(p.y*sine+p.z*cosine)/2752)
        if source.data.materials:
            mesh.materials.append(source.data.materials[0])
        obj=bpy.data.objects.new('Southwest Postern Tower / '+label,mesh)
        collection.objects.link(obj);obj.parent=source.parent
        obj.matrix_world=Matrix.Identity(4)
        for key in ('source_node','asset_group','asset_name','obstacle_index'):
            if key in source: obj[key]=source[key]
        obj['part_name']=label
        obj['round2_postern']=TAG
        obj['round2_component_role']=label
        obj['source_mask_index']=mask
        obj['source_mask_layer']=0
        report['frames'].append({'node':node,'mask':mask,'beams':len(lines),**bad})
    ladder['round2_postern']=TAG
    bpy.context.view_layer.update()
    return report


def _refine_shells():
    """Replace disconnected polygon strips with shared, closed shell topology.

    Anchors follow the existing visible top outline, not a proximity weld. The
    upper wood landing's exposed far corner is shortened to its authored edge.
    """
    tag = 'postern-coherent-shells-round2-v3'
    # World-space top perimeters audited before modification. Each perimeter
    # bounds one solid part; the main tower's ten-point path retains its hollow
    # center rather than capping the whole courtyard opening.
    outlines = {
        'building-009': (0.001, [
            (727.667,-4212.167,183.118),(594.442,-4182.421,183.118),
            (576.372,-4205.551,183.118),(578.803,-4219.701,183.118),
            (590.571,-4224.260,183.118),(704.784,-4257.688,183.118),
            (713.120,-4222.021,183.118)]),
        'building-026': (177.014, [
            (562.799,-4167.164,183.118),(539.914,-4183.560,183.118),
            (526.008,-4193.860,183.118),(559.115,-4196.068,183.118),
            (575.694,-4207.538,183.118),(576.522,-4206.420,183.118),
            (576.372,-4205.551,183.118),(591.555,-4186.117,183.118),
            (594.378,-4182.304,183.118)]),
        'building-033': (124.521, [
            (514.000,-4274.556,128.251),(494.312,-4221.163,128.251),
            (474.056,-4235.917,128.251),(498.507,-4298.772,128.251),
            (535.746,-4312.313,128.251),(565.227,-4315.092,128.251),
            (615.069,-4293.920,128.251),(591.406,-4273.512,128.251),
            (559.237,-4288.847,128.251)]),
        'building-034': (179.456, [
            (612.263,-4230.888,183.118),(591.737,-4274.016,183.118),
            (610.948,-4290.074,183.118),(636.0,0,183.118)]),
        'building-043': (0.001, [
            (519.270,-4265.814,182.639),(511.933,-4223.601,205.486),
            (496.655,-4228.250,197.777),(513.993,-4274.234,175.905),
            (536.650,-4281.428,155.0),(559.308,-4288.621,165.0),
            (591.776,-4273.488,201.464),
            (612.181,-4230.963,233.454),(601.931,-4227.472,232.230),
            (590.939,-4261.946,208.088),(556.182,-4282.980,165.0),
            (537.726,-4274.397,155.0)]),
    }
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    # Native mask 71 retains x=636 at y=2280. Unlike cropping to the mask, only
    # this visibly exposed corner moves; the occluded wall-side join survives.
    outlines['building-034'][1][-1] = (636.0, -(2280+183.118*cosine)/sine, 183.118)
    collection = bpy.data.collections['Derby Working']
    result=[]
    for node,(bottom,top) in outlines.items():
        matches=[o for o in collection.objects if o.type=='MESH'
                 and o.get('asset_group')==ASSET and o.get('source_node')==node
                 and not o.hide_render and not o.get('round2_component_role')]
        if len(matches)!=1:
            raise ValueError(f'Expected one active shell for {node}: {len(matches)}')
        obj=matches[0]
        if obj.get('round2_shell')==tag:
            result.append({'node':node,'reused':True});continue
        n=len(top)
        vertices=[Vector((x,y,bottom)) for x,y,z in top]+[Vector(p) for p in top]
        faces=[tuple(reversed(range(n))),tuple(range(n,n*2))]
        faces.extend((i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n))
        mesh=bpy.data.meshes.new(node+' coherent postern shell')
        inverse=obj.matrix_world.inverted()
        mesh.from_pydata([inverse@p for p in vertices],[],faces);mesh.update()
        bm=bmesh.new();bm.from_mesh(mesh)
        # The variable-height parapet cap is explicitly triangulated so no
        # renderer must guess a nonplanar ngon surface differently.
        bmesh.ops.triangulate(bm,faces=[f for f in bm.faces if len(f.verts)>4])
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        bad={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
             'degenerate_faces':sum(f.calc_area()<1e-8 for f in bm.faces)}
        if any(bad.values()):
            bm.free();raise ValueError({'node':node,**bad})
        bm.to_mesh(mesh);bm.free();mesh.update()
        uv=mesh.uv_layers.new(name='UVMap')
        for loop in mesh.loops:
            p=obj.matrix_world@mesh.vertices[loop.vertex_index].co
            uv.data[loop.index].uv=(p.x/1920,1+(p.y*sine+p.z*cosine)/2752)
        if obj.data.materials:mesh.materials.append(obj.data.materials[0])
        obj.data=mesh
        obj['round2_shell']=tag
        result.append({'node':node,'reused':False,'verts':len(vertices),**bad})
    bpy.context.view_layer.update()
    return result


def _refine_ladders():
    """Use straight parallel rails and the nine/six source rung bands."""
    tag='postern-nine-six-rung-ladders-v5'
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    # Each ladder is planar: one shared slope vector and one across vector.
    # Slight paint irregularity is not used to bend individual rails or rungs.
    specifications=[
        ('building-039',(567.5,2374),(17.0,-3.5),(13.5,103),122.0,0.0,
         [12.5,24,35,46.5,57.5,68,78.5,88.5,96],1.7),
        ('building-040',(595.0,2298),(9.5,5.5),(-11,52.5),183.118,128.183,
         [11,18,24.5,31.5,39,46],1.5),
    ]
    result=[]
    for node,start,across,delta,high,low,rungs,width in specifications:
        objects=[o for o in bpy.data.collections['Derby Working'].objects
                 if o.type=='MESH' and not o.hide_render and o.get('asset_group')==ASSET
                 and o.get('source_node')==node]
        if len(objects)!=1:raise ValueError(node)
        obj=objects[0]
        if obj.get('round2_ladder')==tag:
            result.append({'node':node,'reused':True});continue
        def point(t,side):
            x=start[0]+delta[0]*t+across[0]*side
            y=start[1]+delta[1]*t+across[1]*side
            z=high+(low-high)*t
            return Vector((x,-(y+z*cosine)/sine,z))
        normal=(point(1,0)-point(0,0)).cross(point(0,1)-point(0,0)).normalized()
        vertices=[];faces=[]
        def beam(a,b,w):
            direction=(b-a).normalized()
            side=direction.cross(normal).normalized()*w/2
            depth=normal*w/2
            offset=len(vertices)
            vertices.extend(p+s*side+t*depth for p in (a,b)
                            for s,t in ((-1,-1),(1,-1),(1,1),(-1,1)))
            faces.extend(tuple(offset+i for i in f) for f in
                         ((0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)))
        # The lower rail heads meet the underside of the landing at z124.44;
        # the 0.081-unit center gap is smaller than their timber half-width.
        # Moving the whole planar ladder in depth preserves its screen fit.
        head=-.02 if node=='building-039' else 0
        for side in [0,1]:beam(point(head,side),point(1,side),width)
        for height in rungs:
            t=height/delta[1]
            beam(point(t,0),point(t,1),1.35)
        mesh=bpy.data.meshes.new(node+' source-spaced straight timber ladder')
        inverse=obj.matrix_world.inverted()
        mesh.from_pydata([inverse@p for p in vertices],[],faces);mesh.update()
        bm=bmesh.new();bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        bad={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
             'degenerate_faces':sum(f.calc_area()<1e-8 for f in bm.faces)}
        bm.to_mesh(mesh);bm.free()
        if any(bad.values()):raise ValueError(bad)
        uv=mesh.uv_layers.new(name='UVMap')
        for loop in mesh.loops:
            p=obj.matrix_world@mesh.vertices[loop.vertex_index].co
            uv.data[loop.index].uv=(p.x/1920,1+(p.y*sine+p.z*cosine)/2752)
        if obj.data.materials:mesh.materials.append(obj.data.materials[0])
        obj.data=mesh;obj['round2_ladder']=tag;obj['rung_count']=len(rungs)
        result.append({'node':node,'rungs':len(rungs),**bad})
    return result


def refine():
    return {'scaffold':_refine_scaffold(),'shells':_refine_shells(),'ladders':_refine_ladders()}

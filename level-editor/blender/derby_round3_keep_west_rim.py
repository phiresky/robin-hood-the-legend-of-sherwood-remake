"""Revealed West Tower cut-wall crown, separate from its interior landing."""
import bpy
import bmesh
import math
from mathutils import Vector,Matrix

OUTER=[(416,793),(423,800),(441,806),(450,811),(458,827),(474,834),
       (489,833),(505,827),(516,817),(520,807),(510,798),(496,791),(500,778),(505,767)]
INNER=[(418,790),(426,797),(443,803),(454,808),(462,821),(475,827),
       (489,827),(502,820),(509,812),(512,807),(504,802),(491,794),(494,778),(501,766)]

def refine():
    working=bpy.data.collections['Derby Working']
    previous=[o for o in working.objects if o.get('west_cutwall_version')==1]
    if previous:return {'status':'already-applied','component':previous[0].name}
    floor=next(o for o in working.objects if o.type=='MESH' and not o.hide_render
               and o.get('source_node')=='building-239')
    if floor.get('west_closed_shell_version')!=1:raise ValueError('Closed landing checkpoint required')
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    vertices=[];faces=[]
    for i,(outer,inner) in enumerate(zip(OUTER,INNER)):
        top=324.25 if i<12 else (332.25 if i==12 else 344.25)
        a=Vector((outer[0],(-outer[1]-top*cosine)/sine,top))
        b=Vector((inner[0],(-inner[1]-top*cosine)/sine,top))
        vertices.extend([Vector((a.x,a.y,296.25)),a,b,Vector((b.x,b.y,296.25))])
    for i in range(len(OUTER)-1):
        for j in range(4):faces.append((4*i+j,4*(i+1)+j,4*(i+1)+(j+1)%4,4*i+(j+1)%4))
    faces.extend([(3,2,1,0),tuple(4*(len(OUTER)-1)+i for i in range(4))])
    mesh=bpy.data.meshes.new('West Tower / revealed curved cut-wall crown')
    mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    quality={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
             'degenerate_faces':sum(f.calc_area()<1e-8 for f in bm.faces),'volume':bm.calc_volume()}
    if quality['nonmanifold_edges'] or quality['degenerate_faces'] or quality['volume']<=0:raise ValueError(quality)
    bm.to_mesh(mesh);bm.free()
    neutral=bpy.data.materials.new('West Tower cut-wall / unknown source')
    neutral.diffuse_color=(.25,.25,.25,1);mesh.materials.append(neutral)
    obj=bpy.data.objects.new('Great Keep / West tower revealed cut-wall crown',mesh)
    working.objects.link(obj);obj.parent=floor.parent;obj.matrix_world=Matrix.Identity(4)
    for key in floor.keys():
        if not key.startswith('reprojection_'):obj[key]=floor[key]
    obj['projection_component']='west-revealed-cutwall'
    obj['west_cutwall_version']=1
    obj['patch_id']='patch-001'
    floor['projection_component']='west-revealed-floor'
    return {'changed_nodes':['building-239'],'new_component':obj.name,'quality':quality,
            'floor_top_preserved':304.25,'cutwall_top':324.25,'right_rise_top':344.25,
            'source_outer_trace':OUTER,'source_inner_trace':INNER,
            'inference':'Crown height20 above landing and right rise40 above landing inferred; exact visible crown edges traced in revealed source',
            'state':'patch001 interior receiver; existing cover and exterior unchanged'}

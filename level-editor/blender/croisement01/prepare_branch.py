"""Native mask 70 curved fallen branch construction; private review candidate."""
import sys
import argparse
import json
import math
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from prepare_props import ROOT, OUT, SIN, COS, prepare, modified, validate, acquire, release, sha

# Centers are measured in native source pixels; radii and elevation are inferred.
MAIN=[(1038,337,6,6),(1058,342,8,8),(1082,339,9,9),
      (1102,333,10,10),(1122,322,10,10),(1141,317,9,9),
      (1163,319,8,8),(1180,324,7,7),(1198,336,5,5),(1218,345,2,2)]
TWIG=[(1100,331,10,4),(1109,315,12,3.6),(1120,302,15,3),
      (1134,287,18,1.8)]


def tube(name,trace):
    centers=[Vector((x,-(y+z*COS)/SIN,z)) for x,y,z,r in trace]
    vertices=[];faces=[];n=16
    for i,center in enumerate(centers):
        tangent=(centers[min(i+1,len(centers)-1)]-centers[max(0,i-1)]).normalized()
        side=tangent.cross(Vector((0,0,1))).normalized();up=side.cross(tangent).normalized()
        for j in range(n):
            angle=2*math.pi*j/n
            vertices.append(center+trace[i][3]*(side*math.cos(angle)+up*math.sin(angle)))
    faces.append(tuple(reversed(range(n))))
    for i in range(len(centers)-1):
        for j in range(n):faces.append((i*n+j,i*n+(j+1)%n,(i+1)*n+(j+1)%n,(i+1)*n+j))
    faces.append(tuple(range((len(centers)-1)*n,len(centers)*n)))
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    obj=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(obj)
    return obj


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,choices=[1,2,3,4,5,6,7,8,9,10],default=1)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    main_trace=MAIN;twig_trace=TWIG;fork_trace=None
    if args.revision>=2:
        main_trace=[(1038,338,7,7),(1058,337,12,12),(1082,340,13,13),(1102,333,13,13),(1122,326,13,13),(1141,320,12,12),(1163,318,10,10),(1181,319,10,9),(1193,314,11,7),(1198,309,12,4)]
        fork_trace=[(1179,322,9,5),(1194,331,6,4),(1206,335,4,3),(1218,338,3,1.5)]
    if args.revision in (4,7,8,9,10):
        main_trace=[(1030,338,3,3),(1038,339,8,8),(1058,339,15,15),(1082,343,16,16),(1102,337,16,16),(1122,326,15,15),(1141,322,14,14),(1163,320,12,12),(1181,325,12,12),(1193,317,11,7),(1198,311,12,3)]
        twig_trace=[(1102,331,12,5),(1111,317,14,4.6),(1122,304,17,4),(1135,288,20,2.2)]
        fork_trace=[(1181,330,9,7),(1194,340,8,8),(1206,346,6,6),(1219,352,3,2)]
    if args.revision==7:
        fork_trace=[(1179,322,9,5),(1194,331,6,4),(1206,335,4,3),(1218,338,3,1.5)]
    if args.revision==9:
        fork_trace=[(1179,327,9,7),(1194,336,8,8),(1206,341,7,7),(1218,346,4,3)]
    if args.revision==10:
        main_trace=[(1030,338,3,3),(1038,339,8,8),(1058,339,15,15),(1082,343,16,16),(1102,337,16,16),(1122,326,15,15),(1141,322,14,14),(1163,320,12,12),(1181,322,12,12),(1198,335,10,10),(1218,345,6,5)]
        fork_trace=[(1183,321,12,7),(1194,316,15,6),(1199,310,17,3)]
    if args.revision>=3:
        def smooth(trace):
            result=[]
            for i in range(len(trace)-1):
                a=Vector(trace[max(0,i-1)]);b=Vector(trace[i]);c=Vector(trace[i+1]);d=Vector(trace[min(len(trace)-1,i+2)])
                for j in range(4):
                    t=j/4
                    result.append(tuple(.5*((2*b)+(-a+c)*t+(2*a-5*b+4*c-d)*t*t+(-a+3*b-3*c+d)*t*t*t)))
            result.append(trace[-1]);return result
        main_trace=smooth(main_trace);twig_trace=smooth(twig_trace);fork_trace=smooth(fork_trace)
    asset='croisement01-east-fallen-branch';workspace=OUT/f'branch-round-{args.revision}/assets'/asset
    if workspace.exists():raise FileExistsError(workspace)
    directory=OUT/f'branch-domains-v{args.revision}';directory.mkdir(exist_ok=True)
    inv=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in inv['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    inventory=directory/'native-masks.json';inventory.write_text(json.dumps(inv,indent=2)+'\n')
    assigned_mask=70
    if args.revision>=3:
        from PIL import Image, ImageDraw
        native=next(row for row in inv['masks'] if row['index']==70)
        domain=Image.open(native['png']).convert('L');draw=ImageDraw.Draw(domain)
        grass_polygons=[[(1075,343),(1080,346),(1080,337),(1084,344),(1087,346),(1087,341),(1093,348),(1090,362),(1075,362)],
                        [(1185,339),(1187,329),(1190,338),(1194,331),(1194,337),(1199,336),(1206,346),(1200,359),(1184,359)]]
        for polygon in grass_polygons:draw.polygon([(x-1028,y-284) for x,y in polygon],fill=0)
        domain.save(directory/'wood-domain.png')
        inv['masks'].append(dict(native,index=200,png=str(directory/'wood-domain.png')))
        inventory.write_text(json.dumps(inv,indent=2)+'\n');assigned_mask=200
        (directory/'ownership-review.json').write_text(json.dumps(dict(status='private semantic ownership correction; visual verification pending',native_mask=70,grass_exclusions=grass_polygons,reason='Bright foreground grass crosses lower bark in native artwork. Those observed foreground pixels are not wood texture.',domain_sha256=sha(directory/'wood-domain.png')),indent=2)+'\n')
    masks=directory/'east-fallen-branch.json'
    masks.write_text(json.dumps(dict(version=1,mask_inventory=str(inventory),projections=dict(exterior=dict(
        state='Initial static source',source_sha256=sha(OUT/'baseline/covered.png'),
        assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[assigned_mask])]))),indent=2)+'\n')
    review=directory/'grouping-review.json'
    review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',asset_id=asset,
        catalog_sha256=sha(OUT/'catalog.json'),inventory_sha256=sha(OUT/'grouped-inventory/inventory.json'),
        evidence='Native mask70 and part68 are the source-visible bent fallen branch and upward twig. Native context and coordinate grid inspected.'),indent=2)+'\n')
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    prepare(workspace,asset_id=asset,scene_name='Croisement01 Refinement',collection_name='Croisement01 Working',
        source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',
        inventory_path=OUT/'grouped-inventory/inventory.json',review_path=review,
        source_mask_manifest=masks,width=256,height=256,framing_padding=1.4 if args.revision>=6 else 1.16,
        lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    original=next(o for o in bpy.data.collections['Croisement01 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==asset)
    body=tube('Continuous main branch',main_trace);twig=tube('Upward twig union operand',twig_trace)
    bpy.context.view_layer.objects.active=body;body.select_set(True)
    def volume(obj):
        bm=bmesh.new();bm.from_mesh(obj.data);value=abs(bm.calc_volume(signed=True));bm.free();return value
    def union(operand,label):
        before=volume(body)
        mod=body.modifiers.new(label,'BOOLEAN');mod.operation='UNION';mod.solver='EXACT';mod.object=operand
        bpy.context.view_layer.objects.active=body
        bpy.ops.object.modifier_apply(modifier=mod.name)
        after=volume(body)
        if after<before*.98:raise ValueError(f'Union removed main wood volume: {before} -> {after}')
        bpy.data.objects.remove(operand,do_unlink=True)
    union(twig,'Continuous branch joint')
    if fork_trace:
        fork=tube('Right source fork union operand',fork_trace)
        union(fork,'Continuous right fork')
    mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True)
    inverse=original.matrix_world.inverted()
    for vert in mesh.vertices:vert.co=inverse@vert.co
    for mat in original.data.materials:mesh.materials.append(mat)
    uv=mesh.uv_layers.new(name='Source UV');ownership=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=ownership
    for face in mesh.polygons:
        for loop in face.loop_indices:
            p=original.matrix_world@mesh.vertices[mesh.loops[loop].vertex_index].co
            uv.data[loop].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);ownership.data[loop].color=(0,1,1,1)
    original.data=mesh
    contact=None
    if args.revision>=5:
        from mathutils.bvhtree import BVHTree
        terrain_nodes={'ground'}|{f'building-{i:03d}' for i in list(range(10))+list(range(76,81))}
        vertices=[];faces=[]
        for obj in bpy.data.collections['Croisement01 Working'].all_objects:
            if obj.type!='MESH' or obj.get('source_node') not in terrain_nodes:continue
            offset=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices)
            faces.extend(tuple(offset+i for i in f.vertices) for f in obj.data.polygons)
        if not faces:raise ValueError('Missing archived terrain support')
        support=BVHTree.FromPolygons(vertices,faces)
        original_world=[original.matrix_world@v.co for v in mesh.vertices]
        def displacement(point,height):
            t=max(0,min(1,(point.x-1140)/50)) if args.revision>=6 else 1
            weight=t*t*(3-2*t)
            return Vector((0,-height*weight*COS/SIN,height*weight))
        def gaps(height):
            result=[]
            for p in original_world:
                q=p+displacement(p,height);hit=support.ray_cast(Vector((q.x,q.y,10000)),Vector((0,0,-1)),20000)[0]
                if hit is None:raise ValueError('Missing terrain below candidate vertex')
                result.append(q.z-hit.z)
            return result
        before=gaps(0);height=next((i*.25 for i in range(481) if min(gaps(i*.25))>=-.1),None)
        if height is None:raise ValueError('No contact translation found within declared search interval')
        offset=Vector((0,-height*COS/SIN,height))
        for vert,p in zip(mesh.vertices,original_world):vert.co=inverse@(p+displacement(p,height))
        contact=dict(status='provisional archived terrain support; joint visual review still required',maximum_translation_world=list(offset),translation_profile='smooth right bank ramp from x1140 to1190' if args.revision>=6 else 'uniform translation',minimum_vertex_gap_before=min(before),minimum_vertex_gap_after=min(gaps(height)),source_projection_drift=abs(offset.y*SIN+offset.z*COS))
    bm=bmesh.new();bm.from_mesh(mesh)
    topology=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
    if topology['nonmanifold_edges'] or topology['degenerate_faces']:raise ValueError(topology)
    validate(workspace);modified(workspace)
    inspection=workspace/'inspection';inspection.mkdir(exist_ok=True)
    (inspection/'construction.json').write_text(json.dumps(dict(status='private candidate; self-review pending',main_trace=main_trace,twig_trace=twig_trace,fork_trace=fork_trace,contact=contact,topology=topology,model_sha256=sha(workspace/'model.blend'),limitations=['Native source traces approximate centerlines. Radius, hidden depth and twig height are inferred.','Independent native source coverage, ground contacts and actual material review pending.']),indent=2)+'\n')
    import render_candidate
    sys.argv=['render_candidate','--',str(workspace)];render_candidate.main()
    import audit_native_coverage
    sys.argv=['audit_native_coverage','--',str(workspace),'--mask','70'];audit_native_coverage.main();release()

if __name__=='__main__':main()

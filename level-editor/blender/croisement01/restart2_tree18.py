"""Source-traced northern forked stem with explicitly inferred off-map crown."""
import argparse
import json
import math
import random
import numpy as np
import sys
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha
from render_slots import acquire
from refinement_workspace import prepare,modified,validate
from refinement_inventory import inventory

SIN,COS=math.sin(math.radians(35)),math.cos(math.radians(35))


def tube(name,centers,radii):
    verts=[];faces=[];count=20
    for i,p in enumerate(centers):
        tangent=(centers[min(i+1,len(centers)-1)]-centers[max(i-1,0)]).normalized()
        helper=Vector((1,0,0)) if abs(tangent.z)>.95 else Vector((0,0,1))
        side=tangent.cross(helper).normalized();up=side.cross(tangent).normalized()
        for j in range(count):
            a=math.tau*j/count;v=p+radii[i]*(math.cos(a)*side+math.sin(a)*up)
            v.z=max(.02,v.z);verts.append(v)
    faces.append(tuple(reversed(range(count))))
    for i in range(len(centers)-1):
        for j in range(count):faces.append((i*count+j,i*count+(j+1)%count,(i+1)*count+(j+1)%count,(i+1)*count+j))
    faces.append(tuple(range((len(centers)-1)*count,len(centers)*count)))
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(verts,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    obj=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(obj);return obj


def union(body,operand):
    if body.get('defer_union'):
        vertices=[v.co.copy() for v in body.data.vertices];faces=[tuple(f.vertices) for f in body.data.polygons];offset=len(vertices)
        vertices.extend(v.co.copy() for v in operand.data.vertices);faces.extend(tuple(offset+i for i in f.vertices) for f in operand.data.polygons)
        mesh=bpy.data.meshes.new('Wood union operands');mesh.from_pydata(vertices,[],faces);mesh.update();body.data=mesh
        bpy.data.objects.remove(operand,do_unlink=True);return
    bm=bmesh.new();bm.from_mesh(body.data);before=abs(bm.calc_volume());bm.free()
    bpy.context.view_layer.objects.active=body
    mod=body.modifiers.new('Connected inferred wood','BOOLEAN');mod.operation='UNION';mod.solver='EXACT';mod.object=operand
    bpy.ops.object.modifier_apply(modifier=mod.name)
    bm=bmesh.new();bm.from_mesh(body.data);after=abs(bm.calc_volume());bm.free()
    if after<before*.98:raise ValueError('Wood union removed existing volume')
    bpy.data.objects.remove(operand,do_unlink=True)


def assign_mesh(obj,mesh):
    inverse=obj.matrix_world.inverted()
    for v in mesh.vertices:v.co=inverse@v.co
    for m in obj.data.materials:mesh.materials.append(m)
    uv=mesh.uv_layers.new(name='Source UV');known=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=known
    for loop in mesh.loops:
        p=obj.matrix_world@mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);known.data[loop.index].color=(0,1,1,1)
    obj.data=mesh


def fit_native_width(obj,alpha,role,x0=1059,y0=0):
    """Adjust the full rounded wood volume to the measured camera silhouette."""
    mesh=obj.data;points=np.asarray([tuple(obj.matrix_world@v.co) for v in mesh.vertices]);projected=np.column_stack([points[:,0],-points[:,1]*SIN-points[:,2]*COS])
    edges=np.asarray([tuple(e.vertices) for e in mesh.edges]);a=projected[edges[:,0]];b=projected[edges[:,1]];dy=b[:,1]-a[:,1]
    samples=[];old_min=[];old_max=[];target_min=[];target_max=[]
    for y in range(alpha.shape[0]):
        xs=np.flatnonzero(alpha[y]);sy=y0+y+.5
        if not len(xs):continue
        active=(np.minimum(a[:,1],b[:,1])<=sy)&(np.maximum(a[:,1],b[:,1])>=sy)&(np.abs(dy)>1e-8)
        if not np.any(active):continue
        intersections=a[active,0]+(sy-a[active,1])/dy[active]*(b[active,0]-a[active,0])
        breaks=np.flatnonzero(np.diff(xs)>1)+1;runs=np.split(xs,breaks)
        if role=='main':
            lo,hi=x0+runs[-1][0],x0+1+runs[-1][-1]
            if len(runs)==1 and y<80:lo=max(lo,hi-(18+(y-50)*.15))
        elif role=='fork':
            lo,hi=x0+runs[0][0],x0+1+runs[0][-1]
            if len(runs)==1:hi=min(hi,lo+16)
            if y>82:continue
        else:
            lo,hi=x0+xs[0],x0+1+xs[-1]
            if role=='west-cut' and xs[0]==0:
                # A clipped image edge does not bound the hidden wood volume.
                lo=float(intersections.min())+.25
        samples.append(sy);old_min.append(float(intersections.min()));old_max.append(float(intersections.max()));target_min.append(float(lo)-.25);target_max.append(float(hi)+.25)
    if len(samples)<20:raise ValueError('Insufficient independent silhouette slices')
    for field in (target_min,target_max):
        smooth=np.convolve(np.pad(field,(1,1),mode='edge'),[.25,.5,.25],mode='valid');field[:]=smooth.tolist()
    inverse=obj.matrix_world.inverted();displacements=[]
    for vertex,p,uv in zip(mesh.vertices,points,projected):
        sy=uv[1]
        tail=20 if role=='west-cut' else 1
        if sy<samples[0]-16 or sy>samples[-1]+tail:continue
        lo=np.interp(sy,samples,old_min);hi=np.interp(sy,samples,old_max);left=np.interp(sy,samples,target_min);right=np.interp(sy,samples,target_max)
        if hi-lo<1e-4:continue
        mapped=left+(p[0]-lo)*(right-left)/(hi-lo);weight=min(1,max(0,(sy-samples[0]+16)/16));dx=(mapped-p[0])*weight
        if role=='west-cut' and sy>samples[-1]:dx*=max(0,1-(sy-samples[-1])/20)
        vertex.co=inverse@Vector((p[0]+dx,p[1],p[2]));displacements.append(float(dx))
    mesh.update()
    return dict(role=role,method='Full-volume horizontal contour fit; world depth and height unchanged; smoothed native edge targets.',minimum_x_change=min(displacements),maximum_x_change=max(displacements),samples=len(samples))


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=1);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    dest=OUT/f'restart2/tree18-v{args.revision}';dest.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
    working=bpy.data.collections['Croisement01 Working'];asset='croisement01-tree-18';name='Northeast Forked Forest Tree'
    objects={int(o['source_node'].split('-')[-1]):o for o in working.all_objects if o.type=='MESH' and o.get('source_node') in ('building-052','building-053')}
    if len(objects)!=2:raise ValueError('Both native stem parts are required')
    catalog=json.loads((OUT/'catalog.json').read_text());groups=[]
    for group in catalog['groups']:
        remaining=[p for p in group['parts'] if p.get('obstacle') not in (52,53)]
        if remaining:groups.append(dict(group,parts=remaining))
    crown_node='foliage-tree18-inferred-crown'
    groups.append(dict(id=asset,name=name,parts=[dict(obstacle=52,name='Main stem and roots'),dict(obstacle=53,name='Left fork'),dict(node=crown_node,name='Inferred off-map crown')],status='private candidate'))
    catalog['groups']=groups;catalog['version']=2;catalog['canonical_owners']={f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']}
    catalog_path=dest/'catalog.json';catalog_path.write_text(json.dumps(catalog,indent=2)+'\n')
    for obj in objects.values():obj['asset_group']=asset;obj['asset_name']=name
    # Visible branch centerlines are measured in the original camera. Their
    # hidden depth is inferred; the two stems continue above the map border.
    base_y=-219/SIN
    def stem_point(x,screen_y):return Vector((x,base_y,(219-screen_y)/COS))
    trace=[(1084,211,13),(1085,195,15),(1085,175,14),(1084,155,12),(1083,135,11),(1083,110,9.5),(1083,90,9.5),(1085,65,9),(1089,40,8.5),(1091,20,8.5),(1092,-5,8)]
    if args.revision>=2:
        trace=[(1071,218,5),(1074,213,8),(1080,205,14),(1086,195,17),(1087,175,16),(1086,155,14),(1084,135,12),(1083.5,110,10),(1083.5,90,10),(1087,65,9),(1090,40,8.8),(1091.5,20,8.8),(1092.5,-5,8.8)]
    centers=[stem_point(x,y) for x,y,r in trace]+[Vector((1096,base_y-5,330)),Vector((1091,base_y-9,390)),Vector((1088,base_y-10,439))]
    body=tube('Continuous main forest stem',centers,[r for x,y,r in trace]+[6.2,4,1.2])
    if args.revision>=3:body['defer_union']=True
    roots=[[(1084,base_y,9),(1078,base_y-5,4),(1067,base_y-5,1)],[(1085,base_y,10),(1093,base_y+17,4),(1101,base_y+30,1)],[(1084,base_y,8),(1089,base_y-14,3),(1094,base_y-22,1)]]
    if args.revision>=2:
        roots=[[(1075,base_y,8),(1069,base_y-4,2),(1063,base_y-5,1)],[(1086,base_y,25),(1097,base_y+30,4),(1103,base_y+40,1)]]
    for i,points in enumerate(roots):union(body,tube(f'Root{i}',[Vector(p) for p in points],[9,5,1.4]))
    rng=random.Random(181801)
    for i in range(7):
        a=math.tau*i/7;start=Vector((1092,base_y-7,365+i*6));tip=Vector((1088+math.cos(a)*68,base_y-8+math.sin(a)*78,440+rng.uniform(-20,25)))
        union(body,tube(f'Inferred upper bough{i}',[start,start.lerp(tip,.42)+Vector((0,0,8)),tip],[4.3,2.8,.65]))
    if args.revision>=3:
        bpy.context.view_layer.objects.active=body;remesh=body.modifiers.new('Single continuous wood surface','REMESH');remesh.mode='VOXEL';remesh.voxel_size=.6;remesh.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=remesh.name)
    mainmesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);assign_mesh(objects[52],mainmesh)
    fork_trace=[(1084,80,9),(1077,52,7.4),(1073,32,7.4),(1069,12,7),(1064,-8,6.5)]
    fork_centers=[stem_point(x,y)+Vector((0,2,0)) for x,y,r in fork_trace]+[Vector((1054,base_y-7,325)),Vector((1038,base_y-3,384)),Vector((1034,base_y,427))]
    fork=tube('Continuous left source fork',fork_centers,[r for x,y,r in fork_trace]+[5,3,.8]);forkmesh=fork.data.copy();bpy.data.objects.remove(fork,do_unlink=True);assign_mesh(objects[53],forkmesh)
    if args.revision>=4:
        from PIL import Image
        alpha=np.asarray(Image.open(OUT/'baseline/masks/000018.png').convert('L'))>127
        reports=[fit_native_width(objects[52],alpha,'main'),fit_native_width(objects[53],alpha,'fork')]
        (dest/'source-contour-fit.json').write_text(json.dumps(dict(status='Construction fitting diagnostic, not independent validation',parts=reports),indent=2)+'\n')
    # Crown foliage lies outside the displayed source. It is deliberately
    # untextured pending geometry review, with a rounded volume deeper than wide.
    verts=[];faces=[]
    lobes=[(-53,-15,427,42,48,43),(44,20,445,45,54,48),(-4,-55,462,48,48,40),(-20,45,478,46,55,38),(25,-12,497,36,40,26)]
    for i in range(1050 if args.revision>=2 else 850):
        theta=rng.uniform(0,math.tau);zeta=rng.uniform(-1,1);rad=rng.random()**.35
        center=Vector((1080+94*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y-8+105*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),442+66*rad*zeta))
        if args.revision>=2:
            dx,dy,z,rx,ry,rz=lobes[i%len(lobes)];center=Vector((1080+dx+rx*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y-8+dy+ry*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),z+rz*rad*zeta))
        size=rng.uniform(3,7);normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.7,.9))).normalized();side=normal.cross(Vector((0,0,1))).normalized();up=normal.cross(side).normalized()
        offset=len(verts);verts.extend([center-side*size,center+up*size*.65,center+side*size,center-up*size*.65]);faces.extend([(offset,offset+1,offset+2),(offset,offset+2,offset+3)])
    mesh=bpy.data.meshes.new('Inferred complete tree18 crown');mesh.from_pydata(verts,[],faces);mesh.update()
    mat=bpy.data.materials.new('Unknown off-map foliage');mat.diffuse_color=(.34,.34,.34,1);mesh.materials.append(mat)
    crown=bpy.data.objects.new('Tree18 inferred off-map crown',mesh);working.objects.link(crown)
    for k,v in dict(source_node=crown_node,asset_group=asset,asset_name=name,part_name='Inferred off-map crown',projection_component='crown').items():crown[k]=v
    uv=mesh.uv_layers.new(name='Source UV');known=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=known
    for loop in mesh.loops:
        p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);known.data[loop.index].color=(0,1,1,1)
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'input.blend'))
    inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    assigned_mask=18
    if args.revision>=2:
        from PIL import Image,ImageDraw,ImageChops
        row=next(r for r in native['masks'] if r['index']==18);domain=Image.open(row['png']).convert('L');original=domain.copy();draw=ImageDraw.Draw(domain)
        exclusions=[[(1058,16),(1068,15),(1073,19),(1067,26),(1060,25)],[(1064,42),(1071,42),(1078,48),(1077,56),(1071,58),(1067,52)],[(1080,18),(1088,17),(1095,22),(1098,27),(1091,34),(1084,35),(1080,29)]]
        for polygon in exclusions:draw.polygon([(x-1059,y) for x,y in polygon],fill=0)
        domain.save(dest/'wood-domain.png');ImageChops.subtract(original,domain).save(dest/'deferred-foreground-foliage.png')
        assigned_mask=218;native['masks'].append(dict(row,index=assigned_mask,png=str(dest/'wood-domain.png')))
        (dest/'source-ownership.json').write_text(json.dumps(dict(status='private wood domain; foreground foliage association unresolved',native_mask=18,wood_mask=218,exclusions=exclusions,reason='Visible yellow-green foliage crosses the bare fork; these pixels must not be interpreted as bark. Remaining source pixels are retained unchanged.',deferred_pixels=sum(v>0 for v in ImageChops.subtract(original,domain).getdata())),indent=2)+'\n')
    native_path=dest/'masks.json';native_path.write_text(json.dumps(native,indent=2)+'\n')
    masks=dest/'source-masks.json';masks.write_text(json.dumps(dict(version=1,mask_inventory=str(native_path),projections=dict(exterior=dict(state='Initial native tree18',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[assigned_mask])]))),indent=2)+'\n')
    review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(catalog_path),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Source mask18 and native parts52/53 identify the forked north-edge stem. Crown is an explicit wholly off-map hypothesis, not observed artwork.'),indent=2)+'\n')
    worker=dest/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=catalog_path,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=masks,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    validate(worker);modified(worker)
    inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    measured=[]
    for obj in bpy.data.collections['Croisement01 Working'].all_objects:
        if obj.type!='MESH' or obj.get('asset_group')!=asset:continue
        points=[obj.matrix_world@v.co for v in obj.data.vertices];bm=bmesh.new();bm.from_mesh(obj.data)
        measured.append(dict(source_node=obj.get('source_node'),vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces),bounds=[[min(p[i] for p in points),max(p[i] for p in points)] for i in range(3)]));bm.free()
    (inspection/'geometry-evidence.json').write_text(json.dumps(dict(model_sha256=sha(worker/'model.blend'),meshes=measured,crown_surface='Separate two-sided thin leaf surfaces; open boundary edges are intentional.'),indent=2)+'\n')
    (inspection/'construction.json').write_text(json.dumps(dict(status='private hypothesis; self-review pending',model_sha256=sha(worker/'model.blend'),native_mask=18,native_parts=[52,53],main_trace=trace,fork_trace=fork_trace,crown_nominal_design=dict(width=188,depth=210,height=132,center_z=442,observed_pixels=0,actual_extent_evidence='geometry-evidence.json'),reference_assets=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'],limitations=['Crown, upper boughs and hidden stem depth are inferred. Crown has no observed source pixels.','Root contacts and projected source coverage require independent saved-model checks.','Whole-map source ownership and grouping are not yet integrated.']),indent=2)+'\n')
    import render_candidate
    sys.argv=['render_candidate','--',str(worker)];render_candidate.main()


if __name__=='__main__':main()

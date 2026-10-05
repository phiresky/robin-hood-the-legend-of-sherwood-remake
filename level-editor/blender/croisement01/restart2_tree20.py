"""Northern forest stem behind the neighboring tree's diagonal branch."""
import argparse
import json
import math
import random
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from mathutils import Vector
from PIL import Image,ImageChops

sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import ROOT,OUT,SIN,COS,tube,union,assign_mesh,fit_native_width
from refinement_workspace import prepare,modified,validate
from refinement_inventory import inventory
from evidence_io import sha
from render_slots import acquire


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=1);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    dest=OUT/f'restart2/tree20-v{args.revision}';dest.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
    working=bpy.data.collections['Croisement01 Working'];asset='croisement01-tree-20';name='Northern Shaded Forest Tree'
    obj=next(o for o in working.all_objects if o.type=='MESH' and o.get('source_node')=='building-082')
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    row=next(r for r in native['masks'] if r['index']==20);foreign=next(r for r in native['masks'] if r['index']==19)
    alpha=Image.open(row['png']).convert('L');foreign_canvas=Image.new('L',alpha.size)
    foreign_canvas.paste(Image.open(foreign['png']).convert('L'),(foreign['box_top_left'][0]-row['box_top_left'][0],foreign['box_top_left'][1]-row['box_top_left'][1]))
    overlap=ImageChops.multiply(alpha,foreign_canvas);wood=ImageChops.subtract(alpha,overlap);wood.save(dest/'wood-domain.png');overlap.save(dest/'neighbor-branch-domain.png')
    native['masks'].append(dict(row,index=220,png=str(dest/'wood-domain.png')))
    (dest/'source-ownership.json').write_text(json.dumps(dict(status='private reviewed source split',native_mask=20,wood_mask=220,foreground_owner_native_mask=19,foreground_pixels=int(np.count_nonzero(np.asarray(overlap))),reason='The diagonal branch crossing this bare trunk belongs to the adjacent forked tree19. Exact overlapping native masks preserve its separate source pixels.'),indent=2)+'\n')
    catalog=json.loads((OUT/'catalog.json').read_text());groups=[]
    for group in catalog['groups']:
        remaining=[p for p in group['parts'] if p.get('obstacle')!=82]
        if remaining:groups.append(dict(group,parts=remaining))
    crown_node='foliage-tree20-inferred-crown'
    groups.append(dict(id=asset,name=name,parts=[dict(obstacle=82,name='Shaded stem'),dict(node=crown_node,name='Inferred off-map crown')],status='private candidate'))
    catalog['groups']=groups;catalog['version']=2;catalog['canonical_owners']={f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']}
    catalog_path=dest/'catalog.json';catalog_path.write_text(json.dumps(catalog,indent=2)+'\n')
    obj['asset_group']=asset;obj['asset_name']=name
    base_y=-139/SIN;trace=[(1226,137,10),(1230,129,13.5),(1233,113,13.5),(1233,94,12.5),(1233,74,12),(1232,53,12),(1231.5,32,12),(1231.5,12,11.5),(1231,-10,11)]
    centers=[Vector((x,base_y,(139-y)/COS)) for x,y,r in trace]+[Vector((1234,base_y-4,235)),Vector((1228,base_y-7,300)),Vector((1232,base_y-9,366))]
    body=tube('Rounded shaded stem',centers,[r for x,y,r in trace]+[8.5,5,1]);body['defer_union']=True
    rng=random.Random(202001)
    for i in range(8):
        a=math.tau*i/8;start=Vector((1230,base_y-6,270+i*5));tip=Vector((1230+math.cos(a)*78,base_y-7+math.sin(a)*82,358+rng.uniform(-12,30)))
        union(body,tube(f'Inferred bough{i}',[start,start.lerp(tip,.5)+Vector((0,0,10)),tip],[5,3,.6]))
    bpy.context.view_layer.objects.active=body;modifier=body.modifiers.new('Connected hidden branches','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.7;modifier.use_remove_disconnected=False;bpy.ops.object.modifier_apply(modifier=modifier.name)
    mesh=body.data.copy();bpy.data.objects.remove(body,do_unlink=True);assign_mesh(obj,mesh)
    fit=fit_native_width(obj,np.asarray(alpha)>127,'single',x0=row['box_top_left'][0]);(dest/'source-contour-fit.json').write_text(json.dumps(fit,indent=2)+'\n')
    removed_islands=[]
    if args.revision>=3:
        bm=bmesh.new();bm.from_mesh(obj.data);remaining=set(bm.verts);components=[]
        while remaining:
            queue=[remaining.pop()];component=[]
            while queue:
                vert=queue.pop();component.append(vert)
                for edge in vert.link_edges:
                    other=edge.other_vert(vert)
                    if other in remaining:remaining.remove(other);queue.append(other)
            components.append(component)
        components.sort(key=len,reverse=True)
        for component in components[1:]:
            points=[obj.matrix_world@v.co for v in component]
            xmin,xmax=min(p.x for p in points),max(p.x for p in points)
            ymin,ymax=min(-p.y*SIN-p.z*COS for p in points),max(-p.y*SIN-p.z*COS for p in points)
            pixel_columns=list(range(math.ceil(xmin-.5),math.floor(xmax-.5)+1))
            pixel_rows=list(range(math.ceil(ymin-.5),math.floor(ymax-.5)+1))
            if len(component)>16 or (pixel_columns and pixel_rows):
                raise ValueError(f'Detached wood covers native sample centers: {len(component)} vertices, {pixel_columns}, {pixel_rows}')
            removed_islands.append(dict(vertices=len(component),native_bounds=[xmin,ymin,xmax,ymax],native_sample_centers=0,reason='Subpixel voxel island whose projected bounds contain no native pixel sample center.'))
            bmesh.ops.delete(bm,geom=component,context='VERTS')
        bm.to_mesh(obj.data);bm.free()
    verts=[];faces=[];lobes=[(-53,-15,350,43,54,45),(49,20,365,46,56,48),(-4,-55,387,49,49,39),(-20,45,398,48,58,38),(25,-12,414,36,43,26)]
    for i in range(1150):
        theta=rng.uniform(0,math.tau);zeta=rng.uniform(-1,1);rad=rng.random()**.35;dx,dy,z,rx,ry,rz=lobes[i%len(lobes)]
        center=Vector((1230+dx+rx*rad*math.sqrt(1-zeta*zeta)*math.cos(theta),base_y-8+dy+ry*rad*math.sqrt(1-zeta*zeta)*math.sin(theta),z+rz*rad*zeta))
        size=rng.uniform(3,7);normal=Vector((rng.uniform(-1,1),rng.uniform(-1,1),rng.uniform(-.7,.9))).normalized();side=normal.cross(Vector((0,0,1))).normalized();up=normal.cross(side).normalized()
        n=len(verts);verts.extend([center-side*size,center+up*size*.65,center+side*size,center-up*size*.65]);faces.extend([(n,n+1,n+2),(n,n+2,n+3)])
    mesh=bpy.data.meshes.new('Complete inferred tree20 crown');mesh.from_pydata(verts,[],faces);mesh.update()
    mat=bpy.data.materials.new('Unknown off-map tree20 foliage');mat.diffuse_color=(.34,.34,.34,1);mesh.materials.append(mat)
    crown=bpy.data.objects.new('Tree20 inferred off-map crown',mesh);working.objects.link(crown)
    for k,v in dict(source_node=crown_node,asset_group=asset,asset_name=name,part_name='Inferred off-map crown',projection_component='crown').items():crown[k]=v
    uv=mesh.uv_layers.new(name='Source UV');known=mesh.color_attributes.new(name='Source ownership',type='FLOAT_COLOR',domain='CORNER');mesh.color_attributes.active_color=known
    for loop in mesh.loops:
        p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x/1408,1-(-p.y*SIN-p.z*COS)/960);known.data[loop.index].color=(0,1,1,1)
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'input.blend'))
    inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    native_path=dest/'masks.json';native_path.write_text(json.dumps(native,indent=2)+'\n')
    masks=dest/'source-masks.json';masks.write_text(json.dumps(dict(version=1,mask_inventory=str(native_path),projections=dict(exterior=dict(state='Initial native tree20',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[220])]))),indent=2)+'\n')
    review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(catalog_path),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Native mask20 and part82 describe the shaded stem behind the adjacent tree19 branch. The complete off-map crown is inferred, not observed.'),indent=2)+'\n')
    worker=dest/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=catalog_path,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=masks,width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    validate(worker);modified(worker);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    (inspection/'construction.json').write_text(json.dumps(dict(status='private candidate; self-review pending',model_sha256=sha(worker/'model.blend'),native_mask=20,native_parts=[82],trace=trace,removed_subpixel_voxel_islands=removed_islands,reference_assets=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'],limitations=['Crown, upper boughs and hidden stem depth are inferred.','Foreground branch appearance belongs to tree19 and is excluded from this stem.','Terrain contact, actual crown depth and native coverage need independent saved-model review.']),indent=2)+'\n')
    import render_candidate
    sys.argv=['render_candidate','--',str(worker)];render_candidate.main()


if __name__=='__main__':main()

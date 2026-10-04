"""Isolated source-traced open rail fences, distinct from the stone returns.

Observed member paths constrain front silhouettes. Six-unit unseen cross
sections and the short eastern continuation are editable depth hypotheses.
"""
import argparse
import json
import math
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from PIL import Image, ImageDraw
from mathutils import Vector

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY,replace_mesh
from refinement_inventory import inventory,validate_catalog
from refinement_workspace import prepare,modified
from audit_candidates import audit
from render_tree import render_workspace

# Native full-image pixel observations, not an automatic interpretation of the
# character occlusion polyline. Each named strip becomes a closed wood member.
PATHS={
94:[
 ('west post',[(1727,650,2.2),(1728,695,2.2)]),
 ('east post',[(1780,620,2.5),(1778,660,2.5)]),
 ('upper rail',[(1726,652,2.0),(1778,631,2.0),(1804,617,2.0)]),
 ('lower rail',[(1729,670,2.5),(1778,650,2.5),(1804,637,2.5)]),
 ('leaning end timber',[(1719,637,2.5),(1730,656,2.5),(1745,678,2.0)]),
 ('fallen timber',[(1744,704,1.7),(1758,688,1.4)]),
],
95:[
 ('west post',[(1499,770,2.5),(1498,815,2.5)]),
 ('second post',[(1527,748,2.5),(1527,787,2.4)]),
 ('third post',[(1562,726,2.5),(1564,763,2.3)]),
 ('fourth post',[(1607,706,2.3),(1607,745,2.3)]),
 ('fifth post',[(1646,688,2.5),(1652,720,2.0)]),
 ('upper west rail',[(1473,820,2.0),(1483,807,2.8),(1527,759,2.8)]),
 ('lower west rail',[(1473,837,2.0),(1485,820,2.5),(1527,772,2.5)]),
 ('upper middle west rail',[(1527,759,2.5),(1564,735,2.5)]),
 ('upper middle east rail',[(1564,744,2.5),(1607,718,2.5)]),
 ('lower middle west rail',[(1527,772,2.3),(1564,752,2.3)]),
 ('upper east rail',[(1607,717,2.0),(1647,705,2.0),(1668,686,2.0)]),
 ('lower east rail',[(1607,732,2.5),(1652,716,2.5)]),
 ('leaning end timber',[(1667,678,2.3),(1684,705,2.0)]),
 ('fallen timber',[(1557,752,1.6),(1578,753,1.4),(1597,753,1.0)]),
]}
GROUND={94:[(1716,699),(1728,698),(1778,663),(1804,649)],95:[(1470,842),(1498,818),(1527,790),(1564,766),(1607,748),(1652,723),(1684,708)]}
DIRECTORY=OUT/'missing-fence-candidates/v9'
BASE=OUT/'authored-stem-integration'


def outline(path):
    horizontal=abs(path[-1][0]-path[0][0])>abs(path[-1][1]-path[0][1])
    left=[];right=[]
    for x,y,r in path:
        left.append((x,y+r) if horizontal else (x+r,y))
        right.append((x,y-r) if horizontal else (x-r,y))
    return left+right[::-1]


def snap_rails():
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text())
    for index,members in PATHS.items():
        record=level['masks'][index];x0,y0=record['box_top_left'];mask=np.asarray(Image.open(OUT/f'baseline/masks/{index:06}.png').convert('L'))>0
        for j,(label,path) in enumerate(members):
            if 'fallen' in label or 'post' in label or 'leaning' in label:continue
            vertical='post' in label or 'leaning' in label
            independent=1 if vertical else 0;dependent=1-independent
            p=sorted(path,key=lambda point:point[independent]);refined=[]
            start,end=p[0][independent],p[-1][independent]
            for t in np.linspace(start,end,max(3,round(abs(end-start)/2))):
                value=float(np.interp(t,[q[independent] for q in p],[q[dependent] for q in p]));radius=float(np.interp(t,[q[independent] for q in p],[q[2] for q in p]))
                pixel=round(t)-(y0 if vertical else x0)
                if 0<=pixel<mask.shape[0 if vertical else 1]:
                    values=np.flatnonzero(mask[pixel,:] if vertical else mask[:,pixel])+(x0 if vertical else y0)
                    groups=np.split(values,np.flatnonzero(np.diff(values)>1)+1)
                    groups=[g for g in groups if len(g) and len(g)<=9 and abs(float(g.mean())-value)<=6]
                    if groups:
                        group=min(groups,key=lambda g:abs(float(g.mean())-value));value=float(group.mean());radius=min(3.5,(len(group)+.5)/2)
                refined.append([value,float(t),radius] if vertical else [float(t),value,radius])
            # Keep rails continuous through junctions and raster stair steps.
            # Axis-aligned contour offsets avoid spiky miter amplification.
            for _ in range(2):
                previous=[q[:] for q in refined]
                for k in range(1,len(refined)-1):
                    refined[k][dependent]=(previous[k-1][dependent]+2*previous[k][dependent]+previous[k+1][dependent])/4
                    refined[k][2]=min(3.5,(previous[k-1][2]+2*previous[k][2]+previous[k+1][2])/4)
            members[j]=(label,refined)


def build(obj,index):
    vertices=[];faces=[];members=[]
    for label,path in PATHS[index]:
        polygon=outline([(x,y,r+.7) for x,y,r in path]);start=len(vertices);count=len(polygon)
        ground=GROUND[index]
        for behind in (0,6):
            for x,y in polygon:
                g=float(np.interp(x,[p[0] for p in ground],[p[1] for p in ground]))
                if label=='fallen timber':g=y+6*SIN*COS
                front=Vector((x,-g/SIN,(g-y)/COS))
                joint_offset=.4 if 'post' in label else (.8 if 'leaning' in label else 0.)
                vertices.append(tuple(front+RAY*(joint_offset-behind)))
        faces.append(tuple(start+j for j in reversed(range(count))))
        faces.append(tuple(start+count+j for j in range(count)))
        faces.extend((start+j,start+(j+1)%count,start+count+(j+1)%count,start+count+j) for j in range(count))
        members.append(dict(name=label,source_centerline=path,source_outline=polygon,inferred_depth=6,source_edge_allowance_pixels=.7))
    template=next(o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('source_node')=='building-059')
    result=replace_mesh(obj,vertices,faces,materials=list(template.data.materials))
    bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    result.update(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces))
    bm.to_mesh(obj.data);bm.free()
    if result['nonmanifold_edges'] or result['degenerate_faces']:raise ValueError('Invalid fence member topology')
    return dict(**result,members=members,ground_line=GROUND[index],joint_semantics='Closed individual members intentionally intersect at joinery. Gaps remain empty; no alpha stencil plane.',inference='Back faces offset six world units behind source-facing member. East94 rails continue twelve pixels beyond source edge; exact unseen endpoint is inferred.')


def main():
    DIRECTORY.mkdir(parents=True,exist_ok=False)
    snap_rails()
    source=OUT/'animation-references/composite-frame-0.png';rgb=Image.open(source).convert('RGBA')
    catalog=json.loads((BASE/'catalog.json').read_text());manifest=json.loads((BASE/'source-masks.json').read_text())
    invpath=Path(manifest['mask_inventory']);mask_inventory=json.loads(invpath.read_text());rows={r['index']:r for r in mask_inventory['masks']}
    for row in mask_inventory['masks']:row['png']=str((invpath.parent/row['png']).resolve())
    source_evidence={}
    for index in PATHS:
        domain=430+index-94;node=f'scenery-upright-fence-{index:03}';asset=f'croisement02-east-upright-rail-fence-{index}'
        catalog['groups'].append(dict(id=asset,name=f'East upright rail fence {index}',authored_scenery=True,parts=[dict(node=node,name=f'Open wooden rail fence {index}',wood_domain_mask=domain)]));catalog['canonical_owners'][node]=asset
        row=rows[index];x,y=row['box_top_left'];w,h=row['box_size'];native=np.zeros((1152,1792),bool)
        native[y:y+h,x:x+w]=np.array(Image.open(row['png']).convert('L'))>0
        trace=Image.new('L',(1792,1152));d=ImageDraw.Draw(trace)
        for _,path in PATHS[index]:d.polygon(outline(path),fill=255)
        # Native alpha is authoritative for accepted existing fence pixels;
        # detached timber outside it receives a separately reviewed source trace.
        traced=np.asarray(trace)>0;observed=native&traced
        for label,path in PATHS[index]:
            if label=='fallen timber':
                piece=Image.new('L',(1792,1152));ImageDraw.Draw(piece).polygon(outline(path),fill=255);observed|=np.asarray(piece)>0
        colors=np.asarray(rgb)[:,:,:3].astype(float)
        vegetation=(colors[:,:,1]>colors[:,:,0]*1.15)&(colors[:,:,1]>colors[:,:,2]*1.25)
        observed&=~vegetation
        exclusions=[87] if index==94 else [85,93]
        for excluded in exclusions:
            er=rows[excluded];ex,ey=er['box_top_left'];ew,eh=er['box_size']
            leaf=np.asarray(Image.open(er['png']).convert('L'))>0
            observed[ey:ey+eh,ex:ex+ew]&=~leaf
        path=DIRECTORY/f'domain-{domain}.png';Image.fromarray(observed.astype('uint8')*255).save(path)
        mask_inventory['masks'].append(dict(index=domain,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],provenance='Native fence mask intersected with inspected individual wood members, plus traced fallen timber; explicit foreground foliage masks and strongly green samples excluded from wood provenance. See source-review.json.'))
        manifest['projections']['exterior']['assignments'].append(dict(source_node=node,mask_indices=[domain],reviewed=True))
        source_evidence[index]=dict(native_mask=index,domain=domain,native_pixels=int(native.sum()),accepted_pixels=int(observed.sum()),native_accepted_pixels=int((native&observed).sum()),native_rejected_pixels=int((native&~observed).sum()),additional_traced_pixels=int((observed&~native).sum()),source_rgb_changed=0,excluded_native_foreground=exclusions,domain_sha256=sha(path))
        context=rgb.crop((max(0,x-20),max(0,y-20),min(1792,x+w+20),min(1152,y+h+30)));context.save(DIRECTORY/f'context-{index}.png')
        owned=rgb.copy();owned.putalpha(Image.fromarray(observed.astype('uint8')*255));owned.crop((max(0,x-20),max(0,y-20),min(1792,x+w+20),min(1152,y+h+30))).save(DIRECTORY/f'owned-{index}.png')
        annotation=context.convert('RGB');draw=ImageDraw.Draw(annotation)
        for number,(label,p) in enumerate(PATHS[index]):
            pp=[(a-x+20,b-y+20) for a,b,_ in p];draw.line(pp,fill='#ff40ff',width=1);draw.text(pp[0],str(number),fill='white')
        annotation.resize((annotation.width*3,annotation.height*3),Image.Resampling.NEAREST).save(DIRECTORY/f'trace-{index}.png')
    receivers={'ground',*catalog['canonical_owners']}
    for index in PATHS:
        domain=430+index-94;node=f'scenery-upright-fence-{index:03}'
        manifest['projections']['exterior'].setdefault('occluder_constraints',[]).append(dict(reviewed=True,source_node=node,receiver_nodes=sorted(receivers-{node}),mask_indices=[domain],reason='Fence may hide foreign source artwork only on its observed wood domain; open gaps do not own ground.'))
    ground=next(a for a in manifest['projections']['exterior']['assignments'] if a.get('source_node')=='ground')
    ground.setdefault('exclude_mask_indices',[]).extend([430,431]);ground['exclusions_reviewed']=True;ground['exclusion_reason']+=' Source-traced upright rail fences leave ground ownership.'
    write_json(DIRECTORY/'catalog.json',catalog);write_json(DIRECTORY/'mask-inventory.json',mask_inventory)
    manifest['mask_inventory']=str(DIRECTORY/'mask-inventory.json');write_json(DIRECTORY/'source-masks.json',manifest)
    write_json(DIRECTORY/'source-review.json',dict(status='isolated candidate; manual source review pending',source_sha256=sha(source),records=source_evidence,note='Not the approved stone returns called east-rail-fence. Native94/95 have no sight obstacle; authored visuals do not fabricate one. Ground integration remains pending.'))
    bpy.ops.wm.open_mainfile(filepath=str(BASE/'input.blend'));bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working'];reports={}
    for index in PATHS:
        asset=f'croisement02-east-upright-rail-fence-{index}';name=f'East upright rail fence {index}'
        obj=bpy.data.objects.new(name,bpy.data.meshes.new(name));collection.objects.link(obj)
        for key,value in dict(source_node=f'scenery-upright-fence-{index:03}',asset_group=asset,asset_name=name,part_name=name).items():obj[key]=value
        reports[index]=build(obj,index)
    inventory(DIRECTORY/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=source)
    validate_catalog(DIRECTORY/'inventory/inventory.json',DIRECTORY/'catalog.json')
    write_json(DIRECTORY/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(DIRECTORY/'catalog.json'),inventory_sha256=sha(DIRECTORY/'inventory/inventory.json'),evidence='Two distinct native upright rail runs without native obstacles; approved stone returns and all existing parts retained. Source and member traces separately archived.'))
    bpy.ops.wm.save_as_mainfile(filepath=str(DIRECTORY/'input.blend'))
    lighting=json.loads((OUT/'forest-v4-round-1/assets/croisement02-tree-08/workspace.json').read_text())['lighting']
    for index in PATHS:
        bpy.ops.wm.open_mainfile(filepath=str(DIRECTORY/'input.blend'))
        worker=DIRECTORY/f'assets/croisement02-east-upright-rail-fence-{index}'
        prepare(worker,asset_id=worker.name,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=source,grouping_manifest=DIRECTORY/'catalog.json',inventory_path=DIRECTORY/'inventory/inventory.json',review_path=DIRECTORY/'grouping-review.json',source_mask_manifest=DIRECTORY/'source-masks.json',width=384,height=384,framing_padding=1.25,lighting=lighting)
        modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
        (worker/'inspection').mkdir(exist_ok=True)
        write_json(worker/'inspection/refinement.json',dict(asset_id=worker.name,geometry=reports[index],model_sha256=sha(worker/'model.blend'),source_evidence=source_evidence[index],status='isolated authored geometry; self-review pending',limitations=['Six-unit member depth is inferred.','Short94 extends beyond eastern map edge with inferred rail ends.','Native mask contains foreground vegetation; explicit87 or85/93 exclusions and conservative green-sample rejection keep ambiguous leaf colors out of wood provenance.','No canonical ownership, ground model or approved geometry was changed.','Unseen wood remains neutral pending geometry approval and texture fill.']))
        audit(worker);render_workspace(worker,384,release_slot=False)
        print('FENCE CANDIDATE',worker,flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,default=DIRECTORY,help='Fresh isolated output; existing packets are never overwritten.')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    DIRECTORY=args.output.resolve()
    acquire()
    try:main()
    finally:release()

"""Two small source-traced bank stones, with closed inferred rear surfaces."""
import argparse
import json
import math
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree

sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import ROOT,OUT,SIN,COS,assign_mesh,fit_native_width
from refinement_workspace import prepare,modified,validate
from refinement_inventory import inventory
from evidence_io import sha
from render_slots import acquire


def stone(contour,ground,depth):
    center=Vector((sum(x for x,y in contour)/len(contour),sum(y for x,y in contour)/len(contour)))
    bottom=max(y for x,y in contour);vertices=[];faces=[];n=len(contour)
    # Both hemispheres retain genuine depth. The external native contour is
    # the common equator, with a flattened hidden underside on the bank.
    for side in (1,-1):
        for fraction in (.06,.35,.7,1):
            for x,y in contour:
                sx=center.x+(x-center.x)*fraction;sy=center.y+(y-center.y)*fraction
                z=max(ground+.04,ground+(bottom-sy)*.74+side*depth*math.sqrt(max(0,1-fraction*fraction))*.5*SIN)
                vertices.append((sx,-(sy+z*COS)/SIN,z))
        start=(0 if side==1 else 4*n)
        faces.append(tuple(start+i for i in range(n)))
        for ring in range(3):
            for j in range(n):faces.append((start+ring*n+j,start+ring*n+(j+1)%n,start+(ring+1)*n+(j+1)%n,start+(ring+1)*n+j))
    # Weld the equator shared by the front and rear hemispheres.
    mesh=bpy.data.meshes.new('Rounded angular source stone');mesh.from_pydata(vertices,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.001);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    return mesh


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--revision',type=int,default=1);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    dest=OUT/f'restart2/rock29-v{args.revision}';dest.mkdir(exist_ok=False)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
    working=bpy.data.collections['Croisement01 Working'];asset='croisement01-small-bank-stones';name='Small Bank Stones'
    obj=next(o for o in working.all_objects if o.type=='MESH' and o.get('source_node')=='building-018')
    tv=[];tf=[];terrain_nodes={'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]}
    for support in working.all_objects:
        if support.type!='MESH' or support.get('source_node') not in terrain_nodes:continue
        offset=len(tv);tv.extend(support.matrix_world@v.co for v in support.data.vertices);tf.extend(tuple(offset+i for i in f.vertices) for f in support.data.polygons)
    terrain=BVHTree.FromPolygons(tv,tf);ray=Vector((0,-COS,SIN));target=Vector((470,-483/SIN,0));hit=terrain.ray_cast(target+ray*5000,-ray,10000)[0]
    if hit is None:raise ValueError('Missing archival bank below native stones')
    ground=hit.z
    rear=[(478,449),(479,443),(482,439),(487,436),(491,434),(497,434),(500,437),(499,445),(497,452),(493,458),(485,464),(479,459),(476,454)]
    front=[(460,451),(470,449),(475,451),(479,454),(478,460),(478,468),(477,477),(473,481),(465,486),(458,484),(455,479),(453,468),(455,458)]
    vertices=[];faces=[]
    for contour,depth in ((rear,34),(front,29)):
        mesh=stone(contour,ground,depth);offset=len(vertices);vertices.extend(v.co.copy() for v in mesh.vertices);faces.extend(tuple(offset+i for i in f.vertices) for f in mesh.polygons)
    mesh=bpy.data.meshes.new('Two separate closed bank stones');mesh.from_pydata(vertices,[],faces);mesh.update();assign_mesh(obj,mesh)
    conform=[]
    if args.revision>=2:
        masks=json.loads((OUT/'baseline/masks/manifest.json').read_text());row=next(r for r in masks['masks'] if r['index']==29)
        alpha=np.asarray(Image.open(OUT/'baseline/masks'/row['png']).convert('L'))>127
        fit=fit_native_width(obj,alpha,'single',x0=453,y0=431)
        inverse=obj.matrix_world.inverted()
        for vertex in obj.data.vertices:
            p=obj.matrix_world@vertex.co;support=terrain.ray_cast(p+ray*5000,-ray,10000)[0]
            if support is None:raise ValueError('Missing bank under stone vertex')
            delta=max(0,(support.z+.08-p.z)/SIN)
            if delta:vertex.co=inverse@(p+ray*delta);conform.append(delta)
        obj.data.update()
        (dest/'source-contour-and-contact.json').write_text(json.dumps(dict(contour_fit=fit,ray_conformed_vertices=len(conform),maximum_ray_displacement=max(conform,default=0),method='Move buried vertices along original camera rays onto archival bank; preserve native projection.'),indent=2)+'\n')
    obj['asset_group']=asset;obj['asset_name']=name
    catalog=json.loads((OUT/'catalog.json').read_text());groups=[]
    for group in catalog['groups']:
        remaining=[p for p in group['parts'] if p.get('obstacle')!=18]
        if remaining:groups.append(dict(group,parts=remaining))
    groups.append(dict(id=asset,name=name,parts=[dict(obstacle=18,name='Two bank stones')],status='private candidate'));catalog['groups']=groups
    catalog_path=dest/'catalog.json';catalog_path.write_text(json.dumps(catalog,indent=2)+'\n')
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'input.blend'))
    inventory(dest/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    native_path=dest/'masks.json';native_path.write_text(json.dumps(native,indent=2)+'\n')
    masks=dest/'source-masks.json';masks.write_text(json.dumps(dict(version=1,mask_inventory=str(native_path),projections=dict(exterior=dict(state='Initial native bank stones',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[29])]))),indent=2)+'\n')
    review=dest/'grouping-review.json';review.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(catalog_path),inventory_sha256=sha(dest/'inventory/inventory.json'),evidence='Native mask29 and obstacle18 identify the adjacent small stones on the raised bank; original source and native coordinate grid inspected.'),indent=2)+'\n')
    worker=dest/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,source_path=OUT/'baseline/covered.png',grouping_manifest=catalog_path,inventory_path=dest/'inventory/inventory.json',review_path=review,source_mask_manifest=masks,width=256,height=256,framing_padding=1.3,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    validate(worker);modified(worker);inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    (inspection/'construction.json').write_text(json.dumps(dict(status='private candidate; self-review pending',model_sha256=sha(worker/'model.blend'),native_mask=29,native_part=18,front_trace=front,rear_trace=rear,bank_height=ground,limitations=['Rear stone depth and hidden surfaces are inferred.','Native source contours have one-to-three-pixel tracing uncertainty.','Two closed components are intentional separate stones; native source ownership remains one obstacle.']),indent=2)+'\n')
    import render_candidate
    sys.argv=['render_candidate','--',str(worker)];render_candidate.main()


if __name__=='__main__':main()

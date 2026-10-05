"""Read-only current-scene sign attribution and paired opaque support intervals."""
import json,sys
from pathlib import Path
from collections import defaultdict,Counter
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from sign_context_import import append_verified
from restart2_sign_scene_projection import verify_import,bounds,intersects
from restart2_sign_fragment_bounds import Surface
from sign_rigid_depth import groups_for
DEST=OUT/'restart2-fence/sign58-opaque-intervals-v2'
PROOF=OUT/'restart2-state/sign-scene-projection-v1'

def clip(poly,axis,bound,greater):
    result=[]
    for a,b in zip(poly,poly[1:]+poly[:1]):
        va=(a[axis]-bound)*(1 if greater else -1);vb=(b[axis]-bound)*(1 if greater else -1)
        if va>=0:result.append(a)
        if (va<0<=vb)or(vb<0<=va):result.append(a+(b-a)*(va/(va-vb)))
    return result

def main():
    DEST.mkdir(exist_ok=False)
    inputs=json.loads((PROOF/'inputs.json').read_text());refs=json.loads((PROOF/'evaluated-imports.json').read_text())
    assert refs['inputs_sha256']==sha(PROOF/'inputs.json')
    sign=Path(inputs['sign_model']);assert sha(sign)==inputs['sign_model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(sign));scene=bpy.context.scene
    objects=[];receipts=[]
    sources=[(Path(inputs['frozen_model']),inputs['frozen_model_sha256'],refs['frozen_receivers'])]
    sources += [(Path(v['model']),v['model_sha256'],refs['replacements'][a])for a,v in inputs['changes'].items()]
    for path,digest,reference in sources:
        assert sha(path)==digest
        imported,receipt=append_verified(scene,path,list(reference),reference);verify_import(imported,receipt,reference)
        objects.extend(imported);receipts.extend(receipt)
    assembly=json.loads((sign.parent/'assembly.json').read_text());order=json.loads((OUT/'state-sign-candidate/native-order-reference-v3/manifest.json').read_text());animations=json.loads((OUT/'animation-references/manifest.json').read_text())['animations'];results=[]
    for target in [5,8]:
        row=next(r for r in assembly['instances']if r['target_index']==target);x,y=row['native_target']['position_x'],row['native_target']['position_y'];box=(x-48,y-64,x+48,y+32)
        neighbors=[o for o in objects if intersects(bounds(o),box)]
        leaves=[o for o in neighbors if any(m and m.get('foliage_physical_opacity')for m in o.data.materials)]
        foliage=Surface([(o.get('asset_group',o.name),o)for o in leaves]);solidverts=[];solidtris=[]
        for o in neighbors:
            if o in leaves:continue
            o.data.calc_loop_triangles();offset=len(solidverts);solidverts.extend([tuple(o.matrix_world@v.co)for v in o.data.vertices]);solidtris.extend([tuple(offset+i for i in t.vertices)for t in o.data.loop_triangles])
        solid=BVHTree.FromPolygons(solidverts,solidtris,all_triangles=True)if solidtris else None
        overlayrow=next(r for r in order['records']if r['target_index']==target);layers=[]
        for ov in overlayrow['overlapping_animations']:
            assert ov['after_sign'];anim=next(a for a in animations if a['index']==ov['index']);frames=[]
            for f in anim['frames']:
                canvas=Image.new('RGBA',(1792,1152));canvas.alpha_composite(Image.open(f['image']).convert('RGBA'),tuple(f['bbox'][:2]));frames.append(np.asarray(canvas.crop(box))[:,:,3]>127)
            layers.append(frames)
        period=max([len(f)for f in layers],default=1);masks=[np.logical_or.reduce([f[p%len(f)]for f in layers])if layers else np.zeros((96,96),bool)for p in range(period)]
        ever=np.logical_or.reduce(masks);always=np.logical_and.reduce(masks)
        Image.fromarray(np.uint8(ever)*255).save(DEST/f'target-{target}-native-overlay-any.png');Image.fromarray(np.uint8(always)*255).save(DEST/f'target-{target}-native-overlay-all.png')
        needs=defaultdict(float);pixels=[];counts=[]
        for phase in [0,8,16,24]:
            scene.frame_set(1+phase*2);bpy.context.view_layer.update();body=Surface([('sign',scene.objects[n])for n in row['parts']if'native_body_frame'in scene.objects[n]and scene.objects[n].scale.x>.5])
            alone=np.asarray(Image.open(PROOF/f'target-{target}/phase-{phase:02}-alone.png'))[1::3,1::3,0]>127;joint=np.asarray(Image.open(PROOF/f'target-{target}/phase-{phase:02}-joint.png'))[1::3,1::3,0]>127
            lost=alone&~joint;counts.append(dict(sign_phase=phase,physical_hidden=int(lost.sum()),outside_all_native_overlay_phases=int((lost&~ever).sum()),outside_frame0=int((lost&~masks[0]).sum()),outside_at_least_one_overlay_phase=int((lost&~always).sum())))
            for py,px in zip(*np.nonzero(lost&~masks[0])):
                sx,sy=box[0]+int(px)+.5,box[1]+int(py)+.5;origin=Vector((sx,-sy/SIN,0))+RAY*5000;bodyhits=list(body.intersections(origin))
                if not bodyhits:continue
                front,back=bodyhits[0][1],bodyhits[-1][1];hits=list(foliage.intersections(origin,front-.001));stack=[]
                for hit,distance,r in hits:
                    required=back-distance+.1;needs[(r['object'],r['polygon'])]=max(needs[(r['object'],r['polygon'])],required)
                    stack.append(dict(object=r['object'],polygon=r['polygon'],required=required,hit_z=float(hit.z),role=r['role']))
                pixels.append(dict(sign_phase=phase,source_pixel=[sx-.5,sy-.5],native_overlay_ever=bool(ever[py,px]),blockers=stack))
        intervals=[]
        for o in leaves:
            mesh=o.data;mesh.calc_loop_triangles();world=np.array([tuple(o.matrix_world@v.co)for v in mesh.vertices]);projection=np.column_stack((world[:,0],-SIN*world[:,1]-COS*world[:,2]));groups,roots=groups_for(mesh,world,projection);required=defaultdict(float)
            for (name,p),demand in needs.items():
                if name==o.name:required[roots[mesh.polygons[p].vertices[0]]]=max(required[roots[mesh.polygons[p].vertices[0]]],demand)
            if not required:continue
            limits={g:float('inf')for g in required};mins={g:float('inf')for g in required};samples=Counter();inside=Counter();images={};uv=mesh.uv_layers.get('Foliage UV')or mesh.uv_layers.active
            for tri in mesh.loop_triangles:
                g=roots[tri.vertices[0]]
                if g not in required:continue
                mat=mesh.materials[tri.material_index];im=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image);w,h=im.size
                if im.name not in images:images[im.name]=np.asarray(im.pixels[:],np.float32).reshape(h,w,4)[:,:,3]
                coords=np.array([tuple(uv.data[i].uv)for i in tri.loops])*[w,h];a,b,c=coords;basis=np.column_stack((b-a,c-a))
                if abs(np.linalg.det(basis))<1e-9:continue
                inv=np.linalg.inv(basis);lo=np.floor(coords.min(0)).astype(int);hi=np.ceil(coords.max(0)).astype(int)
                for iy in range(lo[1],hi[1]):
                    for ix in range(lo[0],hi[0]):
                        if images[im.name][iy%h,ix%w]<=.5:continue
                        poly=list(coords.copy())
                        for axis,bound,greater in [(0,ix,True),(0,ix+1,False),(1,iy,True),(1,iy+1,False)]:
                            if len(poly)<3:break
                            poly=clip(poly,axis,bound,greater)
                        if len(poly)<3:continue
                        q=np.array(poly);q=np.vstack((q,q.mean(0)));bc=(q-a)@inv.T;points=np.column_stack((1-bc.sum(1),bc))@world[list(tri.vertices)]
                        for point in points:
                            limit=max(0,(point[2]-.5)/SIN);mins[g]=min(mins[g],float(point[2]));samples[g]+=1
                            if solid:
                                hit,normal,index,distance=solid.ray_cast(Vector(point),-RAY,10000)
                                if hit is not None:
                                    if normal.dot(-RAY)>=0:limit=0;inside[g]+=1
                                    else:limit=min(limit,max(0,distance-.25))
                            limits[g]=min(limits[g],limit)
            for g,demand in required.items():
                assert samples[g]>0,(o.name,g)
                intervals.append(dict(object=o.name,paired_component=g,vertices=len(groups[g]),required_retreat=demand,opaque_clearance_limit=limits[g],opaque_minimum_z=mins[g],opaque_footprint_samples=samples[g],existing_interior_samples=inside[g],feasible_at_sampled_constraints=bool(limits[g]>=demand)))
        result=dict(target=target,counts=counts,overlay_phases=period,pixels=pixels,paired_intervals=intervals,first_hit_objects=dict(Counter(p['blockers'][0]['object']for p in pixels if p['blockers'])),constraints=sum(bool(p['blockers'])for p in pixels),infeasible_components=sum(not r['feasible_at_sampled_constraints']for r in intervals));write_json(DEST/f'target-{target}.json',result);results.append({k:v for k,v in result.items()if k not in ['pixels','paired_intervals']});print('TARGET_DONE',target,flush=True)
    for path,digest,_ in sources:assert sha(path)==digest
    write_json(DEST/'report.json',dict(status='Read-only finite diagnostic; no model mutation or renders',inputs_sha256=sha(PROOF/'inputs.json'),evaluated_imports_sha256=sha(PROOF/'evaluated-imports.json'),imports=receipts,results=results,limitations=['Four physical sign poses; native overlay masks cover all stored relative phases. Counts use fitted body silhouette, not sprite pixel parity.','Footprint intervals preserve rigid paired fragments and existing solid interior contacts. A failed interval excludes that translation, not every alternative reconstruction.','Opaque UV cell corners and centroids are sampled; bank breaklines within cells and coherent branch connection remain separate requirements.','Source-camera ordering may require explicit native presentation instead of physical geometry distortion.']))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()

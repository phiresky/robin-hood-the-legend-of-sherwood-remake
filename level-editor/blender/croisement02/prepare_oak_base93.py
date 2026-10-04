"""Prepare three overlapping foliage lobes for the oak-base foreground foliage without combining them into one globe."""
import json,sys
from pathlib import Path
import bpy,numpy as np
from scipy.ndimage import minimum_filter
from scipy.spatial import cKDTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_inventory import inventory,validate_catalog
from refinement_workspace import prepare,modified
from shrub_geometry import build
from audit_candidates import audit
from render_tree import render_workspace
from opacity_bounds import measure
from tree_geometry import RAY,SIN,COS
BASE=OUT/'ground-plant-integration'
DEST=OUT/'understory-candidates/oak-base93-v2'
SPECS=((93,93,503),)

def main():
    catalog=json.loads((BASE/'catalog.json').read_text())
    # This frozen full source base is reconciled with the current catalog only at registration.
    DEST.mkdir(exist_ok=False);write_json(DEST/'previous-catalog.json',catalog)
    source=OUT/'animation-references/composite-frame-0.png';rgb=np.asarray(Image.open(source).convert('RGBA'))
    manifest=json.loads((BASE/'source-masks.json').read_text());invpath=Path(manifest['mask_inventory']);masks=json.loads(invpath.read_text());rows={r['index']:r for r in masks['masks']}
    def canvas(index):
        row=rows[index];x,y=row['box_top_left'];w,h=row['box_size'];a=np.zeros((1152,1792),bool);a[y:y+h,x:x+w]=np.asarray(Image.open(invpath.parent/row['png']).convert('L'))>127;return a
    packets={};validation=[]
    for native,split,domain in SPECS:
        split_path=OUT/f'mixed-wood-audit/domain-{domain}.png';observed=np.asarray(Image.open(split_path).convert('L'))>127
        physical=observed.copy()
        if np.any(observed&~canvas(native)):raise ValueError('Complement escaped native source')
        path=DEST/f'domain-{domain}.png';Image.fromarray(observed.astype('uint8')*255).save(path)
        node=f'foliage-shrub-{native:03}';asset=f'croisement02-shrub-{native:02}';name=f'Oak-base foliage {native}'
        catalog['groups'].append(dict(id=asset,name=name,authored_scenery=True,native_foliage_mask=native,parts=[dict(node=node,name=name,foliage_domain_mask=domain)]));catalog['canonical_owners'][node]=asset
        masks['masks'].append(dict(index=domain,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],provenance=f'Reviewed mixed oak-base leaf proposal{domain}; existing wood, canopy and initial trap exclusions preserved'))
        manifest['projections']['exterior']['assignments'].append(dict(source_node=node,mask_indices=[domain],reviewed=True))
        folder=DEST/f'shrub-{native}';folder.mkdir();row=rows[native];x,y=row['box_top_left'];w,h=row['box_size'];extension=40 if native==77 else 0
        complete=np.zeros((h+extension,w,4),np.uint8);known=complete.copy()
        for row_index in range(h+extension):
            sample=row_index if row_index<h else h-1-(row_index-h)
            complete[row_index]=rgb[y+sample,x:x+w];complete[row_index,:,3]=physical[y+sample,x:x+w]*255
            if row_index<h:
                known[row_index]=complete[row_index];known[row_index,:,3]=observed[y+sample,x:x+w]*255
            else:
                xx=np.arange(w);gate=((row_index-h)/42.)**2+((xx-w*.5)/(w*.7))**2<1
                complete[row_index,:,3]*=gate.astype(np.uint8)
        Image.fromarray(known).save(folder/'observed-source.png');Image.fromarray(complete).save(folder/'complete-source.png')
        accepted=known[:,:,3]>127
        if int(accepted.sum())!=int(observed.sum()):raise ValueError('Leaf source partition lost pixels')
        if not np.array_equal(known[:h,:,:3][observed[y:y+h,x:x+w]],rgb[y:y+h,x:x+w,:3][observed[y:y+h,x:x+w]]):raise ValueError('Leaf source RGB changed')
        packet=dict(directory=str(folder),native_bbox=[x,y,w,h+extension],native_mask=native,observed_domain=domain,source_sha256=sha(source),cluster_count=500,leaf_size_range=[3.5,6.],curved_front=True,irregular_source_fragments=True,irregular_inferred_alpha=True,source_split_path=str(split_path),source_split_sha256=sha(split_path),physical_silhouette_authority=dict(sha256=sha(folder/'complete-source.png'),reason='Reviewed clear foliage503 only, partitioned into three local masses. Existing oak35, fence95, wall101, shrub85 and canopies remain excluded; reserved147 edge pixels remain unresolved.'),inferred_map_edge_completion=bool(extension),ownership_note='Known native leaves exclude reviewed wood/canopy/state neighbours; covered foliage RGB is inferred only from this same plant.')
        if extension:packet['inferred_front_image']='complete-source.png'
        write_json(folder/'partition.json',packet);packets[native]=packet
        validation.append(dict(native_mask=native,domain=domain,known_rgb_changed=0,observed_pixels=int(observed.sum()),split_sha256=sha(split_path)))
    domains=[np.asarray(Image.open(DEST/f'domain-{domain}.png'))>0 for _,_,domain in SPECS]
    if any(np.any(a&b) for i,a in enumerate(domains) for b in domains[i+1:]):raise ValueError('Duplicate new foliage ownership')
    write_json(DEST/'source-rgb-validation.json',dict(status='PASS',known_rgb_unchanged=True,duplicate_observed_pixels=0,records=validation))
    ext=manifest['projections']['exterior'];receivers={'ground',*catalog['canonical_owners']}
    for native,split,domain in SPECS:
        node=f'foliage-shrub-{native:03}';ext.setdefault('occluder_constraints',[]).append(dict(reviewed=True,source_node=node,receiver_nodes=sorted(receivers-{node}),mask_indices=[domain],reason='Inferred foliage blocks foreign source only within its reviewed native leaf complement.'))
    for rule in ext['occluder_constraints']:
        if rule['source_node'].startswith(('foliage-','scenery-')):rule['receiver_nodes']=sorted(receivers-{rule['source_node']})
    ground=next(r for r in ext['assignments'] if r.get('source_node')=='ground');ground['exclude_mask_indices']+=[503]
    write_json(DEST/'catalog.json',catalog);write_json(DEST/'mask-inventory.json',masks);manifest['mask_inventory']=str(DEST/'mask-inventory.json');write_json(DEST/'source-masks.json',manifest)
    bpy.ops.wm.open_mainfile(filepath=str(BASE/'input.blend'));bpy.context.preferences.filepaths.save_version=0;collection=bpy.data.collections['Croisement02 Working'];reports={}
    for native,packet in packets.items():
        directory=Path(packet['directory']);complete=np.asarray(Image.open(directory/'complete-source.png').convert('RGBA'));known=np.asarray(Image.open(directory/'observed-source.png').convert('RGBA'));x,y,w,h=packet['native_bbox']
        yy,xx=np.indices((h,w));points=np.stack((xx+x,yy+y),axis=-1);centers=np.array([[1372,773],[1459,793],[1493,834]],float);radii=np.array([[26,42],[46,58],[26,36]],float)
        distances=np.sum(((points[None,:,:,:]-centers[:,None,None,:])/radii[:,None,None,:])**2,axis=-1);owners=np.argmin(distances,axis=0);minimum=np.min(distances,axis=0)
        lobes=[];all_objects=[];owned=np.zeros((h,w),int)
        for lobe_index,label in enumerate(('left','central','lower-right')):
            observed=(known[:,:,3]>127)&(owners==lobe_index);physical=(complete[:,:,3]>127)&(distances[lobe_index]<=minimum+.35);physical|=observed
            py,px=np.nonzero(physical);left,right=int(px.min()),int(px.max())+1;top,bottom=int(py.min()),int(py.max())+1
            folder=directory/label;folder.mkdir();rgba=complete[top:bottom,left:right].copy();rgba[:,:,3]=physical[top:bottom,left:right]*255;observed_rgba=known[top:bottom,left:right].copy();observed_rgba[:,:,3]=observed[top:bottom,left:right]*255
            Image.fromarray(rgba).save(folder/'complete-source.png');Image.fromarray(observed_rgba).save(folder/'observed-source.png');owned+=observed
            lobe=dict(packet,directory=str(folder),native_bbox=[x+left,y+top,right-left,bottom-top],cluster_count=500);lobe.pop('physical_silhouette_authority',None)
            known_alpha=observed_rgba[:,:,3]>127;donors=np.argwhere(minimum_filter(known_alpha.astype('uint8'),size=3)>0)
            if len(donors)<8:raise ValueError('Insufficient same-lobe leaf donors')
            rng=np.random.default_rng(930022+lobe_index);fh,fw=known_alpha.shape;sites=np.array([[py+rng.uniform(-1,1),px+rng.uniform(-1,1)] for py in range(0,fh,3) for px in range(0,fw,3)])
            gy,gx=np.indices((fh,fw));positions=np.column_stack((gy.ravel(),gx.ravel()));ids=cKDTree(sites).query(positions)[1];picked=donors[rng.integers(0,len(donors),len(sites))];samples=picked[ids]+np.clip(np.rint(positions-sites[ids]).astype(int),-1,1)
            samples[:,0]=np.clip(samples[:,0],0,fh-1);samples[:,1]=np.clip(samples[:,1],0,fw-1)
            fill=observed_rgba[samples[:,0],samples[:,1]].reshape(fh,fw,4).copy();fill[known_alpha]=observed_rgba[known_alpha];fill[:,:,3]=rgba[:,:,3];Image.fromarray(fill).save(folder/'leaf-fill.png');lobe['inferred_front_image']='leaf-fill.png'
            write_json(folder/'partition.json',lobe)
            name='Oak-base foliage93 '+label;obj=bpy.data.objects.new(name,bpy.data.meshes.new(name));collection.objects.link(obj);all_objects.append(obj)
            for key,value in dict(source_node='foliage-shrub-093',asset_group='croisement02-shrub-93',asset_name='Oak-base foliage 93',part_name='Oak-base foliage 93').items():obj[key]=value
            report=build(obj,lobe);before=measure(obj);delta=RAY*((.5-before['bounds_min'][2])/SIN)
            for vertex in obj.data.vertices:vertex.co+=delta
            obj.data.update();after=measure(obj);report.update(opacity_bounds=after,minimum_z=min(v.co.z for v in obj.data.vertices));lobes.append(report)
            write_json(folder/'support.json',dict(status='ground datum hypothesis; rock/tree neighbourhood pending',world_delta=list(delta),current_opacity_bounds=after,source_pixel_displacement=[float(delta.x),float(-delta.y*SIN-delta.z*COS)]))
        if np.any(owned>1) or not np.array_equal(owned>0,known[:,:,3]>127):raise ValueError('Thicket lobe partition lost or duplicated native leaves')
        reports[93]=dict(geometry_version='native-shrub-leaf-volume-v2',lobes=lobes,source_partition='Three overlapping local volumes; unique observed ownership by normalized native-space distance',minimum_z=min(v.co.z for obj in all_objects for v in obj.data.vertices))
    visibility={o:o.hide_render for o in collection.all_objects if o.type=='MESH'}
    for o in visibility:o.hide_render=False
    inventory(DEST/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=source,patch_manifest=OUT/'source-states/layers.json')
    for o,value in visibility.items():o.hide_render=value
    validate_catalog(DEST/'inventory/inventory.json',DEST/'catalog.json')
    write_json(DEST/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(DEST/'catalog.json'),inventory_sha256=sha(DEST/'inventory/inventory.json'),evidence='Reviewed clear foliage503 is partitioned among three local volumes; full native93 includes existing wood and cannot be treated as plant silhouette. Ground datum support solved; actual neighbourhood review remains pending.'))
    targets={f'croisement02-shrub-{i:02}' for i,_,_ in SPECS}
    for old in list(bpy.data.objects):
        if old.type=='MESH' and old.get('asset_group') not in targets:bpy.data.objects.remove(old,do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)
    scoped=dict(catalog,groups=[g for g in catalog['groups'] if g['id'] in targets],canonical_owners={n:a for n,a in catalog['canonical_owners'].items() if a in targets})
    write_json(DEST/'worker-catalog.json',scoped);scoped_masks=json.loads(json.dumps(manifest));nodes=set(scoped['canonical_owners'])
    for projection in scoped_masks['projections'].values():
        projection['assignments']=[a for a in projection['assignments'] if a.get('source_node') in nodes or a.get('asset_group') in targets]
        projection['occluder_constraints']=[]
    write_json(DEST/'worker-source-masks.json',scoped_masks)
    inventory(DEST/'worker-inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=source)
    validate_catalog(DEST/'worker-inventory/inventory.json',DEST/'worker-catalog.json')
    write_json(DEST/'worker-grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(DEST/'worker-catalog.json'),inventory_sha256=sha(DEST/'worker-inventory/inventory.json'),evidence='Scoped foliage only; full source inventory retained separately and actual fence/log/tree joint review pending.'))
    write_json(DEST/'scope-derivation.json',dict(source=str(BASE/'input.blend'),source_sha256=sha(BASE/'input.blend'),full_catalog_sha256=sha(DEST/'catalog.json'),full_inventory_sha256=sha(DEST/'inventory/inventory.json'),targets=sorted(targets),reason='Scoped foliage workers avoid full-scene copies; no full-scene occlusion claim.'))
    bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'input.blend'),compress=True)
    for native,split,domain in SPECS:
        # Rejoin the shared queue between independent asset workspaces.
        release();acquire()
        bpy.ops.wm.open_mainfile(filepath=str(DEST/'input.blend'));worker=OUT/f'understory-round-22/assets/croisement02-shrub-{native:02}'
        prepare(worker,asset_id=worker.name,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=source,grouping_manifest=DEST/'worker-catalog.json',inventory_path=DEST/'worker-inventory/inventory.json',review_path=DEST/'worker-grouping-review.json',source_mask_manifest=DEST/'worker-source-masks.json',width=384,height=384,framing_padding=1.25)
        modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True)
        (worker/'inspection').mkdir(exist_ok=True)
        write_json(worker/'inspection/refinement.json',dict(asset_id=worker.name,mask=native,crown=reports[native],source_packet=str(DEST/f'shrub-{native}/partition.json'),model_sha256=sha(worker/'model.blend'),status='private oak-base foliage candidate; actual fence/log/tree joint pending',limitations=['Hidden foliage volume is inferred from the same native leaf palette.','Only observed leaf complement is source evidence; excluded existing wood, canopies and initial trap targets remain with their owners.','Visible twigs have source patches, not independent inferred trunk geometry.','No approved geometry changed; no user approval or texture completion claimed.']))
        audit(worker);render_workspace(worker,384,release_slot=False,transparent_bounces=256)
        print('FOREST CLUMP',native,worker,flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()

"""Prepare an elevated north-edge leaf fringe with explicitly inferred crown association."""
import json,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from scipy.ndimage import minimum_filter
from scipy.spatial import cKDTree
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
DEST=OUT/'understory-candidates/north-fringe22-v1'
SPECS=((22,22,480),)

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
        split_path=OUT/f'understory-candidates/north-fringe22-audit/domain-{domain}.png';observed=np.asarray(Image.open(split_path).convert('L'))>127
        physical=canvas(native)
        if np.any(observed&~canvas(native)):raise ValueError('Complement escaped native source')
        path=DEST/f'domain-{domain}.png';Image.fromarray(observed.astype('uint8')*255).save(path)
        node=f'foliage-fringe-{native:03}';asset=f'croisement02-canopy-fringe-{native:02}';name=f'North canopy fringe {native}'
        catalog['groups'].append(dict(id=asset,name=name,authored_scenery=True,native_foliage_mask=native,parts=[dict(node=node,name=name,foliage_domain_mask=domain)]));catalog['canonical_owners'][node]=asset
        masks['masks'].append(dict(index=domain,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],provenance=f'Reviewed north-edge leaf proposal{domain}; existing wood, canopy and initial trap exclusions preserved'))
        manifest['projections']['exterior']['assignments'].append(dict(source_node=node,mask_indices=[domain],reviewed=True))
        folder=DEST/f'shrub-{native}';folder.mkdir();row=rows[native];x,y=row['box_top_left'];w,h=row['box_size'];extension=40
        complete=np.zeros((h+extension,w,4),np.uint8);known=complete.copy()
        complete[extension:]=rgb[y:y+h,x:x+w];complete[extension:,:,3]=physical[y:y+h,x:x+w]*255
        known[extension:]=complete[extension:];known[extension:,:,3]=observed[y:y+h,x:x+w]*255
        for k in range(extension):
            sample=min(k,h-1);xx=np.arange(w);gate=(k/36.)**2+((xx-w*.5)/(w*.7))**2<1
            complete[extension-1-k]=rgb[y+sample,x:x+w];complete[extension-1-k,:,3]=physical[y+sample,x:x+w]*gate*255
        Image.fromarray(known).save(folder/'observed-source.png');Image.fromarray(complete).save(folder/'complete-source.png')
        accepted=known[:,:,3]>127
        if int(accepted.sum())!=int(observed.sum()):raise ValueError('Leaf source partition lost pixels')
        if not np.array_equal(known[extension:,:,:3][observed[y:y+h,x:x+w]],rgb[y:y+h,x:x+w,:3][observed[y:y+h,x:x+w]]):raise ValueError('Leaf source RGB changed')
        packet=dict(directory=str(folder),native_bbox=[x,y-extension,w,h+extension],native_mask=native,observed_domain=domain,source_sha256=sha(source),cluster_count=240,leaf_size_range=[2.5,4.],curved_front=True,irregular_source_fragments=True,irregular_inferred_alpha=True,source_split_path=str(split_path),source_split_sha256=sha(split_path),physical_silhouette_authority=dict(sha256=sha(folder/'complete-source.png'),reason='Exact native22 leaf silhouette with inferred tapered northern continuation. Existing134/135 pixels are excluded from observed ownership; crown association is unproven.'),inferred_map_edge_completion=bool(extension),ownership_note='Known native leaves exclude reviewed wood/canopy/state neighbours; covered foliage RGB is inferred only from this same plant.')
        known_alpha=known[:,:,3]>127;dense=minimum_filter(known_alpha.astype('uint8'),size=3)>0;donors=np.argwhere(dense)
        if len(donors)<8:raise ValueError('Insufficient dense native leaf patches')
        rng=np.random.default_rng(native*711);fh,fw=known_alpha.shape;sites=np.array([[py+rng.uniform(-1.5,1.5),px+rng.uniform(-1.5,1.5)] for py in range(0,fh,3) for px in range(0,fw,3)])
        yy,xx=np.indices((fh,fw));positions=np.column_stack((yy.ravel(),xx.ravel()));ids=cKDTree(sites).query(positions)[1];picked=donors[rng.integers(0,len(donors),len(sites))];samples=picked[ids]+np.clip(np.rint(positions-sites[ids]).astype(int),-1,1)
        samples[:,0]=np.clip(samples[:,0],0,fh-1);samples[:,1]=np.clip(samples[:,1],0,fw-1)
        fill=known[samples[:,0],samples[:,1]].reshape(fh,fw,4).copy();fill[known_alpha]=known[known_alpha];fill[:,:,3]=complete[:,:,3];Image.fromarray(fill).save(folder/'leaf-fill.png');packet['inferred_front_image']='leaf-fill.png'

        write_json(folder/'partition.json',packet);packets[native]=packet
        validation.append(dict(native_mask=native,domain=domain,known_rgb_changed=0,observed_pixels=int(observed.sum()),split_sha256=sha(split_path)))
    domains=[np.asarray(Image.open(DEST/f'domain-{domain}.png'))>0 for _,_,domain in SPECS]
    if any(np.any(a&b) for i,a in enumerate(domains) for b in domains[i+1:]):raise ValueError('Duplicate new foliage ownership')
    write_json(DEST/'source-rgb-validation.json',dict(status='PASS',known_rgb_unchanged=True,duplicate_observed_pixels=0,records=validation))
    ext=manifest['projections']['exterior'];receivers={'ground',*catalog['canonical_owners']}
    for native,split,domain in SPECS:
        node=f'foliage-fringe-{native:03}';ext.setdefault('occluder_constraints',[]).append(dict(reviewed=True,source_node=node,receiver_nodes=sorted(receivers-{node}),mask_indices=[domain],reason='Inferred foliage blocks foreign source only within its reviewed native leaf complement.'))
    for rule in ext['occluder_constraints']:
        if rule['source_node'].startswith(('foliage-','scenery-')):rule['receiver_nodes']=sorted(receivers-{rule['source_node']})
    ground=next(r for r in ext['assignments'] if r.get('source_node')=='ground');ground['exclude_mask_indices'] += [domain for _,_,domain in SPECS]
    write_json(DEST/'catalog.json',catalog);write_json(DEST/'mask-inventory.json',masks);manifest['mask_inventory']=str(DEST/'mask-inventory.json');write_json(DEST/'source-masks.json',manifest)
    bpy.ops.wm.open_mainfile(filepath=str(BASE/'input.blend'));bpy.context.preferences.filepaths.save_version=0;collection=bpy.data.collections['Croisement02 Working'];reports={}
    for native,packet in packets.items():
        name=f'North canopy fringe {native}';obj=bpy.data.objects.new(name,bpy.data.meshes.new(name));collection.objects.link(obj)
        for key,value in dict(source_node=f'foliage-fringe-{native:03}',asset_group=f'croisement02-canopy-fringe-{native:02}',asset_name=name,part_name=name).items():obj[key]=value
        reports[native]=build(obj,packet)
        before=measure(obj);mid_z=(before['bounds_min'][2]+before['bounds_max'][2])*.5;delta=RAY*((250.-mid_z)/SIN)
        for vertex in obj.data.vertices:vertex.co+=delta
        obj.data.update();after=measure(obj)
        support_report=dict(status='Elevated crown-fringe placement hypothesis; exact selected14/15 joint pending',previous_opacity_bounds=before,current_opacity_bounds=after,world_delta=list(delta),source_pixel_displacement=[float(delta.x),float(-delta.y*SIN-delta.z*COS)],reason='Center height250 is inferred from selected tree15 raw crown scaffold hits229–259 under native22. Tree14 intersects only8/97 samples at147–161. Character depth orders masks but does not measure canopy height. No trunk or observed root association is claimed.',placement_evidence=str(OUT/'understory-candidates/north-fringe22-audit/evidence.json'),placement_evidence_sha256=sha(OUT/'understory-candidates/north-fringe22-audit/evidence.json'))
        write_json(DEST/f'shrub-{native}/support.json',support_report)
        reports[native]['opacity_bounds']=support_report['current_opacity_bounds'];reports[native]['support_evidence']=str(DEST/f'shrub-{native}/support.json')
        reports[native]['minimum_z']=min(v.co.z for v in obj.data.vertices)
    visibility={o:o.hide_render for o in collection.all_objects if o.type=='MESH'}
    for o in visibility:o.hide_render=False
    inventory(DEST/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',source_path=source,patch_manifest=OUT/'source-states/layers.json')
    for o,value in visibility.items():o.hide_render=value
    validate_catalog(DEST/'inventory/inventory.json',DEST/'catalog.json')
    write_json(DEST/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(DEST/'catalog.json'),inventory_sha256=sha(DEST/'inventory/inventory.json'),evidence='Unique native22 leaf domain480 excludes134/135; elevated association to nearby crown scaffolds is an explicit hypothesis, requiring exact neighbour review.'))
    targets={f'croisement02-canopy-fringe-{i:02}' for i,_,_ in SPECS}
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
        bpy.ops.wm.open_mainfile(filepath=str(DEST/'input.blend'));worker=OUT/f'understory-round-21/assets/croisement02-canopy-fringe-{native:02}'
        prepare(worker,asset_id=worker.name,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=source,grouping_manifest=DEST/'worker-catalog.json',inventory_path=DEST/'worker-inventory/inventory.json',review_path=DEST/'worker-grouping-review.json',source_mask_manifest=DEST/'worker-source-masks.json',width=384,height=384,framing_padding=1.25)
        modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True)
        (worker/'inspection').mkdir(exist_ok=True)
        write_json(worker/'inspection/refinement.json',dict(asset_id=worker.name,mask=native,crown=reports[native],source_packet=str(DEST/f'shrub-{native}/partition.json'),model_sha256=sha(worker/'model.blend'),status='private elevated north-fringe candidate; actual14/15 crown joint pending',limitations=['Hidden foliage volume is inferred from the same native leaf palette.','Only observed leaf complement is source evidence; excluded existing wood, canopies and initial trap targets remain with their owners.','Association with adjacent tree15 crown is inferred; no new trunk or grounded shrub is claimed.','No approved geometry changed; no user approval or texture completion claimed.']))
        audit(worker);render_workspace(worker,384,release_slot=False)
        print('FOREST CLUMP',native,worker,flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()

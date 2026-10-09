"""Apply only the frozen inferred bark texels; preserve all geometry and source RGBA."""
import hashlib,json,shutil,sys
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(Path(__file__).parent))
from render_slots import acquire,release
from restart2_bake_texture_candidate import snapshot,pixels,array_hash
R=ROOT/'level-editor/work/croisement01-refinement/restart2';C=R/'approved-tree02-fill-v1/croisement01-tree-02';B=C/'baked-v3-same-leaf-crown';O=C/'baked-v4-bounded-bark-gaps';P=R/'tree02-rendered-gap-probe-v2'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
assert not O.exists();assert shutil.disk_usage(R).free>=10*1024**3+64*1024**2
assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
plan=json.loads((P/'curved-quad-donor-plan.json').read_text());probe=json.loads((P/'report.json').read_text());assert sha(B/'worker.blend')==plan['model_sha256']==probe['model_sha256'];assert sha(P/'report.json')==plan['probe_sha256'];assert not plan['rejected'] and len(plan['plans'])==45
acquire();O.mkdir();bpy.ops.wm.open_mainfile(filepath=str(B/'worker.blend'));bpy.context.preferences.filepaths.save_version=0
try:
 manifest=json.loads((C/'experiment/views.json').read_text());scene=bpy.data.scenes[manifest['scene_name']];names=set(manifest['object_names']);before=snapshot(scene,names);uvs={o.name:{uv.name:array_hash([v.uv[:] for v in uv.data]) for uv in o.data.uv_layers} for o in scene.objects if o.type=='MESH'}
 validation=json.loads((B/'validation.json').read_text());entry=next(e for e in validation['layers'][0]['objects'] if e['object']==plan['object']);old_provenance=Path(entry['texel_provenance']['path']);assert sha(old_provenance)==entry['texel_provenance']['sha256'];ownership=np.load(old_provenance)['ownership'];h,w=ownership.shape
 obj=bpy.data.objects[plan['object']];used={f.material_index for f in obj.data.polygons};images={n.image.name:n.image for i,mat in enumerate(obj.data.materials) if i in used and mat and mat.use_nodes for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image and tuple(n.image.size)==(w,h)};assert len(images)==1;image=next(iter(images.values()));image_name=image.name;original=pixels(image);after=original.copy();new_ownership=ownership.copy();other={im.name:array_hash(pixels(im)) for im in bpy.data.images if im!=image and len(im.pixels)};packet=probe['packets'][plan['object']];target_mask=np.zeros((h,w),bool)
 for row in plan['plans']:
  x,y=row['target'];sx,sy=row['donor'];assert ownership[y,x]==0 and ownership[sy,sx]==row['donor_ownership'] and ownership[sy,sx] in [1,2]
  assert np.array_equal(original[y,x],np.array(packet['texels'][f'{x},{y}']['rgba'],np.float32))
  assert np.array_equal(original[sy,sx],np.array(packet['texels'][f'{sx},{sy}']['rgba'],np.float32))
  after[y,x,:3]=original[sy,sx,:3];new_ownership[y,x]=3;target_mask[y,x]=True
 assert target_mask.sum()==45 and np.array_equal(after[~target_mask],original[~target_mask]) and np.array_equal(after[:,:,3],original[:,:,3]);assert np.array_equal(after[ownership!=0],original[ownership!=0])
 image.pixels.foreach_set(after.ravel());image.update();image.pack();model=O/'worker.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model),compress=True);bpy.ops.wm.open_mainfile(filepath=str(model))
 assert snapshot(bpy.data.scenes[manifest['scene_name']],names)==before;assert np.array_equal(pixels(bpy.data.images[image_name]),after);assert all(array_hash(pixels(bpy.data.images[n]))==v for n,v in other.items());assert {o.name:{uv.name:array_hash([v.uv[:] for v in uv.data]) for uv in o.data.uv_layers} for o in bpy.data.scenes[manifest['scene_name']].objects if o.type=='MESH'}==uvs
 provenance=O/'upper-stems-provenance.npz';np.savez_compressed(provenance,ownership=new_ownership);entry['texel_provenance'].update(path=str(provenance.resolve()),sha256=sha(provenance),packed_image_sha256=hashlib.sha256(bpy.data.images[image_name].packed_file.data).hexdigest(),rgba8_sha256=hashlib.sha256(np.rint(np.clip(after,0,1)*255).astype(np.uint8).tobytes()).hexdigest());entry['texel_provenance']['semantics']['3']='bounded-inferred-bark-continuation';entry['inferred_bark_gap_repair']=dict(plan_sha256=sha(P/'curved-quad-donor-plan.json'),targets=45,source_ownership_expansion=False);(O/'validation.json').write_text(json.dumps(validation,indent=2)+'\n')
 W=O/'actual-review-v1';W.mkdir();(W/'inspection').mkdir();(W/'modified').mkdir();(W/'model.blend').symlink_to(model.resolve());shutil.copy2(B/'actual-review-v1/workspace.json',W/'workspace.json');shutil.copy2(B/'actual-review-v1/modified/views.json',W/'modified/views.json')
 report=dict(status='SAVED_PRESERVATION_PASS_ACTUAL_REVIEW_PENDING',model_sha256=sha(model),parent_model_sha256=plan['model_sha256'],inferred_bark_texels=45,all_source_rgba_unchanged=True,all_alpha_unchanged=True,all_other_pixels_images_geometry_uv_unchanged=True,plan_sha256=sha(P/'curved-quad-donor-plan.json'),canonical_writes=False)
 (O/'reopened-preservation.json').write_text(json.dumps(report,indent=2)+'\n');assert sha(B/'worker.blend')==plan['model_sha256'];assert sum(p.stat().st_size for p in O.rglob('*') if p.is_file() and not p.is_symlink())<=64*1024**2;print(json.dumps(report),flush=True)
finally:release()

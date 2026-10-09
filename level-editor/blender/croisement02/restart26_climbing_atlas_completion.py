"""Complete inferred triangle-atlas texels with bounded same-endpoint donors."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from scipy.ndimage import distance_transform_edt
from mathutils import Matrix,Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from restart24_bake_climbing_donors import retained
from bake_texture_candidate import snapshot
from project_reviewed_texture import _read
BASE=OUT/'restart14-hidden-archer';DEST=BASE/'climbing-texture-bake-v4'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main(state):
 assert shutil.disk_usage(BASE).free>=10*1024**3
 assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>=6*1024**3
 src=BASE/f'climbing-texture-bake-v3/profile-05-{state}';record=json.loads((src/'bake-validation.json').read_text());model=src/'model.blend';assert sha(model)==record['model_sha256']
 dest=DEST/f'profile-05-{state}';assert not dest.exists();dest.mkdir(parents=True)
 experiment=BASE/f'climbing-texture-experiments-v1/{state}/experiment';generation=experiment/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary';review=json.loads((generation/'generation-review.json').read_text());assert sha(generation/'generated-raw.png')==review['raw_sha256']
 raw=_read(generation/'generated-raw.png');guide=_read(experiment/'input.png');mask=_read(experiment/'mask.png');manifest=json.loads((experiment/'views.json').read_text());cameras=[]
 for view in manifest['views']:
  c=view['crop'];left=c['left'];bottom=len(raw)-c['top']-c['height'];rgb=raw[bottom:bottom+c['height'],left:left+c['width'],:3];original=guide[bottom:bottom+c['height'],left:left+c['width'],:3]
  valid=((np.ptp(original,axis=2)>.018)|(mask[bottom:bottom+c['height'],left:left+c['width'],3]<.5))&(np.ptp(rgb,axis=2)>.018)
  distance,nearest=distance_transform_edt(~valid,return_indices=True);matrix=Matrix(view['camera_matrix_world']);cameras.append((view,np.array(matrix.inverted()),matrix.to_3x3()@Vector((0,0,1)),distance,nearest,left,bottom))
 bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;obj=next(o for o in scene.objects if o.type=='MESH');before=retained(obj)
 known=lambda:{k:v for k,v in snapshot(scene,{obj.name})['physical_foliage'].items() if v['known_rgba'] is not None}
 observed=known();changes=[]
 for info in record['new_inferred_uv_atlases']:
  slot=info['slot'];mat=obj.data.materials[slot];assert not mat.get('foliage_observed');faces=[f for f in obj.data.polygons if f.material_index==slot];assert len(faces)==info['faces']
  own=obj.data.color_attributes['Source ownership'];assert all(own.data[i].color[0]==0 for f in faces for i in f.loop_indices)
  tex=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');w,h=tex.image.size;assert [w,h]==info['size'];uv=obj.data.uv_layers[tex.inputs['Vector'].links[0].from_node.uv_map]
  arr=np.array(tex.image.pixels[:],np.float32).reshape(h,w,4);old=arr.copy();assert np.all(arr[:,:,3]==1)
  # Padding belongs only to this triangle's6x6 atlas cell. Derive its exact
  # placeholder from the unused corner, never infer source ownership by colour.
  fill_count=0;donor_faces=0;max_radius=0.;columns=w//6
  for number,face in enumerate(faces):
   left=(number%columns)*6;bottom=(number//columns)*6;cell=arr[bottom:bottom+6,left:left+6];placeholder=cell[0,0,:3].copy()
   coords=np.array([uv.data[i].uv[:] for i in face.loop_indices])*[w,h]
   assert np.all(coords>=[left+.99,bottom+.99]) and np.all(coords<=[left+5.01,bottom+5.01])
   pending=np.all(np.abs(cell[:,:,:3]-placeholder)<1e-7,axis=2)
   valid=~pending
   if valid.any():
    _,nearest=distance_transform_edt(~valid,return_indices=True);cell[pending,:3]=cell[nearest[0][pending],nearest[1][pending],:3]
   else:
    point=obj.matrix_world@face.center;normal=(obj.matrix_world.to_3x3().inverted().transposed()@face.normal).normalized();selected=None
    # Hidden/downward surfaces have no observed camera. Reuse local generated
    # material at their projected position, explicitly labelled inference.
    for view,inverse,direction,distance,nearest,ox,oy in sorted(cameras,key=lambda c:-abs(normal.dot(c[2]))):
     p=inverse[:3,:3]@np.array(point)+inverse[:3,3];crop=view['crop'];x=int(np.floor((.5+p[0]/view['ortho_scale'])*crop['width']));y=int(np.floor((.5+p[1]/view['ortho_scale'])*crop['height']))
     if not(0<=x<crop['width'] and 0<=y<crop['height']):continue
     radius=float(distance[y,x])
     if radius>8:continue
     dy,dx=nearest[:,y,x];selected=raw[oy+dy,ox+dx,:3];max_radius=max(max_radius,radius);break
    assert selected is not None,f'No bounded material donor for slot{slot} face{face.index}'
    cell[:,:,:3]=selected;donor_faces+=1
   fill_count+=int(pending.sum())
   assert not np.any(np.all(np.abs(cell[:,:,:3]-placeholder)<1e-7,axis=2))
  assert np.array_equal(arr[:,:,3],old[:,:,3]);replacement=mat.copy();image=tex.image.copy();image.pixels.foreach_set(arr.ravel());image.pack();replacement.node_tree.nodes[tex.name].image=image;replacement['texture_provenance']='Inferred generated RGB; residual triangle-cell texels use own-face nearest colour or bounded same-endpoint generated material donor.';obj.data.materials[slot]=replacement
  changes.append(dict(slot=slot,faces=len(faces),completed_texels=fill_count,wholly_unfilled_faces=donor_faces,max_generated_donor_radius_px=max_radius,physical_alpha_changed=0,observed_pixels_changed=0,unfilled_used_cell_texels=0))
 assert retained(obj)==before and known()==observed
 bpy.context.preferences.filepaths.save_version=0;scene.render.threads_mode='FIXED';scene.render.threads=2;bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True)
 bpy.ops.wm.open_mainfile(filepath=str(dest/'model.blend'));scene=bpy.context.scene;obj=next(o for o in scene.objects if o.type=='MESH');assert retained(obj)==before and known()==observed
 construction=json.loads((src/'construction.json').read_text());construction['model_sha256']=sha(dest/'model.blend');(dest/'construction.json').write_text(json.dumps(construction,indent=2)+'\n')
 record.update(status='SAVED_REOPEN_PRESERVATION_PASS; appearance/native guard pending',parent_model_sha256=sha(model),model_sha256=construction['model_sha256'],inferred_atlas_completion=changes,recipe_sha256=sha(__file__))
 (dest/'bake-validation.json').write_text(json.dumps(record,indent=2)+'\n');assert sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file())<32*1024**2
 print(json.dumps(dict(state=state,model_sha256=record['model_sha256'],changes=changes)),flush=True)
if __name__=='__main__':
 acquire(slots=2)
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()

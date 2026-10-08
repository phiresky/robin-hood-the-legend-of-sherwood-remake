"""Bake inferred climbing materials only, retaining source faces and original UVs."""
import sys,json,math,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Matrix,Vector
from scipy.ndimage import distance_transform_edt
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from refinement_review import _tree
from refinement_workspace import _geometry
from fill_physical_foliage import fill
from project_reviewed_texture import _read
from bake_texture_candidate import snapshot
BASE=OUT/'restart14-hidden-archer';DEST=BASE/'climbing-texture-bake-v2';CAP=32*1024**2

def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def budget():
 used=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()) if DEST.exists() else 0
 assert used<CAP and shutil.disk_usage(BASE).free>=10*1024**3+CAP-used

def retained(obj):
 mesh=obj.data;uv=mesh.uv_layers['Foliage UV'];ownership=mesh.color_attributes['Source ownership']
 return dict(geometry=_geometry(obj),original_uv=hashlib.sha256(np.array([v.uv[:] for v in uv.data],np.float32).tobytes()).hexdigest(),ownership=hashlib.sha256(np.array([v.color[:] for v in ownership.data],np.float32).tobytes()).hexdigest())

def main(state):
 budget();assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>=6*1024**3
 experiment=BASE/f'climbing-texture-experiments-v1/{state}/experiment';generation=experiment/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary';review=json.loads((generation/'generation-review.json').read_text());assert review['ready_for_bake'];model=experiment/'approved-model.blend';assert sha(model)==review['model_sha256']
 for kind in ['raw','preserved']:assert sha(generation/f'generated-{kind}.png')==review[kind+'_sha256']
 manifest=json.loads((experiment/'views.json').read_text());destination=DEST/f'profile-05-{state}';assert not destination.exists();destination.mkdir(parents=True)
 bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];assert len(objects)==1;obj=objects[0];mesh=obj.data;before=retained(obj);known_before={k:v for k,v in snapshot(scene,{obj.name})['physical_foliage'].items() if v['known_rgba'] is not None}
 original_uv=mesh.uv_layers['Foliage UV'];new_uv=mesh.uv_layers.new(name='Inferred generated leaf atlas');new_uv.data.foreach_set('uv',np.array([v.uv[:] for v in original_uv.data],np.float32).ravel());reparameterized=[]
 for slot,material in enumerate(list(mesh.materials)):
  if not material or material.get('foliage_observed'):continue
  faces=[f for f in mesh.polygons if f.material_index==slot]
  if not faces:continue
  textures=[n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
  assert len(textures)==1
  if list(textures[0].image.size)!=[1,1]:continue
  pix=np.array(textures[0].image.pixels[:]);assert pix[3]>=.999
  assert all(mesh.color_attributes['Source ownership'].data[i].color[0]==0 for f in faces for i in f.loop_indices)
  size=int(math.ceil(math.sqrt(len(faces))))*6;atlas=bpy.data.images.new(f'Inferred leaf atlas {state} {slot}',width=size,height=size,alpha=True);atlas.generated_color=(float(pix[0]),float(pix[1]),float(pix[2]),1);atlas.pack();clone=material.copy();clone.name=material.name+' generated atlas';tex=next(n for n in clone.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);tex.image=atlas;tex.inputs['Vector'].links[0].from_node.uv_map=new_uv.name;mesh.materials[slot]=clone
  columns=size//6
  for number,face in enumerate(faces):
   points=np.array([mesh.vertices[i].co[:] for i in face.vertices]);axis=points[1]-points[0];axis/=np.linalg.norm(axis);normal=np.array(face.normal);side=np.cross(normal,axis);xy=np.column_stack([points@axis,points@side]);span=np.ptp(xy,axis=0);assert span.min()>1e-9;coords=(xy-xy.min(0))/span;coords=(coords*4+np.array([(number%columns)*6+1,(number//columns)*6+1]))/size
   for loop,coord in zip(face.loop_indices,coords):new_uv.data[loop].uv=coord
  reparameterized.append(dict(slot=slot,faces=len(faces),size=[size,size],physical_alpha='Original fully opaque1x1 is replaced by fully opaque atlas; face coverage unchanged.',original_uv_unchanged=True))
 assert retained(obj)==before
 mesh.calc_loop_triangles();tree,owners,vertices=_tree(objects);face_of_triangle=[t.polygon_index for t in mesh.loop_triangles];generated=_read(generation/'generated-preserved.png');mask=_read(experiment/'mask.png');cameras=[]
 for view in manifest['views']:
  matrix=Matrix(view['camera_matrix_world']);cameras.append((view,np.array(matrix.inverted()),matrix.to_3x3()@Vector((0,0,1))))
 raw=_read(generation/'generated-raw.png');donors=[]
 # Donors are explicitly generated material inference, not source evidence.
 # Restrict to chromatic pixels inside original occupied foreground, preventing
 # background and detached generated additions from becoming material donors.
 guide=_read(experiment/'input.png')
 for view,inverse,direction in cameras:
  c=view['crop'];left=c['left'];bottom=len(raw)-c['top']-c['height'];rgb=raw[bottom:bottom+c['height'],left:left+c['width'],:3];original=guide[bottom:bottom+c['height'],left:left+c['width'],:3]
  valid=(np.ptp(rgb,axis=2)>.018)&(np.ptp(original,axis=2)>.018)
  # Editable original gray foreground is also a valid generated donor location.
  valid|=(mask[bottom:bottom+c['height'],left:left+c['width'],3]<.5)&(np.ptp(rgb,axis=2)>.018)
  assert valid.any();distance,nearest=distance_transform_edt(~valid,return_indices=True);donors.append((distance,nearest,left,bottom))
 stats=dict(accepted_samples=0,unfilled_samples=0,inferred_donor_samples=0,inferred_donor_max_radius_px=0.)
 def sample(target,normal,positions,accepted,colors,*,face_index,record_statistics=True):
  for score,view,inverse,direction in sorted([(normal.dot(direction),view,inverse,direction) for view,inverse,direction in cameras],key=lambda r:-r[0]):
   if score<=.08:continue
   todo=np.flatnonzero(~accepted)
   if not len(todo):break
   local=positions[todo]@inverse[:3,:3].T+inverse[:3,3];crop=view['crop'];x=np.floor(crop['left']+(.5+local[:,0]/view['ortho_scale'])*crop['width']).astype(int);y=np.floor(len(generated)-crop['top']-(.5-local[:,1]/view['ortho_scale'])*crop['height']).astype(int)
   inside=(x>=crop['left'])&(x<crop['left']+crop['width'])&(y>=len(generated)-crop['top']-crop['height'])&(y<len(generated)-crop['top'])
   for k in np.flatnonzero(inside):
    if mask[y[k],x[k],3]>=.5:continue
    index=todo[k];point=Vector(positions[index]);hit,_,triangle,_=tree.ray_cast(point+direction*4000,-direction)
    if hit is None or face_of_triangle[triangle]!=face_index or (hit-point).length>.005:continue
    colors[index,:3]=generated[y[k],x[k],:3];accepted[index]=True
  for camera_index in sorted(range(len(cameras)),key=lambda i:-normal.dot(cameras[i][2])):
   view,inverse,direction=cameras[camera_index]
   if normal.dot(direction)<=.08:continue
   todo=np.flatnonzero(~accepted)
   if not len(todo):break
   local=positions[todo]@inverse[:3,:3].T+inverse[:3,3];crop=view['crop'];x=np.floor((.5+local[:,0]/view['ortho_scale'])*crop['width']).astype(int);y=np.floor((.5+local[:,1]/view['ortho_scale'])*crop['height']).astype(int)
   distance,nearest,left,bottom=donors[camera_index]
   inside=(x>=0)&(x<crop['width'])&(y>=0)&(y<crop['height'])
   for k in np.flatnonzero(inside):
    radius=float(distance[y[k],x[k]])
    if radius>8:continue
    dy,dx=nearest[:,y[k],x[k]];index=todo[k];colors[index,:3]=raw[bottom+dy,left+dx,:3];accepted[index]=True
    if record_statistics:stats['inferred_donor_samples']+=1;stats['inferred_donor_max_radius_px']=max(stats['inferred_donor_max_radius_px'],radius)
  if record_statistics:stats['accepted_samples']+=int(accepted.sum());stats['unfilled_samples']+=int((~accepted).sum())
  return accepted
 report=fill(objects,sample,None,sha(generation/'generated-preserved.png'));budget();assert retained(obj)==before
 known_after={k:v for k,v in snapshot(scene,{obj.name})['physical_foliage'].items() if v['known_rgba'] is not None};assert known_after==known_before
 scene.render.threads_mode='FIXED';scene.render.threads=2;bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(destination/'model.blend'),compress=True);budget();candidate_hash=sha(destination/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(destination/'model.blend'));obj=next(o for o in bpy.context.scene.objects if o.type=='MESH');assert retained(obj)==before;assert {k:v for k,v in snapshot(bpy.context.scene,{obj.name})['physical_foliage'].items() if v['known_rgba'] is not None}==known_before
 source_record=json.loads((BASE/f'climbing-v21-edge/profile-05-{state}/construction.json').read_text());source_record['model_sha256']=candidate_hash;write(destination/'construction.json',source_record);write(destination/'bake-validation.json',dict(status='SAVED_REOPEN_PRESERVATION_PASS; appearance/native guard pending',source_model_sha256=sha(model),model_sha256=candidate_hash,retained=before,observed_material_atlases_exact=True,original_uv_layer_exact=True,new_inferred_uv_atlases=reparameterized,fill=report,stats=stats,generation_review_sha256=sha(generation/'generation-review.json'),texture_approval='pending',inference='Residual unknown-only RGB uses same-endpoint generated raw chromatic donor within8 view pixels; not observed source evidence.',scope='Texture-only: no changed mesh or physical alpha; new UV layer only for inferred opaque placeholder materials.'))
 assert sha(model)==review['model_sha256'];print(json.dumps(dict(state=state,model_sha256=candidate_hash,stats=stats,output_bytes=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()))),flush=True)
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()

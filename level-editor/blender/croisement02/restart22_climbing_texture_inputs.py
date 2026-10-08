"""Prepare saved-camera climbing texture inputs without requesting generation."""
import sys,json,math,shutil,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from scipy.ndimage import binary_dilation
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from refinement_review import _tree
from tree_geometry import RAY,SIN,COS
from restart2_prepare_endpoint_inputs import signature
from render_slots import acquire,release
from prepare_private_texture_inputs import prepare
BASE=OUT/'restart14-hidden-archer';SOURCE=BASE/'climbing-v21-edge';DEST=BASE/'climbing-texture-inputs-v1';CAP=32*1024**2
APPROVAL=OUT/'restart3-review-batches/next-climbing-shed-v2/user-approval.json'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def budget():
 used=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()) if DEST.exists() else 0
 assert used<CAP and shutil.disk_usage(BASE).free>=10*1024**3+CAP-used

def main():
 assert sha(APPROVAL)=='830263fe013c8a0157c158290e529e91188bf8bad6f984de090c53569795d088'
 mem=int(next(x for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')).split()[1])*1024;assert mem>=6*1024**3
 budget();assert not DEST.exists();DEST.mkdir()
 user=json.loads(APPROVAL.read_text());members={m['asset_id']:m for m in user['members']}
 for state in ['initial','applied']:
  asset=f'croisement02-hidden-archer05-climbing-{state}';approval=members[asset];base=SOURCE/f'profile-05-{state}';model=base/'model.blend';assert sha(model)==approval['model_sha256'];record=json.loads((base/'construction.json').read_text())
  dest=DEST/state;dest.mkdir();modified=dest/'modified';views=modified/'views';views.mkdir(parents=True)
  bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];assert len(objects)==1;obj=objects[0];before=signature(objects);obj['asset_group']=asset;obj['source_node']=asset;mesh=obj.data;mesh.calc_loop_triangles();tree,owners,vertices=_tree(objects)
  collection=bpy.data.collections.new('Approved climbing texture working');scene.collection.children.link(collection);collection.objects.link(obj)
  bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True);assert signature(objects)==before;budget()
  points=np.array([o.matrix_world@v.co for o in objects for v in o.data.vertices]);center=Vector((points.min(0)+points.max(0))/2);scale=float(np.linalg.norm(points.max(0)-points.min(0)))*1.12
  camera=bpy.data.objects.new('Frozen input camera',bpy.data.cameras.new('Frozen input camera'));scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.ortho_scale=scale
  sheets={k:Image.new('RGBA',(1280,640)) for k in ['textured','solid']};records=[];proof=[]
  for index in range(8):
   angle=index*math.pi/4;direction=Vector((COS*math.sin(angle),-COS*math.cos(angle),SIN));camera.location=center+direction*4000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();bpy.context.view_layer.update();matrix=camera.matrix_world.copy();occupied=np.zeros((320,320),bool);known=np.zeros((320,320),bool);unit=scale/320
   for y in range(320):
    for x in range(320):
     origin=matrix.translation+matrix.col[0].to_3d()*((x+.5-160)*unit)+matrix.col[1].to_3d()*((160-y-.5)*unit)
     point,normal,face,distance=tree.ray_cast(origin,-matrix.col[2].to_3d())
     if point is None:continue
     occupied[y,x]=True;known[y,x]=bool(mesh.materials[mesh.loop_triangles[face].material_index].get('foliage_observed'))
   # Protect one-pixel antialias boundaries around observed fragments conservatively.
   protected=binary_dilation(known,iterations=1);editable=occupied&~protected
   original=np.array(Image.open(base/'review-v2'/f'actual-{index}.png').convert('RGBA'));guide=original.copy();guide[editable,:3]=77
   solid=np.array(Image.open(base/'review-v2'/f'solid-{index}.png').convert('RGBA'));solid[:,:,3]=occupied.astype(np.uint8)*255
   ownership=np.full((320,320,4),255,np.uint8);ownership[editable,:3]=0
   assert np.array_equal(guide[~editable],original[~editable])
   for kind,pixels in [('textured',guide),('solid',solid),('known',ownership)]:Image.fromarray(pixels).save(views/f'view-{index}-{kind}.png')
   for kind,pixels in [('textured',guide),('solid',solid)]:sheets[kind].paste(Image.fromarray(pixels),((index%4)*320,(index//4)*320))
   records.append(dict(index=index,azimuth_degrees=index*45,camera_matrix_world=[list(r) for r in matrix],camera_location=list(camera.location),camera_rotation_euler=list(camera.rotation_euler),ortho_scale=scale,crop=dict(left=0,top=0,width=320,height=320),ownership_sha256=sha(views/f'view-{index}-known.png')))
   proof.append(dict(view=index,known_first_hits=int(known.sum()),protected_pixels=int(protected.sum()),editable_pixels=int(editable.sum()),protected_rgba_exact=True));print(state,index,proof[-1],flush=True);budget()
  for kind,image in sheets.items():image.save(modified/f'{kind}.png')
  evidence={str(p):sha(p) for p in [APPROVAL,model,base/'construction.json',base/'review-v2/native-coverage.json',SOURCE/'self-review-v1.json',SOURCE/'root-review-v1.json',Path(record['source'])]}
  manifest=dict(version=1,asset_id=asset,scene_name=scene.name,collection_name=collection.name,tile_size=[320,320],layout=dict(columns=4,rows=2),elevation_degrees=35,source_image=record['source'],source_sha256=record['source_sha256'],source_blend=str(dest/'model.blend'),source_mask_evidence=evidence,object_names=[obj.name],render_object_names=[obj.name],texture_receiver_object_names=[obj.name],review_state=state,views=records,texture_material_suffix='climbing-inferred-only-v1',known_rule='Saved alpha-aware first hit on explicitly observed material is protected, dilated one output pixel for antialias edges. Only inferred material first hits are editable. All native observed faces/materials remain immutable.',projection_layers=[],limitations=['Uses exact approved review-camera rays and saved render pixels; no rescaling.','Gray inferred surfaces only are editable; one-pixel protection is conservative.','Final imported texture must retain all observed materials and pass the7073 native guard.'])
  write(modified/'views.json',manifest);assert signature(objects)==before and sha(model)==approval['model_sha256']
  write(dest/'derivation.json',dict(source_model=str(model),source_model_sha256=sha(model),prepared_model_sha256=sha(dest/'model.blend'),geometry_uv_material_signature=before,geometry_uv_materials_unchanged=True,source_image=record['source'],source_sha256=record['source_sha256'],source_opaque_centers=record['source_opaque_centers'],user_approval=str(APPROVAL),user_approval_sha256=sha(APPROVAL),source_decision=approval,views=proof,changes='Only private asset metadata/collection membership; no mesh/material/UV changes. Exact approved review RGBA is preserved outside inferred-only edit mask.'))
  prepare(dest,sha(dest/'model.blend'),dest/'private-inputs');budget()
 print('Paired local inputs complete; no API called',flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()

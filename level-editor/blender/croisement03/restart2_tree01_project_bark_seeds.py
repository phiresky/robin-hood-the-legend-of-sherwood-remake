"""Privately project reviewed visible bark seeds while retaining unassigned surface context."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
sys.path.insert(0,str(Path(__file__).parent))
import restart2_tree01_canopy_saved as common
B=common.B;sha=common.sha;OUT=B/'tree01-bark-seed-projection-v2';EXTRA_PROOF=None
def main():
 assert not OUT.exists();common.resources();common.acquire(slots=2)
 try:
  common.resources();OUT.mkdir();base=B/'tree01-native-transition-saved-v2/worker.blend';proofpath=base.parent/'bark-receiver-294-v1/receipt.json';proof=json.loads(proofpath.read_text());assert proof['status']=='PASS_SAVED_BARK_RECEIVERS' and sha(base)==proof['model_sha256'];source=B.parent/'baseline/covered.png';assert sha(source)==proof['source_sha256'];src=np.array(Image.open(source).convert('RGBA'));bpy.ops.wm.open_mainfile(filepath=str(base));scene=bpy.context.scene;geometry={};groups={}
  extra_pin=None
  if EXTRA_PROOF is not None:
   extra=json.loads(EXTRA_PROOF.read_text());assert extra['status']=='PASS_SAVED_BARK_RECEIVERS' and extra['model_sha256']==sha(base) and extra['source_sha256']==sha(source) and not extra['full_native_overlap']
   proposal=B/'tree01-bark-completeness-v2/proposed-additional-bark.png';assert extra['mask_sha256']==sha(proposal);mask=np.array(Image.open(proposal))>0;domain={(int(x),int(y))for y,x in np.argwhere(mask)};assert len(domain)==147 and domain=={tuple(r['source'])for r in extra['samples']}
   assert not domain&{tuple(r['source'])for r in proof['samples']};proof['samples']+=extra['samples'];extra_pin={'path':str(EXTRA_PROOF),'sha256':sha(EXTRA_PROOF)}
  expected_count=432 if EXTRA_PROOF is not None else 285
  phasepath=B/'tree01-bark-completeness-v2/receipt.json';phaseproof=json.loads(phasepath.read_text())
  for path,digest in phaseproof['pins'].items():assert sha(Path(path))==digest,path
  withheld={tuple(r['source_xy'])for r in phaseproof['old294_full_native_overlap_details']};assert len(withheld)==9
  measured=set()
  for frame in phaseproof['full_animation_source']['frames']:
   fp=Path(frame['path']);assert sha(fp)==frame['sha256'];rgba=np.array(Image.open(fp).convert('RGBA'));x0,y0=frame['bbox'][:2]
   for sample in proof['samples']:
    x,y=sample['source'];xx,yy=x-x0,y-y0
    if 0<=xx<rgba.shape[1]and 0<=yy<rgba.shape[0]and rgba[yy,xx,3]:measured.add((x,y))
  assert measured==withheld
  for o in scene.objects:
   if o.type=='MESH':geometry[o.name]=([list(v.co)for v in o.data.vertices],[list(p.vertices)for p in o.data.polygons]);o.data.calc_loop_triangles()
  for sample in proof['samples']:
   if tuple(sample['source'])in withheld:continue
   name=sample['receiver'];obj=scene.objects[name];face=obj.data.loop_triangles[sample['triangle']].polygon_index;groups.setdefault((name,face),[]).append(sample['source'])
  pins={};assignments=[]
  for (name,face),pixels in groups.items():
   obj=scene.objects[name];xs,ys=zip(*pixels);l,t,r,b=min(xs),min(ys),max(xs)+1,max(ys)+1;rgba=src[t:b,l:r].copy();rgba[:,:,3]=0
   for x,y in pixels:rgba[y-t,x-l]=src[y,x]
   path=OUT/f'wood-{name.split()[-1]}-face-{face}.png';Image.fromarray(rgba).save(path);pins[str(path)]=sha(path);image=bpy.data.images.load(str(path));image.pack();mat=bpy.data.materials.new(f'Private source bark {name}/{face}');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';em=nodes.new('ShaderNodeEmission');gray=nodes.new('ShaderNodeBsdfPrincipled');gray.inputs['Base Color'].default_value=(.25,.25,.25,1);mix=nodes.new('ShaderNodeMixShader');out=nodes.new('ShaderNodeOutputMaterial');links=mat.node_tree.links;links.new(tex.outputs['Color'],em.inputs[0]);links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(gray.outputs[0],mix.inputs[1]);links.new(em.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],out.inputs[0]);obj.data.materials.append(mat);obj.data.polygons[face].material_index=len(obj.data.materials)-1;uv=obj.data.uv_layers.active or obj.data.uv_layers.new(name='Native bark seed coordinates')
   for li in obj.data.polygons[face].loop_indices:
    v=obj.matrix_world@obj.data.vertices[obj.data.loops[li].vertex_index].co;uv.data[li].uv=((v.x-l)/(r-l),1-(-v.y*common.S-v.z*common.C-t)/(b-t))
   assignments.append({'object':name,'polygon':face,'pixels':pixels,'image':str(path),'source_box':[l,t,r,b]})
  bpy.context.preferences.filepaths.save_version=0;model=OUT/'worker.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));assert model.stat().st_size<8*1024**2;bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
  for name,(v,f) in geometry.items():
   obj=scene.objects[name];assert [list(p.co)for p in obj.data.vertices]==v and [list(p.vertices)for p in obj.data.polygons]==f
  for row in assignments:
   obj=scene.objects[row['object']];mat=obj.data.materials[obj.data.polygons[row['polygon']].material_index];im=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');assert __import__('hashlib').sha256(bytes(im.packed_file.data)).hexdigest()==pins[row['image']]
  assert sha(base)==proof['model_sha256'];report={'status':'PRIVATE_PROJECTION_PENDING_INDEPENDENT_RGBA_AND_VISUAL_REVIEW','input_model_sha256':sha(base),'model_sha256':sha(model),'receiver_proof_sha256':sha(proofpath),'additional_receiver_proof':extra_pin,'source_sha256':sha(source),'full_native_phase_proof':{'path':str(phasepath),'sha256':sha(phasepath)},'withheld_full_native_foreground_samples':sorted(withheld),'pins':pins,'projected_samples':sum(len(r['pixels'])for r in assignments),'assignments':assignments,'geometry_exact':True,'limits':[f'Only{expected_count} reviewed samples independently clear of all14 full shared native frames; nine formerly fragment-clear samples withheld. No hidden bark sampled or synthesized.','Fifteen leaf-covered spines and seven upper misses remain unassigned.','Static leaf ownership remains on hold. This is not complete bark coverage or appearance approval.']};assert report['projected_samples']==expected_count;(OUT/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print({'samples':expected_count,'faces':len(assignments),'model_sha256':report['model_sha256']})
 finally:common.release()
if __name__=='__main__':main()

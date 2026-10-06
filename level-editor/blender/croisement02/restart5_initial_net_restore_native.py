"""Retain exact initial source shaders above generated unknown fallback textures."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from bake_texture_candidate import snapshot,pixels,array_hash
from refinement_review import _tree
from render_multiview_asset import render
from tree_geometry import RAY,SIN
ROOT=OUT/'restart5-initial-nets'
def images(mat):
 return {n.name:dict(name=n.image.name,rgba=array_hash(pixels(n.image)),packed=hashlib.sha256(bytes(n.image.packed_file.data)).hexdigest()if n.image.packed_file else None)for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image}
def main(key, bake_name='bake-v1', output_name='native-retained-v1'):
 assert shutil.disk_usage(OUT).free>25*1024**3
 exp=ROOT/f'texture-fill-v1/profile-{key}/experiment';out=exp/output_name;assert not out.exists();meta=json.loads((exp/'views.json').read_text());names=set(meta['object_names']);native_uv='Initial native source projection';bpy.ops.wm.open_mainfile(filepath=str(exp/'approved-model.blend'));scene=bpy.data.scenes[meta['scene_name']];baseline=snapshot(scene,names);records={}
 for name in names:
  obj=scene.objects[name];records[name]=dict(slots=[p.material_index for p in obj.data.polygons],uv=np.array([d.uv[:]for d in obj.data.uv_layers[native_uv].data]),images=images(obj.data.materials[0]))
 bpy.ops.wm.open_mainfile(filepath=str(exp/bake_name/'worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,names)==baseline;counts={}
 for name,r in records.items():
  obj=scene.objects[name];assert np.array_equal(r['uv'],np.array([d.uv[:]for d in obj.data.uv_layers[native_uv].data]));assert images(obj.data.materials[0])==r['images'];generated_slots={p.material_index for p in obj.data.polygons if obj.data.materials[p.material_index].get('source_ownership_bake')};assert len(generated_slots)==1;gs=next(iter(generated_slots));gm=obj.data.materials[gs];gn=next(n for n in gm.node_tree.nodes if n.type=='TEX_IMAGE');guv=gn.inputs['Vector'].links[0].from_node.uv_map;restored=obj.data.materials[0].copy();restored.name=name+' / exact native with unknown fill';mix=next(n for n in restored.node_tree.nodes if n.type=='MIX_RGB');assert not mix.inputs[1].is_linked;node=restored.node_tree.nodes.new('ShaderNodeTexImage');node.image=gn.image;node.interpolation='Linear';node.extension='EXTEND';uv=restored.node_tree.nodes.new('ShaderNodeUVMap');uv.uv_map=guv;restored.node_tree.links.new(uv.outputs['UV'],node.inputs['Vector']);restored.node_tree.links.new(node.outputs['Color'],mix.inputs[1]);obj.data.materials.append(restored);slot=len(obj.data.materials)-1;count=0
  for face,old in zip(obj.data.polygons,r['slots']):
   if old==0 and face.material_index==gs:face.material_index=slot;count+=1
  counts[name]=count
 assert snapshot(scene,names)==baseline;out.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True);digest=sha(out/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(out/'worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,names)==baseline
 base=ROOT/f'candidate-v5/profile-{key}';source_report=json.loads((base/'report.json').read_text());source=np.array(Image.open(source_report['source']['source']).convert('RGBA'));observed=np.array(Image.open(base/'observed-source.png').convert('RGBA'));objects=[scene.objects[n]for n in sorted(names)];tree,owners,_=_tree(objects);tris=[]
 for obj in objects:obj.data.calc_loop_triangles();tris.extend((obj,t)for t in obj.data.loop_triangles)
 results=[];ox,oy=source_report['source']['origin']
 for y,x in np.argwhere(observed[:,:,3]>0):
  p,n,i,dist=tree.ray_cast(Vector((ox+x+.5,-(oy+y+.5)/SIN,0))+RAY*6000,-RAY);assert p is not None;obj,t=tris[i];assert owners[i]==obj;mat=obj.data.materials[t.material_index];native=next(nd for nd in mat.node_tree.nodes if nd.type=='TEX_IMAGE'and nd.image and nd.inputs['Vector'].links[0].from_node.uv_map==native_uv);coords=[Vector((*obj.data.uv_layers[native_uv].data[j].uv,0))for j in t.loops];q=barycentric_transform(p,*[obj.matrix_world@obj.data.vertices[j].co for j in t.vertices],*coords);a=np.rint(pixels(native.image)*255).astype(np.uint8);hh,ww=a.shape[:2];rgba=a[max(0,min(hh-1,int(q.y*hh))),max(0,min(ww-1,int(q.x*ww)))];assert np.array_equal(rgba,source[y,x]);results.append([int(x),int(y)])
 for name,r in records.items():assert images(scene.objects[name].data.materials[0])==r['images'];assert np.array_equal(r['uv'],np.array([d.uv[:]for d in scene.objects[name].data.uv_layers[native_uv].data]))
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;render(exp/'views.json',out/'actual',width=384);sheet=Image.new('RGBA',(1536,768))
 for i in range(8):sheet.paste(Image.open(out/'actual'/f'view-{i}-textured.png').convert('RGBA'),((i%4)*384,(i//4)*384))
 sheet.save(out/'actual/textured.png');write_json(out/'preservation.json',dict(status='PASS',model_sha256=digest,approved_model_sha256=sha(exp/'approved-model.blend'),geometry_uv_unchanged=True,original_native_image_packed_bytes_and_RGBA_exact=True,native_sample_count=len(results),native_exact_RGBA=len(results),restored_faces=counts,source_review=str(base/'root-review.json'),source_review_sha256=sha(base/'root-review.json'),scope='Generated fallback and unknown reverse only; exact original source and UV retained. Upper-loop attachment geometry unchanged.'))
 source_report.update(model_sha256=digest,parent_approved_geometry_sha256=sha(base/'model.blend'),texture_preservation_sha256=sha(out/'preservation.json'));write_json(out/'report.json',source_report)
if __name__=='__main__':
 acquire()
 try:main(*sys.argv[sys.argv.index('--')+1:])
 finally:release()

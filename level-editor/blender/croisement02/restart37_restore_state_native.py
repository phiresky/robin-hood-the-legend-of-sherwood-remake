"""Retain approved state source shaders over generated unknown-only fills."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
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
BASE=OUT/'restart25-approved-state-materialization-v1'
def main(key):
 assert key in ('hole-initial','hole-applied','mound-initial')
 assert shutil.disk_usage(OUT).free>=10*1024**3
 assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024>=6*1024**3
 exp=BASE/'official-texture-experiments-v2'/key/'experiment';out=exp/('native-state-v4' if key=='mound-initial' else 'native-state-v1');assert not out.exists();meta=json.loads((exp/'views.json').read_text());names=set(meta['object_names']);approval=json.loads((exp/'approval.json').read_text());assert sha(exp/'approved-model.blend')==approval['saved_model_sha256'];derivation=approval['approval_provenance']['derivation'];native_uv=derivation['native_uv'];source=Path(derivation['source_image']);assert sha(source)==derivation['source_sha256'];native=np.array(Image.open(source).convert('RGBA'));h,w=native.shape[:2]
 bpy.ops.wm.open_mainfile(filepath=str(exp/'approved-model.blend'));scene=bpy.data.scenes[meta['scene_name']];before=snapshot(scene,names);records={}
 for name in names:
  o=scene.objects[name];records[name]={'slots':[p.material_index for p in o.data.polygons],'uv':np.array([d.uv[:] for d in o.data.uv_layers[native_uv].data]),'images':{i:[(n.name,array_hash(pixels(n.image)),sha(source) if n.image.packed_file and hashlib.sha256(bytes(n.image.packed_file.data)).hexdigest()==sha(source) else hashlib.sha256(bytes(n.image.packed_file.data)).hexdigest()) for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image] for i,m in enumerate(o.data.materials)}}
 ownership={};origin=None
 if key=='mound-initial':
  authority=json.loads((BASE/'mound-ownership-v1/report.json').read_text());assert sha(BASE/'mound-ownership-v1/report.json')==derivation['ownership_sha256'];owned=np.load(authority['ownership']['path']);site=next(r for r in authority['records'] if r['site']=='site-00');ownership={r['object']:owned[r['ownership_array']] for r in site['objects']};origin=site['source_origin']
 else:
  report=json.loads(Path(derivation['source_model']).with_name('report.json').read_text());origin=report['source']['offset']
 bpy.ops.wm.open_mainfile(filepath=str(exp/'bake-state-v1/worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,names)==before;out.mkdir();restored={}
 for name,r in records.items():
  o=scene.objects[name];assert np.array_equal(r['uv'],np.array([d.uv[:]for d in o.data.uv_layers[native_uv].data]));generated={p.material_index for p in o.data.polygons if o.data.materials[p.material_index].get('source_ownership_bake')};assert len(generated)==1,(name,generated);gs=next(iter(generated));gm=o.data.materials[gs];gn=next(n for n in gm.node_tree.nodes if n.type=='TEX_IMAGE');guv=gn.inputs['Vector'].links[0].from_node.uv_map;oldslots=set(r['slots']);mapping={}
  for oldslot in oldslots:
   mat=o.data.materials[oldslot];textures=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
   known=bool(textures) if key!='mound-initial' else any(n.type=='BSDF_PRINCIPLED' and n.inputs['Emission Color'].is_linked for n in mat.node_tree.nodes)
   if not known:continue
   restoredmat=mat.copy();restoredmat.name=name+' exact native over inferred fill';nodes=restoredmat.node_tree.nodes;links=restoredmat.node_tree.links;newtex=nodes.new('ShaderNodeTexImage');newtex.image=gn.image;newtex.interpolation='Linear';newtex.extension='EXTEND';uv=nodes.new('ShaderNodeUVMap');uv.uv_map=guv;links.new(uv.outputs['UV'],newtex.inputs['Vector'])
   if key!='mound-initial':
    mix=next(n for n in nodes if n.type=='MIX_RGB');assert not mix.inputs[1].is_linked;links.new(newtex.outputs['Color'],mix.inputs[1])
   else:
    native_node=next(n for n in nodes if n.type=='TEX_IMAGE' and n != newtex);assert not native_node.inputs['Vector'].is_linked;native_map=nodes.new('ShaderNodeUVMap');native_map.uv_map=native_uv;links.new(native_map.outputs['UV'],native_node.inputs['Vector']);maskpath=out/(name.replace(' ','_')+'-ownership.png');rgba=np.zeros((h,w,4),np.uint8);rgba[:,:,:3]=np.where(ownership[name][:,:,None],255,0);rgba[:,:,3]=255;Image.fromarray(rgba).save(maskpath);masktex=nodes.new('ShaderNodeTexImage');masktex.image=bpy.data.images.load(str(maskpath));masktex.image.colorspace_settings.name='Non-Color';masktex.image.pack();masktex.interpolation='Closest';masktex.extension='CLIP';nuv=nodes.new('ShaderNodeUVMap');nuv.uv_map=native_uv;links.new(nuv.outputs['UV'],masktex.inputs['Vector']);bsdf=next(n for n in nodes if n.type=='BSDF_PRINCIPLED');oldcolor=bsdf.inputs['Emission Color'].links[0].from_socket;mix=nodes.new('ShaderNodeMixRGB');links.new(masktex.outputs['Color'],mix.inputs[0]);links.new(newtex.outputs['Color'],mix.inputs[1]);links.new(oldcolor,mix.inputs[2]);links.new(mix.outputs['Color'],bsdf.inputs['Emission Color'])
   o.data.materials.append(restoredmat);mapping[oldslot]=len(o.data.materials)-1
  count=0
  for p,oldslot in zip(o.data.polygons,r['slots']):
   if oldslot in mapping:p.material_index=mapping[oldslot];count+=1
  restored[name]=count
 assert snapshot(scene,names)==before;bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True);digest=sha(out/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(out/'worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;bpy.context.view_layer.update();assert snapshot(scene,names)==before
 objects=[scene.objects[n]for n in sorted(names)];tris=[];vertices=[];faces=[]
 for o in objects:
  assert not o.modifiers
  o.data.calc_loop_triangles();offset=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(offset+i for i in t.vertices)for t in o.data.loop_triangles);tris.extend((o,t)for t in o.data.loop_triangles)
 tree=BVHTree.FromPolygons(vertices,faces,all_triangles=True)
 failures=[];tested=0;maxuv=0
 for y,x in np.argwhere(native[:,:,3]>0):
  p,n,i,dist=tree.ray_cast(Vector((origin[0]+x+.5,-(origin[1]+y+.5)/SIN,0))+RAY*2000,-RAY)
  if p is None:failures.append([int(x),int(y),'miss']);continue
  o,t=tris[i];mat=o.data.materials[t.material_index];texs=[nd for nd in mat.node_tree.nodes if nd.type=='TEX_IMAGE' and nd.image and nd.inputs['Vector'].links and nd.inputs['Vector'].links[0].from_node.uv_map==native_uv and nd.image.colorspace_settings.name!='Non-Color'];assert len(texs)==1,(o.name,len(texs));tex=texs[0];q=barycentric_transform(p,*[o.matrix_world@o.data.vertices[j].co for j in t.vertices],*[Vector((*o.data.uv_layers[native_uv].data[j].uv,0))for j in t.loops]);xx=int(q.x*w);yy=int((1-q.y)*h);maxuv=max(maxuv,abs(q.x*w-(x+.5)),abs((1-q.y)*h-(y+.5)));assert 0<=xx<w and 0<=yy<h
  rgba=np.rint(pixels(tex.image)*255).astype(np.uint8)[h-1-yy,xx];ok=np.array_equal(rgba,native[y,x]);ok=ok and (key!='mound-initial' or ownership[o.name][yy,xx]);tested+=1
  if not ok:failures.append([int(x),int(y),'native mismatch'])
 assert not failures,failures[:20]
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;render(exp/'views.json',out/'actual',width=384);sheet=Image.new('RGBA',(1536,768))
 for i in range(8):sheet.paste(Image.open(out/'actual'/f'view-{i}-textured.png').convert('RGBA'),((i%4)*384,(i//4)*384))
 sheet.save(out/'actual/textured.png');write_json(out/'preservation.json',dict(status='PASS',asset_id=meta['asset_id'],model_sha256=digest,approved_model_sha256=sha(exp/'approved-model.blend'),geometry_uv_unchanged=True,native_pixel_centers=tested,native_failures=failures,maximum_native_uv_error=maxuv,restored_faces=restored,scope=('Generated unknown-only fallback; known native shaders and original UV remain exact.' + (' Mound template only: other 19 sites pending individual ownership restoration.' if key=='mound-initial' else '')),texture_approval='pending'))
 print(json.dumps({'key':key,'model_sha256':digest,'native_centers':tested}),flush=True)
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()

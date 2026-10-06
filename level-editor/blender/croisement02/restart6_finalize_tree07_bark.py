"""Freeze a generated bark derivative with original source atlas restoration."""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from PIL import Image,ImageDraw
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
from render_multiview_asset import render
from bake_texture_candidate import snapshot
from restart6_tree07_restore_native import restore_atlas
O=R/'level-editor/work/croisement02-refinement';B=O/'restart6-tree07-bark-fill-v1';E=B/'experiment';S=B/'bake-v1';D=B/'native-restored-v1';G=O/'restart6-tree07-native-atlas-guard-v1/guard.json';W=O/'root-stem-round-2/assets/croisement02-tree-07';h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();read=lambda p:json.loads(p.read_text());expected='8bcb2ea9a6c40f920e15590569934f18c3bb3c002c26bb8160a6903605f107df'
def pixels(im):
 a=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(a);return a.reshape(im.size[1],im.size[0],4)
def atlas(ob):return next((m,next(n.image for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image))for m in ob.data.materials if m and m.get('source_ownership_label')=='exterior')
def sheet(directory,name):
 im=Image.new('RGBA',(1536,768))
 for i in range(8):im.paste(Image.open(directory/f'view-{i}-textured.png'),((i%4)*384,(i//4)*384))
 im.save(directory/name)
assert h(W/'model.blend')==expected;assert not D.exists();D.mkdir();acquire()
try:
 guard=read(G);names={r['object'] for r in guard['records']};meta=read(E/'views.json');bpy.ops.wm.open_mainfile(filepath=str(W/'model.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;baseline=snapshot(scene,names)
 bpy.ops.wm.open_mainfile(filepath=str(S/'worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,names)==baseline;layers=read(S/'layer-0.json');records=[]
 for name in sorted(names):
  ob=bpy.data.objects[name];mat,im=atlas(ob);rgba=pixels(im);row=next(r for r in layers['objects'] if r['object']==name);provenance=Path(row['texel_provenance']['path']);assert h(provenance)==row['texel_provenance']['sha256'];flags=np.load(provenance)['ownership'];height,width=flags.shape;supportimage=Image.new('1',(width,height));draw=ImageDraw.Draw(supportimage);uv=ob.data.uv_layers['Owned source / exterior'];ob.data.calc_loop_triangles()
  for t in ob.data.loop_triangles:draw.polygon([(uv.data[l].uv.x*width,uv.data[l].uv.y*height)for l in t.loops],fill=1)
  support=np.array(supportimage,bool);restored,receipt=restore_atlas(G,name,rgba,flags,support);im.pixels.foreach_set(restored.ravel());im.update();im.pack();records.append(receipt)
 assert snapshot(scene,names)==baseline;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;bpy.ops.wm.save_as_mainfile(filepath=str(D/'worker.blend'));modelhash=h(D/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(D/'worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,names)==baseline
 for r in guard['records']:
  a=np.load(r['original_atlas_path']);saved=pixels(atlas(bpy.data.objects[r['object']])[1]);assert np.array_equal(np.rint(saved[a['known']]*255),np.rint(a['rgba'][a['known']]*255));assert np.array_equal(np.rint(saved[:,:,3]*255),np.rint(a['rgba'][:,:,3]*255))
 render(E/'views.json',D/'close',width=384);sheet(D/'close','textured.png');full=read(W/'inspection/actual-camera-manifest.json');full.pop('render_object_names',None);full.pop('texture_receiver_object_names',None);(D/'full-views.json').write_text(json.dumps(full,indent=2)+'\n');render(D/'full-views.json',D/'full',width=384);sheet(D/'full','textured.png');assert h(W/'model.blend')==expected;proof=dict(status='PASS',model_sha256=modelhash,parent_geometry_sha256=expected,parent_bake_sha256=h(S/'worker.blend'),guard_sha256=h(G),geometry_and_outside_appearance_exact=True,reopened_known_rgba8_exact=True,reopened_alpha8_exact=True,atlas_repairs=records,review='Actual close/full8 manual review pending',files={str(p.relative_to(D)):h(p)for p in D.rglob('*')if p.is_file() and p.name!='worker.blend'});(D/'native-preservation.json').write_text(json.dumps(proof,indent=2)+'\n');print(modelhash,flush=True)
finally:release()

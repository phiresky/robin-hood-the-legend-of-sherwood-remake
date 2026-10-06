"""Read-only two-state diagnosis of fourteen dark fence/contact source pixels."""
import sys,json,argparse
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from restore_ground75_source import geometry
from restart3_fence_receiver import atlas
from restart3_initial_fence_contact import link
from refinement_review import _tree
from review_bank_candidate import camera
from tree_geometry import SIN,RAY
D=OUT/'restart4-fence14-role-audit-v1';GROUND=OUT/'restart4-final-floor-bake-v1/model.blend';INITIAL=OUT/'restart3-initial-fence/geometry-v6/model.blend';APPLIED=OUT/'restart2-textures/approved-cleared-fence-fill-v2/croisement02-south-field-wattle-fence-cleared-state/experiment/bake-v1/worker.blend';GLB=OUT/'restart2-state/cleared-fence-receiver-export-v2/model.glb'
def main():
 global D,GROUND
 parser=argparse.ArgumentParser();parser.add_argument('--ground',type=Path);parser.add_argument('--ground-sha');parser.add_argument('--output',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]if '--'in sys.argv else [])
 if args.ground:
  assert args.ground_sha and args.output;GROUND=args.ground;D=args.output
 assert not D.exists();pins={GROUND:args.ground_sha or '206c7c562b9aa9fc204ce42c0b0e9b1b78a22bee79ac00e6dab7fbbc59c64630',INITIAL:'46579f5398d1495b13e8d8433fa1a0687be447e025cd9ba745f5d7251076c5a0',APPLIED:'13978cf8bffdf7f3ef5deff863f11ea40c008bbf01b3f0d23f201d937203ea82',GLB:'29db0d17df1c600766b3590c745adb50d4a74366ba32a2e99d97148d520be2c1'};assert all(sha(p)==h for p,h in pins.items());D.mkdir()
 authority=OUT/'restart7-fence-residual/source-role-assessment-v1.json';coords=json.loads(authority.read_text())['mask98']['coordinates'];assert len(coords)==14
 source=np.array(Image.open(OUT/'source-states/covered.png').convert('RGBA'));raw=np.array(Image.open(OUT/'restart2-ground-completion/approved-fill-retry-v2/generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-raw.png').convert('RGBA'));known=np.array(Image.open(OUT/'restart4-final-floor-bake-v1/known-native-domain.png').convert('L'))>0
 bpy.ops.wm.open_mainfile(filepath=str(GROUND));scene=bpy.data.scenes.new('Fence14 source roles');bpy.context.window.scene=scene;ground=bpy.data.objects['Croisement02 Terrain'];link(scene,ground);ground.hide_render=False;bpy.context.view_layer.update();sig=geometry(ground);node,base=atlas(ground);imported={}
 names=json.loads((OUT/'fence-state-candidate-v2/applied-views.json').read_text())['object_names']
 for label,path in [('initial',INITIAL),('applied',APPLIED)]:
  with bpy.data.libraries.load(str(path),link=False)as(src,dst):dst.objects=list(src.objects)if label=='initial'else names
  chosen=[o for o in dst.objects if o and o.type=='MESH'and(label=='applied'or o.get('asset_group')=='croisement02-south-field-wattle-fence')]
  for o in chosen:link(scene,o)
  imported[label]=chosen
 old=set(bpy.data.objects);bpy.ops.import_scene.gltf(filepath=str(GLB));overlay=[o for o in bpy.data.objects if o not in old and o.type=='MESH'];assert len(overlay)==1
 bpy.context.view_layer.update();rows=[]
 for state in ['initial','applied']:
  for label,objs in imported.items():
   for o in objs:o.hide_render=label!=state
  for o in overlay:o.hide_render=state!='applied'
  tree,owners,_=_tree([ground]+imported[state]+(overlay if state=='applied'else[]))
  for x,y in coords:
   hit,normal,index,dist=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);owner=owners[index]if hit is not None else None;rows.append(dict(state=state,pixel=[x,y],receiver=owner.name if owner else None,ground_first_hit=owner==ground,hit=list(hit)if hit is not None else None,source_rgba=source[y,x].tolist(),ground_rgba=base[y,x].tolist(),known_native_ground=bool(known[y,x]),generated_raw_exact=bool(np.array_equal(base[y,x],raw[y,x])),inside_terminal_patch=1018<=x<1170 and 811<=y<963))
  for view,direction in [('native',RAY),('oblique',Vector((.55,-.65,.5)).normalized())]:
   camera(scene,Vector((965,-851/SIN,0)),direction,768,576,100);scene.render.filepath=str(D/f'{state}-{view}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
 sheet=Image.new('RGB',(1536,2*604),'#292929');draw=ImageDraw.Draw(sheet)
 for row,view in enumerate(['native','oblique']):
  for col,state in enumerate(['initial','applied']):sheet.paste(Image.open(D/f'{state}-{view}.png'),(col*768,row*604+28));draw.text((col*768+5,row*604+7),view+' '+state,fill='white')
 sheet.save(D/'two-state-close.png');marked=source.copy();domain=np.zeros(base.shape[:2],bool)
 for x,y in coords:marked[y,x,:3]=[255,50,150];domain[y,x]=True
 box=(930,825,990,875);s=Image.new('RGB',(1080,332),'#292929');draw=ImageDraw.Draw(s)
 for col,(a,label)in enumerate([(source,'Exact native source'),(marked,'Fourteen dark boundary centers'),(base,'Current saved ground pixels')]):s.paste(Image.fromarray(a).crop(box).resize((360,300),Image.Resampling.NEAREST),(col*360,32));draw.text((col*360+5,8),label,fill='white')
 s.save(D/'source-versus-underlay.png');Image.fromarray(domain.astype('uint8')*255).save(D/'domain14.png')
 assert geometry(ground)==sig and np.array_equal(atlas(ground)[1],base)and all(sha(p)==h for p,h in pins.items())
 report=dict(status='Read-only source-role check; no user appearance approval implied',authority_sha256=sha(authority),pins={str(p):h for p,h in pins.items()},rows=rows,counts=dict(native_ground_first_hits=sum(r['ground_first_hit']for r in rows if r['state']=='initial'),applied_ground_first_hits=sum(r['ground_first_hit']for r in rows if r['state']=='applied'),native_preserved=sum(np.array_equal(base[y,x],source[y,x])for x,y in coords),regenerated_underlay=sum(np.array_equal(base[y,x],raw[y,x])for x,y in coords),inside_terminal_patch=0),receiver_hypothesis='Ground/contact-shadow receiver is physically available in both states, but native dark strand versus painted shadow is not resolved by first-hit alone. Exact14 RGB source-preservation overlay or floor restoration would require explicit role approval; do not extrude fence below ground or silently transfer wood.',state_conditionality='These14 lie outside terminal152x152 patch and beside surviving fence; applied overlay never owns them. Any accepted observed-context treatment must remain in both states unless independent source proves otherwise.',ground_geometry_unchanged=True,ground_rgba_unchanged=True,new_model_written=False,api_calls=0,files={p.name:sha(p)for p in D.glob('*.png')})
 write_json(D/'report.json',report);print(json.dumps(report['counts']),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
